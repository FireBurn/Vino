// SPDX-License-Identifier: GPL-2.0

//! Framing strips into the records a frame is made of.
//!
//! Record stride, padding, sequence and sub-band coordinates are all checked by the dock, and
//! a frame whose records are mis-framed is accepted byte for byte and displayed as nothing.

use super::*;

/// Write the sixteen-byte header every record on every one of these docks begins with.
///
/// ```text
/// off 0..2   zero
/// off 2..4   size : u16   the stride less four, so a record ends at `size + 4`
/// off 4..8   type : u32   2 for a plaintext marker, 4 for everything else
/// off 8..10  sub  : u16   the plane: a connector for video, a control sub otherwise
/// off 10..12 aux  : u16   the trailing pad count on an image record, a subtype on others
/// off 12..16 seq  : u32   the sealed stream's AES-CTR block counter, else zero
/// ```
///
/// Verified unchanged across all three generations against 92,072 captured records, which is
/// why it is written once. `out` must be the whole record: its length fixes `size`.
pub(crate) fn record_header(out: &mut [u8], kind: u32, sub: u16, aux: u16, seq: u32) {
    let size = (out.len() - 4) as u16;
    out[0..2].fill(0);
    out[2..4].copy_from_slice(&size.to_le_bytes());
    out[4..8].copy_from_slice(&kind.to_le_bytes());
    out[8..10].copy_from_slice(&sub.to_le_bytes());
    out[10..12].copy_from_slice(&aux.to_le_bytes());
    out[12..16].copy_from_slice(&seq.to_le_bytes());
}
/// The shape of an encoded frame, read back off the records themselves.
///
/// Returns `(records, largest record stride, largest strip)`. The wire invariants are that a
/// stride never passes the 4080-byte cap and that a strip fits inside a record; the sizes DLM
/// reaches for comparison are 1758 bytes per strip on DL-6xxx, 1780 on the DL-7400 and 2036 on
/// DL-3x00. Reported when a frame fails to reach the dock, because a dock refuses a malformed
/// record by halting the endpoint several transfers later, where it is indistinguishable from
/// any other transport fault.
pub(crate) fn record_stats(chunks: &[KVec<u8>]) -> (usize, usize, usize) {
    let (mut records, mut max_stride, mut max_strip) = (0usize, 0usize, 0usize);
    for chunk in chunks {
        let mut off = 0usize;
        while off + 16 <= chunk.len() {
            let size = u16::from_le_bytes([chunk[off + 2], chunk[off + 3]]) as usize;
            let stride = size + 4;
            if stride < 16 || off + stride > chunk.len() {
                break;
            }
            let aux = u16::from_le_bytes([chunk[off + 10], chunk[off + 11]]) as usize;
            let body = &chunk[off + 16..off + stride];
            let payload = body.len().saturating_sub(aux);
            let mut at = 0usize;
            while at + 2 <= payload {
                let len = u16::from_le_bytes([body[at], body[at + 1]]) as usize;
                if len == 0 || at + 2 + len > payload {
                    break;
                }
                max_strip = max_strip.max(len);
                at += 2 + len;
            }
            max_stride = max_stride.max(stride);
            records += 1;
            off += stride;
        }
    }
    (records, max_stride, max_strip)
}
/// Frame a raster-ordered list of strip bodies into EP08 records:
///
/// ```text
/// record (one per single-Y band of strips):
///   u16 pad   = 0
///   u16 size  = total record length (TLV..trailer, excludes the inter-record gap)
///   u32 type  = 4
///   u16 sub   = connector | (((y / 16) & 1) << 4)
///   u16 aux   = zero-padding byte count
///   u32 fseq  = 0
///   per strip: u16 strip_id (== strip length) ++ strip bytes
///   u8[aux]   zero padding so the complete `size + 4` stride is 16-byte aligned
/// ```
///
/// The record stream is chunked internally into small record-aligned buffers. The sink streams
/// those fragments directly into persistent 65536-byte URBs, so the internal boundaries are
/// not USB framing. Complete record strides are limited to 4080 bytes.
pub(crate) fn frame_records(
    geometry: Geometry,
    strips: &[KVec<u8>],
    connector: u8,
) -> Result<KVec<KVec<u8>>> {
    frame_records_with_boundary(geometry, strips, connector, None)
}
/// Frame a full live Navarro surface with the ordinary DLM producer order. Other modes and
/// damage subsets deliberately fall back to the generic interlaced order: the measured
/// permutation and its split-worker boundaries describe exactly 3600 128x8 strips.
pub(crate) fn frame_records_navarro_ordinary(
    geometry: Geometry,
    strips: &[KVec<u8>],
    connector: u8,
) -> Result<KVec<KVec<u8>>> {
    frame_records_with_boundary(
        geometry,
        strips,
        connector,
        (strips.len() == 3600).then_some(true),
    )
}
pub(crate) const NAVARRO_PROLOGUE_ROWS: &[u8] = &[
    0, 2, 4, 6, 8, 10, 12, 14, 16, 18, 20, 22, 24, 26, 28, 29, 31, 33, 35, 37, 39, 41, 43, 45, 47,
    49, 51, 53, 54, 57, 59, 60, 62, 64, 66, 68, 70, 72, 74, 76, 78, 80, 82, 84, 86, 88, 90, 92, 94,
    96, 98, 100, 1, 3, 5, 7, 9, 11, 13, 15, 17, 19, 21, 23, 25, 27, 30, 32, 34, 36, 38, 40, 42, 44,
    46, 48, 50, 52, 55, 56, 58, 61, 63, 65, 67, 69, 71, 73, 75, 77, 79, 81, 83, 85, 87, 89, 91, 93,
    95, 97, 99, 101, 102, 103, 110, 112, 114, 116, 118, 120, 122, 124, 126, 128, 130, 132, 134,
    136, 138, 140, 142, 144, 146, 148, 150, 152, 154, 156, 158, 160, 162, 164, 166, 168, 170, 172,
    174, 176, 178, 100, 104, 105, 106, 107, 108, 109, 111, 113, 115, 117, 119, 121, 123, 125, 127,
    129, 131, 133, 135, 137, 139, 141, 143, 145, 147, 149, 151, 153, 155, 157, 159, 161, 163, 165,
    167, 169, 171, 173, 175, 177, 179,
];

