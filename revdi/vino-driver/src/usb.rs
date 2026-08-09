//! USB I/O for a DisplayLink DL3 dock.
//!
//! Backed by **rusb** (safe Rust bindings over the libusb-1.0 C library) so the
//! userspace tools (Chimera and the focused sniff utility) drive the dock through the same
//! libusb DisplayLinkManager itself uses*. libusb resolves by soname
//! `libusb-1.0.so.0`; run the binary with cwd `/opt/displaylink` (DLM's own
//! resolution) or `LD_LIBRARY_PATH=/opt/displaylink` / `LD_PRELOAD=/opt/
//! displaylink/libusb-1.0.so.0.3.0` to load DLM's exact bundled build rather than
//! the system libusb.
//!
//! Control transfers and
//! bulk **OUT** (EP 0x02 control, the profile's video endpoints) are synchronous
//! `libusb_bulk_transfer`/`libusb_control_transfer` with `flags=0`; the EP 0x84
//! dock-reply **IN** path is a *persistently posted* pool of asynchronous
//! `libusb_transfer`s reaped via `libusb_handle_events`, so a read URB is always
//! waiting in the kernel the instant the dock produces a reply (the "host wasn't
//! ready" window a sync `read_bulk`-per-call model could open never exists).
//! Video OUT frames use a bounded async submit/reap window (depth 8) matching
//! DLM's measured submit-ahead.

use crate::profile::{self, DockProfile, Identity, CLASS_VENDOR, MAX_HEADS, PROTOCOL_DL3, VID};
use crate::{EP_IN_CTRL, EP_OUT_CTRL};
use rusb::constants::{LIBUSB_TRANSFER_COMPLETED, LIBUSB_TRANSFER_TYPE_BULK};
use rusb::ffi::{
    libusb_alloc_transfer, libusb_cancel_transfer, libusb_context, libusb_device_handle,
    libusb_free_transfer, libusb_handle_events_timeout_completed, libusb_submit_transfer,
    libusb_transfer,
};
use rusb::{Context, DeviceHandle, UsbContext};
use std::collections::VecDeque;
use std::os::raw::{c_int, c_uint, c_void};
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Depth of the EP 0x84 read queue kept perpetually posted to the kernel.
const RECV_QUEUE_DEPTH: usize = 4;
/// Buffer size for each queued EP 0x84 read. Dock control/HDCP responses are
/// well under 1 KiB; 8 KiB is generous and avoids per-call size juggling.
const RECV_BUF_LEN: usize = 8192;
/// EP 0x83 status interrupt (on interface 2) that DLM polls.
const EP_INTR: u8 = 0x83;

/// bmRequestType bytes (direction | type | recipient).
const RT_VENDOR_IN_IFACE: u8 = 0xc1; // IN  | Vendor   | Interface
const RT_VENDOR_OUT_DEV: u8 = 0x40; //  OUT | Vendor   | Device
const RT_STD_IN_DEV: u8 = 0x80; //      IN  | Standard | Device
const RT_STD_OUT_IFACE: u8 = 0x01; //   OUT | Standard | Interface
const RT_STD_OUT_DEV: u8 = 0x00; //     OUT | Standard | Device
const RT_CLASS_OUT_DEV: u8 = 0x20; //   OUT | Class    | Device

/// Depth of the video OUT submit-ahead window (DLM keeps ~8 outstanding).
const PIPELINE_DEPTH: usize = 8;

pub struct Dock {
    /// EP 0x84 bulk IN — dock → host responses (persistently queued async).
    ///
    /// DECLARED FIRST on purpose so it DROPS FIRST. Rust drops struct fields in declaration order;
    /// `AsyncIn::drop` cancels its in-flight transfers and then pumps libusb events
    /// (`pump_events(self.ctx)`) to drain the completion callbacks before freeing the transfers --
    /// which requires the libusb context to still be alive. If `handle` (device close) and `ctx`
    /// (`libusb_exit`) dropped first, that pump/cancel would run against an already-exited context
    /// and segfault inside libusb (observed: `segfault at 50 in libusb-1.0.so`). Keep `ep84` above
    /// `handle`/`ctx`. See the 2026-07-12 teardown-crash fix.
    ep84: Mutex<AsyncIn>,
    handle: DeviceHandle<Context>,
    ctx: Context,
    pub timeout: Duration,
    /// Serialises host→dock bulk OUT (EP 0x02) — libusb sync bulk is thread-safe
    /// but we keep frame ordering deterministic.
    out_lock: Mutex<()>,
    /// Which dock this is, chosen from the identity descriptor. See [`profile`].
    profile: &'static DockProfile,
    /// What the dock said it is, or `None` if it would not answer.
    identity: Option<Identity>,
    /// Each connector's video bulk-OUT endpoint, or `None` where the device does not expose it.
    ///
    /// Indexed by connector, not by endpoint: Navarro multiplexes four connectors over two
    /// endpoints, so entries repeat. Taking the addresses from the profile rather than naming
    /// `0x08`/`0x0b` inline is what lets one build drive both generations.
    video: [Option<u8>; MAX_HEADS],
    /// How many connectors this device actually backs, which is what loops must use.
    connectors: u8,
    has_intr: bool,
}

