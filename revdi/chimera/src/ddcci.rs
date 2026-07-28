// SPDX-License-Identifier: GPL-2.0-or-later

//! Userspace DDC/CI tunnel used by Chimera's DisplayLinkManager replacement.
//!
//! This is intentionally not part of Vino's in-kernel protocol module. The kernel driver should
//! expose downstream monitor I2C only through the common I2C subsystem once the Rust adapter
//! bindings can express the required lifetime. Chimera still needs the vendor transaction while
//! replacing DisplayLinkManager in userspace, so that policy remains local to the daemon.

use crate::kshim::{rng, Result, EINVAL, EPROTO};
use crate::kvino;

/// DDC/CI's 7-bit monitor I2C address.
pub const I2C_ADDR: u16 = 0x37;

/// Build the vendor message that forwards one DDC/CI write to `head`.
pub fn forward(counter: u16, head: u8, payload: &[u8]) -> Result<Vec<u8>> {
    if payload.len() > 32 {
        return Err(EINVAL);
    }

    let mut message = Vec::with_capacity(64);
    header(&mut message, 0x36, 0x26, counter);
    message.resize(22, 0);
    message.push(head);
    message.push(payload.len() as u8);
    message.extend_from_slice(payload);
    message.resize(56, 0);

    let mut token = [0; 8];
    rng::fill(&mut token);
    message.extend_from_slice(&token);
    Ok(message)
}

/// Build the vendor request for the result of the preceding DDC/CI command.
pub fn read_request(counter: u16, head: u8) -> Vec<u8> {
    let mut message = Vec::with_capacity(32);
    header(&mut message, 0x15, 0x25, counter);
    message.resize(22, 0);
    message.push(head);

    let mut token = [0; 9];
    rng::fill(&mut token);
    message.extend_from_slice(&token);
    message
}

/// Decode an `id=0x20 sub=0x25` DDC/CI reply.
pub fn parse_reply(key: &[u8; 16], outbound_riv: &[u8; 8], wire: &[u8]) -> Result<Option<Vec<u8>>> {
    if wire.len() < 32
        || u32::from_le_bytes(wire[4..8].try_into().map_err(|_| EPROTO)?) != 4
        || u16::from_le_bytes(wire[8..10].try_into().map_err(|_| EPROTO)?) != 0x45
    {
        return Ok(None);
    }

    let sequence = u32::from_le_bytes(wire[12..16].try_into().map_err(|_| EPROTO)?);
    let encrypted = &wire[16..wire.len() - 16];
    for riv in inbound_reply_rivs(outbound_riv) {
        let Ok(plaintext) = kvino::open_in(key, &riv, sequence, encrypted) else {
            continue;
        };
        if plaintext.len() < 23 {
            continue;
        }

        let id = u16::from_le_bytes(plaintext[0..2].try_into().map_err(|_| EPROTO)?);
        let sub = u16::from_le_bytes(plaintext[2..4].try_into().map_err(|_| EPROTO)?);
        let padding = u16::from_le_bytes(plaintext[6..8].try_into().map_err(|_| EPROTO)?);
        if id != 0x20 || sub != 0x25 || padding != 0 {
            continue;
        }

        let length = plaintext[22] as usize;
        let end = 23usize.checked_add(length).ok_or(EPROTO)?;
        let payload = plaintext.get(23..end).ok_or(EPROTO)?;
        return Ok(Some(payload.to_vec()));
    }
    Ok(None)
}

fn header(message: &mut Vec<u8>, id: u16, sub: u16, counter: u16) {
    message.extend_from_slice(&id.to_le_bytes());
    message.extend_from_slice(&sub.to_le_bytes());
    message.extend_from_slice(&counter.to_le_bytes());
    message.extend_from_slice(&[0, 0]);
}

fn inbound_reply_rivs(outbound: &[u8; 8]) -> [[u8; 8]; 4] {
    let primary = kvino::in_riv(outbound);
    let mut primary_head_one = primary;
    primary_head_one[7] ^= 0x80;
    let mut outbound_head_one = *outbound;
    outbound_head_one[7] ^= 0x80;
    [primary, primary_head_one, *outbound, outbound_head_one]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forward_layout_matches_vendor_transaction() {
        let message = forward(0x1234, 1, &[0x51, 0x82, 0x01, 0x10]).unwrap();
        assert_eq!(message.len(), 64);
        assert_eq!(&message[..8], &[0x36, 0, 0x26, 0, 0x34, 0x12, 0, 0]);
        assert_eq!(&message[22..28], &[1, 4, 0x51, 0x82, 0x01, 0x10]);
        assert!(message[28..56].iter().all(|byte| *byte == 0));
    }
}
