// SPDX-License-Identifier: GPL-2.0
//! Encrypted-control-plane message builders (the inner plaintext of the type=4
//! sub=0x24 AES-CTR frames) plus the AES-CTR `seal` that encrypts and frames them.
use super::*;

mod cursor;
mod edid;
mod mode;

pub(crate) use cursor::*;
pub(crate) use edid::*;
pub(crate) use mode::*;

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

/// Derive a stream's AES-CTR content nonce from the RIV its `SKE_Send_Eks` restatement
/// (`id=0x32`) delivered.
///
/// Byte 7 is xored with the stream's content-stream id: the value the stream's
/// `RepeaterAuth_Stream_Manage` restatement declares, which is also the wire `sub` of that
/// stream's control records. The control channel is stream `0x04`, Ridge's video streams are
/// `0x08 | connector`, and Navarro's are `(connector << 3) | 7`.
pub(super) fn stream_content_nonce(riv: &[u8; 8], stream_id: u16) -> [u8; 8] {
    let mut nonce = *riv;
    nonce[7] ^= stream_id as u8;
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
///   `2f(1) 2e(3)` -> mode-set -> `2f(1) 2e(0) 2f(1) 2e(0) 2f(0) 2e(0)`
///
/// Offset 22 selects the connector and offset 23 carries the state.
pub(super) fn stream_marker(counter: u16, connector: u8, sub: u16, state: u8) -> Result<KVec<u8>> {
    let mut b = KVec::with_capacity(32, GFP_KERNEL)?;
    header(&mut b, 0x16, sub, counter)?;
    pad_to(&mut b, 22)?;
    b.push(connector, GFP_KERNEL)?; // off22: downstream connector selector
    b.push(state, GFP_KERNEL)?; // off23: state byte
    let mut token = [0u8; 8];
    rng::fill(&mut token);
    b.extend_from_slice(&token, GFP_KERNEL)?;
    Ok(b)
}

pub(super) fn stream_commit(counter: u16, connector: u8) -> Result<KVec<u8>> {
    let mut b = KVec::with_capacity(32, GFP_KERNEL)?;
    header(&mut b, 0x16, 0x4c, counter)?;
    pad_to(&mut b, 22)?;
    b.push(if connector == 0 { 0 } else { 1 }, GFP_KERNEL)?; // off22: per-connector flag
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

/// DL7400 post-authentication state query (`id=0x15 sub=0x78`).
///
/// The authenticated same-day DLM transcript sends this exactly once after all four per-connector
/// authentication blocks and before the first `0x16/0x4c` finalizer. Its request has the ordinary
/// 32-byte random-tail shape; the dock replies `0x14/0x78` with state `2` at offset 22. The
/// handler's semantic name is not known, so keep the builder descriptive rather than assigning a
/// guessed protocol meaning to that state.
pub(super) fn post_auth_state_req(counter: u16) -> Result<KVec<u8>> {
    random_tail_msg(0x15, 0x0078, counter)
}
/// DL7400 real-time-clock synchronization (`id=0x1e sub=0x94`).
///
/// The ten-byte payload at offset 22 is a compact broken-down local time:
/// `[year LE16, month, day, hour, minute, second, weekday, yday LE16]`. The authenticated
/// A capture carrying Monday as weekday 1 and 214 as the zero-based day of year proves the last
/// three bytes are calendar fields rather than an opaque random tail.
pub(super) fn rtc_sync_req(
    counter: u16,
    unix_seconds: i64,
    utc_offset_minutes: i32,
) -> Result<KVec<u8>> {
    let local = unix_seconds.saturating_add(i64::from(utc_offset_minutes) * 60);
    let days = local.div_euclid(86_400);
    let second_of_day = local.rem_euclid(86_400);

    // Gregorian civil date from days since 1970-01-01 (Howard Hinnant's civil_from_days).
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 }.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let mut year = yoe + era * 400;
    let doy_march = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy_march + 2) / 153;
    let day = doy_march - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    if month <= 2 {
        year += 1;
    }
    if !(0..=u16::MAX as i64).contains(&year) {
        return Err(EINVAL);
    }
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let month_starts = [0u16, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334];
    let mut yday = month_starts[(month - 1) as usize] + day as u16 - 1;
    if leap && month > 2 {
        yday += 1;
    }
    let weekday = (days + 4).rem_euclid(7) as u8; // 1970-01-01 was Thursday (4).

    let mut b = KVec::with_capacity(32, GFP_KERNEL)?;
    header(&mut b, 0x001e, 0x0094, counter)?;
    pad_to(&mut b, 22)?;
    b.extend_from_slice(&[0u8; 10], GFP_KERNEL)?;
    b[22..24].copy_from_slice(&(year as u16).to_le_bytes());
    b[24] = month as u8;
    b[25] = day as u8;
    b[26] = (second_of_day / 3_600) as u8;
    b[27] = ((second_of_day % 3_600) / 60) as u8;
    b[28] = (second_of_day % 60) as u8;
    b[29] = weekday;
    b[30..32].copy_from_slice(&yday.to_le_bytes());
    Ok(b)
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

/// OUT `id=0x14 sub=0x0000`: an inner header and a fresh ten-byte token.
///
/// The first sealed message of a session, and on a dock that carries video on the control pipe
/// also the last message before the sinks are engaged. The token is host-random and the dock has
/// no way to validate it, so what the message states is the counter it carries.
pub(super) fn session_hello(counter: u16) -> [u8; 32] {
    let mut content = [0u8; 32];
    content[0..2].copy_from_slice(&0x0014u16.to_le_bytes());
    content[4..6].copy_from_slice(&counter.to_le_bytes());
    rng::fill(&mut content[22..32]);
    content
}

/// Pixel granularity the render stride is quantised to.
const STRIDE_ALIGN: u32 = 128;

/// Offset 42 is not a polarity field but a flags word, and DLM decodes every bit of it in its own
/// `setupVideo` log line. Read out of the bit tests around DLM 3.4.26 `0x576b26`, which select
/// between an empty string and one of these:
///
/// | bit | mask | DLM's name |
/// |---|---|---|
/// | 0 | `0x0001` | `Interlace` |
/// | 1 | `0x0002` | `Cross-connector synchronized` |
/// | 2 | `0x0004` | `Dual NIVO` |
/// | 3 | `0x0008` | `Just-in-time decode` |
/// | 5 | `0x0020` | `DSC On`/`DSC Off` |
/// | 6 | `0x0040` | `ST2084 colorspace used (HDR)` |
/// | 7 | `0x0080` | `SingleDisplayMode enabled` |
/// | 8 | `0x0100` | `Horizontal Sync Inverted` |
/// | 9 | `0x0200` | `Vertical Syncs Inverted` |
/// | 12 | `0x1000` | `ReducedQuantizationRange On`/`Off` |
/// | 14 | `0x4000` | `Enable Timing for Gamma` |
/// | 15 | `0x8000` | `(Disabled)` |
///
/// Bits 8, 9 and 15 land exactly where the decrypted corpus had already put them, which is what
/// makes the rest of the table trustworthy. Bits 4, 10, 11 and 13 are not logged; bit 10 is the
/// base below, always set and still unexplained.
///
/// Base bit of the offset-42 flags word, set in every message the corpus contains.
const SYNC_FLAGS_BASE: u16 = 0x0400;
/// `hSyncInv`: horizontal sync is active low.
const SYNC_FLAG_HSYNC_INV: u16 = 0x0100;
/// `vSyncInv`: vertical sync is active low.
const SYNC_FLAG_VSYNC_INV: u16 = 0x0200;
/// `ST2084 colorspace used (HDR)`: the connector's pixels are PQ-encoded rather than SDR.
///
/// This is the transfer-function selector that no capture could settle -- the Windows HDR A/B
/// corpus has a sealed control plane, and DLM's Linux build never toggled HDR on this hardware --
/// and it turns out not to need a capture at all. There is exactly one HDR flag: the colour
/// primaries are not carried here, because the dock derives the downstream infoframe itself.
const SYNC_FLAG_ST2084: u16 = 0x0040;
/// `Dual NIVO`: this connector's video endpoint is carrying a second connector's stream too.
///
/// The DL-7400 multiplexes four connectors onto two video bulk endpoints -- `0x08` owns connectors
/// {0, 2} and `0x0a` owns {1, 3} -- so any two monitors in sockets one apart share an endpoint.
/// The dock drives only one of the two streams unless both mode sets declare the sharing here.
/// DLM's `setupVideo` flag decode names this bit `Dual NIVO`, matching the `TiledNivoViewer`
/// strings in its binary.
const SYNC_FLAG_DUAL_NIVO: u16 = 0x0004;
/// The offset-42 word a teardown carries in place of any polarity.
const SYNC_FLAGS_TEARDOWN: u16 = 0x8000;

/// Picture aspect of CTA VICs 1 through 59, one bit per VIC: set for 16:9, clear for 4:3.
///
/// The CTA table pairs most timings, one 4:3 and one 16:9 over the same signal -- VIC 2 and 3 are
/// both 720x480p60, VIC 6 and 7 both 720x480i60 -- so the aspect cannot be recovered from the
/// timing and has to be carried per VIC.
const VIC_ASPECT_16_9: u64 = 0x055_575e_beaa_ed55c;

/// Offset-66 high byte: the mode's picture aspect ratio.
const ASPECT_16_9: u16 = 0x2800;
const ASPECT_4_3: u16 = 0x1800;
/// Sent for a timing with no CTA VIC, which has no CTA aspect to name.
const ASPECT_NONE: u16 = 0x0800;

/// Offset-68 of the `0x48/0x22` message: the colour depth, in the high byte.
///
/// The dock takes a depth enum, not a bit count: 16bpp is 1, 24bpp 2, 30bpp 3, 36bpp 4 and 48bpp
/// 5, and an unrecognised depth falls back to 24bpp. The low byte is a separate field that every
/// capture carries as zero. The three values above 24bpp are 10, 12 and 16 bits per channel --
/// the deep-colour ladder -- and vino drives none of them.
const COLOUR_DEPTH_24BPP: u16 = 0x0200;
/// Offset-68 for 30 bpp: the same enum, one step up the deep-colour ladder (10 bits per channel).
const COLOUR_DEPTH_30BPP: u16 = 0x0300;

/// Offset-23 of the `0x48/0x22` message: the DMA buffer format the connector scans out.
///
/// The dock indexes a four-entry table with this, giving 2, 4, 3 and 4 bytes per pixel for formats
/// 0 through 3, and rejects anything above 3. DLM names all four: the same value selects a string
/// in the helper at 3.4.26 `0x62ecb0`, whose four arms point at the plaintext `NM16`, `NM32`,
/// `NM24` and `NM30`, and the bytes-per-pixel table at `0x8dc320` reads `{2, 4, 3, 4}` in exactly
/// that order.
///
/// | value | name | bytes/px |
/// |---|---|---|
/// | 0 | `NM16` | 2 |
/// | 1 | `NM32` | 4 |
/// | 2 | `NM24` | 3 |
/// | 3 | `NM30` | 4 |
///
/// A teardown writes no timing at all and leaves the field zero.
const DMA_FORMAT_NM24: u8 = 2;
const DMA_FORMAT_NONE: u8 = 0;
/// Offset-23 for a 10-bit connector: `NM30`, the second of the table's two four-byte formats.
///
/// No capture on either dock generation carries anything but `NM24`, so the name has to settle
/// the choice between the table's two four-byte formats: 30 bits per pixel packed into four bytes
/// is what a 2:10:10:10 sample is, and `NM32` is the 8-bit-with-padding format vino has no use for.
const DMA_FORMAT_NM30: u8 = 3;

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
/// firmware which replies using the outgoing RIV. Within each pair, byte 0 bit 7 selects the
/// connector.
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
/// Flipping bit 7 of byte 0 selects the second connector.
pub(super) fn decode_any(
    ks: &[u8; 16],
    out_riv: &[u8; 8],
    wire: &[u8],
) -> Option<(&'static str, u16, u16, u16, [u8; 24])> {
    if wire.len() <= 16 {
        return None;
    }
    let seq = u32::from_le_bytes([wire[12], wire[13], wire[14], wire[15]]);
    let body = &wire[16..];
    let rivs = inbound_reply_rivs(out_riv);
    let variants: [(&'static str, [u8; 8]); 4] = [
        ("out/h0", rivs[2]),
        ("in/h0", rivs[0]),
        ("out/h1", rivs[3]),
        ("in/h1", rivs[1]),
    ];
    let mut best: Option<(i32, &'static str, u16, u16, u16, [u8; 24])> = None;
    for (tag, riv) in variants {
        let Ok(plaintext) = open_in(ks, &riv, seq, body) else {
            continue;
        };
        if plaintext.len() < 8 {
            continue;
        }
        let id = u16::from_le_bytes([plaintext[0], plaintext[1]]);
        let sub = u16::from_le_bytes([plaintext[2], plaintext[3]]);
        let ctr = u16::from_le_bytes([plaintext[4], plaintext[5]]);
        let pad = u16::from_le_bytes([plaintext[6], plaintext[7]]);
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
            let n = plaintext.len().min(24);
            sample[..n].copy_from_slice(&plaintext[..n]);
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
/// byte-0-bit-7 selecting the connector, so all four combinations are checked.
pub(super) fn verify_in_ack(
    ks: &[u8; 16],
    out_riv: &[u8; 8],
    wire: &[u8],
) -> Option<(u16, u16, u16)> {
    if wire.len() <= 16 || u16::from_le_bytes([wire[8], wire[9]]) != 0x45 {
        return None;
    }
    let seq = u32::from_le_bytes([wire[12], wire[13], wire[14], wire[15]]);
    let body = &wire[16..];
    for riv in inbound_reply_rivs(out_riv) {
        let Ok(plaintext) = open_in(ks, &riv, seq, body) else {
            continue;
        };
        if plaintext.len() < 8 {
            continue;
        }
        let id = u16::from_le_bytes([plaintext[0], plaintext[1]]);
        let sub = u16::from_le_bytes([plaintext[2], plaintext[3]]);
        let ctr = u16::from_le_bytes([plaintext[4], plaintext[5]]);
        let pad = u16::from_le_bytes([plaintext[6], plaintext[7]]);
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
/// Recover a dock->host frame's inner plaintext, whichever framing it used.
///
/// Ridge seals every reply as wire `sub=0x45`. Navarro also pushes frames framed in the clear as
/// wire `sub=0x25`, with the inner message at offset 16 and nothing to decrypt.
pub(super) fn inner_plaintext(ks: &[u8; 16], out_riv: &[u8; 8], wire: &[u8]) -> Option<KVec<u8>> {
    if wire.len() <= 16 {
        return None;
    }
    match u16::from_le_bytes([wire[8], wire[9]]) {
        0x25 => {
            let mut plaintext = KVec::with_capacity(wire.len() - 16, GFP_KERNEL).ok()?;
            plaintext.extend_from_slice(&wire[16..], GFP_KERNEL).ok()?;
            Some(plaintext)
        }
        0x45 => {
            let seq = u32::from_le_bytes([wire[12], wire[13], wire[14], wire[15]]);
            for riv in inbound_reply_rivs(out_riv) {
                let Ok(plaintext) = open_in(ks, &riv, seq, &wire[16..]) else {
                    continue;
                };
                // The verified Dl3Cmac identifies a genuine frame. Inner offsets 6..7 must not be
                // tested as padding: Navarro stores connector selector bits there for the third
                // and fourth per-connector HDCP bursts, and rejecting on them dropped those
                // connectors' authentic pushes.
                if plaintext.len() >= 8 {
                    return Some(plaintext);
                }
            }
            None
        }
        _ => None,
    }
}

/// The dock's own log line carried by a `sub=0x0c` push, as printable ASCII.
///
/// The dock reports what it is doing, and what it refuses, on this channel. Recovering it costs
/// one pass over an already-decrypted frame and is the only account of a fault the dock does not
/// otherwise report.
pub(super) fn dock_trace_line(inner: &[u8]) -> Option<KVec<u8>> {
    if inner.len() < 10 || u16::from_le_bytes([inner[2], inner[3]]) != 0x000c {
        return None;
    }
    let mut out = KVec::new();
    for &b in &inner[8..] {
        if b == 0 {
            continue;
        }
        if !(0x20..0x7f).contains(&b) {
            continue;
        }
        out.push(b, GFP_KERNEL).ok()?;
    }
    if out.len() < 4 {
        return None;
    }
    Some(out)
}

pub(super) fn decode_in_lenient(
    ks: &[u8; 16],
    out_riv: &[u8; 8],
    wire: &[u8],
) -> Option<(u16, u16, u16)> {
    if wire.len() <= 16 || u16::from_le_bytes([wire[8], wire[9]]) != 0x45 {
        return None;
    }
    let seq = u32::from_le_bytes([wire[12], wire[13], wire[14], wire[15]]);
    let body = &wire[16..];
    for riv in inbound_reply_rivs(out_riv) {
        let Ok(plaintext) = open_in(ks, &riv, seq, body) else {
            continue;
        };
        if plaintext.len() < 8 {
            continue;
        }
        let id = u16::from_le_bytes([plaintext[0], plaintext[1]]);
        let sub = u16::from_le_bytes([plaintext[2], plaintext[3]]);
        let ctr = u16::from_le_bytes([plaintext[4], plaintext[5]]);
        // Navarro's device-log/status replies use session-varying IDs beyond the old catalogued
        // range (the captured transaction boundary replies with id=0x0405/sub=0x000c). Its
        // per-connector HDCP pushes also use bytes 4--7 as a one-hot 32-bit selector, so `ctr` is
        // only an echo counter for actual request/reply classes and bytes 6..7 need not be zero --
        // `open_in` has already authenticated the whole ciphertext, so no plaintext plausibility
        // restriction is needed or wanted here.
        return Some((id, sub, ctr));
    }
    None
}
/// One decoded downstream-HDCP push carried inside the interactive control session.
///
/// The vendor wrapper pads all of the short HDCP messages to a fixed inner size, so callers must
/// interpret the payload according to `msg_id`; `payload_len` is the available padded region, not
/// a claim that every byte belongs to the HDCP message.  The largest value needed by the current
/// authentication verifier is H'/L'/M' (32 bytes).
#[derive(Clone, Copy)]
pub(super) struct PerheadHdcpPush {
    pub msg_id: u8,
    pub payload: [u8; 38],
    pub payload_len: usize,
}

/// Decode a per-connector HDCP push from either of the two observed vendor framings.
///
/// Ridge can send the inner body directly in `wsub=0x25`; Navarro seals it as `wsub=0x45` with the
/// live control key. One parser covers both, so L', ReceiverID/V', receiver-auth status and M' are
/// decoded alongside Rrx rather than falling through as generic traffic.
pub(super) fn per_connector_hdcp_push(
    ks: &[u8; 16],
    out_riv: &[u8; 8],
    wire: &[u8],
) -> Option<PerheadHdcpPush> {
    if wire.len() <= 16 {
        return None;
    }
    const SUB_HDCP_RESP: u16 = 0x25;
    const SUB_SEALED: u16 = 0x45;
    let wsub = u16::from_le_bytes([wire[8], wire[9]]);

    let copy_push = |inner: &[u8]| -> Option<PerheadHdcpPush> {
        if inner.len() < 10 {
            return None;
        }
        let sub = u16::from_le_bytes([inner[2], inner[3]]);
        if sub != 0x84 {
            return None;
        }
        let src = &inner[10..];
        let n = src.len().min(38);
        let mut payload = [0u8; 38];
        payload[..n].copy_from_slice(&src[..n]);
        Some(PerheadHdcpPush {
            msg_id: inner[9],
            payload,
            payload_len: n,
        })
    };

    if wsub == SUB_HDCP_RESP {
        return copy_push(&wire[16..]);
    }
    if wsub != SUB_SEALED {
        return None;
    }
    let seq = u32::from_le_bytes([wire[12], wire[13], wire[14], wire[15]]);
    let body = &wire[16..];
    for riv in inbound_reply_rivs(out_riv) {
        let Ok(inner) = open_in(ks, &riv, seq, body) else {
            continue;
        };
        if let Some(push) = copy_push(&inner) {
            return Some(push);
        }
    }
    None
}

// All three cursor messages share one 32-byte inner layout:
// off0..7 id/sub/counter header
// off8..21 zero
// off22 0x02 constant marker
// off23 connector_id (0 / 1 across the cold-ref's two monitors)
// off24..25 field1 LE u16 (create: width / move: X / image: 0)
// off26..27 field2 LE u16 (create: height / move: Y / image: 0)
// off28..31 zero
// Cursor images append their w*h*4 BGRA bitmap at off32 and set the high-byte flag in id 0x401c.

/// off23 is the cursor's visible flag, not a message-kind tag: set to show the cursor, clear to
/// hide it. The bitmap-bearing messages carry it clear because an upload is not itself a show.
/// Offset-23 visibility flag of the cursor messages.
const CURSOR_VISIBLE: u8 = 0x01;
const CURSOR_HIDDEN: u8 = 0x00;

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
        0x1e => 0x00, // Navarro RTC synchronization
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
/// Per-connector downstream repeater authentication and stream-open sequence.
///
/// Each entry is `(id, sub, plaintext length)` before [`seal_interactive`] appends the Dl3Cmac.
/// The AKE entries carry the HDCP message id at offset 27 and its payload at offset 28. The driver
/// derives a self-consistent HDCP 2.2 chain independently for each connector.
///
/// [`VinoDriver::send_cp_setup`]: super::VinoDriver::send_cp_setup
pub(super) const CP_SETUP_PER_HEAD: [(u16, u16, usize); 9] = [
    (0x0022, 0x0010, 48),  // AKE_Init -- msg-id 0x02 @off27, 20B random payload
    (0x001f, 0x0010, 48),  // AKE_Transmitter_Info -- msg-id 0x13, fixed 00 06 02 00 02 prefix
    (0x009a, 0x0010, 160), // AKE_No_Stored_km -- msg-id 0x04, 132B payload (10 AES blocks)
    (0x0022, 0x0010, 48),  // LC_Init -- msg-id 0x09 @off27, 20B random payload
    (0x0032, 0x0010, 64),  // per-connector VIDEO KEY -- msg-id 0x0b, fresh 32B key @off28, stashed
    (0x002a, 0x0010, 48),  // LC_Send_L_prime -- msg-id 0x0f @off27, 20B random payload
    // RepeaterAuth_Stream_Manage -- built by `stream_manage_restatement`.
    (0x0026, 0x0010, 48),
    (0x0014, 0x0030, 32), // per-connector stream-open ctl -- no marker/tag, 10B random @off22
    (0x0019, 0x0031, 32), // per-connector strm2 -- connector @off22, fixed 06 [connector*4] 04 @off24
];
/// Layout of a restatement record, as the vendor's own message assembler writes it.
///
/// It allocates the record, stores a connector selector as a `u32`, a flag byte, and then copies
/// an HDCP message -- its id byte first, its payload after -- to a fixed offset. Naming the four
/// positions once keeps every builder below describing the same record rather than each repeating
/// a different set of literals.
pub(super) mod restatement {
    /// `u32` connector selector. The upstream authentication uses `0x30`; a downstream connector
    /// uses its one-based index, which puts `1` or `2` in the selector's second byte.
    pub(super) const SELECTOR: usize = 22;
    /// The HDCP message id, the first byte of the copied message.
    pub(super) const HDCP_ID: usize = 27;
    /// The HDCP payload, everything the message carries after its id.
    pub(super) const PAYLOAD: usize = 28;

    /// How far an HDCP message with a `payload_len`-byte payload reaches into the record.
    ///
    /// This is where the message *ends*, not how long the record is. The vendor assembles the
    /// message into an allocation of exactly this size and then sends it inside a larger fixed
    /// record, so everything past this offset is untouched allocation -- on its side heap
    /// metadata, on ours a fresh token. The record length itself is per message class and comes
    /// from the wire.
    pub(super) const fn message_end(payload_len: usize) -> usize {
        PAYLOAD + payload_len
    }
}

/// Build a `RepeaterAuth_Stream_Manage` restatement for one connector.
///
/// The payload is the HDCP one: a zero `seq_num_M`, a stream count, and that many content-stream
/// ids. One stream per connector, so the record ends after the first id -- there is nothing after
/// it to fill, and appending anything makes the record longer than the message it carries.
pub(super) fn stream_manage_restatement(
    counter: u16,
    connector: u8,
    stream_id: u16,
    onehot: bool,
) -> Result<KVec<u8>> {
    use restatement::*;
    // seq_num_M, stream count, one stream id.
    const PAYLOAD_LEN: usize = 4 + 4 + 4;
    // The record is 48 bytes on the wire whatever the message inside it needs; the vendor's own
    // is the same size and carries whatever its allocation held past `message_end`.
    let mut b = KVec::from_elem(0u8, 48, GFP_KERNEL)?;
    b[0..2].copy_from_slice(&0x0026u16.to_le_bytes());
    b[2..4].copy_from_slice(&0x0010u16.to_le_bytes());
    b[4..6].copy_from_slice(&counter.to_le_bytes());
    connector_marker(&mut b, connector, onehot);
    b[HDCP_ID] = ake::id::REPEATERAUTH_STREAM_MANAGE;
    // `seq_num_M` stays zero at PAYLOAD..PAYLOAD + 4.
    b[PAYLOAD + 4..PAYLOAD + 8].copy_from_slice(&1u32.to_le_bytes());
    b[PAYLOAD + 8..PAYLOAD + 12].copy_from_slice(&u32::from(stream_id).to_le_bytes());
    let mut past_message = [0u8; 48 - message_end(PAYLOAD_LEN)];
    rng::fill(&mut past_message);
    b[message_end(PAYLOAD_LEN)..].copy_from_slice(&past_message);
    Ok(b)
}

/// Write a per-connector record's connector selector.
///
/// Ridge names the connector by a one-based connector number at offset 23. Navarro sets a one-hot
/// bit at offset `22 + connector`, which is why it can address four connectors where Ridge
/// addresses two.
pub(super) fn connector_marker(content: &mut [u8], connector: u8, onehot: bool) {
    if onehot {
        if let Some(byte) = content.get_mut(restatement::SELECTOR + connector as usize) {
            *byte = 0x80;
        }
    } else if let Some(byte) = content.get_mut(restatement::SELECTOR + 1) {
        *byte = connector + 1;
    }
}
/// Stream-finalization sequence sent after both [`CP_SETUP_PER_HEAD`] blocks.
///
/// Each tuple is `(id, sub, value at offset 22)`. Finalization messages are 32 bytes, use
/// `0x01` at offset 23 for `sub=0x4c`, and end with a fresh token.
pub(super) const CP_SETUP_FINALIZE_STEPS: [(u16, u16); 3] =
    [(0x0016, 0x004c), (0x0015, 0x004a), (0x0016, 0x004c)];

/// Video-channel arm sequence prepended to the first frame on each connector's bulk endpoint.
///
/// Entries are `(wire type, connector-0 sub-id, auxiliary value, body length)`; the connector index
/// is added to the sub-id. Entries 0, 1, 4 and 5 are plaintext. Entries 6 and 7 are fixed type-4
/// records containing a tag over an empty payload. Entries 2, 3, 8 and 9 are sealed with the
/// per-connector video key and share one block-counter sequence. The final pair carries the decoder
/// configuration.
///
/// The complete arm sequence and the first encoded frame must be submitted in one URB. Splitting
/// them leaves the video endpoint unarmed.
pub(super) const VIDEO_ARM_BURST: [(u32, u16, u16, usize); 10] = [
    (2, 0x0008, 0x0000, 16),   // #0 plaintext: body 08 00 06
    (2, 0x0018, 0x0000, 16),   // #1 plaintext: body 08 00 16
    (4, 0x0008, 0x000a, 16),   // #2 SEALED 16B, per-connector video key, seq 0
    (4, 0x0018, 0x000a, 16),   // #3 SEALED 16B, per-connector video key, seq 1
    (2, 0x0000, 0x0000, 16),   // #4 plaintext: body 00
    (2, 0x0010, 0x0000, 16),   // #5 plaintext: body 00 00 10
    (4, 0x0000, 0x0004, 16),   // #6 type=4 FIXED plaintext 0a 00 04 ... (sub 0x00, unsealed)
    (4, 0x0010, 0x0004, 16),   // #7 type=4 FIXED plaintext 0a 00 04 ... (sub 0x10, unsealed)
    (4, 0x0008, 0x000e, 1104), // #8 sealed decoder configuration, seq 2
    (4, 0x0018, 0x000e, 1104), // #9 sealed decoder configuration, seq 71
];

/// Build the fully-known 16-byte plaintext body for one of [`VIDEO_ARM_BURST`]'s `wire_type==2`
/// entries at table index `i`, for connector `h`.
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
    super::video::haar::record_header(&mut f, 2, sub, 0, 0);
    f[16..32].copy_from_slice(body);
    f
}

/// Build the plaintext record that announces one stream or video plane to the dock.
///
/// The body names the `sub` a second time and carries a marker: 6 on a connector's content-stream
/// id, 0 on its video `sub`. Every generation sends this pair; they differ only in when. A dock
/// with a video pipe of its own takes them immediately ahead of the first frame, and a dock that
/// shares its control pipe takes them during CP setup, interleaved with the per-connector blocks.
pub(super) fn stream_announce(sub: u16, marker: u16) -> [u8; 32] {
    let mut body = [0u8; 16];
    body[0..2].copy_from_slice(&sub.to_le_bytes());
    body[2..4].copy_from_slice(&marker.to_le_bytes());
    video_arm_plain_frame(sub, &body)
}

/// The marker a record announcing a content stream carries; see [`stream_announce`].
pub(super) const STREAM_ANNOUNCE_MARKER: u16 = 6;

/// Build a sealed type-4 video-arm frame from its header fields and plaintext content.
/// The fixed 14-byte stream marker that opens every Navarro video stream record.
///
/// It is not a normal CP header. The connector is carried solely by the *wire* sub, never here:
/// all four connectors send these same fourteen bytes.
pub(super) const NAVARRO_STREAM_MARKER: [u8; 14] = [
    0x04, 0x00, 0x08, 0x04, 0x05, 0x00, 0x06, 0x00, 0x07, 0x01, 0x08, 0x02, 0x07, 0x00,
];

/// Build the 16-byte plaintext of a Navarro video stream-open, sent once per connector on that
/// connector's video endpoint before any pixels.
///
/// The content is [`NAVARRO_STREAM_MARKER`] followed by a two-byte opaque tail. The tail is host
/// random and differs between observed opens; it is covered by the Dl3Cmac, so its length matters
/// and its value does not.
pub(super) fn navarro_stream_open() -> [u8; 16] {
    let mut open = [0u8; 16];
    open[..14].copy_from_slice(&NAVARRO_STREAM_MARKER);
    rng::fill(&mut open[14..]);
    open
}

/// Build the 16-byte plaintext that opens a connector's sealed video stream on a dock whose marker
/// is six bytes long.
///
/// The first four bytes are shared with [`NAVARRO_STREAM_MARKER`]; `kind` is the fifth, and is the
/// only part that differs between generations. The rest is a host-random token, which the dock
/// cannot validate but which the Dl3Cmac covers, so its length is what matters.
pub(super) fn stream_open(kind: u8) -> [u8; 16] {
    let mut open = [0u8; 16];
    open[..6].copy_from_slice(&[0x04, 0x00, 0x08, 0x04, kind, 0x00]);
    rng::fill(&mut open[6..]);
    open
}

/// Build the 32-byte plaintext of a per-frame stream report that carries nothing but the mode.
///
/// A dock that shares its control pipe restates the mode on every report rather than only around a
/// mode change, and has no equivalent of the DL7400's longer report body.
pub(super) fn stream_report_mode_only(mode_header: &[u8; 26]) -> [u8; 32] {
    let mut out = [0u8; 32];
    out[..26].copy_from_slice(mode_header);
    rng::fill(&mut out[26..]);
    out
}

/// Fixed leader of one slot record in a DL7400 pipe descriptor, observed at 2560x1440.
const NAVARRO_SLOT_HEADER: [u8; 12] = [
    0x00, 0x10, 0xb4, 0x00, 0x14, 0x00, 0x00, 0x40, 0x01, 0x00, 0x00, 0x00,
];

/// Fixed trailer of one slot record.
const NAVARRO_SLOT_TRAILER: [u8; 10] = [0x00, 0x00, 0x00, 0x00, 0x00, 0x50, 0x00, 0x80, 0x01, 0x09];

/// Slot records per connector, and the connector stride in the dock's slot-id space.
const NAVARRO_SLOTS_PER_CONNECTOR: u16 = 6;
const NAVARRO_SLOT_STRIDE: u16 = 8;

/// Dock-side addresses each slot record names, as `base - n * step`.
///
/// The ring index counts in slot ids, so it skips the two ids each connector leaves unused; the
/// two CFB pools count in allocated slots and do not. Both forms are fixed by twelve records
/// across two independently keyed connectors.
const NAVARRO_RING_BASE: u32 = 0x6fcc;
const NAVARRO_RING_STEP: u32 = 0x21c;
const NAVARRO_CFB0_BASE: u32 = 0x71fb_9000;
const NAVARRO_CFB0_STEP: u32 = 0x5000;
const NAVARRO_CFB1_BASE: u32 = 0x7216_6000;
const NAVARRO_CFB1_STEP: u32 = 0x8000;

/// The dock's slot id for one of a connector's pipe buffers.
pub(super) fn navarro_pipe_slot(connector: u8, index: u16) -> u16 {
    (connector as u16) * NAVARRO_SLOT_STRIDE + index
}

/// The ring address a connector's pipe buffer is given.
pub(super) fn navarro_pipe_ring(connector: u8, index: u16) -> u32 {
    NAVARRO_RING_BASE - u32::from(navarro_pipe_slot(connector, index)) * NAVARRO_RING_STEP
}

/// The quiescent body of a DL7400 per-frame stream report, as `[len=0x0052][kind=0x000a]` and
/// thirty-five `u16` values.
///
/// DLM sends one of these on a connector's *stream* sub for every frame it sends on the frame sub
/// -- 165 and 306 of them across a 4.3 s and a 4.7 s session, a median 9-19 ms apart and never
/// more than ~1.0 s apart. vino sent none, and the dock tore the link down a few seconds after
/// its first frame.
///
/// The five-value preamble (`1, 1, 0, 64, 64`) and the trailing zero are fixed. The thirty
/// values between them are three blocks of three `(a, a, b)` triples separated by `(1, 1, 1)`,
/// where the third triple of each block carries twice the `a` of the first two. These are the
/// values DLM sends on a quiescent stream, identical on both connectors in both captures; under
/// load `a` and `b` grow with the frame's cost, but the mapping from a frame to them is not
/// established, so this reports the quiescent set.
const NAVARRO_STREAM_REPORT: [u16; 42] = [
    0x0052, 0x000a, // len, kind
    1, 1, 0, 64, 64, // fixed preamble
    16, // per-report scalar: 16 quiescent, larger under load
    16, 16, 16, 16, 16, 16, 32, 32, 32, // block A
    1, 1, 1, //
    16, 16, 4, 16, 16, 4, 32, 32, 8, // block B
    1, 1, 1, //
    32, 32, 2, 32, 32, 2, 64, 64, 4, // block C
    0,
];

/// Build the 84-byte body shared by both forms of the DL7400 per-frame stream report.
fn navarro_stream_report_body(out: &mut [u8; 84]) {
    for (i, v) in NAVARRO_STREAM_REPORT.iter().enumerate() {
        out[i * 2..i * 2 + 2].copy_from_slice(&v.to_le_bytes());
    }
}

/// Build the 96-byte plaintext of the DL7400's ordinary per-frame stream report (`aux=0x000c`).
///
/// The report body followed by a 12-byte host-random tail. This is the form DLM sends for all but
/// a handful of frames: 159 of 164 on one connector, 304 of 306 on the other.
pub(super) fn navarro_stream_report() -> [u8; 96] {
    let mut out = [0u8; 96];
    let mut body = [0u8; 84];
    navarro_stream_report_body(&mut body);
    out[..84].copy_from_slice(&body);
    rng::fill(&mut out[84..]);
    out
}

/// Build the 112-byte plaintext of the DL7400's mode-restating stream report (`aux=0x0002`).
///
/// The same body, prefixed by the 26-byte mode header that also opens the decoder configuration,
/// and followed by a two-byte host-random tail. DLM sends this form only a handful of times per
/// session, around a mode change.
pub(super) fn navarro_stream_report_mode(mode_header: &[u8; 26]) -> [u8; 112] {
    let mut out = [0u8; 112];
    out[..26].copy_from_slice(mode_header);
    let mut body = [0u8; 84];
    navarro_stream_report_body(&mut body);
    out[26..110].copy_from_slice(&body);
    rng::fill(&mut out[110..]);
    out
}

/// Build a DL7400 pipe descriptor for one connector.
///
/// The 304-byte plaintext is [`NAVARRO_STREAM_MARKER`] twice, then six
/// `[len=0x002c][kind=0x000e][slot]` records of 40 configuration bytes. Records advance by
/// `len + 2`. Each configuration names the connector's slot id and the three dock-side addresses
/// that slot is given. 14 + 14 + 6 * 46 = 304 exactly, so there is no padding and no tail.
///
/// The marker count is not a settled constant: one capture has it once followed by the six records
/// and fourteen unexplained bytes, while a capture taken while DLM was driving both panels has it
/// twice and no trailing bytes. Both plaintexts are 304 bytes. This follows the capture that was
/// working, and it is the reason the fourteen bytes must not be dismissed as AES padding for *this*
/// record: in the working capture they are consumed by a second marker at the front.
///
/// Only 2560x1440 has been observed, and the fixed header carries mode-derived bytes, so callers
/// must not use this for another mode.
pub(super) fn navarro_pipe_descriptor(connector: u8) -> Result<KVec<u8>> {
    let mut b = KVec::with_capacity(304, GFP_KERNEL)?;
    b.extend_from_slice(&NAVARRO_STREAM_MARKER, GFP_KERNEL)?;
    b.extend_from_slice(&NAVARRO_STREAM_MARKER, GFP_KERNEL)?;
    for index in 0..NAVARRO_SLOTS_PER_CONNECTOR {
        let alloc = u32::from((connector as u16) * NAVARRO_SLOTS_PER_CONNECTOR + index);
        b.extend_from_slice(&0x002cu16.to_le_bytes(), GFP_KERNEL)?;
        b.extend_from_slice(&0x000eu16.to_le_bytes(), GFP_KERNEL)?;
        b.extend_from_slice(
            &navarro_pipe_slot(connector, index).to_le_bytes(),
            GFP_KERNEL,
        )?;
        b.extend_from_slice(&NAVARRO_SLOT_HEADER, GFP_KERNEL)?;
        b.extend_from_slice(
            &navarro_pipe_ring(connector, index).to_le_bytes(),
            GFP_KERNEL,
        )?;
        b.extend_from_slice(&[0, 0], GFP_KERNEL)?;
        let cfb0 = NAVARRO_CFB0_BASE - alloc * NAVARRO_CFB0_STEP;
        b.extend_from_slice(&cfb0.to_le_bytes(), GFP_KERNEL)?;
        b.extend_from_slice(&[0, 0, 0, 0], GFP_KERNEL)?;
        let cfb1 = NAVARRO_CFB1_BASE - alloc * NAVARRO_CFB1_STEP;
        b.extend_from_slice(&cfb1.to_le_bytes(), GFP_KERNEL)?;
        b.extend_from_slice(&NAVARRO_SLOT_TRAILER, GFP_KERNEL)?;
    }
    debug_assert_eq!(b.len(), 304);
    debug_assert_eq!(b.len(), 304);
    Ok(b)
}

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
/// Authenticate and decrypt a dock->host CP frame body.
///
/// `body` is everything after the 16-byte clear wire header: AES-CTR ciphertext followed by the
/// 16-byte clear Dl3Cmac. Inbound messages use the same encrypt-then-MAC construction as
/// [`seal_livemac`]. Verifying the tag is important on Navarro because bytes 6--7 of the inner
/// header are not invariably padding: per-connector HDCP pushes put the high half of their
/// one-hot selector there (`00 80` / `80 00`). A zero-padding heuristic therefore rejects two
/// connectors' authentic messages, while accepting arbitrary unauthenticated ciphertext with a
/// chance plaintext prefix would be unsafe.
pub(super) fn open_in(ks: &[u8; 16], in_riv: &[u8; 8], seq: u32, body: &[u8]) -> Result<KVec<u8>> {
    // Both platforms authenticate an inbound frame with a trailing Dl3Cmac over the whole body.
    // Verifying it is what lets callers read the plaintext without also testing it for
    // plausibility -- and that matters, because Navarro's per-connector HDCP pushes carry a
    // one-hot selector in inner bytes 6..7 that the old "those bytes are zero padding" heuristic
    // rejected.
    if body.len() < 16 {
        return Err(EINVAL);
    }
    let (ct, wire_tag) = body.split_at(body.len() - 16);
    let expected = dl3cmac_tag(ks, in_riv, seq as u64, ct)?;
    // Accumulate the difference so a tag mismatch does not reveal the first differing byte.
    let mut different = 0u8;
    for (&actual, &want) in wire_tag.iter().zip(expected.iter()) {
        different |= actual ^ want;
    }
    if different != 0 {
        return Err(EINVAL);
    }

    let cipher = crypto::Aes128::new(ks)?;
    let mut plaintext = KVec::with_capacity(ct.len(), GFP_KERNEL)?;
    for (i, chunk) in ct.chunks(16).enumerate() {
        let mut iv = [0u8; 16];
        iv[..8].copy_from_slice(in_riv);
        iv[12..].copy_from_slice(&seq.wrapping_add(i as u32).to_be_bytes());
        let ksb = cipher.encrypt_block(&iv);
        for (j, &c) in chunk.iter().enumerate() {
            plaintext.push(c ^ ksb[j], GFP_KERNEL)?;
        }
    }
    Ok(plaintext)
}

#[cfg(CONFIG_DRM_VINO_KUNIT_TEST)]
#[kunit_tests(vino_cp)]
mod tests {
    use super::*;

    #[test]
    fn seal_livemac_roundtrip() -> Result {
        // A sealed CP frame must decrypt back to its content under the IN riv, and its
        // appended tag must equal a fresh Dl3Cmac over the ciphertext (encrypt-then-MAC).
        let ks = [
            0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd,
            0xee, 0xff,
        ];
        let riv = [0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17];
        let content = [0xa5u8; 32];
        let mut hdr = [0u8; 16];
        hdr[12..16].copy_from_slice(&4u32.to_le_bytes()); // wire_seq = 4
        let frame = seal_livemac(&ks, &riv, &hdr, &content)?;
        assert_eq!(frame.len(), 16 + 32 + 16);
        let body = &frame[16..];
        let ct = &frame[16..16 + 32];
        // `open_in` verifies the appended Dl3Cmac, then applies AES-CTR with the supplied nonce.
        assert_eq!(&open_in(&ks, &riv, 4, body)?[..], &content[..]);
        // And pin that contract rather than leaving it implicit: the IN nonce really is different,
        // so both its MAC nonce and content keystream reject this fixture.
        assert_ne!(in_riv(&riv), riv);
        assert!(open_in(&ks, &in_riv(&riv), 4, body).is_err());
        assert_eq!(&frame[16 + 32..], &dl3cmac_tag(&ks, &riv, 4, ct)?[..]);

        let mut damaged = KVec::new();
        damaged.extend_from_slice(body, GFP_KERNEL)?;
        let last = damaged.len() - 1;
        damaged[last] ^= 1;
        assert!(open_in(&ks, &riv, 4, &damaged).is_err());
        Ok(())
    }

    #[test]
    fn reply_decoders_accept_all_supported_rivs() -> Result {
        let ks = [0x5au8; 16];
        let out_head0 = [0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17];
        let in_head0 = in_riv(&out_head0);
        let mut out_head1 = out_head0;
        out_head1[0] ^= 0x80;
        let mut in_head1 = in_head0;
        in_head1[0] ^= 0x80;

        let mut header = [0u8; 16];
        header[8..10].copy_from_slice(&0x45u16.to_le_bytes());
        header[12..16].copy_from_slice(&7u32.to_le_bytes());
        let inner = [0x14, 0, 0x30, 0, 9, 0, 0, 0];

        for riv in [in_head0, in_head1, out_head0, out_head1] {
            let frame = seal_livemac(&ks, &riv, &header, &inner)?;
            assert_eq!(
                verify_in_ack(&ks, &out_head0, &frame),
                Some((0x14, 0x30, 9))
            );
            assert_eq!(
                decode_in_lenient(&ks, &out_head0, &frame),
                Some((0x14, 0x30, 9))
            );
        }

        // Navarro's connector 2/3 HDCP pushes carry the upper half of their one-hot selector at
        // inner offsets 6--7. They are authenticated messages, not malformed zero-pad headers.
        let selector_push = [0x10, 0, 0x84, 0, 0, 0, 0, 0x80];
        let frame = seal_livemac(&ks, &in_head0, &header, &selector_push)?;
        assert_eq!(
            decode_in_lenient(&ks, &out_head0, &frame),
            Some((0x10, 0x84, 0))
        );
        assert_eq!(
            &inner_plaintext(&ks, &out_head0, &frame).unwrap()[..],
            &selector_push
        );
        Ok(())
    }

    #[test]
    fn stream_content_nonce_matches_golden_vectors() {
        // Ridge: each connector's video stream is `0x08 | connector`.
        let h0 = stream_content_nonce(&[0xa1, 0x2b, 0xaa, 0xb7, 0x0e, 0x0b, 0x02, 0x74], 0x08);
        assert_eq!(h0, [0xa1, 0x2b, 0xaa, 0xb7, 0x0e, 0x0b, 0x02, 0x7c]);

        let h1 = stream_content_nonce(&[0xd0, 0x2a, 0xc0, 0x83, 0xb6, 0x42, 0x72, 0x57], 0x09);
        assert_eq!(h1, [0xd0, 0x2a, 0xc0, 0x83, 0xb6, 0x42, 0x72, 0x5e]);

        // Navarro: the RIV each connector's SKE_Send_Eks delivered, and the AES-CTR nonce the
        // dock then expects for that connector's stream.
        let riv = [0x7d, 0x2c, 0xb6, 0x6b, 0x2c, 0xd1, 0x75, 0x7c];
        let link = stream_content_nonce(&riv, 0x04);
        assert_eq!(link, [0x7d, 0x2c, 0xb6, 0x6b, 0x2c, 0xd1, 0x75, 0x78]);

        let c0 = stream_content_nonce(&[0xc3, 0x45, 0xfe, 0x55, 0x93, 0x61, 0x39, 0x01], 0x07);
        assert_eq!(c0, [0xc3, 0x45, 0xfe, 0x55, 0x93, 0x61, 0x39, 0x06]);

        let c1 = stream_content_nonce(&[0x94, 0x46, 0xc8, 0x3d, 0xa5, 0xfa, 0x39, 0xe3], 0x0f);
        assert_eq!(c1, [0x94, 0x46, 0xc8, 0x3d, 0xa5, 0xfa, 0x39, 0xec]);
    }

    #[test]
    fn aux_for_id_constants() {
        // The CP header `aux` field is a per-inner-id constant, not body_len/4.
        assert_eq!(aux_for_id(0x14, 48), 0x0a);
        assert_eq!(aux_for_id(0x15, 32), 0x09);
        assert_eq!(aux_for_id(0x36, 80), 0x08);
        assert_eq!(aux_for_id(0x48, 96), 0x06);
        // Cursor message IDs have fixed auxiliary fields; deriving them as `body_len / 4` would
        // produce 0x0c for all three.
        assert_eq!(aux_for_id(0x1a, 48), 0x04); // cursor move
        assert_eq!(aux_for_id(0x1b, 48), 0x03); // cursor create
        assert_eq!(aux_for_id(0x1c, 48), 0x02); // cursor image
        assert_eq!(aux_for_id(0x99, 40), 10); // unknown id falls back to body_len/4
    }

    #[test]
    fn cp_setup_burst_table_framing() -> Result {
        // Pin the post-msg0 `(aux, body_len)` wire profile. `body_len` includes the encrypted
        // content and its 16-byte Dl3Cmac tag.
        const PER_HEAD_FINGERPRINT: [(u16, usize); 9] = [
            (0x0c, 64),
            (0x0f, 64),
            (0x04, 176),
            (0x0c, 64),
            (0x0c, 80),
            (0x04, 64),
            (0x08, 64),
            (0x0a, 48),
            (0x05, 48),
        ];
        // Finalization bodies contain 32 bytes of content and a 16-byte tag. Keep one fingerprint
        // per step so table growth cannot cause an out-of-bounds test access.
        const FINALIZE_FINGERPRINT: [(u16, usize); 3] = [(0x08, 48), (0x09, 48), (0x08, 48)];
        // Keep the fingerprint table and the step table in lockstep: growing one without the
        // other is exactly the defect above.
        build_assert!(FINALIZE_FINGERPRINT.len() == CP_SETUP_FINALIZE_STEPS.len());

        let ks = [0x5au8; 16];
        let riv = [0x11u8; 8];
        for (i, &(id, _sub, content_len)) in CP_SETUP_PER_HEAD.iter().enumerate() {
            let content = KVec::from_elem(0u8, content_len, GFP_KERNEL)?;
            let frame = seal_interactive(&ks, &riv, id, 0, &content)?;
            let (want_aux, want_body) = PER_HEAD_FINGERPRINT[i];
            assert_eq!(frame.len(), 16 + want_body);
            assert_eq!(u16::from_le_bytes([frame[10], frame[11]]), want_aux);
        }
        for (i, &(id, _sub)) in CP_SETUP_FINALIZE_STEPS.iter().enumerate() {
            let frame = seal_interactive(&ks, &riv, id, 0, &[0u8; 32])?;
            let (want_aux, want_body) = FINALIZE_FINGERPRINT[i];
            assert_eq!(frame.len(), 16 + want_body);
            assert_eq!(u16::from_le_bytes([frame[10], frame[11]]), want_aux);
        }
        Ok(())
    }

    #[test]
    fn stream_manage_restatement_matches_dlm() -> Result {
        // All deterministic fields must match the captured plaintext for both connectors. The
        // connector marker is at offset 23, the HDCP message ID at offset 27, and the final three
        // u32 fields contain `0`, `1`, and `connector + 8`.
        const WANT: [[u8; 40]; 2] = [
            [
                0x26, 0x00, 0x10, 0x00, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x00, 0x00,
                0x00, 0x01, 0x00, 0x00, 0x00, 0x10, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00,
                0x08, 0x00, 0x00, 0x00,
            ],
            [
                0x26, 0x00, 0x10, 0x00, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x00, 0x00,
                0x00, 0x02, 0x00, 0x00, 0x00, 0x10, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00,
                0x09, 0x00, 0x00, 0x00,
            ],
        ];
        for connector in 0..2u8 {
            // Ridge: the connector is a one-based connector number at offset 23, and the
            // content-stream id at offset 36 is 8 for connector 0 and 9 for connector 1.
            let c = stream_manage_restatement(0, connector, 8 + u16::from(connector), false)?;
            assert_eq!(c.len(), 48);
            // Bytes 4..6 are the live counter (passed as 0 here, so already covered); the last
            // 8 bytes (offset 40..48) are host-random.
            assert_eq!(&c[..40], &WANT[connector as usize][..]);
        }
        Ok(())
    }

    #[test]
    fn video_arm_burst_table_framing() -> Result {
        // Pin every video-arm entry's type, sub-ID, auxiliary value, and body length to captured
        // traffic. Head 0 uses the table's base sub-IDs; the builders add one for connector 1. The
        // compile-time length check prevents the fixture and production table from drifting.
        const FINGERPRINT_H0: [(u32, u16, u16, usize); 10] = [
            (2, 0x0008, 0x0000, 16),
            (2, 0x0018, 0x0000, 16),
            (4, 0x0008, 0x000a, 16),
            (4, 0x0018, 0x000a, 16),
            (2, 0x0000, 0x0000, 16),
            (2, 0x0010, 0x0000, 16),
            (4, 0x0000, 0x0004, 16),
            (4, 0x0010, 0x0004, 16),
            (4, 0x0008, 0x000e, 1104),
            (4, 0x0018, 0x000e, 1104),
        ];
        build_assert!(FINGERPRINT_H0.len() == VIDEO_ARM_BURST.len());
        let ks = [0x5au8; 16];
        let riv = [0x11u8; 8];
        for (i, &(wire_type, sub_base, aux, body_len)) in VIDEO_ARM_BURST.iter().enumerate() {
            let (want_type, want_sub, want_aux, want_body) = FINGERPRINT_H0[i];
            assert_eq!(
                (wire_type, sub_base, aux, body_len),
                (want_type, want_sub, want_aux, want_body)
            );
            if wire_type == 2 {
                let body = video_arm_plaintext_body(i, 0);
                let frame = video_arm_plain_frame(sub_base, &body);
                assert_eq!(frame.len(), 32);
                assert_eq!(u16::from_le_bytes([frame[8], frame[9]]), want_sub);
            } else {
                let content = KVec::from_elem(0u8, body_len, GFP_KERNEL)?;
                let frame = seal_video_arm(&ks, &riv, sub_base, aux, 0, &content)?;
                assert_eq!(frame.len(), 16 + body_len + 16);
                assert_eq!(u16::from_le_bytes([frame[8], frame[9]]), want_sub);
                assert_eq!(u16::from_le_bytes([frame[10], frame[11]]), want_aux);
            }
        }
        Ok(())
    }

    #[test]
    fn navarro_stream_open_matches_the_wire() -> Result {
        // The connector lives solely in the wire sub, never in the content: all four connectors
        // send the same marker, followed by a two-byte opaque tail.
        let open = navarro_stream_open();
        assert_eq!(open.len(), 16);
        assert_eq!(open[..14], NAVARRO_STREAM_MARKER);

        // Sealing it produces the 48-byte frame the dock is sent: a 16-byte header, the 16-byte
        // ciphertext and a 16-byte Dl3Cmac, with `size` covering all but the first four bytes.
        let frame = seal_video_arm(&[0u8; 16], &[0u8; 8], 0x0007, 0x0002, 0, &open)?;
        assert_eq!(frame.len(), 48);
        assert_eq!(u16::from_le_bytes([frame[2], frame[3]]), 0x002c);
        assert_eq!(u16::from_le_bytes([frame[8], frame[9]]), 0x0007);
        assert_eq!(u16::from_le_bytes([frame[10], frame[11]]), 0x0002);
        Ok(())
    }

    /// Where each message stops meaning something and starts being filler.
    ///
    /// This boundary is the one that has actually cost hardware runs: the DL-3x00 cold-activation
    /// gate was a random tail that began one byte early and buried the `0x16/0x23` connector
    /// selector at offset 23, and nothing on the wire says no -- the dock acknowledges the message
    /// either way and simply does not act on it. Every class below is checked against the vendor's
    /// own corpus, where a byte the vendor varies over two or three values is a field and a byte it
    /// varies uniformly is filler.
    ///
    /// Asserts the structured prefix and the total length. The tail itself cannot be asserted,
    /// which is exactly why its start offset has to be.
    #[test]
    fn random_tails_begin_where_the_vendor_stops_meaning_something() -> Result {
        // `0x14/0x0c`: nothing after the header; the tail is the whole of offsets 22..32.
        let poll = device_query_req(0x1234, 0x000c)?;
        assert_eq!(poll.len(), 32);
        assert!(poll[8..22].iter().all(|&b| b == 0));

        // `0x16/0x2e` and `0x16/0x2f`: connector at 22, state at 23, tail from 24. The vendor's
        // corpus shows exactly two values at each -- connector, and the sink state.
        for (sub, state) in [(0x2eu16, 3u8), (0x2e, 0), (0x2f, 1), (0x2f, 0)] {
            for connector in 0..2u8 {
                let m = stream_marker(0x1234, connector, sub, state)?;
                assert_eq!(m.len(), 32);
                assert!(m[8..22].iter().all(|&b| b == 0));
                assert_eq!(m[22], connector);
                assert_eq!(m[23], state);
            }
        }

        // `0x15/0x20` and `0x15/0x21`: connector at 22 alone, tail from 23.
        for sub in [0x20u16, 0x21] {
            for connector in 0..2u8 {
                let m = cp::get_edid_req_sub(0x1234, sub, connector)?;
                assert_eq!(m.len(), 32);
                assert_eq!(m[22], connector);
            }
        }

        // `0x16/0x23`: the one that was wrong. Both bytes are selectors, and a tail that starts
        // at 22 instead of 24 silently disables the downstream sink enable.
        for connector in 0..2u8 {
            let m = cp::edid_engage_req(0x1234, connector)?;
            assert_eq!(m.len(), 32);
            assert_eq!(m[22], connector);
            assert_eq!(m[23], connector);
        }
        Ok(())
    }

    #[test]
    fn stream_marker_routes_the_selected_head() -> Result {
        let h0 = stream_marker(0x1234, 0, 0x2f, 1)?;
        let h1 = stream_marker(0x1235, 1, 0x2e, 3)?;
        assert_eq!(&h0[0..6], &[0x16, 0, 0x2f, 0, 0x34, 0x12]);
        assert_eq!(&h0[22..24], &[0, 1]);
        assert_eq!(&h1[0..6], &[0x16, 0, 0x2e, 0, 0x35, 0x12]);
        assert_eq!(&h1[22..24], &[1, 3]);
        Ok(())
    }

    #[test]
    fn navarro_pipe_descriptor_matches_authenticated_capture() -> Result {
        // Slot ids and the three dock-side addresses of every record, for both connectors of the
        // authenticated capture.
        for (connector, slots) in [
            (
                0u8,
                [
                    (0x0000u16, 0x6fccu32, 0x71fb_9000u32, 0x7216_6000u32),
                    (0x0001, 0x6db0, 0x71fb_4000, 0x7215_e000),
                    (0x0002, 0x6b94, 0x71fa_f000, 0x7215_6000),
                    (0x0003, 0x6978, 0x71fa_a000, 0x7214_e000),
                    (0x0004, 0x675c, 0x71fa_5000, 0x7214_6000),
                    (0x0005, 0x6540, 0x71fa_0000, 0x7213_e000),
                ],
            ),
            (
                1u8,
                [
                    (0x0008, 0x5eec, 0x71f9_b000, 0x7213_6000),
                    (0x0009, 0x5cd0, 0x71f9_6000, 0x7212_e000),
                    (0x000a, 0x5ab4, 0x71f9_1000, 0x7212_6000),
                    (0x000b, 0x5898, 0x71f8_c000, 0x7211_e000),
                    (0x000c, 0x567c, 0x71f8_7000, 0x7211_6000),
                    (0x000d, 0x5460, 0x71f8_2000, 0x7210_e000),
                ],
            ),
        ] {
            let descriptor = navarro_pipe_descriptor(connector)?;
            assert_eq!(descriptor.len(), 304);
            // The marker is present twice before the slot records; 14 + 14 + 6 * 46 = 304. Assert
            // both copies, so the records are read from 28 rather than from the second marker.
            assert_eq!(&descriptor[..14], &NAVARRO_STREAM_MARKER);
            assert_eq!(&descriptor[14..28], &NAVARRO_STREAM_MARKER);
            for (index, &(slot, ring, plane0, plane1)) in slots.iter().enumerate() {
                let at = 28 + index * 46;
                assert_eq!(&descriptor[at..at + 4], &[0x2c, 0x00, 0x0e, 0x00]);
                assert_eq!(
                    u16::from_le_bytes([descriptor[at + 4], descriptor[at + 5]]),
                    slot
                );
                let cfg = &descriptor[at + 6..at + 46];
                let word =
                    |o: usize| u32::from_le_bytes([cfg[o], cfg[o + 1], cfg[o + 2], cfg[o + 3]]);
                assert_eq!(word(12), ring);
                assert_eq!(word(18), plane0);
                assert_eq!(word(26), plane1);
            }
        }

        // The decoder configuration is the same message Ridge sends, with the DL7400's layout word.
        let tail = [0x5a; 14];
        let header = video_arm::mode_header(2560, 1440, 0x2100, false);
        let config = video_arm::build_config(video_arm::CodeTables::Wide, &header, &tail, false)?;
        assert_eq!(config.len(), 1104);
        assert_eq!(
            &config[..26],
            &[
                0x18, 0x00, 0x0b, 0x03, 0x04, 0x02, 0x02, 0x00, 0x02, 0x00, 0x00, 0x0a, 0xa0, 0x05,
                0x00, 0x21, 0x02, 0x00, 0x00, 0x0a, 0xa0, 0x05, 0x00, 0x21, 0x00, 0x00,
            ]
        );
        assert_eq!(&config[1090..], &tail);
        Ok(())
    }

    #[test]
    fn ella_stream_records_match_the_captured_bytes() -> Result {
        // The three records that open a DL-3x00 stream, each pinned to the bytes DLM sends. A
        // stream opened with any of them wrong is a stream the dock accepts every frame of and
        // presents none of, with nothing on the wire to say so -- so these are checked here rather
        // than on hardware, where each attempt costs a replug.
        let geometry = video::haar::Geometry::new(8, true, false, 0, 0x08, 3);

        // Announcing the content stream, then the video plane. Both connectors, both markers.
        for (connector, stream, plane) in [(0u8, 0x08u16, 0x00u16), (1, 0x09, 0x01)] {
            let announce = stream_announce(stream, STREAM_ANNOUNCE_MARKER);
            assert_eq!(geometry.stream_id(connector), stream);
            assert_eq!(
                &announce[..12],
                &[0, 0, 0x1c, 0, 2, 0, 0, 0, stream as u8, 0, 0, 0]
            );
            assert_eq!(&announce[16..20], &[stream as u8, 0, 6, 0]);
            assert_eq!(announce[20..], [0u8; 12]);

            let announce = stream_announce(plane, 0);
            assert_eq!(u16::from(geometry.connector_selector(connector)), plane);
            assert_eq!(
                &announce[..12],
                &[0, 0, 0x1c, 0, 2, 0, 0, 0, plane as u8, 0, 0, 0]
            );
            assert_eq!(
                announce[16..],
                [plane as u8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]
            );
        }

        // The sealed open. Only the first six bytes are fixed; the rest is a host-random token
        // that the Dl3Cmac covers, so its length is what matters.
        let open = stream_open(0x01);
        assert_eq!(open.len(), 16);
        assert_eq!(&open[..6], &[0x04, 0x00, 0x08, 0x04, 0x01, 0x00]);

        // The decoder configuration, in full. 1920x1080 is stated as 1088 lines: the surface the
        // dock is told about is the padded one the codec actually produces.
        let header = video_arm::mode_header(1920, 1088, 0x1800, false);
        let config = video_arm::build_config(video_arm::CodeTables::Narrow, &header, &[], false)?;
        assert_eq!(config.len(), 304);
        assert_eq!(
            &config[..26],
            &[
                0x18, 0x00, 0x0b, 0x03, 0x04, 0x02, 0x02, 0x00, 0x02, 0x00, 0x80, 0x07, 0x40, 0x04,
                0x00, 0x18, 0x02, 0x00, 0x80, 0x07, 0x40, 0x04, 0x00, 0x18, 0x00, 0x00,
            ]
        );
        assert_eq!(
            &config[26..],
            &[
                0x28, 0x00, 0x09, 0x00, 0x12, 0x00, 0x01, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00,
                0x04, 0x00, 0x00, 0x00, 0x08, 0x00, 0x00, 0x00, 0x10, 0x00, 0x00, 0x00, 0x20, 0x00,
                0x00, 0x00, 0x40, 0x00, 0x00, 0x00, 0x80, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x02,
                0x2c, 0x00, 0x09, 0x01, 0x14, 0x00, 0x01, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00,
                0x04, 0x00, 0x00, 0x00, 0x08, 0x00, 0x00, 0x00, 0x10, 0x00, 0x00, 0x00, 0x20, 0x00,
                0x00, 0x00, 0x40, 0x00, 0x00, 0x00, 0x80, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00,
                0x00, 0x02, 0x00, 0x04, 0x2c, 0x00, 0x09, 0x02, 0x14, 0x00, 0x01, 0x00, 0x00, 0x00,
                0x02, 0x00, 0x00, 0x00, 0x04, 0x00, 0x00, 0x00, 0x08, 0x00, 0x00, 0x00, 0x10, 0x00,
                0x00, 0x00, 0x20, 0x00, 0x00, 0x00, 0x40, 0x00, 0x00, 0x00, 0x80, 0x00, 0x00, 0x00,
                0x00, 0x01, 0x00, 0x00, 0x00, 0x02, 0x00, 0x04, 0x16, 0x00, 0x09, 0x03, 0x09, 0x00,
                0x01, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x04, 0x00, 0x00, 0x00, 0x08, 0x00,
                0x0f, 0x00, 0x02, 0x00, 0x22, 0x00, 0x09, 0x04, 0x0f, 0x00, 0x01, 0x00, 0x00, 0x00,
                0x02, 0x00, 0x00, 0x00, 0x04, 0x00, 0x00, 0x00, 0x08, 0x00, 0x00, 0x00, 0x10, 0x00,
                0x00, 0x00, 0x20, 0x00, 0x00, 0x00, 0x40, 0x00, 0x7f, 0x00, 0x02, 0x00, 0x52, 0x00,
                0x0a, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0x00, 0x40, 0x00, 0x40, 0x00, 0x10, 0x00,
                0x10, 0x00, 0x10, 0x00, 0x10, 0x00, 0x10, 0x00, 0x10, 0x00, 0x10, 0x00, 0x20, 0x00,
                0x20, 0x00, 0x20, 0x00, 0x01, 0x00, 0x01, 0x00, 0x01, 0x00, 0x10, 0x00, 0x10, 0x00,
                0x04, 0x00, 0x10, 0x00, 0x10, 0x00, 0x04, 0x00, 0x20, 0x00, 0x20, 0x00, 0x08, 0x00,
                0x01, 0x00, 0x01, 0x00, 0x01, 0x00, 0x20, 0x00, 0x20, 0x00, 0x02, 0x00, 0x20, 0x00,
                0x20, 0x00, 0x02, 0x00, 0x40, 0x00, 0x40, 0x00, 0x04, 0x00, 0x00, 0x00,
            ]
        );

        // The per-frame report on this dock is the mode header and a six-byte token, nothing else.
        let report = stream_report_mode_only(&header);
        assert_eq!(report.len(), 32);
        assert_eq!(&report[..26], &header);
        Ok(())
    }
}
