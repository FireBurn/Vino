//! The in-kernel vino control-plane code, compiled verbatim in userspace.
//!
//! [`cp`] and [`proto`] are the **actual kernel source files**
//! (`drivers/gpu/drm/vino/{cp,proto}.rs`) pulled in with `include!`. They see
//! the kernel-prelude shim ([`crate::kshim`]) through their own `use super::*;`,
//! so the AES-CTR seal, the Dl3Cmac, the wire framing and every CP message
//! builder are byte-for-byte the code that ships in `vino.ko`.
//!
//! The kernel marks those items `pub(super)` (module-private to the driver), so
//! this module re-exposes the ones the rig drives through thin `pub` wrappers.
//! The wrappers add no logic — they just call straight into the included code.

// Bring the shim (KVec/GFP_KERNEL/Result/Error/EINVAL, and the `crypto` +
// `bindings` modules) into scope as the parent the included files resolve
// `super::*` against.
pub use crate::kshim::*;
pub use ::kernel::drm::display::hdcp as drm_hdcp;

// The literal kernel files, loaded as real modules (so their inner `//!` docs and
// `#![allow(..)]` resolve natively). They live in `chimera/vino/`, vendored from
// the kernel tree by `scripts/sync-kernel-sources.sh` so this project builds
// standalone. Never hand-edit them: edit the kernel tree and re-run that script,
// or the byte-exactness these proofs rest on quietly stops meaning anything.

/// The literal kernel `proto.rs` (wire framing + plaintext session-init).
/// `dead_code` is allowed because the rig drives only the CP subset of the file.
#[path = "../vino/proto.rs"]
#[allow(dead_code)]
#[rustfmt::skip]
pub mod proto;

/// The literal kernel `cp.rs` (CP message builders + the AES-CTR/Dl3Cmac seal).
#[path = "../vino/cp.rs"]
#[allow(dead_code)]
#[rustfmt::skip]
pub mod cp;

/// The literal kernel `video.rs` (Vino WHT codec + EP08 transport framing).
/// `dead_code` is allowed because the rig drives only the solid-strip path.
#[path = "../vino/video.rs"]
#[allow(dead_code)]
#[rustfmt::skip]
pub mod video;

/// The literal kernel video-decoder configuration builder.
#[path = "../vino/video_arm.rs"]
#[allow(dead_code)]
#[rustfmt::skip]
pub mod video_arm;

/// The literal kernel `ake.rs` (HDCP 2.2 AKE wire-layer message builders + IN
/// parser). Pure functions -- no kernel-only types -- so it joins the shim with
/// zero drift, exactly like `cp`/`proto`.
#[path = "../vino/ake.rs"]
#[allow(dead_code)]
#[rustfmt::skip]
pub mod ake;

/// The literal kernel `hdcp.rs` (HDCP 2.2 KDF: dKey/kd/H'/L'/V, RSA-OAEP km wrap,
/// SKE `Edkey(ks)`). Also pure -- built on the shimmed `crypto`/`rng`.
#[path = "../vino/hdcp.rs"]
#[allow(dead_code)]
#[rustfmt::skip]
pub mod hdcp;

// ---- thin pub wrappers over the kernel's pub(super) CP API ------------------
//
// `super::cp::*` items are `pub(in crate::kvino)`, hence visible here. Each
// wrapper is a one-liner delegating into the included kernel function.

/// `cp::seal_interactive` — type=4 sub=0x24 interactive frame (CTR + live Dl3Cmac).
pub fn seal_interactive(
    ks: &[u8; 16],
    riv: &[u8; 8],
    id: u16,
    wire_seq: u32,
    content: &[u8],
) -> Result<Vec<u8>> {
    Ok(cp::seal_interactive(ks, riv, id, wire_seq, content)?.into_vec())
}

/// The bare AES-CTR keystream transform, for the untagged frames only a capture contains.
///
/// The driver's [`cp::open_in`] verifies a trailing Dl3Cmac before decrypting, which is right for
/// everything the dock sends today and wrong for the pre-engagement `wsub=0x04` bodies in the old
/// captures: those carry no tag at all. This is that construction with the authentication removed,
/// and it is deliberately the only hand-written cipher code in the rig -- every frame vino itself
/// emits or accepts goes through the kernel path.
fn ctr_xor(ks: &[u8; 16], riv: &[u8; 8], seq: u32, data: &[u8]) -> Result<Vec<u8>> {
    let cipher = crypto::Aes128::new(ks)?;
    let mut out = Vec::with_capacity(data.len());
    for (i, chunk) in data.chunks(16).enumerate() {
        let mut iv = [0u8; 16];
        iv[..8].copy_from_slice(riv);
        iv[12..].copy_from_slice(&seq.wrapping_add(i as u32).to_be_bytes());
        let block = cipher.encrypt_block(&iv);
        out.extend(chunk.iter().zip(block.iter()).map(|(&c, &k)| c ^ k));
    }
    Ok(out)
}

