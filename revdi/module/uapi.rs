// SPDX-License-Identifier: GPL-2.0

//! Rust names for the EVDI UAPI generated from `include/uapi/drm/evdi_drm.h`.

use kernel::{
    drm::ioctl::CompatIoctl,
    prelude::*,
    uaccess::{UserPtr, UserSlice},
};

pub(crate) use kernel::uapi::{
    drm_evdi_connect as DrmEvdiConnect, drm_evdi_ddcci_response as DrmEvdiDdcciResponse,
    drm_evdi_enable_cursor_events as DrmEvdiEnableCursorEvents, drm_evdi_grabpix as DrmEvdiGrabpix,
    drm_evdi_request_update as DrmEvdiRequestUpdate, DRM_EVDI_EVENT_CURSOR_MOVE,
    DRM_EVDI_EVENT_CURSOR_SET, DRM_EVDI_EVENT_DPMS, DRM_EVDI_EVENT_MODE_CHANGED,
    DRM_EVDI_EVENT_UPDATE_READY,
};

pub(crate) type DrmEvent = kernel::drm::event::EventHeader;

// Local payload types are needed to implement the driver's sealed EventPayload trait. Their
// declarations are checked against the UAPI layout by declare_drm_event_payload! in painter.rs.
#[repr(C)]
#[derive(Copy, Clone)]
pub(crate) struct DrmEvdiEventUpdateReady {
    pub(crate) base: DrmEvent,
}

#[repr(C)]
#[derive(Copy, Clone)]
pub(crate) struct DrmEvdiEventDpms {
    pub(crate) base: DrmEvent,
    pub(crate) mode: i32,
}

#[repr(C)]
#[derive(Copy, Clone)]
pub(crate) struct DrmEvdiEventModeChanged {
    pub(crate) base: DrmEvent,
    pub(crate) hdisplay: i32,
    pub(crate) vdisplay: i32,
    pub(crate) vrefresh: i32,
    pub(crate) bits_per_pixel: i32,
    pub(crate) pixel_format: u32,
}

#[repr(C)]
#[derive(Copy, Clone)]
pub(crate) struct DrmEvdiEventCursorSet {
    pub(crate) base: DrmEvent,
    pub(crate) hot_x: i32,
    pub(crate) hot_y: i32,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) enabled: u8,
    /// Explicit ABI padding. `enabled` is a `__u8` followed by a `__u32` in the C struct, so three
    /// bytes sit between them and are copied to userspace with the rest; naming them keeps the
    /// payload provably padding-free and forces callers to zero them rather than leaking stack.
    pub(crate) _pad: [u8; 3],
    pub(crate) buffer_handle: u32,
    pub(crate) buffer_length: u32,
    pub(crate) pixel_format: u32,
    pub(crate) stride: u32,
}

#[repr(C)]
#[derive(Copy, Clone)]
pub(crate) struct DrmEvdiEventCursorMove {
    pub(crate) base: DrmEvent,
    pub(crate) x: i32,
    pub(crate) y: i32,
}

const _: () = {
    assert!(
        core::mem::size_of::<DrmEvdiEventCursorSet>()
            == core::mem::size_of::<kernel::uapi::drm_evdi_event_cursor_set>()
    );
    assert!(
        core::mem::size_of::<DrmEvdiEventCursorMove>()
            == core::mem::size_of::<kernel::uapi::drm_evdi_event_cursor_move>()
    );
    assert!(
        core::mem::size_of::<DrmEvdiEventUpdateReady>()
            == core::mem::size_of::<kernel::uapi::drm_evdi_event_update_ready>()
    );
    assert!(
        core::mem::size_of::<DrmEvdiEventDpms>()
            == core::mem::size_of::<kernel::uapi::drm_evdi_event_dpms>()
    );
    assert!(
        core::mem::size_of::<DrmEvdiEventModeChanged>()
            == core::mem::size_of::<kernel::uapi::drm_evdi_event_mode_changed>()
    );
};

pub(crate) const EVDI_GRABPIX_MODE_DIRTY: u32 =
    kernel::uapi::drm_evdi_grabpix_mode_EVDI_GRABPIX_MODE_DIRTY;

/// 32-bit layout of `drm_evdi_connect`.
#[repr(C)]
pub(crate) struct DrmEvdiConnect32 {
    connected: i32,
    dev_index: i32,
    edid: u32,
    edid_length: u32,
    pixel_area_limit: u32,
    pixel_per_second_limit: u32,
}

impl CompatIoctl for DrmEvdiConnect32 {
    type Native = DrmEvdiConnect;

    fn read_from_user(ptr: UserPtr) -> Result<Self> {
        let mut reader = UserSlice::new(ptr, core::mem::size_of::<Self>()).reader();
        Ok(Self {
            connected: reader.read()?,
            dev_index: reader.read()?,
            edid: reader.read()?,
            edid_length: reader.read()?,
            pixel_area_limit: reader.read()?,
            pixel_per_second_limit: reader.read()?,
        })
    }

    fn into_native(&self) -> Self::Native {
        DrmEvdiConnect {
            connected: self.connected,
            dev_index: self.dev_index,
            edid: self.edid as usize as *const u8,
            edid_length: self.edid_length,
            pixel_area_limit: self.pixel_area_limit,
            pixel_per_second_limit: self.pixel_per_second_limit,
        }
    }
}

/// 32-bit layout of `drm_evdi_grabpix`.
#[repr(C)]
pub(crate) struct DrmEvdiGrabpix32 {
    mode: u32,
    buf_width: i32,
    buf_height: i32,
    buf_byte_stride: i32,
    buffer: u32,
    num_rects: i32,
    rects: u32,
}

const _: () = {
    assert!(core::mem::size_of::<DrmEvdiConnect32>() == 24);
    assert!(core::mem::size_of::<DrmEvdiGrabpix32>() == 28);
};

impl CompatIoctl for DrmEvdiGrabpix32 {
    type Native = DrmEvdiGrabpix;

    fn read_from_user(ptr: UserPtr) -> Result<Self> {
        let mut reader = UserSlice::new(ptr, core::mem::size_of::<Self>()).reader();
        Ok(Self {
            mode: reader.read()?,
            buf_width: reader.read()?,
            buf_height: reader.read()?,
            buf_byte_stride: reader.read()?,
            buffer: reader.read()?,
            num_rects: reader.read()?,
            rects: reader.read()?,
        })
    }

    fn into_native(&self) -> Self::Native {
        DrmEvdiGrabpix {
            mode: self.mode,
            buf_width: self.buf_width,
            buf_height: self.buf_height,
            buf_byte_stride: self.buf_byte_stride,
            buffer: self.buffer as usize as *mut u8,
            num_rects: self.num_rects,
            rects: self.rects as usize as *mut kernel::uapi::drm_clip_rect,
        }
    }

    fn write_back(&mut self, native: &Self::Native, ptr: UserPtr) -> Result {
        self.num_rects = native.num_rects;
        UserSlice::new(ptr.wrapping_byte_add(20), core::mem::size_of::<i32>())
            .writer()
            .write_slice(&self.num_rects.to_ne_bytes())
    }
}
