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
        aref::ARef, new_condvar, new_mutex, new_spinlock, new_spinlock_irq, Arc, ArcBorrow,
        CondVar, CondVarTimeoutResult, Mutex, SpinLock, SpinLockIrq,
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
use crate::uapi;
use kernel::error::code::{ENODEV, ERESTARTSYS, ETIMEDOUT};

/// DDC/CI slave address on the virtual I2C bus (as used by monitor-control tools).
pub(crate) const DDCCI_ADDRESS: u16 = 0x37;
/// How long a DDC/CI transfer waits for the userspace client's reply.
const DDCCI_TIMEOUT_MS: u32 = 50;
/// Maximum DDC/CI payload carried in one event.
const DDCCI_BUFFER_SIZE: usize = 64;

static PRIMARY_FORMATS: [u32; 1] = [drm::fourcc::XRGB8888];

/// Fallback mode advertised before the DLM client delivers an EDID via CONNECT.
/// Multiplier applied to the DLM client's raw pixel-rate limit before `mode_valid` enforces it --
/// see [`EvdiDrmData::set_mode_limits`] for why the raw figure is an under-estimate. `1` restores
/// the previous behaviour exactly (and caps this dock at 1440p@120); raise it to offer higher-rate
/// modes to the compositor.
const BANDWIDTH_HEADROOM: u32 = 2;

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
    /// Painter state (connection status, cached EDID, cursor-events flag, dirty rects).
    #[pin]
    pub(crate) painter: Mutex<PainterState>,
    /// The current framebuffer and a bounded set of owned, validated swapchain mappings, so
    /// repeated flips and GRABPIX calls do not remap them.
    #[pin]
    scanout: Mutex<ScanoutState>,
    /// Slot for a DDC/CI reply from the client, guarding the request/response handshake between
    /// the I2C `master_xfer` (which waits) and the DDCCI_RESPONSE ioctl (which fills + notifies).
    #[pin]
    pub(crate) ddcci_resp: Mutex<Option<KVec<u8>>>,
    #[pin]
    pub(crate) ddcci_cv: CondVar,
    /// EDID and bandwidth limits delivered by the DLM client through CONNECT.
    #[pin]
    cached_edid: Mutex<Option<KVec<u8>>>,
    pixel_area_limit: AtomicU32,
    pixel_per_second_limit: AtomicU32,
    /// Active software-vblank timer and its cancellation handle.
    #[pin]
    vblank: SpinLock<Option<(Arc<VblankTimer>, ArcHrTimerHandle<VblankTimer>)>>,
    /// Set (once, never cleared) when the card is being torn down, so blocking paths bail out
    /// with `ENODEV` instead of stalling the teardown — e.g. a DDC/CI transfer waiting out its
    /// full timeout would otherwise hold up `i2c_del_adapter` on unbind.
    pub(crate) dying: AtomicBool,
}

impl EvdiDrmData {
    pub(crate) fn new() -> impl PinInit<Self, Error> {
        try_pin_init!(Self {
            events: EventChannel::new()?,
            painter <- new_mutex!(PainterState::new()),
            scanout <- new_mutex!(ScanoutState::new()),
            ddcci_resp <- new_mutex!(None),
            ddcci_cv <- new_condvar!(),
            cached_edid <- new_mutex!(None),
            pixel_area_limit: AtomicU32::new(0),
            pixel_per_second_limit: AtomicU32::new(0),
            vblank <- new_spinlock!(None),
            dying: AtomicBool::new(false),
        })
    }

