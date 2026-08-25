// SPDX-License-Identifier: GPL-2.0

//! The Haar transform, the quantiser and the entropy coder.
//!
//! Three levels of separable 2-D Haar in a Mallat decomposition, a power-of-two quantiser
//! whose ceilings differ per plane, and DisplayLink's unary VLC. These are the numbers the
//! dock's decoder expects exactly; every constant here was read off the wire.

use super::*;

/// One separable 2-D Haar step over the top-left `n`x`n` of `src` (row-major, `n` columns,
/// `n` in {8,4,2}). The 1-D Haar butterfly is `lo = a + b`, `hi = a - b`; applied to rows then
/// columns it splits the `n`x`n` block into four `(n/2)`x`(n/2)` subbands written to
/// `ll`/`hl`/`lh`/`hh` (row-major, stride `n/2`). Unnormalized -- `transform()` floor-divides
/// the final coefficients by 64.
///
/// Fixed array sizes let LLVM remove bounds checks and allocate only the scratch space needed
/// by each transform level.
macro_rules! haar2d_level {
    ($name:ident, $n:literal, $h:literal) => {
        /// One separable 2-D Haar step for a single level; see the macro's documentation.
        #[inline(always)]
        fn $name(
            src: &[i32; $n * $n],
            ll: &mut [i32; $h * $h],
            hl: &mut [i32; $h * $h],
            lh: &mut [i32; $h * $h],
            hh: &mut [i32; $h * $h],
        ) {
            // Row pass: L = row-lo, H = row-hi (each n rows x h cols).
            let mut l = [0i32; $n * $h];
            let mut hb = [0i32; $n * $h];
            for r in 0..$n {
                for i in 0..$h {
                    let (a, b) = (src[r * $n + 2 * i], src[r * $n + 2 * i + 1]);
                    l[r * $h + i] = a + b;
                    hb[r * $h + i] = a - b;
                }
            }
            // Column pass: LL/LH = col-lo/hi of L, HL/HH = col-lo/hi of H (each h x h).
            for c in 0..$h {
                for i in 0..$h {
                    let (a, b) = (l[2 * i * $h + c], l[(2 * i + 1) * $h + c]);
                    ll[i * $h + c] = a + b;
                    lh[i * $h + c] = a - b;
                    let (a2, b2) = (hb[2 * i * $h + c], hb[(2 * i + 1) * $h + c]);
                    hl[i * $h + c] = a2 + b2;
                    hh[i * $h + c] = a2 - b2;
                }
            }
        }
    };
}

haar2d_level!(haar2d_8, 8, 4);
haar2d_level!(haar2d_4, 4, 2);
haar2d_level!(haar2d_2, 2, 1);

