// SPDX-License-Identifier: GPL-2.0
//
// The EVDI DRM/KMS device: registers a `struct drm_device` presenting one virtual
// display head (CRTC + primary plane + virtual encoder + virtual connector) with
// GEM-shmem dumb buffers, built on the safe KMS mode-object layer (`kernel::drm::kms`).
//
// Unlike a real display driver, EVDI's scanout is *pulled* by userspace: the
// DisplayLinkManager daemon grabs framebuffer pixels via the GRABPIX ioctl and is
// told when to do so through `drm_event`s (see `painter.rs`). The KMS callbacks here
// therefore translate atomic commits into those events rather than programming any
// hardware.

use core::sync::atomic::{AtomicBool, AtomicI64, AtomicU32, Ordering};
use kernel::{
    drm,
    drm::event::{EventChannel, EventConnection},
    drm::kms::{
        connector::{self, ConnectorGuard, ConnectorModeValidation, ModeStatus},
        crtc::{self, CrtcAtomicCommit, RawCrtc as _, RawCrtcState as _},
        encoder,
        framebuffer::{Framebuffer, FramebufferVMapOwned},
        modes::DisplayMode,
        plane::{self, PlaneAtomicCommit, RawPlaneState as _},
        vblank::{
            OwnedVblankRef, RawVblankCrtcState as _, VblankGuard, VblankSupport, VblankTimestamp,
        },
        KmsDriver, ModeConfigGuard, ModeConfigInfo, ModeObject as _, UnregisteredKmsDevice,
    },
    impl_has_hr_timer,
    interrupt::LocalInterruptDisabled,
    prelude::*,
    sync::{
        aref::ARef, new_mutex, new_spinlock, new_spinlock_irq, Arc, ArcBorrow, Mutex, SpinLock,
        SpinLockIrq,
    },
    time::{
        hrtimer::{
            ArcHrTimerHandle, HrTimer, HrTimerCallback, HrTimerCallbackContext, HrTimerPointer,
            HrTimerRestart, RelativeHardMode,
        },
        Delta, Monotonic,
    },
};

use crate::painter::PainterState;

/// Cursor-plane format list. libevdi reports the format to the client, which expects ARGB.
static CURSOR_FORMATS: [u32; 1] = [drm::fourcc::ARGB8888];

static PRIMARY_FORMATS: [u32; 8] = [
    drm::fourcc::XRGB8888,
    drm::fourcc::ARGB8888,
    drm::fourcc::XBGR8888,
    drm::fourcc::ABGR8888,
    drm::fourcc::XRGB2101010,
    drm::fourcc::ARGB2101010,
    drm::fourcc::XBGR2101010,
    drm::fourcc::ABGR2101010,
];

/// Fallback mode advertised before the DLM client delivers an EDID via CONNECT.
const FALLBACK_W: u32 = 1024;
const FALLBACK_H: u32 = 768;

// libevdi requires `major == 1 && minor >= 9`. DisplayLinkManager also
// compares this value with the supported upstream evdi ABI, so keep it in
// lockstep with libevdi.
const INFO: drm::DriverInfo = drm::DriverInfo {
    major: 1,
    minor: 15,
    patchlevel: 0,
    name: c"evdi",
    desc: c"Extensible Virtual Display Interface",
};

/// The EVDI DRM driver marker type.
pub(crate) struct EvdiDrmDriver;

/// Convenience alias for our concrete `drm::Device`.
pub(crate) type EvdiDrmDevice = drm::Device<EvdiDrmDriver>;

/// One framebuffer prepared for repeated GRABPIX calls.
pub(crate) struct PreparedScanout {
    pub(crate) framebuffer: ARef<Framebuffer<EvdiDrmDriver>>,
    pub(crate) mapping: FramebufferVMapOwned<EvdiObject>,
}

const SCANOUT_BINDINGS: usize = 4;

struct ScanoutState {
    current: Option<Arc<PreparedScanout>>,
    bindings: [Option<Arc<PreparedScanout>>; SCANOUT_BINDINGS],
    next: usize,
}

impl ScanoutState {
    const fn new() -> Self {
        Self {
            current: None,
            bindings: [const { None }; SCANOUT_BINDINGS],
            next: 0,
        }
    }