#[derive(Debug)]
pub enum Error {
    DeviceNotFound,
    /// Transfer timed out.
    Timeout,
    /// Endpoint STALL / halt condition (libusb `Pipe`).
    Stall,
    /// Device went away mid-transfer.
    Disconnected,
    /// Any other libusb error (open, claim, config, transfer).
    Usb(rusb::Error),
    Decode,
    ShortRead(usize),
    /// The device accepted fewer bytes than were offered. libusb reports this as a successful
    /// transfer with a smaller count, so it has to be turned into an error deliberately or a
    /// truncated frame reaches the dock unnoticed.
    ShortWrite {
        wrote: usize,
        wanted: usize,
    },
}

impl From<rusb::Error> for Error {
    fn from(e: rusb::Error) -> Self {
        map_err(e)
    }
}

/// Normalise a libusb error into our `Error` so callers can match on the common
/// cases (`Timeout`, `Stall`, `Disconnected`) directly.
fn map_err(e: rusb::Error) -> Error {
    match e {
        rusb::Error::Timeout => Error::Timeout,
        rusb::Error::Pipe => Error::Stall,
        rusb::Error::NoDevice | rusb::Error::NotFound => Error::Disconnected,
        other => Error::Usb(other),
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::DeviceNotFound => write!(f, "DisplayLink D6000 (17e9:6006) not found"),
            Error::Timeout => write!(f, "USB transfer timed out"),
            Error::Stall => write!(f, "endpoint stalled (Pipe)"),
            // Keep the "NoDevice" token so existing string-based disconnect
            // checks in callers (`contains(\"NoDevice\")`) still fire.
            Error::Disconnected => write!(f, "device disconnected (NoDevice)"),
            Error::Usb(e) => write!(f, "USB error: {e}"),
            Error::ShortWrite { wrote, wanted } => {
                write!(f, "device accepted {wrote} of {wanted} bytes")
            }
            Error::Decode => write!(f, "frame decode failed"),
            Error::ShortRead(n) => write!(f, "short read: {n} bytes"),
        }
    }
}

impl std::error::Error for Error {}

// ---------------------------------------------------------------------------
// Persistent async IN pool for EP 0x84 (DLM's "always-posted read URB" model).
// ---------------------------------------------------------------------------

/// State shared between the Rust side and the libusb completion callback.
/// Referenced by raw pointer from `libusb_transfer::user_data`; kept alive in a
/// `Box` inside `AsyncIn` and only freed once every transfer is idle.
struct InShared {
    /// Completed dock replies, in arrival order.
    ready: Mutex<VecDeque<Vec<u8>>>,
    /// Transfers currently owned by libusb (submitted, callback not yet final).
    inflight: AtomicI32,
    /// Set during teardown so the callback stops re-arming.
    closing: AtomicBool,
}

/// libusb completion callback for an EP 0x84 IN transfer. Runs on whichever
/// thread is inside `libusb_handle_events`.
extern "system" fn ep84_complete(transfer: *mut libusb_transfer) {
    // SAFETY: `transfer` is a live transfer we allocated; `user_data` points at
    // the `InShared` boxed in the owning `AsyncIn`, which outlives all transfers.
    unsafe {
        let t = &*transfer;
        let shared = &*(t.user_data as *const InShared);
        if t.status == LIBUSB_TRANSFER_COMPLETED && !shared.closing.load(Ordering::Acquire) {
            let n = t.actual_length as usize;
            if n > 0 {
                let data = std::slice::from_raw_parts(t.buffer, n).to_vec();
                shared.ready.lock().unwrap().push_back(data);
            }
            // Re-arm to keep the queue perpetually posted (DLM model).
            if libusb_submit_transfer(transfer) != 0 {
                shared.inflight.fetch_sub(1, Ordering::AcqRel);
            }
        } else {
            // Cancelled, closing, device gone, or error: transfer is now idle.
            shared.inflight.fetch_sub(1, Ordering::AcqRel);
        }
    }
}

struct AsyncIn {
    ctx: *mut libusb_context,
    transfers: Vec<*mut libusb_transfer>,
    /// Stable heap backing for each transfer's buffer (kept alive until Drop).
    _buffers: Vec<Box<[u8]>>,
    shared: Box<InShared>,
}

// SAFETY: every access to the raw pointers goes through `Dock`'s `Mutex<AsyncIn>`;
// the pointers address libusb objects / heap buffers this `AsyncIn` owns.
unsafe impl Send for AsyncIn {}

