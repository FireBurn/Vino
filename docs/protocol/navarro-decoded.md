# The DL7400 wire, decoded

Every message DLM sends this dock, with the derivation for each field. Sources:

* `captures/navarro-dlm-modeset-20260802-005453/` — Linux DLM 6.8.1, keyed. 807 control frames,
  559 tag-verified; 26.7 MB on `ep 0x08` and 13.5 MB on `ep 0x0a`, both walking clean as records.
* `captures/navarro-wincap-20260802/out/` — Windows driver 11.5.6380.0. A second, independent
  implementation. Unkeyed, so record framing only — which is exactly what it is good for.

Nothing here is inferred from Ridge. Where a field is not established, it says so.

## 1. The rule that removes "host-random" from the vocabulary

Every sealed plaintext is a **self-terminating TLV chain**:

```
[u16 len][u16 kind][len - 2 bytes of payload]      record stride = len + 2
```

Walking that chain says where the message ends without anyone deciding a content length. Doing it
across all 29 sealed video records in the DLM capture:

| record | TLV chain ends at | plaintext | leftover |
|---|---|---|---|
| pipe descriptor (`aux=0x000e`) | 290 | 304 | 14 |
| decoder configuration (`aux=0x000e`) | 1090 | 1104 | 14 |
| stream report (`aux=0x000c`) | 84 | 96 | 12 |
| mode-restating stream report (`aux=0x0002`) | 110 | 112 | 2 |
| mode set (`id=0x48 sub=0x22`, control plane) | 74 | 80 | 6 |

In every case the leftover is exactly `-content mod 16` — the pad to an AES block. It is not a
nonce, a token or a checksum, and no field is being truncated to make that true.

**What it actually is**, from the Windows RE (`captures/win-re-20260628/FINDINGS-rng-taint.md`,
2026-06-28): *"the dominant steady-state 10-byte 'random' is final-block CTR padding,
XOR-encrypted and discarded."* An uninitialised tail of a buffer rounded up to the block size.

⚠ Two things that look like contradictions and are not:

* **DLM does have a CSPRNG.** mbedTLS `CTR_DRBG`, entropy-seeded; the msg0 token was traced 40+
  frames through the ctr_drbg cascade on 2026-07-15. The reason "only UUIDs use random" looks true
  from a syscall trace is that libuuid's `getrandom` calls *are* the DRBG's entropy source.
* **The 14-byte tail is shared with Ridge.** `video_arm::build_with_layout_word()` takes
  `nonce: &[u8; 14]` and appends it to the 1104-byte decoder configuration, a record both docks
  send (Ridge layout word `0x4000`, Navarro `0x2100`). On the D6000, vino fills it from the kernel
  CSPRNG and both panels light — so for that record "arbitrary" is backed by a working link, not
  by an argument.

⛔ **RETRACTED FOR THE PIPE DESCRIPTOR (2026-08-03 pm).** Decrypting it from a **same-day capture
taken while DLM was driving both panels on this dock** gives a different layout:

```
working capture:   [marker 14][marker 14][6 x 46-byte slot record]  = 304, no tail
2026-08-02 capture:[marker 14]           [6 x 46-byte slot record]  = 290, + 14 unexplained
```

Both are 304 bytes. **In the working capture those fourteen bytes are consumed by a second marker
at the front** — they are not padding. vino emitted one marker and fourteen CSPRNG bytes, which
also came to 304, which is exactly how a wrong layout survives a length check. Fixed in
`0db19e6bb3e4`; the marker count is still not a settled constant and the code says so.

⇒ The general rule in this section still holds — the TLV chain says where a message ends — but
"whatever is left is padding" does **not** follow from it. Check the leftover against a capture
that was known to be working, per record.

⚠ Still open on the same terms: the stream report's 12 bytes and the mode set's 6.

## 2. Control plane