/// Recover the plaintext of an untagged type-4 stream frame body.
pub fn open_stream(ks: &[u8; 16], riv: &[u8; 8], seq: u32, ct: &[u8]) -> Result<Vec<u8>> {
    ctr_xor(ks, riv, seq, ct)
}

/// Rebuild an untagged type-4 stream frame from the current kernel primitives.
///
/// Older DLM captures contain pre-engagement `wsub=0x04` frames whose bodies use the same AES-CTR
/// transform as [`cp::open_in`] but carry no Dl3Cmac. Vino no longer emits that historical path,
/// so the proof composes [`ctr_xor`] with the literal kernel framer here.
pub fn seal_stream(
    key: &[u8; 16],
    riv: &[u8; 8],
    wsub: u16,
    seq: u32,
    inner: &[u8],
) -> Result<Vec<u8>> {
    let ciphertext = ctr_xor(key, riv, seq, inner)?;
    let id = inner
        .get(..2)
        .map(|id| u16::from_le_bytes([id[0], id[1]]))
        .unwrap_or(0);
    let mut frame = KVec::with_capacity(16 + ciphertext.len(), GFP_KERNEL)?;
    proto::push_frame_with(
        &mut frame,
        4,
        wsub,
        cp::aux_for_id(id, inner.len()),
        seq,
        &ciphertext,
    )?;
    Ok(frame.into_vec())
}

/// `cp::open_in` — verify a dock->host body's trailing Dl3Cmac and AES-CTR decrypt it.
///
/// `body` is everything after the 16-byte wire header, tag included: the tag is what proves the
/// riv candidate, so it must not be stripped first.
pub fn open_in(ks: &[u8; 16], riv: &[u8; 8], seq: u32, body: &[u8]) -> Result<Vec<u8>> {
    Ok(cp::open_in(ks, riv, seq, body)?.into_vec())
}

/// `cp::in_riv` — derive the dock→host riv from the host→dock riv (identity).
pub fn in_riv(out_riv: &[u8; 8]) -> [u8; 8] {
    cp::in_riv(out_riv)
}

/// `cp::aux_for_id` — the per-inner-id wire-header `aux` constant.
pub fn aux_for_id(id: u16, body_len: usize) -> u16 {
    cp::aux_for_id(id, body_len)
}

/// `cp::dl3cmac_tag` — the DisplayLink CP integrity tag.
pub fn dl3cmac_tag(
    ks: &[u8; 16],
    riv: &[u8; 8],
    wire_seq: u64,
    ciphertext: &[u8],
) -> Result<[u8; 16]> {
    cp::dl3cmac_tag(ks, riv, wire_seq, ciphertext)
}

/// `cp::heartbeat` — OUT `id=0x16 sub=0x75` keepalive inner plaintext.
pub fn heartbeat(counter: u16) -> Result<Vec<u8>> {
    Ok(cp::heartbeat(counter)?.into_vec())
}

pub fn stream_marker(counter: u16, head: u8, sub: u16, state: u8) -> Result<Vec<u8>> {
    Ok(cp::stream_marker(counter, head, sub, state)?.into_vec())
}

pub fn stream_commit(counter: u16, head: u8) -> Result<Vec<u8>> {
    Ok(cp::stream_commit(counter, head)?.into_vec())
}

pub fn cp_session_key(ske_ks: &[u8; 16]) -> ::kernel::crypto::Secret<16> {
    cp::cp_session_key(ske_ks)
}

/// `cp::stream_content_nonce` for one of this dock's video streams.
pub fn video_content_nonce(riv: &[u8; 8], head: u8) -> [u8; 8] {
    cp::stream_content_nonce(riv, geometry().stream_id(head))
}

