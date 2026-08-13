#!/usr/bin/env python3
"""Reconstruct full pixels from a DL-3x00 strip stream, AC included.

The DC-only renderer settled the strip grammar, but it is structurally blind to everything above
DC: the coefficient scan, the subband layout and the transform's scaling all change pixel values
without changing a single code length.  A panel showing correct large-scale colour under per-pixel
noise is exactly what that blindness looks like from the outside, so this decodes the whole strip
and inverts the transform.

Run it against the vendor's capture first.  If the vendor's own frame does not come out sharp, the
model here is wrong and nothing it says about the driver's frame means anything.

  ella_render.py CAP.pcapng OUT.png [--skip N] [--frames N]
"""
import argparse
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from ella_decode import R, bits, esc, unit, strips_from, DC_CMAX, AC_CMAX, CHROMA_AC_CMAX  # noqa

from PIL import Image  # noqa: E402

WIDTH, HEIGHT = 1920, 1088
BLOCK = 8
COEFFS = 64

# Quantiser step per coefficient, as the decoder configuration declares it and as the encoder
# applies it.  Luma: DC 16, level-3 16/16/32, level-2 4/4/8, level-1 2/2/4.  Chroma: DC 64,
# level-3 16/16/32, level-2 16/16/32, level-1 32/32/64.
def luma_step(i):
    if i == 0 or i in (1, 2):
        return 16
    if i == 3:
        return 32
    if 4 <= i <= 11:
        return 4
    if 12 <= i <= 15:
        return 8
    if 16 <= i <= 47:
        return 2
    return 4


def chroma_step(i):
    if i == 0:
        return 64
    if i in (1, 2) or 4 <= i <= 11:
        return 16
    if i >= 48:
        return 64
    return 32


SCAN4_MORTON = [0, 2, 8, 10, 1, 3, 9, 11, 4, 6, 12, 14, 5, 7, 13, 15]


def ihaar2d(ll, hl, lh, hh, h):
    """Invert one separable 2-D Haar level: four h x h subbands -> one 2h x 2h block."""
    n = 2 * h
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


def inverse_transform(c):
    """64 coefficients in the wire's Mallat layout -> an 8x8 block, undoing `transform()`."""
    # The forward pass floor-divides by 64 after a gain-64 transform, so scale back first.
    c = [v * 64 for v in c]
    ll3 = [c[0]]
    hl3, lh3, hh3 = [c[1]], [c[2]], [c[3]]
    ll2 = ihaar2d(ll3, hl3, lh3, hh3, 1)
    hl2 = [c[4 + i] for i in range(4)]
    lh2 = [c[8 + i] for i in range(4)]
    hh2 = [c[12 + i] for i in range(4)]
    ll1 = ihaar2d(ll2, hl2, lh2, hh2, 2)
    hl1 = [0] * 16
    lh1 = [0] * 16
    hh1 = [0] * 16
    for i in range(16):
        m = SCAN4_MORTON[i]
        hl1[m] = c[16 + i]
        lh1[m] = c[32 + i]
        hh1[m] = c[48 + i]
    return ihaar2d(ll1, hl1, lh1, hh1, 4)


def decode_strip(s):
    """-> (x, y, [ (cr, cb, y) as 8x8 blocks ] * 16), or None if the strip does not decode."""
    w18 = int.from_bytes(s[10:12], "little")
    w1c = int.from_bytes(s[12:14], "little")
    if not (16 <= w18 <= w1c <= len(s)):
        return None
    r = R(bits(s[16:w18]))
    lasts = [unit(r) for _ in range(16)]
    dc = []
    pcr = pcb = py = 0
    for _ in range(16):
        pcr += esc(r, DC_CMAX)
        pcb += esc(r, DC_CMAX)
        py += esc(r, DC_CMAX)
        dc.append((pcr, pcb, py))

    planes = [[[0] * COEFFS for _ in range(3)] for _ in range(16)]
    for b in range(16):
        planes[b][0][0] = dc[b][0]
        planes[b][1][0] = dc[b][1]
        planes[b][2][0] = dc[b][2]

    # Two AC rows: blocks 0..8 in the first region, 8..16 in the second.
    for row, (lo, hi, start, end) in enumerate(
        ((0, 8, w18, w1c), (8, 16, w1c, len(s)))
    ):
        rr = R(bits(s[start:end]))
        try:
            for b in range(lo, hi):
                lcr, lcb, ly = lasts[b]
                for plane, (last, cmax) in enumerate(
                    ((lcr, CHROMA_AC_CMAX), (lcb, CHROMA_AC_CMAX), (ly, AC_CMAX))
                ):
                    for i in range(1, last + 1):
                        planes[b][plane][i] = esc(rr, cmax)
        except (EOFError, IndexError):
            pass

    out = []
    for b in range(16):
        cr = inverse_transform([planes[b][0][i] * chroma_step(i) for i in range(COEFFS)])
        cb = inverse_transform([planes[b][1][i] * chroma_step(i) for i in range(COEFFS)])
        y = inverse_transform([planes[b][2][i] * luma_step(i) for i in range(COEFFS)])
        out.append((cr, cb, y))
    x = int.from_bytes(s[2:4], "little")
    yy = int.from_bytes(s[4:6], "little")
    return x, yy, out


def to_rgb(y, cb, cr):
    """Invert `colour()`: Y = 64G + 64*((Cb+Cr)>>2), Cb = 64(R-G), Cr = 64(B-G)."""
    cb_raw = cb // 64
    cr_raw = cr // 64
    g = (y - 64 * ((cb_raw + cr_raw) >> 2)) // 64
    return g + cb_raw, g, g + cr_raw


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("cap")
    ap.add_argument("out")
    ap.add_argument("--skip", type=int, default=0)
    ap.add_argument("--strips", type=int, default=2040)
    args = ap.parse_args()

    img = Image.new("RGB", (WIDTH, HEIGHT))
    px = img.load()
    seen = set()
    n = bad = 0
    for s in strips_from(args.cap, args.skip + args.strips * 2)[args.skip:]:
        got = decode_strip(s)
        if got is None:
            bad += 1
            continue
        x, y, blocks = got
        if (x, y) in seen:
            break
        seen.add((x, y))
        n += 1
        for k, (cr, cb, yy) in enumerate(blocks):
            bx = x + (k % 8) * BLOCK
            by = y + (k // 8) * BLOCK
            for j in range(BLOCK):
                for i in range(BLOCK):
                    r, g, b = to_rgb(yy[j * BLOCK + i], cb[j * BLOCK + i], cr[j * BLOCK + i])
                    if 0 <= bx + i < WIDTH and 0 <= by + j < HEIGHT:
                        px[bx + i, by + j] = (
                            max(0, min(255, r)),
                            max(0, min(255, g)),
                            max(0, min(255, b)),
                        )
    print(f"{n} strips rendered, {bad} undecodable")
    img.save(args.out)
    print("wrote", args.out)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
