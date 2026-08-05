#!/usr/bin/env python3
"""Decode a DL7400 video stream back to pixels and score it against known ground truth.

This exists to answer one question that no self-consistency check can: is the repository's model
of the codec right for *busy* strips?  vino's own encoder and this decoder share their model, so
round-tripping vino proves nothing about it.  Feeding DLM's real bytes through the same decoder
does -- if a DLM strip of known content decodes to the reference, the model holds; if flat strips
land and detailed ones turn to noise, the model is wrong exactly where the panels are wrong.

  navarro-render.py <capture> --ep 8 --sub 0x10 --ref screen-ref.png [--out PREFIX]

`<capture>` is a USBPcap file (the Windows corpus) or a usbmon pcapng (the Linux corpus).
"""

import argparse
import os
import struct
import sys

_HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, _HERE)

import colour_decode as cd
from usbpcap_read import iter_transfers
from usbmon_read import iter_transfers as usbmon_transfers

from PIL import Image

STRIP_MAGIC = b"\x01\x28"


def concat_ep(path, ep, device=None):
    """Concatenate every bulk-OUT payload for one endpoint, in capture order.

    Accepts both corpora: the Windows captures are USBPcap, the Linux ones usbmon pcapng.
    """
    reader = usbmon_transfers if path.endswith(".pcapng") else iter_transfers
    out = bytearray()
    kw = {"device": device} if device is not None else {}
    for _device, _ep, data in reader(path, endpoint=ep, transfer_type=3, out_only=True, **kw):
        out += data
    return bytes(out)


def walk_records(blob):
    """Yield `(offset, size, type, sub, aux, body)` for each outer record.

    The stream is contiguous across USB transfer boundaries, so this walks the concatenation.  A
    record that does not parse resynchronises by scanning forward for the next plausible header
    rather than abandoning the rest of the capture.
    """
    o = 0
    n = len(blob)
    while o + 16 <= n:
        size = struct.unpack_from("<H", blob, o + 2)[0]
        typ = struct.unpack_from("<I", blob, o + 4)[0]
        if size < 12 or size > 0x9000 or o + 4 + size > n or typ not in (4, 5, 6):
            o += 4
            continue
        sub = struct.unpack_from("<H", blob, o + 8)[0]
        aux = struct.unpack_from("<H", blob, o + 10)[0]
        yield o, size, typ, sub, aux, blob[o + 16:o + 4 + size]
        o += size + 4


def strips_in(body, aux):
    """Yield raw strip bodies from one image record's payload."""
    p, end = 0, max(0, len(body) - aux)
    while p + 2 <= end:
        sl = struct.unpack_from("<H", body, p)[0]
        if sl < 16 or p + 2 + sl > end:
            break
        s = body[p + 2:p + 2 + sl]
        if s[:2] == STRIP_MAGIC:
            yield s
        p += 2 + sl


def frames_of(blob, sub):
    """Split one connector's records into logical frames on its aux=0x0006 close record."""
    frames = []
    cur = []
    for _o, _size, typ, rsub, aux, body in walk_records(blob):
        if rsub != sub:
            continue
        if typ == 4 and aux == 0x0006:
            if cur:
                frames.append(cur)
                cur = []
            continue
        if typ == 4:
            cur.extend(strips_in(body, aux))
    if cur:
        frames.append(cur)
    return frames


def compose(strips, width, height, blocks_x=16, depth=None):
    """Decode strips onto an RGB surface. Untouched pixels stay magenta so gaps are visible."""
    surf = [[(255, 0, 255)] * width for _ in range(height)]
    placed = 0
    failed = 0
    for s in strips:
        try:
            x, y, blocks = cd.decode_strip(s, depth)
            tile = cd.strip_rgb(blocks, blocks_x=blocks_x)
        except Exception:
            failed += 1
            continue
        for j, row in enumerate(tile):
            if y + j >= height:
                break
            trow = surf[y + j]
            for i, px in enumerate(row):
                if x + i < width and px is not None:
                    trow[x + i] = px
        placed += 1
    return surf, placed, failed


