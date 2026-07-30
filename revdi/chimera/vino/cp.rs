// SPDX-License-Identifier: GPL-2.0
//! Encrypted-control-plane message builders (the inner plaintext of the type=4
//! sub=0x24 AES-CTR frames) plus the AES-CTR `seal` that encrypts and frames them.
use super::*;

/// DisplayLink key whitening applied to the raw SKE session key:
/// ```text
///   cp_session_key = ske_ks XOR CP_KEY_WHITEN
/// ```
///
/// The whitened key is used by both the AES-CTR content cipher and Dl3Cmac. The raw key is wrapped
/// in `Edkey` and delivered to the dock.
pub(super) const CP_KEY_WHITEN: [u8; 16] = [
    0x26, 0xab, 0xee, 0x38, 0x93, 0xd0, 0xc4, 0x32, 0x61, 0x43, 0xa4, 0xbf, 0x5b, 0x45, 0xd6, 0xec,
];

/// Derive the live CP session key from the raw SKE key.
///
/// The result of `ske_ks XOR `[`CP_KEY_WHITEN`] keys the AES-CTR content
/// cipher and the Dl3Cmac in [`seal_livemac`]. The input is wrapped into
/// `Edkey`; the dock applies the same XOR.
pub(super) fn cp_session_key(ske_ks: &[u8; 16]) -> kernel::crypto::Secret<16> {
    let mut key = *ske_ks;
    for i in 0..16 {
        key[i] ^= CP_KEY_WHITEN[i];
    }
    kernel::crypto::Secret::new(key)
}

/// Derive the AES-CTR content nonce for one video head from the RIV carried by that head's
/// `SKE_Send_Eks` restatement (`id=0x32`). Video uses `byte7 ^= 0x08 | head`, distinct from the
/// main control channel's `byte7 ^= 0x04`.
pub(super) fn video_content_nonce(riv: &[u8; 8], head: u8) -> [u8; 8] {
    let mut nonce = *riv;
    nonce[7] ^= 0x08 | head;
    nonce
}

/// Common CP inner header: `[id u16][sub u16][counter u16][00 00]` (sec 6.1/sec 8.6.4).
fn header(out: &mut KVec<u8>, id: u16, sub: u16, counter: u16) -> Result {
    out.extend_from_slice(&id.to_le_bytes(), GFP_KERNEL)?;
    out.extend_from_slice(&sub.to_le_bytes(), GFP_KERNEL)?;
    out.extend_from_slice(&counter.to_le_bytes(), GFP_KERNEL)?;
    out.extend_from_slice(&[0, 0], GFP_KERNEL)?;
    Ok(())
}
fn pad_to(out: &mut KVec<u8>, len: usize) -> Result {
    while out.len() < len {
        out.push(0, GFP_KERNEL)?;
    }
    Ok(())
}
/// OUT session heartbeat: `id=0x16 sub=0x75`, two AES blocks.
///
/// ```text
/// 16 00 75 00 [ctr:2] 00 00   14x 00   e0 2e   [8-byte host-random token]
/// ```
///
/// Offset 22 contains `0x2ee0`; offsets 24..32 are ignored and emitted as zero. The heartbeat runs
/// throughout the streaming session.
pub(super) fn heartbeat(counter: u16) -> Result<KVec<u8>> {
    let mut b = KVec::with_capacity(32, GFP_KERNEL)?;
    header(&mut b, 0x16, 0x75, counter)?;
    pad_to(&mut b, 22)?; // block0 tail + block1[0..6]
    b.extend_from_slice(&[0xe0, 0x2e], GFP_KERNEL)?;
    pad_to(&mut b, 32)?;
    Ok(b)
}
/// Stream enable markers (`id=0x16`, sub `0x2e` or `0x2f`) bracket each mode set:
///   `2f(1) 2e(3)` -> **mode-set** -> `2f(1) 2e(0) 2f(1) 2e(0) 2f(0) 2e(0)`
///
/// Offset 22 selects the head and offset 23 carries the state.
pub(super) fn stream_marker(counter: u16, head: u8, sub: u16, state: u8) -> Result<KVec<u8>> {
    let mut b = KVec::with_capacity(32, GFP_KERNEL)?;
    header(&mut b, 0x16, sub, counter)?;
    pad_to(&mut b, 22)?;
    b.push(head, GFP_KERNEL)?; // off22: downstream head selector
    b.push(state, GFP_KERNEL)?; // off23: state byte
    let mut token = [0u8; 8];
    rng::fill(&mut token);
    b.extend_from_slice(&token, GFP_KERNEL)?;
    Ok(b)
}