impl AsyncIn {
    fn new(handle: *mut libusb_device_handle, ctx: *mut libusb_context, ep: u8) -> AsyncIn {
        let shared = Box::new(InShared {
            ready: Mutex::new(VecDeque::new()),
            inflight: AtomicI32::new(0),
            closing: AtomicBool::new(false),
        });
        let shared_ptr = &*shared as *const InShared as *mut c_void;
        let mut buffers: Vec<Box<[u8]>> = Vec::with_capacity(RECV_QUEUE_DEPTH);
        let mut transfers: Vec<*mut libusb_transfer> = Vec::with_capacity(RECV_QUEUE_DEPTH);
        for _ in 0..RECV_QUEUE_DEPTH {
            let mut buf = vec![0u8; RECV_BUF_LEN].into_boxed_slice();
            let bptr = buf.as_mut_ptr();
            // SAFETY: alloc a bulk transfer (0 iso packets) and fill its fields;
            // `libusb_fill_bulk_transfer` is a C inline, so we set them directly.
            let t = unsafe { libusb_alloc_transfer(0) };
            assert!(!t.is_null(), "libusb_alloc_transfer failed");
            unsafe {
                let tr = &mut *t;
                tr.dev_handle = handle;
                tr.endpoint = ep;
                tr.transfer_type = LIBUSB_TRANSFER_TYPE_BULK;
                tr.timeout = 0; // infinite: stays posted until data or cancel
                tr.buffer = bptr;
                tr.length = RECV_BUF_LEN as c_int;
                tr.callback = ep84_complete;
                tr.user_data = shared_ptr;
                tr.flags = 0;
                tr.num_iso_packets = 0;
            }
            buffers.push(buf);
            transfers.push(t);
        }
        let ai = AsyncIn {
            ctx,
            transfers,
            _buffers: buffers,
            shared,
        };
        for &t in &ai.transfers {
            // SAFETY: `t` is a filled, un-submitted transfer.
            if unsafe { libusb_submit_transfer(t) } == 0 {
                ai.shared.inflight.fetch_add(1, Ordering::AcqRel);
            }
        }
        ai
    }

    /// Read one dock reply, pumping libusb events until one lands or `timeout`.
    fn recv(&self, timeout: Duration) -> Result<Vec<u8>, Error> {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(b) = self.shared.ready.lock().unwrap().pop_front() {
                return Ok(b);
            }
            if self.shared.inflight.load(Ordering::Acquire) == 0 {
                // No posted URBs left (device gone at submit time) and nothing ready.
                return Err(Error::Disconnected);
            }
            let now = Instant::now();
            if now >= deadline {
                return Err(Error::Timeout);
            }
            let slice = (deadline - now).min(Duration::from_millis(20));
            pump_events(self.ctx, slice);
        }
    }
}

impl Drop for AsyncIn {
    fn drop(&mut self) {
        self.shared.closing.store(true, Ordering::Release);
        for &t in &self.transfers {
            // SAFETY: cancelling a live or already-idle transfer is safe
            // (returns NOT_FOUND if not in flight); ignore the result.
            unsafe { libusb_cancel_transfer(t) };
        }
        // Drain callbacks so no transfer is still owned by libusb before we free.
        let deadline = Instant::now() + Duration::from_millis(500);
        while self.shared.inflight.load(Ordering::Acquire) > 0 && Instant::now() < deadline {
            pump_events(self.ctx, Duration::from_millis(20));
        }
        for &t in &self.transfers {
            // SAFETY: transfer is idle (callback fired) — safe to free.
            unsafe { libusb_free_transfer(t) };
        }
    }
}

/// Run libusb's event handler for up to `slice`. Multiple threads may call this
/// concurrently; libusb serialises via its internal event lock.
fn pump_events(ctx: *mut libusb_context, slice: Duration) {
    let tv = libc::timeval {
        tv_sec: slice.as_secs() as libc::time_t,
        tv_usec: slice.subsec_micros() as libc::suseconds_t,
    };
    let mut completed: c_int = 0;
    // SAFETY: `ctx` is a live libusb context; `tv`/`completed` are stack locals.
    unsafe { libusb_handle_events_timeout_completed(ctx, &tv, &mut completed) };
}

fn status_to_err(status: c_int) -> Error {
    match status {
        rusb::constants::LIBUSB_TRANSFER_TIMED_OUT => Error::Timeout,
        rusb::constants::LIBUSB_TRANSFER_STALL => Error::Stall,
        rusb::constants::LIBUSB_TRANSFER_NO_DEVICE => Error::Disconnected,
        _ => Error::Usb(rusb::Error::Other),
    }
}