Full inventory of DLM's outbound messages, from `cpdump.py` over the keyed capture. Inner
framing is `[u16 id][u16 sub][u16 counter][16 zero bytes][payload from off22]`.

### 2.1 The HDCP block, four times — once per connector

The DL7400 runs a **complete AKE per connector**, not one for the link. The connector is a one-hot
bit at `off22 + connector`, and the HDCP message id is at `off27`.

| id/sub | HDCP msg | len | payload |
|---|---|---|---|
| `0x0022/0x10` | `0x02` AKE_Init | 48 | 20-byte `rtx` |
| `0x001f/0x10` | `0x13` | 48 | fixed `00 06 02 00 02` prefix |
| `0x009a/0x10` | `0x04` AKE_No_Stored_km | 160 | 128-byte `Ekpub(km)` |
| `0x0022/0x10` | `0x09` LC_Init | 48 | 20-byte `rn` |
| `0x0032/0x10` | `0x0b` **SKE_Send_Eks** | 64 | see below |
| `0x002a/0x10` | `0x0f` RepeaterAuth_Send_Ack | 48 | 16-byte `V` |
| `0x0026/0x10` | `0x10` RepeaterAuth_Stream_Manage | 48 | `seq_num_M`, `k=1`, StreamID |

### 2.2 ⭐ The per-connector video keys are on the wire

`id=0x0032 sub=0x10` is a literal HDCP 2.2 `SKE_Send_Eks`: 16 bytes of `Edkey(ks)` at **off28**
followed by the 8-byte **`riv` at off44**. Both video keys recovered by the MAC oracle appear
there:

```
connector 0   wire riv c345fe5593613901   in use c345fe5593613906   byte7 ^ 0x07
connector 1   wire riv 9446c83da5fa39e3   in use 9446c83da5fa39ec   byte7 ^ 0x0f
```

The tweak is the stream id, `(connector << 3) | 7`.

⛔ **This refutes `project_navarro_video_plaintext_streamopen_20260802`**, which concluded the
per-head video-key derivation was "3 distinct keys from ONE handshake, not transmitted, no simple
relation ⇒ local key schedule, needs Ghidra, NOT a capture". They are transmitted, by the
transmitter, as ordinary SKE, one per connector — and vino already sends this message.

### 2.3 `RepeaterAuth_Stream_Manage` names the stream

`id=0x0026 sub=0x10`, four of them:

```
off22..25  one-hot connector      off27      0x10
off28..31  seq_num_M = 0          off32..35  k = 1
off36..39  StreamID = 7 / 15 / 23 / 31       off40..47  pad
```

`7 / 15 / 23 / 31` are exactly the video wire subs. vino's `cp::stream_manage_restatement()`
already produces these bytes.

### 2.4 The mode set, `id=0x48 sub=0x22`

Five decrypted: 640x480p60 and 2560x1440 at 60, 120 and 165 Hz.

| off | field | 640x480p60 | 2560x1440p60 | p120 | p165 |
|---|---|---|---|---|---|
| 22 | connector | 0 | 0/1 | 1 | 0 |
| 23 | constant | `0x02` | `0x02` | `0x02` | `0x02` |
| 26 | hactive | 640 | 2560 | 2560 | 2560 |
| 28 | hblank | 160 | 160 | 160 | 160 |
| 30 | hsync front | 16 | 48 | 48 | 48 |
| 32 | hsync width | 96 | 32 | 32 | 32 |
| 34 | vactive | 480 | 1440 | 1440 | 1440 |
| 36 | vblank | 45 | 41 | 85 | 119 |
| 38 | vsync front | 10 | 3 | 3 | 3 |
| 40 | vsync width | 2 | 5 | 5 | 8 |
| 42 | link word | `0x0700` | `0x0600` | `0x0600` | `0x0600` |
| 44 | refresh (Hz) | 60 | 60 | 120 | 165 |
| 46 | ⚠ unknown | `0x0300` | `0x0a80` | `0x0a80` | `0x0a80` |
| 48 | ⚠ unknown | `0x6800` | `0x66db` | `0x66db` | `0x66db` |
| 58 | constant | `0x0080` | | | |
| 60 | constant | `0x00ff` | | | |
| 66 | constant | `0x0800` | `0x0800` | `0x0800` | `0x0800` |
| 68 | constant | `0x0200` | | | |
| 70 | **pixel clock, u32, 10 kHz** | 2517 | 24150 | 49775 | **69949** |

