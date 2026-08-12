#!/usr/bin/env python3
"""Align two senders' opening record sequences and report the first divergence.

The vendor's bring-up is the specification.  Comparing against it by eye means spot-checking, and
spot-checking is how three confident conclusions got retracted in one session.  This walks both
concatenated record streams from byte zero and lines them up on shape alone -- record class, `sub`,
`aux` and length -- which needs no keys, because a sealed record's shape is as fixed as its
contents.

That is enough to catch a whole class of fault: a record the vendor sends and the driver does not,
one sent in the wrong place, or a per-head block that stops early.  A missing record is invisible
in dmesg and looks like a transport fault on the wire, so this is the cheapest place to find it.

  sequence-diff.py DLM.pcapng vino.pcapng --dev-b 7 [--count 60]

Alignment is positional: the streams are expected to agree record for record from the start, so the
first mismatch is reported with context on both sides rather than resynchronised.  A driver that is
merely *late* with a record still shows up here, as the record the vendor sent in that slot.
"""

from __future__ import annotations

import argparse
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "codec"))
from usbmon_read import iter_transfers  # noqa: E402

SUB_CP = (0x04, 0x24, 0x25, 0x45)
CODEC_SYNC = b"\x01\x28"
# Enough of the opening to cover both per-head blocks and the first frame's prologue.
DEFAULT_COUNT = 60
# The opening is a few kilobytes; reading the whole of a 290 MB capture to compare it is waste.
READ_LIMIT = 1 << 20


def shapes(path: str, device: int | None, endpoint: int, count: int):
    """Yield `(offset, label, body)` for the first `count` records of the OUT stream."""
    stream = bytearray()
    for _dev, _ep, payload in iter_transfers(path, endpoint=endpoint, device=device):
        stream += payload
        if len(stream) >= READ_LIMIT:
            break

    off = n = 0
    while off + 16 <= len(stream) and n < count:
        size = int.from_bytes(stream[off + 2:off + 4], "little")
        stride = size + 4
        if stride < 16 or stride % 16 or off + stride > len(stream):
            return
        sub = int.from_bytes(stream[off + 8:off + 10], "little")
        aux = int.from_bytes(stream[off + 10:off + 12], "little")
        body = bytes(stream[off + 16:off + stride])
        if sub in SUB_CP:
            label = f"CP    sub=0x{sub:02x} aux=0x{aux:04x} len={size}"
        elif len(body) >= 4 and body[2:4] == CODEC_SYNC:
            label = f"IMAGE sub=0x{sub:04x} len={size}"
        else:
            label = f"VIDEO sub=0x{sub:04x} aux=0x{aux:04x} len={size}"
        yield off, label, body
        off += stride
        n += 1


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("reference", help="a vendor capture")
    ap.add_argument("compare", help="a driver capture of the same bring-up")
    ap.add_argument("--endpoint", type=int, default=2)
    ap.add_argument("--dev-a", type=int, help="usbmon device number of the reference capture")
    ap.add_argument("--dev-b", type=int, help="usbmon device number of the compared capture")
    ap.add_argument("--count", type=int, default=DEFAULT_COUNT)
    args = ap.parse_args()

    a = list(shapes(args.reference, args.dev_a, args.endpoint, args.count))
    b = list(shapes(args.compare, args.dev_b, args.endpoint, args.count))

    first_bad = None
    for i in range(max(len(a), len(b))):
        ref = a[i] if i < len(a) else None
        got = b[i] if i < len(b) else None
        mark = " "
        if ref is None or got is None or ref[1] != got[1]:
            mark = "*"
            if first_bad is None:
                first_bad = i
        left = f"@{ref[0]:<7} {ref[1]}" if ref else "-"
        right = f"@{got[0]:<7} {got[1]}" if got else "-"
        print(f"{mark}#{i:<4} {left:<46} | {right}")
        # Two records of context past the divergence is enough to say what was skipped; beyond
        # that the streams have shifted and every later line is noise.
        if first_bad is not None and i >= first_bad + 2:
            break

    if first_bad is None:
        print("\nsequences agree over the compared records")
        return 0
    print(f"\nfirst divergence at record #{first_bad}")
    if first_bad < len(a):
        print(f"  reference sends: {a[first_bad][1]}  body {a[first_bad][2][:16].hex()}")
    if first_bad < len(b):
        print(f"  compared  sends: {b[first_bad][1]}  body {b[first_bad][2][:16].hex()}")
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
