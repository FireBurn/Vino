// SPDX-License-Identifier: GPL-2.0

//! HDCP 2.2 AKE wire layer: byte-exact message builders for the state machine.
//! The dock validates fixed per-message `sub_size` and `sub_len_dw` values, so
//! the vendor framing records those values explicitly rather than deriving them.
//!
//! OUT body layout (sec 5.1), after the 16-byte sec 3 transport header:
//! ```text
//!   body[0..2]   u16 sub_size      (fixed per message)
//!   body[2..4]   u16 = 0x0010
//!   body[4..8]   u32 hdcp_seq      increments 1..7 across the AKE OUT messages
//!   body[8..22]  14 zero bytes
//!   body[22..26] u32 = 0x00000030  marker
//!   body[26]     u8  = 0x00        flag
//!   body[27]     u8  = msg_id
//!   body[28..]   HDCP payload (zero-padded to the fixed body length)
//! ```

use super::*;

/// HDCP 2.2 message IDs (sec 5.3). `pub(crate)` so the AKE state machine
/// ([`super::VinoDriver::run_ake`]) can match on the response IDs too.
pub(crate) mod id {
    use kernel::drm::display::hdcp::MessageId;

    // Standard HDCP 2.2 message IDs: reuse the canonical values from
    // `<drm/display/drm_hdcp.h>` rather than redefining them, so vino stays in
    // lockstep with the kernel's HDCP definitions. Only the transport framing
    // around these (the DisplayLink type/sub/ctr header) is vino-specific.
    pub(crate) const AKE_INIT: u8 = MessageId::AKE_INIT.as_u8();
    pub(crate) const AKE_SEND_CERT: u8 = MessageId::AKE_SEND_CERT.as_u8();
    pub(crate) const AKE_NO_STORED_KM: u8 = MessageId::AKE_NO_STORED_KM.as_u8();
    pub(crate) const AKE_SEND_H_PRIME: u8 = MessageId::AKE_SEND_H_PRIME.as_u8();
    pub(crate) const LC_INIT: u8 = MessageId::LC_INIT.as_u8();
    pub(crate) const LC_SEND_L_PRIME: u8 = MessageId::LC_SEND_L_PRIME.as_u8();
    pub(crate) const SKE_SEND_EKS: u8 = MessageId::SKE_SEND_EKS.as_u8();
    pub(crate) const REPEATERAUTH_SEND_RECEIVERID_LIST: u8 =
        MessageId::REPEATERAUTH_SEND_RECEIVERID_LIST.as_u8();
    pub(crate) const REPEATERAUTH_SEND_ACK: u8 = MessageId::REPEATERAUTH_SEND_ACK.as_u8();
    pub(crate) const REPEATERAUTH_STREAM_MANAGE: u8 = MessageId::REPEATERAUTH_STREAM_MANAGE.as_u8();
    pub(crate) const REPEATERAUTH_STREAM_READY: u8 = MessageId::REPEATERAUTH_STREAM_READY.as_u8();

    // DisplayLink-specific message IDs with no `<drm/display/drm_hdcp.h>` equivalent
    // (the AKE_Send_rrx split and the transmitter/receiver-info + auth-status messages
    // the DL3 dock uses), kept as literals.
    pub(crate) const AKE_SEND_RRX: u8 = 0x06;
    pub(crate) const RECEIVER_AUTH_STATUS: u8 = 0x12;
    pub(crate) const AKE_TRANSMITTER_INFO: u8 = 0x13;
}

/// transport `sub_id` for HDCP OUT messages (type=4 sub=0x04, sec 5.1).
const SUB_HDCP: u16 = 0x04;

/// Allocate a `body_len`-byte zeroed body with the sec 5.1 header filled in
/// (`sub_size`, the `0x0010` marker, `hdcp_seq`, the `0x30` marker and `msg_id`).
/// The caller writes the payload into `body[28..]`.
fn body(body_len: usize, sub_size: u16, hdcp_seq: u32, msg_id: u8) -> Result<KVec<u8>> {
    let mut b = KVec::from_elem(0u8, body_len, GFP_KERNEL)?;
    b[0..2].copy_from_slice(&sub_size.to_le_bytes());
    b[2..4].copy_from_slice(&0x0010u16.to_le_bytes());
    b[4..8].copy_from_slice(&hdcp_seq.to_le_bytes());
    b[22..26].copy_from_slice(&0x0000_0030u32.to_le_bytes());
    b[27] = msg_id;
    Ok(b)
}

/// Wrap a finished HDCP body in the vendor transport header (type=4 sub=0x04)
/// with the message's fixed `sub_len_dw` and transport `seq`.
fn wrap(sub_len_dw: u16, seq: u32, body: &[u8]) -> Result<KVec<u8>> {
    let mut frame = KVec::with_capacity(16 + body.len(), GFP_KERNEL)?;
    proto::push_frame_with(&mut frame, 0x04, SUB_HDCP, sub_len_dw, seq, body)?;
    Ok(frame)
}

/// `session-init ACK` (the `id=0x14 sub=0x76` frame).
///
/// This precedes `AKE_Init`, so AKE uses `hdcp_seq` 2..8. It is a minimal
/// `0x14 / 0x76 / hdcp_seq` header: a 32-byte body without a message payload or the
/// `0x0010 / 0x30 / msg_id` trailer written by [`body`], wrapped with `sub_len_dw=0x0a`. The dock
/// echoes it as a `msg_id=0` status frame that `recv_hdcp` skips.
pub(super) fn session_init_ack(hdcp_seq: u32, seq: u32) -> Result<KVec<u8>> {
    let mut b = KVec::from_elem(0u8, 32, GFP_KERNEL)?;
    b[0..2].copy_from_slice(&0x0014u16.to_le_bytes());
    b[2..4].copy_from_slice(&0x0076u16.to_le_bytes());
    b[4..8].copy_from_slice(&hdcp_seq.to_le_bytes());
    wrap(0x000a, seq, &b)
}

