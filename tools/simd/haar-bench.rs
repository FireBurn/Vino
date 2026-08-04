//! Is a vectorised Haar transform worth an in-kernel FPU section, and does it produce identical
//! bytes?
//!
//!     rustc -O tools/simd/haar-bench.rs -o /tmp/haar-bench && /tmp/haar-bench
//!
//! Build it twice. `-C target-cpu=native` auto-vectorises the scalar baseline and understates the
//! gain; the kernel's own flags are the representative case:
//!
//!     rustc -O -C target-feature=-sse,-sse2,-sse3,-ssse3,-sse4.1,-sse4.2,-avx,-avx2 \
//!         tools/simd/haar-bench.rs -o /tmp/haar-bench-kflags
//!
//! Userspace on purpose: none of this changes the arithmetic, and the arithmetic is what decides
//! whether the plumbing is worth adding. The one thing userspace cannot measure is the cost of
//! `kernel_fpu_begin()`/`kernel_fpu_end()`, so the summary reports the per-call saving and the
//! break-even FPU cost instead of guessing at it. `tools/simd/fpu-cost.md` says how to measure the
//! other half.
//!
//! The scalar side is `video::wht`'s transform copied verbatim, so a mismatch here is a real
//! mismatch. The codec is byte-exact against DisplayLink's own encoder, so "faster" is worthless
//! without "identical" -- the check runs first and the benchmark refuses to report a speedup if it
//! fails.
//!
//! ⚠ **Read the batch-3 row, not the full-lane row.** `colour_block` transforms exactly three
//! blocks together -- `cr`, `cb`, `y` -- so a vector path processing eight or sixteen at a time
//! leaves most of its lanes idle on the encoder's real workload. The full-lane row is the ceiling
//! that would need the encode loop restructured to batch across strips; the batch-3 row is what
//! adding an intrinsic today would actually buy.

use std::arch::x86_64::*;
use std::time::Instant;

const PIXELS: usize = 64;
const COEFFS: usize = 64;
/// Blocks `colour_block` transforms together: the `cr`, `cb` and `y` planes of one 8x8 block.
const ENCODER_BATCH: usize = 3;

// ---------------------------------------------------------------- scalar, copied from video::wht