/// `cp::get_edid_req` — OUT `id=0x15 sub=0x21` EDID-read request inner plaintext.
pub fn get_edid_req(counter: u16, head: u8) -> Result<Vec<u8>> {
    Ok(cp::get_edid_req(counter, head)?.into_vec())
}

/// `cp::get_edid_req_sub` — OUT `id=0x15` EDID-family request with an explicit sub
/// (`0x20` readiness probe / `0x21` fetch).
pub fn get_edid_req_sub(counter: u16, sub: u16, head: u8) -> Result<Vec<u8>> {
    Ok(cp::get_edid_req_sub(counter, sub, head)?.into_vec())
}

/// `cp::device_query_req` — OUT `id=0x14` device-status/capability query with an
/// explicit sub (`0x0000` one-shot capability query / `0x000c` repeated status poll).
pub fn device_query_req(counter: u16, sub: u16) -> Result<Vec<u8>> {
    Ok(cp::device_query_req(counter, sub)?.into_vec())
}

/// `cp::edid_engage_req` — OUT `id=0x16 sub=0x0023` EDID-engage, sent (twice) once
/// early placeholder fetches keep failing, right before the long status-poll run.
pub fn edid_engage_req(counter: u16, head: u8) -> Result<Vec<u8>> {
    Ok(cp::edid_engage_req(counter, head)?.into_vec())
}

pub fn edid_readiness_kick(counter: u16, head: u8) -> Result<Vec<u8>> {
    Ok(cp::edid_readiness_kick(counter, head)?.into_vec())
}

pub fn post_edid_query(counter: u16, head: u8) -> Result<Vec<u8>> {
    Ok(cp::post_edid_query(counter, head)?.into_vec())
}

/// `cp::edid_poll_ready` — decode an `id=0x0044 sub=0x0020` EDID-readiness probe
/// reply and report whether the dock's downstream DDC/EDID read has finished
/// (inner byte offset 26, `0x00` busy -> `0x80` ready). `None` if `wire` isn't a
/// decryptable `sub=0x0020` reply at all.
pub fn edid_poll_ready(ks: &[u8; 16], out_riv: &[u8; 8], wire: &[u8]) -> Option<bool> {
    cp::edid_poll_ready(ks, out_riv, wire)
}

/// `cp::probe_reply_status` — decode the handler and status of a presence probe.
pub fn probe_reply_status(
    ks: &[u8; 16],
    out_riv: &[u8; 8],
    wire: &[u8],
) -> Option<(u16, u32, bool)> {
    cp::probe_reply_status(ks, out_riv, wire)
}

/// `cp::cursor_create` inner plaintext.
pub fn cursor_create(counter: u16, head: u8, w: u16, h: u16) -> Result<Vec<u8>> {
    Ok(cp::cursor_create(counter, head, w, h)?.into_vec())
}

/// `cp::cursor_move` inner plaintext. `visible` drives the dock's own visible flag; clearing it is
/// how the cursor is hidden, because the dock wraps an out-of-range origin instead of clipping.
pub fn cursor_move(counter: u16, head: u8, x: u16, y: u16, visible: bool) -> Result<Vec<u8>> {
    Ok(cp::cursor_move(counter, head, x, y, visible)?.into_vec())
}

/// `cp::cursor_image` inner plaintext (32-byte header + `w*h*4` BGRA bitmap).
pub fn cursor_image(counter: u16, head: u8, w: u16, h: u16, bgra: &[u8]) -> Result<Vec<u8>> {
    Ok(cp::cursor_image(counter, head, w, h, bgra)?.into_vec())
}

/// `cp::parse_edid_from_reply` — pull the EDID blob from a dock EDID reply frame.
pub fn parse_edid_from_reply(
    ks: &[u8; 16],
    out_riv: &[u8; 8],
    wire: &[u8],
) -> Result<Option<Vec<u8>>> {
    Ok(cp::parse_edid_from_reply(ks, out_riv, wire)?.map(|v| v.into_vec()))
}

pub const CP_SETUP_PER_HEAD: [(u16, u16, usize); 9] = cp::CP_SETUP_PER_HEAD;

/// The three finalization messages sent per connector. The driver repeats them for each head that
/// authenticated, which is what the six-entry constant this replaced spelled out by hand.
pub const CP_SETUP_FINALIZE_STEPS: [(u16, u16); 3] = cp::CP_SETUP_FINALIZE_STEPS;

