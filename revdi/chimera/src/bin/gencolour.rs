//! gencolour — read colour-strip records from stdin and emit each strip's bytes via the
//! LITERAL kernel `video::wht::colour_strip`. Used by the chroma-AC RE harness to prove the
//! in-kernel colour codec byte-exact against real DLM sink strips.
//!
//! Each record (whitespace ints): `X Y` then 16 blocks × 3 planes × 64 samples = 3074 ints.
//! Planes are [Cr=64*(B-G), Cb=64*(R-G), Y=64*G+64*((Cb+Cr)>>2)]. Prints one hex strip per line.
use std::io::Read;

fn main() {
    let mut s = String::new();
    std::io::stdin().read_to_string(&mut s).unwrap();
    let v: Vec<i32> = s.split_whitespace().map(|t| t.parse().unwrap()).collect();
    const REC: usize = 2 + 16 * 3 * 64;
    assert_eq!(
        v.len() % REC,
        0,
        "ragged input: {} not a multiple of {REC}",
        v.len()
    );
    let mut out = String::new();
    for rec in v.chunks(REC) {
        let x = rec[0] as u16;
        let y = rec[1] as u16;
        let mut planes = [[[0i32; 64]; 3]; 16];
        let mut idx = 2;
        for k in 0..16 {
            for p in 0..3 {
                for i in 0..64 {
                    planes[k][p][i] = rec[idx];
                    idx += 1;
                }
            }
        }
        let strip =
            vino_chimera::kvino::colour_strip_from_planes(&planes, x, y).expect("colour_strip");
        out.push_str(&strip.iter().map(|b| format!("{b:02x}")).collect::<String>());
        out.push('\n');
    }
    print!("{out}");
}