    /// Begin card shutdown: mark the device dying and wake every in-kernel waiter so teardown
    /// (platform unbind → I2C adapter + DRM unregistration) cannot stall behind them. Called
    /// before the rest of the bound data drops; safe to call more than once.
    pub(crate) fn shutdown(&self) {
        self.dying.store(true, Ordering::Relaxed);
        self.ddcci_cv.notify_all();
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

    /// Send one DDC/CI message to the connected client and wait for its reply. A write buffer is
    /// copied into the event; a read buffer is filled from the reply, up to its length.
    pub(crate) fn ddcci_transfer(
        &self,
        addr: u16,
        flags: u16,
        buffer: kernel::i2c::MsgBuffer<'_>,
    ) -> Result {
        if self.dying.load(Ordering::Relaxed) {
            return Err(ENODEV);
        }
        let (is_read, len) = match &buffer {
            kernel::i2c::MsgBuffer::Write(buf) => (false, buf.len()),
            kernel::i2c::MsgBuffer::Read(buf) => (true, buf.len()),
        };
        let mut ev = uapi::DrmEvdiEventDdcciData {
            base: uapi::DrmEvent {
                type_: 0,
                length: 0,
            },
            buffer: [0u8; DDCCI_BUFFER_SIZE],
            buffer_length: len as u32,
            flags,
            address: addr,
        };
        if let kernel::i2c::MsgBuffer::Write(buf) = &buffer {
            let n = core::cmp::min(buf.len(), DDCCI_BUFFER_SIZE);
            ev.buffer[..n].copy_from_slice(&buf[..n]);
        }

        // Hold the slot lock across send+wait so the reply cannot be signalled before we wait
        // (no lost wakeup): `ddcci_respond` can only run once `wait_*` has released the lock.
        let mut slot = self.ddcci_resp.lock();
        *slot = None;
        self.events.send(ev)?;
        // Condvar waits can wake spuriously, so loop until the reply actually arrives,
        // carrying the remaining timeout. A wake landing exactly at expiry is reported as
        // `Timeout`, so re-check the slot once before giving up.
        let mut jiffies = kernel::time::msecs_to_jiffies(DDCCI_TIMEOUT_MS);
        while slot.is_none() {
            if self.dying.load(Ordering::Relaxed) {
                return Err(ENODEV);
            }
            match self.ddcci_cv.wait_interruptible_timeout(&mut slot, jiffies) {
                CondVarTimeoutResult::Woken { jiffies: remaining } => jiffies = remaining,
                CondVarTimeoutResult::Timeout => {
                    if slot.is_some() {
                        break;
                    }
                    return Err(ETIMEDOUT);
                }
                CondVarTimeoutResult::Signal { .. } => return Err(ERESTARTSYS),
            }
        }
        if is_read {
            if let kernel::i2c::MsgBuffer::Read(buf) = buffer {
                if let Some(resp) = slot.take() {
                    let m = core::cmp::min(resp.len(), buf.len());
                    buf[..m].copy_from_slice(&resp[..m]);
                }
            }
        }
        Ok(())
    }

    /// Store a DDC/CI reply from the client (DDCCI_RESPONSE ioctl) and wake the waiting transfer.
    pub(crate) fn ddcci_respond(&self, resp: KVec<u8>) {
        *self.ddcci_resp.lock() = Some(resp);
        self.ddcci_cv.notify_one();
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
    ///
    /// The client limit is a raw-pixel-rate proxy and does not account for transport compression.
    /// Apply the bounded [`BANDWIDTH_HEADROOM`] multiplier before mode validation. This controls
    /// which EDID modes are offered; the client remains responsible for rejecting a mode its
    /// transport cannot present.
    pub(crate) fn set_mode_limits(&self, pixel_area: u32, pixels_per_second: u32) {
        let pixels_per_second = pixels_per_second.saturating_mul(BANDWIDTH_HEADROOM);
        pr_info!(
            "evdi: mode limits -- area {pixel_area}, pixel rate {pixels_per_second}/s \
             (client supplied {}/s x{BANDWIDTH_HEADROOM} compression headroom)\n",
            pixels_per_second / BANDWIDTH_HEADROOM
        );
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

    kernel::declare_drm_ioctls_ext! {
        (EVDI_CONNECT, crate::uapi::DrmEvdiConnect,
            crate::uapi::DRM_IOCTL_EVDI_CONNECT, 0, crate::ioctl::connect),
        (EVDI_REQUEST_UPDATE, crate::uapi::DrmEvdiRequestUpdate,
            crate::uapi::DRM_IOCTL_EVDI_REQUEST_UPDATE, 0, crate::ioctl::request_update),
        (EVDI_GRABPIX, crate::uapi::DrmEvdiGrabpix,
            crate::uapi::DRM_IOCTL_EVDI_GRABPIX, 0, crate::ioctl::grabpix),
        (EVDI_DDCCI_RESPONSE, crate::uapi::DrmEvdiDdcciResponse,
            crate::uapi::DRM_IOCTL_EVDI_DDCCI_RESPONSE, 0, crate::ioctl::ddcci_response),
        (
            EVDI_ENABLE_CURSOR_EVENTS,
            crate::uapi::DrmEvdiEnableCursorEvents,
            crate::uapi::DRM_IOCTL_EVDI_ENABLE_CURSOR_EVENTS,
            0,
            crate::ioctl::enable_cursor_events
        ),
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
            (),
        )?;
        // Advertise FB_DAMAGE_CLIPS so the compositor reports which region
        // changed. Without it, GRABPIX must return the full plane.
        primary.enable_fb_damage_clips();
        // Keep cursor composition in the primary framebuffer for now. Creating a GEM handle for
        // a separate control client's cursor event may sleep, while a drm_file has no independent
        // refcount that a safe asynchronous channel can retain. Advertising no cursor plane avoids
        // reintroducing the erased raw-file lifetime protocol removed from EventChannel.
        let crtc_obj = crtc::UnregisteredCrtc::<EvdiCrtc>::new(
            dev,
            primary,
            None::<&plane::UnregisteredPlane<EvdiPlane>>,
            None,
            (),
        )?;
        let enc = encoder::UnregisteredEncoder::<EvdiEncoder>::new(
            dev,
            encoder::Type::Virtual,
            crtc_obj.mask(),
            0,
            None,
            (),
        )?;
        // Use DVI-I (matching the C evdi) rather than Virtual: `__drm_connector_init` skips
        // `drm_connector_attach_edid_property()` for VIRTUAL/WRITEBACK connectors, and without that
        // property `drm_edid_connector_update()` can't populate `edid_blob_ptr`, so
        // `drm_edid_connector_add_modes()` would return 0 modes for a perfectly valid EDID.
        let conn =
            connector::UnregisteredConnector::<EvdiConnector>::new(dev, connector::Type::DviI, ())?;
        conn.attach_encoder(&*enc)?;
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
/// [`HasHrTimer::start`], which re-queues the timer whether it is dead or still pending —
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
        // re-arms it. A concurrent re-arm racing this return is safe — hrtimer keeps a timer
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
        crate::painter::notify_dpms(data, dev, crate::painter::DPMS_ON);
        crate::painter::notify_mode_changed(
            data,
            dev,
            mode.hdisplay() as i32,
            mode.vdisplay() as i32,
            mode.vrefresh(),
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
        crate::painter::notify_dpms(data, dev, crate::painter::DPMS_OFF);
    }

    /// Arm the page-flip completion event to be sent by the next vblank tick, so userspace is paced
    /// to the refresh rate rather than signalled immediately.
    fn atomic_flush(commit: CrtcAtomicCommit<'_, Self>) {
        let crtc = commit.crtc();
        let mut new = commit.take_new_state();
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

// ---- Planes (primary + cursor) ----------------------------------------------
//
// The safe KMS layer allows a single `DriverPlane` type per driver, so one `EvdiPlane` type serves
// both the primary and cursor planes, told apart by `is_cursor` (set from the plane's `Args`).

#[pin_data]
pub(crate) struct EvdiPlane;

#[derive(Clone, Default)]
pub(crate) struct EvdiPlaneState;

impl plane::DriverPlaneState for EvdiPlaneState {
    type Plane = EvdiPlane;
}

#[vtable]
impl plane::DriverPlane for EvdiPlane {
    type Args = ();
    type Driver = EvdiDrmDriver;
    type State = EvdiPlaneState;

    fn new(_device: &drm::Device<Self::Driver>, _args: ()) -> impl PinInit<Self, Error> {
        try_pin_init!(EvdiPlane {})
    }

    /// A new framebuffer was flipped in.
    ///
    /// EVDI records the new scanout buffer and signals the DLM client to grab it (UPDATE_READY).
    fn atomic_update(commit: PlaneAtomicCommit<'_, Self>) {
        let plane = commit.plane();
        let dev = plane.drm_dev();
        let data: &EvdiDrmData = dev;

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
            crate::painter::notify_update_ready(data, dev);
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
        guard: &ModeConfigGuard<'a, Self::Driver>,
    ) -> i32 {
        let data: &EvdiDrmData = connector.drm_dev();
        if let Some(blob) = data.cached_edid.lock().as_ref() {
            match connector.add_edid_modes(blob) {
                Ok(n) if n > 0 => return n,
                _ => {}
            }
        }
        let _ = guard;
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
            pr_warn!(
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
            pr_warn!(
                "evdi: mode {}x{}@{} exceeds pixel limit; frame rate may be reduced\n",
                mode.hdisplay(),
                mode.vdisplay(),
                vrefresh
            );
            return ModeStatus::Ok;
        }
        pr_warn!(
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