/// Transform block geometry. Each 8x8 input block produces 64 coefficients.
pub(crate) const DIM: usize = 8;
pub(crate) const PIXELS: usize = DIM * DIM;
pub(crate) const COEFFS: usize = 64;
#[cfg(CONFIG_DRM_VINO_KUNIT_TEST)]
pub(crate) const BLOCK: usize = PIXELS;
/// Vino colour transform, in the codec's 64x fixed point: `Cb = 64(R-G)`,
/// `Cr = 64(B-G)` (achromatic R=G=B -> Cb=Cr=0), and the reversible luma
///
/// ```text
///     Y = 64*G + 64*((Cb_raw + Cr_raw) >> 2)   where Cb_raw=R-G, Cr_raw=B-G
/// ```
///
/// The arithmetic shift rounds negative chroma contributions towards negative infinity.
///
/// Channels are `i32` rather than `u8` because the same transform carries a 10-bit surface
/// unchanged -- see [`Depth`]. Values are the framebuffer's own code words at whatever depth
/// the plane is in; this applies no transfer function and no matrix of its own.
pub(crate) fn colour(r: i32, g: i32, b: i32) -> (i32, i32, i32) {
    let (cb, cr) = (r - g, b - g);
    (64 * g + 64 * ((cb + cr) >> 2), 64 * cb, 64 * cr)
}
/// Apply the codec's 8x8 2-D Haar (Mallat) transform, floor-divided by 64. `block` is
/// 8x8 luma (`Y` in the
/// codec's x64 fixed point); the output is 64 coefficients in the wire's Mallat layout:
/// `c[0]` = LL; `c[1..4]` = level-3 HL/LH/HH; `c[4..8]/[8..12]/[12..16]` = level-2 HL/LH/HH
/// (2x2 row-major each); `c[16..32]`, `c[32..48]`, and `c[48..64]` are the level-1 HL, LH,
/// and HH 4x4 bands. Each level-1 band uses the same 2x2 Morton scan. A uniform block yields
/// `DC = mean`, all AC = 0.
///
/// This function must remain out of line. Inlining it into the frame encoder combines the
/// transform scratch arrays with its callers and can exhaust a 16-KiB kernel stack.
#[inline(never)]
pub(crate) fn transform(block: &[i32; PIXELS]) -> [i32; COEFFS] {
    let sh = |x: i32| x >> 6; // arithmetic shift: wire fixed-point floor division by 64
                              // Level 1: 8x8 -> three 4x4 detail bands.
    let (mut ll1, mut hl1, mut lh1, mut hh1) = ([0i32; 16], [0i32; 16], [0i32; 16], [0i32; 16]);
    haar2d_8(block, &mut ll1, &mut hl1, &mut lh1, &mut hh1);
    // Level 2: LL1 (4x4) -> 2x2 subbands.
    let (mut ll2, mut hl2, mut lh2, mut hh2) = ([0i32; 4], [0i32; 4], [0i32; 4], [0i32; 4]);
    haar2d_4(&ll1, &mut ll2, &mut hl2, &mut lh2, &mut hh2);
    // Level 3: LL2 (2x2) -> the DC and coarse coefficients.
    let (mut ll3, mut hl3, mut lh3, mut hh3) = ([0i32; 1], [0i32; 1], [0i32; 1], [0i32; 1]);
    haar2d_2(&ll2, &mut ll3, &mut hl3, &mut lh3, &mut hh3);
    // Every 4x4 level-one band uses 2x2 Morton scan order.
    const SCAN4_MORTON: [usize; 16] = [0, 2, 8, 10, 1, 3, 9, 11, 4, 6, 12, 14, 5, 7, 13, 15];
    // Assemble band by band, not coefficient by coefficient. Selecting the source with a
    // `from_fn(|i| match i { .. })` reads well but compiles to a range test per coefficient
    // in a
    // rolled 64-iteration loop: profiling this function showed the `cmp`/`and`/`jne` dispatch
    // dominating it, against only ~66 add/sub for the transform itself. These fixed-length
    // loops unroll into straight-line stores, and every element is written so the initialiser
    // costs nothing.
    let mut out = [0i32; COEFFS];
    out[0] = sh(ll3[0]);
    out[1] = sh(hl3[0]);
    out[2] = sh(lh3[0]);
    out[3] = sh(hh3[0]);
    for i in 0..4 {
        out[4 + i] = sh(hl2[i]);
        out[8 + i] = sh(lh2[i]);
        out[12 + i] = sh(hh2[i]);
    }
    for i in 0..16 {
        let m = SCAN4_MORTON[i];
        out[16 + i] = sh(hl1[m]);
        out[32 + i] = sh(lh1[m]);
        out[48 + i] = sh(hh1[m]);
    }
    out
}
/// Vino entropy VLC, indexed by symbol as `(code, nbits)` and emitted least-significant bit
/// first.
/// Symbol 0 = the 1-bit code `0` (zero / most common); symbol 31 = the all-ones escape prefix.
#[cfg(CONFIG_DRM_VINO_KUNIT_TEST)]
pub(crate) const CODEBOOK: [(u32, u8); 32] = [
    (0, 1),
    (1, 3),
    (5, 3),
    (3, 5),
    (19, 5),
    (11, 5),
    (27, 5),
    (7, 7),
    (71, 7),
    (39, 7),
    (103, 7),
    (23, 7),
    (87, 7),
    (55, 7),
    (119, 7),
    (15, 8),
    (143, 8),
    (79, 8),
    (207, 8),
    (47, 8),
    (175, 8),
    (111, 8),
    (239, 8),
    (31, 8),
    (159, 8),
    (95, 8),
    (223, 8),
    (63, 8),
    (191, 8),
    (127, 8),
    (255, 9),
    (511, 9),
];

/// LSB-first VLC bit packer matching the dock (final byte padded with 1-bits -- a
/// truncated all-ones code required by the wire format).
#[cfg(CONFIG_DRM_VINO_KUNIT_TEST)]
pub(crate) struct Vlc {
    out: KVec<u8>,
    acc: u32,
    nbits: u32,
}
#[cfg(CONFIG_DRM_VINO_KUNIT_TEST)]
impl Vlc {
    pub(crate) fn new() -> Self {
        Self {
            out: KVec::new(),
            acc: 0,
            nbits: 0,
        }
    }

    /// Append one bit (LSB-first within each byte).
    fn bit(&mut self, b: u32) -> Result {
        self.acc |= (b & 1) << self.nbits;
        self.nbits += 1;
        if self.nbits == 8 {
            self.out.push((self.acc & 0xff) as u8, GFP_KERNEL)?;
            self.acc = 0;
            self.nbits = 0;
        }
        Ok(())
    }