impl Dock {
    /// Open the first DisplayLink display function found, and place it by what it says it is.
    ///
    /// Devices are found by *function* -- vendor `17e9` with an interface of class `0xff`,
    /// subclass `0`, protocol `0x03` -- not by a list of product IDs, which can only ever
    /// describe the hardware somebody tested. The family then comes from the dock's own identity
    /// descriptor. This mirrors the in-kernel driver exactly; see `docs/adding-a-device.md`.
    pub fn open() -> Result<Self, Error> {
        let ctx = Context::new().map_err(map_err)?;

        let find_open = |ctx: &Context| -> Result<(DeviceHandle<Context>, u16), Error> {
            for dev in ctx.devices().map_err(map_err)?.iter() {
                let Ok(desc) = dev.device_descriptor() else {
                    continue;
                };
                if desc.vendor_id() != VID || !is_dl3_display_function(&dev) {
                    continue;
                }
                return Ok((dev.open().map_err(map_err)?, desc.product_id()));
            }
            Err(Error::DeviceNotFound)
        };

        let (handle, product) = find_open(&ctx)?;

        // Chimera owns the complete display function for the lifetime of this
        // handle. Detach stale interface drivers before selecting the known
        // D6000 configuration.
        for iface in 0u8..8 {
            let _ = handle.detach_kernel_driver(iface);
        }
        let _ = handle.set_active_configuration(1);

        // Configuration changes can rebind an interface, so detach once more
        // before claiming the control/video interface.
        let _ = handle.detach_kernel_driver(0);
        handle.claim_interface(0).map_err(map_err)?;
        let _ = handle.set_alternate_setting(0, 0);

        // What this hardware is, asked of the hardware: one standard GET_DESCRIPTOR, no session
        // and no crypto. A dock that names a family nobody here drives is declined rather than
        // guessed at, because the way a dock rejects a guess is to reset itself. A dock that
        // cannot be *asked* falls back to the product-ID quirk table, so a transient descriptor
        // failure does not cost a known dock its displays.
        let identity = read_identity(&handle);
        let profile = match identity.as_ref().and_then(Identity::family) {
            Some(family) => match profile::for_family(family) {
                Some(profile) => profile,
                None => {
                    let id = identity.as_ref().expect("family implies identity");
                    eprintln!("[usb] {id} is not a family this stack drives yet");
                    return Err(Error::DeviceNotFound);
                }
            },
            None => match profile::for_product(product) {
                Some(profile) => {
                    eprintln!("[usb] identity unreadable; using the quirk entry for {product:04x}");
                    profile
                }
                None => {
                    eprintln!("[usb] no identity descriptor and no quirk entry for {product:04x}");
                    return Err(Error::DeviceNotFound);
                }
            },
        };
        if let Some(id) = identity.as_ref() {
            eprintln!("[usb] {id} running firmware {}", id.version);
        }

        // Each connector's video endpoint, taken from the profile and checked against the
        // descriptor. A connector whose endpoint the device does not expose is dropped rather
        // than guessed at, and the connector count follows: a dock in a known family with fewer
        // outputs is driven with the outputs it has.
        let mut video = [None; MAX_HEADS];
        let mut connectors = 0u8;
        for (slot, addr) in video.iter_mut().zip(profile.video_eps) {
            if connectors >= profile.connectors || !has_endpoint(&handle, addr) {
                break;
            }
            *slot = Some(addr);
            connectors += 1;
        }
        if connectors == 0 {
            eprintln!("[usb] no video endpoint from {:#04x?}", profile.video_eps);
            return Err(Error::DeviceNotFound);
        }
        eprintln!(
            "[usb] matched profile \"{}\", {connectors} connector(s), video endpoints {:#04x?}",
            profile.name,
            &video[..usize::from(connectors)]
        );

        // EP 0x83 lives on interface 2. It is auxiliary status input, so a
        // failure to claim it does not invalidate the control/video endpoints.
        let _ = handle.detach_kernel_driver(2);
        let has_intr = match handle.claim_interface(2) {
            Ok(()) => has_endpoint(&handle, EP_INTR),
            Err(e) => {
                eprintln!("[usb] claim interface 2 for EP 0x83 failed: {e}");
                false
            }
        };

        // Clear endpoint state left by a previous userspace owner.
        let _ = handle.clear_halt(EP_OUT_CTRL);
        let _ = handle.clear_halt(EP_IN_CTRL);
        for (head, addr) in video.iter().enumerate() {
            let Some(addr) = addr else { continue };
            if let Err(e) = handle.clear_halt(*addr) {
                eprintln!("[usb] clear head {head} video halt (EP {addr:#04x}) failed: {e}");
            }
        }

        // Bring up the persistent async EP 0x84 IN pool.
        let ep84 = AsyncIn::new(handle.as_raw(), ctx.as_raw(), EP_IN_CTRL);

        Ok(Self {
            handle,
            ctx,
            timeout: Duration::from_millis(2000),
            ep84: Mutex::new(ep84),
            out_lock: Mutex::new(()),
            profile,
            identity,
            video,
            connectors,
            has_intr,
        })
    }