    fn prepare(&mut self, fb: Option<&Framebuffer<EvdiDrmDriver>>) -> Result {
        let Some(fb) = fb else {
            self.discard();
            return Ok(());
        };
        let prepared = if let Some(prepared) = self
            .bindings
            .iter()
            .flatten()
            .find(|prepared| &*prepared.framebuffer == fb)
        {
            prepared.clone()
        } else {
            let prepared = Arc::new(
                PreparedScanout {
                    framebuffer: ARef::from(fb),
                    mapping: fb.owned_vmap::<EvdiObject>()?,
                },
                GFP_KERNEL,
            )?;
            self.bindings[self.next] = Some(prepared.clone());
            self.next = (self.next + 1) % SCANOUT_BINDINGS;
            prepared
        };
        self.current = Some(prepared);
        Ok(())
    }

    fn discard(&mut self) {
        self.current = None;
        self.bindings = [const { None }; SCANOUT_BINDINGS];
        self.next = 0;
    }
}

/// DRM device-private data.
#[pin_data]
pub(crate) struct EvdiDrmData {
    /// Event channel to the connected DLM client (`drm_event` delivery).
    pub(crate) events: Arc<EventChannel<EvdiDrmDriver, EvdiDrmFile>>,
    /// Painter state (connection status and dirty rectangles).
    #[pin]
    pub(crate) painter: Mutex<PainterState>,
    /// The current framebuffer and a bounded set of owned, validated swapchain mappings, so
    /// repeated flips and GRABPIX calls do not remap them.
    #[pin]
    scanout: Mutex<ScanoutState>,
    /// EDID and bandwidth limits delivered by the DLM client through CONNECT.
    #[pin]
    cached_edid: Mutex<Option<KVec<u8>>>,
    pixel_area_limit: AtomicU32,
    pixel_per_second_limit: AtomicU32,
    /// Active software-vblank timer and its cancellation handle.
    #[pin]
    vblank: SpinLock<Option<(Arc<VblankTimer>, ArcHrTimerHandle<VblankTimer>)>>,
    /// The CRTC's colour transform (`CTM` and `GAMMA_LUT`), applied in software during GRABPIX.
    ///
    /// evdi has no colour hardware, so a compositor correcting through these properties has
    /// nowhere else to put the correction.
    #[pin]
    pub(crate) color: Mutex<Option<crate::color::ColorPipeline>>,
    /// Whether the client asked for cursor events (`EVDI_ENABLE_CURSOR_EVENTS`).
    ///
    /// Cleared by default: a client that has not opted in composites the pointer into the primary
    /// framebuffer itself and must not also receive it out of band.
    pub(crate) cursor_events: AtomicBool,
    /// The mode announced by the last [`EvdiCrtc::atomic_enable`], as `(hdisplay, vdisplay,
    /// vrefresh)`, or `None` while the CRTC is off.
    ///
    /// MODE_CHANGED is an edge, and a client that connects to a card the compositor has *already*
    /// configured never sees it -- it waits forever for a mode that was announced before it
    /// attached, and its display stays dark. Keeping the current mode lets CONNECT replay it.
    #[pin]
    announced_mode: Mutex<Option<(i32, i32, i32)>>,
}

impl EvdiDrmData {
    pub(crate) fn new() -> impl PinInit<Self, Error> {
        try_pin_init!(Self {
            events: EventChannel::new()?,
            painter <- new_mutex!(PainterState::new()),
            scanout <- new_mutex!(ScanoutState::new()),
            cached_edid <- new_mutex!(None),
            pixel_area_limit: AtomicU32::new(0),
            pixel_per_second_limit: AtomicU32::new(0),
            vblank <- new_spinlock!(None),
            color <- new_mutex!(None),
            cursor_events: AtomicBool::new(false),
            announced_mode <- new_mutex!(None),
        })
    }

    /// Tell a client that has just connected what the display is already doing.
    ///
    /// Called from CONNECT, after the caller is registered as the event receiver. Silent when the
    /// CRTC is off, which is the ordinary first-connect case: the mode then arrives from
    /// [`EvdiCrtc::atomic_enable`] as usual.
    pub(crate) fn replay_state(&self) {
        let Some((hdisplay, vdisplay, vrefresh)) = *self.announced_mode.lock() else {
            return;
        };
        crate::painter::notify_dpms(self, crate::painter::DPMS_ON);
        crate::painter::notify_mode_changed(
            self,
            hdisplay,
            vdisplay,
            vrefresh,
            32,
            drm::fourcc::XRGB8888,
        );
    }

    /// Stop timer activity and release prepared scanout state before unbind.
    pub(crate) fn shutdown(&self) {
        self.scanout.lock().discard();
        let timer = self.vblank.lock().take();
        if let Some((timer, handle)) = timer {
            timer.enabled.store(false, Ordering::Relaxed);
            drop(handle);
            if let Some(crtc) = timer.crtc.lock().take() {
                drop(crtc.crtc().vblank_pinned.lock().take());
            }
        }
    }

