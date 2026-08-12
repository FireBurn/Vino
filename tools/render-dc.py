#!/usr/bin/env python3
"""Reconstruct a DL-3x00 frame from its DC coefficients and write a PNG.

The strongest check on a strip grammar that does not need a dock, and the only one that can see an
escape payload's bit order: reversing it changes coefficient values but not code lengths, so every
landing-based oracle scores it identically. Pixels do not -- get it wrong and a desktop becomes
streaks.

  render-dc.py CAP.pcapng OUT.png [--skip N]

`--skip` steps past the flat carrier frame every stream opens with, whose DC is uniformly zero.
"""
import argparse
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent / "codec"))
from ella_decode import R, bits, esc, unit, strips_from, DC_CMAX  # noqa: E402

from PIL import Image  # noqa: E402

WIDTH, HEIGHT = 1920, 1088
BLOCK = 8


def strip_dc(s):
    """-> (x, y, [(cr, cb, y)] * 16) from the main section alone."""
    r = R(bits(s[16:]))
    for _ in range(16):
        unit(r)
    out = []
    pcr = pcb = py = 0
    for _ in range(16):
        pcr += esc(r, DC_CMAX)
        pcb += esc(r, DC_CMAX)
        py += esc(r, DC_CMAX)
        out.append((pcr, pcb, py))
    x = int.from_bytes(s[2:4], "little")
    y = int.from_bytes(s[4:6], "little")
    return x, y, out


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("cap")
    ap.add_argument("out")
    ap.add_argument("--skip", type=int, default=0, help="strips to step past first")
    args = ap.parse_args()

    bw, bh = WIDTH // BLOCK, HEIGHT // BLOCK
    luma = [[0] * bw for _ in range(bh)]
    chroma = [[(0, 0)] * bw for _ in range(bh)]
    seen = set()
    n = bad = 0
    # Strips arrive in transmission order and cover the surface once; a repeated coordinate is the
    # next frame starting.
    for s in strips_from(args.cap, args.skip + 4000)[args.skip:]:
        try:
            x, y, dcs = strip_dc(s)
        except (EOFError, IndexError):
            bad += 1
            continue
        if (x, y) in seen:
            break
        seen.add((x, y))
        n += 1
        for k, (cr, cb, yv) in enumerate(dcs):
            bx, by = x // BLOCK + (k % 8), y // BLOCK + (k // 8)
            if 0 <= bx < bw and 0 <= by < bh:
                luma[by][bx] = yv
                chroma[by][bx] = (cr, cb)

    vals = [v for row in luma for v in row]
    lo, hi = min(vals), max(vals)
    print(f"{n} strips, {bad} undecodable; luma DC {lo}..{hi}")
    if lo < 0:
        print("  luma DC cannot be negative -- the grammar is wrong somewhere")
    span = max(hi - lo, 1)
    img = Image.new("RGB", (bw, bh))
    px = img.load()
    for j in range(bh):
        for i in range(bw):
            v = int(255 * (luma[j][i] - lo) / span)
            cr, cb = chroma[j][i]
            px[i, j] = (max(0, min(255, v + (cr >> 2))), v, max(0, min(255, v + (cb >> 2))))
    img.resize((bw * 2, bh * 2), Image.NEAREST).save(args.out)
    print("wrote", args.out)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
