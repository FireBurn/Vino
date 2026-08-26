// SPDX-License-Identifier: GPL-2.0
//
// EVDI driver-private ioctls (libevdi / DisplayLinkManager entry points), declared via
// `kernel::declare_drm_ioctls!` in `kms.rs`. Each handler runs inside a
// `drm_dev_enter/exit` critical section (returns `ENODEV` if the card was unplugged).

use kernel::{
    drm,
    io::Io as _,
    prelude::*,
    uaccess::{UserPtr, UserSlice},
};

use crate::kms::{EvdiDrmData, EvdiDrmDriver, EvdiDrmFile};
use crate::uapi;

/// Maximum EDID blob we accept from userspace (128-byte base block + up to 255 extensions).
const EDID_MAX: usize = 32 * 1024;

type Dev = drm::Device<EvdiDrmDriver>;
type File = drm::File<EvdiDrmFile>;

/// `DRM_IOCTL_EVDI_CONNECT`: the DLM client connects (with an EDID) or disconnects the
/// virtual display. Registers/clears the event receiver for this device.
pub(crate) fn connect(
    dev: &Dev,
    _reg: &(),
    arg: &mut uapi::DrmEvdiConnect,
    file: &File,
) -> Result<u32> {
    let data: &EvdiDrmData = dev;
    if arg.connected != 0 {
        // Copy the EDID the client supplied so the connector's mode list reflects the real
        // monitor. A zero-length/NULL EDID is allowed (connector falls back to a default mode).
        let len = arg.edid_length as usize;
        let mut edid = KVec::new();
        if len > 0 && !arg.edid.is_null() {
            if len > EDID_MAX {
                return Err(EINVAL);
            }
            UserSlice::new(UserPtr::from_ptr(arg.edid.cast_mut().cast()), len)
                .read_all(&mut edid, GFP_KERNEL)?;
        }

        // Register this file as the event receiver, record the client's bandwidth limits, and
        // publish the EDID. The hotplug causes modes to be checked against the new limits.
        let connection = data.events.connect(dev, file)?;
        *file.inner().connection.lock() = Some(connection);
        data.set_mode_limits(arg.pixel_area_limit, arg.pixel_per_second_limit);
        if !edid.is_empty() {
            data.set_edid(dev, edid);
        }
        // Tell this client what the display is already doing. A card the compositor configured
        // before anyone connected -- a client restarting against a card that outlived it, most
        // obviously -- has already sent its MODE_CHANGED, and nothing would ever send another.
        data.replay_state();
    } else {
        *file.inner().connection.lock() = None;
        // Drop the EDID so the connector reports disconnected until the next CONNECT.
        data.clear_edid(dev);
    }
    Ok(0)
}

/// `DRM_IOCTL_EVDI_REQUEST_UPDATE`: the client asks to be told (via UPDATE_READY) when the
/// next frame is ready to grab.
pub(crate) fn request_update(
    dev: &Dev,
    _reg: &(),
    _arg: &mut uapi::DrmEvdiRequestUpdate,
    _file: &File,
) -> Result<u32> {
    // If an ungrabbed frame is already waiting, return 1 so the client
    // grabs immediately (`grabImmediately`). Otherwise the next flip's
    // UPDATE_READY event wakes it. Sending an event here would create an
    // unbounded request/event/grab cycle without a new frame.
    let data: &EvdiDrmData = dev;
    if data.painter.lock().frame_dirty {
        Ok(1)
    } else {
        Ok(0)
    }
}