    /// Prepare the framebuffer currently on the primary plane for later GRABPIX calls.
    pub(crate) fn set_scanout(&self, fb: Option<&Framebuffer<EvdiDrmDriver>>) -> Result {
        self.scanout.lock().prepare(fb)
    }

    /// Take an owned handle to the current prepared scanout, if any.
    pub(crate) fn prepared_scanout(&self) -> Option<Arc<PreparedScanout>> {
        self.scanout.lock().current.clone()
    }

    /// Install a new EDID blob (from CONNECT) into the connector and fire a hotplug so
    /// the compositor re-probes the connector's mode list.
    pub(crate) fn set_edid(&self, dev: &EvdiDrmDevice, blob: KVec<u8>) {
        *self.cached_edid.lock() = Some(blob);
        dev.hotplug_event();
    }

    /// Drop the connector's cached EDID (on CONNECT disconnect) and fire a hotplug so the connector
    /// reports disconnected again -- see [`EvdiConnector::detect`].
    pub(crate) fn clear_edid(&self, dev: &EvdiDrmDevice) {
        *self.cached_edid.lock() = None;
        dev.hotplug_event();
    }

    /// Store the dock bandwidth limits the client supplied via CONNECT, for
    /// [`EvdiConnector`]'s `mode_valid` to enforce. Must be called before the EDID is
    /// published (`set_edid`'s hotplug re-probes the mode list against these limits).
    pub(crate) fn set_mode_limits(&self, pixel_area: u32, pixels_per_second: u32) {
        self.pixel_area_limit.store(pixel_area, Ordering::Relaxed);
        self.pixel_per_second_limit
            .store(pixels_per_second, Ordering::Relaxed);
    }
}

/// GEM object inner data. Empty: the shmem-backed object wires
/// `drm_gem_shmem_dumb_create`, so `DRM_IOCTL_MODE_CREATE_DUMB` works and the GRABPIX
/// ioctl can `vmap` the resulting framebuffer to copy pixels to userspace.
#[pin_data]
pub(crate) struct EvdiObject {}

impl drm::gem::DriverObject for EvdiObject {
    type Driver = EvdiDrmDriver;
    type Args = ();

    fn new(
        _dev: &drm::Device<EvdiDrmDriver>,
        _size: usize,
        _args: (),
    ) -> impl PinInit<Self, Error> {
        try_pin_init!(EvdiObject {})
    }
}

/// Per-open DRM client state.
#[pin_data]
pub(crate) struct EvdiDrmFile {
    /// The owning device, so file close can reach the event channel to disconnect.
    dev: ARef<EvdiDrmDevice>,
    /// The connection token is owned by the file, so closing or disconnecting drops it and
    /// automatically detaches this receiver from the channel.
    #[pin]
    pub(crate) connection: Mutex<Option<EventConnection<EvdiDrmDriver, EvdiDrmFile>>>,
}

impl drm::file::DriverFile for EvdiDrmFile {
    type Driver = EvdiDrmDriver;

    fn open(dev: &drm::Device<Self::Driver>) -> Result<Pin<KBox<Self>>> {
        KBox::try_pin_init(
            try_pin_init!(Self {
                dev: dev.into(),
                connection <- new_mutex!(None),
            }),
            GFP_KERNEL,
        )
    }
}

#[vtable]
impl drm::Driver for EvdiDrmDriver {
    type Data = EvdiDrmData;
    type File = EvdiDrmFile;
    type Object = drm::gem::shmem::Object<EvdiObject>;
    type ParentDevice<Ctx: kernel::device::DeviceContext> = kernel::platform::Device<Ctx>;
    type RegistrationData<'a> = ();
    type Kms = Self;

    const INFO: drm::DriverInfo = INFO;

    kernel::declare_drm_ioctls! {
        (EVDI_CONNECT, drm_evdi_connect, 0, crate::ioctl::connect),
        (EVDI_REQUEST_UPDATE, drm_evdi_request_update, 0, crate::ioctl::request_update),
        (EVDI_GRABPIX, drm_evdi_grabpix, 0, crate::ioctl::grabpix),
        (EVDI_DDCCI_RESPONSE, drm_evdi_ddcci_response, 0, crate::ioctl::ddcci_response),
        (EVDI_ENABLE_CURSOR_EVENTS, drm_evdi_enable_cursor_events, 0,
            crate::ioctl::enable_cursor_events),
    }

