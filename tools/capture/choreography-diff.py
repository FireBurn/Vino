#!/usr/bin/env python3
"""Enumerate EVERY ordering difference between two senders' record streams, not the first.

`sequence-diff.py` aligns positionally and stops at the first mismatch, which is the right tool
for "is the driver speaking the same language" and the wrong one for "what is it still missing".
A single missing record shifts every later position, so one omission reads as total divergence and
hides the nine behind it. Fixing them one hardware cycle at a time is what that costs.

This aligns the two streams with a real sequence matcher and prints every insertion, deletion and
substitution, so one pass over one capture yields the whole list.

Records are compared on shape -- class, `sub`, `aux`, length -- which needs no keys, because a
sealed record's shape is as fixed as its contents. Runs of image records collapse to one token
carrying the run length, so a frame is a single event and a difference in how many frames separate
two control records is visible as a substitution rather than as thousands of unaligned lines.

  choreography-diff.py VENDOR.pcapng DRIVER.pcapng [--count 400] [--context 3]
"""

from __future__ import annotations

import argparse
import difflib
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "codec"))
from usbmon_read import iter_transfers  # noqa: E402

CODEC_SYNC = b"\x01\x28"
SUB_CP = (0x04, 0x24, 0x25, 0x45)


def records(path: str, endpoint: int):
    """Yield whole records from the concatenated OUT stream, as `record-stream.py` does."""
    buf = bytearray()
    for _dev, _ep, payload in iter_transfers(path, endpoint=endpoint, transfer_type=3):
        buf += payload
        while len(buf) >= 16:
            stride = int.from_bytes(buf[2:4], "little") + 4
            if stride < 16 or stride % 16:
                del buf[:1]
                continue
            if len(buf) < stride:
                break
            record = bytes(buf[:stride])
            del buf[:stride]
            yield record


def token(record: bytes) -> str:
    """The comparable shape of one record."""
    typ = int.from_bytes(record[4:8], "little")
    sub = int.from_bytes(record[8:10], "little")
    aux = int.from_bytes(record[10:12], "little")
    body = record[16:]
    if len(body) >= 4 and body[2:4] == CODEC_SYNC:
        return "IMAGE"
    if sub in SUB_CP:
        return f"CP sub={sub:#04x} aux={aux:#06x} len={len(record)}"
    return f"VIDEO sub={sub:#06x} aux={aux:#06x} len={len(record)}"


def sequence(path: str, endpoint: int, count: int) -> list[str]:
    """Shape tokens with image runs collapsed, stopping after `count` of them."""
    out: list[str] = []
    run = 0
    for record in records(path, endpoint):
        tok = token(record)
        if tok == "IMAGE":
            run += 1
            continue
        if run:
            out.append(f"FRAME {run} records")
            run = 0
        out.append(tok)
        if len(out) >= count:
            return out
    if run:
        out.append(f"FRAME {run} records")
    return out


def main() -> int:
    ap = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    ap.add_argument("vendor")
    ap.add_argument("driver")
    ap.add_argument("--endpoint", type=int, default=2)
    ap.add_argument("--count", type=int, default=400, help="tokens to compare from each")
    ap.add_argument("--context", type=int, default=2, help="matching tokens shown around a block")
    args = ap.parse_args()

    a = sequence(args.vendor, args.endpoint, args.count)
    b = sequence(args.driver, args.endpoint, args.count)
    print(f"vendor {len(a)} tokens, driver {len(b)} tokens\n")

    matcher = difflib.SequenceMatcher(a=a, b=b, autojunk=False)
    blocks = [op for op in matcher.get_opcodes() if op[0] != "equal"]
    if not blocks:
        print("no differences")
        return 0

    print(f"{len(blocks)} divergence(s)\n")
    for n, (kind, i1, i2, j1, j2) in enumerate(blocks, 1):
        print(f"--- #{n}: {kind} at vendor[{i1}:{i2}] driver[{j1}:{j2}]")
        for k in range(max(0, i1 - args.context), i1):
            print(f"      {a[k]}")
        for k in range(i1, i2):
            print(f"  vendor only:  {a[k]}")
        for k in range(j1, j2):
            print(f"  driver only:  {b[k]}")
        for k in range(i2, min(len(a), i2 + args.context)):
            print(f"      {a[k]}")
        print()
    ratio = matcher.ratio()
    print(f"overall shape similarity {ratio * 100:.1f}%")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
