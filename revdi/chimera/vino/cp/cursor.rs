// SPDX-License-Identifier: GPL-2.0

//! The dock-composited cursor.
//!
//! A cursor image is one control message carrying the whole 64x64 premultiplied bitmap. Only a
//! dock with a video pipe of its own is offered the plane; where control and pixels share an
//! endpoint the vendor sends no cursor message at all and draws the pointer into the frame.

use super::*;

/// The dock's connector id at off22 of every cursor message, indexed by vino's connector number.
///
/// Cursor wire layout (sec 8.6.1). All three messages share the 32-byte inner header built by
/// [`cursor_header`], with the connector selector at off22 and a flag at off23.
///
/// The selector is a connector bitmask, `1 << connector`; the dock numbers its connectors from one,
/// so `0` is never valid. The two measured entries were `[0x01, 0x02]`, which is both `1 <<
/// connector` and `connector + 1`, so they do not distinguish the two readings. They diverge from
/// connector 2 on, and a connector sent `connector + 1` draws no cursor.
fn cursor_head_id(connector: u8) -> Result<u8> {
    if usize::from(connector) >= crate::drm_sink::MAX_CONNECTORS {
        return Err(EINVAL);
    }
    Ok(1u8 << connector)
}
/// Common prologue of the cursor messages: the dock-side connector id at offset 22 and the
/// visibility flag at offset 23.
fn cursor_header(
    b: &mut KVec<u8>,
    id: u16,
    sub: u16,
    counter: u16,
    dock_connector: u8,
    visible: u8,
) -> Result {
    header(b, id, sub, counter)?;
    pad_to(b, 22)?;
    b.push(dock_connector, GFP_KERNEL)?;
    b.push(visible, GFP_KERNEL)?;
    Ok(())
}
/// cursor create: `id=0x1b sub=0x42`, advertises `w x h`. Sent once per bitmap geometry.
pub(crate) fn cursor_create(counter: u16, connector: u8, w: u16, h: u16) -> Result<KVec<u8>> {
    let mut b = KVec::with_capacity(32, GFP_KERNEL)?;
    let dock_connector = cursor_head_id(connector)?;
    cursor_header(&mut b, 0x1b, 0x42, counter, dock_connector, CURSOR_HIDDEN)?;
    b.extend_from_slice(&w.to_le_bytes(), GFP_KERNEL)?; // off24..25
    b.extend_from_slice(&h.to_le_bytes(), GFP_KERNEL)?; // off26..27
    pad_to(&mut b, 32)?; // off28..31 reserved
    Ok(b)
}
/// cursor move: `id=0x1a sub=0x43`, X at off24 and Y at off26 (LE), for one connector.
pub(crate) fn cursor_move(
    counter: u16,
    connector: u8,
    x: u16,
    y: u16,
    visible: bool,
) -> Result<KVec<u8>> {
    let mut b = KVec::with_capacity(32, GFP_KERNEL)?;
    let dock_connector = cursor_head_id(connector)?;
    let visible_flag = if visible {
        CURSOR_VISIBLE
    } else {
        CURSOR_HIDDEN
    };
    cursor_header(&mut b, 0x1a, 0x43, counter, dock_connector, visible_flag)?;
    b.extend_from_slice(&x.to_le_bytes(), GFP_KERNEL)?; // off24..25
    b.extend_from_slice(&y.to_le_bytes(), GFP_KERNEL)?; // off26..27
    pad_to(&mut b, 32)?; // off28..31 reserved
    Ok(b)
}
/// cursor image: inner `id=0x401c sub=0x41` (the `0x40` high-byte flag marks the bitmap-bearing
/// message), a 32-byte header then the bitmap. `w`/`h` come from [`cursor_create`].
///
/// Pixels are DRM `ARGB8888` (`[B, G, R, A]`, premultiplied) and start at off34: off32..33 are
/// zero and the final pixel is truncated, so the message stays `32 + w*h*4` bytes.
pub(crate) fn cursor_image(
    counter: u16,
    connector: u8,
    w: u16,
    h: u16,
    bgra: &[u8],
) -> Result<KVec<u8>> {
    // `w*h*4` can wrap a 32-bit `usize` (max ~1.7e10 > u32::MAX), which would let an
    // undersized `bgra` pass the check; compute it with checked arithmetic so an
    // overflow is rejected as a mismatch rather than silently bypassing validation.
    let expected = (w as usize)
        .checked_mul(h as usize)
        .and_then(|n| n.checked_mul(4));
    if expected != Some(bgra.len()) {
        return Err(EINVAL);
    }
    let mut b = KVec::with_capacity(32 + bgra.len(), GFP_KERNEL)?;
    let dock_connector = cursor_head_id(connector)?;
    cursor_header(&mut b, 0x401c, 0x41, counter, dock_connector, CURSOR_HIDDEN)?;
    pad_to(&mut b, 32)?; // off24..31 zero (no w/h here)
    b.extend_from_slice(&[0, 0], GFP_KERNEL)?; // off32..33
    b.extend_from_slice(&bgra[..bgra.len() - 2], GFP_KERNEL)?; // pixels @ off34
    Ok(b)
}

