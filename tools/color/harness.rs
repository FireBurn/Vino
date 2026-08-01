//! Runs the drivers' real `color.rs` outside the kernel, so the colour arithmetic can actually be
//! executed rather than only compiled.
//!
//! `color-selftest.sh` copies `drivers/gpu/drm/vino/color.rs` next to this file and rewrites only
//! the `kernel::` paths; every line of arithmetic under test is the exact source compiled into
//! vino.ko and evdi.ko. The in-tree KUnit tests cover the same ground, but they need
//! CONFIG_KUNIT=y and a kernel built with it -- which meant they were silently not compiled at
//! all for a long time, and the maths went unrun. This needs nothing but rustc.
//!
//! It has already earned its place: it caught `narrow()` dividing by 256 where `expand()`
//! multiplies by 257, which made even an identity gamma ramp shift every pixel above ~128.
#![allow(dead_code)]
pub mod kernel {
    pub mod drm { pub mod kms { pub mod crtc {
        #[derive(Clone, Copy)]
        pub struct ColorLut { r: u16, g: u16, b: u16 }
        impl ColorLut {
            pub const fn new(red: u16, green: u16, blue: u16) -> Self { Self { r: red, g: green, b: blue } }
            pub fn red(&self) -> u16 { self.r }
            pub fn green(&self) -> u16 { self.g }
            pub fn blue(&self) -> u16 { self.b }
        }
        pub struct ColorCtm { m: [u64; 9] }
        impl ColorCtm {
            pub const fn from_raw(matrix: [u64; 9]) -> Self { Self { m: matrix } }
            // Byte-for-byte the kernel binding's decode.
            pub fn coefficient(&self, i: usize) -> Option<i64> {
                let raw = *self.m.get(i)?;
                let magnitude = (raw & !(1u64 << 63)) as i64;
                Some(if raw & (1u64 << 63) != 0 { -magnitude } else { magnitude })
            }
            pub fn coefficients(&self) -> [i64; 9] {
                let mut out = [0i64; 9];
                for (i, o) in out.iter_mut().enumerate() { *o = self.coefficient(i).unwrap_or(0); }
                out
            }
        }
    } } }
    pub mod xxhash {
        pub fn xxh64(data: &[u8], seed: u64) -> u64 {
            let mut h = seed ^ 0x9e3779b185ebca87;
            for &b in data { h = (h ^ b as u64).wrapping_mul(0x100000001b3); }
            h
        }
    }
}
use kernel::drm::kms::crtc::{ColorCtm, ColorLut};
include!("real_color.rs");
const ONE: u64 = 1 << 32;
const HALF: u64 = 1 << 31;                       // +0.5
const NEG_HALF: u64 = (1u64 << 63) | (1u64 << 31); // -0.5
fn diag(r: u64, g: u64, b: u64) -> ColorCtm { ColorCtm::from_raw([r,0,0, 0,g,0, 0,0,b]) }
fn half_lut() -> Vec<ColorLut> {
    // Round the fixture: entry 255 is 65535/2 = 32767.5, and truncating it to 32767 would
    // make the LUT itself ask for 127 rather than 128.
    (0..LUT_LEN).map(|i| { let h = ((i*257 + 1)/2) as u16; ColorLut::new(h,h,h) }).collect()
}

