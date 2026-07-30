// SPDX-License-Identifier: GPL-2.0

//! DisplayLink full-colour video encoder and framing.

use super::*;

/// Pack 8-bit RGB into RGB565.
#[cfg(CONFIG_DRM_VINO_KUNIT_TEST)]
pub(crate) fn rgb565(r: u8, g: u8, b: u8) -> u16 {
    ((r as u16 >> 3) << 11) | ((g as u16 >> 2) << 5) | (b as u16 >> 3)
}

/// DisplayLink's 8x8 Walsh-Hadamard codec and 64x16 strip grammar.
pub(crate) mod wht {
    use super::*;

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
    pub(crate) fn colour(r: u8, g: u8, b: u8) -> (i32, i32, i32) {
        let (r, g, b) = (r as i32, g as i32, b as i32);
        let (cb, cr) = (r - g, b - g);
        (64 * g + 64 * ((cb + cr) >> 2), 64 * cb, 64 * cr)
    }

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

    /// LSB-first VLC bit packer matching the dock (final byte padded with **1-bits** -- a
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

    /// Maximum DC escape category; the maximum category omits the unary 0-terminator (a complete
    /// prefix code on categories `1..=SOLID_DC_CMAX`). `|qY| <= 1020 => c <= 10`, `|qCb| <= 255`.
    const SOLID_DC_CMAX: u32 = 10;

    /// Max LUMA AC magnitude category (the maximum category omits the unary 0-terminator).
    const AC_CMAX: u32 = 9;

    /// Maximum chroma AC magnitude category. It is higher than luma's, so a category-9 chroma
    /// coefficient still carries the unary 0-terminator that luma's omits.
    const CHROMA_AC_CMAX: u32 = 10;

    /// LSB-first bit accumulator for the production AC-strip coder.
    ///
    /// Bits are buffered in a 64-bit word and copied to `out` a byte at a time. `out` is incomplete
    /// until [`Bits::finish`] flushes the final zero-padded partial byte.
    struct Bits {
        out: KVec<u8>,
        /// Pending bits, LSB-first, valid in the low `nacc`.
        acc: u64,
        nacc: u32,
    }