macro_rules! haar2d_level {
    ($name:ident, $n:literal, $h:literal) => {
        #[inline(always)]
        fn $name(
            src: &[i32; $n * $n],
            ll: &mut [i32; $h * $h],
            hl: &mut [i32; $h * $h],
            lh: &mut [i32; $h * $h],
            hh: &mut [i32; $h * $h],
        ) {
            let mut l = [0i32; $n * $h];
            let mut hb = [0i32; $n * $h];
            for r in 0..$n {
                for i in 0..$h {
                    let (a, b) = (src[r * $n + 2 * i], src[r * $n + 2 * i + 1]);
                    l[r * $h + i] = a + b;
                    hb[r * $h + i] = a - b;
                }
            }
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

const SCAN4_MORTON: [usize; 16] = [0, 2, 8, 10, 1, 3, 9, 11, 4, 6, 12, 14, 5, 7, 13, 15];

fn transform_scalar(block: &[i32; PIXELS]) -> [i32; COEFFS] {
    let sh = |x: i32| x >> 6;
    let (mut ll1, mut hl1, mut lh1, mut hh1) = ([0i32; 16], [0i32; 16], [0i32; 16], [0i32; 16]);
    haar2d_8(block, &mut ll1, &mut hl1, &mut lh1, &mut hh1);
    let (mut ll2, mut hl2, mut lh2, mut hh2) = ([0i32; 4], [0i32; 4], [0i32; 4], [0i32; 4]);
    haar2d_4(&ll1, &mut ll2, &mut hl2, &mut lh2, &mut hh2);
    let (mut ll3, mut hl3, mut lh3, mut hh3) = ([0i32; 1], [0i32; 1], [0i32; 1], [0i32; 1]);
    haar2d_2(&ll2, &mut ll3, &mut hl3, &mut lh3, &mut hh3);
    let mut out = [0i32; COEFFS];
    out[0] = sh(ll3[0]);
    out[1] = sh(hl3[0]);
    out[2] = sh(lh3[0]);
    out[3] = sh(hh3[0]);
    for i in 0..4 {
        out[4 + i] = sh(hl2[i]);
    }
    for i in 0..4 {
        out[8 + i] = sh(lh2[i]);
    }
    for i in 0..4 {
        out[12 + i] = sh(hh2[i]);
    }
    for (i, &s) in SCAN4_MORTON.iter().enumerate() {
        out[16 + i] = sh(hl1[s]);
    }
    for (i, &s) in SCAN4_MORTON.iter().enumerate() {
        out[32 + i] = sh(lh1[s]);
    }
    for (i, &s) in SCAN4_MORTON.iter().enumerate() {
        out[48 + i] = sh(hh1[s]);
    }
    out
}

// ---------------------------------------------------------------- vector paths
//
// One lane per block, so pairing neighbours needs no shuffle: element `i` of a vector is the same
// coefficient of `LANES` different blocks. Loads and stores go through `read_unaligned`/
// `write_unaligned` rather than the width-specific `_mm*_loadu_*` intrinsics, whose signatures
// differ between widths and have changed between compiler releases.

macro_rules! simd_transform {
    ($name:ident, $feature:literal, $vec:ty, $lanes:literal, $set0:ident, $add:ident, $sub:ident, $sra:ident) => {
        /// Transform `$lanes` blocks at once. `blocks` and `out` must both hold `$lanes` entries;
        /// lanes past the caller's real batch are transformed and discarded.
        #[target_feature(enable = $feature)]
        unsafe fn $name(blocks: &[[i32; PIXELS]], out: &mut [[i32; COEFFS]]) {
            assert!(blocks.len() >= $lanes && out.len() >= $lanes);

            #[target_feature(enable = $feature)]
            unsafe fn level<const N: usize, const H: usize>(
                src: &[$vec],
                ll: &mut [$vec],
                hl: &mut [$vec],
                lh: &mut [$vec],
                hh: &mut [$vec],
            ) {
                let mut l = [$set0(); 64];
                let mut hb = [$set0(); 64];
                for r in 0..N {
                    for i in 0..H {
                        let (a, b) = (src[r * N + 2 * i], src[r * N + 2 * i + 1]);
                        l[r * H + i] = $add(a, b);
                        hb[r * H + i] = $sub(a, b);
                    }
                }
                for c in 0..H {
                    for i in 0..H {
                        let (a, b) = (l[2 * i * H + c], l[(2 * i + 1) * H + c]);
                        ll[i * H + c] = $add(a, b);
                        lh[i * H + c] = $sub(a, b);
                        let (a2, b2) = (hb[2 * i * H + c], hb[(2 * i + 1) * H + c]);
                        hl[i * H + c] = $add(a2, b2);
                        hh[i * H + c] = $sub(a2, b2);
                    }
                }
            }

            // Transpose to lanes-per-coefficient once; every stage below is then shuffle-free.
            let mut src = [$set0(); PIXELS];
            let mut tmp = [0i32; $lanes];
            for (p, slot) in src.iter_mut().enumerate() {
                for (b, t) in tmp.iter_mut().enumerate() {
                    *t = blocks[b][p];
                }
                *slot = std::ptr::read_unaligned(tmp.as_ptr() as *const $vec);
            }

            let z = $set0();
            let (mut ll1, mut hl1, mut lh1, mut hh1) = ([z; 16], [z; 16], [z; 16], [z; 16]);
            level::<8, 4>(&src, &mut ll1, &mut hl1, &mut lh1, &mut hh1);
            let (mut ll2, mut hl2, mut lh2, mut hh2) = ([z; 4], [z; 4], [z; 4], [z; 4]);
            level::<4, 2>(&ll1, &mut ll2, &mut hl2, &mut lh2, &mut hh2);
            let (mut ll3, mut hl3, mut lh3, mut hh3) = ([z; 1], [z; 1], [z; 1], [z; 1]);
            level::<2, 1>(&ll2, &mut ll3, &mut hl3, &mut lh3, &mut hh3);

            // `>> 6` is an arithmetic shift in both paths.
            let mut store = |coeff: usize, v: $vec, out: &mut [[i32; COEFFS]]| {
                let mut lanes = [0i32; $lanes];
                std::ptr::write_unaligned(lanes.as_mut_ptr() as *mut $vec, $sra(v, 6));
                for (b, val) in lanes.iter().enumerate() {
                    out[b][coeff] = *val;
                }
            };
            store(0, ll3[0], out);
            store(1, hl3[0], out);
            store(2, lh3[0], out);
            store(3, hh3[0], out);
            for i in 0..4 {
                store(4 + i, hl2[i], out);
            }
            for i in 0..4 {
                store(8 + i, lh2[i], out);
            }
            for i in 0..4 {
                store(12 + i, hh2[i], out);
            }
            for (i, &s) in SCAN4_MORTON.iter().enumerate() {
                store(16 + i, hl1[s], out);
            }
            for (i, &s) in SCAN4_MORTON.iter().enumerate() {
                store(32 + i, lh1[s], out);
            }
            for (i, &s) in SCAN4_MORTON.iter().enumerate() {
                store(48 + i, hh1[s], out);
            }
        }
    };
}

simd_transform!(
    transform_avx2,
    "avx2",
    __m256i,
    8,
    _mm256_setzero_si256,
    _mm256_add_epi32,
    _mm256_sub_epi32,
    _mm256_srai_epi32
);

simd_transform!(
    transform_avx512,
    "avx512f",
    __m512i,
    16,
    _mm512_setzero_si512,
    _mm512_add_epi32,
    _mm512_sub_epi32,
    _mm512_srai_epi32
);

// ---------------------------------------------------------------- harness

/// Deterministic pseudo-random blocks spanning the 8-bit input range the codec sees.
fn make_blocks(n: usize) -> Vec<[i32; PIXELS]> {
    let mut state = 0x2545_f491_4f6c_dd1du64;
    let mut rnd = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    (0..n)
        .map(|_| {
            let mut b = [0i32; PIXELS];
            for px in b.iter_mut() {
                *px = (rnd() % 256) as i32;
            }
            b
        })
        .collect()
}

struct Row {
    name: &'static str,
    lanes: usize,
    /// Seconds to transform one batch of `ENCODER_BATCH` blocks, the encoder's real call shape.
    per_encoder_batch_s: f64,
    /// Seconds per block when every lane is fed, i.e. the ceiling if the encode loop batched
    /// across strips.
    per_block_full_s: f64,
}

fn bench<F>(lanes: usize, name: &'static str, blocks: &[[i32; PIXELS]], reps: usize, mut run: F) -> Row
where
    F: FnMut(&[[i32; PIXELS]], &mut [[i32; COEFFS]]),
{
    let mut out = vec![[0i32; COEFFS]; lanes];
    // Full lanes: every call does `lanes` useful blocks.
    let batches = blocks.len() / lanes;
    let t = Instant::now();
    for _ in 0..reps {
        for c in 0..batches {
            run(&blocks[c * lanes..(c + 1) * lanes], &mut out);
        }
    }
    let full = t.elapsed().as_secs_f64() / (reps * batches * lanes) as f64;

    // The encoder's shape: one call per three blocks, whatever the lane count.
    let calls = blocks.len() / ENCODER_BATCH.max(lanes.min(ENCODER_BATCH));
    let t = Instant::now();
    for _ in 0..reps {
        for c in 0..blocks.len() / lanes.max(ENCODER_BATCH) {
            run(&blocks[c * lanes.max(ENCODER_BATCH)..], &mut out);
        }
    }
    let elapsed = t.elapsed().as_secs_f64();
    let _ = calls;
    let per_call = elapsed / (reps * (blocks.len() / lanes.max(ENCODER_BATCH))) as f64;

    Row {
        name,
        lanes,
        per_encoder_batch_s: per_call,
        per_block_full_s: full,
    }
}

fn main() {
    const BLOCKS: usize = 32768;
    const REPS: usize = 40;
    let blocks = make_blocks(BLOCKS);

    println!("== correctness (gates everything below)");
    let mut rows = Vec::new();

    // Scalar reference, and its own timing.
    let mut out = vec![[0i32; COEFFS]; 16];
    let t = Instant::now();
    let mut sink = 0i64;
    for _ in 0..REPS {
        for b in blocks.iter() {
            sink += transform_scalar(b)[0] as i64;
        }
    }
    let scalar_per_block = t.elapsed().as_secs_f64() / (REPS * BLOCKS) as f64;
    println!("   scalar is the oracle");

    macro_rules! check_and_bench {
        ($feat:literal, $f:ident, $lanes:literal, $label:literal) => {
            if is_x86_feature_detected!($feat) {
                let mut bad = 0usize;
                for c in 0..BLOCKS / $lanes {
                    let batch = &blocks[c * $lanes..(c + 1) * $lanes];
                    unsafe { $f(batch, &mut out) };
                    for (i, b) in batch.iter().enumerate() {
                        if transform_scalar(b) != out[i] {
                            bad += 1;
                        }
                    }
                }
                if bad != 0 {
                    println!("   {} MISMATCH on {bad} blocks -- stopping", $label);
                    std::process::exit(1);
                }
                println!("   {:<7} identical to scalar over {BLOCKS} blocks", $label);
                rows.push(bench($lanes, $label, &blocks, REPS, |b, o| unsafe {
                    $f(b, o)
                }));
            } else {
                println!("   {:<7} not supported on this CPU -- skipped", $label);
            }
        };
    }
    check_and_bench!("avx2", transform_avx2, 8, "avx2");
    check_and_bench!("avx512f", transform_avx512, 16, "avx512");

    let scalar_batch = scalar_per_block * ENCODER_BATCH as f64;
    println!("\n== the encoder's real call: {ENCODER_BATCH} blocks (cr, cb, y)");
    println!("   scalar  {:>8.1} ns/call", scalar_batch * 1e9);
    for r in rows.iter() {
        let idle = r.lanes.saturating_sub(ENCODER_BATCH);
        println!(
            "   {:<7} {:>8.1} ns/call   {:.2}x   ({idle} of {} lanes idle)",
            r.name,
            r.per_encoder_batch_s * 1e9,
            scalar_batch / r.per_encoder_batch_s,
            r.lanes
        );
    }

    println!("\n== ceiling, if the encode loop batched across strips to fill the lanes");
    println!("   scalar  {:>8.2} ns/block", scalar_per_block * 1e9);
    for r in rows.iter() {
        println!(
            "   {:<7} {:>8.2} ns/block   {:.2}x",
            r.name,
            r.per_block_full_s * 1e9,
            scalar_per_block / r.per_block_full_s
        );
    }

    println!("\n== is an FPU section worth it?");
    println!("   An in-kernel vector path must wrap each region in kernel_fpu_begin()/end().");
    println!("   A change pays for itself only when the saving exceeds that cost:");
    for r in rows.iter() {
        let save_call = (scalar_batch - r.per_encoder_batch_s) * 1e9;
        let save_block = (scalar_per_block - r.per_block_full_s) * 1e9;
        println!(
            "   {:<7} saves {:>7.1} ns per {ENCODER_BATCH}-block call, {:>6.2} ns per block at full lanes",
            r.name, save_call, save_block
        );
        if save_call > 0.0 {
            println!(
                "           -> break-even needs the FPU section under {:.0} ns, or one section \
                 amortised over {:.0} calls at a 200 ns cost",
                save_call,
                (200.0 / save_call).ceil()
            );
        } else {
            println!("           -> no saving at this batch size; the lanes are too empty");
        }
    }
    println!("\n   Measure the FPU section itself in-kernel; see tools/simd/fpu-cost.md.");
    if sink == i64::MIN {
        println!("unreachable {sink}");
    }
}