// Producer band order for a 1920x1080 DL-3x00 surface: 30 strips across x 68 bands of 64x16.
// Taken from DLM's own stream. Like the DL7400, this dock stops draining at the first band it
// did not expect, so the generic even-then-odd interlace is not close enough -- it diverges at
// the second band, sending 2 where the dock wants 3. No band appears twice, so unlike the
// DL7400 there are no split-row producer boundaries to reproduce.
/// The DL-3x00 producer band order, exposed so a selftest can pin it.
pub(crate) fn ella_rows_1080p() -> &'static [u8] {
    ELLA_ROWS_1080P
}
pub(crate) const ELLA_ROWS_1080P: &[u8] = &[
    0, 3, 5, 7, 9, 11, 13, 16, 18, 21, 23, 25, 27, 29, 31, 33, 35, 37, 39, 41, 43, 44, 46, 48, 50,
    51, 53, 55, 56, 58, 60, 61, 63, 65, 66, 1, 2, 4, 6, 8, 10, 12, 14, 15, 17, 19, 20, 22, 24, 26,
    28, 30, 32, 34, 36, 38, 40, 42, 45, 47, 49, 52, 54, 57, 59, 62, 64, 67,
];

pub(crate) const NAVARRO_ORDINARY_ROWS: &[u8] = &[
    1, 2, 4, 6, 8, 10, 12, 14, 16, 18, 20, 22, 24, 26, 29, 31, 33, 35, 37, 39, 41, 43, 45, 47, 49,
    51, 53, 55, 57, 59, 61, 63, 64, 66, 68, 70, 72, 73, 75, 77, 79, 81, 83, 85, 87, 89, 91, 93, 95,
    97, 99, 0, 3, 5, 7, 9, 11, 13, 15, 17, 19, 21, 23, 25, 27, 28, 30, 32, 34, 36, 38, 40, 42, 44,
    46, 48, 50, 52, 54, 56, 58, 60, 62, 65, 67, 69, 71, 74, 76, 78, 80, 82, 84, 86, 88, 90, 92, 94,
    96, 98, 100, 102, 99, 101, 102, 103, 105, 107, 109, 111, 113, 115, 117, 118, 120, 122, 125,
    127, 129, 131, 133, 135, 137, 139, 141, 144, 146, 148, 150, 152, 154, 156, 158, 160, 162, 164,
    166, 168, 170, 172, 174, 176, 178, 101, 104, 106, 108, 110, 112, 114, 116, 119, 121, 123, 124,
    126, 128, 130, 132, 134, 136, 138, 140, 142, 143, 145, 147, 149, 151, 153, 155, 157, 159, 161,
    163, 165, 167, 169, 171, 173, 175, 177, 179,
];