pub(super) fn stream_commit(counter: u16, head: u8) -> Result<KVec<u8>> {
    let mut b = KVec::with_capacity(32, GFP_KERNEL)?;
    header(&mut b, 0x16, 0x4c, counter)?;
    pad_to(&mut b, 22)?;
    b.push(if head == 0 { 0 } else { 1 }, GFP_KERNEL)?; // off22: per-head flag
    b.push(0, GFP_KERNEL)?; // off23
    let mut token = [0u8; 8];
    rng::fill(&mut token);
    b.extend_from_slice(&token, GFP_KERNEL)?;
    Ok(b)
}
/// OUT device-status/capability query: `id=0x14`. Subcommand 0 performs initial capability
/// discovery; subcommand `0x0c` polls runtime status.
pub(super) fn device_query_req(counter: u16, sub: u16) -> Result<KVec<u8>> {
    random_tail_msg(0x14, sub, counter)
}
/// Shared builder for the many CP messages that share one wire shape: the standard 8-byte
/// `[id][sub][counter][00 00]` header, 14 zero bytes, then a fresh 10-byte host-random tail the
/// dock treats as an opaque token.
fn random_tail_msg(id: u16, sub: u16, counter: u16) -> Result<KVec<u8>> {
    let mut b = KVec::with_capacity(32, GFP_KERNEL)?;
    header(&mut b, id, sub, counter)?;
    pad_to(&mut b, 22)?;
    let mut tail = [0u8; 10];
    rng::fill(&mut tail);
    b.extend_from_slice(&tail, GFP_KERNEL)?;
    Ok(b)
}
/// OUT `id=0x16 sub=0x0023` downstream-sink engage request. Offsets 22 and 23 both carry the head
/// selector; the remaining bytes are an opaque token.
pub(super) fn edid_engage_req(counter: u16, head: u8) -> Result<KVec<u8>> {
    // Both selector bytes are required by the sink-engage handler.
    let mut b = KVec::with_capacity(32, GFP_KERNEL)?;
    header(&mut b, 0x16, 0x0023, counter)?;
    pad_to(&mut b, 22)?;
    b.push(head, GFP_KERNEL)?;
    b.push(head, GFP_KERNEL)?;
    let mut tail = [0u8; 8];
    rng::fill(&mut tail);
    b.extend_from_slice(&tail, GFP_KERNEL)?;
    Ok(b)
}
/// OUT `id=0x15 sub=0x0053` post-EDID capability query. Offset 22 carries a one-based head index.
pub(super) fn post_edid_query(counter: u16, head: u8) -> Result<KVec<u8>> {
    let mut b = KVec::with_capacity(32, GFP_KERNEL)?;
    header(&mut b, 0x15, 0x0053, counter)?;
    pad_to(&mut b, 22)?;
    b.push(head + 1, GFP_KERNEL)?;
    let mut tail = [0u8; 9];
    rng::fill(&mut tail);
    b.extend_from_slice(&tail, GFP_KERNEL)?;
    Ok(b)
}
/// OUT `id=0x16 sub=0x004b` downstream EDID-read kick.
pub(super) fn edid_readiness_kick(counter: u16, head: u8) -> Result<KVec<u8>> {
    let mut b = KVec::with_capacity(32, GFP_KERNEL)?;
    header(&mut b, 0x16, 0x4b, counter)?;
    pad_to(&mut b, 22)?;
    // Offset 22 selects the downstream head and offset 23 starts the read.
    b.extend_from_slice(&[head, 0x01], GFP_KERNEL)?;
    let mut tail = [0u8; 8];
    rng::fill(&mut tail);
    b.extend_from_slice(&tail, GFP_KERNEL)?;
    Ok(b)
}
/// OUT get-EDID request (`id=0x15 sub=0x21`). A `sub=0x20` probe must precede each fetch attempt.
/// The dock may initially return an internal placeholder, so callers retry until a downstream EDID
/// arrives.
pub(super) fn get_edid_req(counter: u16, head: u8) -> Result<KVec<u8>> {
    get_edid_req_sub(counter, 0x21, head)
}
/// Build an `id=0x15` EDID-family request with an explicit `sub` (`0x20` = probe/seek,
/// `0x21` = fetch -- see [`get_edid_req`]'s doc comment). Same 32-byte wire shape for both.
pub(super) fn get_edid_req_sub(counter: u16, sub: u16, head: u8) -> Result<KVec<u8>> {
    let mut b = KVec::with_capacity(32, GFP_KERNEL)?;
    header(&mut b, 0x15, sub, counter)?;
    pad_to(&mut b, 22)?;
    // Offset 22 selects the downstream head; the remaining bytes are an opaque token.
    b.push(head, GFP_KERNEL)?;
    let mut tail = [0u8; 9];
    rng::fill(&mut tail);
    b.extend_from_slice(&tail, GFP_KERNEL)?;
    Ok(b)
}
/// A video timing as carried by the `0x48/0x22` set-mode message.
#[derive(Clone, Copy)]
pub(super) struct Timing {
    pub hactive: u16,
    pub hblank: u16,
    pub hsync_front: u16,
    pub hsync_width: u16,
    pub vactive: u16,
    pub vblank: u16,
    pub vsync_front: u16,
    pub vsync_width: u16,
    pub refresh_hz: u16,
    /// Pixel clock in 10 kHz units, serialized at offset 70.
    pub pixel_clock_10khz: u16,
    /// Link configuration word at offset 42, selected by [`mode_profile`].
    pub field42: u16,
    /// Mode-dependent word at offset 66, selected by [`mode_profile`].
    pub off66: u16,
}

/// Select the offset-42 downstream link word from the mode's width.
///
/// The field is resolution-keyed, not bandwidth-keyed: 2560x1440p60 runs at 241.50 MHz and carries
/// `0x0600`, while 1920x1080p120 runs *faster* at 297.00 MHz and carries `0x0400`. That rules out
/// reading it as a DP link-bandwidth tier, which `docs/VIDEO.md` had suggested and which would put
/// both of those modes inside 4-lane HBR.
///
/// Measured: 1920x1080 at 60 and 120 Hz -> `0x0400`; 2560x1440 at 60 and 120 Hz -> `0x0600`.
/// The 1280x720p60 and 3840x2160p60 values predate the decrypted corpus but fall on the same
/// ladder.
///
/// ⚠ `vdisplay` tracks `hdisplay` in every sample, so nothing distinguishes a width ladder from
/// an area one; width is chosen because the steps land on standard widths.
fn link_word_42(hdisplay: u16) -> u16 {
    match hdisplay {
        0..=1920 => 0x0400,
        1921..=2560 => 0x0600,
        _ => 0x0604,
    }
}

/// Select the offset-66 mode word: a base in the high byte, the mode's CTA VIC in the low byte.
///
/// The low byte is the VIC, or zero for a timing that has none. Measured: `0x10` for 1920x1080p60
/// (VIC 16), `0x3f` for 1920x1080p120 (VIC 63), `0x00` for the VIC-less 2560x1440p60 and p120
/// CVT-RB timings.
///
/// The base is `0x2800` only for a mode that both has a VIC *and* runs at 60 Hz or below; every
/// other measured message uses `0x0800`. That is what separates 1920x1080p60 (`0x2810`) from
/// 2560x1440p60 (`0x0800`) -- same refresh, but the 1440p timing is CVT-RB and carries no VIC, so
/// refresh alone does not select the base.
///
/// ⚠ `0x2800` is observed in exactly one message and its meaning is undecoded, so treat any mode
/// that lands on it without a measurement as suspect first if a panel stays dark.
fn mode_word_66(mode: &kernel::drm::kms::modes::DisplayMode, refresh: u16) -> u16 {
    let vic = u16::from(mode.cea_vic()) & 0x00ff;
    let base: u16 = if vic != 0 && refresh <= 60 {
        0x2800
    } else {
        0x0800
    };
    base | vic
}

/// A mode's offset-42 and offset-66 set-mode words, and how they were obtained.
pub(super) struct ModeProfile {
    pub off42: u16,
    pub off66: u16,
    /// True when these bytes are reproduced from a decrypted DLM set-mode message.
    pub measured: bool,
}

