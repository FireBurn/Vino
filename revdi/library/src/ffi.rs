// SPDX-License-Identifier: LGPL-2.1-or-later
// Copyright (c) 2025 revdi contributors
//
// Thin FFI layer: the handful of libc entry points the library needs that have
// no ergonomic `std` equivalent (raw `ioctl`/`mmap` on a DRM fd), plus the
// generic DRM ioctl numbers and argument structs used for device probing.

use core::ffi::{c_char, c_int, c_uint, c_ulong, c_void};

// --- libc entry points -----------------------------------------------------

extern "C" {
    /// `open(2)` — called without `O_CREAT`, so the variadic `mode` argument is
    /// unused and the 2-argument form is ABI-correct.
    pub fn open(path: *const c_char, flags: c_int) -> c_int;
    pub fn close(fd: c_int) -> c_int;
    pub fn read(fd: c_int, buf: *mut c_void, count: usize) -> isize;
    pub fn poll(fds: *mut PollFd, nfds: usize, timeout: c_int) -> c_int;
    /// `ioctl(2)` — the request carries a single pointer argument for every
    /// call site here, so the fixed 3-argument form matches the C ABI.
    pub fn ioctl(fd: c_int, request: c_ulong, arg: *mut c_void) -> c_int;
    pub fn mmap(
        addr: *mut c_void,
        len: usize,
        prot: c_int,
        flags: c_int,
        fd: c_int,
        offset: i64,
    ) -> *mut c_void;
    pub fn munmap(addr: *mut c_void, len: usize) -> c_int;
    pub fn free(ptr: *mut c_void);
    fn __errno_location() -> *mut c_int;
}

pub const O_RDWR: c_int = 0o2;
pub const POLLIN: i16 = 0x0001;
pub const PROT_READ: c_int = 0x1;
pub const MAP_SHARED: c_int = 0x1;
pub const MAP_FAILED: *mut c_void = usize::MAX as *mut c_void;

pub const EINTR: c_int = 4;
pub const EAGAIN: c_int = 11;
pub const EACCES: c_int = 13;

#[repr(C)]
pub struct PollFd {
    pub fd: c_int,
    pub events: i16,
    pub revents: i16,
}

/// Current thread's `errno`.
pub fn errno() -> c_int {
    // SAFETY: `__errno_location` always returns a valid pointer to this
    // thread's `errno` storage.
    unsafe { *__errno_location() }
}

// --- ioctl number construction (asm-generic `_IOC`) ------------------------

const IOC_NRSHIFT: u32 = 0;
const IOC_TYPESHIFT: u32 = 8;
const IOC_SIZESHIFT: u32 = 16;
const IOC_DIRSHIFT: u32 = 30;

const IOC_NONE: u32 = 0;
const IOC_WRITE: u32 = 1;
const IOC_READ: u32 = 2;

/// DRM ioctl type letter (`'d'`).
const DRM_IOCTL_BASE: u32 = 0x64;

const fn ioc(dir: u32, nr: u32, size: usize) -> c_ulong {
    ((dir << IOC_DIRSHIFT)
        | (DRM_IOCTL_BASE << IOC_TYPESHIFT)
        | (nr << IOC_NRSHIFT)
        | ((size as u32) << IOC_SIZESHIFT)) as c_ulong
}

const fn drm_io(nr: u32) -> c_ulong {
    ioc(IOC_NONE, nr, 0)
}
const fn drm_iow(nr: u32, size: usize) -> c_ulong {
    ioc(IOC_WRITE, nr, size)
}
const fn drm_iowr(nr: u32, size: usize) -> c_ulong {
    ioc(IOC_READ | IOC_WRITE, nr, size)
}

// --- Generic DRM ioctls used for probing/authentication --------------------

/// `struct drm_version` (LP64 layout, 64 bytes).
#[repr(C)]
pub struct DrmVersion {
    pub version_major: c_int,
    pub version_minor: c_int,
    pub version_patchlevel: c_int,
    pub name_len: usize,
    pub name: *mut c_char,
    pub date_len: usize,
    pub date: *mut c_char,
    pub desc_len: usize,
    pub desc: *mut c_char,
}

impl Default for DrmVersion {
    fn default() -> Self {
        // SAFETY: `DrmVersion` is plain data for which all-zero is a valid,
        // well-defined value (null pointers, zero lengths).
        unsafe { core::mem::zeroed() }
    }
}

/// `struct drm_auth { drm_magic_t magic; }`.
#[repr(C)]
pub struct DrmAuth {
    pub magic: c_uint,
}

/// `struct drm_mode_map_dumb { __u32 handle; __u32 pad; __u64 offset; }`.
#[repr(C)]
#[derive(Default)]
pub struct DrmModeMapDumb {
    pub handle: u32,
    pub pad: u32,
    pub offset: u64,
}

/// `struct drm_gem_close { __u32 handle; __u32 pad; }`.
#[repr(C)]
#[derive(Default)]
pub struct DrmGemClose {
    pub handle: u32,
    pub pad: u32,
}

pub fn drm_ioctl_version() -> c_ulong {
    drm_iowr(0x00, core::mem::size_of::<DrmVersion>())
}
pub fn drm_ioctl_auth_magic() -> c_ulong {
    drm_iow(0x11, core::mem::size_of::<DrmAuth>())
}
pub fn drm_ioctl_drop_master() -> c_ulong {
    drm_io(0x1f)
}
pub fn drm_ioctl_mode_map_dumb() -> c_ulong {
    drm_iowr(0xb3, core::mem::size_of::<DrmModeMapDumb>())
}
pub fn drm_ioctl_gem_close() -> c_ulong {
    drm_iow(0x09, core::mem::size_of::<DrmGemClose>())
}

/// Retrying `ioctl` wrapper mirroring libevdi's `drm_ioctl`: restart on
/// `EINTR`/`EAGAIN`, otherwise return the raw result.
///
/// # Safety
/// `arg` must point to a valid argument object of the size encoded in
/// `request`, and `fd` must be an open file descriptor.
pub unsafe fn drm_ioctl(fd: c_int, request: c_ulong, arg: *mut c_void) -> c_int {
    loop {
        let ret = ioctl(fd, request, arg);
        if ret == -1 && (errno() == EINTR || errno() == EAGAIN) {
            continue;
        }
        return ret;
    }
}