The porches check out as textbook timings — VGA 640x480 (16/96, 10/2) and CVT-RB v2 1440p
(48/32, 3/5) — and `off70 / (vtotal x refresh)` recovers `htotal = 2720` at all three 1440p rates
and 800 at 640x480. That is what makes this a derivation rather than a table.

⭐ **off70..73 is a `u32`.** Ridge could never show that: it is never driven past 497.75 MHz, so
its high half is always zero. The DL7400 at 1440p165 sends `0x0001113d` = 699.49 MHz. This settles
the "off72 is untested and DLM can never settle it" note in `CLAUDE.md` — off72 is simply the high
half of the clock, and it is used.

⭐ **off66 is constant `0x0800` on Navarro** across 640x480p60 and 1440p at 60/120/165 — it does
not move with refresh the way Ridge's does.

⚠ **off46 and off48 are unresolved.** Both track resolution alone (identical across three
refresh rates at 1440p). `0x0a80`/`0x0300` are both `hactive + 128`; `0x66db`/`0x6800` have no
proposed derivation. Two samples cannot separate a formula from a lookup, so
`cp::navarro_mode_words()` is a measured table that refuses to extrapolate.

⚠ **off42 at 640x480 is `0x0700`**, where vino's `link_word_42()` ladder gives `0x0400` for any
width up to 1920. One sample, on a mode nobody drives — recorded, not acted on.

## 3. Video records

Wire framing, both endpoints, both implementations:

```
[u16 zero][u16 size][u32 type][u16 sub][u16 aux][u32 seq]   record stride = size + 4
```

`sub` is `(connector << 3)` for frame records and `(connector << 3) | 7` for the stream. Windows
confirms the endpoint map independently: `0x08` carries connectors 0 and 2, `0x0a` carries 1 and 3.

### 3.1 URB boundaries are not record boundaries

Windows splits every video transfer on a record boundary — 1108 of 1108 URBs in the full-payload
capture walk exactly to their end. **Linux DLM does not**: it splits at 65536 and every URB after
the first starts mid-record. So record alignment is a Windows habit, not a dock requirement, and
vino's 64 KiB chunking is fine.

### 3.2 The frame grammar

Each frame on a connector is bracketed by two 28-byte plaintext records:

```
[u16 len=0x000a][u8 kind=0x04][u8 slot][u16 0][u16 ring_cur][u16 0][u16 ring_prev]   aux=0x0004
   ... sealed stream report on the stream sub, then image records ...
[u16 len=0x0008][u8 kind=0x05][u8 slot][u16 0][u16 ring][u8 0][u8 frame_no]          aux=0x0006
```

`slot = connector * 8 + index` and `ring = 0x6fcc - slot * 0x21c`, the same arithmetic the pipe
descriptor uses. Each connector cycles **three** buffers — index 0, 2, 4 — and `frame_no` is a
small one-based counter. Byte-identical between Linux DLM and Windows.

vino's `video::wht::navarro_frame_trailer()` reproduces both records exactly, and its prologue ring
record matches DLM's.

### 3.3 ⭐ `aux` is a producer lane, not a record kind