    kernel::declare_drm_compat_ioctls! {
        EvdiDrmDriver;
        (EVDI_CONNECT, drm_evdi_connect, crate::uapi::DrmEvdiConnect32, IOWR,
            crate::ioctl::connect),
        (EVDI_GRABPIX, drm_evdi_grabpix, crate::uapi::DrmEvdiGrabpix32, IOWR,
            crate::ioctl::grabpix),
    }
}

#[vtable]
impl KmsDriver for EvdiDrmDriver {
    type Connector = EvdiConnector;
    type Plane = EvdiPlane;
    type Crtc = EvdiCrtc;
    type Encoder = EvdiEncoder;

    fn mode_config_info(
        _dev: &kernel::device::Device,
        _drm_data: &Self::Data,
    ) -> Result<ModeConfigInfo> {
        Ok(ModeConfigInfo {
            min_resolution: (0, 0),
            max_resolution: (8192, 8192),
            max_cursor: (64, 64),
            preferred_depth: 32,
            preferred_fourcc: Some(drm::fourcc::XRGB8888),
        })
    }

    fn create_objects(dev: &UnregisteredKmsDevice<'_, Self>) -> Result {
        let primary = plane::UnregisteredPlane::<EvdiPlane>::new(
            dev,
            1,
            &PRIMARY_FORMATS,
            None,
            plane::Type::Primary,
            None,
            false,
        )?;
        // Advertise FB_DAMAGE_CLIPS so the compositor reports which region
        // changed. Without it, GRABPIX must return the full plane.
        primary.enable_fb_damage_clips();
        // PRIMARY_FORMATS includes ARGB/ABGR scanout formats.  DRM requires every plane that
        // exposes an alpha format to describe its alpha convention, even though EVDI forwards
        // the resulting pixels to its userspace consumer instead of doing hardware compositing.
        // Compositors supply the primary scanout in premultiplied form.
        primary.create_blend_mode_property(plane::BlendModes::PREMULTIPLIED)?;
        // A real cursor plane, so the compositor keeps the pointer out of the primary framebuffer
        // and the client can drive its sink's own cursor. Without one, every pointer movement
        // dirties the desktop and costs a full frame.
        let cursor = plane::UnregisteredPlane::<EvdiPlane>::new(
            dev,
            1,
            &CURSOR_FORMATS,
            None,
            plane::Type::Cursor,
            None,
            true,
        )?;
        // ARGB8888 exposes alpha, and DRM core requires a blend mode alongside it -- without this
        // drm_mode_config_validate() warns "pixel format with alpha exposed but blend mode not
        // setup" at registration. The bitmap the client receives is premultiplied.
        cursor.create_blend_mode_property(plane::BlendModes::PREMULTIPLIED)?;
        let crtc_obj =
            crtc::UnregisteredCrtc::<EvdiCrtc>::new(dev, primary, Some(&cursor), None, ())?;
        // Advertise CTM and a 256-entry GAMMA_LUT. evdi has no colour hardware to program, so
        // GRABPIX applies both in software on the way to the client; without these properties a
        // compositor's colour correction silently does nothing on an evdi output while native
        // outputs are corrected normally.
        crtc_obj.enable_color_mgmt(0, true, crate::color::LUT_LEN as u32);
        let enc = encoder::UnregisteredEncoder::<EvdiEncoder>::new(
            dev,
            encoder::Type::Virtual,
            crtc_obj.mask(),
            0,
            None,
            (),
        )?;
        // Use DisplayPort rather than Virtual: `__drm_connector_init` skips
        // `drm_connector_attach_edid_property()` for VIRTUAL/WRITEBACK connectors, and without that
        // property `drm_edid_connector_update()` can't populate `edid_blob_ptr`, so
        // `drm_edid_connector_add_modes()` would return 0 modes for a perfectly valid EDID.
        let conn = connector::UnregisteredConnector::<EvdiConnector>::new(
            dev,
            connector::Type::DisplayPort,
            (),
        )?;
        conn.attach_encoder(&*enc)?;
        // Do not merely advertise 10-bit framebuffer formats: compositors select them through
        // this standard range property. EVDI's GRABPIX path transports the packed 32-bit pixels
        // unchanged; actual HDR signalling remains conditional on the client/sink path.
        conn.attach_max_bpc_property(8, 10)?;
        conn.attach_hdr_output_metadata_property();
        conn.attach_colorspace_property()?;
        Ok(())
    }
}

// ---- CRTC -------------------------------------------------------------------

