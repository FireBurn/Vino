// SPDX-License-Identifier: GPL-2.0-or-later

//! Revdi framebuffer source for the Chimera daemon.
//!
//! Handle ownership and the C ABI boundary live in `librevdi`'s safe Rust client. Chimera only
//! deals in borrowed coherent frames.

pub use evdi::safe::{Cursor, DamageRect, Device as RevdiCard, DeviceEvent, Error, Frame, Mode};

/// Connect a Revdi output using the dock sink's EDID.
pub fn connect_monitor(card: &mut RevdiCard, edid: &[u8]) -> Result<(), Error> {
    const MAX_PIXEL_AREA: u32 = 3840 * 2160;
    card.connect(edid, MAX_PIXEL_AREA, MAX_PIXEL_AREA * 60)
}

/// Convert a coherent XRGB8888 framebuffer to the packed RGB raster used by Vino's codec.
pub fn to_rgb888(frame: &Frame<'_>) -> Vec<u8> {
    let mut out = vec![0u8; frame.width * frame.height * 3];
    for y in 0..frame.height {
        for x in 0..frame.width {
            let source = y * frame.stride + x * 4;
            let target = (y * frame.width + x) * 3;
            let Some(pixel) = frame.pixels.get(source..source + 4) else {
                continue;
            };
            let pixel = u32::from_le_bytes(pixel.try_into().unwrap());
            out[target] = ((pixel >> 16) & 0xff) as u8;
            out[target + 1] = ((pixel >> 8) & 0xff) as u8;
            out[target + 2] = (pixel & 0xff) as u8;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_xrgb8888_with_stride() {
        let pixels = [
            0x33, 0x22, 0x11, 0xff, 0x66, 0x55, 0x44, 0xff, 0xaa, 0xaa, 0xaa, 0xaa,
        ];
        let frame = Frame {
            pixels: &pixels,
            width: 2,
            height: 1,
            stride: 12,
            damage: Vec::new(),
        };
        assert_eq!(to_rgb888(&frame), [0x11, 0x22, 0x33, 0x44, 0x55, 0x66]);
    }
}