pub fn stream_manage_restatement(counter: u16, head: u8) -> Result<Vec<u8>> {
    let geom = geometry();
    Ok(cp::stream_manage_restatement(counter, head, geom.stream_id(head), NAVARRO)?.into_vec())
}

/// The fresh per-head `rrx` from an `AKE_Send_rrx` push, if this frame is one.
///
/// The kernel decodes every downstream-HDCP push through one parser and dispatches on the HDCP
/// message id; the rig needs only `AKE_Send_rrx` (`0x06`), whose payload starts with the eight
/// `rrx` bytes.
pub fn perhead_rrx(ks: &[u8; 16], out_riv: &[u8; 8], wire: &[u8]) -> Option<[u8; 8]> {
    let push = cp::perhead_hdcp_push(ks, out_riv, wire)?;
    if push.msg_id != 0x06 || push.payload_len < 8 {
        return None;
    }
    let mut rrx = [0u8; 8];
    rrx.copy_from_slice(&push.payload[..8]);
    Some(rrx)
}

// ---- video: solid-colour frame builder over the kernel WHT codec ------------

/// Vino strip geometry: each `solid_strip` covers a 64-px-wide × 16-px-tall tile.
pub const STRIP_W: u16 = 64;
pub const STRIP_H: u16 = 16;

/// Which protocol generation the rig drives. The driver's `Generation` names the same split; the
/// rig speaks Ridge only, so the calls that branch on it -- the per-head connector selector and the
/// mode-set words -- take `false` here.
const NAVARRO: bool = false;

/// The dock geometry every call below encodes for.
///
/// The driver carries one of these per dock in `profile.rs` and passes it down; the rig drives a
/// Ridge dock only, so it names the kernel's own `RIDGE_GEOMETRY` in one place instead of
/// threading a parameter no caller varies.
pub(crate) fn geometry() -> video::wht::Geometry {
    video::wht::RIDGE_GEOMETRY
}

/// How many buffers the dock rotates through as it presents frames.
///
/// A keyframe must reach every one of them, so it is presented this many times with an advancing
/// trailer phase; see [`crate::scanout`].
pub fn dock_buffers() -> u32 {
    u32::from(geometry().dock_buffers)
}

/// How many consecutive frames must carry a strip after its content changes, so that every one of
/// the dock's buffers receives it. The driver's `damage_repeats()`: the ring depth plus one frame
/// of margin for a presentation the dock drops or applies to the buffer it just used.
pub fn damage_repeats() -> u8 {
    geometry().dock_buffers.saturating_add(1)
}

/// The strip the codec tiles a surface into: 64x16 px on Ridge.
pub fn strip_dims() -> (usize, usize) {
    let geom = geometry();
    (geom.strip_w(), geom.strip_h())
}

/// `video::wht::colour` — the Vino integer colour transform
/// `(Y=16R+32G+16B, Cb=64(R−G), Cr=64(B−G))`, yielding the per-plane DC values.
pub fn colour(r: u8, g: u8, b: u8) -> (i32, i32, i32) {
    video::wht::colour(r, g, b)
}

/// `video::wht::colour_strip` driven from 16 blocks of three raw planes (each 64 samples in
/// the codec's ×64 fixed point: `[Cr=64*(B-G), Cb=64*(R-G), Y=64*G+64*((Cb+Cr)>>2)]`). Runs the
/// LITERAL kernel `colour_block` + `colour_strip`, so the RE harness can prove the in-kernel
/// colour codec byte-exact against real DLM sink strips.
pub fn colour_strip_from_planes(planes: &[[[i32; 64]; 3]; 16], x: u16, y: u16) -> Result<Vec<u8>> {
    let blocks: [video::wht::ColourBlock; 16] = core::array::from_fn(|k| {
        video::wht::colour_block(&planes[k][0], &planes[k][1], &planes[k][2])
    });
    Ok(video::wht::colour_strip(&blocks, x, y)?.into_vec())
}

/// `video::wht::colour_frame_ep08` driven from a packed 8-bit RGB frame (`width*height*3`
/// bytes, R,G,B raster order). Runs the LITERAL kernel colour FRAME assembler (strip tiling +
/// forward-hint tail chaining + EP08 split), so the rig can prove the in-kernel colour frame
/// path, not just individual strips. Returns the ready-to-send EP08 frames and the next `seq`.
pub fn colour_frame_ep08(
    width: usize,
    height: usize,
    rgb: &[u8],
    seq0: u32,
) -> Result<(Vec<Vec<u8>>, u32)> {
    colour_frame_ep08_head(width, height, rgb, seq0, 0)
}

