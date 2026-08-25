// SPDX-License-Identifier: GPL-2.0

//! DisplayLink full-colour video encoder and framing.

use super::*;

/// Pack 8-bit RGB into RGB565.
#[cfg(CONFIG_DRM_VINO_KUNIT_TEST)]
pub(crate) fn rgb565(r: u8, g: u8, b: u8) -> u16 {
    ((r as u16 >> 3) << 11) | ((g as u16 >> 2) << 5) | (b as u16 >> 3)
}

/// DisplayLink's 8x8 multilevel Haar codec and 64x16 strip grammar.
pub(crate) mod haar {
    mod records;
    mod strip;
    mod transform;

    pub(crate) use records::*;
    pub(crate) use strip::*;
    pub(crate) use transform::*;

    use super::*;

    /// Per-coefficient `(shift, bias)` quantization parameters.
    ///
    /// All steps are powers of two. Arithmetic right shift therefore implements the required floor
    /// division for both positive and negative coefficients without an integer division.
    const fn step_bias(i: usize) -> (u32, i32) {
        match i {
            0 => (4, 8),
            1 | 2 => (4, 8),
            3 => (5, 16),
            4..=11 => (2, 2),
            12..=15 => (3, 4),
            16..=47 => (1, 0),
            _ => (2, 2), // 48..=63
        }
    }

    /// [`step_bias`] and the chroma-AC shift, resolved for every coefficient at compile time.
    ///
    /// Both are pure functions of the coefficient index over only 64 inputs, but they were being
    /// evaluated per coefficient -- 64 times per plane, three planes per block, sixteen blocks per
    /// strip. Profiling `colour_block` found the resulting range-test chains dominating it: 190
    /// compare and branch instructions against 61 arithmetic ones. A table turns each into a load.
    static STEP_BIAS: [(u32, i32); COEFFS] = {
        let mut t = [(0u32, 0i32); COEFFS];
        let mut i = 0;
        while i < COEFFS {
            t[i] = step_bias(i);
            i += 1;
        }
        t
    };

    static CHROMA_AC_SHIFT: [u32; COEFFS] = {
        let mut t = [0u32; COEFFS];
        let mut i = 0;
        while i < COEFFS {
            t[i] = if matches!(i, 1 | 2 | 4..=11) {
                4
            } else if i >= 48 {
                6
            } else {
                5
            };
            i += 1;
        }
        t
    };

    /// Quantize coefficient `coeff` at position `i`: `sign(coeff) * floor((|coeff| + bias) / step)`
    /// and clamp it to the 12-bit signed long-token range.
    pub(crate) fn quantize(coeff: i32, i: usize) -> i32 {
        let (shift, bias) = STEP_BIAS[i];
        // Coarse bands round half-up on the signed value; the finest bands truncate towards zero.
        let q = if bias == 0 {
            let q = coeff.abs() >> shift;
            if coeff < 0 {
                -q
            } else {
                q
            }
        } else {
            (coeff + bias) >> shift
        };
        q.clamp(-2048, 2047)
    }

    // The AC ceilings deliberately do not vary with depth. The DC one has to, because a DC is a
    // direct multiple of the sample and 10-bit content immediately overflows category 10. No
    // capture shows an AC ceiling above 8-bit's: the largest luma AC coefficient measured on
    // 10-bit content is |273|, comfortably inside category 9, because the host tone-maps to the
    // sink's peak luminance before the codec sees it. Guessing upward is the unsafe direction --
    // `esc` saturates a magnitude whose category exceeds the ceiling, so an under-sized ceiling
    // clips extreme AC detail while an over-sized one desynchronises the dock. Raise these only
    // against a capture that needs them.

