// SPDX-License-Identifier: GPL-2.0
//! Software colour management for the CRTC's `CTM` and `GAMMA_LUT` properties.
//!
//! Neither driver that uses this has colour hardware to program: vino sends already-encoded
//! pixels to a dock, and evdi hands framebuffer pixels to a userspace client. A compositor that
//! colour-corrects through the KMS properties -- GNOME's Night Light and KDE's Night Colour both
//! do, rather than rewriting the framebuffer -- therefore has nowhere to put the correction on
//! such an output unless the driver applies it while it still has the pixels. This module is that
//! application.
//!
//! ⚠ **This file is shared verbatim between `drm/vino` and `drm/evdi`.** They are separate
//! modules and cannot share a crate, so the copies are kept byte-identical and
//! `tools/color-selftest.sh` fails if they drift.
//!
//! Pipeline order follows DRM's: degamma, then CTM, then gamma. No degamma LUT is advertised, so
//! what runs here is **CTM then gamma**.
//!
//! ⚠ The transform is applied to the framebuffer's encoded (typically sRGB) values, not to linear
//! light, because there is no degamma stage to linearise them first. That is the same
//! simplification every software implementation of this makes, and it is what compositors expect
//! when they compute a correction for a CRTC that advertises no degamma LUT.
//!
//! # Why the representation is an enum
//!
//! This runs on every pixel of every changed region -- vino's `PixelSource::px` is the
//! third-hottest symbol in the kernel under fullscreen video (13.8% of the machine on a 4K clip),
//! and evdi's GRABPIX copies every damaged pixel of every frame -- so the per-pixel cost matters
//! more than generality. A CTM that only scales each channel, which is the shape *every*
//! colour-temperature corrector produces, collapses into per-channel lookup tables exactly as a
//! gamma ramp does; [`ColorPipeline::Fused`] keeps that case at one table lookup per channel, the
//! same cost as before CTM existed. Only a matrix that genuinely mixes channels pays for
//! arithmetic.

use kernel::drm::kms::crtc::{ColorCtm, ColorLut};

/// Fixed-point scale for pixel values and matrix coefficients: 1.0 is `1 << 16`.
///
/// Q16 rather than the UAPI's S31.32 so that a coefficient-by-channel multiply is `i32 * i32` into
/// an `i64`. S31.32 would need 128-bit intermediates, which are not available on every
/// architecture the kernel builds for.
const Q: i32 = 1 << 16;

/// Largest Q16 pixel value, i.e. 1.0 in the pixel range.
const MAX: i32 = 0xffff;

/// Added before a `>> 16` so a fixed-point product rounds to nearest instead of truncating.
///
/// Truncation biases every multiply downwards by up to half a level, and the bias is systematic
/// across the whole image rather than random -- a half-gain of 255 would come out at 127 instead
/// of 128, and a corrected desktop would sit measurably darker than the correction asked for.
const HALF_Q: i64 = 1 << 15;

/// Entries in each channel's lookup table. Matches the `GAMMA_LUT_SIZE` advertised to userspace.
pub(crate) const LUT_LEN: usize = 256;

/// Expand an 8-bit channel to Q16 so that 0 -> 0 and 255 -> 65535 exactly.
#[inline]
fn expand(v: u8) -> i32 {
    v as i32 * 257
}

/// Round a Q16 channel back to 8 bits.
///
/// The divisor is 257, not 256, because that is what [`expand`] multiplied by. Rounding by 256
/// instead makes `narrow(expand(v))` drift above `v` -- 200 comes back as 201 -- so even an
/// identity transform would shift the whole image.
#[inline]
fn narrow(v: i32) -> u8 {
    ((v.clamp(0, MAX) + 128) / 257).min(255) as u8
}