pub fn colour_frame_ep08_head(
    width: usize,
    height: usize,
    rgb: &[u8],
    seq0: u32,
    head: u8,
) -> Result<(Vec<Vec<u8>>, u32)> {
    let (frames, seq) =
        video::wht::colour_frame_ep08(geometry(), width, height, seq0, head, |x, y| {
            let i = (y * width + x) * 3;
            (rgb[i], rgb[i + 1], rgb[i + 2])
        })?;
    Ok((
        frames
            .into_vec()
            .into_iter()
            .map(|f| f.into_vec())
            .collect(),
        seq,
    ))
}

/// `video::wht::damage_strip_coords` / `all_strip_coords` — the strips a frame carries, raster
/// ordered.
///
/// **The order is load-bearing**: [`frame_records`] groups strips into one record per single-Y
/// band and requires them x-ordered within each band, so reordering changes the wire format.
/// `clips` of `None` selects every strip.
pub fn strip_coords(
    width: usize,
    height: usize,
    clips: Option<&[(usize, usize, usize, usize)]>,
) -> Result<Vec<(usize, usize)>> {
    let geom = geometry();
    let coords = match clips {
        Some(clips) => video::wht::damage_strip_coords(geom, width, height, clips)?,
        None => video::wht::all_strip_coords(geom, width, height)?,
    };
    Ok(coords.into_vec())
}

/// `video::wht::colour_strip_at` — encode the one strip whose top-left pixel is `(sx, sy)`.
pub fn encode_strip(width: usize, rgb: &[u8], sx: usize, sy: usize) -> Result<Vec<u8>> {
    let mut px = |x: usize, y: usize| {
        let i = (y * width + x) * 3;
        (rgb[i], rgb[i + 1], rgb[i + 2])
    };
    Ok(video::wht::colour_strip_at(geometry(), sx, sy, &mut px)?.into_vec())
}

