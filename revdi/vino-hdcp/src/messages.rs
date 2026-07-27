//! HDCP 2.2 message types and their layouts.
//!
//! In the DL3 transport, each HDCP message is wrapped by a DLM inner header
//! (8 bytes: `size sub_type 30 00 00 00`) which itself sits inside a TLV
//! envelope (16 bytes per `vino-transport::frame`). This module deals only
//! with the innermost HDCP message bytes — the wrapping is handled by the
//! caller.

/// HDCP 2.2 message IDs (from §2.3 of the spec).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum MessageId {
    AkeInit = 0x02,
    AkeSendCert = 0x03,
    AkeNoStoredKm = 0x04,
    AkeStoredKm = 0x05,
    AkeSendRrx = 0x06,
    AkeSendHPrime = 0x07,
    AkeSendPairingInfo = 0x08,
    LcInit = 0x09,
    LcSendLPrime = 0x0a,
    SkeSendEks = 0x0b,
    RepeaterAuthSendReceiverIdList = 0x0c,
    RepeaterAuthSendAck = 0x0f,
    RepeaterAuthStreamManage = 0x10,
    RepeaterAuthStreamReady = 0x11,
    ReceiverAuthStatus = 0x12,
    AkeTransmitterInfo = 0x13,
    AkeReceiverInfo = 0x14,
}

impl MessageId {
    pub fn from_u8(v: u8) -> Option<Self> {
        use MessageId::*;
        Some(match v {
            0x02 => AkeInit,
            0x03 => AkeSendCert,
            0x04 => AkeNoStoredKm,
            0x05 => AkeStoredKm,
            0x06 => AkeSendRrx,
            0x07 => AkeSendHPrime,
            0x08 => AkeSendPairingInfo,
            0x09 => LcInit,
            0x0a => LcSendLPrime,
            0x0b => SkeSendEks,
            0x0c => RepeaterAuthSendReceiverIdList,
            0x0f => RepeaterAuthSendAck,
            0x10 => RepeaterAuthStreamManage,
            0x11 => RepeaterAuthStreamReady,
            0x12 => ReceiverAuthStatus,
            0x13 => AkeTransmitterInfo,
            0x14 => AkeReceiverInfo,
            _ => return None,
        })
    }
}

/// One parsed HDCP message.
#[derive(Debug, Clone)]
pub enum HdcpMessage {
    /// Tx → Rx, 11 bytes payload (rtx[8] + TxCaps[3])
    AkeInit { rtx: [u8; 8], tx_caps: [u8; 3] },
    /// Rx → Tx, 534 bytes payload (cert_rx[522] + rrx[8] + RxCaps[3])
    AkeSendCert {
        cert_rx: Vec<u8>,
        rrx: [u8; 8],
        rx_caps: [u8; 3],
    },
    /// Tx → Rx, 128 bytes payload (Ekpub_km, RSA-OAEP'd master key)
    AkeNoStoredKm { ekpub_km: Vec<u8> },
    /// Rx → Tx, 32 bytes payload (H'[32], HMAC over (rtx || RxCaps || TxCaps))
    AkeSendHPrime { h_prime: [u8; 32] },
    /// Rx → Tx, 16 bytes payload (Ekh_km — stored pairing info)
    AkeSendPairingInfo { ekh_km: [u8; 16] },
    /// Other / unparsed
    Unknown { id: u8, body: Vec<u8> },
}

impl HdcpMessage {
    /// Parse a raw HDCP message: 1 byte msg_id + payload.
    pub fn from_bytes(buf: &[u8]) -> Option<Self> {
        if buf.is_empty() {
            return None;
        }
        let id = buf[0];
        let payload = &buf[1..];
        Some(match MessageId::from_u8(id)? {
            MessageId::AkeInit => {
                if payload.len() < 11 {
                    return None;
                }
                let mut rtx = [0u8; 8];
                let mut tx_caps = [0u8; 3];
                rtx.copy_from_slice(&payload[0..8]);
                tx_caps.copy_from_slice(&payload[8..11]);
                HdcpMessage::AkeInit { rtx, tx_caps }
            }
            MessageId::AkeSendCert => {
                if payload.len() < 533 {
                    return None;
                }
                let cert_rx = payload[0..522].to_vec();
                let mut rrx = [0u8; 8];
                let mut rx_caps = [0u8; 3];
                rrx.copy_from_slice(&payload[522..530]);
                rx_caps.copy_from_slice(&payload[530..533]);
                HdcpMessage::AkeSendCert {
                    cert_rx,
                    rrx,
                    rx_caps,
                }
            }
            MessageId::AkeNoStoredKm => HdcpMessage::AkeNoStoredKm {
                ekpub_km: payload.to_vec(),
            },
            MessageId::AkeSendHPrime => {
                if payload.len() < 32 {
                    return None;
                }
                let mut h_prime = [0u8; 32];
                h_prime.copy_from_slice(&payload[0..32]);
                HdcpMessage::AkeSendHPrime { h_prime }
            }
            MessageId::AkeSendPairingInfo => {
                if payload.len() < 16 {
                    return None;
                }
                let mut ekh_km = [0u8; 16];
                ekh_km.copy_from_slice(&payload[0..16]);
                HdcpMessage::AkeSendPairingInfo { ekh_km }
            }
            _ => HdcpMessage::Unknown {
                id,
                body: payload.to_vec(),
            },
        })
    }
}