/// A software vblank source: an hrtimer that fires once per frame and drives
/// `drm_crtc_handle_vblank()`, so the atomic helpers pace page-flips against a real vblank
/// (via `drm_crtc_arm_vblank_event()` in [`EvdiCrtc::atomic_flush`]) instead of completing them
/// immediately with a fake vblank -- which is what makes updates smooth rather than bursty.
///
/// The timer stops itself when vblank is disabled (mirroring the C core's
/// `drm_vblank_timer_function`, which returns `HRTIMER_NORESTART` on a zeroed interval): the
/// callback sees `enabled == false` and returns [`HrTimerRestart::NoRestart`], so an idle or
/// DPMS-off output costs no wakeups. `enable_vblank` re-arms it with a raw
/// [`HasHrTimer::start`], which re-queues the timer whether it is dead or still pending --
/// no new handle is minted, so the single [`ArcHrTimerHandle`] taken at first start remains
/// the sole owner and its drop (at CRTC teardown, before the `drm_crtc` is freed) is the only
/// full `hrtimer_cancel`. Neither enable nor disable ever blocks on the callback, which is
/// what makes them deadlock-free against `drm_crtc_handle_vblank` (see the deadlock note in
/// `drm_crtc_vblank_cancel_timer`).
#[pin_data]
pub(crate) struct VblankTimer {
    #[pin]
    timer: HrTimer<Self>,
    /// Owned CRTC reference used by the hard-timer callback.
    #[pin]
    crtc: SpinLockIrq<Option<crtc::CrtcRef<EvdiCrtc>>>,
    /// One scanout frame in nanoseconds (from the mode's `framedur_ns`).
    interval_ns: AtomicI64,
    /// Whether vblanks should currently be delivered (toggled by enable/disable_vblank).
    enabled: AtomicBool,
}

impl VblankTimer {
    fn new() -> impl PinInit<Self> {
        pin_init!(VblankTimer {
            timer <- HrTimer::new(),
            crtc <- new_spinlock_irq!(None, "evdi::vblank_crtc"),
            interval_ns: AtomicI64::new(16_666_666), // ~60 Hz until a mode sets it
            enabled: AtomicBool::new(false),
        })
    }
}

impl HrTimerCallback for VblankTimer {
    type Pointer<'a> = Arc<Self>;

    fn run(this: ArcBorrow<'_, Self>, mut ctx: HrTimerCallbackContext<'_, Self>) -> HrTimerRestart {
        // Vblank is off: let the timer die instead of ticking uselessly; `enable_vblank`
        // re-arms it. A concurrent re-arm racing this return is safe -- hrtimer keeps a timer
        // that was re-queued during its callback enqueued even on NORESTART.
        if !this.enabled.load(Ordering::Relaxed) {
            return HrTimerRestart::NoRestart;
        }
        let crtc = this.crtc.lock_with(ctx.local_interrupt_disabled()).clone();
        if let Some(crtc) = crtc {
            crtc.crtc().handle_vblank();
        }
        let interval = this.interval_ns.load(Ordering::Relaxed).max(1_000_000);
        ctx.forward_now(Delta::from_nanos(interval));
        HrTimerRestart::Restart
    }
}

impl_has_hr_timer! {
    impl HasHrTimer<Self> for VblankTimer {
        mode: RelativeHardMode<Monotonic>, field: self.timer
    }
}

#[pin_data]
pub(crate) struct EvdiCrtc {
    /// The software vblank source for this CRTC.
    vblank: Arc<VblankTimer>,
    /// Driver-owned vblank reference held for the active interval.
    #[pin]
    vblank_pinned: Mutex<Option<OwnedVblankRef<EvdiCrtc>>>,
}

#[derive(Clone, Default)]
pub(crate) struct EvdiCrtcState;

impl crtc::DriverCrtcState for EvdiCrtcState {
    type Crtc = EvdiCrtc;
}

#[vtable]
impl crtc::DriverCrtc for EvdiCrtc {
    type Args = ();
    type Driver = EvdiDrmDriver;
    type State = EvdiCrtcState;
    type VblankImpl = Self;

    fn new(_device: &drm::Device<Self::Driver>, _args: &()) -> impl PinInit<Self, Error> {
        try_pin_init!(EvdiCrtc {
            vblank: Arc::pin_init(VblankTimer::new(), GFP_KERNEL)?,
            vblank_pinned <- new_mutex!(None),
        })
    }