    /// Issue a vendor-specific IN control transfer (bmRequestType=0xc1).
    pub fn vendor_in(
        &self,
        request: u8,
        value: u16,
        index: u16,
        length: u16,
    ) -> Result<Vec<u8>, Error> {
        let mut buf = vec![0u8; length as usize];
        let n = self
            .handle
            .read_control(
                RT_VENDOR_IN_IFACE,
                request,
                value,
                index,
                &mut buf,
                self.timeout,
            )
            .map_err(map_err)?;
        buf.truncate(n);
        Ok(buf)
    }

    /// Issue a vendor-specific OUT control transfer (bmRequestType=0x40).
    pub fn vendor_out(
        &self,
        request: u8,
        value: u16,
        index: u16,
        data: &[u8],
    ) -> Result<(), Error> {
        self.handle
            .write_control(RT_VENDOR_OUT_DEV, request, value, index, data, self.timeout)
            .map(|_| ())
            .map_err(map_err)
    }

    /// Standard IN control transfer (e.g. GET_DESCRIPTOR), recipient=device.
    pub fn std_in(
        &self,
        request: u8,
        value: u16,
        index: u16,
        length: u16,
        timeout: Duration,
    ) -> Result<Vec<u8>, Error> {
        let mut buf = vec![0u8; length as usize];
        let n = self
            .handle
            .read_control(RT_STD_IN_DEV, request, value, index, &mut buf, timeout)
            .map_err(map_err)?;
        buf.truncate(n);
        Ok(buf)
    }

    /// Standard OUT control transfer with recipient=interface (e.g. SET_INTERFACE).
    pub fn std_out_iface(
        &self,
        request: u8,
        value: u16,
        index: u16,
        data: &[u8],
        timeout: Duration,
    ) -> Result<(), Error> {
        self.handle
            .write_control(RT_STD_OUT_IFACE, request, value, index, data, timeout)
            .map(|_| ())
            .map_err(map_err)
    }

    /// Class OUT control transfer with recipient=device. DLM issues
    /// `bmRequestType=0x20 bRequest=0x0c wValue=0/1` (no data) during early setup.
    pub fn class_out_device(
        &self,
        request: u8,
        value: u16,
        index: u16,
        data: &[u8],
    ) -> Result<(), Error> {
        self.handle
            .write_control(RT_CLASS_OUT_DEV, request, value, index, data, self.timeout)
            .map(|_| ())
            .map_err(map_err)
    }

    /// Claim an additional interface beyond the ones `open()` claims by default
    /// (0 for the control plane, 2 for the EP 0x83 status poll). DLM's libusb
    /// claims interface 1 (the DFU/app-specific interface) too, via its own
    /// `usbfs` open, and the in-kernel driver binds it as a real driver
    /// (confirmed live: `vino 2-1.1:1.1: bound D6000 interface 1`) -- needed
    /// before `std_out_iface`'s `SET_INTERFACE` on it will succeed.
    pub fn claim_interface(&self, iface: u8) -> Result<(), Error> {
        let _ = self.handle.detach_kernel_driver(iface);
        self.handle.claim_interface(iface).map_err(map_err)
    }

    /// Issue an empty SET (host→device) control transfer — DLM does this between
    /// vendor requests; function unknown but apparently benign.
    pub fn ctrl_noop(&self) -> Result<(), Error> {
        let _ = self
            .handle
            .write_control(RT_STD_OUT_DEV, 0, 0, 0, &[], self.timeout);
        Ok(())
    }

