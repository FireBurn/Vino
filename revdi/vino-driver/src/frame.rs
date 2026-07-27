//! Builder/parser for the 16-byte DLM USB framing.
//!
//! ```text
//! 0-1   pad (zeros)
//! 2-3   u16 size       = total bytes - 4
//! 4-7   u32 type
//! 8-9   u16 sub_id
//! 10-11 u16 sub_len_dw (body length in dwords)
//! 12-15 u32 seq
//! 16+   body
//! ```

#[derive(Debug, Clone)]
pub struct Frame {
    pub msg_type: u32,
    pub sub_id: u16,
    pub sub_len_dw: u16,
    pub seq: u32,
    pub body: Vec<u8>,
}

impl Frame {
    /// Build a complete USB bulk payload from this Frame.
    pub fn encode(&self) -> Vec<u8> {
        let total = 16 + self.body.len();
        // size = total - 4
        let size_field = (total - 4) as u16;
        let mut out = Vec::with_capacity(total);
        out.extend_from_slice(&[0, 0]); // pad
        out.extend_from_slice(&size_field.to_le_bytes());
        out.extend_from_slice(&self.msg_type.to_le_bytes());
        out.extend_from_slice(&self.sub_id.to_le_bytes());
        out.extend_from_slice(&self.sub_len_dw.to_le_bytes());
        out.extend_from_slice(&self.seq.to_le_bytes());
        out.extend_from_slice(&self.body);
        out
    }

    pub fn decode(buf: &[u8]) -> Option<Frame> {
        if buf.len() < 16 {
            return None;
        }
        let size = u16::from_le_bytes([buf[2], buf[3]]) as usize;
        let msg_type = u32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]);
        let sub_id = u16::from_le_bytes([buf[8], buf[9]]);
        let sub_len_dw = u16::from_le_bytes([buf[10], buf[11]]);
        let seq = u32::from_le_bytes([buf[12], buf[13], buf[14], buf[15]]);
        // body length = size - 12 (size counts everything from byte 4 onwards minus the leading 4 of header)
        // Total message bytes = size + 4. Header is 16 bytes. So body = size + 4 - 16 = size - 12.
        let body_len = size.checked_sub(12)?;
        if buf.len() < 16 + body_len {
            return None;
        }
        Some(Frame {
            msg_type,
            sub_id,
            sub_len_dw,
            seq,
            body: buf[16..16 + body_len].to_vec(),
        })
    }
}

/// Convenience constructor for OUT frames.
pub fn build_frame(msg_type: u32, sub_id: u16, seq: u32, body: Vec<u8>) -> Frame {
    let body_dw = (body.len() / 4) as u16;
    Frame {
        msg_type,
        sub_id,
        sub_len_dw: body_dw,
        seq,
        body,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_init_message() {
        // type=2 sub=0x04 init message
        let f = build_frame(
            2,
            0x04,
            0,
            vec![0x04, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        );
        let encoded = f.encode();
        // 16-byte header + 16-byte body = 32 bytes, size field = 28
        assert_eq!(encoded.len(), 32);
        assert_eq!(encoded[2..4], [28, 0]); // size = 28
        assert_eq!(encoded[4..8], [2, 0, 0, 0]); // type = 2
        assert_eq!(encoded[8..10], [4, 0]); // sub_id = 4
        let decoded = Frame::decode(&encoded).unwrap();
        assert_eq!(decoded.msg_type, 2);
        assert_eq!(decoded.sub_id, 4);
        assert_eq!(decoded.body.len(), 16);
    }
}