    // Colour strip codec (Cb/Cr planes).
    //
    // Per block the 3 planes are (Cr=64*(B-G), Cb=64*(R-G), Y=64*G + 64*((Cb+Cr)>>2)).
    //  * SYNC unit = [Cr field][Cb field][Y field]; chroma fields present only when last>0
    //    (the per-block plane mask), Y field always present (luma `sync_unit`).
    //  * DC plane = 16-block DPCM (Cr,Cb,Y), 3 tokens/block, chroma step 64 / luma step 16,
    //    round-half-up on the signed value.
    //  * AC rows (row0 blocks 0..8, row1 8..16): per block (Cr,Cb,Y) present planes, chroma
    //    quant flat step 16 (truncate toward zero), positions 1..last, run-bit `0` for zeros.
    //  * Strip length = w1c + round_even(row1) (the 2-byte tail overlaps row1's tail).

    /// Encode a full `width`x`height` RGB frame into Haar colour records. `px(x, y)` yields the
    /// source pixel's `(R, G, B)`; the caller applies rotation, gamma and format conversion. The
    /// surface is tiled into 64x16 strips in raster order, each built from [`colour_block`] +
    /// [`colour_strip`], and the strip stream is framed for the wire by [`frame_records`] using
    /// the EP08 TLV layout. The frame counter belongs to the records that name a ring slot, not
    /// here.
    ///
    /// `width`/`height` must be multiples of 64 and 16 (`EINVAL` otherwise). Live scanout pads a
    /// non-aligned mode with black to this strip grid while preserving the real mode dimensions.
    /// This function remains out of line to bound kernel stack use.
    #[inline(never)]
    pub(crate) fn colour_frame_ep08(
        geometry: Geometry,
        width: usize,
        height: usize,
        connector: u8,
        mut px: impl FnMut(usize, usize) -> (u16, u16, u16),
    ) -> Result<KVec<KVec<u8>>> {
        colour_frame_ep08_variant(geometry, width, height, connector, None, &mut px)
    }

    /// Encode a full live Navarro frame using the ordinary-frame producer permutation measured
    /// from DLM. The first ordinary band is y=8, not y=0, and the captured worker boundaries are
    /// part of the record grammar for a 2560x1440 surface.
    pub(crate) fn colour_frame_ep08_navarro_ordinary(
        geometry: Geometry,
        width: usize,
        height: usize,
        connector: u8,
        mut px: impl FnMut(usize, usize) -> (u16, u16, u16),
    ) -> Result<KVec<KVec<u8>>> {
        colour_frame_ep08_variant(geometry, width, height, connector, Some(true), &mut px)
    }

    fn colour_frame_ep08_variant(
        geometry: Geometry,
        width: usize,
        height: usize,
        connector: u8,
        navarro_ordinary: Option<bool>,
        px: &mut impl FnMut(usize, usize) -> (u16, u16, u16),
    ) -> Result<KVec<KVec<u8>>> {
        if width & (geometry.strip_w() - 1) != 0 || height & (geometry.strip_h() - 1) != 0 {
            return Err(kernel::error::code::EINVAL);
        }
        // Build every strip body (raster order; each strip's natural row1 tail, no echo).
        let mut strips: KVec<KVec<u8>> = KVec::new();
        let mut sy = 0usize;
        while sy < height {
            let mut sx = 0usize;
            while sx < width {
                let blocks = colour_strip_blocks(geometry, sx, sy, px)?;
                strips.push(
                    colour_strip(geometry, &blocks, sx as u16, sy as u16)?,
                    GFP_KERNEL,
                )?;
                sx += geometry.strip_w();
            }
            sy += geometry.strip_h();
        }
        frame_records_with_boundary(
            geometry,
            &strips,
            connector,
            navarro_ordinary.filter(|_| strips.len() == 3600),
        )
    }

    /// Build a valid all-black Haar frame without sampling or transforming a framebuffer.
    ///
    /// This is the post-mode-set training carrier. Its zero-coefficient blocks are built once and
    /// reused, so construction is proportional to the strip grid rather than the pixel count. A
    /// real framebuffer keyframe follows.
    pub(crate) fn black_frame_ep08(
        geometry: Geometry,
        width: usize,
        height: usize,
        connector: u8,
    ) -> Result<KVec<KVec<u8>>> {
        black_frame_ep08_variant(geometry, width, height, connector, Some(false))
    }

