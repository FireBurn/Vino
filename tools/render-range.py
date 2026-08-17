#!/usr/bin/env python3
"""Render the DC image of the records in a chosen byte range of the OUT stream.

`strips_from` stops at the first bad stride and caps the stream at 40 MB, so it cannot reach a
frame 136 MB in. This uses the resyncing walk instead, and restricts decoding to one frame's byte
range so a specific frame -- the one the dock halted on -- can be rendered on its own.

    render-range.py CAP.pcapng --list
    render-range.py CAP.pcapng --from N --to M --out X.png
"""
import argparse
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent / "codec"))
from usbmon_read import iter_transfers            # noqa: E402
from ella_decode import R, bits, esc, unit, DC_CMAX  # noqa: E402
from PIL import Image                             # noqa: E402

CODEC_SYNC = b"\x01\x28"
SUB_CP = (0x04, 0x24, 0x25, 0x45)
WIDTH, HEIGHT, BLOCK = 1920, 1088, 8


def load(path):
    return b"".join(p for _d, _e, p in iter_transfers(path, endpoint=2, transfer_type=3))


def plausible(buf, off):
    if off + 16 > len(buf):
        return None
    size = int.from_bytes(buf[off + 2:off + 4], "little")
    stride = size + 4
    if stride < 16 or stride % 16 or stride > 4096 or off + stride > len(buf):
        return None
    sub = int.from_bytes(buf[off + 8:off + 10], "little")
    aux = int.from_bytes(buf[off + 10:off + 12], "little")
    if buf[off + 18:off + 20] == CODEC_SYNC:
        return stride
    if sub in SUB_CP:
        return stride
    if size == 44 and aux == 0x000A and buf[off + 16:off + 19] == b"\x08\x00\x05":
        return stride
    return None


def walk(buf):
    off = 0
    while off + 16 <= len(buf):
        stride = plausible(buf, off)
        if stride is None:
            off += 16
            continue
        yield off, stride, buf[off:off + stride]
        off += stride


def strip_dc(s):
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
    return int.from_bytes(s[2:4], "little"), int.from_bytes(s[4:6], "little"), out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("cap")
    ap.add_argument("--list", action="store_true")
    ap.add_argument("--from", dest="lo", type=int, default=0)
    ap.add_argument("--to", dest="hi", type=int, default=1 << 62)
    ap.add_argument("--out")
    a = ap.parse_args()

    buf = load(a.cap)

    if a.list:
        runs = []
        for off, stride, rec in walk(buf):
            if rec[18:20] != CODEC_SYNC:
                continue
            head = int.from_bytes(rec[8:10], "little") & 0x0F
            if runs and runs[-1][0] == head and runs[-1][2] + 16384 > off:
                runs[-1][2] = off + stride
                runs[-1][3] += 1
            else:
                runs.append([head, off, off + stride, 1])
        print("last 10 image runs (head, start, end, records):")
        for head, lo, hi, n in runs[-10:]:
            print(f"  head {head}  {lo:>12,} .. {hi:>12,}  ({hi-lo:>9,} B, {n} recs)")
        return

    bw, bh = WIDTH // BLOCK, HEIGHT // BLOCK
    luma = [[0] * bw for _ in range(bh)]
    chroma = [[(0, 0)] * bw for _ in range(bh)]
    n = bad = 0
    for off, stride, rec in walk(buf):
        if off < a.lo or off >= a.hi or rec[18:20] != CODEC_SYNC:
            continue
        body = rec[16:]
        p = 0
        while p + 2 <= len(body):
            ln = int.from_bytes(body[p:p + 2], "little")
            if ln == 0 or p + 2 + ln > len(body):
                break
            s = body[p + 2:p + 2 + ln]
            p += 2 + ln
            try:
                x, y, dcs = strip_dc(s)
            except (EOFError, IndexError):
                bad += 1
                continue
            n += 1
            for k, (cr, cb, yv) in enumerate(dcs):
                bx, by = x // BLOCK + (k % 8), y // BLOCK + (k // 8)
                if 0 <= bx < bw and 0 <= by < bh:
                    luma[by][bx] = yv
                    chroma[by][bx] = (cr, cb)

    vals = [v for row in luma for v in row]
    lo_v, hi_v = min(vals), max(vals)
    print(f"range {a.lo:,}..{a.hi:,}: {n} strips decoded, {bad} undecodable; "
          f"luma DC {lo_v}..{hi_v}")
    if lo_v < 0:
        print("  luma DC cannot be negative -- decode is off the rails")
    span = max(hi_v - lo_v, 1)
    img = Image.new("RGB", (bw, bh))
    px = img.load()
    for j in range(bh):
        for i in range(bw):
            v = int(255 * (luma[j][i] - lo_v) / span)
            cr, cb = chroma[j][i]
            px[i, j] = (max(0, min(255, v + (cr >> 2))), v, max(0, min(255, v + (cb >> 2))))
    if a.out:
        img.resize((bw * 2, bh * 2), Image.NEAREST).save(a.out)
        print("wrote", a.out)


main()
