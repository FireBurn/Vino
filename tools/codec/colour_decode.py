#!/usr/bin/env python3
"""A full decoder for vino's COLOUR 64x16 strip -- bitstream back to RGB pixels.

The repo had no colour decoder: `dec.py` implements the older LUMA grammar (different sync
field) and `colourstrip.py` is an *encoder* mirror. Without an inverse there was no way to ask
the only question that matters -- "do the bytes vino sends actually decode back to the pixels it
was given?" -- so every codec theory had to be argued rather than measured.

This mirrors `drivers/gpu/drm/vino/video.rs` exactly:

  strip header : [0]=0x01 [1]=0x28, x@2, y@4, w18@10, w1c@12  (w18 = start of AC row0,
                 w1c = start of AC row1, len = w1c + round_even(row1))
  main @16     : 16 x colour_sync_unit, then 16 x (Cr,Cb,Y) DC DPCM escapes (cmax 10)
  row0 @w18    : blocks 0..8  AC
  row1 @w1c    : blocks 8..16 AC
  AC per block : planes (Cr,Cb,Y), positions 1..=last, `0` for an insignificant coeff else
                 the magnitude escape (cmax 9)

All bit fields are LSB-first within each byte.

The significance tree shares its two leading zero bits with the chroma planes: a present Cr
replaces the first root branch, a present Cb the second (`colour_sync_unit`), so the luma field
is emitted with 1 or 2 of its leading bits elided. Decoding therefore has to reconstruct those
bits rather than read them.
"""

import struct

COEFFS = 64
DIM = 8
PIXELS = 64
DC_CMAX = 10
AC_CMAX = 9
# Chroma AC uses a HIGHER codebook maximum than luma, so a category-9 chroma coefficient still
# carries the unary 0-terminator that luma's omits. Recovered 2026-07-27 against DLM on high-chroma
# content (the old corpus had none). Decoding chroma with luma's cmax desyncs by one bit on any
# block holding |q| >= 256 -- which is exactly the thumbnail artifact.
CHROMA_AC_CMAX = 10
SCAN4_MORTON = [0, 2, 8, 10, 1, 3, 9, 11, 4, 6, 12, 14, 5, 7, 13, 15]


class BitReader:
    """LSB-first bit reader over a byte slice, starting at a byte offset."""

    def __init__(self, buf, byte_off):
        self.b = buf
        self.p = byte_off * 8

    def bit(self):
        i = self.p >> 3
        if i >= len(self.b):
            raise EOFError("bitstream overrun")
        v = (self.b[i] >> (self.p & 7)) & 1
        self.p += 1
        return v

    def msb(self, n):
        v = 0
        for _ in range(n):
            v = (v << 1) | self.bit()
        return v


def dec_esc(r, cmax):
    """Inverse of `Bits::esc`: 0 -> one `0` bit; else unary(c) [+ 0-term if c<cmax]
    + (c-1)-bit MSB-first offset + sign (1 = positive)."""
    c = 0
    while c < cmax and r.bit():
        c += 1
    if c == 0:
        return 0
    if c < cmax:
        pass  # the terminating 0 was already consumed by the loop condition
    off = r.msb(c - 1) if c > 1 else 0
    sign = r.bit()
    mag = (1 << (c - 1)) + off
    return mag if sign else -mag


def dec_chroma_base(r, first_one_consumed=True):
    """Inverse of `Bits::chroma_base`: `1`xc + `0` + c-bit MSB-first offset;
    last = (2^c - 1) + offset. The caller has already consumed the leading `1`."""
    c = 1 if first_one_consumed else 0
    while r.bit():
        c += 1
    off = r.msb(c)
    return ((1 << c) - 1) + off