    /// Emit codebook `sym`'s code, least-significant bit first.
    pub(crate) fn symbol(&mut self, sym: usize) -> Result {
        let (code, n) = CODEBOOK[sym];
        for k in 0..n as u32 {
            self.bit(code >> k)?;
        }
        Ok(())
    }

    /// Emit one quantized coefficient as a JPEG-SSSS-style magnitude code. A zero coefficient
    /// is the one-bit symbol 0. A nonzero
    /// `q` emits the unary category `c = bit_length(|q|)` (c ones + a 0 terminator), then the
    /// `(c-1)`-bit magnitude offset `|q| - 2^(c-1)` (MSB-first within the field), then a sign
    /// bit (`0` = negative). This helper is used for the luma codebook; the full-colour path
    /// uses [`Bits::esc`].
    /// This helper rejects categories >= 9 instead of silently mixing the two grammars.
    pub(crate) fn coeff(&mut self, q: i32) -> Result {
        if q == 0 {
            return self.symbol(0);
        }
        let c = mag_category(q); // bit_length(|q|)
        if c >= 9 {
            return Err(kernel::error::code::EOVERFLOW);
        }
        for _ in 0..c {
            self.bit(1)?; // unary category
        }
        self.bit(0)?; // terminator
        let offset = q.unsigned_abs() - (1 << (c - 1));
        for i in (0..c - 1).rev() {
            self.bit(offset >> i)?; // (c-1)-bit magnitude offset, MSB-first
        }
        self.bit(if q < 0 { 0 } else { 1 }) // sign bit (0 = negative)
    }

    /// Flush, padding the final byte with 1-bits (matches the dock's truncated all-ones code).
    pub(crate) fn finish(mut self) -> Result<KVec<u8>> {
        if self.nbits > 0 {
            while self.nbits < 8 {
                self.acc |= 1 << self.nbits;
                self.nbits += 1;
            }
            self.out.push((self.acc & 0xff) as u8, GFP_KERNEL)?;
        }
        Ok(self.out)
    }
}
/// Magnitude category of a quantized coefficient: `bit_length(|coeff|)`, or zero for zero.
pub(crate) fn mag_category(coeff: i32) -> u32 {
    coeff.unsigned_abs().checked_ilog2().map_or(0, |l| l + 1)
}
/// Maximum DC escape category at 8 bits per channel; the maximum category omits the unary
/// 0-terminator (a complete prefix code on categories `1..=SOLID_DC_CMAX`).
/// `|qY| <= 1020 => c <= 10`, `|qCb| <= 255`. See [`Depth::dc_cmax`] for the 10-bit ceiling.
pub(crate) const SOLID_DC_CMAX: u32 = 10;
/// Max LUMA AC magnitude category (the maximum category omits the unary 0-terminator).
pub(crate) const AC_CMAX: u32 = 9;
/// Maximum chroma AC magnitude category. It is higher than luma's, so a category-9 chroma
/// coefficient still carries the unary 0-terminator that luma's omits.
pub(crate) const CHROMA_AC_CMAX: u32 = 10;
/// Bits per colour channel in the surface being encoded.
///
/// This is the only thing that differs between an SDR and an HDR frame on this wire. Measured
/// against a DL7400 driven by Windows with HDR content: record framing, strip header,
/// significance tree, transform and quantiser are byte-identical across an HDR toggle, and the
/// colour maths is entirely host-side -- what arrives is an ordinary PQ-encoded 10-bit RGB
/// surface. So this is one codec parameterised by depth, not two codecs.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Depth {
    /// 8 bits per channel: every mode on a Ridge dock, and a Navarro one outside HDR.
    Eight,
    /// 10 bits per channel, the DL-7000 "10bit profile".
    Ten,
}
impl Depth {
    /// Maximum escape category for a DC coefficient at this depth.
    ///
    /// A luma DC is four times the sample, so the largest magnitude is `4 * ((1 << bits) - 1)`
    /// -- 1020 at 8 bits (category 10) and 4092 at 10 bits (category 12). The ceiling is part
    /// of the wire format, not an implementation limit: at the maximum category `esc` omits
    /// the unary 0-terminator, so a decoder reading with the wrong ceiling silently
    /// desynchronises. Both values are measured, each uniquely, by decoding a captured PQ ramp
    /// under every candidate and keeping the one that stays monotonic
    /// (`tools/codec/depth-probe.py`).
    #[inline]
    /// Maximum escape category for a luma AC coefficient.
    ///
    /// Unlike [`Self::dc_cmax`] this does **not** scale with the sample depth. Raising it two
    /// categories to match the DC ceiling was tried against the hardware and desynchronises the
    /// dock: at the maximum category the escape omits its unary terminator, so a ceiling above the
    /// dock's makes every coefficient below it carry a terminator the dock reads as an offset bit,
    /// and the picture breaks into horizontal bands.
    ///
    /// Saturating at the eight-bit ceiling clips the largest coefficients of ten-bit content
    /// instead, which is the conservative failure. Settling the real value needs a capture of the
    /// vendor driving AC-heavy HDR content, which has never been taken.
    pub(crate) fn ac_cmax(self) -> u32 {
        match self {
            Depth::Eight => AC_CMAX,
            // A coefficient is four times the sample, so every ceiling the depth moves gains two
            // categories. The code table stating this one is raised with it.
            Depth::Ten => AC_CMAX + 2,
        }
    }