def save(surf, path, shift=0):
    """Write the surface as an 8-bit PNG.

    A 10-bit decode carries values up to 1023, so `shift` brings them back into a byte. It is a
    shift and not a rescale so a wrong depth stays obvious in the picture rather than being
    quietly normalised away.
    """
    h, w = len(surf), len(surf[0])
    im = Image.new("RGB", (w, h))
    im.putdata([tuple(min(255, max(0, c >> shift)) for c in px)
                for row in surf for px in row])
    im.save(path)
    return im


def score(surf, ref_path, width, height):
    """Per-strip mean absolute error against the reference, split flat vs detailed."""
    ref = Image.open(ref_path).convert("RGB").resize((width, height))
    rp = ref.load()
    rows = []
    for by in range(0, height, 8):
        for bx in range(0, width, 128):
            err = 0
            n = 0
            var = 0
            first = None
            for j in range(8):
                for i in range(0, 128, 4):  # sample every 4th column: enough for a strip verdict
                    x, y = bx + i, by + j
                    if x >= width or y >= height:
                        continue
                    r0, g0, b0 = surf[y][x]
                    r1, g1, b1 = rp[x, y]
                    err += abs(r0 - r1) + abs(g0 - g1) + abs(b0 - b1)
                    if first is None:
                        first = (r1, g1, b1)
                    var += abs(r1 - first[0]) + abs(g1 - first[1]) + abs(b1 - first[2])
                    n += 3
            if n:
                rows.append((bx, by, err / n, var / n))
    return rows


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("capture")
    ap.add_argument("--ep", type=lambda v: int(v, 0), default=0x08)
    ap.add_argument("--sub", type=lambda v: int(v, 0), default=0x0010)
    ap.add_argument("--width", type=int, default=2560)
    ap.add_argument("--height", type=int, default=1440)
    ap.add_argument("--blocks-x", type=int, default=16)
    ap.add_argument("--frame", type=int, default=None, help="frame index; default = most strips")
    ap.add_argument("--ref")
    ap.add_argument("--depth", type=int, choices=(8, 10), default=8,
                    help="sample depth of the captured stream; 10 for a dock in its HDR profile")
    ap.add_argument("--out", default="navarro")
    args = ap.parse_args()

    blob = concat_ep(args.capture, args.ep)
    print(f"endpoint 0x{args.ep:02x}: {len(blob):,} bytes")

    frames = frames_of(blob, args.sub)
    print(f"sub 0x{args.sub:04x}: {len(frames)} frames, "
          f"strips per frame max {max((len(f) for f in frames), default=0)}")

    if not frames:
        raise SystemExit("no frames for that connector")

    idx = args.frame
    if idx is None:
        idx = max(range(len(frames)), key=lambda i: len(frames[i]))
    print(f"decoding frame {idx} ({len(frames[idx])} strips)")

    depth = cd.Depth.ten() if args.depth == 10 else cd.Depth.eight()
    if args.depth == 10 and not depth.ac_measured:
        print("note: the 10-bit AC codebook ceilings are inherited from 8-bit and unmeasured "
              "(see docs/hdr.md); a strip with a large AC coefficient may decode wrong")
    surf, placed, failed = compose(frames[idx], args.width, args.height, args.blocks_x, depth)
    print(f"placed {placed} strips, {failed} failed to decode")
    save(surf, f"{args.out}-decoded.png", shift=depth.bits - 8)
    print(f"wrote {args.out}-decoded.png")

    if args.ref:
        rows = score(surf, args.ref, args.width, args.height)
        touched = [r for r in rows if surf[r[1]][r[0]] != (255, 0, 255)]
        flat = [r for r in touched if r[3] < 4]
        busy = [r for r in touched if r[3] >= 4]
        print(f"strips scored: {len(touched)} covered of {len(rows)}")
        for name, group in (("flat", flat), ("busy", busy)):
            if group:
                errs = sorted(r[2] for r in group)
                print(f"  {name:4s} n={len(group):5d}  mean {sum(errs)/len(errs):7.2f}  "
                      f"median {errs[len(errs)//2]:7.2f}  p95 {errs[int(len(errs)*0.95)]:7.2f}  "
                      f"max {errs[-1]:7.2f}")


if __name__ == "__main__":
    main()
