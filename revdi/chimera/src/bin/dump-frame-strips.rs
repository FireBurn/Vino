//! Encode a real captured framebuffer with the LITERAL in-kernel frame assembler and dump the
//! resulting EP08 record stream, so every strip can be structurally decoded offline.
//!
//! The point is to check vino's own output for self-consistency on *real desktop content* --
//! the RE corpus was solids, ramps and noise, and byte-exactness was only ever proven against
//! those. A strip whose DC plane overruns its own `w18`/`w1c` section offsets is malformed, and
//! the dock would decode garbage from it while vino's per-strip hash happily records the strip
//! as delivered.
//!
//! Run: `cargo run --release --bin dump-frame-strips -- <rgb.bin> <w> <h> <out.bin>`

use vino_chimera::kvino;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 5 {
        eprintln!("usage: dump-frame-strips <rgb.bin> <w> <h> <out.bin>");
        return;
    }
    let (path, w, h, out) = (
        args[1].clone(),
        args[2].parse::<usize>().unwrap(),
        args[3].parse::<usize>().unwrap(),
        args[4].clone(),
    );
    let rgb = std::fs::read(&path).expect("read rgb");
    assert_eq!(rgb.len(), w * h * 3, "unexpected image size");

    let (frames, _seq) = kvino::colour_frame_ep08(w, h, &rgb, 0).expect("encode");
    let total: usize = frames.iter().map(|f| f.len()).sum();
    println!("encoded {w}x{h}: {} chunk(s), {total} bytes", frames.len());

    let mut blob = Vec::with_capacity(total);
    for f in &frames {
        blob.extend_from_slice(f);
    }
    std::fs::write(&out, &blob).expect("write");
    println!("wrote {out} ({} bytes)", blob.len());
}
