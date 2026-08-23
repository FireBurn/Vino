# Video path

## Pipeline

```text
XRGB8888 framebuffer
  → coherent driver-owned strip snapshot
  → integer Y/Cb/Cr transform
  → 8×8 multilevel Haar transform (Mallat decomposition)
  → power-of-two quantization
  → DisplayLink VLC coding
  → 64×16 strip record
  → at-most-4096-byte record envelope
  → per-head bulk-OUT request ring
```

The codec is implemented in `video.rs`; the decoder setup generated during a
cold arm is in `video_arm.rs`.

## Geometry and color

Damage is quantized to 64×16 strips. An 8×8 input block produces 64 transform
coefficients. The fixed-point color transform is:

```text
Cb = 64 × (R - G)
Cr = 64 × (B - G)
Y  = 64 × G + 64 × (((R - G) + (B - G)) >> 2)
```

The arithmetic shift is significant for negative chroma values. Quantizer
steps are powers of two and use shifts, avoiding divisions in the hot path.

## Records

Each complete strip remains intact inside an image record. Records are
16-byte-aligned and no larger than 4096 bytes. The header identifies the head,
strip parity, padding, and record sequence. USB requests carry the concatenated
logical frame; only the final request may be short.

The first presentation after mode activation prefixes the frame with the
decoder configuration and training sequence. Arm construction is fallible and
an incomplete record is never submitted.

## Damage and retries

DRM damage clips alone are insufficient because a newer atomic commit can
supersede an unsent one. Vino fingerprints strips against the last frame that
the dock successfully received. Failed submissions leave those strips dirty.
A bounded debt mechanism retransmits strips needed by the dock's internal
buffers.

Encoding reuses immutable snapshot data and cached encoded strips where a debt
retransmission does not require a new source read. Static desktops therefore
stop producing traffic once presentation state is synchronized.

## Independent validation

The production encoder is tested against captured DisplayLinkManager records
and an independently structured userspace oracle. Boundary cases include the
separate luma/chroma escape ranges, padding, non-aligned display dimensions,
record-size limits, and both heads.

