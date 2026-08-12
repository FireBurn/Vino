#!/usr/bin/env python3
"""Census the frame openers in a capture's video record stream, and diff two senders.

The frame opener is the record that tells the dock which ring buffer a frame landed in.  Getting
its bytes right is not enough: the *counter* handed to the builder has to start where the vendor's
starts, and a stream that begins one slot ahead tells the dock to scan out a buffer the host never
wrote.  That failure looks exactly like a transport fault -- the dock simply stops consuming and
stalls the endpoint some time later -- so it has to be read off the wire rather than inferred.

Needs no keys: openers are plaintext, and the record stream is walked by its own `size` fields.

  ring-openers.py DLM.pcapng                       census one capture
  ring-openers.py DLM.pcapng vino.pcapng --dev-b 7 diff the two walks

A healthy DL-3x00 stream walks `(cur, count, next)` = (0,1,1) (1,2,2) (2,3,0) (0,4,1) ... on every
head, starting at slot zero with a one-based counter.
"""

from __future__ import annotations

import argparse
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "codec"))
from usbmon_read import iter_transfers  # noqa: E402

SUB_CP = (0x04, 0x24, 0x25, 0x45)
CODEC_SYNC = b"\x01\x28"
# `aux` is a record subtype on the docks that carry video on the control pipe: this one starts a
# frame, 0x0008 opens a stream.
AUX_FRAME_OPENER = 0x000A
OPENER_SIZE = 44
# The opener's body names its own subtype.  A sealed record on a stream sub can land on the same
# size and `aux` by chance, and its ciphertext then decodes as a plausible ring walk.
OPENER_MAGIC = b"\x08\x00\x05"


def stream(path: str, device: int | None, endpoint: int) -> bytes:
    """The endpoint's OUT payloads concatenated, which is the stream the dock parses."""
    out = bytearray()
    for _dev, _ep, payload in iter_transfers(path, endpoint=endpoint, device=device):
        out += payload
    return bytes(out)


def openers(buf: bytes):
    """Yield `(offset, sub, cur, count, next)` for every frame opener, in wire order."""
    off = 0
    while off + 16 <= len(buf):
        size = int.from_bytes(buf[off + 2:off + 4], "little")
        stride = size + 4
        if stride < 16 or stride % 16 or off + stride > len(buf):
            return
        sub = int.from_bytes(buf[off + 8:off + 10], "little")
        aux = int.from_bytes(buf[off + 10:off + 12], "little")
        body = buf[off + 16:off + stride]
        image = len(body) >= 4 and body[2:4] == CODEC_SYNC
        opener = (aux == AUX_FRAME_OPENER and size == OPENER_SIZE
                  and body[:3] == OPENER_MAGIC)
        if sub not in SUB_CP and not image and opener:
            yield off, sub, body[3], body[9], body[13]
        off += stride


def census(path: str, device: int | None, endpoint: int):
    buf = stream(path, device, endpoint)
    by_sub: dict[int, list] = {}
    for off, sub, cur, count, nxt in openers(buf):
        by_sub.setdefault(sub, []).append((off, cur, count, nxt))
    print(f"{path}: {len(buf)} bytes")
    for sub, seq in sorted(by_sub.items()):
        walk = " ".join(f"({c},{n},{x})" for _o, c, n, x in seq[:8])
        print(f"  sub 0x{sub:04x}: {len(seq)} openers   {walk}{' ...' if len(seq) > 8 else ''}")
    return by_sub


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("reference")
    ap.add_argument("compare", nargs="?")
    ap.add_argument("--endpoint", type=int, default=2)
    ap.add_argument("--dev-a", type=int, help="usbmon device number of the reference capture")
    ap.add_argument("--dev-b", type=int, help="usbmon device number of the compared capture")
    args = ap.parse_args()

    a = census(args.reference, args.dev_a, args.endpoint)
    if not args.compare:
        return 0
    b = census(args.compare, args.dev_b, args.endpoint)

    # A head's own sub differs between docks and between heads, so compare the walks rather than
    # the subs: what matters is where each stream's counter starts and how it steps.
    print()
    ref = [(c, n, x) for sub in sorted(a) for _o, c, n, x in a[sub][:6]][:6]
    got = [(c, n, x) for sub in sorted(b) for _o, c, n, x in b[sub][:6]][:6]
    print(f"  reference first 6: {ref}")
    print(f"  compared  first 6: {got}")
    if ref and got and ref[0] != got[0]:
        lag = next((i for i, v in enumerate(ref) if v == got[0]), None)
        detail = f" -- compared starts {lag} opener(s) ahead" if lag else ""
        print(f"  MISMATCH at the first opener{detail}")
        return 1
    print("  walks agree at the first opener")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