    /// Display turning on: enable vblank delivery, then tell the DLM client DPMS-on + the mode.
    fn atomic_enable(commit: CrtcAtomicCommit<'_, Self>) {
        let crtc = commit.crtc();
        crtc.vblank_on();
        let mut pinned = crtc.vblank_pinned.lock();
        if pinned.is_none() {
            if let Ok(vblank_ref) = crtc.vblank_get() {
                *pinned = Some(vblank_ref.into_owned());
            }
        }
        drop(pinned);
        let dev = crtc.drm_dev();
        let data: &EvdiDrmData = dev;
        let new = commit.take_new_state();
        let mode = new.mode();
        let (hdisplay, vdisplay, vrefresh) = (
            mode.hdisplay() as i32,
            mode.vdisplay() as i32,
            mode.vrefresh(),
        );
        *data.announced_mode.lock() = Some((hdisplay, vdisplay, vrefresh));
        crate::painter::notify_dpms(data, crate::painter::DPMS_ON);
        crate::painter::notify_mode_changed(
            data,
            hdisplay,
            vdisplay,
            vrefresh,
            32,
            drm::fourcc::XRGB8888,
        );
    }

    /// Display turning off: stop vblank delivery and tell the DLM client DPMS-off.
    fn atomic_disable(commit: CrtcAtomicCommit<'_, Self>) {
        let crtc = commit.crtc();
        drop(crtc.vblank_pinned.lock().take());
        crtc.vblank_off();
        let dev = crtc.drm_dev();
        let data: &EvdiDrmData = dev;
        let _ = data.set_scanout(None);
        *data.color.lock() = None;
        *data.announced_mode.lock() = None;
        crate::painter::notify_dpms(data, crate::painter::DPMS_OFF);
    }

    /// Arm the page-flip completion event to be sent by the next vblank tick, so userspace is paced
    /// to the refresh rate rather than signalled immediately.
    fn atomic_flush(commit: CrtcAtomicCommit<'_, Self>) {
        let crtc = commit.crtc();
        let mut new = commit.take_new_state();
        // Re-cache on every commit rather than only at enable: a night-light corrector ramps its
        // CTM continuously on an already-enabled CRTC, which never re-runs atomic_enable.
        {
            let data: &EvdiDrmData = crtc.drm_dev();
            let built = crate::color::ColorPipeline::build(new.gamma_lut(), new.ctm());
            *data.color.lock() = built;
        }
        if let Some(pending) = new.get_pending_vblank_event() {
            match crtc.vblank_get() {
                Ok(vbl_ref) => pending.arm(vbl_ref),
                // Vblank couldn't be enabled (e.g. mid-teardown): fall back to sending now.
                Err(_) => pending.send(),
            }
        }
    }
}

impl VblankSupport for EvdiCrtc {
    type Crtc = EvdiCrtc;

    fn enable_vblank(
        crtc: &crtc::Crtc<Self::Crtc>,
        vblank_guard: &VblankGuard<'_, Self::Crtc>,
        irq: &LocalInterruptDisabled,
    ) -> Result {
        let data: &EvdiCrtc = crtc;
        // Track the mode's real frame duration so the tick matches the negotiated refresh rate.
        let fd = vblank_guard.frame_duration();
        if fd > 0 {
            data.vblank.interval_ns.store(fd as i64, Ordering::Relaxed);
        }
        {
            let mut published = data.vblank.crtc.lock_with(irq);
            if published.is_none() {
                *published = Some(crtc.to_owned_ref());
            }
        }
        data.vblank.enabled.store(true, Ordering::Relaxed);
        let interval = data.vblank.interval_ns.load(Ordering::Relaxed);
        let drm_data: &EvdiDrmData = crtc.drm_dev();
        let mut timer = drm_data.vblank.lock();
        match &*timer {
            None => {
                *timer = Some((
                    data.vblank.clone(),
                    data.vblank.clone().start(Delta::from_nanos(interval)),
                ));
            }
            Some((_, handle)) => {
                handle.restart(Delta::from_nanos(interval));
            }
        }
        Ok(())
    }

    fn disable_vblank(
        crtc: &crtc::Crtc<Self::Crtc>,
        _vblank_guard: &VblankGuard<'_, Self::Crtc>,
        _irq: &LocalInterruptDisabled,
    ) {
        let data: &EvdiCrtc = crtc;
        data.vblank.enabled.store(false, Ordering::Relaxed);
    }

    fn get_vblank_timestamp(
        _crtc: &crtc::Crtc<Self::Crtc>,
        _in_vblank_irq: bool,
    ) -> Option<VblankTimestamp> {
        // Let DRM estimate the timestamp from the mode timings.
        None
    }
}

// ---- Primary plane -----------------------------------------------------------

