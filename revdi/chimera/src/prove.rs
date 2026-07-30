//! Offline byte-exact proof: replay a real DLM session through the *literal
//! kernel* CP code ([`crate::kvino`]) and assert equivalence.
//!
//! Two independent proofs, both driven entirely by the included `cp.rs`:
//!
//! 1. **Seal equivalence** — for every OUT CP frame DLM put on the wire, recover
//!    the inner plaintext (AES-CTR is symmetric, via `cp::open_in`), re-seal it
//!    with `cp::seal_interactive`/`cp::seal_stream`, and assert the regenerated
//!    wire frame is byte-for-byte what DLM sent. This proves the AES-CTR
//!    keystream, the Dl3Cmac and the wire framing.
//!
//! 2. **Builder equivalence** — for the frames whose inner message the kernel can
//!    build from scratch (heartbeat, cursor create/move/image), regenerate the
//!    plaintext from the fields DLM used and assert it matches DLM's recovered
//!    plaintext. This proves the post-msg0 message *content*, not just the crypto.
//!
//! The fixtures (a candidate (ks,riv) list and the captured OUT CP frames) are
//! embedded from `fixtures/`, extracted once from
//! `captures/cold-ref-20260608-200850/` — so the proof runs with no pcap, no
//! tshark and no hardware.

use crate::kvino;

const CANDS: &str = include_str!("../fixtures/coldref-cands.txt");
const FRAMES: &str = include_str!("../fixtures/coldref-out-cp.hex");
/// Real DLM colour sink strips (cramp256/bramp256/rramp256) + their per-plane samples, for the
/// colour-AC proof. Triples of `# name` / `X Y <16*3*64 ints>` / `<expected hex minus tail>`.
const COLOUR_STRIPS: &str = include_str!("../fixtures/colour-strips.txt");

/// CP `sub` ids seen on the wire — a recovered plaintext whose `sub` is one of
/// these (and whose post-counter pad word is zero) is the right (ks,riv).
/// Mirrors `cp::is_known_sub` (which the kernel keeps module-private).
const KNOWN_SUB: &[u16] = &[
    0x00, 0x04, 0x0c, 0x10, 0x20, 0x21, 0x22, 0x24, 0x25, 0x30, 0x41, 0x42, 0x43, 0x45, 0x75, 0x84,
];

fn unhex(s: &str) -> Vec<u8> {
    let s = s.trim();
    (0..s.len() / 2)
        .map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap())
        .collect()
}

fn le16(b: &[u8], off: usize) -> u16 {
    u16::from_le_bytes([b[off], b[off + 1]])
}
fn le32(b: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]])
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

type Cand = ([u8; 16], [u8; 8]);

fn load_cands() -> Vec<Cand> {
    CANDS
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            let mut it = l.split_whitespace();
            let k = unhex(it.next().unwrap());
            let r = unhex(it.next().unwrap());
            let mut ks = [0u8; 16];
            ks.copy_from_slice(&k);
            let mut riv = [0u8; 8];
            riv.copy_from_slice(&r);
            (ks, riv)
        })
        .collect()
}

fn load_frames() -> Vec<Vec<u8>> {
    FRAMES
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(unhex)
        .collect()
}

/// Recover the inner plaintext DLM would have been handed. `livemac` bodies are
/// `ct(content) || tag16`; `stream` bodies are `ct(whole inner)`. AES-CTR is
/// symmetric so `cp::open_in` (the kernel decrypt) recovers it.
fn recover(ks: &[u8; 16], riv: &[u8; 8], seq: u32, body: &[u8], livemac: bool) -> Option<Vec<u8>> {
    let ct = if livemac {
        if body.len() < 16 {
            return None;
        }
        &body[..body.len() - 16]
    } else {
        body
    };
    kvino::open_in(ks, riv, seq, ct).ok()
}

/// Find the (ks,riv) whose decrypt yields a sane inner header (known sub, zero
/// pad word). Returns the chosen candidate and the recovered plaintext.
fn pick(cands: &[Cand], seq: u32, body: &[u8], livemac: bool) -> Option<(Cand, Vec<u8>)> {
    for c in cands {
        if let Some(pt) = recover(&c.0, &c.1, seq, body, livemac) {
            if pt.len() >= 8 && KNOWN_SUB.contains(&le16(&pt, 2)) && le16(&pt, 6) == 0 {
                return Some((*c, pt));
            }
        }
    }
    None
}