pub(crate) fn frame_records_with_boundary(
    geometry: Geometry,
    strips: &[KVec<u8>],
    connector: u8,
    navarro_ordinary: Option<bool>,
) -> Result<KVec<KVec<u8>>> {
    let Geometry {
        band_parity_bit,
        interlaced_bands,
        ..
    } = geometry;
    // Both platforms count an image record's 0..15 padding bytes in `aux`. Navarro also uses
    // `aux` as a subtype on non-image records; its fixed black carriers masked the image rule
    // because their 4048-byte strides require no padding.
    let aux_is_pad_count = true;
    const PREFIX: usize = 8;
    const STRIDE_CAP: usize = 0x0ff0;
    // Allocation boundary only, not wire framing.
    const CHUNK: usize = 0x4000;
    let mut frames: KVec<KVec<u8>> = KVec::new();
    let mut chunk: KVec<u8> = KVec::new();
    // Interlaced ordering sends even bands before odd bands while preserving x order.
    let mut order: KVec<usize> = KVec::with_capacity(strips.len(), GFP_KERNEL)?;
    // The rows below are a producer permutation of a 2560x1440 Navarro surface: 20 strips
    // across x 180 bands, its strips being 128x8. A strip count alone cannot select it --
    // Ridge at the same resolution is also exactly 3600 strips, 40 across x 90 bands -- and
    // both black carriers reach here on every dock, so match the layout itself.
    let navarro_layout = geometry.interlaced_bands
        && geometry.strip_w() == STRIP_BLOCKS * DIM
        && strips.len() == 3600;
    // 64-wide strips with interlaced bands is DL-3x00 and nothing else: Ridge is 64 wide but
    // raster, the DL7400 is 128 wide. Deliberately not conditioned on the strip count -- a
    // partial frame has to reach the dock in the same order a full one would.
    let ella_layout = geometry.interlaced_bands && geometry.strip_w() == DIM * 8;
    let navarro_rows = match navarro_ordinary {
        Some(false) if navarro_layout => Some(NAVARRO_PROLOGUE_ROWS),
        Some(true) if navarro_layout => Some(NAVARRO_ORDINARY_ROWS),
        _ => None,
    };
    if let Some(rows) = navarro_rows {
        // 2560 px / 128 px per strip on the DL7400, 1920 / 64 on the DL-3x00. Guaranteed by
        // the layout checks above.
        let strips_across = if ella_layout { 30 } else { 20 };
        let ordinary = navarro_ordinary == Some(true);
        for (run, &y) in rows.iter().enumerate() {
            // The DL-3x00 order carries whole bands; only the DL7400 splits rows at its
            // producer boundaries.
            let (x0, x1) = if ella_layout {
                (0, strips_across)
            } else if ordinary {
                match run {
                    50 => (0, 8),
                    101 => (0, 8),
                    102 => (8, 20),
                    103 => (0, 4),
                    104 => (8, 20),
                    143 => (4, 20),
                    _ => (0, 20),
                }
            } else {
                match run {
                    51 => (0, 4),
                    139 => (4, 20),
                    _ => (0, 20),
                }
            };
            for x in x0..x1 {
                order.push(y as usize * strips_across + x, GFP_KERNEL)?;
            }
        }
    } else if ella_layout {
        // Order by where each strip's band sits in the producer table, read from the strip
        // itself rather than from an assumed full-surface layout. That is what lets a frame
        // carry a *subset* of the surface and still arrive in the order the dock expects --
        // which it must, because this dock will not take a whole surface in one frame.
        let rank = |s: &KVec<u8>| -> usize {
            let band = (strip_y(s) >> geometry.strip_h_shift()) as u8;
            ELLA_ROWS_1080P
                .iter()
                .position(|&b| b == band)
                .unwrap_or(usize::MAX)
        };
        let mut idx: KVec<usize> = KVec::with_capacity(strips.len(), GFP_KERNEL)?;
        for n in 0..strips.len() {
            idx.push(n, GFP_KERNEL)?;
        }
        // Insertion sort: the comparison is a table lookup and the driver has no sort in
        // scope; a frame's strip count here is bounded by the dock's frame ceiling.
        for a in 1..idx.len() {
            let mut b = a;
            while b > 0 {
                let (l, r) = (idx[b - 1], idx[b]);
                let key = |n: usize| (rank(&strips[n]), strip_x(&strips[n]));
                if key(l) <= key(r) {
                    break;
                }
                idx.swap(b - 1, b);
                b -= 1;
            }
        }
        for n in idx {
            order.push(n, GFP_KERNEL)?;
        }
    } else if interlaced_bands {
        for pass in 0..2u16 {
            for (n, s) in strips.iter().enumerate() {
                if (strip_y(s) >> geometry.strip_h_shift()) & 1 == pass {
                    order.push(n, GFP_KERNEL)?;
                }
            }
        }
    } else {
        for n in 0..strips.len() {
            order.push(n, GFP_KERNEL)?;
        }
    }
    let mut i = 0usize;
    while i < order.len() {
        let y0 = strip_y(&strips[order[i]]);
        let mut record: KVec<u8> = KVec::new();
        // Sub bit 4 carries the y-band parity when the selected framing
        // profile requires it.
        record.extend_from_slice(&[0u8; 8 + PREFIX], GFP_KERNEL)?;
        let parity = u16::from(band_parity_bit) & ((y0 >> geometry.strip_h_shift()) & 1);
        // Once a stream is running the vendor marks every image record here. Its frames and
        // vino's are otherwise byte-identical -- same size, type, aux, sequence and payload --
        // so this one bit is the whole difference between a stream the dock keeps taking and
        // one it stops accepting with the endpoint still reporting healthy.
        let sub = u16::from(geometry.connector_selector(connector))
            | (parity << 4)
            | u16::from(geometry.steady_sub_bit);
        let mut n = 0usize;
        // A record ends at a y-band boundary only where the band is part of its identity.
        // Ridge carries the band parity in `sub`, so a record cannot span two bands; Navarro
        // does not, and fills each record to the stride cap instead.
        while i < order.len() && (!band_parity_bit || strip_y(&strips[order[i]]) == y0) {
            // Preserve DLM's captured producer flushes exactly. They are not a uniform
            // 1024-strip rule: applying that to the final 1552-strip segment added a record
            // at 3072 and made the prologue 210064 bytes instead of 210048. The ordinary
            // carrier also schedules the two 1024-strip producers differently, so it has its
            // own three boundaries and one extra image record (208624 bytes total).
            let producer_boundary = match navarro_ordinary {
                Some(false) => matches!(i, 1024 | 2048 | 2764),
                Some(true) => matches!(i, 2032 | 2048 | 2804),
                None => false,
            };
            if interlaced_bands && n > 0 && producer_boundary {
                break;
            }
            let s = &strips[order[i]];
            let projected = record.len() + 2 + s.len();
            let projected_aligned = (projected + 15) & !15;
            if projected_aligned > STRIDE_CAP {
                if n > 0 {
                    break;
                }
                // A strip too big for a record of its own cannot be framed at all. Emitting it
                // anyway produces a record whose stride is over the cap, which a dock answers
                // by halting the endpoint -- a failure indistinguishable from any other
                // transport fault, surfacing several transfers later and blamed on the
                // transport. Refuse the frame instead, so the encoder is what gets looked at.
                pr_err!(
                    "vino: strip at {}x{} encoded to {} B, over the {} a record can carry\n",
                    strip_x(s),
                    strip_y(s),
                    s.len(),
                    STRIDE_CAP - 18
                );
                return Err(kernel::error::code::EOVERFLOW);
            }
            record.extend_from_slice(&(s.len() as u16).to_le_bytes(), GFP_KERNEL)?;
            record.extend_from_slice(s, GFP_KERNEL)?;
            n += 1;
            i += 1;
        }
        // The wire format pads each complete record stride to 16 bytes. There is no
        // additional trailer or inter-record gap: `size` counts from after the four-byte
        // pad+size prefix, so `stride = size + 4` lands on the next record.
        //
        // Ridge carries the pad count in `aux`. Navarro does not: there `aux` names a record
        // type, and every one of its image records carries zero, so a pad count written there
        // would be read as some other kind of record entirely.
        let pad = (16 - (record.len() & 15)) & 15;
        record.extend_from_slice(&[0u8; 15][..pad], GFP_KERNEL)?;
        // Written once the strips and the padding are in, because the stride the header
        // states is the length of the finished record.
        let aux = if aux_is_pad_count { pad as u16 } else { 0 };
        record_header(&mut record, 4, sub, aux, 0);

        if !chunk.is_empty() && chunk.len() + record.len() > CHUNK {
            frames.push(chunk, GFP_KERNEL)?;
            chunk = KVec::new();
        }
        chunk.extend_from_slice(&record, GFP_KERNEL)?;
    }
    if !chunk.is_empty() {
        frames.push(chunk, GFP_KERNEL)?;
    }
    Ok(frames)
}
/// Bands described by one `kind=0x200f` sub-record, and the row stride of a band's values.
///
/// Measured: every full sub-record carries eight bands and 256 payload bytes, so a band is a
/// fixed 32 bytes regardless of how many strips actually occupy it -- at 2560 wide only the
/// first 20 are ever non-zero and bytes 20..32 are zero in all 5760 bytes of every map.
///
/// 32 covers any width up to 4096 px, but nothing wider has been captured, so a mode past
/// that must not assume the stride still holds.
pub(crate) const PARAM_BANDS_PER_TLV: usize = 8;
pub(crate) const PARAM_BAND_STRIDE: usize = 32;
/// Sub-records in the first of the pair; the rest go in the second. DLM splits 180 bands as
/// 120 + 60, which is this many full sub-records and then the remainder.
pub(crate) const PARAM_TLVS_PER_RECORD: usize = 15;
/// A strip's size class, as carried in the `kind=0x200f` map.
///
/// The dock needs each strip's length before it parses the strip, because a strip is a
/// self-delimiting bitstream whose end it cannot otherwise find. The class is simply the
/// length in 512-byte units.
///
/// Measured over 68,347 `(strip, map value)` pairs in a DLM capture with zero disagreements:
/// value 0 covers 54..510 bytes, 1 covers 512..1022, 2 covers 1024..1498 and 3 covers
/// 1594..1670. Every boundary falls on a multiple of 512.
#[inline]
pub(crate) fn strip_size_class(len: usize) -> u8 {
    // The field does not saturate: the corpus only reaches class 3 because its longest strip
    // is 1670 B, and clamping a live desktop's larger strips to 3 corrupts them.
    (len >> 9) as u8
}
/// Collect `(x, y, byte length)` for every strip in an already-framed set of image records.
///
/// The map is derived from the exact bytes about to go on the wire rather than from the
/// encoder's intermediate state, so it cannot drift from what the dock actually receives.
/// Each record body holds strips as `[u16 len][body]`, and every strip body opens with the
/// codec's `01 28` magic -- which is what terminates the walk when a record's trailing
/// padding is reached, since `aux` is a producer lane on this dock and not a pad count.
pub(crate) fn framed_strip_extents(
    frames: &[KVec<u8>],
) -> impl Iterator<Item = (usize, usize, usize)> + '_ {
    frames.iter().flat_map(|chunk| {
        // Each element of `frames` is an allocation chunk holding several complete records,
        // not one record, so walk records by their own stride: a record is `[u16 pad][u16
        // size][12 B header][ (u16 len ++ strip)* ][ 0..15 pad ]` and the next begins
        // `size + 4` bytes on. Walking strips to the end of a chunk instead stops at the first
        // inner record boundary, because the next record's zero `pad` reads as a zero length.
        let mut next_record = 0usize;
        let mut p = 0usize;
        let mut record_end = 0usize;
        core::iter::from_fn(move || loop {
            if p + 2 > record_end {
                // This record is exhausted (or none has started yet): step to the next.
                let hdr = chunk.get(next_record..next_record + 16)?;
                let stride = usize::from(u16::from_le_bytes([hdr[2], hdr[3]])) + 4;
                if stride < 16 || next_record + stride > chunk.len() {
                    return None;
                }
                p = next_record + 16;
                record_end = next_record + stride;
                next_record = record_end;
                continue;
            }
            let sl = usize::from(u16::from_le_bytes([chunk[p], chunk[p + 1]]));
            // A length that cannot be a strip is this record's trailing zero padding.
            if sl < 16 || p + 2 + sl > record_end {
                p = record_end;
                continue;
            }
            let s = &chunk[p + 2..p + 2 + sl];
            p += 2 + sl;
            if s[0] != 0x01 || s[1] != 0x28 {
                p = record_end;
                continue;
            }
            let x = usize::from(u16::from_le_bytes([s[2], s[3]]));
            let y = usize::from(u16::from_le_bytes([s[4], s[5]]));
            return Some((x, y, sl));
        })
    })
}
/// Build the DL7400's per-strip parameter map for one frame, as the pair of `kind=0x200f`
/// records DLM sends.
///
/// The map covers the whole frame: one byte per strip, `height / strip_h` bands of
/// `width / strip_w` strips, each band padded to [`PARAM_BAND_STRIDE`]. At 2560x1440 that is
/// 180 bands of 20 strips, which is what every captured DLM map contains, split 120 + 60
/// across the two records.
///
/// Values come from [`strip_size_class`] applied to the strips in `frames`. A position this
/// frame does not carry stays zero, which is what DLM sends for it.
///
/// An all-zero map is not a harmless approximation: it announces every strip as "under 512
/// bytes", so the dock mis-parses exactly the detailed strips and renders them as coloured
/// noise while flat fills stay perfect.
pub(crate) fn navarro_strip_params(
    geometry: Geometry,
    connector: u8,
    width: usize,
    height: usize,
    frames: &[KVec<u8>],
    remembered: &mut KVec<u8>,
) -> Result<KVec<u8>> {
    let bands = height.div_ceil(geometry.strip_h());
    let across = width.div_ceil(geometry.strip_w()).min(PARAM_BAND_STRIDE);
    let sub = u16::from(geometry.connector_selector(connector));
    let mut out = KVec::new();

    // One byte per map slot, laid out exactly as the sub-records carry it.
    //
    // The map covers the whole surface while a delta frame carries only its damaged strips,
    // so carry the previous frame's classes forward and overwrite only what this frame sends;
    // rebuilding from zero would re-declare every untouched strip as class 0. `remembered` is
    // the caller's per-connector buffer, zeroed by the resize a mode change triggers, which is
    // correct because the dock's framebuffer is undefined until the keyframe that follows.
    if remembered.len() != bands * PARAM_BAND_STRIDE {
        remembered.clear();
        remembered.resize(bands * PARAM_BAND_STRIDE, 0, GFP_KERNEL)?;
    }
    let mut values: KVec<u8> = KVec::new();
    values.resize(bands * PARAM_BAND_STRIDE, 0, GFP_KERNEL)?;
    values.copy_from_slice(remembered);
    let mut described = 0usize;
    let mut classes = [0usize; 8];
    let mut longest = 0usize;
    for (x, y, len) in framed_strip_extents(frames) {
        let (bx, by) = (x / geometry.strip_w(), y / geometry.strip_h());
        if bx >= across || by >= bands {
            continue;
        }
        let class = strip_size_class(len);
        values[by * PARAM_BAND_STRIDE + bx] = class;
        if let Some(slot) = classes.get_mut(usize::from(class)) {
            *slot += 1;
        }
        longest = longest.max(len);
        described += 1;
    }
    // A position the walk misses is announced as class 0, so `described` must equal the strip
    // count of the records handed in, and the histogram distinguishes a map that covers every
    // strip from one that calls them all class 0.
    vino_debug!(
        "vino: connector={connector} strip map {described} strip(s) over {} chunk(s), classes {:?}, longest {longest} B\n",
        frames.len(),
        &classes[..4]
    );
    remembered.copy_from_slice(&values);

    let mut band = 0usize;
    let mut record = 0usize;
    while band < bands {
        // The first record takes up to PARAM_TLVS_PER_RECORD sub-records, the second the rest.
        let take_tlvs = if record == 0 {
            PARAM_TLVS_PER_RECORD
        } else {
            bands.div_ceil(PARAM_BANDS_PER_TLV)
        };
        let mut body = KVec::new();
        for _ in 0..take_tlvs {
            if band >= bands {
                break;
            }
            let count = PARAM_BANDS_PER_TLV.min(bands - band);
            let payload = count * PARAM_BAND_STRIDE;
            body.extend_from_slice(&((6 + payload) as u16).to_le_bytes(), GFP_KERNEL)?;
            body.extend_from_slice(&0x200fu16.to_le_bytes(), GFP_KERNEL)?;
            body.extend_from_slice(&(band as u16).to_le_bytes(), GFP_KERNEL)?;
            body.extend_from_slice(&(count as u16).to_le_bytes(), GFP_KERNEL)?;
            let from = band * PARAM_BAND_STRIDE;
            body.extend_from_slice(&values[from..from + payload], GFP_KERNEL)?;
            band += count;
        }
        // DLM pads the first record's body to 3968 bytes and leaves the second exact.
        if record == 0 && body.len() < 3968 {
            body.resize(3968, 0, GFP_KERNEL)?;
        }
        let size = (body.len() + 12) as u16;
        let aux: u16 = if record == 0 { 0x0008 } else { 0x0000 };
        out.extend_from_slice(&0u16.to_le_bytes(), GFP_KERNEL)?;
        out.extend_from_slice(&size.to_le_bytes(), GFP_KERNEL)?;
        out.extend_from_slice(&4u32.to_le_bytes(), GFP_KERNEL)?;
        out.extend_from_slice(&sub.to_le_bytes(), GFP_KERNEL)?;
        out.extend_from_slice(&aux.to_le_bytes(), GFP_KERNEL)?;
        out.extend_from_slice(&0u32.to_le_bytes(), GFP_KERNEL)?;
        out.extend_from_slice(&body, GFP_KERNEL)?;
        record += 1;
    }
    Ok(out)
}
/// A frame's closing records.
///
/// The two platforms delimit a frame with a different number of records, so this carries its
/// own length and derefs to exactly the bytes that go on the wire.
pub(crate) struct FrameTrailer {
    bytes: [u8; 96],
    len: usize,
}
impl FrameTrailer {
    /// A frame that ends at its last strip record, with nothing closing it.
    ///
    /// A trailer borrowed from another generation is an unrecognised record in the middle of
    /// the stream, so a dock whose format is not known closes nothing.
    pub(crate) fn none() -> Self {
        Self {
            bytes: [0u8; 96],
            len: 0,
        }
    }