    /// Build the ordinary Navarro black carrier which follows the prologue frame.
    ///
    /// DLM's ordinary 2560x1440 carriers contain the same 201600 bytes of strip payload as the
    /// prologue, but split them across 53 image records rather than 52. Its additional boundary is
    /// after strip 2804, making the complete frame 208624 bytes. Navarro accepts vino's 208608-byte
    /// second frame and then NAKs the first transfer of frame three, so this distinction is part of
    /// the producer grammar rather than harmless USB chunking.
    pub(crate) fn black_frame_ep08_ordinary(
        geometry: Geometry,
        width: usize,
        height: usize,
        connector: u8,
    ) -> Result<KVec<KVec<u8>>> {
        black_frame_ep08_variant(geometry, width, height, connector, Some(true))
    }

    fn black_frame_ep08_variant(
        geometry: Geometry,
        width: usize,
        height: usize,
        connector: u8,
        navarro_ordinary: Option<bool>,
    ) -> Result<KVec<KVec<u8>>> {
        if width & (geometry.strip_w() - 1) != 0 || height & (geometry.strip_h() - 1) != 0 {
            return Err(kernel::error::code::EINVAL);
        }
        let mut blocks: KVec<ColourBlock> = KVec::with_capacity(STRIP_BLOCKS, GFP_KERNEL)?;
        for _ in 0..STRIP_BLOCKS {
            blocks.push(
                ColourBlock {
                    qcr: [0; COEFFS],
                    qcb: [0; COEFFS],
                    qy: [0; COEFFS],
                    lcr: 0,
                    lcb: 0,
                    ly: 0,
                },
                GFP_KERNEL,
            )?;
        }
        let mut strips: KVec<KVec<u8>> = KVec::new();
        let mut sy = 0usize;
        while sy < height {
            let mut sx = 0usize;
            while sx < width {
                strips.push(
                    colour_strip(geometry, &blocks, sx as u16, sy as u16)?,
                    GFP_KERNEL,
                )?;
                sx += geometry.strip_w();
            }
            sy += geometry.strip_h();
        }
        frame_records_with_boundary(geometry, &strips, connector, navarro_ordinary)
    }

    /// The raster-ordered top-left coordinate of every strip a damage set selects.
    ///
    /// Split out of [`colour_frame_ep08_damage`] so the serial and parallel encoders select
    /// exactly the same strips in exactly the same order. The order is load-bearing:
    /// [`frame_records`] groups strips into one record per single-Y band and requires them
    /// x-ordered within each band, so reordering here changes the wire format.
    ///
    /// A strip is selected when a clip overlaps its 256x64 macro-tile. Every strip in a touched
    /// tile is resent.
    pub(crate) fn damage_strip_coords(
        geometry: Geometry,
        width: usize,
        height: usize,
        clips: &[(usize, usize, usize, usize)],
    ) -> Result<KVec<(usize, usize)>> {
        let mut coords: KVec<(usize, usize)> = KVec::new();
        let (mw, mh) = (geometry.macro_w(), geometry.macro_h());
        let mut sy = 0usize;
        while sy < height {
            let mut sx = 0usize;
            while sx < width {
                let mx = sx & !(mw - 1);
                let my = sy & !(mh - 1);
                let hit = clips
                    .iter()
                    .any(|&(x0, y0, x1, y1)| mx < x1 && x0 < mx + mw && my < y1 && y0 < my + mh);
                if hit {
                    coords.push((sx, sy), GFP_KERNEL)?;
                }
                sx += geometry.strip_w();
            }
            sy += geometry.strip_h();
        }
        Ok(coords)
    }