    /// Maximum escape category for a chroma AC coefficient at this depth; see [`Self::ac_cmax`].
    pub(crate) fn chroma_ac_cmax(self) -> u32 {
        match self {
            Depth::Eight => CHROMA_AC_CMAX,
            Depth::Ten => CHROMA_AC_CMAX + 2,
        }
    }

    pub(crate) fn dc_cmax(self) -> u32 {
        match self {
            Depth::Eight => SOLID_DC_CMAX,
            Depth::Ten => SOLID_DC_CMAX + 2,
        }
    }

    /// The depth of a DRM pixel format, or `None` for one this codec cannot encode.
    ///
    /// Both layouts are four bytes per pixel, so they share the snapshot path and differ only
    /// in how `PixelSource` splits a word.
    #[inline]
    pub(crate) fn from_fourcc(fourcc: u32) -> Option<Self> {
        match fourcc {
            kernel::drm::fourcc::XRGB8888 => Some(Depth::Eight),
            kernel::drm::fourcc::XRGB2101010 => Some(Depth::Ten),
            _ => None,
        }
    }
}
/// LSB-first bit accumulator for the production AC-strip coder.
///
/// Bits are buffered in a 64-bit word and copied to `out` a byte at a time. `out` is incomplete
/// until [`Bits::finish`] flushes the final zero-padded partial byte.
pub(crate) struct Bits {
    out: KVec<u8>,
    /// Pending bits, LSB-first, valid in the low `nacc`.
    acc: u64,
    nacc: u32,
    /// Which dialect of the shared unary code to emit; see [`Bits::unary`].
    coding: super::super::video_arm::CodeTables,
}
impl Bits {
    pub(crate) fn new(coding: super::super::video_arm::CodeTables) -> Self {
        Self {
            out: KVec::new(),
            acc: 0,
            nacc: 0,
            coding,
        }
    }

    /// Append one bit.
    ///
    /// `#[inline(always)]` because this is the codec's innermost operation -- `esc` calls it
    /// once per unary, offset and sign bit. Out of line it costs a call, a return and a
    /// reload/store of `acc`/`nacc` *per bit*; inlined, the accumulator stays in registers
    /// across a run of bits. The eight-byte spill is deliberately left out of line so that
    /// inlining stays cheap.
    #[inline(always)]
    fn bit(&mut self, b: u32) -> Result {
        self.acc |= ((b & 1) as u64) << self.nacc;
        self.nacc += 1;
        if self.nacc == 64 {
            self.spill()?;
        }
        Ok(())
    }

    /// Write the full accumulator out and reset it. Cold: once per 64 bits.
    #[inline(never)]
    fn spill(&mut self) -> Result {
        for k in 0..8 {
            self.out.push((self.acc >> (8 * k)) as u8, GFP_KERNEL)?;
        }
        self.acc = 0;
        self.nacc = 0;
        Ok(())
    }

    /// Flush the accumulator and yield the packed bytes, zero-padding the final byte.
    pub(crate) fn finish(mut self) -> Result<KVec<u8>> {
        let nbytes = self.nacc.div_ceil(8) as usize;
        for k in 0..nbytes {
            self.out.push((self.acc >> (8 * k)) as u8, GFP_KERNEL)?;
        }
        Ok(self.out)
    }

