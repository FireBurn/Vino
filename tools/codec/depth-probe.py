#!/usr/bin/env python3
"""Find the escape-codebook ceiling a captured video stream was encoded with.

A wrong ceiling does not crash the decoder. The unary prefix is the only place the ceiling shows:
at the maximum category the encoder omits the 0-terminator, so reading with a ceiling that is too
low consumes the *next* value's first bit as an offset bit and everything after it is plausible
rubbish. That is why the DL7400's 10-bit stream decoded to negative luma for a year's worth of
tooling without anything looking broken.

The oracle is content, not framing: a monotonic ramp across the screen must decode monotonically.
Point this at a horizontal band of a captured PQ ramp and only the right ceiling survives.

  depth-probe.py <capture> --device 5 --ep 8 --row 720 --since 15:42:54 --until 15:42:55

Written for `captures/navarro-wincap-20260805/`, whose `hdr-content/` carries a 64-step PQ ramp
exactly for this. See `docs/hdr.md`.
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

STRIP_MAGIC = b"\x01\x28"


def clock(v):
    """`HH:MM:SS[.mmm]` -> seconds since local midnight, matching the phase logs."""
    if v is None:
        return None
    h, m, s = v.split(":")
    return int(h) * 3600 + int(m) * 60 + float(s)


def concat(path, ep, device, since, until):
    reader = usbmon_transfers if path.endswith(".pcapng") else iter_transfers
    kw = {"device": device} if device is not None else {}
    if not path.endswith(".pcapng"):
        kw.update(since=since, until=until)
    out = bytearray()
    for _d, _e, data in reader(path, endpoint=ep, transfer_type=3, out_only=True, **kw):
        out += data
    return bytes(out)


def walk(blob):
    o, n = 0, len(blob)
    while o + 16 <= n:
        size = struct.unpack_from("<H", blob, o + 2)[0]
        typ = struct.unpack_from("<I", blob, o + 4)[0]
        if size < 12 or size > 0x9000 or o + 4 + size > n or typ not in (4, 5, 6):
            o += 4
            continue
        yield (typ,
               struct.unpack_from("<H", blob, o + 8)[0],
               struct.unpack_from("<H", blob, o + 10)[0],
               blob[o + 16:o + 4 + size])
        o += size + 4


def strips_in(body, aux):
    p, end = 0, max(0, len(body) - aux)
    while p + 2 <= end:
        sl = struct.unpack_from("<H", body, p)[0]
        if sl < 16 or p + 2 + sl > end:
            break
        s = body[p + 2:p + 2 + sl]
        if s[:2] == STRIP_MAGIC:
            yield s
        p += 2 + sl


def dc_chain(s, cmax):
    """The strip's sixteen cumulative luma DC values under a candidate ceiling."""
    r = cd.BitReader(s, 16)
    for _ in range(16):
        cd.dec_sync_unit(r)
    out = []
    pcr = pcb = py = 0
    for _ in range(16):
        pcr += cd.dec_esc(r, cmax)
        pcb += cd.dec_esc(r, cmax)
        py += cd.dec_esc(r, cmax)
        out.append(py)
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("capture")
    ap.add_argument("--ep", type=lambda v: int(v, 0), default=0x08)
    ap.add_argument("--sub", type=lambda v: int(v, 0), default=0)
    ap.add_argument("--device", type=int)
    ap.add_argument("--row", type=int, default=720, help="strip y to walk across")
    ap.add_argument("--since", help="HH:MM:SS[.mmm] local, USBPcap captures only")
    ap.add_argument("--until")
    ap.add_argument("--cmax", type=int, nargs="*", default=[10, 11, 12, 13])
    args = ap.parse_args()

    blob = concat(args.capture, args.ep, args.device, clock(args.since), clock(args.until))
    row = {}
    for typ, sub, aux, body in walk(blob):
        if typ != 4 or sub != args.sub:
            continue
        for s in strips_in(body, aux):
            if struct.unpack_from("<H", s, 4)[0] == args.row:
                row[struct.unpack_from("<H", s, 2)[0]] = s
    if not row:
        raise SystemExit(f"no strips at y={args.row} (walked {len(blob):,} bytes)")
    print(f"{len(blob):,} bytes, {len(row)} strips across y={args.row}")

    for cmax in args.cmax:
        seq = []
        for x in sorted(row):
            try:
                seq.extend(dc_chain(row[x], cmax))
            except Exception:
                seq.append(None)
                break
        # Count the inversions. A ramp has none; a desync produces a sign flip and then many.
        drops = sum(1 for a, b in zip(seq, seq[1:])
                    if a is None or b is None or b < a)
        lo = min((v for v in seq if v is not None), default=0)
        hi = max((v for v in seq if v is not None), default=0)
        verdict = "MONOTONIC" if drops == 0 else f"{drops} inversion(s)"
        print(f"  DC cmax {cmax:2d}: luma DC {lo:6d} .. {hi:6d}   {verdict}")

    print("\nA ramp decodes monotonically under exactly one ceiling. The highest category a\n"
          "10-bit DC can reach is 12 (4 x 1023 = 4092), so a ceiling above that is unreachable\n"
          "and would look identical -- 12 and 13 only differ once a category-12 value appears.")


if __name__ == "__main__":
    main()