/// Return the two mode-dependent set-mode words at offsets 42 and 66.
///
/// Modes that appear in a decrypted DLM message are reproduced byte-exactly. Everything else is
/// derived by [`link_word_42`] and [`mode_word_66`], which between them reproduce every measured
/// message, so the dock's vocabulary is no longer limited to the handful of timings that happen to
/// have been captured. The envelope DLM itself stays inside -- refresh ceiling, per-head clock and
/// the shared pixel budget -- is enforced by `drm_sink`'s `mode_valid`, not here.
pub(super) fn mode_profile(mode: &kernel::drm::kms::modes::DisplayMode) -> Option<ModeProfile> {
    let clock = mode.clock();
    if clock <= 0 {
        return None;
    }
    let refresh = mode.vrefresh();
    if refresh <= 0 {
        return None;
    }
    let refresh = refresh as u16;

    let measured = matches!(
        (
            clock,
            mode.hdisplay(),
            mode.hsync_start(),
            mode.hsync_end(),
            mode.htotal(),
            mode.vdisplay(),
            mode.vsync_start(),
            mode.vsync_end(),
            mode.vtotal(),
        ),
        // The whole decrypted DLM corpus: 1920x1080p60 and p120 (CTA), 2560x1440p60 and p120
        // (CVT-RB). The derivation reproduces all four byte-exactly, so these need no override --
        // the match only records that the bytes are backed by a capture.
        (148_500, 1920, 2008, 2052, 2200, 1080, 1084, 1089, 1125)
            | (297_000, 1920, 2008, 2052, 2200, 1080, 1084, 1089, 1125)
            | (241_500, 2560, 2608, 2640, 2720, 1440, 1443, 1448, 1481)
            | (497_750, 2560, 2608, 2640, 2720, 1440, 1443, 1448, 1525)
    );

    Some(ModeProfile {
        off42: link_word_42(mode.hdisplay()),
        off66: mode_word_66(mode, refresh),
        measured,
    })
}

/// Whether the dock can be given a mode profile for `mode`.
pub(super) fn mode_supported(mode: &kernel::drm::kms::modes::DisplayMode) -> bool {
    mode_profile(mode).is_some()
}

/// Set-mode (`id=0x48 sub=0x22`): an 80-byte inner message carrying the target head and a
/// detailed timing record. Offsets 26 through 46 contain geometry, link configuration, refresh and
/// flags; offset 66 is mode-dependent, offset 68 is fixed, offset 70 contains the pixel clock, and
/// offsets 74 through 79 contain a fresh token.
pub(super) fn set_mode(counter: u16, head: u8, t: &Timing) -> Result<KVec<u8>> {
    let mut b = KVec::with_capacity(80, GFP_KERNEL)?;
    header(&mut b, 0x48, 0x22, counter)?;
    pad_to(&mut b, 22)?;
    b.push(head, GFP_KERNEL)?; // off22: downstream head selector
    b.push(2, GFP_KERNEL)?; // off23: fixed generation/type value
    pad_to(&mut b, 26)?; // off24..25 zero; timing begins at off26
    for v in [
        t.hactive,
        t.hblank,
        t.hsync_front,
        t.hsync_width,
        t.vactive,
        t.vblank,
        t.vsync_front,
        t.vsync_width,
        t.field42,
        t.refresh_hz,
        0x4000,
        /* off46 flags */ 0x6000, /* off48 */
    ] {
        b.extend_from_slice(&v.to_le_bytes(), GFP_KERNEL)?;
    }
    pad_to(&mut b, 58)?;
    b.extend_from_slice(&0x0080u16.to_le_bytes(), GFP_KERNEL)?; // off58: profile constant
    b.extend_from_slice(&0x00ffu16.to_le_bytes(), GFP_KERNEL)?; // off60: profile constant
    pad_to(&mut b, 66)?;
    b.extend_from_slice(&t.off66.to_le_bytes(), GFP_KERNEL)?; // off66: see `mode_profile`
    b.extend_from_slice(&0x0200u16.to_le_bytes(), GFP_KERNEL)?; // off68: profile constant
    b.extend_from_slice(&t.pixel_clock_10khz.to_le_bytes(), GFP_KERNEL)?; // off70: 10 kHz units
    pad_to(&mut b, 74)?;
    let mut tail = [0u8; 6];
    rng::fill(&mut tail);
    b.extend_from_slice(&tail, GFP_KERNEL)?; // off74..79: fresh per-message token
    Ok(b)
}
/// Convert a DRM display mode into the dock's set-mode timing representation.
pub(super) fn timing_from_drm_mode(mode: &kernel::drm::kms::modes::DisplayMode) -> Result<Timing> {
    let refresh = mode.vrefresh() as u16;
    let sub = |a: u16, b: u16| a.saturating_sub(b);
    let profile = mode_profile(mode).ok_or(EINVAL)?;
    let clock = mode.clock();
    if clock <= 0 {
        return Err(EINVAL);
    }
    // A dark panel on a mode with no decrypted message is far more likely to be these two words
    // than anything else in the pipeline, so name them in the log.
    if !profile.measured {
        pr_info!(
            "vino: {}x{}@{} has no decrypted DLM profile; inferring off42={:#06x} off66={:#06x}\n",
            mode.hdisplay(),
            mode.vdisplay(),
            refresh,
            profile.off42,
            profile.off66
        );
    }
    let pixel_clock_10khz = u16::try_from((clock as u32) / 10).map_err(|_| EINVAL)?;
    Ok(Timing {
        hactive: mode.hdisplay(),
        hblank: sub(mode.htotal(), mode.hdisplay()),
        hsync_front: sub(mode.hsync_start(), mode.hdisplay()),
        hsync_width: sub(mode.hsync_end(), mode.hsync_start()),
        vactive: mode.vdisplay(),
        vblank: sub(mode.vtotal(), mode.vdisplay()),
        vsync_front: sub(mode.vsync_start(), mode.vdisplay()),
        vsync_width: sub(mode.vsync_end(), mode.vsync_start()),
        refresh_hz: refresh,
        pixel_clock_10khz,
        field42: profile.off42,
        off66: profile.off66,
    })
}
/// Known CP `sub` identifiers used to validate a decrypted header.
fn is_known_sub(sub: u16) -> bool {
    matches!(
        sub,
        0x00 | 0x04
            | 0x0b
            | 0x0c
            | 0x10
            | 0x20
            | 0x21
            | 0x22
            | 0x24
            | 0x25
            | 0x2a
            | 0x30
            | 0x31
            | 0x41
            | 0x42
            | 0x43
            | 0x45
            | 0x4a
            | 0x4b
            | 0x4c
            | 0x75
            | 0x84
            | 0x86
    )
}