    impl Bits {
        fn new() -> Self {
            Self {
                out: KVec::new(),
                acc: 0,
                nacc: 0,
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
        fn finish(mut self) -> Result<KVec<u8>> {
            let nbytes = self.nacc.div_ceil(8) as usize;
            for k in 0..nbytes {
                self.out.push((self.acc >> (8 * k)) as u8, GFP_KERNEL)?;
            }
            Ok(self.out)
        }

        /// The shared escape value code: a 0 is one `0` bit; else `unary(c) ++ [0-term IFF c<cmax]
        /// ++ offset(c-1, MSB-first) ++ sign(1=positive)`. `c = bit_length(|v|)`.
        fn esc(&mut self, v: i32, cmax: u32) -> Result {
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
            for _ in 0..c {
                self.bit(1)?;
            }
            if c < cmax {
                self.bit(0)?;
            }
            for i in (0..c.saturating_sub(1)).rev() {
                self.bit(off >> i)?;
            }
            self.bit(u32::from(v > 0))
        }

        /// Per-block luma significance code recovered across every last position in 1..63.
        /// For `k=floor(log2(64-last))`, emit `00`, `k` one bits, `0`, then the `k`-bit
        /// MSB-first value `(64-2^k)-last`. A flat block retains its separate 15-bit code.
        fn sync_unit_after(&mut self, last: usize, mut skip: usize) -> Result {
            fn emit(bits: &mut Bits, value: u32, skip: &mut usize) -> Result {
                if *skip != 0 {
                    *skip -= 1;
                    Ok(())
                } else {
                    bits.bit(value)
                }
            }
            if last == 0 {
                for &bit in &[0u32, 0, 1, 1, 1, 1, 1, 1] {
                    emit(self, bit, &mut skip)?;
                }
                for _ in 0..7 {
                    emit(self, 0, &mut skip)?;
                }
            } else {
                debug_assert!(last < COEFFS);
                emit(self, 0, &mut skip)?;
                emit(self, 0, &mut skip)?;
                let remaining = COEFFS - last;
                let k = usize::BITS - 1 - remaining.leading_zeros();
                for _ in 0..k {
                    emit(self, 1, &mut skip)?;
                }
                emit(self, 0, &mut skip)?;
                let end = COEFFS - (1usize << k);
                let v = (end - last) as u32;
                for i in (0..k).rev() {
                    emit(self, (v >> i) & 1, &mut skip)?;
                }
            }
            debug_assert_eq!(skip, 0);
            Ok(())
        }

        fn sync_unit(&mut self, last: usize) -> Result {
            self.sync_unit_after(last, 0)
        }
    }

    /// Number of blocks across a 64-px-wide strip and the rows of blocks within its 16-px height.
    const STRIP_BLOCKS_X: usize = 8;
    const STRIP_ROW_BLOCKS: usize = 8; // blocks in one 8-row half (8 across x 1 down)
    const STRIP_BLOCKS: usize = 16; // 8 across x 2 down

    /// Round a byte count up to an even number (every coder sub-region is even-aligned).
    fn round_even(n: usize) -> usize {
        n + (n & 1)
    }

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
    /// Chroma AC quantizer. Coarse bands 1/2 and 4..11 use step 16; coefficient 3 and positions
    /// 12..47 use step 32; the final HH band uses step 64. All use signed half-up rounding.
    ///
    /// Shifts rather than divisions, for the reason given on [`step_bias`]; steps 16/32/64 are
    /// shifts 4/5/6 and `step / 2` is `1 << (shift - 1)`.
    fn quantize_chroma_ac(coeff: i32, i: usize) -> i32 {
        let shift = CHROMA_AC_SHIFT[i];
        (coeff + (1 << (shift - 1))) >> shift
    }

    /// Per-plane DC quantizer, round-half-up on the SIGNED value (toward +inf): luma (plane 0)
    /// step 16, chroma step 64. `+224/64 = 3.5 -> 4`; `-8416/64 = -131.5 -> -131`.
    fn quantize_dc_round(plane: usize, v: i32) -> i32 {
        let shift: u32 = if plane == 0 { 4 } else { 6 };
        (v + (1 << (shift - 1))) >> shift
    }

    impl Bits {
        /// Exact chroma last-position tree node. For
        /// `c=floor(log2(last+1))`, emit `1`x`c`, `0`, then the `c`-bit MSB-first
        /// offset `last-(2^c-1)`.
        fn chroma_base(&mut self, last: usize) -> Result {
            debug_assert!(last > 0 && last < COEFFS);
            let c = usize::BITS - 1 - (last + 1).leading_zeros();
            for _ in 0..c {
                self.bit(1)?;
            }
            self.bit(0)?;
            let offset = (last - ((1usize << c) - 1)) as u32;
            for bit in (0..c).rev() {
                self.bit((offset >> bit) & 1)?;
            }
            Ok(())
        }

        /// One block's three-plane significance tree. The luma code begins with two
        /// zero root branches. A present Cr replaces the first with its chroma node; a present Cb
        /// replaces the second.
        fn colour_sync_unit(&mut self, lcr: usize, lcb: usize, ly: usize) -> Result {
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
        fn colour_block_ac(
            &mut self,
            qcr: &[i32; COEFFS],
            qcb: &[i32; COEFFS],
            qy: &[i32; COEFFS],
            lcr: usize,
            lcb: usize,
            ly: usize,
        ) -> Result {
            for &(q, last, cmax) in &[
                (qcr, lcr, CHROMA_AC_CMAX),
                (qcb, lcb, CHROMA_AC_CMAX),
                (qy, ly, AC_CMAX),
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

    /// One quantized colour block: the three planes' 64 coefficients and exact last-significant
    /// AC positions. Built by [`colour_block`] from a block's per-plane samples.
    pub(crate) struct ColourBlock {
        qcr: [i32; COEFFS],
        qcb: [i32; COEFFS],
        qy: [i32; COEFFS],
        lcr: usize,
        lcb: usize,
        ly: usize,
    }

    /// Return the exact chroma AC extent.
    /// Only the KUnit tests call this now: `colour_block` folds the same search into the pass that
    /// writes the coefficients, so the production path never scans them a second time.
    #[cfg(CONFIG_DRM_VINO_KUNIT_TEST)]
    pub(crate) fn chroma_last(q: &[i32; COEFFS]) -> usize {
        (1..COEFFS).rev().find(|&i| q[i] != 0).unwrap_or(0)
    }

    /// Transform + quantize one block's three planes (each 64 samples in the codec's x64 fixed
    /// point: `cr[i] = 64*(B-G)`, `cb[i] = 64*(R-G)`, `y[i] = 64*G + 64*((Cb+Cr)>>2)`). Luma uses
    /// the per-position `quantize`; chroma AC uses `quantize_chroma_ac`; all DCs use
    /// `quantize_dc_round`.
    ///
    /// `#[inline(never)]`: see `haar2d`'s doc comment -- part of the kernel-stack-overflow fix.
    #[inline(never)]
    pub(crate) fn colour_block(
        cr: &[i32; PIXELS],
        cb: &[i32; PIXELS],
        y: &[i32; PIXELS],
    ) -> ColourBlock {
        let tcr = transform(cr);
        let tcb = transform(cb);
        let ty = transform(y);
        // Quantise all three planes and find each one's last significant coefficient in a single
        // pass. These were separate steps, so every block ran three further 63-element reverse
        // scans ([`chroma_last`]) across arrays it had only just written. Folding the search into
        // the write removes those passes and keeps the three planes in cache together.
        //
        // An explicit ascending loop rather than `core::array::from_fn`: the fold depends on the
        // index order, and `from_fn` does not document one. Every element is written, so the zero
        // initialiser costs nothing.
        let mut qcr = [0i32; COEFFS];
        let mut qcb = [0i32; COEFFS];
        let mut qy = [0i32; COEFFS];
        let (mut lcr, mut lcb, mut ly) = (0usize, 0usize, 0usize);
        qcr[0] = quantize_dc_round(2, tcr[0]);
        qcb[0] = quantize_dc_round(1, tcb[0]);
        qy[0] = quantize_dc_round(0, ty[0]);
        // Coefficient 0 is deliberately excluded: the last-significant index is over 1..COEFFS.
        for i in 1..COEFFS {
            let (vcr, vcb, vy) = (
                quantize_chroma_ac(tcr[i], i),
                quantize_chroma_ac(tcb[i], i),
                quantize(ty[i], i),
            );
            if vcr != 0 {
                lcr = i;
            }
            if vcb != 0 {
                lcb = i;
            }
            if vy != 0 {
                ly = i;
            }
            qcr[i] = vcr;
            qcb[i] = vcb;
            qy[i] = vy;
        }
        ColourBlock {
            qcr,
            qcb,
            qy,
            lcr,
            lcb,
            ly,
        }
    }

    /// Encode one 64x16 COLOUR strip at pixel `(x, y)` from its 16 quantized colour blocks
    /// (raster: 0..8 top 8-px half, 8..16 bottom).
    ///
    /// `#[inline(never)]`: see `haar2d`'s doc comment -- part of the kernel-stack-overflow fix.
    #[inline(never)]
    pub(crate) fn colour_strip(blocks: &[ColourBlock], x: u16, y: u16) -> Result<KVec<u8>> {
        let mut main = Bits::new();
        for b in blocks {
            main.colour_sync_unit(b.lcr, b.lcb, b.ly)?;
        }
        let (mut pcr, mut pcb, mut py) = (0i32, 0i32, 0i32);
        for b in blocks {
            let (cr, cb, yv) = (b.qcr[0], b.qcb[0], b.qy[0]);
            main.esc(cr - pcr, SOLID_DC_CMAX)?;
            main.esc(cb - pcb, SOLID_DC_CMAX)?;
            main.esc(yv - py, SOLID_DC_CMAX)?;
            (pcr, pcb, py) = (cr, cb, yv);
        }
        let mut row0 = Bits::new();
        for b in &blocks[..STRIP_ROW_BLOCKS] {
            row0.colour_block_ac(&b.qcr, &b.qcb, &b.qy, b.lcr, b.lcb, b.ly)?;
        }
        let mut row1 = Bits::new();
        for b in &blocks[STRIP_ROW_BLOCKS..] {
            row1.colour_block_ac(&b.qcr, &b.qcb, &b.qy, b.lcr, b.lcb, b.ly)?;
        }

        // `Bits` buffers into a word, so `out` is only complete once finished -- take all three
        // before any length is read.
        let (main, row0, row1) = (main.finish()?, row0.finish()?, row1.finish()?);
        let r0 = round_even(row0.len());
        let r1 = round_even(row1.len());
        let main_b = round_even(main.len()) + 2;
        let w18 = 16 + main_b;
        let w1c = w18 + r0;
        // The 2-byte tail overlaps the end of the row1 region (len = w1c + round_even(row1)).
        let len = w1c + r1;

        let mut out = KVec::new();
        out.resize(len, 0, GFP_KERNEL)?;
        out[0] = 0x01;
        out[1] = 0x28;
        out[2..4].copy_from_slice(&x.to_le_bytes());
        // Strip y is the band's top edge.
        out[4..6].copy_from_slice(&y.to_le_bytes());
        out[10..12].copy_from_slice(&(w18 as u16).to_le_bytes());
        out[12..14].copy_from_slice(&(w1c as u16).to_le_bytes());
        out[16..16 + main.len()].copy_from_slice(&main);
        out[w18..w18 + row0.len()].copy_from_slice(&row0);
        out[w1c..w1c + row1.len()].copy_from_slice(&row1);
        // No forward-hint tail: on the EP08 wire the strip's last 2 bytes are the natural row1
        // bit-packing. The record framing carries the length as `strip_id == len`, so the in-strip
        // echo the sink hook showed is not transmitted on the wire. See `frame_records`.
        Ok(out)
    }

    /// Strip pixel geometry: a strip is 64 px wide and 16 px tall
    /// (`STRIP_BLOCKS_X * DIM` x `2 * DIM`).
    pub(crate) const STRIP_W: usize = STRIP_BLOCKS_X * DIM; // 64
    pub(crate) const STRIP_H: usize = 2 * DIM; // 16

    /// Damage macro-tile: 256x64 pixels, or 4x4 strips. A touched macro-tile must be resent in full
    /// because the dock rotates its backing buffers at this granularity.
    pub(crate) const MACRO_W: usize = 4 * STRIP_W; // 256
    pub(crate) const MACRO_H: usize = 4 * STRIP_H; // 64

    /// Gather one 64x16 strip's 16 colour blocks from a pixel source. `px(x, y)` returns the
    /// 8-bit `(R, G, B)` at absolute frame coordinate `(x, y)`; `(ox, oy)` is the strip's
    /// top-left pixel. Each block's three planes are built in the codec's x64 fixed point via
    /// [`colour`] (per-pixel `(Y, Cb, Cr)`, stored `(Cr, Cb, Y)` for [`colour_block`]). Blocks are
    /// raster order within the strip (0..8 top 8-px half, 8..16 bottom), matching [`colour_strip`].
    ///
    /// `#[inline(never)]`: see `haar2d`'s doc comment -- part of the kernel-stack-overflow fix.
    /// The block array is heap allocated because it is about 6.5 KiB and nested copies can exhaust
    /// the kernel stack.
    #[inline(never)]
    fn colour_strip_blocks(
        ox: usize,
        oy: usize,
        px: &mut impl FnMut(usize, usize) -> (u8, u8, u8),
    ) -> Result<KVec<ColourBlock>> {
        let mut blocks = KVec::with_capacity(STRIP_BLOCKS, GFP_KERNEL)?;
        for k in 0..STRIP_BLOCKS {
            let (bx, by) = (k % STRIP_BLOCKS_X, k / STRIP_BLOCKS_X);
            // One `px` call per pixel, in the same row-major order as before -- it is the
            // expensive accessor, so the converted values are gathered once and then split into
            // the three planes. `from_fn` avoids the zeroing that filling `[0i32; PIXELS]`
            // afterwards would leave behind.
            // Left as a zero-initialised fill on purpose. Gathering into an interleaved
            // array and splitting it (as `colour_block`/`transform` now do) removes the `memset`
            // but costs ~736 bytes of stack here, and this path already runs ~8 KB deep inside a
            // 16 KB kernel stack -- the same budget the `#[inline(never)]` markers protect.
            let (mut cr, mut cb, mut y) = ([0i32; PIXELS], [0i32; PIXELS], [0i32; PIXELS]);
            // Monotonic indexing lets LLVM prove that every element is initialized before use.
            let mut i = 0usize;
            for r in 0..DIM {
                for c in 0..DIM {
                    let (rr, gg, bb) = px(ox + bx * DIM + c, oy + by * DIM + r);
                    let (yv, cbv, crv) = colour(rr, gg, bb);
                    (cr[i], cb[i], y[i]) = (crv, cbv, yv);
                    i += 1;
                }
            }
            blocks.push(colour_block(&cr, &cb, &y), GFP_KERNEL)?;
        }
        Ok(blocks)
    }

    /// Encode a full `width`x`height` RGB frame into WHT colour records. `px(x, y)` yields the
    /// source pixel's `(R, G, B)`; the caller applies rotation, gamma and format conversion. The
    /// surface is tiled into 64x16 strips in raster order, each built from [`colour_block`] +
    /// [`colour_strip`], and the strip stream is framed for the wire by [`frame_records`] using
    /// the EP08 TLV layout. `seq0` is the logical frame number and the returned value is advanced
    /// for the next frame.
    ///
    /// `width`/`height` must be multiples of 64 and 16 (`EINVAL` otherwise). Live scanout pads a
    /// non-aligned mode with black to this strip grid while preserving the real mode dimensions.
    /// This function remains out of line to bound kernel stack use.
    #[inline(never)]
    pub(crate) fn colour_frame_ep08(
        width: usize,
        height: usize,
        seq0: u32,
        head: u8,
        band_parity: bool,
        interlaced: bool,
        mut px: impl FnMut(usize, usize) -> (u8, u8, u8),
    ) -> Result<(KVec<KVec<u8>>, u32)> {
        if width % STRIP_W != 0 || height % STRIP_H != 0 {
            return Err(kernel::error::code::EINVAL);
        }
        // Build every strip body (raster order; each strip's natural row1 tail, no echo).
        let mut strips: KVec<KVec<u8>> = KVec::new();
        let mut sy = 0usize;
        while sy < height {
            let mut sx = 0usize;
            while sx < width {
                let blocks = colour_strip_blocks(sx, sy, &mut px)?;
                strips.push(colour_strip(&blocks, sx as u16, sy as u16)?, GFP_KERNEL)?;
                sx += STRIP_W;
            }
            sy += STRIP_H;
        }
        Ok((
            frame_records(&strips, head, band_parity, interlaced)?,
            seq0.wrapping_add(1),
        ))
    }

    /// Build a valid all-black WHT frame without sampling or transforming a framebuffer.
    ///
    /// This is the post-mode-set training carrier. Its zero-coefficient blocks are built once and
    /// reused, so construction is proportional to the strip grid rather than the pixel count. A
    /// real framebuffer keyframe follows.
    pub(crate) fn black_frame_ep08(
        width: usize,
        height: usize,
        head: u8,
        band_parity: bool,
        interlaced: bool,
    ) -> Result<KVec<KVec<u8>>> {
        if width % STRIP_W != 0 || height % STRIP_H != 0 {
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
                strips.push(colour_strip(&blocks, sx as u16, sy as u16)?, GFP_KERNEL)?;
                sx += STRIP_W;
            }
            sy += STRIP_H;
        }
        frame_records(&strips, head, band_parity, interlaced)
    }

    /// Encode ONE 64x16 strip whose top-left output pixel is `(sx, sy)`.
    ///
    /// This is the unit of work both frame encoders are built from, and it is the reason a
    /// parallel encode is possible at all: a strip reads only its own 64x16 region through `px`
    /// and produces its own independent byte vector, sharing no state with any other strip. The
    /// scanout encoder in `drm_sink.rs` fans batches of these across CPUs; see `EncodeChunk`.
    pub(crate) fn colour_strip_at(
        sx: usize,
        sy: usize,
        px: &mut impl FnMut(usize, usize) -> (u8, u8, u8),
    ) -> Result<KVec<u8>> {
        let blocks = colour_strip_blocks(sx, sy, px)?;
        colour_strip(&blocks, sx as u16, sy as u16)
    }

    /// The raster-ordered top-left coordinate of every strip a damage set selects.
    ///
    /// Split out of [`colour_frame_ep08_damage`] so the serial and parallel encoders select
    /// exactly the same strips in exactly the same order. **The order is load-bearing:**
    /// [`frame_records`] groups strips into one record per single-Y band and requires them
    /// x-ordered within each band, so reordering here changes the wire format.
    ///
    /// A strip is selected when a clip overlaps its 256x64 macro-tile. Every strip in a touched
    /// tile is resent.
    pub(crate) fn damage_strip_coords(
        width: usize,
        height: usize,
        clips: &[(usize, usize, usize, usize)],
    ) -> Result<KVec<(usize, usize)>> {
        let mut coords: KVec<(usize, usize)> = KVec::new();
        let mut sy = 0usize;
        while sy < height {
            let mut sx = 0usize;
            while sx < width {
                let mx = (sx / MACRO_W) * MACRO_W;
                let my = (sy / MACRO_H) * MACRO_H;
                let hit = clips.iter().any(|&(x0, y0, x1, y1)| {
                    mx < x1 && x0 < mx + MACRO_W && my < y1 && y0 < my + MACRO_H
                });
                if hit {
                    coords.push((sx, sy), GFP_KERNEL)?;
                }
                sx += STRIP_W;
            }
            sy += STRIP_H;
        }
        Ok(coords)
    }

    /// Every strip of a full frame, in the same raster order as [`damage_strip_coords`].
    pub(crate) fn all_strip_coords(width: usize, height: usize) -> Result<KVec<(usize, usize)>> {
        let mut coords: KVec<(usize, usize)> = KVec::new();
        let mut sy = 0usize;
        while sy < height {
            let mut sx = 0usize;
            while sx < width {
                coords.push((sx, sy), GFP_KERNEL)?;
                sx += STRIP_W;
            }
            sy += STRIP_H;
        }
        Ok(coords)
    }

    /// Damage-aware variant of [`colour_frame_ep08`]. It encodes only the macro-tiles selected by
    /// the client damage rectangles.
    ///
    /// `clips` are `(x0, y0, x1, y1)` half-open rectangles in output/source pixels (identity
    /// rotation only -- the caller sends a full [`colour_frame_ep08`] otherwise). A strip at
    /// `(sx, sy)` is included iff some clip overlaps
    /// `[sx, sx+STRIP_W) x [sy, sy+STRIP_H)`. Raster iteration keeps strips
    /// x-ordered within each y-band, so [`frame_records`] groups them as the
    /// full-frame path does. Returns an **empty** frame list when no strip is
    /// touched -- the caller must skip the USB write in that case (no-op
    /// flip). **The first frame after a mode-set must still be a full
    /// keyframe** (the dock's framebuffer is undefined until then).
    ///
    /// This function remains out of line for the same stack bound as [`colour_frame_ep08`].
    #[inline(never)]
    pub(crate) fn colour_frame_ep08_damage(
        width: usize,
        height: usize,
        seq0: u32,
        head: u8,
        clips: &[(usize, usize, usize, usize)],
        band_parity: bool,
        interlaced: bool,
        mut px: impl FnMut(usize, usize) -> (u8, u8, u8),
    ) -> Result<(KVec<KVec<u8>>, u32)> {
        if width % STRIP_W != 0 || height % STRIP_H != 0 {
            return Err(kernel::error::code::EINVAL);
        }
        let coords = damage_strip_coords(width, height, clips)?;
        let mut strips: KVec<KVec<u8>> = KVec::with_capacity(coords.len(), GFP_KERNEL)?;
        for &(sx, sy) in coords.iter() {
            let blocks = colour_strip_blocks(sx, sy, &mut px)?;
            strips.push(colour_strip(&blocks, sx as u16, sy as u16)?, GFP_KERNEL)?;
        }
        let records = frame_records(&strips, head, band_parity, interlaced)?;
        Ok((records, seq0.wrapping_add(1)))
    }

    /// A strip's `y` (the EP08 record bands group strips by row). Reads the `y` field the strip
    /// builders write at byte offset 4 ([`colour_strip`] / [`solid_strip`]).
    fn strip_y(s: &[u8]) -> u16 {
        u16::from_le_bytes([s[4], s[5]])
    }

    /// Frame a raster-ordered list of strip bodies into EP08 records:
    ///
    /// ```text
    /// record (one per single-Y band of strips):
    ///   u16 pad   = 0
    ///   u16 size  = total record length (TLV..trailer, excludes the inter-record gap)
    ///   u32 type  = 4
    ///   u16 sub   = head | (((y / 16) & 1) << 4)
    ///   u16 aux   = zero-padding byte count
    ///   u32 fseq  = 0
    ///   per strip: u16 strip_id (== strip length) ++ strip bytes
    ///   u8[aux]   zero padding so the complete `size + 4` stride is 16-byte aligned
    /// ```
    ///
    /// The record stream is chunked internally into small record-aligned buffers. The sink streams
    /// those fragments directly into persistent 65536-byte URBs, so the internal boundaries are
    /// not USB framing. Complete record strides are limited to 4080 bytes.
    pub(crate) fn frame_records(
        strips: &[KVec<u8>],
        head: u8,
        band_parity_bit: bool,
        interlaced_bands: bool,
    ) -> Result<KVec<KVec<u8>>> {
        const PREFIX: usize = 8;
        const STRIDE_CAP: usize = 0x0ff0;
        // Allocation boundary only, not wire framing.
        const CHUNK: usize = 0x4000;
        let mut frames: KVec<KVec<u8>> = KVec::new();
        let mut chunk: KVec<u8> = KVec::new();
        // Interlaced ordering sends even bands before odd bands while preserving x order.
        let mut order: KVec<usize> = KVec::with_capacity(strips.len(), GFP_KERNEL)?;
        if interlaced_bands {
            for pass in 0..2u16 {
                for (n, s) in strips.iter().enumerate() {
                    if (strip_y(s) / STRIP_H as u16) & 1 == pass {
                        order.push(n, GFP_KERNEL)?;
                    }
                }
            }
        } else {
            for n in 0..strips.len() {
                order.push(n, GFP_KERNEL)?;
            }
        }
        let mut i = 0usize;
        while i < order.len() {
            let y0 = strip_y(&strips[order[i]]);
            let mut record: KVec<u8> = KVec::new();
            // Sub bit 4 carries the y-band parity when the selected framing
            // profile requires it.
            record.extend_from_slice(&[0u8; 8 + PREFIX], GFP_KERNEL)?;
            record[4..8].copy_from_slice(&4u32.to_le_bytes());
            let parity = u16::from(band_parity_bit) & ((y0 / STRIP_H as u16) & 1);
            let sub = head as u16 | (parity << 4);
            record[8..10].copy_from_slice(&sub.to_le_bytes());
            let mut n = 0usize;
            while i < order.len() && strip_y(&strips[order[i]]) == y0 {
                let s = &strips[order[i]];
                let projected = record.len() + 2 + s.len();
                let projected_aligned = (projected + 15) & !15;
                if n > 0 && projected_aligned > STRIDE_CAP {
                    break;
                }
                record.extend_from_slice(&(s.len() as u16).to_le_bytes(), GFP_KERNEL)?;
                record.extend_from_slice(s, GFP_KERNEL)?;
                n += 1;
                i += 1;
            }
            // The wire format pads each complete record stride to 16 bytes and carries that count
            // in `aux`. There is no additional trailer or inter-record gap: `size` counts from
            // after the four-byte pad+size prefix, so `stride = size + 4` lands on the next record.
            let pad = (16 - (record.len() & 15)) & 15;
            record[10..12].copy_from_slice(&(pad as u16).to_le_bytes());
            record.extend_from_slice(&[0u8; 15][..pad], GFP_KERNEL)?;
            let size = (record.len() - 4) as u16;
            record[2..4].copy_from_slice(&size.to_le_bytes());

            if !chunk.is_empty() && chunk.len() + record.len() > CHUNK {
                frames.push(chunk, GFP_KERNEL)?;
                chunk = KVec::new();
            }
            chunk.extend_from_slice(&record, GFP_KERNEL)?;
        }
        if !chunk.is_empty() {
            frames.push(chunk, GFP_KERNEL)?;
        }
        Ok(frames)
    }

    /// Build the three 32-byte end-of-frame records.
    ///
    /// They delimit every logical frame, including the ARM-prefixed first frame. The first record
    /// carries a wrapping one-based frame counter; all three carry a three-slot phase (`0,2,4`) and
    /// the selected head.
    pub(crate) fn frame_trailer(head: u8, seq0: u32) -> [u8; 96] {
        let phase = ((seq0 % 3) as u8) * 2;
        let next_phase = (phase + 2) % 6;
        let phase_off = phase * 4;
        let next_off = next_phase * 4;
        let frame_no = (seq0 as u8).wrapping_add(1);
        let mut out = [0u8; 96];

        for (i, head_byte) in [head, head, head | 0x10].into_iter().enumerate() {
            let o = i * 32;
            out[o + 2] = 0x1c; // size=28 -> 32-byte record
            out[o + 4] = 0x04; // type=4
                               // `sub` is the little-endian u16 at bytes 8..10.
            out[o + 8] = head_byte;
            out[o + 10] = if i == 0 { 0x06 } else { 0x04 };
        }

        // Record A: frame-present marker + current ring phase + one-based u8 frame number.
        out[16] = 0x08;
        out[18] = 0x05;
        out[19] = phase;
        out[23] = phase_off;
        out[25] = frame_no;

        // Records B/C are identical apart from C's head|0x10 header selector.
        for o in [32usize, 64] {
            out[o + 16] = 0x0a;
            out[o + 18] = 0x04;
            out[o + 19] = next_phase;
            out[o + 23] = next_off;
            out[o + 27] = phase_off;
        }
        out
    }
}
