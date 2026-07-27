// SPDX-License-Identifier: LGPL-2.1-or-later
// Copyright (c) 2025 revdi contributors
//
// librevdi — a drop-in, ABI-compatible reimplementation of DisplayLink's
// `libevdi` in Rust. It exposes the exact C ABI declared in `evdi_lib.h`
// (installed as `libevdi.so.1`) so the closed-source DisplayLinkManager loads
// it unchanged, and drives the companion Rust EVDI kernel module through the
// `evdi_drm.h` ioctl interface.
//
// The behaviour is a faithful port of the upstream LGPL `library/evdi_lib.c`
// and `library/evdi_procfs.c`; comments below note where it maps to the C.

// The public types deliberately keep libevdi's C names (snake_case, SCREAMING
// enum variants) so this file reads against evdi_lib.h one-to-one.
#![allow(non_camel_case_types)]

mod ffi;
pub mod safe;
mod uapi;

use core::ffi::{c_char, c_int, c_uint, c_void};
use std::ffi::{CStr, CString};
use std::fs;
use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;

// ---------------------------------------------------------------------------
// Versioning (mirrors evdi_lib.h / evdi_lib.c compatibility constants)
// ---------------------------------------------------------------------------

const LIBEVDI_VERSION_MAJOR: c_int = 1;
const LIBEVDI_VERSION_MINOR: c_int = 15;
const LIBEVDI_VERSION_PATCH: c_int = 0;

const EVDI_MODULE_COMPATIBILITY_VERSION_MAJOR: c_int = 1;
const EVDI_MODULE_COMPATIBILITY_VERSION_MINOR: c_int = 9;

const MAX_DIRTS: usize = 16;
const EVDI_USAGE_LEN: usize = 64;
const INVALID_DEVICE_INDEX: c_int = -1;

// ---------------------------------------------------------------------------
// Public C ABI types (byte-for-byte mirror of evdi_lib.h)
// ---------------------------------------------------------------------------