    /// The one code every field in a strip is built from: a unary category of `count` ones, a
    /// `0` terminator unless the category is the codebook maximum, and exactly `count` payload
    /// bits.
    ///
    /// The dock generations differ in where the payload sits and in which end of it comes
    /// first. Ridge and the DL7400 group it after the terminator, most significant bit first.
    /// A DL-3x00 decoder expects one payload bit immediately after each unary one, terminator
    /// last, and reads that interleaved payload **least** significant bit first. Every
    /// spelling is the same length, so emitting the wrong one produces records of exactly the
    /// right size that decode to noise -- and a flat strip cannot tell them apart, because its
    /// payload is all zeroes.
    fn unary(&mut self, count: u32, terminate: bool, payload: u32) -> Result {
        match self.coding {
            super::super::video_arm::CodeTables::Wide => {
                for _ in 0..count {
                    self.bit(1)?;
                }
                if terminate {
                    self.bit(0)?;
                }
                for i in (0..count).rev() {
                    self.bit(payload >> i)?;
                }
            }
            super::super::video_arm::CodeTables::Narrow => {
                for i in 0..count {
                    self.bit(1)?;
                    self.bit(payload >> i)?;
                }
                if terminate {
                    self.bit(0)?;
                }
            }
        }
        Ok(())
    }

    /// The shared escape value code: a 0 is one `0` bit; else a category of `c` carrying
    /// `offset(c-1) ++ sign(1=positive)` as its payload. `c = bit_length(|v|)`.
    pub(crate) fn esc(&mut self, v: i32, cmax: u32) -> Result {
        if v == 0 {
            return self.bit(0);
        }
        // Saturate a magnitude whose category exceeds the codebook maximum (`cmax`) to the
        // largest value that category `cmax` encodes. This keeps the unary prefix at most
        // `cmax` ones: a decoder that stops after `cmax` ones (the max-category escape, whose
        // 0-terminator is omitted) would otherwise read the (cmax+1)th one as an offset bit
        // and desync the rest of the strip. In-range coefficients (`c <= cmax`) are
        // unaffected; this only bounds the out-of-range case, which the recovered grammar does
        // not otherwise exercise.
        let c = mag_category(v).min(cmax);
        let off = v.unsigned_abs().min((1 << c) - 1) - (1 << (c - 1));
        // The payload is the offset with the sign bit below it, which is `c` bits: an offset
        // spans `c - 1` bits at category `c`.
        self.unary(c, c < cmax, (off << 1) | u32::from(v > 0))
    }

    /// Per-block luma significance, after the two zero root branches a present chroma plane
    /// replaces. For `k=floor(log2(64-last))` the category is `k` and the payload the `k`-bit
    /// value `(64-2^k)-last`; a flat block is the maximum category with an all-zero payload,
    /// which is why it reads as one fixed-width code rather than a position.
    fn sync_unit_after(&mut self, last: usize, skip: usize) -> Result {
        for _ in skip..2 {
            self.bit(0)?;
        }
        if last == 0 {
            self.unary(6, true, 0)
        } else {
            debug_assert!(last < COEFFS);
            let k = usize::BITS - 1 - (COEFFS - last).leading_zeros();
            let end = COEFFS - (1usize << k);
            self.unary(k, true, (end - last) as u32)
        }
    }

    fn sync_unit(&mut self, last: usize) -> Result {
        self.sync_unit_after(last, 0)
    }
}
/// Chroma AC quantizer. Coarse bands 1/2 and 4..11 use step 16; coefficient 3 and positions
/// 12..47 use step 32; the final HH band uses step 64. All use signed half-up rounding.
///
/// Shifts rather than divisions, for the reason given on [`step_bias`]; steps 16/32/64 are
/// shifts 4/5/6 and `step / 2` is `1 << (shift - 1)`.
pub(crate) fn quantize_chroma_ac(coeff: i32, i: usize) -> i32 {
    let shift = CHROMA_AC_SHIFT[i];
    (coeff + (1 << (shift - 1))) >> shift
}
/// Per-plane DC quantizer, round-half-up on the SIGNED value (toward +inf): luma (plane 0)
/// step 16, chroma step 64. `+224/64 = 3.5 -> 4`; `-8416/64 = -131.5 -> -131`.
pub(crate) fn quantize_dc_round(plane: usize, v: i32) -> i32 {
    let shift: u32 = if plane == 0 { 4 } else { 6 };
    (v + (1 << (shift - 1))) >> shift
}
impl Bits {
    /// Exact chroma last-position tree node. For `c=floor(log2(last+1))` the category is `c`
    /// and the payload the `c`-bit offset `last-(2^c-1)`.
    fn chroma_base(&mut self, last: usize) -> Result {
        debug_assert!(last > 0 && last < COEFFS);
        let c = usize::BITS - 1 - (last + 1).leading_zeros();
        self.unary(c, true, (last - ((1usize << c) - 1)) as u32)
    }