/// Return the supported dock-to-host RIVs in reply-preference order.
///
/// The first pair uses the direction bit preferred by interactive replies. The second pair covers
/// firmware which replies using the outgoing RIV. Within each pair, byte 0 bit 7 selects the head.
fn inbound_reply_rivs(out_riv: &[u8; 8]) -> [[u8; 8]; 4] {
    let in_head0 = in_riv(out_riv);
    let mut in_head1 = in_head0;
    in_head1[0] ^= 0x80;
    let out_head0 = *out_riv;
    let mut out_head1 = out_head0;
    out_head1[0] ^= 0x80;
    [in_head0, in_head1, out_head0, out_head1]
}

/// Try the supported RIV variants and return the best-scoring inner header and prefix.
///
/// Interactive replies use [`in_riv`], while capability replies can use the outgoing RIV.
/// Flipping bit 7 of byte 0 selects the second head.
pub(super) fn decode_any(
    ks: &[u8; 16],
    out_riv: &[u8; 8],
    wire: &[u8],
) -> Option<(&'static str, u16, u16, u16, [u8; 24])> {
    if wire.len() <= 16 {
        return None;
    }
    let seq = u32::from_le_bytes([wire[12], wire[13], wire[14], wire[15]]);
    let head = &wire[16..wire.len().min(48)];
    let rivs = inbound_reply_rivs(out_riv);
    let variants: [(&'static str, [u8; 8]); 4] = [
        ("out/h0", rivs[2]),
        ("in/h0", rivs[0]),
        ("out/h1", rivs[3]),
        ("in/h1", rivs[1]),
    ];
    let mut best: Option<(i32, &'static str, u16, u16, u16, [u8; 24])> = None;
    for (tag, riv) in variants {
        let Ok(pt) = open_in(ks, &riv, seq, head) else {
            continue;
        };
        if pt.len() < 8 {
            continue;
        }
        let id = u16::from_le_bytes([pt[0], pt[1]]);
        let sub = u16::from_le_bytes([pt[2], pt[3]]);
        let ctr = u16::from_le_bytes([pt[4], pt[5]]);
        let pad = u16::from_le_bytes([pt[6], pt[7]]);
        let mut sc = 0i32;
        if is_known_sub(sub) {
            sc += 50;
        }
        if pad == 0 {
            sc += 10;
        }
        if ctr < 0x400 {
            sc += 5;
        }
        if best.map_or(true, |b| sc > b.0) {
            // Retain enough plaintext to identify the decoded message class.
            let mut sample = [0u8; 24];
            let n = pt.len().min(24);
            sample[..n].copy_from_slice(&pt[..n]);
            best = Some((sc, tag, id, sub, ctr, sample));
        }
    }
    best.map(|(_, tag, id, sub, ctr, sample)| (tag, id, sub, ctr, sample))
}
/// Verify a dock-to-host `sub=0x45` acknowledgment for the active session.
///
/// The wire tag alone is insufficient because status frames also use `sub=0x45`. A valid
/// acknowledgment must decrypt to a small id, a known sub-id and a zero header pad.
///
/// Firmware revisions use both the outgoing RIV and its byte-7-bit-0 variant for replies, with
/// byte-0-bit-7 selecting the head, so all four combinations are checked.
pub(super) fn verify_in_ack(
    ks: &[u8; 16],
    out_riv: &[u8; 8],
    wire: &[u8],
) -> Option<(u16, u16, u16)> {
    if wire.len() <= 16 || u16::from_le_bytes([wire[8], wire[9]]) != 0x45 {
        return None;
    }
    let seq = u32::from_le_bytes([wire[12], wire[13], wire[14], wire[15]]);
    let head = &wire[16..wire.len().min(32)];
    for riv in inbound_reply_rivs(out_riv) {
        let Ok(pt) = open_in(ks, &riv, seq, head) else {
            continue;
        };
        if pt.len() < 8 {
            continue;
        }
        let id = u16::from_le_bytes([pt[0], pt[1]]);
        let sub = u16::from_le_bytes([pt[2], pt[3]]);
        let ctr = u16::from_le_bytes([pt[4], pt[5]]);
        let pad = u16::from_le_bytes([pt[6], pt[7]]);
        if id < 0x400 && is_known_sub(sub) && pad == 0 {
            return Some((id, sub, ctr));
        }
    }
    None
}

/// Lenient sibling of [`verify_in_ack`] that also accepts uncatalogued sub-ids.
///
/// This distinguishes a valid message using a newly observed sub-id from a frame that cannot be
/// decrypted under any supported RIV variant.
pub(super) fn decode_in_lenient(
    ks: &[u8; 16],
    out_riv: &[u8; 8],
    wire: &[u8],
) -> Option<(u16, u16, u16)> {
    if wire.len() <= 16 || u16::from_le_bytes([wire[8], wire[9]]) != 0x45 {
        return None;
    }
    let seq = u32::from_le_bytes([wire[12], wire[13], wire[14], wire[15]]);
    let head = &wire[16..wire.len().min(32)];
    for riv in inbound_reply_rivs(out_riv) {
        let Ok(pt) = open_in(ks, &riv, seq, head) else {
            continue;
        };
        if pt.len() < 8 {
            continue;
        }
        let id = u16::from_le_bytes([pt[0], pt[1]]);
        let sub = u16::from_le_bytes([pt[2], pt[3]]);
        let ctr = u16::from_le_bytes([pt[4], pt[5]]);
        let pad = u16::from_le_bytes([pt[6], pt[7]]);
        if id < 0x400 && pad == 0 {
            return Some((id, sub, ctr));
        }
    }
    None
}
/// Extract the fresh per-head `rrx` from an `AKE_Send_rrx` push.
///
/// Each per-head repeater authentication supplies a distinct `rrx`. The derived `kd`, encryption
/// key and `V` must use this value rather than the main-link `rrx`, otherwise repeater
/// authentication and the downstream DDC path fail.
pub(super) fn perhead_rrx(ks: &[u8; 16], out_riv: &[u8; 8], wire: &[u8]) -> Option<[u8; 8]> {
    if wire.len() <= 16 || u16::from_le_bytes([wire[8], wire[9]]) != 0x45 {
        return None;
    }
    let seq = u32::from_le_bytes([wire[12], wire[13], wire[14], wire[15]]);
    let head = &wire[16..wire.len().min(48)];
    for riv in inbound_reply_rivs(out_riv) {
        let Ok(pt) = open_in(ks, &riv, seq, head) else {
            continue;
        };
        if pt.len() < 18 {
            continue;
        }
        let id = u16::from_le_bytes([pt[0], pt[1]]);
        let sub = u16::from_le_bytes([pt[2], pt[3]]);
        // id=0x10 sub=0x84 push, inner HDCP msg-id (byte 9) == AKE_SEND_RRX (0x06).
        if id == 0x10 && sub == 0x84 && pt[9] == 0x06 {
            let mut rrx = [0u8; 8];
            rrx.copy_from_slice(&pt[10..18]);
            return Some(rrx);
        }
    }
    None
}
// All three cursor messages share one 32-byte inner layout:
// off0..7 id/sub/counter header
// off8..21 zero
// off22 0x02 constant marker
// off23 head_id (0 / 1 across the cold-ref's two monitors)
// off24..25 field1 LE u16 (create: width / move: X / image: 0)
// off26..27 field2 LE u16 (create: height / move: Y / image: 0)
// off28..31 zero
// Cursor images append their w*h*4 BGRA bitmap at off32 and set the high-byte flag in id 0x401c.
/// The dock's head id at off22 of every cursor message, indexed by vino's head number.
///
/// Cursor wire layout (sec 8.6.1). All three messages share the 32-byte inner header built by
/// [`cursor_header`], with the head selector at off22 and a flag at off23.
///
/// The dock numbers its heads from **one**; `0` is not a valid selector.
const CURSOR_HEAD_IDS: [u8; 2] = [0x01, 0x02];

/// off23 is the cursor's **visible** flag, not a message-kind tag: set to show the cursor, clear to
/// hide it. The bitmap-bearing messages carry it clear because an upload is not itself a show.
const CURSOR_OFF23_VISIBLE: u8 = 0x01;
const CURSOR_OFF23_HIDDEN: u8 = 0x00;

fn cursor_header(
    b: &mut KVec<u8>,
    id: u16,
    sub: u16,
    counter: u16,
    off22: u8,
    off23: u8,
) -> Result {
    header(b, id, sub, counter)?;
    pad_to(b, 22)?;
    b.push(off22, GFP_KERNEL)?;
    b.push(off23, GFP_KERNEL)?;
    Ok(())
}

/// cursor create: `id=0x1b sub=0x42`, advertises `w x h`. Sent once per bitmap geometry.
pub(super) fn cursor_create(counter: u16, head: u8, w: u16, h: u16) -> Result<KVec<u8>> {
    let mut b = KVec::with_capacity(32, GFP_KERNEL)?;
    let dock_head = CURSOR_HEAD_IDS.get(head as usize).copied().ok_or(EINVAL)?;
    cursor_header(&mut b, 0x1b, 0x42, counter, dock_head, CURSOR_OFF23_HIDDEN)?;
    b.extend_from_slice(&w.to_le_bytes(), GFP_KERNEL)?; // off24..25
    b.extend_from_slice(&h.to_le_bytes(), GFP_KERNEL)?; // off26..27
    pad_to(&mut b, 32)?; // off28..31 reserved
    Ok(b)
}
/// cursor move: `id=0x1a sub=0x43`, X at off24 and Y at off26 (LE), for one head.
pub(super) fn cursor_move(
    counter: u16,
    head: u8,
    x: u16,
    y: u16,
    visible: bool,
) -> Result<KVec<u8>> {
    let mut b = KVec::with_capacity(32, GFP_KERNEL)?;
    let dock_head = CURSOR_HEAD_IDS.get(head as usize).copied().ok_or(EINVAL)?;
    let off23 = if visible {
        CURSOR_OFF23_VISIBLE
    } else {
        CURSOR_OFF23_HIDDEN
    };
    cursor_header(&mut b, 0x1a, 0x43, counter, dock_head, off23)?;
    b.extend_from_slice(&x.to_le_bytes(), GFP_KERNEL)?; // off24..25
    b.extend_from_slice(&y.to_le_bytes(), GFP_KERNEL)?; // off26..27
    pad_to(&mut b, 32)?; // off28..31 reserved
    Ok(b)
}
/// cursor image: inner `id=0x401c sub=0x41` (the `0x40` high-byte flag marks the bitmap-bearing
/// message), a 32-byte header then the bitmap. `w`/`h` come from [`cursor_create`].
///
/// Pixels are DRM `ARGB8888` (`[B, G, R, A]`, premultiplied) and **start at off34**: off32..33 are
/// zero and the final pixel is truncated, so the message stays `32 + w*h*4` bytes.
pub(super) fn cursor_image(
    counter: u16,
    head: u8,
    w: u16,
    h: u16,
    bgra: &[u8],
) -> Result<KVec<u8>> {
    // `w*h*4` can wrap a 32-bit `usize` (max ~1.7e10 > u32::MAX), which would let an
    // undersized `bgra` pass the check; compute it with checked arithmetic so an
    // overflow is rejected as a mismatch rather than silently bypassing validation.
    let expected = (w as usize)
        .checked_mul(h as usize)
        .and_then(|n| n.checked_mul(4));
    if expected != Some(bgra.len()) {
        return Err(EINVAL);
    }
    let mut b = KVec::with_capacity(32 + bgra.len(), GFP_KERNEL)?;
    let dock_head = CURSOR_HEAD_IDS.get(head as usize).copied().ok_or(EINVAL)?;
    cursor_header(&mut b, 0x401c, 0x41, counter, dock_head, CURSOR_OFF23_HIDDEN)?;
    pad_to(&mut b, 32)?; // off24..31 zero (no w/h here)
    b.extend_from_slice(&[0, 0], GFP_KERNEL)?; // off32..33
    b.extend_from_slice(&bgra[..bgra.len() - 2], GFP_KERNEL)?; // pixels @ off34
    Ok(b)
}
/// Compute the 16-byte DisplayLink Dl3Cmac control-message integrity tag:
/// `tag = AES-CMAC(ks, mac_nonce(8) || BE64(wire_seq) || ciphertext)` where
/// - `mac_nonce` = the AES-CTR content nonce (`riv`) with `byte0 ^= 0x80`. Pass the CTR `riv`
///   and this function applies the byte-0 transform.
/// - `wire_seq` = the AES-CTR block counter (frame header off-12), zero-extended to BE64,
/// - `ciphertext` = the AES-CTR ciphertext content (encrypt-then-MAC), tag appended IN CLEAR.
///
/// The Dl3Cmac key is the session key `ks`; the CTR and CMAC nonces differ by byte-0 bit 7.
pub(super) fn dl3cmac_tag(
    ks: &[u8; 16],
    riv: &[u8; 8],
    wire_seq: u64,
    ciphertext: &[u8],
) -> Result<[u8; 16]> {
    let mut mac_nonce = *riv;
    mac_nonce[0] ^= 0x80;
    let mut buf = KVec::with_capacity(16 + ciphertext.len(), GFP_KERNEL)?;
    buf.extend_from_slice(&mac_nonce, GFP_KERNEL)?;
    buf.extend_from_slice(&wire_seq.to_be_bytes(), GFP_KERNEL)?;
    buf.extend_from_slice(ciphertext, GFP_KERNEL)?;
    Ok(crypto::aes_cmac(ks, &buf))
}
/// Seal a CP message with AES-CTR followed by a freshly computed Dl3Cmac.
///
/// `content_pt` excludes the 16-byte tag. The clear wire header supplies the sequence counter.
pub(super) fn seal_livemac(
    ks: &[u8; 16],
    riv: &[u8; 8],
    header: &[u8],
    content_pt: &[u8],
) -> Result<KVec<u8>> {
    let seq = u32::from_le_bytes([header[12], header[13], header[14], header[15]]);
    let cipher = crypto::Aes128::new(ks)?;
    let mut ct = KVec::with_capacity(content_pt.len(), GFP_KERNEL)?;
    for (i, chunk) in content_pt.chunks(16).enumerate() {
        let mut iv = [0u8; 16];
        iv[..8].copy_from_slice(riv);
        iv[12..].copy_from_slice(&seq.wrapping_add(i as u32).to_be_bytes());
        let ksb = cipher.encrypt_block(&iv);
        for (j, &p) in chunk.iter().enumerate() {
            ct.push(p ^ ksb[j], GFP_KERNEL)?;
        }
    }
    let tag = dl3cmac_tag(ks, riv, seq as u64, &ct)?;
    let mut frame = KVec::with_capacity(16 + ct.len() + 16, GFP_KERNEL)?;
    frame.extend_from_slice(&header[..16], GFP_KERNEL)?;
    frame.extend_from_slice(&ct, GFP_KERNEL)?;
    frame.extend_from_slice(&tag, GFP_KERNEL)?;
    Ok(frame)
}
/// Build a fully sealed interactive CP frame (`type=4 sub=0x24`) at `wire_seq` over `content`
/// (the inner plaintext, WITHOUT any appended 16-byte tag placeholder): the 16-byte wire
/// header -- size, `type=4`, `sub=0x24`, the per-`id` [`aux_for_id`] field, and `wire_seq` --
/// followed by [`seal_livemac`] (AES-CTR ciphertext + appended live Dl3Cmac). Shared by the
/// bring-up live loop ([`VinoDriver::send_live_cp`]) and the runtime KMS senders
/// ([`drm_sink::VinoDrmData::send_cp`]) so both produce a byte-identical wire frame.
pub(super) fn seal_interactive(
    ks: &[u8; 16],
    riv: &[u8; 8],
    id: u16,
    wire_seq: u32,
    content: &[u8],
) -> Result<KVec<u8>> {
    let body_len = content.len() + 16; // AES-CTR ciphertext + 16-byte Dl3Cmac
    let size = ((16 + body_len) - 4) as u16;
    let aux = aux_for_id(id, body_len);
    let mut hdr = [0u8; 16];
    hdr[2..4].copy_from_slice(&size.to_le_bytes());
    hdr[4..8].copy_from_slice(&4u32.to_le_bytes()); // type=4
    hdr[8..10].copy_from_slice(&0x24u16.to_le_bytes()); // sub=0x24 (interactive CP)
    hdr[10..12].copy_from_slice(&aux.to_le_bytes());
    hdr[12..16].copy_from_slice(&wire_seq.to_le_bytes());
    seal_livemac(ks, riv, &hdr, content)
}
/// Return the wire-header auxiliary value for an inner message id.
///
/// Known ids use protocol constants rather than the message length. Unknown ids fall back to the
/// body length in dwords.
pub(super) fn aux_for_id(id: u16, body_len: usize) -> u16 {
    match id {
        0x14 => 0x0a,
        0x15 => 0x09,
        0x16 => 0x08,
        0x19 => 0x05,
        0x1a => 0x04, // cursor move
        0x1b => 0x03, // cursor create
        0x1c => 0x02, // cursor image
        0x1f => 0x0f,
        0x22 => 0x0c,
        0x26 => 0x08,
        0x2a => 0x04,
        0x32 => 0x0c,
        0x36 => 0x08, // DDC/CI write
        0x48 => 0x06,
        0x9a => 0x04,
        _ => (body_len / 4) as u16,
    }
}
/// Per-head downstream repeater authentication and stream-open sequence.
///
/// Each entry is `(id, sub, plaintext length)` before [`seal_interactive`] appends the Dl3Cmac.
/// The AKE entries carry the HDCP message id at offset 27 and its payload at offset 28. The driver
/// derives a self-consistent HDCP 2.2 chain independently for each head.
///
/// [`VinoDriver::send_cp_setup`]: super::VinoDriver::send_cp_setup
pub(super) const CP_SETUP_PER_HEAD: [(u16, u16, usize); 9] = [
    (0x0022, 0x0010, 48),  // AKE_Init -- msg-id 0x02 @off27, 20B random payload
    (0x001f, 0x0010, 48),  // AKE_Transmitter_Info -- msg-id 0x13, fixed 00 06 02 00 02 prefix
    (0x009a, 0x0010, 160), // AKE_No_Stored_km -- msg-id 0x04, 132B payload (10 AES blocks)
    (0x0022, 0x0010, 48),  // LC_Init -- msg-id 0x09 @off27, 20B random payload
    (0x0032, 0x0010, 64),  // per-head VIDEO KEY -- msg-id 0x0b, fresh 32B key @off28, stashed
    (0x002a, 0x0010, 48),  // LC_Send_L_prime -- msg-id 0x0f @off27, 20B random payload
    (0x0026, 0x0010, 48),  // RepeaterAuth Stream_Manage -- built by stream_manage_restatement
    (0x0014, 0x0030, 32),  // per-head stream-open ctl -- no marker/tag, 10B random @off22
    (0x0019, 0x0031, 32),  // per-head strm2 -- head @off22, fixed 06 [head*4] 04 @off24
];
/// Build a 48-byte `RepeaterAuth_Stream_Manage` restatement for one head.
///
/// Offset 23 contains the one-based head, offset 27 the HDCP message id, offsets 32 and 36 contain
/// the stream count and sequence, and offsets 40 through 47 contain a fresh token.
pub(super) fn stream_manage_restatement(counter: u16, head: u8) -> Result<KVec<u8>> {
    let mut b = KVec::from_elem(0u8, 48, GFP_KERNEL)?;
    b[0..2].copy_from_slice(&0x0026u16.to_le_bytes());
    b[2..4].copy_from_slice(&0x0010u16.to_le_bytes());
    b[4..6].copy_from_slice(&counter.to_le_bytes());
    b[23] = head + 1; // head marker
    b[27] = 0x10; // HDCP msg-id (RepeaterAuth_Stream_Manage)
    b[32..36].copy_from_slice(&1u32.to_le_bytes());
    b[36..40].copy_from_slice(&(head as u32 + 8).to_le_bytes()); // seq: 8 head0 / 9 head1
    let mut tail = [0u8; 8];
    rng::fill(&mut tail);
    b[40..48].copy_from_slice(&tail);
    Ok(b)
}
/// Stream-finalization sequence sent after both [`CP_SETUP_PER_HEAD`] blocks.
///
/// Each tuple is `(id, sub, value at offset 22)`. Finalization messages are 32 bytes, use
/// `0x01` at offset 23 for `sub=0x4c`, and end with a fresh token.
pub(super) const CP_SETUP_FINALIZE: [(u16, u16, u8); 6] = [
    (0x0016, 0x004c, 0),
    (0x0015, 0x004a, 0),
    (0x0016, 0x004c, 0),
    (0x0016, 0x004c, 1),
    (0x0015, 0x004a, 1),
    (0x0016, 0x004c, 1),
];

/// Video-channel arm sequence prepended to the first frame on each head's bulk endpoint.
///
/// Entries are `(wire type, head-0 sub-id, auxiliary value, body length)`; the head index is added
/// to the sub-id. Entries 0, 1, 4 and 5 are plaintext. Entries 6 and 7 are fixed type-4 records
/// containing a tag over an empty payload. Entries 2, 3, 8 and 9 are sealed with the per-head video
/// key and share one block-counter sequence. The final pair carries the decoder configuration.
///
/// The complete arm sequence and the first encoded frame must be submitted in one URB. Splitting
/// them leaves the video endpoint unarmed.
pub(super) const VIDEO_ARM_BURST: [(u32, u16, u16, usize); 10] = [
    (2, 0x0008, 0x0000, 16),   // #0 plaintext: body 08 00 06
    (2, 0x0018, 0x0000, 16),   // #1 plaintext: body 08 00 16
    (4, 0x0008, 0x000a, 16),   // #2 SEALED 16B, per-head video key, seq 0
    (4, 0x0018, 0x000a, 16),   // #3 SEALED 16B, per-head video key, seq 1
    (2, 0x0000, 0x0000, 16),   // #4 plaintext: body 00
    (2, 0x0010, 0x0000, 16),   // #5 plaintext: body 00 00 10
    (4, 0x0000, 0x0004, 16),   // #6 type=4 FIXED plaintext 0a 00 04 … (sub 0x00, unsealed)
    (4, 0x0010, 0x0004, 16),   // #7 type=4 FIXED plaintext 0a 00 04 … (sub 0x10, unsealed)
    (4, 0x0008, 0x000e, 1104), // #8 sealed decoder configuration, seq 2
    (4, 0x0018, 0x000e, 1104), // #9 sealed decoder configuration, seq 71
];

/// Build the fully-known 16-byte plaintext body for one of [`VIDEO_ARM_BURST`]'s `wire_type==2`
/// entries at table index `i`, for head `h`.
pub(super) fn video_arm_plaintext_body(i: usize, h: u16) -> [u8; 16] {
    let mut b = [0u8; 16];
    match i {
        0 => {
            b[0..2].copy_from_slice(&(0x0008u16 + h).to_le_bytes());
            b[2..4].copy_from_slice(&0x0006u16.to_le_bytes());
        }
        1 => {
            b[0..2].copy_from_slice(&(0x0008u16 + h).to_le_bytes());
            b[2..4].copy_from_slice(&0x0016u16.to_le_bytes());
        }
        4 => {
            b[0..2].copy_from_slice(&h.to_le_bytes());
        }
        5 => {
            b[0..2].copy_from_slice(&h.to_le_bytes());
            b[2..4].copy_from_slice(&0x0010u16.to_le_bytes());
        }
        // Entries 6 and 7 are type-4 records built directly by `build_arm_burst_buf`.
        _ => {}
    }
    b
}

/// Build a fixed 32-byte `wire_type=2` (plaintext) video-arm-burst frame: 16-byte header
/// (`size=0x1c`, `type=2`, `sub`, `aux=0`, `seq=0`) + the 16-byte `body`. Matches
/// [`VIDEO_ARM_BURST`]'s plaintext entries byte-exact.
pub(super) fn video_arm_plain_frame(sub: u16, body: &[u8; 16]) -> [u8; 32] {
    let mut f = [0u8; 32];
    f[2..4].copy_from_slice(&0x001cu16.to_le_bytes());
    f[4..8].copy_from_slice(&2u32.to_le_bytes());
    f[8..10].copy_from_slice(&sub.to_le_bytes());
    f[16..32].copy_from_slice(body);
    f
}

/// Build a sealed type-4 video-arm frame from its header fields and plaintext content.
pub(super) fn seal_video_arm(
    key: &[u8; 16],
    riv: &[u8; 8],
    sub: u16,
    aux: u16,
    seq: u32,
    content: &[u8],
) -> Result<KVec<u8>> {
    let body_len = content.len() + 16; // AES-CTR ciphertext + 16-byte Dl3Cmac
    let size = ((16 + body_len) - 4) as u16;
    let mut hdr = [0u8; 16];
    hdr[2..4].copy_from_slice(&size.to_le_bytes());
    hdr[4..8].copy_from_slice(&4u32.to_le_bytes()); // type=4
    hdr[8..10].copy_from_slice(&sub.to_le_bytes());
    hdr[10..12].copy_from_slice(&aux.to_le_bytes());
    hdr[12..16].copy_from_slice(&seq.to_le_bytes());
    seal_livemac(key, riv, &hdr, content)
}
/// Derive the primary dock-to-host CP RIV from the host-to-dock RIV.
///
/// The two directions differ by bit 0 of byte 7 on current dock firmware.
pub(super) fn in_riv(out_riv: &[u8; 8]) -> [u8; 8] {
    let mut riv = *out_riv;
    riv[7] ^= 0x01;
    riv
}
/// Decrypt a dock->host CP frame body (AES-CTR, the same keystream as [`seal`] but
/// keyed with the IN `riv`). `ct` is the ciphertext (wire bytes after the 16-byte
/// cleartext header); `seq` is the wire counter at wire offset 12.
pub(super) fn open_in(ks: &[u8; 16], in_riv: &[u8; 8], seq: u32, ct: &[u8]) -> Result<KVec<u8>> {
    let cipher = crypto::Aes128::new(ks)?;
    let mut pt = KVec::with_capacity(ct.len(), GFP_KERNEL)?;
    for (i, chunk) in ct.chunks(16).enumerate() {
        let mut iv = [0u8; 16];
        iv[..8].copy_from_slice(in_riv);
        iv[12..].copy_from_slice(&seq.wrapping_add(i as u32).to_be_bytes());
        let ksb = cipher.encrypt_block(&iv);
        for (j, &c) in chunk.iter().enumerate() {
            pt.push(c ^ ksb[j], GFP_KERNEL)?;
        }
    }
    Ok(pt)
}
/// Decrypt an EDID reply and return its complete base block and extensions.
///
/// EDID replies use wire `sub=0x45` and inner `id=0x194 sub=0x21`. The EDID starts at inner offset
/// 22 and its base-block extension count determines the returned length. All supported direction
/// and head RIV variants are checked.
pub(super) fn parse_edid_from_reply(
    ks: &[u8; 16],
    out_riv: &[u8; 8],
    wire: &[u8],
) -> Result<Option<KVec<u8>>> {
    // Wire header: [.. type@4 u32 .. sub@8 u16 .. seq@12 u32]; body at off16.
    if wire.len() <= 16 || u16::from_le_bytes([wire[8], wire[9]]) != 0x45 {
        return Ok(None);
    }
    let seq = u32::from_le_bytes([wire[12], wire[13], wire[14], wire[15]]);
    let body = &wire[16..];
    for riv in inbound_reply_rivs(out_riv) {
        let Ok(inner) = open_in(ks, &riv, seq, body) else {
            continue;
        };
        // Inner header: [id u16][sub u16][counter u16][00 00]; EDID payload at off22.
        const EDID_OFF: usize = 22;
        if inner.len() < EDID_OFF + 128 {
            continue;
        }
        let id = u16::from_le_bytes([inner[0], inner[1]]);
        let sub = u16::from_le_bytes([inner[2], inner[3]]);
        // Some firmware reports the EDID reply id without the high bit.
        if (id != 0x94 && id != 0x194) || sub != 0x21 {
            continue;
        }
        let edid = &inner[EDID_OFF..];
        // Validate the EDID base-block magic `00 FF FF FF FF FF FF 00`.
        const MAGIC: [u8; 8] = [0x00, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x00];
        if edid[..8] != MAGIC {
            continue;
        }
        let total = ((1 + edid[126] as usize) * 128).min(edid.len());
        let mut out = KVec::with_capacity(total, GFP_KERNEL)?;
        out.extend_from_slice(&edid[..total], GFP_KERNEL)?;
        return Ok(Some(out));
    }
    Ok(None)
}
/// Decode the downstream status carried by an EDID probe reply.
///
/// Returns the inner message id, the little-endian status at offsets 22 through 25 and the ready
/// bit at offset 26. `None` means no matching reply was decrypted.
pub(super) fn probe_reply_status(
    ks: &[u8; 16],
    out_riv: &[u8; 8],
    wire: &[u8],
) -> Option<(u16, u32, bool)> {
    if wire.len() <= 16 || u16::from_le_bytes([wire[8], wire[9]]) != 0x45 {
        return None;
    }
    let seq = u32::from_le_bytes([wire[12], wire[13], wire[14], wire[15]]);
    let body = &wire[16..];
    for riv in inbound_reply_rivs(out_riv) {
        let Ok(inner) = open_in(ks, &riv, seq, body) else {
            continue;
        };
        if inner.len() < 8 {
            continue;
        }
        let id = u16::from_le_bytes([inner[0], inner[1]]);
        let sub = u16::from_le_bytes([inner[2], inner[3]]);
        let pad = u16::from_le_bytes([inner[6], inner[7]]);
        if id >= 0x400 || pad != 0 {
            continue;
        }
        // Ignore unrelated traffic: only a downstream capability/EDID handler response or a
        // generic negative acknowledgment can answer this probe.
        if !(matches!(id, 0x44 | 0x78 | 0x194) && sub == 0x0020) && id != 0x14 {
            continue;
        }
        // A short generic ack (the `id=0x14` the dock sends when it cannot route the probe)
        // carries no status region at all; report zeros rather than refusing to decode, so the
        // caller still learns the id.
        let status = if inner.len() >= 26 {
            u32::from_le_bytes([inner[22], inner[23], inner[24], inner[25]])
        } else {
            0
        };
        let ready = inner.len() >= 27 && inner[26] & 0x80 != 0;
        return Some((id, status, ready));
    }
    None
}
/// Decode an EDID-readiness probe reply.
///
/// Inner offset 26 bit 7 indicates that the downstream DDC read has completed. `None` distinguishes
/// an unrelated or undecipherable frame from a matching reply that is not ready.
pub(super) fn edid_poll_ready(ks: &[u8; 16], out_riv: &[u8; 8], wire: &[u8]) -> Option<bool> {
    if wire.len() <= 16 || u16::from_le_bytes([wire[8], wire[9]]) != 0x45 {
        return None;
    }
    let seq = u32::from_le_bytes([wire[12], wire[13], wire[14], wire[15]]);
    let body = &wire[16..];
    for riv in inbound_reply_rivs(out_riv) {
        let Ok(inner) = open_in(ks, &riv, seq, body) else {
            continue;
        };
        if inner.len() < 27 {
            continue;
        }
        let id = u16::from_le_bytes([inner[0], inner[1]]);
        let sub = u16::from_le_bytes([inner[2], inner[3]]);
        if id != 0x44 || sub != 0x20 {
            continue;
        }
        return Some(inner[26] & 0x80 != 0);
    }
    None
}
