//! Does an AVX2 Haar transform beat the scalar one, and does it produce identical bytes?
//!
//!     rustc -O -C target-cpu=native tools/simd/haar-bench.rs -o /tmp/haar-bench && /tmp/haar-bench
//!
//! Userspace on purpose. The kernel disables SIMD for Rust globally and vector registers need an
//! FPU section (see `docs/simd.md`), so a kernel prototype needs a binding that does not exist
//! yet. None of that changes the arithmetic, and the question worth answering first is whether the
//! speedup justifies the plumbing at all.
//!
//! The scalar side is `video::wht`'s transform copied verbatim, so a mismatch here is a real
//! mismatch. The codec is byte-exact against DisplayLink's own encoder, so "faster" is worthless
//! without "identical" -- the check runs first and the benchmark refuses to report a speedup if it
//! fails.
//!
//! The vectorisation is across blocks rather than within one. A single 8x8 block needs shuffles to
//! pair neighbours; eight blocks in eight lanes need none, so every operation is a straight vector
//! add or subtract and the transform's own structure is untouched.

use std::arch::x86_64::*;
use std::time::Instant;

const PIXELS: usize = 64;
const COEFFS: usize = 64;
const LANES: usize = 8;

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

// ---------------------------------------------------------------- AVX2, eight blocks at a time

/// One lane per block, so pairing neighbours needs no shuffle: `src[i]` is the same coefficient of
/// eight different blocks.
#[target_feature(enable = "avx2")]
unsafe fn haar_level_v<const N: usize, const H: usize>(
    src: &[__m256i],
    ll: &mut [__m256i],
    hl: &mut [__m256i],
    lh: &mut [__m256i],
    hh: &mut [__m256i],
) {
    let mut l = [_mm256_setzero_si256(); 64];
    let mut hb = [_mm256_setzero_si256(); 64];
    for r in 0..N {
        for i in 0..H {
            let a = src[r * N + 2 * i];
            let b = src[r * N + 2 * i + 1];
            l[r * H + i] = _mm256_add_epi32(a, b);
            hb[r * H + i] = _mm256_sub_epi32(a, b);
        }
    }
    for c in 0..H {
        for i in 0..H {
            let a = l[2 * i * H + c];
            let b = l[(2 * i + 1) * H + c];
            ll[i * H + c] = _mm256_add_epi32(a, b);
            lh[i * H + c] = _mm256_sub_epi32(a, b);
            let a2 = hb[2 * i * H + c];
            let b2 = hb[(2 * i + 1) * H + c];
            hl[i * H + c] = _mm256_add_epi32(a2, b2);
            hh[i * H + c] = _mm256_sub_epi32(a2, b2);
        }
    }
}

/// `blocks` is eight blocks; `out` receives their eight coefficient sets.
#[target_feature(enable = "avx2")]
unsafe fn transform_avx2(blocks: &[[i32; PIXELS]; LANES], out: &mut [[i32; COEFFS]; LANES]) {
    // Transpose to lanes-per-coefficient once; every stage below is then shuffle-free.
    let mut src = [_mm256_setzero_si256(); PIXELS];
    let mut tmp = [0i32; LANES];
    for (p, slot) in src.iter_mut().enumerate() {
        for (b, t) in tmp.iter_mut().enumerate() {
            *t = blocks[b][p];
        }
        *slot = _mm256_loadu_si256(tmp.as_ptr() as *const __m256i);
    }

    let z = _mm256_setzero_si256();
    let (mut ll1, mut hl1, mut lh1, mut hh1) = ([z; 16], [z; 16], [z; 16], [z; 16]);
    haar_level_v::<8, 4>(&src, &mut ll1, &mut hl1, &mut lh1, &mut hh1);
    let (mut ll2, mut hl2, mut lh2, mut hh2) = ([z; 4], [z; 4], [z; 4], [z; 4]);
    haar_level_v::<4, 2>(&ll1, &mut ll2, &mut hl2, &mut lh2, &mut hh2);
    let (mut ll3, mut hl3, mut lh3, mut hh3) = ([z; 1], [z; 1], [z; 1], [z; 1]);
    haar_level_v::<2, 1>(&ll2, &mut ll3, &mut hl3, &mut lh3, &mut hh3);

    // `>> 6` is an arithmetic shift in both paths; _mm256_srai_epi32 matches Rust's `>>` on i32.
    let mut store = |coeff: usize, v: __m256i, out: &mut [[i32; COEFFS]; LANES]| {
        let mut lanes = [0i32; LANES];
        _mm256_storeu_si256(lanes.as_mut_ptr() as *mut __m256i, _mm256_srai_epi32(v, 6));
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

// ---------------------------------------------------------------- harness

fn main() {
    if !is_x86_feature_detected!("avx2") {
        println!("no AVX2 on this CPU");
        return;
    }
    // Deterministic pseudo-random blocks spanning the full 8-bit input range the codec sees.
    let mut state = 0x2545_f491_4f6c_dd1du64;
    let mut rnd = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    const BATCHES: usize = 4096;
    let mut input = vec![[[0i32; PIXELS]; LANES]; BATCHES];
    for batch in input.iter_mut() {
        for block in batch.iter_mut() {
            for px in block.iter_mut() {
                *px = (rnd() % 256) as i32;
            }
        }
    }

    // 1. byte-exactness, before any timing
    let mut mismatches = 0usize;
    for batch in input.iter() {
        let mut got = [[0i32; COEFFS]; LANES];
        unsafe { transform_avx2(batch, &mut got) };
        for (b, block) in batch.iter().enumerate() {
            if transform_scalar(block) != got[b] {
                mismatches += 1;
            }
        }
    }
    let blocks = BATCHES * LANES;
    println!("== correctness");
    if mismatches == 0 {
        println!("   {blocks} blocks, AVX2 output identical to scalar");
    } else {
        println!("   {mismatches}/{blocks} MISMATCH -- speedup is meaningless, stopping");
        std::process::exit(1);
    }

    // 2. throughput
    let reps = 40;
    let mut sink = 0i64;
    let t0 = Instant::now();
    for _ in 0..reps {
        for batch in input.iter() {
            for block in batch.iter() {
                sink += transform_scalar(block)[0] as i64;
            }
        }
    }
    let scalar = t0.elapsed();

    let t1 = Instant::now();
    let mut got = [[0i32; COEFFS]; LANES];
    for _ in 0..reps {
        for batch in input.iter() {
            unsafe { transform_avx2(batch, &mut got) };
            sink += got[0][0] as i64;
        }
    }
    let simd = t1.elapsed();

    let total = (blocks * reps) as f64;
    println!("\n== throughput ({total:.0} blocks each)");
    println!(
        "   scalar {:>8.1} ms   {:>7.1} M blocks/s",
        scalar.as_secs_f64() * 1e3,
        total / scalar.as_secs_f64() / 1e6
    );
    println!(
        "   avx2   {:>8.1} ms   {:>7.1} M blocks/s",
        simd.as_secs_f64() * 1e3,
        total / simd.as_secs_f64() / 1e6
    );
    println!(
        "   speedup {:.2}x",
        scalar.as_secs_f64() / simd.as_secs_f64()
    );
    if sink == i64::MIN {
        println!("unreachable {sink}");
    }
}