    /// Carry a single closing record, for a generation that delimits a frame with one.
    pub(crate) fn one(record: &[u8]) -> Self {
        let mut bytes = [0u8; 96];
        let len = record.len().min(bytes.len());
        bytes[..len].copy_from_slice(&record[..len]);
        Self { bytes, len }
    }
}
impl core::ops::Deref for FrameTrailer {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        &self.bytes[..self.len]
    }
}
/// The ring slot a frame writes, and the one it writes next.
///
/// Both platforms cycle a connector's frames through three buffers. Ridge names them by a phase
/// of `0`, `2` or `4`; Navarro names the dock-side slot id and address of each.
pub(crate) fn ring_phase(seq0: u32) -> (u8, u8) {
    let phase = ((seq0 % 3) as u8) * 2;
    (phase, (phase + 2) % 6)
}
/// Build the DL7400 record that opens a non-prologue frame.
///
/// Both working transports put a USB-transfer boundary between the `aux=0x0006` close and this
/// `aux=0x0004` next-slot record, so the opener belongs to the frame it describes rather than
/// to the preceding trailer. This is protocol framing, not cosmetic grouping.
pub(crate) fn navarro_frame_opener(geometry: Geometry, connector: u8, seq0: u32) -> [u8; 32] {
    let (phase, _) = ring_phase(seq0);
    let prev_phase = (phase + 4) % 6;
    let slot = super::super::cp::navarro_pipe_slot(connector, u16::from(phase));
    let ring = super::super::cp::navarro_pipe_ring(connector, u16::from(phase)) as u16;
    let prev_ring = super::super::cp::navarro_pipe_ring(connector, u16::from(prev_phase)) as u16;
    let sub = u16::from(geometry.connector_selector(connector));

    let mut out = [0u8; 32];
    record_header(&mut out, 4, sub, 0x0004, 0);
    out[16..19].copy_from_slice(&[0x0a, 0x00, 0x04]);
    out[19] = slot as u8;
    out[22..24].copy_from_slice(&ring.to_le_bytes());
    out[26..28].copy_from_slice(&prev_ring.to_le_bytes());
    out
}
/// Build the DL-3x00 record that opens a connector's video stream, sent once before any frame.
///
/// Two ring descriptors naming slot 0, distinguished from the per-frame close by `aux`: this
/// dock uses that field as a record subtype on non-image records, `0x0008` here and `0x000a`
/// for [`ella_frame_close`]. Sending the wrong opening leaves the stream unconfigured and the
/// dock stalls the endpoint on the first image write.
pub(crate) fn ella_stream_open(geometry: Geometry, connector: u8) -> [u8; 48] {
    let mut out = [0u8; 48];
    ella_record_header(&mut out, geometry, connector, 0x0008);
    // Ring descriptor: slot 0 next, with the last slot named as the one it follows.
    out[16..18].copy_from_slice(&10u16.to_le_bytes());
    out[18] = 0x04;
    out[27] = (ELLA_RING_SLOTS - 1) as u8;
    out[28..30].copy_from_slice(&10u16.to_le_bytes());
    out[30] = 0x04;
    out
}
/// Build the DL-3x00 record that closes a frame.
///
/// Names the ring slot this frame filled and the slot the next one will fill. The dock rotates
/// [`ELLA_RING_SLOTS`] buffers, so a wrong modulus hands it a slot it is still scanning out.
///
/// It is the last record of the frame's final USB transfer, which is what tells the dock the
/// frame is complete: every short-terminated transfer carrying pixels ends on one of these.
pub(crate) fn ella_frame_close(geometry: Geometry, connector: u8, seq0: u32) -> [u8; 48] {
    let cur = (seq0 % ELLA_RING_SLOTS) as u8;
    let next = ((seq0 + 1) % ELLA_RING_SLOTS) as u8;
    // The frame counter is one-based and occupies a single byte on the wire.
    let seq = (seq0 % 256) as u8;

    let mut out = [0u8; 48];
    ella_record_header(&mut out, geometry, connector, 0x000a);
    out[16..18].copy_from_slice(&8u16.to_le_bytes());
    out[18] = 0x05;
    out[19] = cur;
    out[23] = cur;
    out[24] = 0x01;
    out[25] = seq.wrapping_add(1);
    out[26..28].copy_from_slice(&10u16.to_le_bytes());
    out[28] = 0x04;
    out[29] = next;
    out[33] = next;
    // The slot this frame filled, repeated after the pair naming the next one.
    out[37] = cur;
    out
}
/// Buffers the DL-3x00 dock rotates through; see `DockProfile::dock_buffers`.
pub(crate) const ELLA_RING_SLOTS: u32 = 3;
/// Fill the 16-byte record header shared by this dock's non-image records.
pub(crate) fn ella_record_header(out: &mut [u8; 48], geometry: Geometry, connector: u8, aux: u16) {
    record_header(
        out,
        4,
        u16::from(geometry.connector_selector(connector)),
        aux,
        0,
    );
}
/// Build the DL7400's closing record for the ring slot this frame filled.
///
/// The next slot is announced by [`navarro_frame_opener`] only after this frame's final USB
/// transfer has terminated.
pub(crate) fn navarro_frame_trailer(geometry: Geometry, connector: u8, seq0: u32) -> FrameTrailer {
    let (phase, _) = ring_phase(seq0);
    let slot = super::super::cp::navarro_pipe_slot(connector, u16::from(phase));
    let ring = super::super::cp::navarro_pipe_ring(connector, u16::from(phase)) as u16;
    let sub = u16::from(geometry.connector_selector(connector));

    let mut out = [0u8; 96];
    record_header(&mut out[..32], 4, sub, 0x0006, 0);

    // Slot complete: its id, its ring address, and this frame's number.
    out[16..19].copy_from_slice(&[0x08, 0x00, 0x05]);
    out[19] = slot as u8;
    out[22..24].copy_from_slice(&ring.to_le_bytes());
    out[25] = (seq0 as u8).wrapping_add(1);

    FrameTrailer {
        bytes: out,
        len: 32,
    }
}
/// They delimit every logical frame, including the ARM-prefixed first frame. The first record
/// carries a wrapping one-based frame counter; all three carry a three-slot phase (`0,2,4`) and
/// the selected connector.
pub(crate) fn frame_trailer(geometry: Geometry, connector: u8, seq0: u32) -> FrameTrailer {
    let (phase, next_phase) = ring_phase(seq0);
    let phase_off = phase * 4;
    let next_off = next_phase * 4;
    let frame_no = (seq0 as u8).wrapping_add(1);
    let mut out = [0u8; 96];

    let h = geometry.connector_selector(connector);
    for (i, connector_byte) in [h, h, h | 0x10].into_iter().enumerate() {
        let o = i * 32;
        let aux = if i == 0 { 0x0006 } else { 0x0004 };
        record_header(&mut out[o..o + 32], 4, u16::from(connector_byte), aux, 0);
    }

    // Record A: frame-present marker + current ring phase + one-based u8 frame number.
    out[16] = 0x08;
    out[18] = 0x05;
    out[19] = phase;
    out[23] = phase_off;
    out[25] = frame_no;

    // Records B/C are identical apart from C's connector|0x10 header selector.
    for o in [32usize, 64] {
        out[o + 16] = 0x0a;
        out[o + 18] = 0x04;
        out[o + 19] = next_phase;
        out[o + 23] = next_off;
        out[o + 27] = phase_off;
    }
    FrameTrailer {
        bytes: out,
        len: 96,
    }
}