def dec_luma_after(r, skip):
    """Inverse of `Bits::sync_unit_after`: `skip` leading zero bits were elided by the caller.

    last == 0 -> `0,0` + six `1` + seven `0`   (15 bits)
    last  > 0 -> `0,0` + k x `1` + `0` + k-bit MSB-first v, last = (64 - 2^k) - v, k <= 5
    """
    for _ in range(2 - skip):
        if r.bit() != 0:
            raise ValueError("luma sync: expected a root zero bit")
    ones = 0
    while ones < 6 and r.bit():
        ones += 1
    if ones == 6:
        for _ in range(7):
            r.bit()
        return 0
    k = ones
    v = r.msb(k) if k else 0
    return (COEFFS - (1 << k)) - v


def dec_sync_unit(r):
    """One block's three-plane significance tree -> (lcr, lcb, ly)."""
    if r.bit():
        lcr = dec_chroma_base(r)
        if r.bit():                       # chroma_base(lcb) starts with a 1
            lcb = dec_chroma_base(r)
            return lcr, lcb, dec_luma_after(r, 2)
        # that 0 was the luma field's second root branch; the first was elided
        return lcr, 0, dec_luma_after(r, 2)
    if r.bit():
        lcb = dec_chroma_base(r)
        return 0, lcb, dec_luma_after(r, 2)
    return 0, 0, dec_luma_after(r, 2)     # both root zeros consumed


def step_bias(i):
    if i in (0, 1, 2):
        return 16, 8
    if i == 3:
        return 32, 16
    if 4 <= i <= 11:
        return 4, 2
    if 12 <= i <= 15:
        return 8, 4
    if 16 <= i <= 47:
        return 2, 0
    return 4, 2


def chroma_ac_step(i):
    if i in (1, 2) or 4 <= i <= 11:
        return 16
    if i >= 48:
        return 64
    return 32


def decode_strip(s):
    """Decode one colour strip body -> list of 16 blocks, each (qcr, qcb, qy)."""
    if s[0] != 0x01 or s[1] != 0x28:
        raise ValueError("not a vino strip (bad magic)")
    x, y = struct.unpack_from("<H", s, 2)[0], struct.unpack_from("<H", s, 4)[0]
    w18, w1c = struct.unpack_from("<H", s, 10)[0], struct.unpack_from("<H", s, 12)[0]

    main = BitReader(s, 16)
    lasts = [dec_sync_unit(main) for _ in range(16)]

    dcs = []
    pcr = pcb = py = 0
    for _ in range(16):
        pcr += dec_esc(main, DC_CMAX)
        pcb += dec_esc(main, DC_CMAX)
        py += dec_esc(main, DC_CMAX)
        dcs.append((pcr, pcb, py))

    blocks = []
    for row, off in ((range(0, 8), w18), (range(8, 16), w1c)):
        r = BitReader(s, off)
        for k in row:
            lcr, lcb, ly = lasts[k]
            qcr = [0] * COEFFS
            qcb = [0] * COEFFS
            qy = [0] * COEFFS
            qcr[0], qcb[0], qy[0] = dcs[k]
            for q, last, cmax in (
                (qcr, lcr, CHROMA_AC_CMAX),
                (qcb, lcb, CHROMA_AC_CMAX),
                (qy, ly, AC_CMAX),
            ):
                for i in range(1, last + 1):
                    q[i] = dec_esc(r, cmax)
            blocks.append((k, qcr, qcb, qy))
    blocks.sort(key=lambda b: b[0])
    return x, y, [(b[1], b[2], b[3]) for b in blocks]


def dequant(qcr, qcb, qy):
    """Undo the per-plane quantizers (the reconstruction the dock must perform)."""
    tcr = [0] * COEFFS
    tcb = [0] * COEFFS
    ty = [0] * COEFFS
    tcr[0] = qcr[0] * 64
    tcb[0] = qcb[0] * 64
    ty[0] = qy[0] * 16
    for i in range(1, COEFFS):
        st = chroma_ac_step(i)
        tcr[i] = qcr[i] * st
        tcb[i] = qcb[i] * st
        ty[i] = qy[i] * step_bias(i)[0]
    return tcr, tcb, ty