/// Sample a 256-entry Q16 table at a Q16 input, interpolating between neighbours.
///
/// Straight indexing by the top 8 bits would quantise the CTM's output back to 8 bits before the
/// gamma ramp ever saw it, which shows up as banding on the smooth gradients this is most often
/// used on.
#[inline]
fn sample(table: &[u16], v: i32) -> i32 {
    let v = v.clamp(0, MAX);
    // Entry `i` of the table describes input `i * 257`, so the step is 257 -- the same divisor
    // [`narrow`] uses, and for the same reason.
    let idx = (v / 257) as usize;
    let frac = v - idx as i32 * 257;
    let a = table[idx.min(LUT_LEN - 1)] as i32;
    let b = table[(idx + 1).min(LUT_LEN - 1)] as i32;
    (a * (257 - frac) + b * frac + 128) / 257
}

/// A CRTC's colour transform, precomputed into whichever form is cheapest to apply per pixel.
#[derive(Clone, Copy)]
pub(crate) enum ColorPipeline {
    /// Per-channel 8-bit tables, red then green then blue. Covers a gamma ramp alone and a gamma
    /// ramp after a channel-independent CTM.
    Fused([u8; 3 * LUT_LEN]),
    /// A CTM that mixes channels, so it cannot collapse into per-channel tables. `ctm` is Q16,
    /// row-major, and `gamma` is applied after it.
    Mixed {
        ctm: [i32; 9],
        gamma: Option<[u16; 3 * LUT_LEN]>,
    },
}

/// Read a DRM gamma blob into three Q16 channel tables, extending an under-length LUT with
/// identity rather than with zeroes (which would render the output black).
fn read_lut(lut: &[ColorLut]) -> [u16; 3 * LUT_LEN] {
    let mut t = [0u16; 3 * LUT_LEN];
    for i in 0..LUT_LEN {
        let identity = (i * 257) as u16;
        match lut.get(i) {
            Some(e) => {
                t[i] = e.red();
                t[LUT_LEN + i] = e.green();
                t[2 * LUT_LEN + i] = e.blue();
            }
            None => {
                t[i] = identity;
                t[LUT_LEN + i] = identity;
                t[2 * LUT_LEN + i] = identity;
            }
        }
    }
    t
}

/// Convert a decoded S31.32 coefficient to Q16, saturating rather than wrapping.
fn to_q16(v: i64) -> i32 {
    (v >> 16).clamp(i32::MIN as i64, i32::MAX as i64) as i32
}

impl ColorPipeline {
    /// Build the pipeline for a CRTC state's `CTM` and `GAMMA_LUT`, or [`None`] when neither is
    /// programmed or both are the identity.
    ///
    /// [`None`] is not merely an optimisation: it is what keeps vino's direct-scanout path and
    /// evdi's plain copy-to-userspace path available, so an uncorrected desktop pays nothing at
    /// all for this feature existing.
    pub(crate) fn build(lut: Option<&[ColorLut]>, ctm: Option<&ColorCtm>) -> Option<Self> {
        let gamma = lut.map(read_lut);
        let matrix = ctm.map(|c| {
            let raw = c.coefficients();
            let mut m = [0i32; 9];
            for (o, r) in m.iter_mut().zip(raw.iter()) {
                *o = to_q16(*r);
            }
            m
        });

        // An identity matrix is what a compositor programs when it turns a corrector *off*, and it
        // arrives as a real blob rather than as a removal. Treating it as "no CTM" is what lets
        // the fast path come back afterwards.
        let mixes = |m: &[i32; 9]| {
            m[1] != 0 || m[2] != 0 || m[3] != 0 || m[5] != 0 || m[6] != 0 || m[7] != 0
        };
        let matrix = match matrix {
            Some(m) if mixes(&m) => return Some(Self::mixed(m, gamma)),
            Some(m) if m[0] == Q && m[4] == Q && m[8] == Q => None,
            other => other,
        };

        if matrix.is_none() && gamma.is_none() {
            return None;
        }
        Some(Self::fuse(matrix, gamma))
    }

    fn mixed(ctm: [i32; 9], gamma: Option<[u16; 3 * LUT_LEN]>) -> Self {
        Self::Mixed { ctm, gamma }
    }