/// `video::wht::frame_records` — frame encoded strip bodies into EP08 records.
pub fn frame_records(strips: &[Vec<u8>], head: u8) -> Result<Vec<Vec<u8>>> {
    let owned: Vec<KVec<u8>> = strips
        .iter()
        .map(|s| {
            let mut k = KVec::new();
            k.extend_from_slice(s, GFP_KERNEL)?;
            Ok(k)
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(video::wht::frame_records(geometry(), &owned, head)?
        .into_vec()
        .into_iter()
        .map(|r| r.into_vec())
        .collect())
}

/// `video::wht::frame_trailer` — the 96-byte per-frame trailer the dock expects after each image.
///
/// The kernel driver appends this itself rather than folding it into the codec output, so any
/// caller building an EP08 stream must do the same. Its phase is derived from the sequence number
/// (`seq % 3`), which is how the dock rotates its buffers: repeating one sequence pins the phase.
pub fn frame_trailer(head: u8, seq: u32) -> Vec<u8> {
    video::wht::frame_trailer(geometry(), head, seq).to_vec()
}

pub fn black_frame_ep08(width: usize, height: usize, head: u8) -> Result<Vec<Vec<u8>>> {
    Ok(
        video::wht::black_frame_ep08(geometry(), width, height, head)?
            .into_vec()
            .into_iter()
            .map(|frame| frame.into_vec())
            .collect(),
    )
}

pub fn set_mode_profile(
    counter: u16,
    head: u8,
    width: u16,
    height: u16,
    refresh_hz: u16,
) -> Result<Vec<u8>> {
    let raw = match (width, height, refresh_hz) {
        (1280, 720, 60) => bindings::drm_display_mode {
            clock: 74_250,
            hdisplay: 1280,
            hsync_start: 1390,
            hsync_end: 1430,
            htotal: 1650,
            vdisplay: 720,
            vsync_start: 725,
            vsync_end: 730,
            vtotal: 750,
        },
        (1920, 1080, 60 | 120) => bindings::drm_display_mode {
            clock: if refresh_hz == 60 { 148_500 } else { 297_000 },
            hdisplay: 1920,
            hsync_start: 2008,
            hsync_end: 2052,
            htotal: 2200,
            vdisplay: 1080,
            vsync_start: 1084,
            vsync_end: 1089,
            vtotal: 1125,
        },
        (2560, 1440, 60 | 120) => bindings::drm_display_mode {
            clock: if refresh_hz == 60 { 241_500 } else { 497_750 },
            hdisplay: 2560,
            hsync_start: 2608,
            hsync_end: 2640,
            htotal: 2720,
            vdisplay: 1440,
            vsync_start: 1443,
            vsync_end: 1448,
            vtotal: if refresh_hz == 60 { 1481 } else { 1525 },
        },
        (3840, 2160, 60) => bindings::drm_display_mode {
            clock: 533_120,
            hdisplay: 3840,
            hsync_start: 3888,
            hsync_end: 3920,
            htotal: 4000,
            vdisplay: 2160,
            vsync_start: 2163,
            vsync_end: 2168,
            vtotal: 2222,
        },
        _ => return Err(kernel::error::code::EOPNOTSUPP),
    };
    let mode = kernel::drm::kms::modes::DisplayMode(raw);
    let timing = cp::timing_from_drm_mode(&mode, NAVARRO)?;
    Ok(cp::set_mode(counter, head, &timing)?.into_vec())
}

pub fn video_arm_burst(
    head: u8,
    key: &[u8; 16],
    nonce: &[u8; 8],
    width: u16,
    height: u16,
) -> Result<Vec<u8>> {
    let mut output = Vec::with_capacity(2560);
    let mut seal_seq = 0;
    for (index, &(_wire_type, sub_base, aux, body_len)) in cp::VIDEO_ARM_BURST.iter().enumerate() {
        let sub = sub_base.wrapping_add(head as u16);
        match index {
            2 | 3 => {
                let mut content = [0u8; 16];
                content[..6].copy_from_slice(&[0x04, 0x00, 0x08, 0x04, 0x03, 0x00]);
                rng::fill(&mut content[6..]);
                output.extend_from_slice(&cp::seal_video_arm(
                    key, nonce, sub, aux, seal_seq, &content,
                )?);
                seal_seq += 1;
            }
            6 | 7 => {
                let mut frame = [0u8; 32];
                frame[2..4].copy_from_slice(&0x001cu16.to_le_bytes());
                frame[4..8].copy_from_slice(&4u32.to_le_bytes());
                frame[8..10].copy_from_slice(&sub.to_le_bytes());
                frame[10..12].copy_from_slice(&aux.to_le_bytes());
                frame[16..].copy_from_slice(&[
                    0x0a, 0x00, 0x04, 0x00, 0, 0, 0, 0, 0, 0, 0, 0x10, 0, 0, 0, 0,
                ]);
                output.extend_from_slice(&frame);
            }
            8 | 9 => {
                debug_assert_eq!(body_len, 1104);
                let mut random = [0; 14];
                rng::fill(&mut random);
                let content = video_arm::build(width, height, &random)
                    .map_err(|_| Error(12))?
                    .into_vec();
                output.extend_from_slice(&cp::seal_video_arm(
                    key, nonce, sub, aux, seal_seq, &content,
                )?);
                seal_seq += (body_len / 16) as u32;
            }
            _ => {
                let body = cp::video_arm_plaintext_body(index, head as u16);
                output.extend_from_slice(&cp::video_arm_plain_frame(sub, &body));
            }
        }
    }
    Ok(output)
}

/// One byte-exact colour 64×16 strip at `(x,y)` from 16 blocks of three raw planes (each 64
/// samples in the codec's ×64 fixed point, `[Cr, Cb, Y]`), via the kernel `colour_strip`. Same
/// as [`colour_strip_from_planes`] but exposed for the frame-assembler proof to reconstruct each
/// strip independently and check the assembler matches it (tail aside).
pub fn colour_strip_blocks(planes: &[[[i32; 64]; 3]; 16], x: u16, y: u16) -> Result<Vec<u8>> {
    colour_strip_from_planes(planes, x, y)
}

/// `video::wht::transform` + `quantize` for one 8×8 plane (values already in the
/// codec's ×64 fixed point), returning the 32 quantized coeffs.
pub fn transform_raw(plane: &[i32; 64]) -> [i32; 64] {
    video::wht::transform(plane)
}

pub fn transform_quantize(plane: &[i32; 64]) -> [i32; 64] {
    let c = video::wht::transform(plane);
    let mut q = [0i32; 64];
    for i in 0..64 {
        q[i] = video::wht::quantize(c[i], i);
    }
    q
}

// ---- thin pub wrappers over the kernel's pub(super) AKE/HDCP API -----------
//
// Same pattern as the CP wrappers above: `ake`/`hdcp` items are `pub(super)`
// inside the included kernel files, i.e. visible here (in `kvino`) but not
// outside it -- and NOTE that "outside it" includes other crates in this same
// Cargo workspace/package: each `[[bin]]` target compiles as its own crate
// linking `vino-chimera` as an external dependency, so even `pub(crate)` items
// (like `ake::id`) are invisible from `src/bin/*.rs`. Every item the
// orchestration binaries need gets a genuine `pub` wrapper here.

/// HDCP 2.2 message IDs (`ake::id` is `pub(crate)`, invisible from the `[[bin]]`
/// targets -- see the note above). Values copied by reference from the
/// literal kernel constants, not re-stated, so they can't drift.
pub mod id {
    use super::ake::id as k;
    pub const AKE_INIT: u8 = k::AKE_INIT;
    pub const AKE_SEND_CERT: u8 = k::AKE_SEND_CERT;
    pub const AKE_NO_STORED_KM: u8 = k::AKE_NO_STORED_KM;
    pub const AKE_SEND_H_PRIME: u8 = k::AKE_SEND_H_PRIME;
    pub const LC_INIT: u8 = k::LC_INIT;
    pub const LC_SEND_L_PRIME: u8 = k::LC_SEND_L_PRIME;
    pub const SKE_SEND_EKS: u8 = k::SKE_SEND_EKS;
    pub const REPEATERAUTH_SEND_RECEIVERID_LIST: u8 = k::REPEATERAUTH_SEND_RECEIVERID_LIST;
    pub const REPEATERAUTH_SEND_ACK: u8 = k::REPEATERAUTH_SEND_ACK;
    pub const REPEATERAUTH_STREAM_MANAGE: u8 = k::REPEATERAUTH_STREAM_MANAGE;
    pub const REPEATERAUTH_STREAM_READY: u8 = k::REPEATERAUTH_STREAM_READY;
    pub const AKE_SEND_RRX: u8 = k::AKE_SEND_RRX;
    pub const RECEIVER_AUTH_STATUS: u8 = k::RECEIVER_AUTH_STATUS;
    pub const AKE_TRANSMITTER_INFO: u8 = k::AKE_TRANSMITTER_INFO;
}

/// `proto::init_0`/`init_25`/`init_4_probe` — the plaintext session-init
/// messages (`pub(super)` in the kernel file, hence the wrapper).
pub fn init_0() -> Result<Vec<u8>> {
    Ok(proto::init_0()?.into_vec())
}
pub fn init_25() -> Result<Vec<u8>> {
    Ok(proto::init_25()?.into_vec())
}
pub fn init_4_probe() -> Result<Vec<u8>> {
    Ok(proto::init_4_probe()?.into_vec())
}
pub fn session_init_ack(hdcp_seq: u32, seq: u32) -> Result<Vec<u8>> {
    Ok(ake::session_init_ack(hdcp_seq, seq)?.into_vec())
}
/// `cp::seal_livemac` — seal `msg0` (fresh live Dl3Cmac over live content,
/// reusing a captured wire header's `seq`/`aux`). See `cp.rs` for the formula.
pub fn seal_livemac(
    ks: &[u8; 16],
    riv: &[u8; 8],
    header: &[u8],
    content: &[u8],
) -> Result<Vec<u8>> {
    Ok(cp::seal_livemac(ks, riv, header, content)?.into_vec())
}

pub fn ake_init(hdcp_seq: u32, seq: u32, rtx: &[u8; 8], tx_caps: &[u8; 3]) -> Result<Vec<u8>> {
    Ok(ake::ake_init(hdcp_seq, seq, rtx, tx_caps)?.into_vec())
}
pub fn ake_transmitter_info(hdcp_seq: u32, seq: u32) -> Result<Vec<u8>> {
    Ok(ake::ake_transmitter_info(hdcp_seq, seq)?.into_vec())
}
pub fn ake_no_stored_km(hdcp_seq: u32, seq: u32, ekpub_km: &[u8; 128]) -> Result<Vec<u8>> {
    Ok(ake::ake_no_stored_km(hdcp_seq, seq, ekpub_km)?.into_vec())
}
pub fn lc_init(hdcp_seq: u32, seq: u32, rn: &[u8; 8]) -> Result<Vec<u8>> {
    Ok(ake::lc_init(hdcp_seq, seq, rn)?.into_vec())
}
pub fn ske_send_eks(
    hdcp_seq: u32,
    seq: u32,
    edkey_ks: &[u8; 16],
    riv: &[u8; 8],
) -> Result<Vec<u8>> {
    Ok(ake::ske_send_eks(hdcp_seq, seq, edkey_ks, riv)?.into_vec())
}
pub fn repeater_auth_send_ack(hdcp_seq: u32, seq: u32, v: &[u8; 16]) -> Result<Vec<u8>> {
    Ok(ake::repeater_auth_send_ack(hdcp_seq, seq, v)?.into_vec())
}
pub fn repeater_auth_stream_manage(hdcp_seq: u32, seq: u32) -> Result<Vec<u8>> {
    Ok(ake::repeater_auth_stream_manage(hdcp_seq, seq)?.into_vec())
}
/// `ake::parse_in` — parse an IN HDCP body into `(msg_id, payload)`.
pub fn ake_parse_in(body: &[u8]) -> Option<(u8, &[u8])> {
    ake::parse_in(body)
}

pub fn derive_kd(km: &[u8; 16], rtx: &[u8; 8], rrx: &[u8; 8]) -> Result<[u8; 32]> {
    hdcp::derive_kd(km, rtx, rrx).map(|key| *key)
}
pub fn compute_h(kd: &[u8; 32], rtx: &[u8; 8], repeater: bool) -> [u8; 32] {
    hdcp::compute_h(kd, rtx, repeater)
}
pub fn compute_l(kd: &[u8; 32], rrx: &[u8; 8], rn: &[u8; 8]) -> [u8; 32] {
    hdcp::compute_l(kd, rrx, rn)
}
pub fn compute_v_full(kd: &[u8; 32], list_header: &[u8]) -> [u8; 32] {
    hdcp::compute_v_full(kd, list_header)
}
pub type RsaPublicKey = ::kernel::crypto::akcipher::RsaPublicKey;

pub fn rsa_public_key(modulus: &[u8; 128], exponent: &[u8]) -> Result<RsaPublicKey> {
    Ok(RsaPublicKey::new(modulus, exponent, GFP_KERNEL)?)
}

pub fn oaep_encrypt_km(key: &mut RsaPublicKey, km: &[u8; 16]) -> Result<[u8; 128]> {
    hdcp::oaep_encrypt_km(key, km)
}
pub fn compute_eks(
    km: &[u8; 16],
    rtx: &[u8; 8],
    rrx: &[u8; 8],
    rn: &[u8; 8],
    ks: &[u8; 16],
) -> Result<[u8; 16]> {
    hdcp::compute_eks(km, rtx, rrx, rn, ks)
}

#[cfg(test)]
mod production_builders {
    use super::*;

    #[test]
    fn every_advertised_mode_has_a_wire_profile() {
        for (width, height, refresh) in [
            (1280, 720, 60),
            (1920, 1080, 60),
            (1920, 1080, 120),
            (2560, 1440, 60),
            (2560, 1440, 120),
            (3840, 2160, 60),
        ] {
            assert_eq!(
                set_mode_profile(1, 0, width, height, refresh)
                    .unwrap()
                    .len(),
                80
            );
        }
        assert!(set_mode_profile(1, 0, 1024, 768, 60).is_err());
    }

    #[test]
    fn complete_video_arm_is_one_prefix() {
        assert_eq!(
            video_arm_burst(0, &[0; 16], &[0; 8], 1920, 1080)
                .unwrap()
                .len(),
            2560
        );
    }

    #[test]
    fn display_capability_reply_reports_presence() {
        let key = [0x5a; 16];
        let riv = [0x33; 8];
        let mut inner = [0; 32];
        inner[0..2].copy_from_slice(&0x78u16.to_le_bytes());
        inner[2..4].copy_from_slice(&0x20u16.to_le_bytes());
        inner[22..26].copy_from_slice(&0x1234u32.to_le_bytes());
        inner[26] = 0x80;
        let mut frame = seal_interactive(&key, &riv, 0x78, 11, &inner).unwrap();
        frame[8..10].copy_from_slice(&0x45u16.to_le_bytes());

        assert_eq!(
            probe_reply_status(&key, &riv, &frame),
            Some((0x78, 0x1234, true))
        );
    }
}