fn main() {
    let mut pass = 0; let mut fail = 0;
    let mut check = |ok: bool, what: &str| {
        if ok { pass += 1; println!("  PASS  {what}"); } else { fail += 1; println!("  FAIL  {what}"); }
    };

    // 1. sign-magnitude decode
    let m = diag(ONE, NEG_HALF, ONE);
    check(m.coefficient(0) == Some(1i64<<32) && m.coefficient(4) == Some(-(1i64<<31))
          && m.coefficient(9).is_none(), "CTM decodes sign-magnitude, not two's complement");

    // 2. identity collapses to None (keeps the direct-scanout fast path)
    check(ColorPipeline::build(None, None).is_none(), "no properties -> no pipeline");
    check(ColorPipeline::build(None, Some(&diag(ONE,ONE,ONE))).is_none(), "identity CTM -> no pipeline");

    // 3. gamma only
    let lut = half_lut();
    let p = ColorPipeline::build(Some(&lut), None).unwrap();
    check(p.apply(0,0,0) == (0,0,0) && p.apply(255,255,255) == (128,128,128), "gamma ramp applied (255 -> 128)");

    // 4. diagonal fast path == general matrix path
    let fast = ColorPipeline::build(None, Some(&diag(ONE, HALF, ONE))).unwrap();
    // Same transform, but with a real (not sub-Q16) off-diagonal zero-effect term so it must take
    // the mixing path: red gets 1.0*R + 0.0*G.
    let slow = ColorPipeline::build(None, Some(&ColorCtm::from_raw([ONE,0,ONE/65536, 0,HALF,0, 0,0,ONE]))).unwrap();
    let mut agree = true; let mut correct = true;
    for v in [0u8,1,63,127,128,200,254,255] {
        let f = fast.apply(v,v,v); let s = slow.apply(v,v,v);
        if f != s { agree = false; }
        if f != (v, ((v as u32 + 1)/2) as u8, v) { correct = false; eprintln!("   v={v} got {f:?} want {:?}", (v,(v as u32+1)/2,v)); }
    }
    check(agree, "fused diagonal path agrees with the general matrix path");
    check(correct, "diagonal CTM halves the green channel");
    check(fast.apply(255,255,255) == (255,128,255), "half gain: 255 -> 128");
    check(matches!(fast, ColorPipeline::Fused(_)), "diagonal CTM takes the Fused (fast) path");
    check(matches!(slow, ColorPipeline::Mixed{..}), "off-diagonal CTM takes the Mixed path");

    // 5. channel mixing / row-major order
    let swap = ColorCtm::from_raw([0,0,ONE, 0,ONE,0, ONE,0,0]);
    let p = ColorPipeline::build(None, Some(&swap)).unwrap();
    check(p.apply(200,100,50) == (50,100,200), "mixing CTM swaps R and B (row-major)");

    // 6. saturation
    let p = ColorPipeline::build(None, Some(&diag(4*ONE,4*ONE,4*ONE))).unwrap();
    check(p.apply(200,100,255) == (255,255,255) && p.apply(0,0,0) == (0,0,0),
          "out-of-gamut saturates instead of wrapping");

    // 7. short LUT extends with identity, not black
    let short: Vec<ColorLut> = (0..4).map(|i| { let v=(i*257) as u16; ColorLut::new(v,v,v) }).collect();
    let p = ColorPipeline::build(Some(&short), None).unwrap();
    check(p.apply(255,255,255) == (255,255,255), "short LUT extends with identity, not black");

    // 8. tag/eq invalidation
    let a = ColorPipeline::build(None, Some(&diag(ONE,NEG_HALF,ONE))).unwrap();
    let b = ColorPipeline::build(None, Some(&diag(NEG_HALF,ONE,ONE))).unwrap();
    check(a.tag() != b.tag() && a != b, "a transform change changes the strip-cache tag");

    // 9. negative coefficient really darkens (the bug the sign decode prevents)
    let p = ColorPipeline::build(None, Some(&diag(ONE, NEG_HALF, ONE))).unwrap();
    check(p.apply(255,255,255) == (255,0,255),
          "a NEGATIVE coefficient clamps to black (a two's-complement misread would saturate)");
    // The bug this whole exercise found: narrow(expand(v)) must be v, or an identity transform
    // shifts every pixel.
    let ident_lut: Vec<ColorLut> = (0..LUT_LEN).map(|i| { let v=(i*257) as u16; ColorLut::new(v,v,v) }).collect();
    let p = ColorPipeline::build(Some(&ident_lut), None).unwrap();
    let mut roundtrip = true;
    for v in 0..=255u8 { if p.apply(v,v,v) != (v,v,v) { roundtrip = false; } }
    check(roundtrip, "an identity gamma ramp is a no-op for all 256 input values");

    println!("\n== {pass} pass, {fail} fail");
    std::process::exit(if fail > 0 { 1 } else { 0 });
}
#[allow(dead_code)]
fn debug_values() {
    let lut = half_lut();
    let p = ColorPipeline::build(Some(&lut), None).unwrap();
    println!("[dbg] half-LUT: 0->{:?} 255->{:?}", p.apply(0,0,0), p.apply(255,255,255));
    let f = ColorPipeline::build(None, Some(&diag(ONE, HALF, ONE))).unwrap();
    println!("[dbg] half-gain: {:?}", [0u8,1,2,127,128,254,255].iter().map(|&v| f.apply(v,v,v).1).collect::<Vec<_>>());
    println!("[dbg] HALF coeff={:?}", diag(ONE,HALF,ONE).coefficient(4));
}
