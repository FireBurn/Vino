// SPDX-License-Identifier: GPL-2.0

//! Video decoder configuration carried in cold pipe-arm records.

use kernel::{alloc::flags::GFP_KERNEL, prelude::*};

const WIDE_TABLE_RECORD_LEN: u16 = 194;
const QUANT_TABLE_LEN: u16 = 82;

/// Record kind of a code table, which also fixes how its values are laid out.
const WIDE_TABLE_KIND: u16 = 0x000d;
const NARROW_TABLE_KIND: u16 = 0x0009;

/// Record kind of the quantiser table, the same on every generation.
const QUANT_TABLE_KIND: u16 = 0x000a;

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

// The same five tables for a DL-3x00 decoder, which states them as a counted list of 16-bit
// values rather than a version word and 47 32-bit ones. The code they describe is a different one:
// where the wider form carries a run of unary prefixes, this carries a plain power-of-two ladder.
const NARROW_CODE_TABLES: [&[u16]; 5] = [
    &[
        1, 0, 2, 0, 4, 0, 8, 0, 16, 0, 32, 0, 64, 0, 128, 0, 256, 512,
    ],
    &[
        1, 0, 2, 0, 4, 0, 8, 0, 16, 0, 32, 0, 64, 0, 128, 0, 256, 0, 512, 1024,
    ],
    &[
        1, 0, 2, 0, 4, 0, 8, 0, 16, 0, 32, 0, 64, 0, 128, 0, 256, 0, 512, 1024,
    ],
    &[1, 0, 2, 0, 4, 0, 8, 15, 2],
    &[1, 0, 2, 0, 4, 0, 8, 0, 16, 0, 32, 0, 64, 127, 2],
];

/// The decoder code tables one dock generation states when it opens a stream.
///
/// The configuration record is otherwise identical everywhere -- the same mode header opens it and
/// the same quantiser table closes it -- so only the code tables select a variant.
///
/// This also selects the dialect the encoder emits, because the two must agree: a dock told one
/// code and sent another decodes every strip to noise while each record stays exactly the right
/// length. Keeping both behind one field is what makes disagreeing impossible.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum CodeTables {
    /// Five 47-entry 32-bit tables, each with a version word, under record kind 0x0d.
    Wide,
    /// Five counted 16-bit tables under record kind 0x09.
    Narrow,
}

// Decoder quantization parameters. These match the Haar configuration used by the video encoder.
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

/// The 26-byte `[len=0x0018][kind=0x030b]` header that states a stream's mode.
///
/// It opens the decoder configuration and is repeated verbatim by the mode-restating form of the
/// per-frame stream report, so both build it here. The mode appears twice, each time as
/// `[0x0002][width][height][layout word]`.
pub(super) fn mode_header(width: u16, height: u16, layout_word: u16) -> [u8; 26] {
    let mut out = [0u8; 26];
    for (i, value) in [
        0x0018u16,
        0x030b,
        0x0204,
        0x0002,
        0x0002,
        width,
        height,
        layout_word,
        0x0002,
        width,
        height,
        layout_word,
        0,
    ]
    .into_iter()
    .enumerate()
    {
        out[i * 2..i * 2 + 2].copy_from_slice(&value.to_le_bytes());
    }
    out
}

/// Build the plaintext decoder configuration a connector's stream opens with.
///
/// The configuration opens with the stream's [`mode_header`] and closes with the quantiser table;
/// `tail` is whatever the generation appends after that, which for some is a host-random nonce and
/// for others nothing at all.
pub(super) fn build_config(
    tables: CodeTables,
    mode_header: &[u8; 26],
    tail: &[u8],
) -> Result<KVec<u8>> {
    let mut out = KVec::new();

    out.extend_from_slice(mode_header, GFP_KERNEL)?;

    match tables {
        CodeTables::Wide => {
            for (index, table) in CODE_TABLES.iter().enumerate() {
                push_u16(&mut out, WIDE_TABLE_RECORD_LEN)?;
                push_u16(&mut out, ((index as u16) << 8) | WIDE_TABLE_KIND)?;
                push_u32(&mut out, 1)?;
                for &value in table {
                    push_u32(&mut out, value)?;
                }
            }
        }
        CodeTables::Narrow => {
            for (index, table) in NARROW_CODE_TABLES.iter().enumerate() {
                // The record length counts everything after itself: the kind word, the count
                // word, and the values.
                push_u16(&mut out, 4 + 2 * table.len() as u16)?;
                push_u16(&mut out, ((index as u16) << 8) | NARROW_TABLE_KIND)?;
                push_u16(&mut out, table.len() as u16)?;
                for &value in table.iter() {
                    push_u16(&mut out, value)?;
                }
            }
        }
    }

    push_u16(&mut out, QUANT_TABLE_LEN)?;
    debug_assert_eq!(QUANT_TABLE[0], QUANT_TABLE_KIND);
    for value in QUANT_TABLE {
        push_u16(&mut out, value)?;
    }
    out.extend_from_slice(tail, GFP_KERNEL)?;
    Ok(out)
}