/// `DRM_IOCTL_EVDI_GRABPIX`: copy the current scanout framebuffer to the client's buffer.
///
/// The pixels are read through the prepared shmem mapping and its `SysMem` view, then written to
/// userspace one row at a time (bounded by both the framebuffer and the client's buffer geometry).
pub(crate) fn grabpix(
    dev: &Dev,
    _reg: &(),
    arg: &mut uapi::DrmEvdiGrabpix,
    _file: &File,
) -> Result<u32> {
    // Every format the primary plane advertises is 32 bits per pixel, packed.
    const BPP: usize = 4;

    // MODE_RECTS would have the client hand us the rectangles to copy; only MODE_DIRTY, where
    // the driver reports the damage it accumulated, is implemented.
    if arg.mode != uapi::EVDI_GRABPIX_MODE_DIRTY {
        return Err(EINVAL);
    }
    if arg.num_rects < 1 {
        return Err(EINVAL);
    }

    let data: &EvdiDrmData = dev;
    // No frame has been flipped in yet (or the pipe is down, e.g. DPMS off): report zero rects.
    // Not -EAGAIN -- libevdi's `drm_ioctl` retries EAGAIN in a tight loop, which would busy-spin
    // a core until the next flip.
    let Some(scanout) = data.prepared_scanout() else {
        arg.num_rects = 0;
        return Ok(0);
    };
    let fb = &scanout.framebuffer;

    // Take the regions accumulated since the last grab and mark the frame consumed.
    let mut dmg = {
        let mut p = data.painter.lock();
        p.frame_dirty = false;
        let d = p.damage;
        p.damage.clear();
        d
    };

    let map = &scanout.mapping;
    let src_len = map.view().len();
    let src_pitch = map.pitch();
    let fb_w = fb.width() as i32;
    let fb_h = fb.height() as i32;

    // The geometry is client-controlled: reject impossible values before any of it reaches
    // arithmetic (a negative stride cast to usize is ~2^63 and `y * dst_stride` wraps -- a
    // kernel panic with overflow checks on).
    if arg.buffer.is_null() || arg.buf_byte_stride <= 0 || arg.buf_width < 0 || arg.buf_height < 0 {
        return Err(EINVAL);
    }
    let dst_stride = arg.buf_byte_stride as usize;
    let max_w = core::cmp::min(fb_w, (dst_stride / BPP) as i32);
    let max_h = core::cmp::min(fb_h, arg.buf_height);

    // Nothing recorded (the client polled without a flip): fall back to one full-frame rectangle.
    if dmg.count == 0 {
        dmg.rects[0] = (0, 0, fb_w, fb_h);
        dmg.count = 1;
    }

    // Clamp each rectangle to the framebuffer and the client's buffer, dropping empty ones.
    let mut rects = [(0i32, 0i32, 0i32, 0i32); crate::painter::MAX_DAMAGE_RECTS];
    let mut count = 0usize;
    for &(rx1, ry1, rx2, ry2) in &dmg.rects[..dmg.count] {
        let x1 = rx1.clamp(0, max_w);
        let y1 = ry1.clamp(0, max_h);
        let x2 = rx2.clamp(x1, max_w);
        let y2 = ry2.clamp(y1, max_h);
        if x2 > x1 && y2 > y1 {
            rects[count] = (x1, y1, x2, y2);
            count += 1;
        }
    }
    if count == 0 {
        // The arg is copied back to userspace as passed in, so zero the count explicitly --
        // otherwise the client sees its own pre-filled value (MAX_DIRTS) and reports that many
        // empty rectangles.
        arg.num_rects = 0;
        return Ok(0);
    }
    // The client's `rects` buffer holds `num_rects` entries; if we somehow have more, coalesce into
    // a single bounding box so we never overrun it.
    let cap = (arg.num_rects.max(1) as usize).min(crate::painter::MAX_DAMAGE_RECTS);
    if count > cap {
        let mut bb = rects[0];
        for &(x1, y1, x2, y2) in &rects[1..count] {
            bb = (bb.0.min(x1), bb.1.min(y1), bb.2.max(x2), bb.3.max(y2));
        }
        rects[0] = bb;
        count = 1;
    }

    // Report the rectangles so DLM transmits exactly them
    // (`struct drm_clip_rect` = 4x u16). An empty rectangle array tells DLM
    // that nothing changed.
    if !arg.rects.is_null() {
        let mut buf = [0u8; crate::painter::MAX_DAMAGE_RECTS * 8];
        for (i, &(x1, y1, x2, y2)) in rects[..count].iter().enumerate() {
            let o = i * 8;
            buf[o..o + 2].copy_from_slice(&(x1 as u16).to_ne_bytes());
            buf[o + 2..o + 4].copy_from_slice(&(y1 as u16).to_ne_bytes());
            buf[o + 4..o + 6].copy_from_slice(&(x2 as u16).to_ne_bytes());
            buf[o + 6..o + 8].copy_from_slice(&(y2 as u16).to_ne_bytes());
        }
        UserSlice::new(UserPtr::from_ptr(arg.rects.cast()), count * 8)
            .writer()
            .write_slice(&buf[..count * 8])?;
    }
    // The IOWR arg is copied back to userspace, so the count reaches DLM.
    arg.num_rects = count as i32;

    // The CRTC's colour transform, snapshotted once so the per-row loop holds no lock. `None`
    // (no CTM and no gamma ramp programmed) leaves the copy exactly as it was.
    let color = *data.color.lock();

    // Copy each changed rectangle into the client's (persistent, full-frame) buffer at the same
    // position, so the userspace frame accumulates the per-grab deltas.
    for &(x1, y1, x2, y2) in &rects[..count] {
        let xoff = x1 as usize * BPP;
        let span = (x2 - x1) as usize * BPP;
        let mut row = KVec::new();
        row.resize(span, 0, GFP_KERNEL)?;
        for y in y1 as usize..y2 as usize {
            let so = y * src_pitch + xoff;
            let end = so.checked_add(span).ok_or(EINVAL)?;
            if end > src_len {
                break;
            }
            let src = kernel::io_project!(map.view(), [try: so..end]);
            src.copy_to_slice(&mut row);
            if let Some(pipeline) = &color {
                // XRGB8888 little-endian: B, G, R, X. The X byte is left alone -- the format
                // carries no alpha, and the client may rely on what it holds.
                for px in row.chunks_exact_mut(BPP) {
                    let (r, g, b) = pipeline.apply(px[2], px[1], px[0]);
                    px[2] = r;
                    px[1] = g;
                    px[0] = b;
                }
            }
            // Fully checked: on 32-bit `usize` the row product can overflow even after the
            // sign validation above.
            let dst = y
                .checked_mul(dst_stride)
                .and_then(|off| off.checked_add(xoff))
                .and_then(|off| (arg.buffer as usize).checked_add(off))
                .ok_or(EINVAL)?;
            UserSlice::new(UserPtr::from_ptr(dst as *mut kernel::ffi::c_void), span)
                .writer()
                .write_slice(&row)?;
        }
    }
    Ok(0)
}

/// `DRM_IOCTL_EVDI_DDCCI_RESPONSE`: accept a DDC/CI response from an EVDI client.
pub(crate) fn ddcci_response(
    _dev: &Dev,
    _reg: &(),
    _arg: &mut uapi::DrmEvdiDdcciResponse,
    _file: &File,
) -> Result<u32> {
    // DDC/CI forwarding is optional in the EVDI ABI. Accept an unsolicited
    // response for compatibility when this device has no virtual I2C bus.
    Ok(0)
}

/// `DRM_IOCTL_EVDI_ENABLE_CURSOR_EVENTS`: opt in to (or out of) cursor reporting.
///
/// While disabled, the cursor plane stays silent and the compositor's own composition of the
/// pointer into the primary framebuffer is what the client grabs. While enabled, the pointer is
/// reported separately and the client drives its sink's cursor with it.
pub(crate) fn enable_cursor_events(
    dev: &Dev,
    _reg: &(),
    arg: &mut uapi::DrmEvdiEnableCursorEvents,
    _file: &File,
) -> Result<u32> {
    let data: &crate::kms::EvdiDrmData = dev;
    data.cursor_events
        .store(arg.enable != 0, core::sync::atomic::Ordering::Relaxed);
    Ok(0)
}
