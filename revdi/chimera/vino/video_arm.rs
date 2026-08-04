// SPDX-License-Identifier: GPL-2.0

//! Video decoder configuration carried in cold pipe-arm records.

use kernel::{alloc::flags::GFP_KERNEL, prelude::*};

const CONFIG_LEN: usize = 1104;
const TABLE_RECORD_LEN: u16 = 194;
const QUANT_TABLE_LEN: u16 = 82;

// Five decoder code tables follow the mode header. Each record contains a table index, a version
// word, and 47 little-endian values.
const CODE_TABLES: [[u32; 47]; 5] = [
    [
        0, 6, 0, 28, 0, 120, 0, 496, 0, 2016, 0, 8128, 0, 32640, 0, 130816, 262144, 0, 0, 0, 0, 0,
        0, 0, 0, 3, 0, 21, 0, 105, 0, 465, 0, 1953, 0, 8001, 0, 32385, 0, 130305, 261121, 0, 0, 0,
        0, 0, 0,
    ],
    [
        0, 6, 0, 28, 0, 120, 0, 496, 0, 2016, 0, 8128, 0, 32640, 0, 130816, 0, 523776, 1048576, 0,
        0, 0, 0, 0, 0, 3, 0, 21, 0, 105, 0, 465, 0, 1953, 0, 8001, 0, 32385, 0, 130305, 0, 522753,
        1046529, 0, 0, 0, 0,
    ],
    [
        0, 6, 0, 28, 0, 120, 0, 496, 0, 2016, 0, 8128, 0, 32640, 0, 130816, 0, 523776, 1048576, 0,
        0, 0, 0, 0, 0, 3, 0, 21, 0, 105, 0, 465, 0, 1953, 0, 8001, 0, 32385, 0, 130305, 0, 522753,
        1046529, 0, 0, 0, 0,
    ],
    [
        0, 6, 0, 28, 0, 120, 255, 512, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 3, 0, 21,
        0, 105, 225, 480, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    ],
    [
        0, 6, 0, 28, 0, 120, 0, 496, 0, 2016, 0, 8128, 16383, 32768, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 3, 0, 21, 0, 105, 0, 465, 0, 1953, 0, 8001, 16129, 32512, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    ],
];

// Decoder quantization parameters. These match the WHT configuration used by the video encoder.
const QUANT_TABLE: [u16; 41] = [
    10, 1, 1, 0, 64, 64, 16, 16, 16, 16, 16, 16, 16, 32, 32, 32, 1, 1, 1, 16, 16, 4, 16, 16, 4, 32,
    32, 8, 1, 1, 1, 32, 32, 2, 32, 32, 2, 64, 64, 4, 0,
];

fn push_u16(out: &mut KVec<u8>, value: u16) -> Result {
    out.extend_from_slice(&value.to_le_bytes(), GFP_KERNEL)?;
    Ok(())
}

fn push_u32(out: &mut KVec<u8>, value: u32) -> Result {
    out.extend_from_slice(&value.to_le_bytes(), GFP_KERNEL)?;
    Ok(())
}

/// Build the plaintext decoder configuration for a cold pipe-arm record.
pub(super) fn build(width: u16, height: u16, nonce: &[u8; 14]) -> Result<KVec<u8>> {
    build_with_layout_word(width, height, 0x4000, nonce)
}

/// Build a plaintext decoder configuration with the captured repeated layout word.
///
/// Ridge uses `0x4000`.  The DL7400's independently authenticated startup records use `0x2100`
/// at 2560x1440.  Its semantic name is not established, so callers must pass an observed value
/// rather than treating it as a generic pitch calculation.
/// The 26-byte `[len=0x0018][kind=0x030b]` header that states a stream's mode.
///
/// It opens the decoder configuration and is repeated verbatim by the mode-restating form of the
/// per-frame stream report, so both build it here. The mode appears twice, each time as
/// `[0x0002][width][height][layout word]`.
pub(super) fn mode_header(width: u16, height: u16, layout_word: u16) -> [u8; 26] {
    let mut out = [0u8; 26];
    for (i, value) in [
        0x0018u16, 0x030b, 0x0204, 0x0002, 0x0002, width, height, layout_word, 0x0002, width,
        height, layout_word, 0,
    ]
    .into_iter()
    .enumerate()
    {
        out[i * 2..i * 2 + 2].copy_from_slice(&value.to_le_bytes());
    }
    out
}

pub(super) fn build_with_layout_word(
    width: u16,
    height: u16,
    layout_word: u16,
    nonce: &[u8; 14],
) -> Result<KVec<u8>> {
    let mut out = KVec::with_capacity(CONFIG_LEN, GFP_KERNEL)?;

    out.extend_from_slice(&mode_header(width, height, layout_word), GFP_KERNEL)?;

    for (index, table) in CODE_TABLES.iter().enumerate() {
        push_u16(&mut out, TABLE_RECORD_LEN)?;
        push_u16(&mut out, ((index as u16) << 8) | 0x000d)?;
        push_u32(&mut out, 1)?;
        for &value in table {
            push_u32(&mut out, value)?;
        }
    }

    push_u16(&mut out, QUANT_TABLE_LEN)?;
    for value in QUANT_TABLE {
        push_u16(&mut out, value)?;
    }
    out.extend_from_slice(nonce, GFP_KERNEL)?;
    debug_assert_eq!(out.len(), CONFIG_LEN);
    Ok(out)
}