#[cfg(CONFIG_DRM_VINO_KUNIT_TEST)]
#[kunit_tests(vino_haar_records)]
mod tests {
    use super::*;

    #[test]
    fn ella_stream_records_match_the_dlm_capture() -> Result {
        // Byte-for-byte against a DLM capture of an Ella dock driving 1920x1080 on two connectors.
        // The dock accepts a malformed record and then simply never paints, so nothing on the wire
        // and nothing in dmesg reports a mistake here -- only this comparison does. Field meanings,
        // in the order they appear: `aux` is a record subtype on this dock (0x0008 opens a stream,
        // 0x000a closes a frame); the closing record names the slot the frame filled, the slot the
        // next frame will fill, and a one-based frame counter.
        let geometry = profile::PROFILE_ELLA.geometry();

        // Stream open, connector 0. Two ring descriptors naming slot 0.
        let open = ella_stream_open(geometry, 0);
        assert_eq!(
            &open[..40],
            &[
                0x00, 0x00, 0x2c, 0x00, 0x04, 0x00, 0x00, 0x00, 0x00, 0x00, 0x08, 0x00, 0x00, 0x00,
                0x00, 0x00, 0x0a, 0x00, 0x04, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02,
                0x0a, 0x00, 0x04, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            ][..]
        );

        // Three consecutive frame closes on connector 0, walking the ring 0 -> 1 -> 2.
        let expect: [[u8; 40]; 3] = [
            [
                0x00, 0x00, 0x2c, 0x00, 0x04, 0x00, 0x00, 0x00, 0x00, 0x00, 0x0a, 0x00, 0x00, 0x00,
                0x00, 0x00, 0x08, 0x00, 0x05, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x01, 0x0a, 0x00,
                0x04, 0x01, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            ],
            [
                0x00, 0x00, 0x2c, 0x00, 0x04, 0x00, 0x00, 0x00, 0x00, 0x00, 0x0a, 0x00, 0x00, 0x00,
                0x00, 0x00, 0x08, 0x00, 0x05, 0x01, 0x00, 0x00, 0x00, 0x01, 0x01, 0x02, 0x0a, 0x00,
                0x04, 0x02, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00,
            ],
            [
                0x00, 0x00, 0x2c, 0x00, 0x04, 0x00, 0x00, 0x00, 0x00, 0x00, 0x0a, 0x00, 0x00, 0x00,
                0x00, 0x00, 0x08, 0x00, 0x05, 0x02, 0x00, 0x00, 0x00, 0x02, 0x01, 0x03, 0x0a, 0x00,
                0x04, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00,
            ],
        ];
        for (seq0, want) in expect.iter().enumerate() {
            let got = ella_frame_close(geometry, 0, seq0 as u32);
            assert_eq!(&got[..40], &want[..]);
        }

        // Head 1 differs only in the record `sub`: this dock uses the bare connector number.
        let head1 = ella_frame_close(geometry, 1, 0);
        assert_eq!(head1[8], 0x01);
        assert_eq!(&head1[16..38], &expect[0][16..38]);
        Ok(())
    }

