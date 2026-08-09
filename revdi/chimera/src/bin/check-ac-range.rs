//! Does the AC escape coder's `AC_CMAX = 9` ceiling actually bound real desktop content?
//!
//! `Bits::esc` SATURATES any coefficient whose magnitude category exceeds `cmax`, on the stated
//! grounds that "the recovered grammar does not otherwise exercise" the out-of-range case. The RE
//! corpus was solid colours, smooth ramps and full-spectrum noise -- it contained no sharp,
//! high-contrast UI edge. This runs the LITERAL kernel transform over real captured desktop pixels
//! (and over a synthetic edge at every sub-block phase) and reports the quantized AC magnitudes, so
//! "it never fires" is a measurement rather than an assumption.
//!
//! Run: `cargo run --bin check-ac-range -- <rgb.bin> <w> <h>`

use vino_chimera::kvino;

const AC_CMAX: u32 = 9;

fn mag_category(v: i32) -> u32 {
    if v == 0 {
        0
    } else {
        32 - v.unsigned_abs().leading_zeros()
    }
}

/// `video::wht::step_bias` + `quantize`, for the luma plane.
fn quantize_luma(coeff: i32, i: usize) -> i32 {
    let (step, bias) = match i {
        0 | 1 | 2 => (16, 8),
        3 => (32, 16),
        4..=11 => (4, 2),
        12..=15 => (8, 4),
        16..=47 => (2, 0),
        _ => (4, 2),
    };
    if bias == 0 {
        let q = coeff.abs() / step;
        if coeff < 0 {
            -q
        } else {
            q
        }
    } else {
        (coeff + step / 2).div_euclid(step)
    }
}

/// `video::wht::quantize_chroma_ac`.
fn quantize_chroma_ac(coeff: i32, i: usize) -> i32 {
    let step = if matches!(i, 1 | 2 | 4..=11) {
        16
    } else if i >= 48 {
        64
    } else {
        32
    };
    (coeff + step / 2).div_euclid(step)
}

struct Stats {
    blocks: usize,
    over: usize,
    worst: i32,
    worst_pos: usize,
    worst_plane: &'static str,
    over_blocks: usize,
}

impl Stats {
    fn new() -> Self {
        Stats {
            blocks: 0,
            over: 0,
            worst: 0,
            worst_pos: 0,
            worst_plane: "-",
            over_blocks: 0,
        }
    }
}

/// Transform + quantize one 8x8 block of RGB and count coefficients past `AC_CMAX`.
fn scan_block(px: &dyn Fn(usize, usize) -> (u8, u8, u8), bx: usize, by: usize, st: &mut Stats) {
    let mut cr = [0i32; 64];
    let mut cb = [0i32; 64];
    let mut y = [0i32; 64];
    for j in 0..8 {
        for i in 0..8 {
            let (r, g, b) = px(bx + i, by + j);
            let (vy, vcb, vcr) = kvino::colour(r.into(), g.into(), b.into());
            y[j * 8 + i] = vy;
            cb[j * 8 + i] = vcb;
            cr[j * 8 + i] = vcr;
        }
    }
    st.blocks += 1;
    let mut hit = false;
    for (name, plane, chroma) in [("Y", &y, false), ("Cb", &cb, true), ("Cr", &cr, true)] {
        let t = kvino::transform_raw(plane);
        for i in 1..64 {
            let q = if chroma {
                quantize_chroma_ac(t[i], i)
            } else {
                quantize_luma(t[i], i)
            };
            if mag_category(q) > AC_CMAX {
                st.over += 1;
                hit = true;
                if q.abs() > st.worst.abs() {
                    st.worst = q;
                    st.worst_pos = i;
                    st.worst_plane = name;
                }
            }
        }
    }
    if hit {
        st.over_blocks += 1;
    }
}

fn report(label: &str, st: &Stats) {
    let max_encodable = (1i32 << AC_CMAX) - 1;
    println!(
        "{label:<34} blocks={:<6} blocks_with_overflow={:<6} coeffs_over_cmax={:<6} worst={} \
         (plane {}, pos {}) -> encoded as {}",
        st.blocks,
        st.over_blocks,
        st.over,
        st.worst,
        st.worst_plane,
        st.worst_pos,
        if st.worst < 0 {
            -max_encodable
        } else {
            max_encodable
        }
    );
}

fn main() {
    // 1. A synthetic hard edge (the icon's two real colours) at every phase within an 8x8 block.
    let bg = (20u8, 22u8, 24u8);
    let fg = (61u8, 174u8, 233u8);
    for phase in 0..8 {
        let mut st = Stats::new();
        let px = move |x: usize, _y: usize| if x % 8 >= phase { fg } else { bg };
        scan_block(&px, 0, 0, &mut st);
        report(&format!("synthetic vertical edge phase {phase}"), &st);
    }

    // 2. The real captured pixels.
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 4 {
        println!("\n(no image given; pass <rgb.bin> <w> <h> for the real-content scan)");
        return;
    }
    let (path, w, h) = (
        args[1].clone(),
        args[2].parse::<usize>().unwrap(),
        args[3].parse::<usize>().unwrap(),
    );
    let rgb = std::fs::read(&path).expect("read rgb");
    assert_eq!(rgb.len(), w * h * 3, "unexpected image size");
    let px = |x: usize, y: usize| {
        let i = (y.min(h - 1) * w + x.min(w - 1)) * 3;
        (rgb[i], rgb[i + 1], rgb[i + 2])
    };
    let mut st = Stats::new();
    for by in (0..h / 8 * 8).step_by(8) {
        for bx in (0..w / 8 * 8).step_by(8) {
            scan_block(&px, bx, by, &mut st);
        }
    }
    println!();
    report(&format!("real capture {w}x{h}"), &st);
}
