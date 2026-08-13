#!/usr/bin/env python3
"""Census a capture's video frames: how many, how big, and whether they repeat.

Answers the question a byte total cannot: when a dock stops accepting a stream, was it metering
bytes or frames?  Walking the OUT record stream and reporting the cumulative totals in every
candidate unit at each frame opener settles it -- two runs that halt at the same value in one
column and different values in the others name the unit.

It also hashes each frame's concatenated image payload, which separates the two very different
bugs that both look like "the frames are too big":

  * a compositor genuinely redrawing everything, which gives a different digest every frame; and
  * one keyframe presented `dock_buffers` times and then re-raised, which gives the same digest
    over and over.  A run whose frames all share a digest has sent the dock a single image
    several times, and the fix is upstream of the codec entirely.

Needs no keys: frame openers and image records are plaintext, and the record stream is walked by
its own `size` fields.

  frame-census.py CAP.pcapng [CAP2.pcapng ...] [--endpoint 2]
"""

from __future__ import annotations

import argparse
import hashlib
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "codec"))
from usbmon_read import iter_transfers  # noqa: E402

SUB_CP = (0x04, 0x24, 0x25, 0x45)
CODEC_SYNC = b"\x01\x28"
# `aux` is a record subtype on the docks that carry video on the control pipe. An image record
# stores its zero-padding count in the same field, so the opener needs its size and body magic
# checked too: padding of ten bytes is otherwise indistinguishable from a frame opener.
AUX_FRAME_OPENER = 0x000A
OPENER_SIZE = 44
OPENER_MAGIC = b"\x08\x00\x05"


def records(path: str, endpoint: int):
    """Yield whole records from the concatenated OUT stream of one endpoint."""
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
            rec = bytes(buf[:stride])
            del buf[:stride]
            yield rec


def census(path: str, endpoint: int) -> None:
    off = img_bytes = img_payload = img_count = 0
    per: dict[int, list] = {}      # sub -> [frames, payload, imgrecs]
    cur: dict[int, list] = {}      # sub -> [frame_no, hasher, nrecs, nbytes]
    digests: dict[int, list] = {}
    rows = []

    def close(sub):
        if sub in cur and cur[sub][2]:
            n, h, recs, byts = cur[sub]
            digests.setdefault(sub, []).append((n, h.hexdigest()[:16], recs, byts))

    for rec in records(path, endpoint):
        size = int.from_bytes(rec[2:4], "little")
        sub = int.from_bytes(rec[8:10], "little")
        aux = int.from_bytes(rec[10:12], "little")
        body = rec[16:]
        image = len(body) >= 4 and body[2:4] == CODEC_SYNC
        opener = (sub not in SUB_CP and not image and aux == AUX_FRAME_OPENER
                  and size == OPENER_SIZE and body[:3] == OPENER_MAGIC)
        if opener:
            st = per.setdefault(sub, [0, 0, 0])
            st[0] += 1
            rows.append((st[0], sub, body[3], body[9], off, img_payload, img_count,
                         st[1], st[2]))
            close(sub)
            cur[sub] = [st[0], hashlib.sha256(), 0, 0]
        if image:
            img_bytes += len(rec)
            img_payload += len(body)
            img_count += 1
            st = per.setdefault(sub, [0, 0, 0])
            st[1] += len(body)
            st[2] += 1
            if sub in cur:
                cur[sub][1].update(body)
                cur[sub][2] += 1
                cur[sub][3] += len(body)
        off += len(rec)
    for sub in list(cur):
        close(sub)

    mib16 = 16 * 1024 * 1024
    print(f"\n===== {Path(path).name} =====")
    print(f"stream={off:,}  image={img_bytes:,}  payload={img_payload:,}  "
          f"image-records={img_count:,}")
    print("\nframe openers, with the totals as of each one:")
    print(f"{'#':>4} {'sub':>4} {'slot':>4} {'cnt':>4} {'stream off':>13} "
          f"{'payload':>13} {'imgrecs':>8} {'sub payload':>13} {'sub recs':>9} {'/16MiB':>7}")
    for n, sub, slot, cnt, o, pay, recs, spay, srecs in rows:
        print(f"{n:>4} {sub:>4} {slot:>4} {cnt:>4} {o:>13,} {pay:>13,} {recs:>8,} "
              f"{spay:>13,} {srecs:>9,} {pay / mib16:>7.3f}")

    print("\nper-frame image payload digests (a repeated digest is a re-sent image):")
    for sub in sorted(digests):
        if len(digests[sub]) < 2:
            continue
        print(f"-- sub {sub} --")
        seen: dict[str, int] = {}
        for n, d, recs, byts in digests[sub]:
            first = seen.setdefault(d, n)
            note = "" if first == n else f"  == frame {first}"
            print(f"   frame {n:>3}  recs={recs:>4}  bytes={byts:>10,}  {d}{note}")


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("captures", nargs="+")
    ap.add_argument("--endpoint", type=int, default=2, help="OUT endpoint number (default 2)")
    args = ap.parse_args()
    for path in args.captures:
        census(path, args.endpoint)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