    #[test]
    fn an_encoded_strip_never_exceeds_the_decoder_input_bound() -> Result {
        // A strip has to fit inside one record. The record builder starts a fresh record when the
        // next strip would pass the stride cap, but the *first* strip of a record is taken at
        // whatever size it has -- it has to be, or a frame carrying such a strip could not be
        // built at all. So an over-long strip does not produce a short record, it produces a
        // record whose stride is over the cap: a wire-format violation the dock can only report by
        // halting the endpoint, which is indistinguishable from any other transport fault.
        //
        // The bound is the cap less the record header and the strip's own length prefix. For
        // reference, DLM never exceeds 1758 bytes on DL-6xxx, 1780 on the DL-7400 or 2036 on
        // DL-3x00 across roughly half a million strips, so conforming content has a wide margin;
        // pseudo-random pixels are the worst case for an entropy coder and are what this uses.
        const DECODER_STRIP_BOUND: usize = 0x0ff0 - 16 - 2;
        let mut state: u32 = 0x1234_5678;
        let mut next = || {
            state = state.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            (state >> 16) as u16
        };
        let mut worst = 0usize;
        let mut worst_eight = 0usize;
        for geometry in [
            video::haar::RIDGE_GEOMETRY,
            video::haar::Geometry::new(8, true, false, 0, 0x08, 3),
            video::haar::Geometry::new(16, true, false, 3, 0x07, 3),
        ] {
            // Eight-bit samples are what every mode on these docks carries; the ten-bit range is
            // the DL-7000 HDR profile and is measured separately so an out-of-range sample cannot
            // be mistaken for an encoder that overshoots.
            for (mask, ten_bit) in [(0xffu16, false), (0x3ff, true)] {
                let geometry = if ten_bit {
                    geometry.with_depth(video::haar::Depth::Ten)
                } else {
                    geometry
                };
                for _ in 0..16 {
                    let mut px =
                        |_x: usize, _y: usize| (next() & mask, next() & mask, next() & mask);
                    let strip = video::haar::colour_strip_at(geometry, 0, 0, &mut px)?;
                    worst = worst.max(strip.len());
                    if !ten_bit {
                        worst_eight = worst_eight.max(strip.len());
                    }
                }
            }
        }
        pr_info!("vino-selftest: worst 8-bit encoded strip {worst_eight} B\n");

        // A flat strip is the one picture whose encoding can be compared to the vendor's exactly:
        // every strip of a black frame encodes identically, so a whole captured frame collapses to
        // a single payload and any difference is unambiguous. These are the bytes DLM puts on the
        // wire for the strip at 0,0 of a black 1920x1088 DL-3x00 frame, and all 1024 strips of that
        // frame carry them. Matching the length is not enough: the same 54 bytes can hold a
        // different code, and a dock told one code and sent another decodes every strip to noise.
        const DLM_FLAT_STRIP: [u8; 54] = [
            0x01, 0x28, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x36, 0x00, 0x36, 0x00,
            0x00, 0x00, 0x54, 0x15, 0xaa, 0x0a, 0x55, 0x85, 0xaa, 0x42, 0x55, 0xa1, 0xaa, 0x50,
            0x55, 0xa8, 0x2a, 0x54, 0x15, 0xaa, 0x0a, 0x55, 0x85, 0xaa, 0x42, 0x55, 0xa1, 0xaa,
            0x50, 0x55, 0xa8, 0x2a, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ];
        let ella = video::haar::Geometry::new(8, true, false, 0, 0x08, 3)
            .with_coding(video_arm::CodeTables::Narrow);
        let mut black = |_x: usize, _y: usize| (0u16, 0u16, 0u16);
        let flat = video::haar::colour_strip_at(ella, 0, 0, &mut black)?;
        // A smooth gradient stands in for an ordinary desktop, where DLM averages about 175 bytes.
        let mut ramp = |x: usize, y: usize| {
            let v = ((x + y) & 0xff) as u16;
            (v, v, v)
        };
        let gradient = video::haar::colour_strip_at(ella, 0, 0, &mut ramp)?;

        // A flat strip cannot tell the payload orders apart: every field of it carries an all-zero
        // payload, so the bytes above are identical whichever end goes out first. Pin a strip that
        // does carry payload bits. The order itself was settled against 8000 captured DL-3x00
        // strips three ways -- reading the interleaved payload least significant bit first takes
        // whole-strip decoding from 77% to 100%, brings the recovered luma DC into its exact
        // 0..1020 range instead of an impossible -3205..996, and turns the reconstructed frame
        // from streaks into a legible desktop.
        const NARROW_GRADIENT_HEAD: [u8; 64] = [
            0x01, 0x28, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x48, 0x00, 0xf8, 0x00,
            0x00, 0x00, 0x5c, 0xe1, 0x0a, 0x57, 0xb8, 0xc2, 0x15, 0xae, 0x70, 0x85, 0x2b, 0x5c,
            0xe1, 0x0a, 0x57, 0xb8, 0xc2, 0x15, 0xae, 0x70, 0x85, 0x2b, 0x5c, 0x8f, 0xab, 0xc2,
            0x55, 0xe1, 0xaa, 0x70, 0x55, 0xb8, 0x2a, 0x5c, 0x15, 0xae, 0x0a, 0x55, 0xd5, 0xb8,
            0x2a, 0x5c, 0x15, 0xae, 0x0a, 0x57, 0x85, 0xab,
        ];
        assert_eq!(
            &gradient[..NARROW_GRADIENT_HEAD.len()],
            &NARROW_GRADIENT_HEAD[..]
        );
        pr_info!(
            "vino-selftest: flat strip {} B (DLM sends 54), gradient strip {} B\n",
            flat.len(),
            gradient.len()
        );
        assert_eq!(flat.len(), DLM_FLAT_STRIP.len());
        if let Some(at) = (0..flat.len()).find(|&i| flat[i] != DLM_FLAT_STRIP[i]) {
            pr_err!(
                "vino-selftest: flat strip differs from DLM at byte {}: {:#04x} vs {:#04x}\n",
                at,
                flat[at],
                DLM_FLAT_STRIP[at]
            );
        }
        assert_eq!(&flat[..], &DLM_FLAT_STRIP[..]);
        pr_info!("vino-selftest: worst encoded strip {worst} B (bound {DECODER_STRIP_BOUND})\n");
        // Eight bits per channel is every mode these docks are driven at today, and it stays
        // inside the bound with room to spare.
        assert!(worst_eight <= DECODER_STRIP_BOUND);
        // The ten-bit profile does not, so the framing has to refuse it rather than put an
        // over-cap record on the wire. Build a frame from one such strip and check that it does.
        if worst > DECODER_STRIP_BOUND {
            let geometry = video::haar::RIDGE_GEOMETRY.with_depth(video::haar::Depth::Ten);
            let mut oversized: KVec<KVec<u8>> = KVec::new();
            loop {
                let mut px =
                    |_x: usize, _y: usize| (next() & 0x3ff, next() & 0x3ff, next() & 0x3ff);
                let strip = video::haar::colour_strip_at(geometry, 0, 0, &mut px)?;
                if strip.len() > DECODER_STRIP_BOUND {
                    oversized.push(strip, GFP_KERNEL)?;
                    break;
                }
            }
            assert!(frame_records(geometry, &oversized, 0).is_err());
        }
        Ok(())
    }

