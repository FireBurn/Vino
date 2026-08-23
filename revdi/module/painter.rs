// SPDX-License-Identifier: GPL-2.0
//
// The EVDI "painter": the per-device connection + event-delivery bookkeeping that
// bridges the KMS callbacks (and ioctls) to the DisplayLinkManager userspace client.
//
// Events are delivered through the DRM-core `drm_event` mechanism (the safe
// `kernel::drm::event::EventChannel` binding), which serializes delivery against file
// close under `event_lock` -- so an event can never be sent to a client that has just
// disconnected.

use kernel::drm::gem::BaseObject;

use crate::kms::{EvdiDrmData, EvdiDrmDevice};
use crate::uapi;

/// DPMS mode codes as understood by the DLM client (matching `DRM_MODE_DPMS_*`).
pub(crate) const DPMS_ON: i32 = 0;
pub(crate) const DPMS_OFF: i32 = 3;

/// Mutable per-device painter state, guarded by a mutex in [`EvdiDrmData`].
///
/// Neither of the two things a client connection implies lives here: its EDID is the connector's
/// `cached_edid` (the source of truth for the mode list), and whether anyone is attached is
/// [`EventChannel::is_connected`].
pub(crate) struct PainterState {
    /// A frame has been flipped in but not yet grabbed (the C evdi's `num_dirts > 0`). Lets
    /// REQUEST_UPDATE answer "grab now" (ioctl returns 1) when fresh pixels are already waiting,
    /// instead of self-triggering an UPDATE_READY event (which busy-loops the client).
    pub(crate) frame_dirty: bool,
    /// Regions changed since the last GRABPIX, accumulated across flips.
    ///
    /// GRABPIX reports and copies these rectangles, then clears them.
    pub(crate) damage: Damage,
}

/// Maximum number of distinct damage rectangles tracked between grabs (mirrors the C evdi's
/// `MAX_DIRTS`); on overflow they collapse into a single bounding box.
pub(crate) const MAX_DAMAGE_RECTS: usize = 16;

/// Accumulated frame damage: up to [`MAX_DAMAGE_RECTS`] changed rectangles `(x1, y1, x2, y2)` since
/// the last GRABPIX. `count == 0` means nothing was recorded (GRABPIX falls back to a full frame).
#[derive(Copy, Clone)]
pub(crate) struct Damage {
    pub(crate) rects: [(i32, i32, i32, i32); MAX_DAMAGE_RECTS],
    pub(crate) count: usize,
}

impl Damage {
    pub(crate) const fn new() -> Self {
        Self {
            rects: [(0, 0, 0, 0); MAX_DAMAGE_RECTS],
            count: 0,
        }
    }

    /// Record a changed rectangle.
    ///
    /// When full, collapse the list and `r` into a single bounding box.
    pub(crate) fn push(&mut self, r: (i32, i32, i32, i32)) {
        if self.count < MAX_DAMAGE_RECTS {
            self.rects[self.count] = r;
            self.count += 1;
            return;
        }
        let mut bb = r;
        for i in 0..self.count {
            let (x1, y1, x2, y2) = self.rects[i];
            bb = (bb.0.min(x1), bb.1.min(y1), bb.2.max(x2), bb.3.max(y2));
        }
        self.rects[0] = bb;
        self.count = 1;
    }

    pub(crate) fn clear(&mut self) {
        self.count = 0;
    }
}

impl PainterState {
    pub(crate) fn new() -> Self {
        Self {
            frame_dirty: false,
            damage: Damage::new(),
        }
    }
}

kernel::declare_drm_event_payload!(
    uapi::DrmEvdiEventUpdateReady,
    uapi::DRM_EVDI_EVENT_UPDATE_READY,
    []
);
kernel::declare_drm_event_payload!(uapi::DrmEvdiEventDpms, uapi::DRM_EVDI_EVENT_DPMS, [i32]);
kernel::declare_drm_event_payload!(
    uapi::DrmEvdiEventModeChanged,
    uapi::DRM_EVDI_EVENT_MODE_CHANGED,
    [i32, i32, i32, i32, u32]
);
kernel::declare_drm_event_payload!(
    uapi::DrmEvdiEventCursorSet,
    uapi::DRM_EVDI_EVENT_CURSOR_SET,
    [i32, i32, u32, u32, u8, [u8; 3], u32, u32, u32, u32]
);
kernel::declare_drm_event_payload!(
    uapi::DrmEvdiEventCursorMove,
    uapi::DRM_EVDI_EVENT_CURSOR_MOVE,
    [i32, i32]
);
/// Zeroed `drm_event` header; [`EventChannel::send`] overwrites `type`/`length`.
const fn hdr() -> uapi::DrmEvent {
    uapi::DrmEvent {
        type_: 0,
        length: 0,
    }
}