/// Result of the proof run, so a binary can set its exit code from it.
pub struct Report {
    pub seal_ok: usize,
    pub seal_total: usize,
    pub seal_skipped: usize,
    pub builder_exact: usize,
    pub builder_documented: usize,
    pub builder_total: usize,
    pub mismatches: usize,
    pub video_ok: bool,
}

impl Report {
    pub fn passed(&self) -> bool {
        self.mismatches == 0
            && self.seal_ok == self.seal_total
            && self.seal_total > 0
            && self.builder_documented == self.builder_total
            && self.video_ok
    }
}

/// Run both proofs, printing a transcript, and return the [`Report`].
/// Sixteen identical flat 8x8 blocks of one RGB colour, in the codec's x64 fixed point --
/// the input `colour_strip` needs to emit a uniform 64x16 strip.
fn flat_planes(r: u8, g: u8, b: u8) -> [[[i32; 64]; 3]; 16] {
    let (y, cb, cr) = crate::kvino::colour(r, g, b);
    [[[cr; 64], [cb; 64], [y; 64]]; 16]
}

pub fn run() -> Report {
    let cands = load_cands();
    let frames = load_frames();
    println!(
        "vino-chimera — proving the LITERAL kernel cp.rs/proto.rs against a real DLM session\n\
         source: captures/cold-ref-20260608-200850 (D6000 cold plug, DLM engaged)\n\
         {} (ks,riv) candidates · {} captured OUT type=4 CP frames\n",
        cands.len(),
        frames.len()
    );

    // ---- proof 1: seal equivalence -----------------------------------------
    println!("== Proof 1: kernel seal reproduces DLM's wire byte-for-byte ==");
    let mut seal_ok = 0usize;
    let mut seal_total = 0usize;
    let mut skipped = 0usize;
    let mut mismatches = 0usize;
    // Stash recovered (cand, content) for matched frames so proof 2 can reuse it.
    let mut recovered: Vec<(Cand, Vec<u8>)> = Vec::new();

    for wire in &frames {
        if wire.len() < 32 {
            continue;
        }
        let wsub = le16(wire, 8);
        let seq = le32(wire, 12);
        let body = &wire[16..];
        let livemac = wsub == 0x24;
        let Some((cand, content)) = pick(&cands, seq, body, livemac) else {
            skipped += 1;
            continue;
        };
        let (ks, riv) = cand;
        let regen = if livemac {
            // The driver's `send_cp` passes the message *type* id to `seal_interactive`
            // (which keys `aux_for_id`), not the on-wire inner id. They differ for the
            // cursor image, whose inner id carries a 0x4000 bitmap flag (0x401c) while
            // the type id is 0x1c — so strip the flag to mirror the real driver.
            let inner_id = le16(&content, 0) & !0x4000;
            kvino::seal_interactive(&ks, &riv, inner_id, seq, &content)
        } else {
            kvino::seal_stream(&ks, &riv, wsub, seq, &content)
        }
        .expect("seal");

        seal_total += 1;
        if regen == *wire {
            seal_ok += 1;
        } else {
            mismatches += 1;
            let id = le16(&content, 0);
            let sub = le16(&content, 2);
            println!(
                "  *** MISMATCH wsub={wsub:#06x} seq={seq:#06x} id={id:#06x} sub={sub:#04x} ***"
            );
            for off in (0..wire.len().max(regen.len())).step_by(16) {
                let a = hex(&wire[off..(off + 16).min(wire.len())]);
                let b = hex(&regen[off..(off + 16).min(regen.len())]);
                if a != b {
                    println!("      @{off:3} dlm ={a}");
                    println!("           vino={b}");
                }
            }
        }
        recovered.push((cand, content));
    }
    println!(
        "  {seal_ok}/{seal_total} OUT CP frames regenerate BYTE-IDENTICAL  \
         (skipped {skipped} undecryptable/non-CP)"
    );
    if seal_total > 0 && seal_ok == seal_total {
        println!("  [PASS] kernel cp.rs AES-CTR + Dl3Cmac + wire framing == DLM, this session.");
    } else {
        println!("  [FAIL] seal divergence — see MISMATCH lines above.");
    }

    // ---- proof 2: builder equivalence --------------------------------------
    println!("\n== Proof 2: kernel CP builders reproduce DLM's inner plaintext ==");
    // Per-family tally: (exact, documented-region-masked, total).
    let mut hb = (0usize, 0usize, 0usize);
    let mut legacy_hb = 0usize;
    let mut cur = (0usize, 0usize, 0usize);

    for (_, content) in &recovered {
        if content.len() < 8 {
            continue;
        }
        let id = le16(content, 0);
        let sub = le16(content, 2);
        let ctr = le16(content, 4);

        // Heartbeat: id=0x16 sub=0x75, 32-byte inner. The kernel builder zeroes
        // the trailing 8 bytes (off24..32); DLM leaks an unspecified value there,
        // so an off24..32 mask is the documented region (the dock can't validate
        // bytes that vary per frame).
        if id == 0x16 && sub == 0x75 && content.len() == 32 {
            // This fixture predates the live-confirmed 0x2ee0 heartbeat marker and carries the
            // older 0x2710 value. Do not count those frames as a current-builder oracle.
            if content[22..24] != [0xe0, 0x2e] {
                legacy_hb += 1;
                continue;
            }
            let mine = kvino::heartbeat(ctr).expect("heartbeat");
            hb.2 += 1;
            if mine == *content {
                hb.0 += 1;
                hb.1 += 1;
            } else {
                let mut a = mine.clone();
                let mut b = content.clone();
                for x in &mut a[24..32] {
                    *x = 0;
                }
                for x in &mut b[24..32] {
                    *x = 0;
                }
                if a == b {
                    hb.1 += 1;
                }
            }
            continue;
        }

        // Cursor create/move/image. Layout recovered byte-exact by the cp-seal
        // differential; create/move leak uninitialised padding at off28..31.
        if matches!(sub, 0x41 | 0x42 | 0x43) {
            // off22 carries the dock's head id, counted from ONE; off23 is the cursor's visible
            // flag, set on move and clear on the bitmap-bearing messages. Reading off23 as the head
            // (as this did) happened to round-trip only because the flag and head id share values.
            let head = content[22].saturating_sub(1);
            let visible = content[23] != 0;
            let f1 = le16(content, 24);
            let f2 = le16(content, 26);
            let mine = match sub {
                0x42 => kvino::cursor_create(ctr, head, f1, f2),
                0x43 => kvino::cursor_move(ctr, head, f1, f2, visible),
                _ => {
                    // image: off32..33 are zero and the bitmap starts at off34, with the final
                    // pixel truncated by two bytes so the message stays 32 + w*h*4. Rebuild the
                    // builder's input by restoring those two bytes; feeding from off32 shifts the
                    // whole bitmap and only diverges at the first non-transparent pixel.
                    let mut bgra = content[34..].to_vec();
                    bgra.extend_from_slice(&[0, 0]);
                    let w = (bgra.len() / 4) as u16;
                    kvino::cursor_image(ctr, head, w, 1, &bgra)
                }
            }
            .expect("cursor");
            cur.2 += 1;
            if mine == *content {
                cur.0 += 1;
                cur.1 += 1;
            } else if matches!(sub, 0x42 | 0x43) {
                let mut a = mine.clone();
                let mut b = content.clone();
                for x in &mut a[28..32] {
                    *x = 0;
                }
                for x in &mut b[28..32] {
                    *x = 0;
                }
                if a == b {
                    cur.1 += 1;
                }
            }
        }
    }

    if hb.2 > 0 {
        println!(
            "  heartbeat  (id=0x16/0x75): {}/{} exact, {}/{} match modulo DLM's unspecified \
             off24..32 trailer",
            hb.0, hb.2, hb.1, hb.2
        );
    }
    if legacy_hb > 0 {
        println!(
            "  heartbeat: skipped {legacy_hb} legacy 0x2710-marker fixture frame(s); current \
             captures and the kernel use 0x2ee0"
        );
    }
    if cur.2 > 0 {
        println!(
            "  cursor     (sub 0x41/42/43): {}/{} exact, {}/{} match modulo DLM's uninitialised \
             off28..31 padding",
            cur.0, cur.2, cur.1, cur.2
        );
    }
    if hb.2 == 0 && cur.2 == 0 {
        println!("  (no builder-reproducible frames in this capture window)");
    }

    // ---- proof 3: video codec (solid colour) -------------------------------
    // The kernel `video::wht::solid_strip` is KUnit-verified byte-exact vs DLM
    // (14/14 colours; 3508/3508 strips of the grey128 capture). Here the chimera
    // runs that literal code and must reproduce the known-good grey128 reference
    // strip (X=Y=0), proving the codec head of the rig is the kernel's bytes.
    println!("\n== Proof 3: kernel colour codec reproduces DLM's uniform strip ==");
    // grey128 = colour(128,128,128) -> DC (8192,0,0); reference strip from
    // scripts/wht-strip-encoder.py / KUnit wht_solid_strip_byte_exact.
    const GREY128_REF: &str = "012800000000000000003a003a000000fc007e003f801fc00fe007f003f80\
                               1fc007e003f801fc00fe007f003f801fc0f200000000000000000003a00";
    // Emitted by the LIVE encoder (`colour_strip`), not the retired `solid_strip`: for a uniform
    // strip the two produce the same body. DLM's "constant 30-byte main frame" is exactly the 16
    // flat blocks' 15-bit significance codes (16 x 15 = 240 bits = 30 bytes) followed by the DC
    // escapes, which is what `colour_strip` builds when every block is flat. Proving it through
    // the path that actually runs is strictly better than proving it through one that does not.
    let mine = crate::kvino::colour_strip_from_planes(&flat_planes(128, 128, 128), 0, 0)
        .expect("colour_strip");
    // Compare excluding the 2-byte tail: the reference is a sink-hook capture that carries the
    // `L-2` echo there, but on the EP08 wire those bytes are the natural row1 padding (the record
    // framing's strip_id carries the length). Same tail-excluded comparison as Proof 4.
    // The reference is a sink-hook capture and carries the `L-2` echo in its last two bytes; the
    // EP08 wire does not (the record framing's `strip_id` carries the length), so `colour_strip`
    // emits a strip exactly two bytes shorter. Compare the whole of ours against the reference
    // minus that echo -- the earlier arithmetic also trimmed two bytes from OURS, which silently
    // compared 56 bytes against 58 and could only ever fail.
    let ref_clean = GREY128_REF.replace([' ', '\n'], "");
    let video_ok = hex(&mine) == ref_clean[..ref_clean.len() - 4];
    if video_ok {
        println!(
            "  grey128 strip ({}B) byte-identical to DLM reference (tail aside).",
            mine.len()
        );
    } else {
        println!(
            "  *** grey128 strip MISMATCH ***\n    dlm ={GREY128_REF}\n    vino={}",
            hex(&mine)
        );
    }
    // Colour transform DC vs DLM's encoder-IO ground truth (the 9 measured
    // colours from validate-transform-encoderio.py): (R,G,B) -> (Y,Cb,Cr) DC.
    const COLOUR_GT: &[((u8, u8, u8), (i32, i32, i32))] = &[
        ((0, 0, 0), (0, 0, 0)),
        ((128, 128, 128), (8192, 0, 0)),
        ((255, 255, 255), (16320, 0, 0)),
        ((255, 0, 0), (4032, 16320, 0)),
        ((0, 255, 0), (8128, -16320, -16320)),
        ((0, 0, 255), (4032, 0, 16320)),
        ((0, 255, 255), (12224, -16320, 0)),
        ((255, 0, 255), (8128, 16320, 16320)),
        ((255, 255, 0), (12224, 0, -16320)),
    ];
    let mut col_ok = 0usize;
    for &((r, g, b), gt) in COLOUR_GT {
        if crate::kvino::colour(r, g, b) == gt {
            col_ok += 1;
        } else {
            println!(
                "  *** colour({r},{g},{b}) = {:?}, DLM = {gt:?} ***",
                crate::kvino::colour(r, g, b)
            );
        }
    }
    let colour_ok = col_ok == COLOUR_GT.len();
    println!(
        "  colour transform DC: {col_ok}/{} match DLM encoder-IO ground truth",
        COLOUR_GT.len()
    );

    // Chroma-DC SOLID strip vs a REAL DLM colour strip (a colour bar from
    // captures/codebook-paired-20260622-211813/bars.bin, @X=512 Y=16, 62 B). The
    // kernel solid_strip builds SOLID_MAIN + the (Cr,Cb,Y) DC escapes; feeding it
    // one primary's colour() DC must reproduce DLM's bytes exactly. This proves the
    // chroma-DC escape coding (nonzero Cb/Cr) against live DLM colour output.
    const BARS_STRIP_X512_Y16: &str = "012800021000000000003c003c000000fc007e003f801fc00fe007f0\
                                       03f801fc007e003f801fc00fe007f003f801fffefcef27000000000000\
                                       0000003c00";
    let target = BARS_STRIP_X512_Y16.replace([' ', '\n'], "");
    let primaries: &[(&str, (u8, u8, u8))] = &[
        ("red", (255, 0, 0)),
        ("green", (0, 255, 0)),
        ("blue", (0, 0, 255)),
        ("cyan", (0, 255, 255)),
        ("magenta", (255, 0, 255)),
        ("yellow", (255, 255, 0)),
        ("white", (255, 255, 255)),
    ];
    let target_body = &target[..target.len() - 4]; // exclude the sink echo (tail), as above
    let mut chroma_match: Option<&str> = None;
    for &(name, (r, g, b)) in primaries {
        if let Ok(s) = crate::kvino::colour_strip_from_planes(&flat_planes(r, g, b), 512, 16) {
            if hex(&s) == target_body {
                chroma_match = Some(name);
                break;
            }
        }
    }
    let chroma_ok = chroma_match.is_some();
    match chroma_match {
        Some(name) => println!(
            "  chroma-DC solid strip: kernel reproduces DLM's bars {name} strip @X512 BYTE-EXACT"
        ),
        None => {
            println!("  *** chroma-DC solid strip: NO primary reproduces the DLM bars strip ***")
        }
    }

    // ---- proof 4: colour AC (chroma Cb/Cr) vs REAL DLM sink strips ----------
    // The kernel `video::wht::colour_strip` (chroma sync fields + (Cr,Cb,Y) DC/AC + col-major
    // L1HL + chroma quant) reproduces real DLM colour strips byte-for-byte. The fixture holds
    // three sink-captured strips (captures/codec-sink-sweep-*) with both chroma planes
    // (cramp256), Cr-only (bramp256) and Cb-only (rramp256), each as its 16x3x64 planes + the
    // expected bytes (minus the 2-byte forward-hint tail). Recovered/verified 2026-06-27/28;
    // full proof (2700/2872 strips) in scripts/codec-re/verify-colour-ac.py.
    println!("\n== Proof 4: kernel colour codec reproduces DLM's chroma-AC strips ==");
    let mut colour_ac_ok = 0usize;
    let mut colour_ac_total = 0usize;
    let mut lines = COLOUR_STRIPS.lines();
    while let Some(hdr) = lines.next() {
        if !hdr.starts_with('#') {
            continue;
        }
        let name = hdr.trim_start_matches("# ").to_string();
        let data = lines.next().expect("planes line");
        let want = lines.next().expect("expected hex line");
        let nums: Vec<i32> = data
            .split_whitespace()
            .map(|t| t.parse().unwrap())
            .collect();
        let x = nums[0] as u16;
        let y = nums[1] as u16;
        let mut planes = [[[0i32; 64]; 3]; 16];
        let mut idx = 2;
        for k in 0..16 {
            for p in 0..3 {
                for i in 0..64 {
                    planes[k][p][i] = nums[idx];
                    idx += 1;
                }
            }
        }
        let strip = crate::kvino::colour_strip_from_planes(&planes, x, y).expect("colour_strip");
        // The fixture's expected hex is the strip minus its 2-byte tail (a forward-length hint
        // the frame assembler overwrites); compare the body.
        let mine = hex(&strip[..strip.len() - 2]);
        colour_ac_total += 1;
        if mine == want {
            colour_ac_ok += 1;
        } else {
            println!("  *** colour strip {name} MISMATCH ***");
        }
    }
    let colour_ac_pass = colour_ac_total > 0 && colour_ac_ok == colour_ac_total;
    println!(
        "  colour-AC strips: {colour_ac_ok}/{colour_ac_total} reproduce DLM byte-exact (Cr+Cb, Cr-only, Cb-only)"
    );

    // ---- proof 5: colour FRAME assembler -> DLM's EP08 wire RECORD framing ----------------------
    // `video::wht::colour_frame_ep08` must tile the surface into 64x16 strips and frame them the way
    // DLM puts them on the EP08 wire (RE'd 2026-06-28, captures/ep08-framing-20260628/): a `type=4`
    // TLV record per single-Y band of strips -- `pad=0, size, type=4`, an 8-byte record prefix
    // (`flag, 0, fseq, 0, 0,0,0,0`), then per strip `strip_id(2) == strip length ++ strip bytes`,
    // a 4-byte trailer, and a 4-byte inter-record gap (`stride = size + 4`); the record stream is
    // chunked into <= 64 KB transfers. We feed a colourful gradient, reconstruct each strip
    // independently (the Proof-4-verified path), then walk the concatenated transfers and assert the
    // record framing is exactly this and every strip equals its standalone bytes (tail included --
    // there is no forward-hint tail on the wire). (Structural: each strip's bytes are proven vs DLM
    // in Proof 4; this proves the assembler wraps them in DLM's wire framing.)
    println!("\n== Proof 5: kernel colour FRAME assembler -> DLM EP08 record framing ==");
    let (fw, fh) = (640usize, 1056usize); // 10 x 66 = 660 strips, 66 single-Y record bands
    let mut rgb = vec![0u8; fw * fh * 3];
    for y in 0..fh {
        for x in 0..fw {
            let i = (y * fw + x) * 3;
            rgb[i] = ((x * 7) & 0xff) as u8;
            rgb[i + 1] = ((y * 5) & 0xff) as u8;
            rgb[i + 2] = (((x + y) * 3) & 0xff) as u8;
        }
    }
    let (frames, _next_seq) =
        crate::kvino::colour_frame_ep08(fw, fh, &rgb, 0).expect("colour_frame_ep08");
    let (sx, sy) = (fw / 64, fh / 16);
    let nstrips = sx * sy;
    // Rebuild each strip independently from the same pixels (the Proof-4-verified path).
    let mut standalone: Vec<Vec<u8>> = Vec::with_capacity(nstrips);
    for ry in 0..sy {
        for rx in 0..sx {
            let (ox, oy) = (rx * 64, ry * 16);
            let mut planes = [[[0i32; 64]; 3]; 16];
            for k in 0..16 {
                let (bx, by) = (k % 8, k / 8);
                for r in 0..8 {
                    for c in 0..8 {
                        let (px, py) = (ox + bx * 8 + c, oy + by * 8 + r);
                        let idx = (py * fw + px) * 3;
                        let (yv, cbv, crv) =
                            crate::kvino::colour(rgb[idx], rgb[idx + 1], rgb[idx + 2]);
                        let i = r * 8 + c;
                        planes[k][0][i] = crv;
                        planes[k][1][i] = cbv;
                        planes[k][2][i] = yv;
                    }
                }
            }
            standalone
                .push(crate::kvino::colour_strip_blocks(&planes, ox as u16, oy as u16).unwrap());
        }
    }
    let mut frame_ok = true;
    // No per-transfer header now: concatenating the transfers yields DLM's continuous record stream.
    for f in &frames {
        if f.len() > 65536 {
            frame_ok = false;
            println!("  *** transfer exceeds 64 KB ({}) ***", f.len());
        }
    }
    let stream: Vec<u8> = frames.iter().flat_map(|f| f.iter().copied()).collect();
    // Walk records, checking every strip against the standalone bytes in order.
    let mut off = 0usize;
    let mut k = 0usize;
    let mut nrecords = 0usize;
    while off + 16 <= stream.len() && frame_ok {
        let pad = u16::from_le_bytes([stream[off], stream[off + 1]]);
        let size = u16::from_le_bytes([stream[off + 2], stream[off + 3]]) as usize;
        let ty = u32::from_le_bytes([
            stream[off + 4],
            stream[off + 5],
            stream[off + 6],
            stream[off + 7],
        ]);
        // The 8-byte prefix is `sub(2) ++ aux(2) ++ fseq(4)`, not the zero block this proof used
        // to assert -- `frame_records` has carried `sub = head | (((y/16) & 1) << 4)` and `aux` =
        // the record's own 16-byte-alignment padding since the 2026-07-22 framing fix. Checking for
        // zeroes there made this proof fail on every correctly-framed record.
        let sub = u16::from_le_bytes([stream[off + 8], stream[off + 9]]);
        let aux = u16::from_le_bytes([stream[off + 10], stream[off + 11]]) as usize;
        let fseq = u32::from_le_bytes([
            stream[off + 12],
            stream[off + 13],
            stream[off + 14],
            stream[off + 15],
        ]);
        if pad != 0 || ty != 4 || off + size + 4 > stream.len() || fseq != 0 || (size + 4) % 16 != 0
        {
            frame_ok = false;
            println!(
                "  *** record at {off} malformed (pad={pad} type={ty} size={size} fseq={fseq}) ***"
            );
            break;
        }
        // Strips: strip_id(2)==len ++ strip; all sharing one Y; until the 4-byte trailer.
        let mut pos = off + 16;
        let band_end = off + 4 + size - aux;
        let mut band_y: Option<u16> = None;
        while pos < band_end {
            if k >= nstrips {
                frame_ok = false;
                break;
            }
            let sid = u16::from_le_bytes([stream[pos], stream[pos + 1]]) as usize;
            let want = &standalone[k];
            let y = u16::from_le_bytes([want[4], want[5]]);
            if sid != want.len()
                || pos + 2 + sid > band_end
                || &stream[pos + 2..pos + 2 + sid] != want.as_slice()
                || *band_y.get_or_insert(y) != y
            {
                frame_ok = false;
                println!("  *** strip {k} (record {nrecords}) strip_id/bytes/Y mismatch ***");
                break;
            }
            // `sub` selects the head and carries the band's parity bit.
            if sub != (((y / 16) & 1) << 4) {
                frame_ok = false;
                println!(
                    "  *** record {nrecords} sub={sub:#x} does not match y={y} band parity ***"
                );
                break;
            }
            pos += 2 + sid;
            k += 1;
        }
        // There is no 4-byte trailer and no inter-record gap: DLM pads each record to a 16-byte
        // stride and carries that exact count in `aux`, so the next record begins at `size + 4`.
        // (The old trailer/gap assertion described the pre-2026-07-22 reading of the framing.)
        if pos != band_end || stream[band_end..off + 4 + size].iter().any(|&b| b != 0) {
            frame_ok = false;
            println!("  *** record {nrecords} padding not zero / band not fully consumed ***");
        }
        off += size + 4;
        nrecords += 1;
    }
    if k != nstrips || off != stream.len() {
        frame_ok = false;
        println!(
            "  *** consumed {k}/{nstrips} strips, {off}/{} stream bytes ***",
            stream.len()
        );
    }
    println!(
        "  {nstrips} strips in {nrecords} record band(s) across {} transfer(s): strip_id==len, \
         bodies == standalone colour_strip, prefix/trailer/gap valid -> {}",
        frames.len(),
        if frame_ok { "OK" } else { "FAIL" }
    );

    let video_ok = video_ok && colour_ok && chroma_ok && colour_ac_pass && frame_ok;

    // Report the per-head 1440p flat-fill the live demo sends (2560x1440).
    let flat: Vec<u8> = [200u8, 30, 30].repeat(2560 * 1440);
    if let Ok((frames, _)) = crate::kvino::colour_frame_ep08(2560, 1440, &flat, 0) {
        let bytes: usize = frames.iter().map(|f| f.len()).sum();
        println!(
            "  1440p flat fill: {} EP08 frame(s), {} total bytes ({} strips of 64x16)",
            frames.len(),
            bytes,
            (2560 / 64) * (1440 / 16)
        );
    }
    if video_ok {
        println!("  [PASS] kernel video::wht::colour_strip == DLM; 1440p flat fill built.");
    }

    let builder_exact = hb.0 + cur.0;
    let builder_documented = hb.1 + cur.1;
    let builder_total = hb.2 + cur.2;
    let builders_pass = builder_total == 0 || builder_documented == builder_total;
    if builders_pass && builder_total > 0 {
        println!(
            "  [PASS] kernel CP builders reproduce DLM's plaintext (documented regions aside)."
        );
    }

    Report {
        seal_ok,
        seal_total,
        seal_skipped: skipped,
        builder_exact,
        builder_documented,
        builder_total,
        mismatches,
        video_ok,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The whole point, as a CI gate: the literal kernel `cp.rs` must reproduce
    /// DLM's captured wire byte-for-byte, and every builder-reproducible inner
    /// message must match (documented uninitialised regions aside).
    #[test]
    fn kernel_cp_matches_dlm() {
        let r = run();
        assert_eq!(r.seal_ok, r.seal_total, "seal divergence from DLM");
        assert!(r.seal_total > 0, "no decryptable CP frames in fixtures");
        assert_eq!(r.mismatches, 0, "{} seal mismatches", r.mismatches);
        assert_eq!(
            r.builder_documented, r.builder_total,
            "CP builder divergence from DLM plaintext"
        );
        assert!(
            r.video_ok,
            "video codec solid strip diverges from DLM reference"
        );
    }

    /// **Chimera-first EDID-wait proof (2026-07-17).** Before this landed in `vino.ko`, prove
    /// against the literal kernel `cp.rs` (via `kvino`) that `edid_poll_ready` correctly reads
    /// the dock's downstream-DDC readiness bit from two REAL `id=0x0044 sub=0x0020` probe
    /// replies (wire seq 205/857 in
    /// `captures/rr-out-sequence-20260716/full-session-trace1/raw-out-in-192msg.txt`) -- the
    /// first is DLM's own probe right before its FIRST placeholder `id=0x114` fetch, the second
    /// is the probe right before its FIRST real `id=0x194` fetch. Also proves the new
    /// `edid_engage_req`/`device_query_req(.., 0x0c)` builders produce the documented 32-byte
    /// shape. Userspace-testable without a kernel rebuild -- exactly the "prove in chimera
    /// before vino" workflow this file exists for.
    #[test]
    fn edid_poll_ready_and_new_builders() {
        const KS: [u8; 16] = [
            0xd9, 0xec, 0x1f, 0xbc, 0x8b, 0x5a, 0xb3, 0xd8, 0x71, 0x0f, 0xd3, 0xbd, 0x42, 0x04,
            0x06, 0x55,
        ];
        const OUT_RIV: [u8; 8] = [0xf6, 0x21, 0xdc, 0x0d, 0x22, 0x7e, 0xf4, 0xaf];
        #[rustfmt::skip]
        const NOT_READY: [u8; 112] = [
            0x00, 0x00, 0x6c, 0x00, 0x04, 0x00, 0x00, 0x00, 0x45, 0x00, 0x0a, 0x00, 0xcd, 0x00,
            0x00, 0x00, 0xa5, 0xea, 0x5d, 0x51, 0xf6, 0xa8, 0x6b, 0xb6, 0x89, 0x88, 0x01, 0xa2,
            0x47, 0x30, 0xbd, 0x6c, 0x84, 0xb8, 0xaf, 0x9f, 0x85, 0xf2, 0x8a, 0x20, 0xc8, 0xec,
            0x51, 0x9e, 0x8d, 0xeb, 0xef, 0x5a, 0x3a, 0x1d, 0xb5, 0xc7, 0x80, 0x02, 0xfe, 0x1e,
            0xed, 0x07, 0xdd, 0x71, 0x00, 0x7f, 0x45, 0x77, 0x6c, 0x82, 0xf6, 0xe9, 0xc3, 0x0d,
            0xdf, 0x67, 0x82, 0xac, 0xa8, 0x23, 0xd5, 0x5a, 0x1c, 0xce, 0xcb, 0x89, 0xb5, 0x98,
            0x65, 0xba, 0xbb, 0xb6, 0x2d, 0x0e, 0x9b, 0x55, 0xee, 0xfd, 0x46, 0x0c, 0x22, 0x35,
            0x6f, 0x84, 0xe5, 0x36, 0x95, 0xd0, 0xdc, 0xfc, 0x6f, 0x8a, 0x57, 0xda, 0xa2, 0xae,
        ];
        #[rustfmt::skip]
        const READY: [u8; 112] = [
            0x00, 0x00, 0x6c, 0x00, 0x04, 0x00, 0x00, 0x00, 0x45, 0x00, 0x0a, 0x00, 0x59, 0x03,
            0x00, 0x00, 0xf7, 0x8e, 0x70, 0xb2, 0xa3, 0x24, 0xe2, 0x6f, 0x9f, 0xb6, 0xe9, 0x8e,
            0x32, 0x55, 0x11, 0x21, 0x99, 0x74, 0xf6, 0xfb, 0xea, 0x97, 0xd5, 0x7f, 0xa6, 0x45,
            0x9d, 0x35, 0xf0, 0xa7, 0xbe, 0xd3, 0x9b, 0x19, 0x24, 0x8c, 0x98, 0xa6, 0x0c, 0xa2,
            0x4d, 0x8e, 0x83, 0xaa, 0x74, 0xd5, 0x8b, 0xe0, 0x6f, 0xb1, 0x9f, 0xa4, 0xb9, 0xae,
            0x39, 0xc6, 0x0a, 0x9c, 0x63, 0x70, 0xdb, 0x49, 0x74, 0xe5, 0x85, 0x42, 0x07, 0x7e,
            0xc2, 0x49, 0xfb, 0x67, 0x54, 0xd5, 0x47, 0x72, 0xb7, 0x19, 0x24, 0x8f, 0xb1, 0xb0,
            0xb2, 0x83, 0x89, 0x62, 0x4b, 0xcb, 0x59, 0x15, 0x1f, 0x8f, 0x85, 0xc3, 0xa5, 0x9d,
        ];
        assert_eq!(
            kvino::edid_poll_ready(&KS, &OUT_RIV, &NOT_READY),
            Some(false)
        );
        assert_eq!(kvino::edid_poll_ready(&KS, &OUT_RIV, &READY), Some(true));

        let engage = kvino::edid_engage_req(0x30, 0).unwrap();
        assert_eq!(engage.len(), 32);
        assert_eq!(
            &engage[0..8],
            &[0x16, 0x00, 0x23, 0x00, 0x30, 0x00, 0x00, 0x00]
        );
        assert_eq!(&engage[8..22], &[0u8; 14]);

        let heartbeat = kvino::heartbeat(0x25).unwrap();
        assert_eq!(
            &heartbeat[0..8],
            &[0x16, 0x00, 0x75, 0x00, 0x25, 0x00, 0x00, 0x00]
        );
        assert_eq!(&heartbeat[22..24], &[0xe0, 0x2e]);

        let poll = kvino::device_query_req(0x50, 0x000c).unwrap();
        assert_eq!(poll.len(), 32);
        assert_eq!(
            &poll[0..8],
            &[0x14, 0x00, 0x0c, 0x00, 0x50, 0x00, 0x00, 0x00]
        );
    }
}