def ihaar2d(ll, hl, lh, hh, n):
    """Inverse of `haar2d` (forward is unnormalised a+b / a-b, so the inverse halves)."""
    h = n // 2
    l = [0] * (n * h)
    hb = [0] * (n * h)
    for c in range(h):
        for i in range(h):
            a, b = ll[i * h + c], lh[i * h + c]
            l[(2 * i) * h + c] = (a + b) // 2
            l[(2 * i + 1) * h + c] = (a - b) // 2
            a2, b2 = hl[i * h + c], hh[i * h + c]
            hb[(2 * i) * h + c] = (a2 + b2) // 2
            hb[(2 * i + 1) * h + c] = (a2 - b2) // 2
    out = [0] * (n * n)
    for r in range(n):
        for i in range(h):
            a, b = l[r * h + i], hb[r * h + i]
            out[r * n + 2 * i] = (a + b) // 2
            out[r * n + 2 * i + 1] = (a - b) // 2
    return out


def itransform(c):
    """Inverse of `video::wht::transform` (each level was floor-divided by 64 on the way out)."""
    ll3 = [c[0] << 6]
    hl3 = [c[1] << 6]
    lh3 = [c[2] << 6]
    hh3 = [c[3] << 6]
    ll2 = ihaar2d(ll3, hl3, lh3, hh3, 2)
    hl2 = [c[4 + i] << 6 for i in range(4)]
    lh2 = [c[8 + i] << 6 for i in range(4)]
    hh2 = [c[12 + i] << 6 for i in range(4)]
    ll1 = ihaar2d(ll2, hl2, lh2, hh2, 4)
    hl1 = [0] * 16
    lh1 = [0] * 16
    hh1 = [0] * 16
    for p in range(16):
        hl1[SCAN4_MORTON[p]] = c[16 + p] << 6
        lh1[SCAN4_MORTON[p]] = c[32 + p] << 6
        hh1[SCAN4_MORTON[p]] = c[48 + p] << 6
    return ihaar2d(ll1, hl1, lh1, hh1, 8)


def block_rgb(qcr, qcb, qy):
    """One 8x8 block of reconstructed RGB, inverting `video::wht::colour`."""
    tcr, tcb, ty = dequant(qcr, qcb, qy)
    scr = itransform(tcr)
    scb = itransform(tcb)
    sy = itransform(ty)
    out = []
    for i in range(PIXELS):
        cb = scb[i] // 64          # r - g
        cr = scr[i] // 64          # b - g
        g = sy[i] // 64 - ((cb + cr) >> 2)
        out.append((g + cb, g, g + cr))
    return out


def strip_rgb(blocks, blocks_x=8):
    """Place 16 decoded blocks using the selected dock profile's strip geometry.

    Ridge uses 8 blocks across by 2 down (64x16); Navarro uses all 16 blocks
    across by 1 down (128x8).  Entropy decoding is unchanged -- only the spatial
    placement of the already decoded blocks differs.
    """
    if blocks_x not in (8, 16):
        raise ValueError(f"unsupported blocks-per-strip row: {blocks_x}")
    rows = 16 // blocks_x
    tile = [[None] * (blocks_x * 8) for _ in range(rows * 8)]
    for k, (qcr, qcb, qy) in enumerate(blocks):
        bx, by = (k % blocks_x) * 8, (k // blocks_x) * 8
        px = block_rgb(qcr, qcb, qy)
        for j in range(8):
            for i in range(8):
                tile[by + j][bx + i] = px[j * 8 + i]
    return tile


def records_to_strips(b):
    """Walk the EP08 record stream, yielding raw strip bodies."""
    o = 0
    out = []
    while o + 16 <= len(b):
        size = struct.unpack_from("<H", b, o + 2)[0]
        if size == 0 or size > 0x9000:
            break
        aux = struct.unpack_from("<H", b, o + 10)[0]
        p, end = o + 16, o + 4 + size - aux
        while p + 2 <= end:
            sl = struct.unpack_from("<H", b, p)[0]
            if sl == 0 or p + 2 + sl > end:
                break
            out.append(b[p + 2:p + 2 + sl])
            p += 2 + sl
        o += size + 4
    return out
