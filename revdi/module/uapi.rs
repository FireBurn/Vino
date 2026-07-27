// SPDX-License-Identifier: GPL-2.0
//
// Userspace ABI for the EVDI DRM driver.
//
// This is a byte-for-byte Rust mirror of the C uAPI header
// `evdi/module/evdi_drm.h` (DisplayLink (UK) Ltd.). It is reproduced here rather
// than pulled from `kernel::uapi` because EVDI is an out-of-tree driver with its
// own private ioctl range; the struct layouts and ioctl numbers MUST stay
// identical to the C module so existing userspace (libevdi / DisplayLinkManager)
// keeps working unchanged.
//
// Every structure is `#[repr(C)]`; user pointers are represented as `usize` so the
// native (LP64/ILP32) ABI width matches the C `T __user *` exactly. Compile-time
// assertions at the bottom pin each `size_of` to the value the C ABI produces.

#![allow(dead_code)]

use kernel::drm::ioctl::{IO, IOWR};

/// DRM driver-private ioctl base (`DRM_COMMAND_BASE`).
pub(crate) const DRM_COMMAND_BASE: u32 = 0x40;

// --- Output event type codes (driver -> libevdi), `drm_event::type` values. ---
pub(crate) const DRM_EVDI_EVENT_UPDATE_READY: u32 = 0x8000_0000;
pub(crate) const DRM_EVDI_EVENT_DPMS: u32 = 0x8000_0001;
pub(crate) const DRM_EVDI_EVENT_MODE_CHANGED: u32 = 0x8000_0002;
pub(crate) const DRM_EVDI_EVENT_CRTC_STATE: u32 = 0x8000_0003;
pub(crate) const DRM_EVDI_EVENT_CURSOR_SET: u32 = 0x8000_0004;
pub(crate) const DRM_EVDI_EVENT_CURSOR_MOVE: u32 = 0x8000_0005;
pub(crate) const DRM_EVDI_EVENT_DDCCI_DATA: u32 = 0x8000_0006;

/// Fixed DDC/CI payload size carried by [`DrmEvdiEventDdcciData`].
pub(crate) const DDCCI_BUFFER_SIZE: usize = 64;

/// `struct drm_event` — the common header every DRM event starts with.
pub(crate) type DrmEvent = kernel::drm::event::EventHeader;

/// `struct drm_evdi_event_update_ready`.
#[repr(C)]
#[derive(Copy, Clone)]
pub(crate) struct DrmEvdiEventUpdateReady {
    pub base: DrmEvent,
}

/// `struct drm_evdi_event_dpms`.
#[repr(C)]
#[derive(Copy, Clone)]
pub(crate) struct DrmEvdiEventDpms {
    pub base: DrmEvent,
    pub mode: i32,
}

/// `struct drm_evdi_event_mode_changed`.
#[repr(C)]
#[derive(Copy, Clone)]
pub(crate) struct DrmEvdiEventModeChanged {
    pub base: DrmEvent,
    pub hdisplay: i32,
    pub vdisplay: i32,
    pub vrefresh: i32,
    pub bits_per_pixel: i32,
    pub pixel_format: u32,
}

/// `struct drm_evdi_event_crtc_state`.
#[repr(C)]
#[derive(Copy, Clone)]
pub(crate) struct DrmEvdiEventCrtcState {
    pub base: DrmEvent,
    pub state: i32,
}

/// `struct drm_evdi_event_cursor_set`.
#[repr(C)]
#[derive(Copy, Clone)]
pub(crate) struct DrmEvdiEventCursorSet {
    pub base: DrmEvent,
    pub hot_x: i32,
    pub hot_y: i32,
    pub width: u32,
    pub height: u32,
    pub enabled: u8,
    pub reserved: [u8; 3],
    pub buffer_handle: u32,
    pub buffer_length: u32,
    pub pixel_format: u32,
    pub stride: u32,
}

/// `struct drm_evdi_event_cursor_move`.
#[repr(C)]
#[derive(Copy, Clone)]
pub(crate) struct DrmEvdiEventCursorMove {
    pub base: DrmEvent,
    pub x: i32,
    pub y: i32,
}

/// `struct drm_evdi_event_ddcci_data`.
#[repr(C)]
#[derive(Copy, Clone)]
pub(crate) struct DrmEvdiEventDdcciData {
    pub base: DrmEvent,
    pub buffer: [u8; DDCCI_BUFFER_SIZE],
    pub buffer_length: u32,
    pub flags: u16,
    pub address: u16,
}

// --- Input ioctl argument structures (libevdi -> driver). ---

/// `struct drm_evdi_connect`. `edid` is a `const unsigned char __user *`.
#[repr(C)]
#[derive(Copy, Clone)]
pub(crate) struct DrmEvdiConnect {
    pub connected: i32,
    pub dev_index: i32,
    pub edid: usize,
    pub edid_length: u32,
    pub pixel_area_limit: u32,
    pub pixel_per_second_limit: u32,
}