#[repr(C)]
pub struct evdi_lib_version {
    pub version_major: c_int,
    pub version_minor: c_int,
    pub version_patchlevel: c_int,
}

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum evdi_device_status {
    AVAILABLE = 0,
    UNRECOGNIZED = 1,
    NOT_PRESENT = 2,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct evdi_rect {
    pub x1: c_int,
    pub y1: c_int,
    pub x2: c_int,
    pub y2: c_int,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct evdi_mode {
    pub width: c_int,
    pub height: c_int,
    pub refresh_rate: c_int,
    pub bits_per_pixel: c_int,
    pub pixel_format: c_uint,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct evdi_buffer {
    pub id: c_int,
    pub buffer: *mut c_void,
    pub width: c_int,
    pub height: c_int,
    pub stride: c_int,
    pub rects: *mut evdi_rect,
    pub rect_count: c_int,
}

#[repr(C)]
pub struct evdi_cursor_set {
    pub hot_x: i32,
    pub hot_y: i32,
    pub width: u32,
    pub height: u32,
    pub enabled: u8,
    pub buffer_length: u32,
    pub buffer: *mut u32,
    pub pixel_format: u32,
    pub stride: u32,
}

#[repr(C)]
pub struct evdi_cursor_move {
    pub x: i32,
    pub y: i32,
}

#[repr(C)]
pub struct evdi_ddcci_data {
    pub address: u16,
    pub flags: u16,
    pub buffer_length: u32,
    pub buffer: *mut u8,
}

#[repr(C)]
pub struct evdi_event_context {
    pub dpms_handler: Option<extern "C" fn(dpms_mode: c_int, user_data: *mut c_void)>,
    pub mode_changed_handler: Option<extern "C" fn(mode: evdi_mode, user_data: *mut c_void)>,
    pub update_ready_handler:
        Option<extern "C" fn(buffer_to_be_updated: c_int, user_data: *mut c_void)>,
    pub crtc_state_handler: Option<extern "C" fn(state: c_int, user_data: *mut c_void)>,
    pub cursor_set_handler:
        Option<extern "C" fn(cursor_set: evdi_cursor_set, user_data: *mut c_void)>,
    pub cursor_move_handler:
        Option<extern "C" fn(cursor_move: evdi_cursor_move, user_data: *mut c_void)>,
    pub ddcci_data_handler:
        Option<extern "C" fn(ddcci_data: evdi_ddcci_data, user_data: *mut c_void)>,
    pub user_data: *mut c_void,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct evdi_logging {
    pub function: Option<unsafe extern "C" fn(user_data: *mut c_void, fmt: *const c_char, ...)>,
    pub user_data: *mut c_void,
}

/// Opaque device context (`struct evdi_device_context`); `evdi_handle` points to this.
pub struct evdi_device_context {
    fd: c_int,
    buffer_to_update: c_int,
    frame_buffers: Vec<evdi_buffer>,
    device_index: c_int,
}

pub type evdi_handle = *mut evdi_device_context;

// ---------------------------------------------------------------------------
// Process-global state (mirrors C `card_usage[]` and `g_evdi_logging`)
// ---------------------------------------------------------------------------

struct Globals {
    /// Whether card N is currently held open by *this* process, so
    /// `find_unused_card_for` skips it (mirrors the C `card_usage[]` array).
    card_in_use: [bool; EVDI_USAGE_LEN],
    logging: evdi_logging,
}

// SAFETY: the only non-Send members are the raw pointer / fn pointer inside
// `evdi_logging`; access is always serialized through the `Mutex`, and the
// values are opaque cookies the library never dereferences except when invoking
// the client's own callback.
unsafe impl Send for Globals {}

static GLOBALS: Mutex<Globals> = Mutex::new(Globals {
    card_in_use: [false; EVDI_USAGE_LEN],
    logging: evdi_logging {
        function: None,
        user_data: core::ptr::null_mut(),
    },
});

/// Emit a diagnostic line, routing through the client-supplied logging callback
/// if one was registered (via `evdi_set_logging`), else to stdout — matching the
/// C `evdi_log` macro.
fn evdi_log(msg: &str) {
    let logging = {
        let g = GLOBALS.lock().unwrap();
        g.logging
    };
    match logging.function {
        Some(f) => {
            let cmsg = CString::new(msg).unwrap_or_default();
            // Call the client's printf-style callback with a literal "%s" so any
            // '%' in our message is treated as data, not a format specifier.
            // SAFETY: `f` is a client-provided C variadic callback; we pass a
            // valid NUL-terminated format and one matching `const char *` arg.
            unsafe {
                f(logging.user_data, c"%s".as_ptr(), cmsg.as_ptr());
            }
        }
        None => println!("[librevdi] {msg}"),
    }
}

macro_rules! evdi_log {
    ($($arg:tt)*) => { evdi_log(&format!($($arg)*)) };
}

// ---------------------------------------------------------------------------
// Low-level DRM helpers (port of the C static helpers)
// ---------------------------------------------------------------------------

/// `ioctl` wrapper that logs on failure, mirroring the C `do_ioctl`.
///
/// # Safety
/// `arg` must point to a valid argument object matching `request`.
unsafe fn do_ioctl(fd: c_int, request: core::ffi::c_ulong, arg: *mut c_void, msg: &str) -> c_int {
    let err = ffi::drm_ioctl(fd, request, arg);
    if err < 0 {
        evdi_log!("Ioctl {msg} error (errno {})", ffi::errno());
    }
    err
}

fn drm_auth_magic(fd: c_int, magic: c_uint) -> c_int {
    let mut auth = ffi::DrmAuth { magic };
    // SAFETY: `auth` is a valid DrmAuth for the AUTH_MAGIC ioctl.
    let ret = unsafe {
        ffi::drm_ioctl(
            fd,
            ffi::drm_ioctl_auth_magic(),
            (&mut auth as *mut ffi::DrmAuth).cast(),
        )
    };
    if ret != 0 {
        -ffi::errno()
    } else {
        0
    }
}

/// Detect DRM-master by attempting to authenticate magic 0: a master fd fails
/// with `EINVAL`, a non-master with `EACCES`. Mirrors the C `drm_is_master`.
fn drm_is_master(fd: c_int) -> bool {
    drm_auth_magic(fd, 0) != -ffi::EACCES
}

fn path_exists(path: &str) -> bool {
    Path::new(path).exists()
}

fn device_exists(device: c_int) -> bool {
    path_exists(&format!("/dev/dri/card{device}"))
}

/// Does `link` resolve to a path containing `substr`? (`does_path_links_to`)
fn does_path_link_to(link: &Path, substr: &str) -> bool {
    match fs::read_link(link) {
        Ok(target) => target.to_string_lossy().contains(substr),
        Err(_) => false,
    }
}

fn process_opened_files(pid: &str, device_file_path: &str) -> bool {
    let dir = format!("/proc/{pid}/fd");
    let Ok(entries) = fs::read_dir(&dir) else {
        return false;
    };
    for e in entries.flatten() {
        if does_path_link_to(&e.path(), device_file_path) {
            return true;
        }
    }
    false
}

fn process_opened_device(pid: &str, device_file_path: &str) -> bool {
    match fs::read_to_string(format!("/proc/{pid}/maps")) {
        Ok(maps) => maps.contains(device_file_path),
        Err(_) => false,
    }
}

/// Is any *other* process holding `device_file_path` open? (`device_has_master`)
fn device_has_master(device_file_path: &str) -> bool {
    let myself = std::process::id();
    let Ok(entries) = fs::read_dir("/proc") else {
        return false;
    };
    for e in entries.flatten() {
        let name = e.file_name();
        let name = name.to_string_lossy();
        let Ok(pid) = name.parse::<u32>() else {
            continue;
        };
        if pid == myself {
            continue;
        }
        if process_opened_files(&name, device_file_path)
            || process_opened_device(&name, device_file_path)
        {
            return true;
        }
    }
    false
}

fn wait_for_master(device_path: &str) {
    // 5 s total, polled every 100 ms (matches the C constants).
    let mut cnt = 50;
    while !device_has_master(device_path) && cnt > 0 {
        std::thread::sleep(Duration::from_millis(100));
        cnt -= 1;
    }
    if !device_has_master(device_path) {
        evdi_log!("Wait for master timed out");
    }
}

/// Open `device_path` and ensure we do not hold DRM-master on it, dropping it if
/// necessary. Returns the fd, or `-1`. (`open_as_slave`)
fn open_as_slave(device_path: &str) -> c_int {
    let Ok(cpath) = CString::new(device_path) else {
        return -1;
    };
    // SAFETY: `cpath` is a valid NUL-terminated path.
    let fd = unsafe { ffi::open(cpath.as_ptr(), ffi::O_RDWR) };
    if fd < 0 {
        return -1;
    }

    if drm_is_master(fd) {
        evdi_log!("Process has master on {device_path}");
        // SAFETY: fd is open; DROP_MASTER takes no argument.
        let err =
            unsafe { ffi::drm_ioctl(fd, ffi::drm_ioctl_drop_master(), core::ptr::null_mut()) };
        if err < 0 || drm_is_master(fd) {
            evdi_log!("Drop master on {device_path} failed");
            // SAFETY: fd is a valid descriptor we opened above.
            unsafe { ffi::close(fd) };
            return -1;
        }
    }

    evdi_log!("Opened {device_path} as slave drm device");
    fd
}

fn wait_for_device(device_path: &str) -> c_int {
    let mut cnt = 50;
    loop {
        let fd = open_as_slave(device_path);
        if fd >= 0 {
            return fd;
        }
        if cnt == 0 {
            evdi_log!("Failed to open a device");
            return fd;
        }
        std::thread::sleep(Duration::from_millis(100));
        cnt -= 1;
    }
}

fn open_device(device: c_int) -> c_int {
    let dev = format!("/dev/dri/card{device}");

    if xorg_running() {
        wait_for_master(&dev);
    }

    let fd = wait_for_device(&dev);
    if fd >= 0 {
        // SAFETY: fd is open; DROP_MASTER takes no argument.
        let err =
            unsafe { ffi::drm_ioctl(fd, ffi::drm_ioctl_drop_master(), core::ptr::null_mut()) };
        if err == 0 {
            evdi_log!("Dropped master on {dev}");
        }
    }
    fd
}

/// Append `buffer` to `/sys/devices/evdi/add`; returns bytes written (0 on error).
fn write_add_device(buffer: &[u8]) -> usize {
    match fs::write("/sys/devices/evdi/add", buffer) {
        Ok(()) => buffer.len(),
        Err(_) => 0,
    }
}

/// First `cardN` index under `dir`, or -1 — the C `get_drm_device_index` returns on the first
/// match (a platform device's DRM directory only ever holds one card).
fn get_drm_device_index(dir: &str) -> c_int {
    let Ok(entries) = fs::read_dir(dir) else {
        evdi_log!("Failed to open dir {dir}");
        return INVALID_DEVICE_INDEX;
    };
    for e in entries.flatten() {
        let name = e.file_name();
        let name = name.to_string_lossy();
        if let Some(rest) = name.strip_prefix("card") {
            if let Ok(n) = rest.parse::<c_int>() {
                return n;
            }
        }
    }
    INVALID_DEVICE_INDEX
}

/// Does the evdi platform device at `evdi_path` hang off `parent_device`?
/// With `parent_device == None`, matches devices that have *no* `device` link
/// (i.e. the generic, non-USB-attached cards). (`is_correct_parent_device`)
fn is_correct_parent_device(evdi_path: &str, parent_device: Option<&[u8]>) -> bool {
    let link_path = format!("{evdi_path}/device");

    let Some(parent) = parent_device else {
        return !Path::new(&link_path).exists();
    };

    let Ok(target) = fs::read_link(&link_path) else {
        return false;
    };
    let Some(token) = target.file_name() else {
        return false;
    };
    if parent.len() < 2 {
        return false;
    }
    token.as_encoded_bytes() == parent
}

/// Find a card whose parent matches (or the first free generic card), not yet in
/// use by this process. (`find_unused_card_for`)
fn find_unused_card_for(parent_device: Option<&[u8]>) -> c_int {
    let root = "/sys/bus/platform/devices";
    let Ok(entries) = fs::read_dir(root) else {
        evdi_log!("Failed to open dir {root}");
        return INVALID_DEVICE_INDEX;
    };

    for e in entries.flatten() {
        let name = e.file_name();
        let name = name.to_string_lossy();
        if !name.starts_with("evdi") {
            continue;
        }

        let evdi_path = format!("{root}/{name}");
        if !is_correct_parent_device(&evdi_path, parent_device) {
            continue;
        }

        let dev_index = get_drm_device_index(&format!("{evdi_path}/drm"));
        if dev_index < 0 || dev_index as usize >= EVDI_USAGE_LEN {
            continue;
        }

        if !GLOBALS.lock().unwrap().card_in_use[dev_index as usize] {
            return dev_index;
        }
    }
    INVALID_DEVICE_INDEX
}

fn get_generic_device() -> c_int {
    let idx = find_unused_card_for(None);
    if idx != INVALID_DEVICE_INDEX {
        return idx;
    }
    evdi_log!("Creating card in /sys/devices/platform");
    write_add_device(b"1");
    find_unused_card_for(None)
}

/// Resolve/create a card bound to the given `usb:<syspath>` parent. `raw` is the
/// full `usb:...` string; `parent` is the portion after `usb:`.
fn get_device_attached_to_usb(raw: &[u8], parent: &[u8]) -> c_int {
    let idx = find_unused_card_for(Some(parent));
    if idx != INVALID_DEVICE_INDEX {
        return idx;
    }
    evdi_log!(
        "Creating card for usb device {} in /sys/bus/platform/devices",
        String::from_utf8_lossy(parent)
    );
    write_add_device(raw);
    find_unused_card_for(Some(parent))
}

/// DRM_IOCTL_VERSION with a name buffer; true if the driver name is "evdi".
fn is_evdi(fd: c_int) -> bool {
    let mut name = [0u8; 64];
    let mut ver = ffi::DrmVersion {
        name_len: name.len(),
        name: name.as_mut_ptr().cast(),
        ..Default::default()
    };
    // SAFETY: ver/name are valid for the VERSION ioctl to fill.
    let ret = unsafe {
        do_ioctl(
            fd,
            ffi::drm_ioctl_version(),
            (&mut ver as *mut ffi::DrmVersion).cast(),
            "version",
        )
    };
    if ret != 0 {
        return false;
    }
    let end = name.iter().position(|&b| b == 0).unwrap_or(name.len());
    &name[..end] == b"evdi"
}

/// True if the loaded module's DRM version satisfies the library's minimum.
fn is_evdi_compatible(fd: c_int) -> bool {
    evdi_log!(
        "LibEvdi version ({}.{}.{})",
        LIBEVDI_VERSION_MAJOR,
        LIBEVDI_VERSION_MINOR,
        LIBEVDI_VERSION_PATCH
    );

    let mut ver = ffi::DrmVersion::default();
    // SAFETY: ver is valid for the VERSION ioctl.
    let ret = unsafe {
        do_ioctl(
            fd,
            ffi::drm_ioctl_version(),
            (&mut ver as *mut ffi::DrmVersion).cast(),
            "version",
        )
    };
    if ret != 0 {
        return false;
    }

    evdi_log!(
        "Evdi version ({}.{}.{})",
        ver.version_major,
        ver.version_minor,
        ver.version_patchlevel
    );

    if ver.version_major == EVDI_MODULE_COMPATIBILITY_VERSION_MAJOR
        && ver.version_minor >= EVDI_MODULE_COMPATIBILITY_VERSION_MINOR
    {
        return true;
    }

    evdi_log!(
        "Doesn't match LibEvdi compatibility one ({}.{}.0)",
        EVDI_MODULE_COMPATIBILITY_VERSION_MAJOR,
        EVDI_MODULE_COMPATIBILITY_VERSION_MINOR
    );
    false
}

// ---------------------------------------------------------------------------
// Handle helpers
// ---------------------------------------------------------------------------

/// Borrow the context behind a client handle, or return early.
///
/// # Safety
/// `handle` must be a live handle returned by `evdi_open*` and not yet closed.
unsafe fn ctx<'a>(handle: evdi_handle) -> Option<&'a mut evdi_device_context> {
    handle.as_mut()
}

fn find_buffer<'a>(ctx: &'a evdi_device_context, id: c_int) -> Option<&'a evdi_buffer> {
    ctx.frame_buffers.iter().find(|b| b.id == id)
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

#[no_mangle]
pub extern "C" fn evdi_open(device: c_int) -> evdi_handle {
    let fd = open_device(device);
    if fd <= 0 {
        return core::ptr::null_mut();
    }

    if !(is_evdi(fd) && is_evdi_compatible(fd)) {
        // SAFETY: fd is a descriptor we opened.
        unsafe { ffi::close(fd) };
        return core::ptr::null_mut();
    }

    if device < 0 || device as usize >= EVDI_USAGE_LEN {
        unsafe { ffi::close(fd) };
        return core::ptr::null_mut();
    }

    let ctx = Box::new(evdi_device_context {
        fd,
        buffer_to_update: 0,
        frame_buffers: Vec::new(),
        device_index: device,
    });
    GLOBALS.lock().unwrap().card_in_use[device as usize] = true;
    evdi_log!("Using /dev/dri/card{device}");
    Box::into_raw(ctx)
}

fn device_to_platform(device: c_int) -> evdi_device_status {
    if !device_exists(device) {
        return evdi_device_status::NOT_PRESENT;
    }
    let Ok(entries) = fs::read_dir("/sys/bus/platform/devices") else {
        evdi_log!("Failed to list platform devices");
        return evdi_device_status::NOT_PRESENT;
    };
    for e in entries.flatten() {
        let name = e.file_name();
        let name = name.to_string_lossy();
        if !name.starts_with("evdi") {
            continue;
        }
        let card_path = format!("/sys/bus/platform/devices/{name}/drm/card{device}");
        if path_exists(&card_path) {
            return evdi_device_status::AVAILABLE;
        }
    }
    evdi_device_status::UNRECOGNIZED
}

#[no_mangle]
pub extern "C" fn evdi_check_device(device: c_int) -> evdi_device_status {
    device_to_platform(device)
}

#[no_mangle]
pub extern "C" fn evdi_add_device() -> c_int {
    write_add_device(b"1") as c_int
}

/// # Safety
/// `sysfs_parent_device`, if non-null, must be a valid NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn evdi_open_attached_to(sysfs_parent_device: *const c_char) -> evdi_handle {
    if sysfs_parent_device.is_null() {
        return evdi_open_attached_to_fixed(core::ptr::null(), 0);
    }
    let len = CStr::from_ptr(sysfs_parent_device).to_bytes().len();
    evdi_open_attached_to_fixed(sysfs_parent_device, len)
}

/// # Safety
/// `sysfs_parent_device`, if non-null, must point to at least `length` bytes.
#[no_mangle]
pub unsafe extern "C" fn evdi_open_attached_to_fixed(
    sysfs_parent_device: *const c_char,
    length: usize,
) -> evdi_handle {
    let mut device_index = INVALID_DEVICE_INDEX;

    if sysfs_parent_device.is_null() {
        device_index = get_generic_device();
    } else if length > 4 {
        let raw = core::slice::from_raw_parts(sysfs_parent_device.cast::<u8>(), length);
        if &raw[..4] == b"usb:" {
            device_index = get_device_attached_to_usb(raw, &raw[4..]);
        }
    }

    if device_index >= 0 && (device_index as usize) < EVDI_USAGE_LEN {
        return evdi_open(device_index);
    }
    core::ptr::null_mut()
}

/// # Safety
/// `handle` must be a live handle from `evdi_open*`.
#[no_mangle]
pub unsafe extern "C" fn evdi_close(handle: evdi_handle) {
    if handle.is_null() {
        return;
    }
    // Reclaim ownership and drop the Box (closes nothing yet).
    let ctx = Box::from_raw(handle);
    ffi::close(ctx.fd);

    // Release the lock before logging: evdi_log takes GLOBALS too, and the std
    // Mutex is not re-entrant (this deadlocked DisplayLinkManager's device-removal
    // path on dock unplug until systemd SIGKILLed it).
    let mut freed = None;
    {
        let mut g = GLOBALS.lock().unwrap();
        for (i, used) in g.card_in_use.iter_mut().enumerate() {
            if *used && i as c_int == ctx.device_index {
                *used = false;
                freed = Some(i);
            }
        }
    }
    if let Some(i) = freed {
        evdi_log!("Marking /dev/dri/card{i} as unused");
    }
}

/// # Safety
/// `handle` must be live; `edid` must point to `edid_length` bytes (or be null).
#[no_mangle]
pub unsafe extern "C" fn evdi_connect(
    handle: evdi_handle,
    edid: *const u8,
    edid_length: c_uint,
    sku_area_limit: u32,
) {
    evdi_connect2(
        handle,
        edid,
        edid_length,
        sku_area_limit,
        sku_area_limit.wrapping_mul(60),
    );
}

/// # Safety
/// See [`evdi_connect`].
#[no_mangle]
pub unsafe extern "C" fn evdi_connect2(
    handle: evdi_handle,
    edid: *const u8,
    edid_length: c_uint,
    pixel_area_limit: u32,
    pixel_per_second_limit: u32,
) {
    let Some(ctx) = ctx(handle) else { return };
    let mut cmd = uapi::DrmEvdiConnect {
        connected: 1,
        dev_index: ctx.device_index,
        edid,
        edid_length,
        pixel_area_limit,
        pixel_per_second_limit,
    };
    do_ioctl(
        ctx.fd,
        uapi::ioctl_connect(),
        (&mut cmd as *mut uapi::DrmEvdiConnect).cast(),
        "connect",
    );
}

/// # Safety
/// `handle` must be live.
#[no_mangle]
pub unsafe extern "C" fn evdi_disconnect(handle: evdi_handle) {
    let Some(ctx) = ctx(handle) else { return };
    let mut cmd = uapi::DrmEvdiConnect {
        connected: 0,
        dev_index: 0,
        edid: core::ptr::null(),
        edid_length: 0,
        pixel_area_limit: 0,
        pixel_per_second_limit: 0,
    };
    do_ioctl(
        ctx.fd,
        uapi::ioctl_connect(),
        (&mut cmd as *mut uapi::DrmEvdiConnect).cast(),
        "disconnect",
    );
}

/// # Safety
/// `handle` must be live.
#[no_mangle]
pub unsafe extern "C" fn evdi_enable_cursor_events(handle: evdi_handle, enable: bool) {
    let Some(ctx) = ctx(handle) else { return };
    evdi_log!(
        "{} cursor events on /dev/dri/card{}",
        if enable { "Enabling" } else { "Disabling" },
        ctx.device_index
    );
    let mut cmd = uapi::DrmEvdiEnableCursorEvents {
        base: uapi::DrmEvent {
            type_: 0,
            length: 0,
        },
        enable: enable as u8,
    };
    do_ioctl(
        ctx.fd,
        uapi::ioctl_enable_cursor_events(),
        (&mut cmd as *mut uapi::DrmEvdiEnableCursorEvents).cast(),
        "enable cursor events",
    );
}

/// # Safety
/// `handle` must be live; `rects` must point to at least `MAX_DIRTS` entries and
/// `num_rects` must be a valid pointer.
#[no_mangle]
pub unsafe extern "C" fn evdi_grab_pixels(
    handle: evdi_handle,
    rects: *mut evdi_rect,
    num_rects: *mut c_int,
) {
    // Guard once up front: every exit path below writes through `num_rects`.
    if num_rects.is_null() {
        return;
    }
    let Some(ctx) = ctx(handle) else {
        *num_rects = 0;
        return;
    };

    let Some(dest) = find_buffer(ctx, ctx.buffer_to_update).copied() else {
        evdi_log!("Buffer {} not found. Not grabbing.", ctx.buffer_to_update);
        *num_rects = 0;
        return;
    };

    let mut kernel_dirts = [uapi::DrmClipRect::default(); MAX_DIRTS];
    let mut grab = uapi::DrmEvdiGrabpix {
        mode: uapi::EVDI_GRABPIX_MODE_DIRTY,
        buf_width: dest.width,
        buf_height: dest.height,
        buf_byte_stride: dest.stride,
        buffer: dest.buffer.cast(),
        num_rects: MAX_DIRTS as c_int,
        rects: kernel_dirts.as_mut_ptr(),
    };

    if do_ioctl(
        ctx.fd,
        uapi::ioctl_grabpix(),
        (&mut grab as *mut uapi::DrmEvdiGrabpix).cast(),
        "grabpix",
    ) == 0
    {
        let n = grab.num_rects.max(0) as usize;
        for (r, kd) in kernel_dirts.iter().enumerate().take(n) {
            *rects.add(r) = evdi_rect {
                x1: kd.x1 as c_int,
                y1: kd.y1 as c_int,
                x2: kd.x2 as c_int,
                y2: kd.y2 as c_int,
            };
        }
        *num_rects = grab.num_rects;
    } else {
        evdi_log!("Grabbing pixels for buffer {} failed.", dest.id);
        evdi_log!("Ignore if caused by change of mode.");
        *num_rects = 0;
    }
}

/// # Safety
/// `handle` must be live.
#[no_mangle]
pub unsafe extern "C" fn evdi_register_buffer(handle: evdi_handle, buffer: evdi_buffer) {
    let Some(ctx) = ctx(handle) else { return };
    debug_assert!(find_buffer(ctx, buffer.id).is_none());
    ctx.frame_buffers.push(buffer);
}

/// # Safety
/// `handle` must be live.
#[no_mangle]
pub unsafe extern "C" fn evdi_unregister_buffer(handle: evdi_handle, buffer_id: c_int) {
    let Some(ctx) = ctx(handle) else { return };
    ctx.frame_buffers.retain(|b| b.id != buffer_id);
}

/// # Safety
/// `handle` must be live.
#[no_mangle]
pub unsafe extern "C" fn evdi_request_update(handle: evdi_handle, buffer_id: c_int) -> bool {
    let Some(ctx) = ctx(handle) else { return false };
    ctx.buffer_to_update = buffer_id;
    let mut cmd = uapi::DrmEvdiRequestUpdate::default();
    let result = do_ioctl(
        ctx.fd,
        uapi::ioctl_request_update(),
        (&mut cmd as *mut uapi::DrmEvdiRequestUpdate).cast(),
        "request_update",
    );
    result == 1
}

/// # Safety
/// `handle` must be live; `buffer` must point to `buffer_length` bytes.
#[no_mangle]
pub unsafe extern "C" fn evdi_ddcci_response(
    handle: evdi_handle,
    buffer: *const u8,
    buffer_length: u32,
    result: bool,
) {
    let Some(ctx) = ctx(handle) else { return };
    let mut cmd = uapi::DrmEvdiDdcciResponse {
        buffer,
        buffer_length,
        result: result as u8,
    };
    do_ioctl(
        ctx.fd,
        uapi::ioctl_ddcci_response(),
        (&mut cmd as *mut uapi::DrmEvdiDdcciResponse).cast(),
        "ddcci_response",
    );
}

fn evdi_get_dumb_offset(fd: c_int, handle: u32) -> Option<u64> {
    let mut map = ffi::DrmModeMapDumb {
        handle,
        ..Default::default()
    };
    // SAFETY: map is valid for the MAP_DUMB ioctl.
    let ret = unsafe {
        do_ioctl(
            fd,
            ffi::drm_ioctl_mode_map_dumb(),
            (&mut map as *mut ffi::DrmModeMapDumb).cast(),
            "DRM_MODE_MAP_DUMB",
        )
    };
    if ret != 0 {
        None
    } else {
        Some(map.offset)
    }
}

/// Build the public `evdi_cursor_set` from the kernel event, mapping+copying the
/// cursor bitmap out of the dumb buffer. Mirrors C `to_evdi_cursor_set`.
///
/// # Safety
/// `ev` must be a valid cursor-set event.
unsafe fn to_evdi_cursor_set(
    ctx: &evdi_device_context,
    ev: &uapi::DrmEvdiEventCursorSet,
) -> evdi_cursor_set {
    let mut cs = evdi_cursor_set {
        hot_x: ev.hot_x,
        hot_y: ev.hot_y,
        width: ev.width,
        height: ev.height,
        enabled: ev.enabled,
        buffer_length: 0,
        buffer: core::ptr::null_mut(),
        pixel_format: ev.pixel_format,
        stride: ev.stride,
    };

    if ev.enabled != 0 {
        let size = ev.buffer_length as usize;
        if let Some(offset) = evdi_get_dumb_offset(ctx.fd, ev.buffer_handle) {
            let ptr = ffi::mmap(
                core::ptr::null_mut(),
                size,
                ffi::PROT_READ,
                ffi::MAP_SHARED,
                ctx.fd,
                offset as i64,
            );
            if ptr != ffi::MAP_FAILED {
                // malloc(size) so the client frees it with free(); libc malloc is
                // reachable through the process allocator.
                let out = libc_malloc(size);
                if !out.is_null() {
                    core::ptr::copy_nonoverlapping(ptr.cast::<u8>(), out.cast::<u8>(), size);
                    cs.buffer = out.cast();
                    cs.buffer_length = size as u32;
                }
                ffi::munmap(ptr, size);
            } else {
                evdi_log!("Error: mmap failed");
            }
        } else {
            evdi_log!("Error: DRM_IOCTL_MODE_MAP_DUMB failed");
        }
        // The kernel mints a fresh GEM handle for every CURSOR_SET; we own it now and have our
        // own copy of the bitmap (the compositor still pins the BO via its plane state), so
        // close it — otherwise this file's handle table grows by one per cursor change for the
        // life of the session. Upstream libevdi leaks these; we don't have to.
        if ev.buffer_handle != 0 {
            let mut gem_close = ffi::DrmGemClose {
                handle: ev.buffer_handle,
                pad: 0,
            };
            ffi::drm_ioctl(
                ctx.fd,
                ffi::drm_ioctl_gem_close(),
                (&mut gem_close as *mut ffi::DrmGemClose).cast(),
            );
        }
    }
    cs
}

extern "C" {
    #[link_name = "malloc"]
    fn libc_malloc(size: usize) -> *mut c_void;
}

/// # Safety
/// `handle` must be live; `evtctx` must be a valid event context.
#[no_mangle]
pub unsafe extern "C" fn evdi_handle_events(handle: evdi_handle, evtctx: *mut evdi_event_context) {
    let Some(ctx) = ctx(handle) else { return };

    // The bytes are reinterpreted as `uapi::DrmEvent` (and larger event structs
    // with u32 fields) below, so the backing storage must be at least 8-byte
    // aligned — a bare `[u8; N]` is only 1-aligned, which would make the
    // `&*(ptr as *const DrmEvent)` reads misaligned (UB; SIGBUS on strict arches).
    #[repr(C, align(8))]
    struct EventBuf([u8; 1024]);
    let mut buf_container = EventBuf([0u8; 1024]);
    let buf = &mut buf_container.0;
    // SAFETY: buf is a valid 1024-byte destination.
    let bytes_read = ffi::read(ctx.fd, buf.as_mut_ptr().cast(), buf.len());

    let Some(evtctx) = evtctx.as_ref() else {
        evdi_log!("Error: Event context is null!");
        return;
    };

    if bytes_read <= 0 {
        return;
    }
    let bytes_read = bytes_read as usize;

    let mut i = 0usize;
    while i + core::mem::size_of::<uapi::DrmEvent>() <= bytes_read {
        // SAFETY: bounds checked against bytes_read above.
        let hdr = &*(buf.as_ptr().add(i) as *const uapi::DrmEvent);
        let len = hdr.length as usize;
        if len == 0 || i + len > bytes_read {
            break;
        }
        dispatch_event(ctx, evtctx, buf.as_ptr().add(i), hdr.type_);
        i += len;
    }
}

/// # Safety
/// `raw` must point to a complete event of the type `etype`.
unsafe fn dispatch_event(
    ctx: &evdi_device_context,
    evtctx: &evdi_event_context,
    raw: *const u8,
    etype: u32,
) {
    match etype {
        uapi::DRM_EVDI_EVENT_UPDATE_READY => {
            if let Some(h) = evtctx.update_ready_handler {
                h(ctx.buffer_to_update, evtctx.user_data);
            }
        }
        uapi::DRM_EVDI_EVENT_DPMS => {
            if let Some(h) = evtctx.dpms_handler {
                let ev = &*(raw as *const uapi::DrmEvdiEventDpms);
                h(ev.mode, evtctx.user_data);
            }
        }
        uapi::DRM_EVDI_EVENT_MODE_CHANGED => {
            if let Some(h) = evtctx.mode_changed_handler {
                let ev = &*(raw as *const uapi::DrmEvdiEventModeChanged);
                let mode = evdi_mode {
                    width: ev.hdisplay,
                    height: ev.vdisplay,
                    refresh_rate: ev.vrefresh,
                    bits_per_pixel: ev.bits_per_pixel,
                    pixel_format: ev.pixel_format,
                };
                h(mode, evtctx.user_data);
            }
        }
        uapi::DRM_EVDI_EVENT_CRTC_STATE => {
            if let Some(h) = evtctx.crtc_state_handler {
                let ev = &*(raw as *const uapi::DrmEvdiEventCrtcState);
                h(ev.state, evtctx.user_data);
            }
        }
        uapi::DRM_EVDI_EVENT_CURSOR_SET => {
            if let Some(h) = evtctx.cursor_set_handler {
                let ev = &*(raw as *const uapi::DrmEvdiEventCursorSet);
                let mut cs = to_evdi_cursor_set(ctx, ev);
                if cs.enabled != 0 && cs.buffer.is_null() {
                    evdi_log!("Error: Cursor buffer is null!");
                    evdi_log!("Disabling cursor events");
                    // The handle is shared &; re-issue the ioctl directly.
                    let mut cmd = uapi::DrmEvdiEnableCursorEvents {
                        base: uapi::DrmEvent {
                            type_: 0,
                            length: 0,
                        },
                        enable: 0,
                    };
                    do_ioctl(
                        ctx.fd,
                        uapi::ioctl_enable_cursor_events(),
                        (&mut cmd as *mut uapi::DrmEvdiEnableCursorEvents).cast(),
                        "enable cursor events",
                    );
                    cs.enabled = 0;
                    cs.buffer_length = 0;
                }
                h(cs, evtctx.user_data);
            }
        }
        uapi::DRM_EVDI_EVENT_CURSOR_MOVE => {
            if let Some(h) = evtctx.cursor_move_handler {
                let ev = &*(raw as *const uapi::DrmEvdiEventCursorMove);
                h(evdi_cursor_move { x: ev.x, y: ev.y }, evtctx.user_data);
            }
        }
        uapi::DRM_EVDI_EVENT_DDCCI_DATA => {
            if let Some(h) = evtctx.ddcci_data_handler {
                let ev = &*(raw as *const uapi::DrmEvdiEventDdcciData);
                let data = evdi_ddcci_data {
                    address: ev.address,
                    flags: ev.flags,
                    buffer_length: ev.buffer_length,
                    // Points into the event bytes for the duration of the callback.
                    buffer: ev.buffer.as_ptr() as *mut u8,
                };
                h(data, evtctx.user_data);
            }
        }
        _ => evdi_log!("Warning: Unhandled event"),
    }
}

/// # Safety
/// `handle` must be live.
#[no_mangle]
pub unsafe extern "C" fn evdi_get_event_ready(handle: evdi_handle) -> c_int {
    match ctx(handle) {
        Some(ctx) => ctx.fd,
        None => -1,
    }
}

/// # Safety
/// `version`, if non-null, must point to a valid `evdi_lib_version`.
#[no_mangle]
pub unsafe extern "C" fn evdi_get_lib_version(version: *mut evdi_lib_version) {
    if let Some(v) = version.as_mut() {
        v.version_major = LIBEVDI_VERSION_MAJOR;
        v.version_minor = LIBEVDI_VERSION_MINOR;
        v.version_patchlevel = LIBEVDI_VERSION_PATCH;
    }
}

#[no_mangle]
pub extern "C" fn evdi_set_logging(logging: evdi_logging) {
    GLOBALS.lock().unwrap().logging = logging;
}

/// Is an X server running? Scans `/proc/*/comm` for "Xorg". (`Xorg_running`)
#[no_mangle]
pub extern "C" fn Xorg_running() -> bool {
    xorg_running()
}

fn xorg_running() -> bool {
    let Ok(entries) = fs::read_dir("/proc") else {
        return false;
    };
    for e in entries.flatten() {
        let name = e.file_name();
        let name = name.to_string_lossy();
        if name.parse::<u32>().is_err() {
            continue;
        }
        if let Ok(comm) = fs::read_to_string(format!("/proc/{name}/comm")) {
            if comm.trim_end().contains("Xorg") {
                return true;
            }
        }
    }
    false
}