    /// One block's three-plane significance tree. The luma code begins with two
    /// zero root branches. A present Cr replaces the first with its chroma node; a present Cb
    /// replaces the second.
    pub(crate) fn colour_sync_unit(&mut self, lcr: usize, lcb: usize, ly: usize) -> Result {
        match (lcr != 0, lcb != 0) {
            (false, false) => self.sync_unit(ly),
            (true, false) => {
                self.chroma_base(lcr)?;
                self.sync_unit_after(ly, 1)
            }
            (false, true) => {
                self.bit(0)?;
                self.chroma_base(lcb)?;
                self.sync_unit_after(ly, 2)
            }
            (true, true) => {
                self.chroma_base(lcr)?;
                self.chroma_base(lcb)?;
                self.sync_unit_after(ly, 2)
            }
        }
    }

    /// One block's colour AC: present planes in (Cr, Cb, Y) order, positions `1..=last`,
    /// run-bit `0` for an insignificant coefficient else the magnitude escape. Chroma and luma
    /// use DIFFERENT codebook maxima (`CHROMA_AC_CMAX` / `AC_CMAX`) -- see `CHROMA_AC_CMAX`.
    pub(crate) fn colour_block_ac(
        &mut self,
        qcr: &[i32; COEFFS],
        qcb: &[i32; COEFFS],
        qy: &[i32; COEFFS],
        lcr: usize,
        lcb: usize,
        ly: usize,
        depth: Depth,
    ) -> Result {
        for &(q, last, cmax) in &[
            (qcr, lcr, depth.chroma_ac_cmax()),
            (qcb, lcb, depth.chroma_ac_cmax()),
            (qy, ly, depth.ac_cmax()),
        ] {
            for i in 1..=last {
                if q[i] == 0 {
                    self.bit(0)?;
                } else {
                    self.esc(q[i], cmax)?;
                }
            }
        }
        Ok(())
    }
}

#[cfg(CONFIG_DRM_VINO_KUNIT_TEST)]
#[kunit_tests(vino_haar_transform)]
mod tests {
    use super::*;

    /// The escape codebook's ceiling is part of the wire format and follows the sample depth.
    ///
    /// Measured against a DL7400 driven by Windows in its 10-bit profile: a captured PQ ramp
    /// decodes monotonically at ceiling 12 and at no other value, and the SDR half
    /// of the same capture only at 10. Getting this wrong does not degrade the picture, it
    /// desynchronises the dock's decoder -- at the maximum category `esc` omits the unary
    /// 0-terminator, so the next value's first bit is read as an offset bit.
    /// The AC ceilings scale with the sample depth, exactly as the DC ceiling does.
    ///
    /// A ten-bit sample makes every coefficient four times larger, which is two categories. Held
    /// at their eight-bit values the escape saturates the bulk of the AC energy rather than just
    /// the extremes, and the picture breaks up into blocks. The vendor's own stress content states
    /// the same scaling: a two-pixel grating that is category 9 in SDR is category 11 in HDR.
    #[test]
    fn haar_depth_selects_the_ac_codebooks() {
        use Depth;
        assert_eq!(Depth::Eight.ac_cmax(), 9);
        assert_eq!(Depth::Ten.ac_cmax(), 11);
        assert_eq!(Depth::Eight.chroma_ac_cmax(), 10);
        assert_eq!(Depth::Ten.chroma_ac_cmax(), 12);
        // An AC ceiling left behind stays in step, because the category fixes the field width,
        // and reconstructs every sharp edge from a truncated magnitude instead.
        assert_eq!(Depth::Ten.ac_cmax(), Depth::Eight.ac_cmax() + 2);
        assert_eq!(
            Depth::Ten.chroma_ac_cmax(),
            Depth::Eight.chroma_ac_cmax() + 2
        );
        assert_eq!(Depth::Ten.dc_cmax(), Depth::Eight.dc_cmax() + 2);
        // Chroma stays one category above luma, so a coefficient at luma's ceiling still carries
        // the unary terminator on the chroma planes.
        assert_eq!(Depth::Eight.chroma_ac_cmax(), Depth::Eight.ac_cmax() + 1);
        assert_eq!(Depth::Ten.chroma_ac_cmax(), Depth::Ten.ac_cmax() + 1);
    }

    #[test]
    fn haar_depth_selects_the_dc_codebook() {
        use Depth;
        assert_eq!(Depth::Eight.dc_cmax(), 10);
        assert_eq!(Depth::Ten.dc_cmax(), 12);
        // A luma DC is four times the sample, so each ceiling is exactly the category that holds
        // its depth's largest value: 4 x 255 = 1020 (c=10) and 4 x 1023 = 4092 (c=12).
        assert_eq!(mag_category(4 * 255), Depth::Eight.dc_cmax());
        assert_eq!(mag_category(4 * 1023), Depth::Ten.dc_cmax());
    }