    /// Every strip of a full frame, in the same raster order as [`damage_strip_coords`].
    pub(crate) fn all_strip_coords(
        geometry: Geometry,
        width: usize,
        height: usize,
    ) -> Result<KVec<(usize, usize)>> {
        let mut coords: KVec<(usize, usize)> = KVec::new();
        let mut sy = 0usize;
        while sy < height {
            let mut sx = 0usize;
            while sx < width {
                coords.push((sx, sy), GFP_KERNEL)?;
                sx += geometry.strip_w();
            }
            sy += geometry.strip_h();
        }
        Ok(coords)
    }

    /// Damage-aware variant of [`colour_frame_ep08`]. It encodes only the macro-tiles selected by
    /// the client damage rectangles.
    ///
    /// `clips` are `(x0, y0, x1, y1)` half-open rectangles in output/source pixels (identity
    /// rotation only -- the caller sends a full [`colour_frame_ep08`] otherwise). A strip at
    /// `(sx, sy)` is included iff some clip overlaps
    /// `[sx, sx+strip_w()) x [sy, sy+strip_h())`. Raster iteration keeps strips
    /// x-ordered within each y-band, so [`frame_records`] groups them as the
    /// full-frame path does. Returns an empty frame list when no strip is
    /// touched -- the caller must skip the USB write in that case (no-op
    /// flip). The first frame after a mode-set must still be a full
    /// keyframe (the dock's framebuffer is undefined until then).
    ///
    /// This function remains out of line for the same stack bound as [`colour_frame_ep08`].
    #[inline(never)]
    pub(crate) fn colour_frame_ep08_damage(
        geometry: Geometry,
        width: usize,
        height: usize,
        connector: u8,
        clips: &[(usize, usize, usize, usize)],
        mut px: impl FnMut(usize, usize) -> (u16, u16, u16),
    ) -> Result<KVec<KVec<u8>>> {
        if width & (geometry.strip_w() - 1) != 0 || height & (geometry.strip_h() - 1) != 0 {
            return Err(kernel::error::code::EINVAL);
        }
        let coords = damage_strip_coords(geometry, width, height, clips)?;
        let mut strips: KVec<KVec<u8>> = KVec::with_capacity(coords.len(), GFP_KERNEL)?;
        for &(sx, sy) in coords.iter() {
            let blocks = colour_strip_blocks(geometry, sx, sy, &mut px)?;
            strips.push(
                colour_strip(geometry, &blocks, sx as u16, sy as u16)?,
                GFP_KERNEL,
            )?;
        }
        frame_records(geometry, &strips, connector)
    }

    // Exact producer completion order from DLM's authenticated 2560x1440 cold capture. Navarro
    // stops draining immediately after vino's first ordering mismatch, at strip 300. Rows alone
    // encode almost the whole permutation; the handful of split rows below are worker boundaries.
}

#[cfg(CONFIG_DRM_VINO_KUNIT_TEST)]
#[kunit_tests(vino_video)]
mod tests {
    use super::*;

    #[test]
    fn rgb565_packing() {
        assert_eq!(rgb565(0xff, 0x00, 0x00), 0xf800);
        assert_eq!(rgb565(0x00, 0xff, 0x00), 0x07e0);
        assert_eq!(rgb565(0x00, 0x00, 0xff), 0x001f);
    }