/// `struct drm_evdi_request_update`.
#[repr(C)]
#[derive(Copy, Clone)]
pub(crate) struct DrmEvdiRequestUpdate {
    pub reserved: i32,
}

/// `enum drm_evdi_grabpix_mode` (C `int`).
pub(crate) const EVDI_GRABPIX_MODE_RECTS: i32 = 0;
pub(crate) const EVDI_GRABPIX_MODE_DIRTY: i32 = 1;

/// `struct drm_evdi_grabpix`. `buffer` and `rects` are `__user` pointers.
#[repr(C)]
#[derive(Copy, Clone)]
pub(crate) struct DrmEvdiGrabpix {
    pub mode: i32,
    pub buf_width: i32,
    pub buf_height: i32,
    pub buf_byte_stride: i32,
    pub buffer: usize,
    pub num_rects: i32,
    pub rects: usize,
}

/// `struct drm_evdi_ddcci_response`. `buffer` is a `const unsigned char __user *`.
#[repr(C)]
#[derive(Copy, Clone)]
pub(crate) struct DrmEvdiDdcciResponse {
    pub buffer: usize,
    pub buffer_length: u32,
    pub result: u8,
}

/// `struct drm_evdi_enable_cursor_events`.
#[repr(C)]
#[derive(Copy, Clone)]
pub(crate) struct DrmEvdiEnableCursorEvents {
    pub base: DrmEvent,
    pub enable: u8,
}

// --- Input ioctl command numbers (`DRM_IOCTL_EVDI_*`). ---
pub(crate) const DRM_EVDI_CONNECT: u32 = 0x00;
pub(crate) const DRM_EVDI_REQUEST_UPDATE: u32 = 0x01;
pub(crate) const DRM_EVDI_GRABPIX: u32 = 0x02;
pub(crate) const DRM_EVDI_DDCCI_RESPONSE: u32 = 0x03;
pub(crate) const DRM_EVDI_ENABLE_CURSOR_EVENTS: u32 = 0x04;

/// `DRM_IOCTL_EVDI_CONNECT`.
pub(crate) const DRM_IOCTL_EVDI_CONNECT: u32 =
    IOWR::<DrmEvdiConnect>(DRM_COMMAND_BASE + DRM_EVDI_CONNECT);
/// `DRM_IOCTL_EVDI_REQUEST_UPDATE`.
pub(crate) const DRM_IOCTL_EVDI_REQUEST_UPDATE: u32 =
    IOWR::<DrmEvdiRequestUpdate>(DRM_COMMAND_BASE + DRM_EVDI_REQUEST_UPDATE);
/// `DRM_IOCTL_EVDI_GRABPIX`.
pub(crate) const DRM_IOCTL_EVDI_GRABPIX: u32 =
    IOWR::<DrmEvdiGrabpix>(DRM_COMMAND_BASE + DRM_EVDI_GRABPIX);
/// `DRM_IOCTL_EVDI_DDCCI_RESPONSE`.
pub(crate) const DRM_IOCTL_EVDI_DDCCI_RESPONSE: u32 =
    IOWR::<DrmEvdiDdcciResponse>(DRM_COMMAND_BASE + DRM_EVDI_DDCCI_RESPONSE);
/// `DRM_IOCTL_EVDI_ENABLE_CURSOR_EVENTS`.
pub(crate) const DRM_IOCTL_EVDI_ENABLE_CURSOR_EVENTS: u32 =
    IOWR::<DrmEvdiEnableCursorEvents>(DRM_COMMAND_BASE + DRM_EVDI_ENABLE_CURSOR_EVENTS);

// Silence unused-import warnings until the no-arg `IO` helper is needed.
const _: fn(u32) -> u32 = IO;

// --- ABI size assertions: these must match the C `sizeof` on LP64. ---
const _: () = {
    assert!(core::mem::size_of::<DrmEvent>() == 8);
    assert!(core::mem::size_of::<DrmEvdiEventUpdateReady>() == 8);
    assert!(core::mem::size_of::<DrmEvdiEventDpms>() == 12);
    assert!(core::mem::size_of::<DrmEvdiEventModeChanged>() == 28);
    assert!(core::mem::size_of::<DrmEvdiEventCrtcState>() == 12);
    assert!(core::mem::size_of::<DrmEvdiEventCursorSet>() == 44);
    assert!(core::mem::size_of::<DrmEvdiEventCursorMove>() == 16);
    assert!(core::mem::size_of::<DrmEvdiEventDdcciData>() == 80);
    assert!(core::mem::size_of::<DrmEvdiConnect>() == 32);
    assert!(core::mem::size_of::<DrmEvdiRequestUpdate>() == 4);
    assert!(core::mem::size_of::<DrmEvdiGrabpix>() == 40);
    assert!(core::mem::size_of::<DrmEvdiDdcciResponse>() == 16);
    assert!(core::mem::size_of::<DrmEvdiEnableCursorEvents>() == 12);
};