    /// Submit one control-plane transfer through libusb's asynchronous bulk
    /// path. The transfer itself has no libusb timeout; `completion_deadline`
    /// bounds the caller wait and triggers a synchronous cancellation/drain.
    pub fn write_ctrl_raw_async(
        &self,
        bytes: &[u8],
        completion_deadline: Duration,
    ) -> Result<usize, Error> {
        struct OutShared {
            done: AtomicBool,
            status: AtomicI32,
            actual_length: AtomicI32,
        }
        extern "system" fn out_complete(transfer: *mut libusb_transfer) {
            // SAFETY: `transfer` is the one transfer this call owns; `user_data`
            // points at the `OutShared` kept alive on this function's stack
            // until the transfer is known idle (join loop below).
            unsafe {
                let t = &*transfer;
                let shared = &*(t.user_data as *const OutShared);
                shared.status.store(t.status, Ordering::Release);
                shared
                    .actual_length
                    .store(t.actual_length, Ordering::Release);
                shared.done.store(true, Ordering::Release);
            }
        }

        let _g = self.out_lock.lock().unwrap();
        let shared = Box::new(OutShared {
            done: AtomicBool::new(false),
            status: AtomicI32::new(-1),
            actual_length: AtomicI32::new(0),
        });
        let shared_ptr = &*shared as *const OutShared as *mut c_void;

        // SAFETY: alloc a bulk transfer (0 iso packets); fields set directly,
        // mirroring `AsyncIn::new`'s pattern for the IN side.
        let t = unsafe { libusb_alloc_transfer(0) };
        assert!(!t.is_null(), "libusb_alloc_transfer failed");
        // `bytes` outlives the transfer (freed/cancelled before this fn
        // returns), so casting away constness for the C struct is sound: the
        // OUT direction never has libusb write back into this buffer.
        unsafe {
            let tr = &mut *t;
            tr.dev_handle = self.handle.as_raw();
            tr.endpoint = EP_OUT_CTRL;
            tr.transfer_type = LIBUSB_TRANSFER_TYPE_BULK;
            tr.timeout = 0; // matches DLM exactly -- see doc comment above
            tr.buffer = bytes.as_ptr() as *mut u8;
            tr.length = bytes.len() as c_int;
            tr.callback = out_complete;
            tr.user_data = shared_ptr;
            tr.flags = 0;
            tr.num_iso_packets = 0;
        }
        // SAFETY: `t` is a filled, un-submitted transfer.
        if unsafe { libusb_submit_transfer(t) } != 0 {
            unsafe { libusb_free_transfer(t) };
            return Err(Error::Usb(rusb::Error::Io));
        }

        let deadline = Instant::now() + completion_deadline;
        while !shared.done.load(Ordering::Acquire) {
            if Instant::now() >= deadline {
                // `shared` is stack-owned and the callback writes through a raw pointer to it,
                // so the cancellation completion must be drained before either it or the
                // transfer is released. This wait is deliberately unbounded: giving up early and
                // freeing anyway is a use-after-free the moment the callback does fire, and a
                // completion that never arrives means the device is gone and libusb will report
                // it.
                unsafe { libusb_cancel_transfer(t) };
                while !shared.done.load(Ordering::Acquire) {
                    pump_events(self.ctx.as_raw(), Duration::from_millis(20));
                }
                unsafe { libusb_free_transfer(t) };
                return Err(Error::Timeout);
            }
            pump_events(self.ctx.as_raw(), Duration::from_millis(20));
        }
        let status = shared.status.load(Ordering::Acquire);
        let actual = shared.actual_length.load(Ordering::Acquire) as usize;
        unsafe { libusb_free_transfer(t) };
        if status != LIBUSB_TRANSFER_COMPLETED {
            return Err(status_to_err(status));
        }
        if actual != bytes.len() {
            return Err(Error::ShortWrite {
                wrote: actual,
                wanted: bytes.len(),
            });
        }
        Ok(actual)
    }

    /// [`write_ctrl_raw_async`] with the standard two-second completion deadline.
    pub fn write_ctrl_dlm(&self, bytes: &[u8]) -> Result<usize, Error> {
        self.write_ctrl_raw_async(bytes, Duration::from_secs(2))
    }

    /// Write raw bytes synchronously to the control-plane bulk OUT endpoint.
    pub fn write_ctrl_raw(&self, bytes: &[u8]) -> Result<usize, Error> {
        let _g = self.out_lock.lock().unwrap();
        self.handle
            .write_bulk(EP_OUT_CTRL, bytes, self.timeout)
            .map_err(map_err)
    }