/// `AKE_Init` (msg_id 0x02): `rtx[8] || TxCaps[3]`, padded to a 48-byte body
/// (`sub_size=0x22`, `sub_len_dw=0x0c`).
pub(super) fn ake_init(
    hdcp_seq: u32,
    seq: u32,
    rtx: &[u8; drm_hdcp::RTX_LEN],
    tx_caps: &[u8; 3],
) -> Result<KVec<u8>> {
    let mut b = body(48, 0x0022, hdcp_seq, id::AKE_INIT)?;
    b[28..36].copy_from_slice(rtx);
    b[36..39].copy_from_slice(tx_caps);
    wrap(0x000c, seq, &b)
}

/// `AKE_Transmitter_Info` (msg_id 0x13): byte-exact vendor framing
/// (`sub_size=0x1f`, `sub_len_dw=0x0f`), payload `00 06 02 00 02`.
pub(super) fn ake_transmitter_info(hdcp_seq: u32, seq: u32) -> Result<KVec<u8>> {
    let mut b = body(48, 0x001f, hdcp_seq, id::AKE_TRANSMITTER_INFO)?;
    b[28..33].copy_from_slice(&[0x00, 0x06, 0x02, 0x00, 0x02]);
    wrap(0x000f, seq, &b)
}

/// `AKE_No_Stored_km` (msg_id 0x04): the 128-byte RSA-OAEP-SHA256 `Ekpub(km)`
/// in a 160-byte body (`sub_size=0x9a`, `sub_len_dw=0x04`).
pub(super) fn ake_no_stored_km(
    hdcp_seq: u32,
    seq: u32,
    ekpub_km: &[u8; drm_hdcp::ENCRYPTED_MASTER_KEY_LEN],
) -> Result<KVec<u8>> {
    let mut b = body(160, 0x009a, hdcp_seq, id::AKE_NO_STORED_KM)?;
    b[28..156].copy_from_slice(ekpub_km);
    wrap(0x0004, seq, &b)
}

/// `LC_Init` (msg_id 0x09): `rn[8]` in a 48-byte body
/// (`sub_size=0x22`, `sub_len_dw=0x0c`).
pub(super) fn lc_init(hdcp_seq: u32, seq: u32, rn: &[u8; drm_hdcp::RN_LEN]) -> Result<KVec<u8>> {
    let mut b = body(48, 0x0022, hdcp_seq, id::LC_INIT)?;
    b[28..36].copy_from_slice(rn);
    wrap(0x000c, seq, &b)
}

/// `SKE_Send_Eks` (msg_id 0x0b): `Edkey(ks)[16] || riv[8]` in a 64-byte body
/// (`sub_size=0x32`, `sub_len_dw=0x0c`).
pub(super) fn ske_send_eks(
    hdcp_seq: u32,
    seq: u32,
    edkey_ks: &[u8; drm_hdcp::ENCRYPTED_SESSION_KEY_LEN],
    riv: &[u8; drm_hdcp::RIV_LEN],
) -> Result<KVec<u8>> {
    let mut b = body(64, 0x0032, hdcp_seq, id::SKE_SEND_EKS)?;
    b[28..44].copy_from_slice(edkey_ks);
    b[44..52].copy_from_slice(riv);
    wrap(0x000c, seq, &b)
}

/// `RepeaterAuth_Send_ACK` (msg_id 0x0f): the full `V[16]` in a 48-byte body
/// (`sub_size=0x2a`, `sub_len_dw=0x04`).
pub(super) fn repeater_auth_send_ack(
    hdcp_seq: u32,
    seq: u32,
    v: &[u8; drm_hdcp::V_PRIME_HALF_LEN],
) -> Result<KVec<u8>> {
    let mut b = body(48, 0x002a, hdcp_seq, id::REPEATERAUTH_SEND_ACK)?;
    b[28..44].copy_from_slice(v);
    wrap(0x0004, seq, &b)
}

/// `RepeaterAuth_Stream_Manage` SM2 (msg_id 0x10): `k=2`,
/// `StreamID_Type[0]=4` and `StreamID_Type[1]=5`.
pub(super) fn repeater_auth_stream_manage(hdcp_seq: u32, seq: u32) -> Result<KVec<u8>> {
    let mut b = body(48, 0x002d, hdcp_seq, id::REPEATERAUTH_STREAM_MANAGE)?;
    b[32..36].copy_from_slice(&[0x02, 0, 0, 0]); // k = 2 (LE)
    b[36..40].copy_from_slice(&[0x04, 0, 0, 0]); // StreamID_Type[0]
    b[43] = 0x05; // StreamID_Type[1]
    wrap(0x0001, seq, &b)
}

/// Parse an IN HDCP message body (sec 5.2): `body[8]` marker, `body[9]` msg_id,
/// `body[10..]` payload (for `AKE_Send_Cert`, `body[10]` is a version flag).
/// Returns `(msg_id, payload)`.
pub(super) fn parse_in(body: &[u8]) -> Option<(u8, &[u8])> {
    if body.len() < 10 {
        return None;
    }
    Some((body[9], &body[10..]))
}