/// Tell the DLM client a fresh frame is ready to be grabbed (`UPDATE_READY`).
pub(crate) fn notify_update_ready(data: &EvdiDrmData) {
    let ev = uapi::DrmEvdiEventUpdateReady { base: hdr() };
    let _ = data.events.send(ev);
}

/// Tell the DLM client the display's DPMS power state changed.
pub(crate) fn notify_dpms(data: &EvdiDrmData, mode: i32) {
    let ev = uapi::DrmEvdiEventDpms { base: hdr(), mode };
    let _ = data.events.send(ev);
}

/// Tell the DLM client the negotiated mode changed.
pub(crate) fn notify_mode_changed(
    data: &EvdiDrmData,
    hdisplay: i32,
    vdisplay: i32,
    vrefresh: i32,
    bits_per_pixel: i32,
    pixel_format: u32,
) {
    let ev = uapi::DrmEvdiEventModeChanged {
        base: hdr(),
        hdisplay,
        vdisplay,
        vrefresh,
        bits_per_pixel,
        pixel_format,
    };
    let _ = data.events.send(ev);
}

/// Cursor geometry and pixel layout accompanying a bitmap change.
pub(crate) struct CursorShape {
    pub(crate) hot_x: i32,
    pub(crate) hot_y: i32,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) pixel_format: u32,
    pub(crate) stride: u32,
    pub(crate) buffer_length: u32,
}

/// Hand the client a fresh GEM handle for the cursor buffer and tell it the new shape.
///
/// The handle must be minted in the *client's* file, which is why this goes through
/// [`EventChannel::with_connected_file`] rather than [`EventChannel::send`]: creating a handle
/// allocates and takes mutexes, so it cannot run under `event_lock`. libevdi maps the handle,
/// copies the bitmap out and closes it, so a fresh handle is minted per change.
pub(crate) fn notify_cursor_set(
    data: &EvdiDrmData,
    dev: &EvdiDrmDevice,
    object: &kernel::drm::gem::shmem::Object<crate::kms::EvdiObject>,
    shape: &CursorShape,
) {
    let handle = data
        .events
        .with_connected_file(dev, |file| object.create_handle(file));
    let Ok(Some(buffer_handle)) = handle else {
        return;
    };
    let ev = uapi::DrmEvdiEventCursorSet {
        base: hdr(),
        hot_x: shape.hot_x,
        hot_y: shape.hot_y,
        width: shape.width,
        height: shape.height,
        enabled: 1,
        _pad: [0; 3],
        buffer_handle,
        buffer_length: shape.buffer_length,
        pixel_format: shape.pixel_format,
        stride: shape.stride,
    };
    let _ = data.events.send(ev);
}

/// Tell the client the cursor is no longer visible. No buffer accompanies this.
pub(crate) fn notify_cursor_disabled(data: &EvdiDrmData) {
    let ev = uapi::DrmEvdiEventCursorSet {
        base: hdr(),
        hot_x: 0,
        hot_y: 0,
        width: 0,
        height: 0,
        enabled: 0,
        _pad: [0; 3],
        buffer_handle: 0,
        buffer_length: 0,
        pixel_format: 0,
        stride: 0,
    };
    let _ = data.events.send(ev);
}

/// Tell the client the cursor moved. Position changes are far more frequent than shape changes and
/// carry no buffer, so they never need the client's file.
pub(crate) fn notify_cursor_move(data: &EvdiDrmData, x: i32, y: i32) {
    let ev = uapi::DrmEvdiEventCursorMove { base: hdr(), x, y };
    let _ = data.events.send(ev);
}