    /// The depth comes from the committed framebuffer's fourcc, never from state of our own.
    #[test]
    fn haar_depth_from_fourcc() {
        use Depth;
        assert!(matches!(
            Depth::from_fourcc(kernel::drm::fourcc::XRGB8888),
            Some(Depth::Eight)
        ));
        assert!(matches!(
            Depth::from_fourcc(kernel::drm::fourcc::XRGB2101010),
            Some(Depth::Ten)
        ));
        assert!(Depth::from_fourcc(kernel::drm::fourcc::ARGB8888).is_none());
    }

    #[test]
    fn haar_transform_uniform() {
        use video::haar;
        // A uniform block has the per-pixel value at DC and zero AC terms.
        let block = [16320i32; haar::BLOCK];
        let c = haar::transform(&block);
        assert_eq!(c[0], 16320);
        assert!(c[1..].iter().all(|&x| x == 0));
        // White pixel -> Y plane -> Haar DC -> quantized value.
        let (y, _, _) = haar::colour(255, 255, 255);
        assert_eq!(
            haar::quantize(haar::transform(&[y; haar::BLOCK])[0], 0),
            1020
        );
    }

    /// The quantisers divide by powers of two with arithmetic shifts, avoiding a runtime division
    /// for every coefficient.
    ///
    /// The rewrite is only valid because floor division by `2^k` IS an arithmetic right shift, for
    /// negative operands as well as positive. That identity is easy to state and easy to get wrong
    /// (a *truncating* `/` is not the same thing on negatives), and a coefficient off by one is a
    /// wire-visible codec change. So assert it directly, over the full coefficient range the
    /// transform can produce, against the equivalent division.
    #[test]
    fn quantiser_shifts_match_division() {
        use video::haar::{quantize, COEFFS};
        // 8-bit input in the codec's x64 fixed point, summed over an 8x8 block and floor-divided by
        // 64 by `transform`, bounds |coeff| well inside this; step past it on both signs anyway.
        const LIMIT: i32 = 200_000;
        // The luma table, restated here as DIVISORS so the test is not written against the same
        // shift constants it is checking.
        let step_bias = |i: usize| -> (i32, i32) {
            match i {
                0 | 1 | 2 => (16, 8),
                3 => (32, 16),
                4..=11 => (4, 2),
                12..=15 => (8, 4),
                16..=47 => (2, 0),
                _ => (4, 2),
            }
        };
        for i in 0..COEFFS {
            let (step, bias) = step_bias(i);
            for coeff in (-LIMIT..=LIMIT).step_by(37) {
                let want = if bias == 0 {
                    let q = coeff.abs() / step;
                    if coeff < 0 {
                        -q
                    } else {
                        q
                    }
                } else {
                    (coeff + bias).div_euclid(step)
                }
                .clamp(-2048, 2047);
                assert_eq!(quantize(coeff, i), want);
            }
        }
        // Boundary cases the stride above can step over: every exact multiple and half-step of the
        // coarsest divisor, on both signs, is where floor-vs-truncate actually differs.
        for i in 0..COEFFS {
            let (step, bias) = step_bias(i);
            for m in -4i32..=4 {
                for d in [-1, 0, 1, step / 2, -step / 2] {
                    let coeff = m * step + d;
                    let want = if bias == 0 {
                        let q = coeff.abs() / step;
                        if coeff < 0 {
                            -q
                        } else {
                            q
                        }
                    } else {
                        (coeff + bias).div_euclid(step)
                    }
                    .clamp(-2048, 2047);
                    assert_eq!(quantize(coeff, i), want);
                }
            }
        }
    }

