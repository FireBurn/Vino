// SPDX-License-Identifier: GPL-2.0

//! Asking the dock what is plugged into a connector.
//!
//! The EDID path is where the dock is least forgiving: the selector at offset 22 names the
//! connector, the reply's own id carries its length, and a fetch issued before the handler is
//! engaged returns a block the dock synthesises for itself.

use super::*;

/// OUT `id=0x16 sub=0x0023` downstream-sink state request. Offset 22 selects the connector and
/// offset 23 carries the state. Navarro's cold transcript uses `0xff` to tear the sink down, then
/// the connector selector itself (`0` or `1`) to re-engage it.
/// Vendor and product id of the descriptor the dock serves for itself.
///
/// A fetch the dock cannot answer from the monitor is answered from here, so this pair is the only
/// thing separating that block from a real one.
const BRIDGE_ID: [u8; 4] = [0x3a, 0xd4, 0x9c, 0x07];

pub(crate) fn edid_sink_state(counter: u16, connector: u8, state: u8) -> Result<KVec<u8>> {
    let mut b = KVec::with_capacity(32, GFP_KERNEL)?;
    header(&mut b, 0x16, 0x0023, counter)?;
    pad_to(&mut b, 22)?;
    b.extend_from_slice(&[connector, state], GFP_KERNEL)?;
    let mut tail = [0u8; 8];
    rng::fill(&mut tail);
    b.extend_from_slice(&tail, GFP_KERNEL)?;
    Ok(b)
}
/// Engage one downstream sink after its EDID exchange.
pub(crate) fn edid_engage_req(counter: u16, connector: u8) -> Result<KVec<u8>> {
    edid_sink_state(counter, connector, connector)
}
/// OUT `id=0x15 sub=0x0053` post-EDID capability query. Offset 22 is a connector bitmask.
///
/// `connector + 1` and `1 << connector` are the same byte for connectors 0 and 1, so a capture with
/// both monitors in the first two sockets cannot distinguish them. DLM sends `4` for connector 2,
/// where a one-based index would send 3.
pub(crate) fn post_edid_query(counter: u16, connector: u8) -> Result<KVec<u8>> {
    let mut b = KVec::with_capacity(32, GFP_KERNEL)?;
    header(&mut b, 0x15, 0x0053, counter)?;
    pad_to(&mut b, 22)?;
    b.push(1u8 << connector, GFP_KERNEL)?;
    let mut tail = [0u8; 9];
    rng::fill(&mut tail);
    b.extend_from_slice(&tail, GFP_KERNEL)?;
    Ok(b)
}
/// OUT `id=0x16 sub=0x004b` downstream EDID-reader state request.
pub(crate) fn edid_readiness_state(counter: u16, connector: u8, state: u8) -> Result<KVec<u8>> {
    let mut b = KVec::with_capacity(32, GFP_KERNEL)?;
    header(&mut b, 0x16, 0x4b, counter)?;
    pad_to(&mut b, 22)?;
    // Offset 22 selects the downstream connector and offset 23 stops/starts the reader.
    b.extend_from_slice(&[connector, state], GFP_KERNEL)?;
    let mut tail = [0u8; 8];
    rng::fill(&mut tail);
    b.extend_from_slice(&tail, GFP_KERNEL)?;
    Ok(b)
}
/// Start one downstream EDID read.
pub(crate) fn edid_readiness_kick(counter: u16, connector: u8) -> Result<KVec<u8>> {
    edid_readiness_state(counter, connector, 1)
}
/// OUT get-EDID request (`id=0x15 sub=0x21`). A `sub=0x20` probe must precede each fetch attempt.
/// The dock may initially return an internal placeholder, so callers retry until a downstream EDID
/// arrives.
pub(crate) fn get_edid_req(counter: u16, connector: u8) -> Result<KVec<u8>> {
    get_edid_req_sub(counter, 0x21, connector)
}
/// Build an `id=0x15` EDID-family request with an explicit `sub` (`0x20` = probe/seek,
/// `0x21` = fetch -- see [`get_edid_req`]'s doc comment). Same 32-byte wire shape for both.
pub(crate) fn get_edid_req_sub(counter: u16, sub: u16, connector: u8) -> Result<KVec<u8>> {
    let mut b = KVec::with_capacity(32, GFP_KERNEL)?;
    header(&mut b, 0x15, sub, counter)?;
    pad_to(&mut b, 22)?;
    // Offset 22 selects the downstream connector; the remaining bytes are an opaque token.
    b.push(connector, GFP_KERNEL)?;
    let mut tail = [0u8; 9];
    rng::fill(&mut tail);
    b.extend_from_slice(&tail, GFP_KERNEL)?;
    Ok(b)
}
/// How many EDID bytes a reply id says it carries, if it names an EDID reply at all.
///
/// There is no single id for an EDID reply. The field is `0x14` -- the dock's generic reply -- plus
/// the number of EDID bytes behind it, so a monitor whose EDID is one block answers `0x94`, a
/// two-block one `0x114` and a three-block one `0x194`. Matching a fixed value makes every monitor
/// with a different extension count invisible: the fetch is answered, the answer is discarded, and
/// the connector is reported as having no sink at all.
pub(crate) fn edid_reply_len(id: u16) -> Option<usize> {
    let n = usize::from(id).checked_sub(0x14)?;
    (n >= 128 && n % 128 == 0).then_some(n)
}
/// Whether a reply carries a connector's downstream display capability.
///
/// The inner sub names the message; the id is `0x14` plus the payload length, exactly as for an
/// EDID reply (see [`edid_reply_len`]), so it moves with the descriptor the attached monitor
/// produces. Pinning it to one observed length makes a monitor answering a shorter descriptor read
/// as an empty socket, and that connector is then never probed for an EDID.
pub(crate) fn is_display_cap_reply(id: u16, sub: u16) -> bool {
    sub == 0x30 && id > 0x14
}
/// Decrypt an EDID reply and return its complete base block and extensions.
///
/// EDID replies use wire `sub=0x45` and inner `sub=0x21`, with the id naming the payload length
/// (see [`edid_reply_len`]). The EDID starts at inner offset 22 and its base-block extension count
/// determines the returned length. All supported direction and connector RIV variants are checked.
pub(crate) fn parse_edid_from_reply(
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
        let Some(declared) = edid_reply_len(id).filter(|_| sub == 0x21) else {
            continue;
        };
        let edid = &inner[EDID_OFF..];
        // Say what arrived, not just that nothing valid did. "no EDID came back" is true of a
        // sink that answered with a block this rejected and of one that never answered at all,
        // and those want opposite fixes.
        if crate::debug_enabled() {
            vino_debug!(
                "vino: EDID reply candidate: inner {} B, payload {} B, first 8 {:02x?}\n",
                inner.len(),
                edid.len(),
                &edid[..8.min(edid.len())]
            );
        }
        // Validate the EDID base-block magic `00 FF FF FF FF FF FF 00`.
        const MAGIC: [u8; 8] = [0x00, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x00];
        if edid[..8] != MAGIC {
            if crate::debug_enabled() {
                vino_debug!("vino: EDID reply rejected: bad base-block magic\n");
            }
            continue;
        }
        // ...and its checksum. The magic is only eight bytes and a dock with an empty port can
        // return a block that carries it, which is enough to be mistaken for a monitor: the
        // connector is then declared connected, a hotplug is raised for a sink that is not there,
        // and the dock resets. A real base block sums to zero modulo 256.
        if edid.len() < 128 {
            continue;
        }
        if edid[..128].iter().fold(0u8, |a, b| a.wrapping_add(*b)) != 0 {
            if crate::debug_enabled() {
                vino_debug!("vino: EDID reply rejected: base block checksum\n");
            }
            continue;
        }
        // A fetch the dock cannot yet answer from the monitor is answered from itself: a block
        // describing a 1920x1080 panel under the bridge's own vendor and product id. It passes the
        // magic and the checksum, so nothing above catches it, and publishing it drives the sink at
        // a timing it never advertised. Refuse it and let the caller ask again.
        if edid[8..12] == BRIDGE_ID {
            if crate::debug_enabled() {
                vino_debug!("vino: EDID reply rejected: the dock's own bridge descriptor\n");
            }
            continue;
        }
        if crate::debug_enabled() {
            vino_debug!(
                "vino: EDID base block accepted: {} extension block(s) declared, {} B available\n",
                edid[126],
                edid.len()
            );
        }
        // The reply says how much EDID it carries; the base block says how much the monitor has.
        // Take the smaller, so a truncated reply is never read past its end and a base block
        // claiming more extensions than arrived cannot manufacture them.
        let total = ((1 + edid[126] as usize) * 128)
            .min(edid.len())
            .min(declared);
        // Keep only extension blocks that are wholly present and sum to zero. The core validates
        // the whole blob, so one bad extension costs the monitor every mode it described --
        // the connector then falls back to a synthesised list and the sink is driven at a timing
        // it never advertised. A base block alone is a valid EDID and still carries the native
        // mode, so salvage what checks out.
        let mut blocks = 1;
        while blocks * 128 + 128 <= total {
            let ext = &edid[blocks * 128..blocks * 128 + 128];
            if ext.iter().fold(0u8, |a, b| a.wrapping_add(*b)) != 0 {
                break;
            }
            blocks += 1;
        }
        let kept = blocks * 128;
        if crate::debug_enabled() && kept != total {
            vino_debug!(
                "vino: EDID extension blocks: {} of {} kept, rest failed checksum\n",
                blocks - 1,
                total / 128 - 1
            );
        }
        let mut out = KVec::with_capacity(kept, GFP_KERNEL)?;
        out.extend_from_slice(&edid[..kept], GFP_KERNEL)?;
        // The extension count and the base-block checksum have to agree with what is actually
        // being handed over, or the core rejects a blob whose blocks are individually sound.
        if out[126] != (blocks - 1) as u8 {
            out[126] = (blocks - 1) as u8;
            out[127] = 0;
            let sum = out[..128].iter().fold(0u8, |a, b| a.wrapping_add(*b));
            out[127] = (0u8).wrapping_sub(sum);
        }
        return Ok(Some(out));
    }
    Ok(None)
}
/// Decode the downstream status carried by an EDID probe reply.
///
/// Returns the inner message id, the little-endian status at offsets 22 through 25 and the ready
/// bit at offset 26. `None` means no matching reply was decrypted.
pub(crate) fn probe_reply_status(
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
        // generic negative acknowledgment can answer this probe. The handler's id is `0x14` plus
        // its payload length (see `edid_reply_len`), so it names a descriptor size rather than a
        // message type and cannot be matched against a list of the sizes seen so far.
        if !(id > 0x14 && sub == 0x0020) && id != 0x14 {
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
pub(crate) fn edid_poll_ready(ks: &[u8; 16], out_riv: &[u8; 8], wire: &[u8]) -> Option<bool> {
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

#[cfg(CONFIG_DRM_VINO_KUNIT_TEST)]
#[kunit_tests(vino_cp_edid)]
mod tests {
    #[test]
    fn the_docks_own_bridge_descriptor_is_never_published() {
        // Built from a block the dock served on a warm plug: valid magic and checksum, so only the
        // vendor and product id separate it from a monitor's.
        let mut block = [0u8; 128];
        block[..8].copy_from_slice(&[0x00, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x00]);
        block[8..12].copy_from_slice(&[0x3a, 0xd4, 0x9c, 0x07]);
        let sum = block[..127].iter().fold(0u8, |a, b| a.wrapping_add(*b));
        block[127] = 0u8.wrapping_sub(sum);
        assert_eq!(block.iter().fold(0u8, |a, b| a.wrapping_add(*b)), 0);
        assert_eq!(block[8..12], super::BRIDGE_ID);
    }

    use super::*;

    #[test]
    fn edid_reply_guards() -> Result {
        // The pre-decrypt guards reject non-EDID frames without touching the cipher.
        let ks = [0u8; 16];
        let riv = [0u8; 8];
        assert!(parse_edid_from_reply(&ks, &riv, &[0u8; 10])?.is_none());
        let mut wrong_sub = [0u8; 20];
        wrong_sub[8] = 0x44; // wire sub != 0x45
        assert!(parse_edid_from_reply(&ks, &riv, &wrong_sub)?.is_none());
        Ok(())
    }

    /// An EDID reply's id is its payload length, not a message type.
    ///
    /// Both values below are off the wire in one session: the dock answered one connector's fetch
    /// with `0x114` and the other's with `0x194`, and the difference is exactly the 128 bytes of
    /// one extension block. Accepting only the larger left a monitor that the vendor drives
    /// reported as an empty socket for the whole life of the driver.
    #[test]
    fn edid_reply_id_is_the_payload_length() {
        assert_eq!(edid_reply_len(0x94), Some(128));
        assert_eq!(edid_reply_len(0x114), Some(256));
        assert_eq!(edid_reply_len(0x194), Some(384));
        assert_eq!(edid_reply_len(0x214), Some(512));
        // The generic reply itself carries no EDID, and neither does anything off the 128-byte
        // grid: a status or capability id must never be read as a base block.
        assert_eq!(edid_reply_len(0x14), None);
        assert_eq!(edid_reply_len(0x44), None);
        assert_eq!(edid_reply_len(0x78), None);
        assert_eq!(edid_reply_len(0x0), None);
        assert_eq!(edid_reply_len(0x95), None);
    }

    #[test]
    fn get_edid_req_matches_dlm_wire_shape() -> Result {
        // The captured request is 32 bytes: an 8-byte header, 14 zero bytes, and a 10-byte random
        // tail at offset 22.
        let req = get_edid_req(0x2c, 0)?;
        assert_eq!(req.len(), 32);
        assert_eq!(
            &req[0..8],
            &[0x15, 0x00, 0x21, 0x00, 0x2c, 0x00, 0x00, 0x00]
        );
        assert_eq!(&req[8..22], &[0u8; 14]);
        // Pin the complete wire framing as well:
        // aux=0x09 (cp::aux_for_id(0x15, ..)), body = 32 + 16 (tag) = 48 bytes.
        let frame = cp::seal_interactive(&[0x5au8; 16], &[0x11u8; 8], 0x15, 0, &req)?;
        assert_eq!(frame.len(), 16 + 32 + 16);
        assert_eq!(u16::from_le_bytes([frame[10], frame[11]]), 0x09);
        Ok(())
    }

    #[test]
    fn edid_engage_req_matches_dlm_wire_shape() -> Result {
        // Independent captures agree on `id=0x16 sub=0x0023` with the same 32-byte shape as
        // `get_edid_req`: an 8-byte header, 14 zero bytes, and a 10-byte random tail.
        let req = edid_engage_req(0x30, 0)?;
        assert_eq!(req.len(), 32);
        assert_eq!(
            &req[0..8],
            &[0x16, 0x00, 0x23, 0x00, 0x30, 0x00, 0x00, 0x00]
        );
        assert_eq!(&req[8..22], &[0u8; 14]);
        let frame = cp::seal_interactive(&[0x5au8; 16], &[0x11u8; 8], 0x16, 0, &req)?;
        assert_eq!(frame.len(), 16 + 32 + 16);
        assert_eq!(u16::from_le_bytes([frame[10], frame[11]]), 0x08); // cp::aux_for_id(0x16, ..)
        Ok(())
    }

    #[test]
    fn edid_poll_ready_byte_matches_golden_replies() -> Result {
        // Golden dock-to-host `id=0x0044 sub=0x0020` replies pin the readiness bit at inner offset
        // 26. The first precedes a placeholder `id=0x114` fetch and the second precedes a real
        // `id=0x194` EDID fetch.
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
        assert_eq!(edid_poll_ready(&KS, &OUT_RIV, &NOT_READY), Some(false));
        assert_eq!(edid_poll_ready(&KS, &OUT_RIV, &READY), Some(true));
        Ok(())
    }

    #[test]
    fn per_head_selectors_match_dlm_in_the_far_sockets() -> Result {
        // `connector`, `connector + 1` and `1 << connector` agree for connectors 0 and 1, so only a
        // capture with a monitor in a later socket separates them. These are the bytes DLM sends
        // for connectors 1 and 2.

        // `id=0x16 sub=0x23` names the connector twice, at offset 22 and offset 23.
        for connector in 0..drm_sink::MAX_CONNECTORS as u8 {
            let req = edid_engage_req(0x30, connector)?;
            assert_eq!(req[22], connector);
            assert_eq!(req[23], connector);
        }

        // `id=0x15 sub=0x53` carries a connector bitmask at offset 22: 2 for connector 1 and 4 for
        // connector 2, where a one-based index would send 3.
        assert_eq!(post_edid_query(0x30, 1)?[22], 2);
        assert_eq!(post_edid_query(0x30, 2)?[22], 4);
        for connector in 0..drm_sink::MAX_CONNECTORS as u8 {
            assert_eq!(post_edid_query(0x30, connector)?[22], 1u8 << connector);
        }
        Ok(())
    }

    #[test]
    fn display_capability_reply_reports_presence() -> Result {
        let key = [0x5au8; 16];
        let riv = [0x33u8; 8];
        let mut inner = [0u8; 32];
        inner[0..2].copy_from_slice(&0x78u16.to_le_bytes());
        inner[2..4].copy_from_slice(&0x20u16.to_le_bytes());
        inner[22..26].copy_from_slice(&0x1234u32.to_le_bytes());
        inner[26] = 0x80;
        let mut wire = cp::seal_interactive(&key, &riv, 0x78, 11, &inner)?;
        wire[8..10].copy_from_slice(&0x45u16.to_le_bytes());

        assert_eq!(
            probe_reply_status(&key, &riv, &wire),
            Some((0x78, 0x1234, true))
        );
        Ok(())
    }
}
