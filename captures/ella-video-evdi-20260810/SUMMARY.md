# How DLM talks to the Ella dock (HP 3005pr, DL-3900)

Source: `wire.pcapng`, DLM driving two evdi heads at **1920x1080@60** (with a brief 1280x1024
excursion on card2). Decrypt: `keys.candidates.json`, session key `4b006f27418c..`,
riv `ca156e58d099aaa4`.

Everything below is measured against the whole 290 MB capture, not sampled.

## 1. One endpoint carries both planes

The DL3 display interface exposes **only** `ep 0x02` OUT and `ep 0x84` IN. There is no video
endpoint -- `ep 0x0a` on this device belongs to interface 6 (CDC Data, the NIC), not to video.

| endpoint | packets | bytes |
|---|---|---|
| `0x02` OUT | 19,648 | **289,818,224** |
| `0x84` IN | 625 | 68,026 |

So control and video are multiplexed down one pipe. This is what `profile.rs` means by "it carries
video on the control pipe, which needs the two writers serialised".

## 2. The outer record framing is Ridge's, unchanged

```
off 0..2    zero
off 2..4    size : u16 LE        stride = size + 4, always 16-byte aligned
off 4..8    type : u32 LE        1, 2, or 4
off 8..10   sub  : u16 LE        <- the plane discriminator, see below
off 10..12  aux  : u16 LE        pad count on video records
off 12..16  zero
off 16..    body
```

Validated by parsing the concatenated EP02 stream end to end:

- **92,072 records, 289,818,224 bytes consumed, zero resync skips.**
- **zero** records with a non-16-byte stride.
- Max observed stride **4080**, exactly `STRIDE_CAP = 0x0ff0` in `video.rs`.

Records span USB transfer boundaries, so the stream must be concatenated before parsing. A
per-transfer parse overruns (first 64 KiB transfer parses to 67,152 bytes).

## 3. The `sub` field at offset 8 separates the planes

| `sub` | plane | count |
|---|---|---|
| `0x00` | video, head 0 | 67,589 |
| `0x01` | video, head 1 | 24,235 |
| `0x04` | plaintext CP | early session only |
| `0x24` | sealed CP OUT | 212 |
| `0x25`, `0x45` | CP IN (on `ep 0x84`) | -- |

`0x24` OUT / `0x45` IN are the same sealed-CP wire subs the driver already uses. A record with
`sub = 0x24` lays out as `[id:u16][sub:u16][counter:u32][sealed body]` from offset 8 -- byte for
byte what the DL7400 sends.

⚠ A naive CP decoder run over this stream reports thousands of bogus messages with sub `0x2801`:
those are video records, where the tool reads the first strip's length as an id and the codec sync
word as a sub. Filter on the offset-8 `sub` before decoding.

## 4. Video records are exactly what `frame_records` already emits

Body from offset 16 is a run of `[strip_len : u16 LE][strip bytes]`, `aux` is the 0..15 pad count,
and **every strip begins with the `0x2801` codec sync word**. Worked example:

```
00 00 cc 0f | 04 00 00 00 | 00 00 | 00 00 | 00 00 00 00 | 36 00 | 01 28 ...
             type 4         sub 0   aux 0                 len 54   strip
size 0x0fcc = 4044 -> stride 4048, 4048 % 16 == 0, pad 0
```

```
00 00 ec 0f | 04 00 00 00 | 00 00 | 04 00 | 00 00 00 00 | d8 00 | 01 28 ...
size 0x0fec = 4076 -> stride 4080, pad 4                  len 216
```

Both match `frame_records_with_boundary` field for field. **The codec is the same and the framing
is the same; nothing new has to be written for either.**

## 5. Which profile knobs Ella needs

Measured, and it is a *mix* of the two existing docks -- which is the reason it needs its own
`DockProfile` rather than reusing either:

| knob | Ella | same as |
|---|---|---|
| `strip_w_shift` / `strip_h_shift` | **6 / 4** (64x16) | Ridge |
| `head_sub_shift` | **0** -- bare head number, `sub` is 0 and 1 | Ridge |
| `band_parity_bit` | **false** -- bit 4 never set across 92,063 records | Navarro |
| record framing / `STRIDE_CAP` | Ridge's, unchanged | Ridge |
| set-mode | `id=0x48 sub=0x22`, 112 B sealed, same field map | both |

Ridge sets `band_parity_bit = true`; Navarro shifts the head by 3. Ella does neither -- it is
Ridge everywhere except the parity bit, which is why it needs its own profile rather than reusing
`PROFILE_RIDGE`.

### Geometry, measured directly from strip coordinates

`strip_y(s)` reads `s[4..6]`, so a strip carries its own position. Over **992,496 strips**:

| axis | distinct | range | step |
|---|---|---|---|
| x | **30** | 0 .. 1856 | **64** |
| y | **68** | 0 .. 1072 | **16** |

30 x 68 = **2040 strips** at 1920x1080, which is the 64x16 layout exactly. The 128x8 alternative
would have given 15 x 135 = 2025 with a y step of 8. Settled: **Ella is 64x16, same as Ridge.**

## 6. The decrypted set-mode

`0x48/0x22`, 112 B, four instances (two heads x two mode changes):

```
48 00 22 00 4b 00 0000  ...  80 07 18 01 58 00 2c 00 38 04 2d 00 04 00 05 00 00 04 3c 00 ...
id    sub   ctr           off26=1920  280   88    44  off34=1080  45    4     5  0x0400  60
```

Every field lands where the existing map says it should: off26/34 active, htotal 2200 and vtotal
1125 (textbook 1080p60), off42 = `0x0400` for 1080p, off44 = 60 = refresh, off66 = `0x2810`,
off68 = `0x0200`, off72 = 0. **No new set-mode code is needed.**

## 7. Still to measure

- `interlaced_bands` -- needs the band emission order checked against the y sequence.
- `stream_id_mask`, `dock_buffers`.
- The 48-byte records (709 of them) are the stream-open shape; confirm against Navarro's.

## 7. The one genuinely new thing to build

Serialising the two writers on `ep 0x02`. Today `cp_link` (a mutex) and the bulk video queue are
independent paths; on Ella a video record must never interleave with a CP record mid-stream. The
record framing makes the boundary explicit, so the serialisation can be at record granularity
rather than a coarse lock over a whole frame.

Everything else -- session, seal, HDCP, set-mode, record framing, codec, strip encoder -- is
already in the driver and should be shared, not duplicated.