    /// This dock's profile, chosen from its identity descriptor.
    pub fn profile(&self) -> &'static DockProfile {
        self.profile
    }

    /// What the dock said it is, or `None` if it would not answer.
    pub fn identity(&self) -> Option<&Identity> {
        self.identity.as_ref()
    }

    /// How many connectors this device actually backs. Loops must use this, never [`MAX_HEADS`].
    pub fn connectors(&self) -> usize {
        usize::from(self.connectors)
    }

    /// A connector's video bulk-OUT endpoint.
    fn video_ep(&self, head: usize) -> Result<u8, Error> {
        self.video
            .get(head)
            .copied()
            .flatten()
            .ok_or(Error::Disconnected)
    }

    /// Write a video frame to one connector. Returns bytes written.
    ///
    /// Takes the OUT lock like every other host->dock write: a video frame interleaved with a
    /// control message is a torn frame, and on a dock whose video shares the control endpoint it
    /// is a corrupted control stream as well.
    pub fn write_video(&self, head: usize, bytes: &[u8]) -> Result<usize, Error> {
        let ep = self.video_ep(head)?;
        let _g = self.out_lock.lock().unwrap();
        let written = self
            .handle
            .write_bulk(ep, bytes, self.timeout)
            .map_err(map_err)?;
        // A short write is a truncated frame, not a success. libusb reports the byte count and
        // returns Ok, so nothing else notices.
        if written == bytes.len() {
            Ok(written)
        } else {
            Err(Error::ShortWrite {
                wrote: written,
                wanted: bytes.len(),
            })
        }
    }

    /// Pipeline a whole frame's chunks on the head-0 video EP (0x08): submit up
    /// to [`PIPELINE_DEPTH`] transfers to the URB ring, reap completions, and
    /// submit-ahead — matching DLM's async libusb submission so the dock's frame
    /// assembler never sees a host-side stall mid-frame. Returns total bytes.
    pub fn write_video_frame(&self, head: usize, chunks: &[Vec<u8>]) -> Result<usize, Error> {
        self.pipeline_out(self.video_ep(head)?, chunks)
    }

    /// Bounded async submit/reap of `chunks` on a bulk OUT endpoint. Keeps at
    /// most [`PIPELINE_DEPTH`] transfers outstanding (DLM's measured depth),
    /// submitting the next chunk the instant one completes.
    fn pipeline_out(&self, ep: u8, chunks: &[Vec<u8>]) -> Result<usize, Error> {
        if chunks.is_empty() {
            return Ok(0);
        }
        let _g = self.out_lock.lock().unwrap();
        let handle = self.handle.as_raw();
        let ctx = self.ctx.as_raw();
        let depth = PIPELINE_DEPTH.min(chunks.len());
        // Completions land here (transfer ptr, status, actual_length). Callbacks
        // fire on this thread inside `pump_events`, so no cross-thread hazard.
        let completions: Box<Mutex<VecDeque<(*mut libusb_transfer, c_int, c_int)>>> =
            Box::new(Mutex::new(VecDeque::new()));
        let cptr = &*completions as *const Mutex<VecDeque<(*mut libusb_transfer, c_int, c_int)>>
            as *mut c_void;

        let transfers: Vec<*mut libusb_transfer> = (0..depth)
            .map(|_| unsafe { libusb_alloc_transfer(0) })
            .collect();
        assert!(
            transfers.iter().all(|t| !t.is_null()),
            "libusb_alloc_transfer failed"
        );
        let timeout_ms = self.timeout.as_millis() as c_uint;

        // SAFETY: fill+submit a transfer for chunk `ci` on transfer object `t`.
        let submit = |t: *mut libusb_transfer, ci: usize| -> c_int {
            unsafe {
                let tr = &mut *t;
                tr.dev_handle = handle;
                tr.endpoint = ep;
                tr.transfer_type = LIBUSB_TRANSFER_TYPE_BULK;
                tr.timeout = timeout_ms;
                tr.buffer = chunks[ci].as_ptr() as *mut u8;
                tr.length = chunks[ci].len() as c_int;
                tr.callback = out_complete;
                tr.user_data = cptr;
                tr.flags = 0;
                tr.num_iso_packets = 0;
                libusb_submit_transfer(t)
            }
        };

        let mut next = 0usize;
        let mut submitted = 0usize;
        let mut reaped = 0usize;
        let mut total = 0usize;
        let mut err: Option<Error> = None;

        // Prime the window.
        for &t in &transfers {
            if next >= chunks.len() {
                break;
            }
            if submit(t, next) == 0 {
                submitted += 1;
                next += 1;
            } else {
                err = Some(Error::Disconnected);
                break;
            }
        }

        // Reap and submit-ahead until every submitted transfer has completed.
        let deadline = Instant::now() + self.timeout + Duration::from_millis(500);
        while reaped < submitted {
            let done = completions.lock().unwrap().pop_front();
            let (t, status, actual) = match done {
                Some(x) => x,
                None => {
                    if Instant::now() >= deadline {
                        err = err.or(Some(Error::Timeout));
                        break;
                    }
                    pump_events(ctx, Duration::from_millis(20));
                    continue;
                }
            };
            reaped += 1;
            if status != LIBUSB_TRANSFER_COMPLETED {
                if err.is_none() {
                    err = Some(status_to_err(status));
                }
                continue;
            }
            // A completed transfer that moved fewer bytes than it was given is a truncated
            // record on the wire; the dock's frame assembler will not tell us about it.
            let wanted = unsafe { (*t).length.max(0) as usize };
            let wrote = actual.max(0) as usize;
            if wrote != wanted {
                err = err.or(Some(Error::ShortWrite { wrote, wanted }));
                continue;
            }
            total += wrote;
            // Submit-ahead: reuse the just-completed transfer for the next chunk.
            if err.is_none() && next < chunks.len() {
                if submit(t, next) == 0 {
                    submitted += 1;
                    next += 1;
                } else if err.is_none() {
                    err = Some(Error::Disconnected);
                }
            }
        }

        // Teardown: cancel anything still outstanding and drain its callback so
        // no transfer is owned by libusb when we free.
        for &t in &transfers {
            unsafe { libusb_cancel_transfer(t) };
        }
        // Unbounded for the same reason as the single-transfer path: `completions` is a local,
        // and the callbacks write into it. Freeing the transfers while one can still fire is a
        // use-after-free, so every submitted transfer has to be accounted for first.
        while reaped < submitted {
            if completions.lock().unwrap().pop_front().is_some() {
                reaped += 1;
            } else {
                pump_events(ctx, Duration::from_millis(20));
            }
        }
        for &t in &transfers {
            unsafe { libusb_free_transfer(t) };
        }

        match err {
            Some(e) => Err(e),
            None => Ok(total),
        }
    }

    /// Clear the halt/stall condition on one connector's video endpoint.
    pub fn clear_video_halt(&self, head: usize) -> Result<(), Error> {
        self.handle
            .clear_halt(self.video_ep(head)?)
            .map_err(map_err)
    }

    /// Read one EP 0x83 interrupt event into `buf`. Returns the byte count
    /// (0 on timeout — matching the old poller's "no event" handling).
    pub fn read_intr(&self, buf: &mut [u8], timeout: Duration) -> Result<usize, Error> {
        if !self.has_intr {
            return Err(Error::Disconnected);
        }
        match self.handle.read_interrupt(EP_INTR, buf, timeout) {
            Ok(n) => Ok(n),
            Err(rusb::Error::Timeout) => Ok(0), // no event
            Err(e) => Err(map_err(e)),
        }
    }

    /// Read one frame from EP 0x84 using the persistent read queue. Uses the
    /// default timeout.
    pub fn recv_frame_raw(&self, max_len: usize) -> Result<Vec<u8>, Error> {
        self.recv_frame_raw_timeout(max_len, self.timeout)
    }

    /// Read one frame from EP 0x84 with an explicit timeout. The async pool keeps
    /// `RECV_QUEUE_DEPTH` URBs posted so the dock always has a waiting buffer.
    pub fn recv_frame_raw_timeout(
        &self,
        _max_len: usize,
        timeout: Duration,
    ) -> Result<Vec<u8>, Error> {
        let ep = self.ep84.lock().unwrap();
        ep.recv(timeout)
    }
}