    #[test]
    fn record_stats_reads_a_frame_back_off_its_own_records() -> Result {
        // The diagnostic that runs when a frame fails to reach a dock, so it has to be right when
        // nothing else is: a wrong reading here sends the next investigation at the transport.
        let geometry = video::haar::Geometry::new(8, true, false, 0, 0x08, 3);
        let mut strips: KVec<KVec<u8>> = KVec::new();
        let mut px = |_x: usize, _y: usize| (0u16, 0u16, 0u16);
        for _ in 0..40 {
            strips.push(
                video::haar::colour_strip_at(geometry, 0, 0, &mut px)?,
                GFP_KERNEL,
            )?;
        }
        let flat = strips[0].len();
        let records = frame_records(geometry, &strips, 0)?;
        let (count, max_stride, max_strip) = record_stats(&records);
        assert!(count > 0);
        assert_eq!(max_strip, flat);
        assert!(max_stride <= 0x0ff0);
        assert_eq!(max_stride % 16, 0);
        Ok(())
    }

    #[test]
    fn ella_band_order_is_the_producer_order_not_a_plain_interlace() -> Result {
        // This dock stops draining at the first band it did not expect, so the order is part of
        // the format rather than a preference. The generic even-then-odd interlace diverges at the
        // second band -- it sends 2 where the dock wants 3 -- which is inside the first sixty
        // strips of every frame.
        let rows = ella_rows_1080p();
        assert_eq!(rows.len(), 68);
        assert_eq!(&rows[..8], &[0, 3, 5, 7, 9, 11, 13, 16]);
        assert_eq!(&rows[33..37], &[65, 66, 1, 2]);
        assert_eq!(rows[67], 67);
        // Every band exactly once: a repeat would mean a split-row producer boundary, which this
        // dock does not have and which the ordering code would silently mis-handle.
        let mut seen = [false; 68];
        for &y in rows {
            assert!(!seen[y as usize]);
            seen[y as usize] = true;
        }
        assert!(seen.iter().all(|s| *s));
        Ok(())
    }