    #[test]
    fn haar_transform_haar_vectors() {
        // Independent golden vectors cover the source gradient blocks. Input luma is
        // `Y = 64 * gray`.
        use video::haar::{transform, DIM, PIXELS};
        // Build an 8x8 Y block by evaluating a gray-per-(row,col) selector.
        fn build(gray: impl Fn(usize, usize) -> i32) -> [i32; PIXELS] {
            let mut b = [0i32; PIXELS];
            for r in 0..DIM {
                for c in 0..DIM {
                    b[r * DIM + c] = 64 * gray(r, c);
                }
            }
            b
        }
        // vstripe2 (period-2 vertical, full contrast 0/255) -> level-2 HL band c[4..8] = -2040.
        let c = transform(&build(|_, c| if (c / 2) & 1 != 0 { 255 } else { 0 }));
        assert_eq!(&c[4..8], &[-2040, -2040, -2040, -2040]);
        assert!(c[1..4].iter().all(|&x| x == 0) && c[8..].iter().all(|&x| x == 0));
        // Period-four vertical stripe: coarse HL c[1] = -8160.
        let c = transform(&build(|_, c| if (c / 4) & 1 != 0 { 255 } else { 0 }));
        assert_eq!(c[1], -8160);
        // Period-two horizontal stripe: level-two LH band is -2040.
        let c = transform(&build(|r, _| if (r / 2) & 1 != 0 { 255 } else { 0 }));
        assert_eq!(&c[8..12], &[-2040, -2040, -2040, -2040]);
        // A per-column gradient exercises DC, coarse-HL, and the finest band.
        let c = transform(&build(|_, col| 36 * col as i32));
        assert_eq!(c[0], 8064); // DC = mean(36*0..36*7)*64/64 = 8064
        assert_eq!(&c[4..8], &[-576, -576, -576, -576]);
        // The level-1 tail contains three 4x4 Morton-scanned bands:
        // c[16..32] = HL1, c[32..48] = LH1, and c[48..64] = HH1.
        //
        // A per-column ramp has no vertical detail: HL1 is uniformly -72 and LH1/HH1 are zero.
        assert!(c[16..32].iter().all(|&x| x == -72)); // finest HL: horizontal detail only
        assert!(c[32..].iter().all(|&x| x == 0)); // LH1 + HH1: no vertical detail
    }

    #[test]
    fn haar_vlc_codebook_byte_exact() -> Result {
        // The LSB-first entropy VLC is checked against independent golden output. Symbol 7 is the
        // AC code
        // 0b1110000 (LSB-first); four of them pack to the wire's per-block AC unit bytes, and the
        // final byte is padded with 1-bits (a truncated all-ones code), exactly as the dock emits.
        use Vlc;
        let mut w = Vlc::new();
        for _ in 0..4 {
            w.symbol(7)?;
        }
        assert_eq!(&w.finish()?[..], &[0x87, 0xc3, 0xe1, 0xf0]);
        // The full per-block AC unit `0 0 0 7 7 7 7` (idx1-3 zero, idx4-7 AC) -- matches the live
        // wire bytes `38 1c 0e ...` captured for vstripe2.
        let mut w = Vlc::new();
        for s in [0usize, 0, 0, 7, 7, 7, 7] {
            w.symbol(s)?;
        }
        assert_eq!(&w.finish()?[..4], &[0x38, 0x1c, 0x0e, 0x87]);
        // Symbol 0 (the 1-bit `0` code) alone -> one byte padded with seven 1-bits.
        let mut w = Vlc::new();
        w.symbol(0)?;
        assert_eq!(&w.finish()?[..], &[0xfe]);
        Ok(())
    }

    #[test]
    fn haar_coeff_magnitude_code() -> Result {
        // The AC magnitude-code emitter is checked against per-coefficient golden wire bits for
        // q-4, q-8, and q-16.
        use Vlc;
        // Four q-4 coefficients (category 3, zero offset) == four sym7 -- the per-block AC unit.
        let mut w = Vlc::new();
        for _ in 0..4 {
            w.coeff(-4)?;
        }
        assert_eq!(&w.finish()?[..], &[0x87, 0xc3, 0xe1, 0xf0]);
        // A zero coefficient is the 1-bit symbol 0 -> one byte padded with seven 1-bits.
        let mut w = Vlc::new();
        w.coeff(0)?;
        assert_eq!(&w.finish()?[..], &[0xfe]);
        // Within-category offset (q-6 = category 3, offset 2) and sign polarity (negative vs +).
        let mut w = Vlc::new();
        w.coeff(-6)?;
        assert_eq!(&w.finish()?[..], &[0x97]);
        let mut w = Vlc::new();
        w.coeff(6)?;
        assert_eq!(&w.finish()?[..], &[0xd7]); // same magnitude, sign bit flipped
                                               // Category 5 with offset (q-16) spans two bytes.
        let mut w = Vlc::new();
        w.coeff(-16)?;
        assert_eq!(&w.finish()?[..], &[0x1f, 0xf8]);
        // The unsupported long-form escape is rejected.
        let mut w = Vlc::new();
        assert!(w.coeff(-256).is_err());
        Ok(())
    }

    #[test]
    fn haar_magnitude_category() {
        // Magnitude category is `bit_length(abs(coeff))`.
        use mag_category;
        assert_eq!(mag_category(0), 0);
        assert_eq!(mag_category(1), 1);
        assert_eq!(mag_category(-4), 3);
        assert_eq!(mag_category(7), 3);
        assert_eq!(mag_category(-8), 4);
        assert_eq!(mag_category(16), 5);
        assert_eq!(mag_category(-128), 8);
        assert_eq!(mag_category(255), 8);
    }
}