    /// Collapse a channel-independent CTM and a gamma ramp into one 8-bit table per channel.
    ///
    /// Done once per property change over 768 entries, so the per-pixel path never sees the
    /// arithmetic.
    fn fuse(diag: Option<[i32; 9]>, gamma: Option<[u16; 3 * LUT_LEN]>) -> Self {
        let mut fused = [0u8; 3 * LUT_LEN];
        // Diagonal entries of a row-major 3x3.
        let gains = diag.map(|m| [m[0], m[4], m[8]]);
        for c in 0..3 {
            for i in 0..LUT_LEN {
                let mut v = expand(i as u8);
                if let Some(g) = gains {
                    v = (((g[c] as i64 * v as i64 + HALF_Q) >> 16)).clamp(0, MAX as i64) as i32;
                }
                if let Some(t) = &gamma {
                    v = sample(&t[c * LUT_LEN..(c + 1) * LUT_LEN], v);
                }
                fused[c * LUT_LEN + i] = narrow(v.clamp(0, MAX));
            }
        }
        Self::Fused(fused)
    }

    /// Apply the transform to one pixel.
    #[inline]
    pub(crate) fn apply(&self, r: u8, g: u8, b: u8) -> (u8, u8, u8) {
        match self {
            Self::Fused(t) => (
                t[r as usize],
                t[LUT_LEN + g as usize],
                t[2 * LUT_LEN + b as usize],
            ),
            Self::Mixed { ctm, gamma } => {
                let (r, g, b) = (expand(r) as i64, expand(g) as i64, expand(b) as i64);
                let mul = |row: usize| {
                    let acc = ctm[row * 3] as i64 * r
                        + ctm[row * 3 + 1] as i64 * g
                        + ctm[row * 3 + 2] as i64 * b;
                    // Clamping here, before the gamma ramp, is what keeps an out-of-gamut
                    // intermediate from wrapping into the opposite corner of the colour cube.
                    (((acc + HALF_Q) >> 16).clamp(0, MAX as i64)) as i32
                };
                let mut out = [mul(0), mul(1), mul(2)];
                if let Some(t) = gamma {
                    for (c, o) in out.iter_mut().enumerate() {
                        *o = sample(&t[c * LUT_LEN..(c + 1) * LUT_LEN], *o);
                    }
                }
                (
                    narrow(out[0].clamp(0, MAX)),
                    narrow(out[1].clamp(0, MAX)),
                    narrow(out[2].clamp(0, MAX)),
                )
            }
        }
    }

    /// A value that changes whenever the transform does.
    ///
    /// The encoded-strip cache keys on the pixels a strip contained, so a transform change that
    /// leaves the source pixels alone would otherwise serve a stale body.
    pub(crate) fn tag(&self) -> u64 {
        const SEED: u64 = 0x9e37_79b1_85eb_ca87;
        match self {
            Self::Fused(t) => kernel::xxhash::xxh64(&t[..], SEED),
            Self::Mixed { ctm, gamma } => {
                let mut bytes = [0u8; 9 * 4];
                for (i, c) in ctm.iter().enumerate() {
                    bytes[i * 4..i * 4 + 4].copy_from_slice(&c.to_le_bytes());
                }
                let h = kernel::xxhash::xxh64(&bytes, SEED);
                match gamma {
                    Some(t) => {
                        let mut g = [0u8; 2 * 3 * LUT_LEN];
                        for (i, v) in t.iter().enumerate() {
                            g[i * 2..i * 2 + 2].copy_from_slice(&v.to_le_bytes());
                        }
                        kernel::xxhash::xxh64(&g, h)
                    }
                    None => h,
                }
            }
        }
    }
}

impl PartialEq for ColorPipeline {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Fused(a), Self::Fused(b)) => a[..] == b[..],
            (
                Self::Mixed {
                    ctm: a,
                    gamma: ga,
                },
                Self::Mixed {
                    ctm: b,
                    gamma: gb,
                },
            ) => {
                a == b
                    && match (ga, gb) {
                        (Some(x), Some(y)) => x[..] == y[..],
                        (None, None) => true,
                        _ => false,
                    }
            }
            _ => false,
        }
    }
}