    #[test]
    fn video_frame_trailer_matches_dlm_cycle_and_head() {
        let geometry = profile::PROFILE_RIDGE.geometry();
        let t0 = frame_trailer(geometry, 0, 0);
        assert_eq!(
            &t0[..32],
            &[
                0, 0, 0x1c, 0, 4, 0, 0, 0, 0, 0, 6, 0, 0, 0, 0, 0, 8, 0, 5, 0, 0, 0, 0, 0, 0, 1, 0,
                0, 0, 0, 0, 0,
            ]
        );
        assert_eq!(
            &t0[32..64],
            &[
                0, 0, 0x1c, 0, 4, 0, 0, 0, 0, 0, 4, 0, 0, 0, 0, 0, 0x0a, 0, 4, 2, 0, 0, 0, 8, 0, 0,
                0, 0, 0, 0, 0, 0,
            ]
        );
        // Record C ORs the connector selector with 0x10. `sub` is the little-endian u16 at bytes
        // 8..10, so the selector belongs in byte 8; placing it in byte 9 would encode 0x1100
        // instead of 0x0011 and prevent connector 1 from presenting the frame.
        assert_eq!(
            &t0[64..],
            &[
                0, 0, 0x1c, 0, 4, 0, 0, 0, 0x10, 0, 4, 0, 0, 0, 0, 0, 0x0a, 0, 4, 2, 0, 0, 0, 8, 0,
                0, 0, 0, 0, 0, 0, 0,
            ]
        );

        let t1 = frame_trailer(geometry, 1, 1);
        assert_eq!(u16::from_le_bytes([t1[8], t1[9]]), 0x0001);
        assert_eq!(u16::from_le_bytes([t1[32 + 8], t1[32 + 9]]), 0x0001);
        assert_eq!(u16::from_le_bytes([t1[64 + 8], t1[64 + 9]]), 0x0011);
        assert_eq!(t1[19], 2);
        assert_eq!(t1[23], 8);
        assert_eq!(t1[25], 2);
        assert_eq!(t1[32 + 19], 4);
        assert_eq!(t1[32 + 23], 16);
        assert_eq!(t1[32 + 27], 8);
    }
}
