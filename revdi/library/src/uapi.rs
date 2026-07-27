// SPDX-License-Identifier: LGPL-2.1-or-later
// Copyright (c) 2025 revdi contributors
//
// Userspace mirror of the EVDI kernel uAPI (`evdi/module/evdi_drm.h`). These
// layouts MUST stay byte-identical to the kernel module's `uapi.rs`; the
// `size_of` assertions at the bottom pin them.

use core::ffi::c_uchar;

pub const DRM_COMMAND_BASE: u32 = 0x40;

// Output event type codes (driver -> library), carried in `DrmEvent::type_`.
pub const DRM_EVDI_EVENT_UPDATE_READY: u32 = 0x8000_0000;
pub const DRM_EVDI_EVENT_DPMS: u32 = 0x8000_0001;
pub const DRM_EVDI_EVENT_MODE_CHANGED: u32 = 0x8000_0002;
pub const DRM_EVDI_EVENT_CRTC_STATE: u32 = 0x8000_0003;
pub const DRM_EVDI_EVENT_CURSOR_SET: u32 = 0x8000_0004;
pub const DRM_EVDI_EVENT_CURSOR_MOVE: u32 = 0x8000_0005;
pub const DRM_EVDI_EVENT_DDCCI_DATA: u32 = 0x8000_0006;

pub const DDCCI_BUFFER_SIZE: usize = 64;

/// `struct drm_event` — common header every DRM event starts with.
#[repr(C)]
#[derive(Copy, Clone)]
pub struct DrmEvent {
    pub type_: u32,
    pub length: u32,
}

#[repr(C)]
pub struct DrmEvdiEventDpms {
    pub base: DrmEvent,
    pub mode: i32,
}

#[repr(C)]
pub struct DrmEvdiEventModeChanged {
    pub base: DrmEvent,
    pub hdisplay: i32,
    pub vdisplay: i32,
    pub vrefresh: i32,
    pub bits_per_pixel: i32,
    pub pixel_format: u32,
}

#[repr(C)]
pub struct DrmEvdiEventCrtcState {
    pub base: DrmEvent,
    pub state: i32,
}

#[repr(C)]
pub struct DrmEvdiEventCursorSet {
    pub base: DrmEvent,
    pub hot_x: i32,
    pub hot_y: i32,
    pub width: u32,
    pub height: u32,
    pub enabled: u8,
    pub buffer_handle: u32,
    pub buffer_length: u32,
    pub pixel_format: u32,
    pub stride: u32,
}

#[repr(C)]
pub struct DrmEvdiEventCursorMove {
    pub base: DrmEvent,
    pub x: i32,
    pub y: i32,
}

#[repr(C)]
pub struct DrmEvdiEventDdcciData {
    pub base: DrmEvent,
    pub buffer: [u8; DDCCI_BUFFER_SIZE],
    pub buffer_length: u32,
    pub flags: u16,
    pub address: u16,
}

// Input ioctl argument structures (library -> driver).

#[repr(C)]
pub struct DrmEvdiConnect {
    pub connected: i32,
    pub dev_index: i32,
    pub edid: *const c_uchar,
    pub edid_length: u32,
    pub pixel_area_limit: u32,
    pub pixel_per_second_limit: u32,
}

#[repr(C)]
#[derive(Default)]
pub struct DrmEvdiRequestUpdate {
    pub reserved: i32,
}

pub const EVDI_GRABPIX_MODE_DIRTY: i32 = 1;

/// `struct drm_clip_rect { u16 x1, y1, x2, y2; }`.
#[repr(C)]
#[derive(Copy, Clone, Default)]
pub struct DrmClipRect {
    pub x1: u16,
    pub y1: u16,
    pub x2: u16,
    pub y2: u16,
}

#[repr(C)]
pub struct DrmEvdiGrabpix {
    pub mode: i32,
    pub buf_width: i32,
    pub buf_height: i32,
    pub buf_byte_stride: i32,
    pub buffer: *mut c_uchar,
    pub num_rects: i32,
    pub rects: *mut DrmClipRect,
}

#[repr(C)]
pub struct DrmEvdiDdcciResponse {
    pub buffer: *const c_uchar,
    pub buffer_length: u32,
    pub result: u8,
}

#[repr(C)]
pub struct DrmEvdiEnableCursorEvents {
    pub base: DrmEvent,
    pub enable: u8,
}

// Input ioctl command numbers.
const DRM_EVDI_CONNECT: u32 = 0x00;
const DRM_EVDI_REQUEST_UPDATE: u32 = 0x01;
const DRM_EVDI_GRABPIX: u32 = 0x02;
const DRM_EVDI_DDCCI_RESPONSE: u32 = 0x03;
const DRM_EVDI_ENABLE_CURSOR_EVENTS: u32 = 0x04;

const fn iowr(nr: u32, size: usize) -> core::ffi::c_ulong {
    // asm-generic _IOC(READ|WRITE, 'd', nr, size); see ffi::ioc.
    (((2u32 | 1u32) << 30) | (0x64u32 << 8) | (nr << 0) | ((size as u32) << 16))
        as core::ffi::c_ulong
}

pub fn ioctl_connect() -> core::ffi::c_ulong {
    iowr(
        DRM_COMMAND_BASE + DRM_EVDI_CONNECT,
        core::mem::size_of::<DrmEvdiConnect>(),
    )
}
pub fn ioctl_request_update() -> core::ffi::c_ulong {
    iowr(
        DRM_COMMAND_BASE + DRM_EVDI_REQUEST_UPDATE,
        core::mem::size_of::<DrmEvdiRequestUpdate>(),
    )
}
pub fn ioctl_grabpix() -> core::ffi::c_ulong {
    iowr(
        DRM_COMMAND_BASE + DRM_EVDI_GRABPIX,
        core::mem::size_of::<DrmEvdiGrabpix>(),
    )
}
pub fn ioctl_ddcci_response() -> core::ffi::c_ulong {
    iowr(
        DRM_COMMAND_BASE + DRM_EVDI_DDCCI_RESPONSE,
        core::mem::size_of::<DrmEvdiDdcciResponse>(),
    )
}
pub fn ioctl_enable_cursor_events() -> core::ffi::c_ulong {
    iowr(
        DRM_COMMAND_BASE + DRM_EVDI_ENABLE_CURSOR_EVENTS,
        core::mem::size_of::<DrmEvdiEnableCursorEvents>(),
    )
}

// ABI size assertions (LP64) — must match the kernel module's uapi.rs.
const _: () = {
    assert!(core::mem::size_of::<DrmEvent>() == 8);
    assert!(core::mem::size_of::<DrmEvdiEventDpms>() == 12);
    assert!(core::mem::size_of::<DrmEvdiEventModeChanged>() == 28);
    assert!(core::mem::size_of::<DrmEvdiEventCrtcState>() == 12);
    assert!(core::mem::size_of::<DrmEvdiEventCursorSet>() == 44);
    assert!(core::mem::size_of::<DrmEvdiEventCursorMove>() == 16);
    assert!(core::mem::size_of::<DrmEvdiEventDdcciData>() == 80);
    assert!(core::mem::size_of::<DrmEvdiConnect>() == 32);
    assert!(core::mem::size_of::<DrmEvdiRequestUpdate>() == 4);
    assert!(core::mem::size_of::<DrmClipRect>() == 8);
    assert!(core::mem::size_of::<DrmEvdiGrabpix>() == 40);
    assert!(core::mem::size_of::<DrmEvdiDdcciResponse>() == 16);
    assert!(core::mem::size_of::<DrmEvdiEnableCursorEvents>() == 12);
};