#[cfg(CONFIG_DRM_VINO_KUNIT_TEST)]
#[kunit_tests(vino_cp_cursor)]
mod tests {
    use super::*;

    #[test]
    fn cursor_messages_structure() -> Result {
        // Shared 32-byte cursor layout: the dock's connector selector at 22, the visible flag at
        // 23, and two little-endian u16 fields at 24 and 26. Check the selector against more than
        // one connector: `1 << connector` and a fixed byte agree for a single connector and diverge
        // past it. Create (connector 0): id=0x1b sub=0x42, fields = w,h. An upload is not a show,
        // so hidden.
        let c = cursor_create(7, 0, 64, 64)?;
        assert_eq!(c.len(), 32);
        assert_eq!(&c[0..6], &[0x1b, 0x00, 0x42, 0x00, 0x07, 0x00]); // id, sub, counter (LE)
        assert_eq!(c[22], 0x01); // connector 0 -> dock connector 1
        assert_eq!(c[23], 0x00); // not visible
        assert_eq!(u16::from_le_bytes([c[24], c[25]]), 64); // width
        assert_eq!(u16::from_le_bytes([c[26], c[27]]), 64); // height

        // Move (connector 1): id=0x1a sub=0x43, connector@22, visible@23, X@24, Y@26 (LE).
        let m = cursor_move(9, 1, 0x0140, 0x00f0, true)?;
        assert_eq!(m.len(), 32);
        assert_eq!(&m[0..4], &[0x1a, 0x00, 0x43, 0x00]); // id, sub
        assert_eq!(m[22], 0x02); // connector 1 -> dock connector 2
        assert_eq!(m[23], 0x01); // visible
        assert_eq!(u16::from_le_bytes([m[24], m[25]]), 0x0140); // X
        assert_eq!(u16::from_le_bytes([m[26], m[27]]), 0x00f0); // Y

        // Every connector this driver exposes must produce a message. A two-entry lookup table left
        // the DL7400's third and fourth connectors returning `EINVAL`, which `cmd_work` drops
        // rather than retries -- so a monitor in socket 3 or 4 had no hardware cursor at all. The
        // selector is a bitmask, not a one-based index: the original two-entry table was `[0x01,
        // 0x02]`, which is `1 << connector` for the only two connectors that dock had.
        for connector in 0..drm_sink::MAX_CONNECTORS as u8 {
            let m = cursor_move(1, connector, 0, 0, true)?;
            assert_eq!(m[22], 1u8 << connector);
        }
        assert!(cursor_move(1, drm_sink::MAX_CONNECTORS as u8, 0, 0, true).is_err());

        // Image: 32-byte header (inner id 0x401c, the 0x40 bitmap flag) + w*h*4 BGRA at off32;
        // wrong-size input rejected.
        let bitmap = KVec::from_elem(0xabu8, 64 * 64 * 4, GFP_KERNEL)?;
        let img = cursor_image(3, 0, 64, 64, &bitmap)?;
        assert_eq!(img.len(), 32 + 64 * 64 * 4);
        assert_eq!(&img[0..4], &[0x1c, 0x40, 0x41, 0x00]); // inner id 0x401c, sub 0x41
        assert_eq!(img[22], 0x01); // connector 0 -> dock connector 1
                                   // The bitmap begins at off34, not off32: offsets 32..33 are zero
                                   // and the last pixel is truncated so the message still measures
                                   // `32 + w*h*4`.
        assert_eq!(&img[32..34], &[0x00, 0x00]);
        assert_eq!(img[34], 0xab);
        assert!(cursor_image(3, 0, 64, 64, &[0u8; 16]).is_err()); // wrong bitmap length
        Ok(())
    }
}