    #[test]
    fn colour_frame_ep08_damage_selects_changed_strips() -> Result {
        // Deterministic gradient source (a plain fn item so it's Copy/reusable across calls).
        fn g(x: usize, y: usize) -> (u16, u16, u16) {
            (
                ((x * 7) & 0xff) as u16,
                ((y * 5) & 0xff) as u16,
                (((x + y) * 3) & 0xff) as u16,
            )
        }
        let total = |fs: &KVec<KVec<u8>>| fs.iter().map(|f| f.len()).sum::<usize>();
        let flat = |fs: &KVec<KVec<u8>>| -> Result<KVec<u8>> {
            let mut v = KVec::new();
            for f in fs.iter() {
                v.extend_from_slice(f, GFP_KERNEL)?;
            }
            Ok(v)
        };
        // Damage granularity is the 256x64 macro-tile (`MACRO_W`/`MACRO_H`), not the 64x16 strip:
        // every strip of a touched macro-tile is resent. Use several macro-tiles so the partial
        // update assertions below can distinguish one selected tile from the full frame.
        //
        // 512x128 = 8 strips wide (512/64) x 8 bands (128/16) = 64 strips
        //         = 2 x 2 macro-tiles, each 4 strips wide x 4 bands = 16 strips.
        let (w, h) = (512usize, 128usize);
        const STRIPS_PER_MACRO: usize = 16;
        let geometry = profile::PROFILE_RIDGE.geometry();
        let full = haar::colour_frame_ep08(geometry, w, h, 0, g)?;

        // A damage clip covering the WHOLE surface selects every strip in the same raster order as
        // the full-frame path, so the wire bytes are identical.
        let dfull = haar::colour_frame_ep08_damage(geometry, w, h, 0, &[(0, 0, w, h)], g)?;
        assert_eq!(flat(&full)?.as_slice(), flat(&dfull)?.as_slice());

        // No damage -> no strips -> empty frame list (caller must skip the USB write).
        let empty = haar::colour_frame_ep08_damage(geometry, w, h, 0, &[], g)?;
        assert!(empty.is_empty());

        // Selection is exact and macro-tile-quantised. Assert the strip COUNT directly (the shared
        // selector both encoders use) as well as the byte totals -- a count is a far sharper
        // statement than "smaller than full", and it is what actually pins the tiling behaviour.
        let coords = |clips: &[(usize, usize, usize, usize)]| -> Result<usize> {
            Ok(haar::damage_strip_coords(geometry, w, h, clips)?.len())
        };
        assert_eq!(coords(&[])?, 0);
        assert_eq!(coords(&[(0, 0, w, h)])?, 4 * STRIPS_PER_MACRO); // all four macro-tiles

        // A 1-pixel clip lands in ONE macro-tile and selects all 16 of its strips -- not 1.
        assert_eq!(coords(&[(1, 1, 2, 2)])?, STRIPS_PER_MACRO);
        let d1 = haar::colour_frame_ep08_damage(geometry, w, h, 0, &[(1, 1, 2, 2)], g)?;
        assert!(!d1.is_empty());
        assert!(total(&d1) < total(&full));

        // A 1-pixel-wide clip down the whole left edge spans the left macro-tile COLUMN: 2 tiles.
        assert_eq!(coords(&[(0, 0, 1, h)])?, 2 * STRIPS_PER_MACRO);
        let d2 = haar::colour_frame_ep08_damage(geometry, w, h, 0, &[(0, 0, 1, h)], g)?;
        assert!(total(&d1) < total(&d2) && total(&d2) < total(&full));

        // Non-aligned geometry is rejected (same contract as colour_frame_ep08).
        assert!(haar::colour_frame_ep08_damage(geometry, 100, 32, 0, &[(0, 0, 1, 1)], g).is_err());
        Ok(())
    }

    #[test]
    fn black_training_frame_matches_captured_1440p_size() -> Result {
        // Captured first writes are 205,696 bytes:
        // 2,560-byte arm prefix + 203,040-byte black image + 96-byte frame trailer.
        let geometry = profile::PROFILE_RIDGE.geometry();
        let frame = haar::black_frame_ep08(geometry, 2560, 1440, 0)?;
        let image_len = frame.iter().map(|part| part.len()).sum::<usize>();
        assert_eq!(image_len, 203_040);
        assert_eq!(
            2_560 + image_len + haar::frame_trailer(geometry, 0, 0).len(),
            205_696
        );
        Ok(())
    }
}