#[pin_data]
pub(crate) struct EvdiPlane {
    /// Cursor planes report their contents to the client instead of being composited here.
    is_cursor: bool,
}

#[derive(Clone, Default)]
pub(crate) struct EvdiPlaneState;

impl plane::DriverPlaneState for EvdiPlaneState {
    type Plane = EvdiPlane;
}

#[vtable]
impl plane::DriverPlane for EvdiPlane {
    type Args = bool;
    type Driver = EvdiDrmDriver;
    type State = EvdiPlaneState;

    fn new(_device: &drm::Device<Self::Driver>, is_cursor: bool) -> impl PinInit<Self, Error> {
        try_pin_init!(EvdiPlane { is_cursor })
    }

    /// A new framebuffer was flipped in.
    ///
    /// EVDI records the new scanout buffer and signals the DLM client to grab it (UPDATE_READY).
    fn atomic_update(commit: PlaneAtomicCommit<'_, Self>) {
        let plane = commit.plane();
        let dev = plane.drm_dev();
        let data: &EvdiDrmData = dev;

        if plane.is_cursor {
            update_cursor(commit, data, dev);
            return;
        }

        // Record the framebuffer plus each region the compositor changed, accumulate them for
        // GRABPIX, and signal the client. UPDATE_READY fires on every real flip; REQUEST_UPDATE
        // deliberately does not self-signal, avoiding a request/event/grab busy loop.
        let (old, new) = commit.take_old_new_state();
        let fb = new.framebuffer::<EvdiDrmDriver>();
        if let Err(error) = data.set_scanout(fb) {
            pr_warn!("evdi: failed to prepare scanout framebuffer ({error:?})\n");
            return;
        }
        if fb.is_some() {
            {
                let mut p = data.painter.lock();
                new.for_each_damage_clip(old, |r| p.damage.push((r.x1, r.y1, r.x2, r.y2)));
                p.frame_dirty = true;
            }
            crate::painter::notify_update_ready(data);
        }
    }
}

// ---- Encoder ----------------------------------------------------------------

#[pin_data]
pub(crate) struct EvdiEncoder;

#[vtable]
impl encoder::DriverEncoder for EvdiEncoder {
    type Driver = EvdiDrmDriver;
    type Args = ();

    fn new(_device: &drm::Device<Self::Driver>, _args: ()) -> impl PinInit<Self, Error> {
        try_pin_init!(EvdiEncoder {})
    }
}

// ---- Connector --------------------------------------------------------------

#[pin_data]
pub(crate) struct EvdiConnector;

#[derive(Clone, Default)]
pub(crate) struct EvdiConnectorState;

impl connector::DriverConnectorState for EvdiConnectorState {
    type Connector = EvdiConnector;
}

#[vtable]
impl connector::DriverConnector for EvdiConnector {
    type Args = ();
    type Driver = EvdiDrmDriver;
    type State = EvdiConnectorState;

    fn new(_device: &drm::Device<Self::Driver>, _args: ()) -> impl PinInit<Self, Error> {
        try_pin_init!(EvdiConnector {})
    }

    /// Install the DLM-provided EDID when present, else advertise a fallback mode list so
    /// the connector stays usable before CONNECT.
    fn get_modes<'a>(
        connector: ConnectorGuard<'a, Self>,
        _guard: &ModeConfigGuard<'a, Self::Driver>,
    ) -> i32 {
        let data: &EvdiDrmData = connector.drm_dev();
        if let Some(blob) = data.cached_edid.lock().as_ref() {
            match connector.add_edid_modes(blob) {
                Ok(n) if n > 0 => return n,
                _ => {}
            }
        }
        let n = connector.add_modes_noedid((FALLBACK_W, FALLBACK_H));
        connector.set_preferred_mode((FALLBACK_W, FALLBACK_H));
        n
    }

    /// Report the display as connected after CONNECT supplies an EDID.
    ///
    /// This mirrors C evdi's hotplug sequencing and prevents userspace from
    /// configuring a fallback mode before the monitor modes are available.
    fn detect(connector: &connector::Connector<Self>, _force: bool) -> connector::Status {
        let data: &EvdiDrmData = connector.drm_dev();
        if data.cached_edid.lock().is_some() {
            connector::Status::Connected
        } else {
            connector::Status::Disconnected
        }
    }

    /// Reject modes the dock cannot move, using the `pixel_area_limit` /
    /// `pixel_per_second_limit` the DLM client supplied through CONNECT -- a port of the C
    /// evdi's `evdi_mode_valid`. Like C, the lowest-refresh mode of each resolution is kept
    /// even when it exceeds the pixel-rate budget (the device then runs it at a limited frame
    /// rate rather than losing the resolution entirely).
    fn mode_valid(connector: ConnectorModeValidation<'_, Self>, mode: &DisplayMode) -> ModeStatus {
        let data: &EvdiDrmData = connector.drm_dev();
        let pps = data.pixel_per_second_limit.load(Ordering::Relaxed);
        if pps == 0 {
            return ModeStatus::Ok;
        }
        let area = u32::from(mode.hdisplay()) * u32::from(mode.vdisplay());
        let vrefresh = mode.vrefresh().max(0) as u32;
        if area > data.pixel_area_limit.load(Ordering::Relaxed) {
            pr_debug!(
                "evdi: mode {}x{}@{} rejected: mode area too big\n",
                mode.hdisplay(),
                mode.vdisplay(),
                vrefresh
            );
            return ModeStatus::Bad;
        }
        if area.saturating_mul(vrefresh) <= pps {
            return ModeStatus::Ok;
        }
        if is_lowest_frequency_mode_of_resolution(&connector, mode) {
            pr_debug!(
                "evdi: mode {}x{}@{} exceeds pixel limit; frame rate may be reduced\n",
                mode.hdisplay(),
                mode.vdisplay(),
                vrefresh
            );
            return ModeStatus::Ok;
        }
        pr_debug!(
            "evdi: mode {}x{}@{} rejected: pixel rate too high\n",
            mode.hdisplay(),
            mode.vdisplay(),
            vrefresh
        );
        ModeStatus::Bad
    }
}

