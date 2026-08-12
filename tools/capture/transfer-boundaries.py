#!/usr/bin/env python3
"""Where does the vendor cut its USB transfers, relative to record boundaries?

On a dock that carries video on its control pipe there is one ordered byte stream and two
things writing into it.  Whether a control record can be handed to the endpoint while a video
record is only half written is decided here, by measuring what the vendor actually does:

  * the transfer-length histogram, which says whether a short packet mid-frame is allowed at
    all; and
  * for every record, whether it starts at a transfer boundary and whether it spans one,
    split by control versus video.

If control records always start a transfer and never sit inside one, the two writers are
serialised at record granularity and a driver that splits at a fixed size is wrong.

Usage:
    transfer-boundaries.py CAP.pcapng [--endpoint 2] [--limit N]
"""

from __future__ import annotations

import argparse
import bisect
import sys
from collections import Counter
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "codec"))
from usbmon_read import iter_transfers  # noqa: E402

SUB_CP_PLAIN = 0x04
SUB_CP_SEALED = 0x24


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("cap")
    ap.add_argument("--endpoint", type=int, default=2)
    ap.add_argument("--limit", type=int, default=0, help="stop after N transfers")
    args = ap.parse_args()

    # Byte offset of every transfer boundary in the concatenated stream.
    bounds: list[int] = [0]
    stream = bytearray()
    lens: Counter[int] = Counter()
    for _dev, _ep, payload in iter_transfers(args.cap, endpoint=args.endpoint):
        if not payload:
            continue
        stream += payload
        bounds.append(len(stream))
        lens[len(payload)] += 1
        if args.limit and len(bounds) > args.limit:
            break
    bset = set(bounds)

    print(f"transfers      : {len(bounds) - 1}")
    print(f"stream bytes   : {len(stream)}")
    print("\ntransfer lengths, most common:")
    for n, c in lens.most_common(12):
        print(f"  {n:>8} B  x{c}   {'(multiple of 1024)' if n % 1024 == 0 else '<-- SHORT PACKET'}")
    short = sum(c for n, c in lens.items() if n % 1024)
    print(f"  transfers ending in a short packet: {short} / {sum(lens.values())}")

    # Walk the records.
    off = 0
    stats = {"cp": Counter(), "video": Counter()}
    n_rec = 0
    while off + 16 <= len(stream):
        size = int.from_bytes(stream[off + 2:off + 4], "little")
        stride = size + 4
        if stride < 16 or stride % 16 or off + stride > len(stream):
            print(f"\nresync/stop at offset {off} (stride {stride})")
            break
        sub = int.from_bytes(stream[off + 8:off + 10], "little")
        kind = "cp" if sub in (SUB_CP_PLAIN, SUB_CP_SEALED) else "video"
        s = stats[kind]
        s["records"] += 1
        if off in bset:
            s["starts a transfer"] += 1
        # Does this record cross a boundary?
        i = bisect.bisect_right(bounds, off)
        if i < len(bounds) and bounds[i] < off + stride:
            s["spans a boundary"] += 1
        off += stride
        n_rec += 1

    print(f"\nrecords parsed : {n_rec}, bytes consumed {off}/{len(stream)}")
    for kind in ("cp", "video"):
        s = stats[kind]
        tot = s["records"] or 1
        print(f"\n{kind} records: {s['records']}")
        print(f"  start exactly at a transfer boundary : {s['starts a transfer']} "
              f"({100 * s['starts a transfer'] / tot:.1f}%)")
        print(f"  span a transfer boundary             : {s['spans a boundary']} "
              f"({100 * s['spans a boundary'] / tot:.1f}%)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