The handover of 2026-08-03 left `aux` unexplained ("the even numbers 0..0xe, uncorrelated with
size"). Per frame:

* DLM's first five frames — ~54 image records each — carry **`aux = 0` on every one**.
* Later frames, 1000+ records, spread `aux` evenly over `0, 2, 4, ..., 0xe`.

The same `aux` value carries an ordinary image record in one frame and a ring record in another, so
it cannot be a kind. It is a per-record tag that only appears once there are many records: a
producer/lane index from DLM's parallel encoder.

⇒ **vino emitting `aux = 0` from a single producer is exactly what DLM does when it has one.** This
is not the video blocker.

### 3.4 ⭐ `kind=0x200f` is a per-strip parameter map — and vino sent none

Emitted in pairs, a 3980-byte record (`aux=0x0008`) and a 1996-byte one (`aux=0x0000`), two to five
times per frame. Each is a chain of sub-records:

```
[u16 len][u16 kind=0x200f][u16 first_band][u16 band_count][band_count x 32 bytes]
```

A full sub-record carries eight bands and 256 payload bytes, so **a band is 32 bytes**. The pair
always covers the frame exactly: 15 sub-records of 8 bands (0..119) then 7 of 8 plus one of 4
(120..179).

**180 bands, and only the first 20 bytes of each band are ever non-zero.** At 2560x1440 with the
DL7400's 128x8 strips that is `1440 / 8 = 180` bands of `2560 / 128 = 20` strips — **one byte per
strip, for every strip in the frame.** Values are 0..3 and track picture content: a quiescent frame's
map is all zero, a busy one is a mix. Bytes 20..31 of every band are zero in all 5760 bytes of
every map.

⚠ **What the values mean is not established.** vino sends the all-zero map, which is byte-for-byte
what DLM sends on the quiescent startup frames that do light this dock. Do not invent values.

⚠ Band **ordering** within the second record varies in DLM's own output (`120, 136, 128, 144…`) —
its parallel encoder finishing out of order, like the `aux` lane tag in §3.3. Coverage is always
complete. vino emits them in order, which reproduces DLM's in-order instances byte-for-byte.

This was the only record kind in DLM's video stream that vino never emitted, and it is most of the
~4.4 KB per frame by which vino's frames were smaller than DLM's (204,208 vs 208,640 bytes for the
same quiescent 1440p frame). Implemented as `video::wht::navarro_strip_params()`, sent ahead of the
image records — it is what tells the dock how to read them.

## 4. What is eliminated for the video jam

The dock accepts one 65536-byte URB per endpoint and then never drains again; ~1.05 s later every
outstanding URB completes `-ESHUTDOWN` and the control plane goes deaf too.

Eliminated by measurement, in addition to the four in the previous handover:

* **Control-plane completeness.** Counting outbound frames up to the first video byte, vino and
  DLM agree exactly on every message size that carries an HDCP, mode-set or stream record:
  80x22, 192x4, 96x4, 112x2, 176x1, 48x1, 32x1, 16x1. vino sends more 64-byte status polls, and
  nothing else differs. No control message is missing.
* **The per-connector video key.** Transmitted by the transmitter (§2.2); vino chooses and sends
  its own.
* **`RepeaterAuth_Stream_Manage`.** Sent, with the right stream ids (§2.3).
* **The frame bracket.** Sent, byte-exact (§3.2).
* **`aux`.** Legitimately 0 (§3.3).
* **URB record alignment.** Not required (§3.1).
* **Fixed frame size.** DLM's frames run 208,608 to 7,617,776 bytes on one connector, so the dock
  is not waiting for a fixed byte count.

Remaining, in order:

1. **The per-strip parameter map** (§3.4) — implemented 2026-08-03, **not yet hardware-tested**.
   This is the strongest candidate: without it the dock has no per-strip parameters for any strip
   it is being sent, which is a coherent reason to accept one FIFO's worth of image records and
   then stop.
2. **The mode-set words** (§2.4), wrong until 2026-08-03 and also untested.
3. **Startup pacing** — DLM ramps from 6.3 MB/s over a second with **at most two URBs in flight**;
   vino queues four to eight from a standing start.