/// libusb completion callback for a video OUT transfer (see [`Dock::pipeline_out`]).
extern "system" fn out_complete(transfer: *mut libusb_transfer) {
    // SAFETY: `user_data` points at the `completions` queue boxed on the stack of
    // the `pipeline_out` call that submitted this transfer; that call does not
    // return until every transfer it submitted has been reaped or cancelled.
    unsafe {
        let t = &*transfer;
        let q = &*(t.user_data as *const Mutex<VecDeque<(*mut libusb_transfer, c_int, c_int)>>);
        q.lock()
            .unwrap()
            .push_back((transfer, t.status, t.actual_length));
    }
}

/// Does the active configuration expose an endpoint with address `addr`?
/// Whether this device exposes a DL3 display function.
///
/// Class `0xff` / subclass `0` / protocol `0x03`, which is what DisplayLink's own udev rules key
/// on. Protocol `0x00` is the older `udl` hardware and is excluded for free.
fn is_dl3_display_function(dev: &rusb::Device<Context>) -> bool {
    let Ok(config) = dev.active_config_descriptor() else {
        return false;
    };
    config.interfaces().flat_map(|i| i.descriptors()).any(|d| {
        d.class_code() == CLASS_VENDOR
            && d.sub_class_code() == 0
            && d.protocol_code() == PROTOCOL_DL3
    })
}

/// Read the dock's identity by walking its configuration descriptor.
///
/// The blob is a vendor descriptor inside the configuration, not a separately addressable one, so
/// the whole configuration is fetched and walked.
fn read_identity(handle: &DeviceHandle<Context>) -> Option<Identity> {
    const DESCRIPTOR_CONFIG: u16 = 0x02 << 8;
    const CONFIG_DESCRIPTOR_MAX: usize = 1024;
    let timeout = Duration::from_millis(1000);

    let mut head = [0u8; 9];
    handle
        .read_control(
            RT_STD_IN_DEV,
            0x06,
            DESCRIPTOR_CONFIG,
            0,
            &mut head,
            timeout,
        )
        .ok()?;
    let total = usize::from(u16::from_le_bytes([head[2], head[3]]));
    if total < head.len() || total > CONFIG_DESCRIPTOR_MAX {
        return None;
    }
    let mut all = vec![0u8; total];
    handle
        .read_control(RT_STD_IN_DEV, 0x06, DESCRIPTOR_CONFIG, 0, &mut all, timeout)
        .ok()?;
    Identity::parse(&all)
}

fn has_endpoint(handle: &DeviceHandle<Context>, addr: u8) -> bool {
    let dev = handle.device();
    let Ok(config) = dev.active_config_descriptor() else {
        return false;
    };
    config
        .interfaces()
        .flat_map(|i| i.descriptors())
        .flat_map(|d| d.endpoint_descriptors())
        .any(|e| e.address() == addr)
}