/// C evdi's `is_lowest_frequency_mode_of_given_resolution`: true if no probed mode of the
/// same resolution has a lower vrefresh than `mode`.
fn is_lowest_frequency_mode_of_resolution(
    connector: &ConnectorModeValidation<'_, EvdiConnector>,
    mode: &DisplayMode,
) -> bool {
    let (hdisplay, vdisplay) = (mode.hdisplay(), mode.vdisplay());
    let vrefresh = mode.vrefresh();
    !connector.any_mode(|candidate| {
        candidate.hdisplay() == hdisplay
            && candidate.vdisplay() == vdisplay
            && candidate.vrefresh() < vrefresh
    })
}

/// Report a cursor-plane commit to the client.
///
/// Shape changes carry a fresh GEM handle for the bitmap; movements carry only coordinates, and are
/// far more frequent. Both are dropped unless the client asked for cursor events, because a client
/// that composites the pointer itself must not also receive it out of band.
fn update_cursor(
    commit: PlaneAtomicCommit<'_, EvdiPlane>,
    data: &EvdiDrmData,
    dev: &EvdiDrmDevice,
) {
    if !data.cursor_events.load(Ordering::Relaxed) {
        return;
    }
    let (old, new) = commit.take_old_new_state();
    let Some(fb) = new.framebuffer::<EvdiDrmDriver>() else {
        crate::painter::notify_cursor_disabled(data);
        return;
    };

    // Re-send the bitmap only when it actually changed: the compositor commits the cursor plane on
    // every movement, and each CURSOR_SET costs the client a map, a copy and a handle close.
    let shape_changed = old
        .framebuffer::<EvdiDrmDriver>()
        .is_none_or(|previous| !core::ptr::eq(previous, fb));
    if shape_changed {
        if let (Ok(object), Ok(stride)) = (fb.object::<EvdiObject>(), fb.pitch(0)) {
            let shape = crate::painter::CursorShape {
                // Report no hotspot. The compositor has already applied it when it placed the
                // plane, so the destination rect below IS where the bitmap goes; reporting one as
                // well would have the client subtract it a second time and shift the pointer. This
                // matches vino, which drives the same sinks and positions by the bitmap corner.
                hot_x: 0,
                hot_y: 0,
                width: fb.width(),
                height: fb.height(),
                pixel_format: fb.format(),
                stride,
                buffer_length: stride.saturating_mul(fb.height()),
            };
            crate::painter::notify_cursor_set(data, dev, object, &shape);
        }
    }

    // Send the UNCLIPPED origin. When the cursor straddles an edge the helper clips the destination
    // and advances the source by the same amount, so the difference recovers where the bitmap's
    // corner actually is -- including off-screen. `destination` alone would pin the pointer to the
    // edge and make it drift as it left the screen.
    if let (Ok(Some(source)), Some(destination)) = (new.visible_source(), new.visible_destination())
    {
        let x = destination.x1 - source.x1;
        let y = destination.y1 - source.y1;
        crate::painter::notify_cursor_move(data, x, y);
    }
}
