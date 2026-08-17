#!/usr/bin/env python3
"""A record walk that resyncs correctly, and reports how much it had to resync.

Records are 16-byte aligned and contiguous, so a walk that starts aligned should never drift. It
does drift, because the stream carries parts that are not records in this layout (the arm block and
the parameter block), and a byte-at-a-time resync then lands mid-payload and invents records with
impossible `sub` values at unaligned offsets.

This walks 16 bytes at a time and accepts a header only if it is self-consistent, so every resync
is a counted, bounded skip rather than silent corruption. Anything it reports as skipped is a real
"this is not a record" region, which is itself the interesting measurement.

    walk2.py CAP.pcapng [--endpoint 2]
"""
import argparse
import sys
from collections import Counter
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "codec"))
from usbmon_read import iter_transfers  # noqa: E402

CODEC_SYNC = b"\x01\x28"
SUB_CP = (0x04, 0x24, 0x25, 0x45)
STRIDE_MAX = 4096


def load(path, endpoint):
    return b"".join(p for _d, _e, p in
                    iter_transfers(path, endpoint=endpoint, transfer_type=3))


def plausible(buf, off):
    """-> stride if a record header at `off` is self-consistent, else None."""
    if off + 16 > len(buf):
        return None
    size = int.from_bytes(buf[off + 2:off + 4], "little")
    stride = size + 4
    if stride < 16 or stride % 16 or stride > STRIDE_MAX or off + stride > len(buf):
        return None
    sub = int.from_bytes(buf[off + 8:off + 10], "little")
    aux = int.from_bytes(buf[off + 10:off + 12], "little")
    body = buf[off + 16:off + 20]
    # A record is either an image record (codec sync in the body), a known control sub, or the
    # 44-byte frame marker. Anything else at this offset means we are not on a header.
    if len(body) >= 4 and body[2:4] == CODEC_SYNC:
        return stride
    if sub in SUB_CP:
        return stride
    if size == 44 and aux == 0x000A and buf[off + 16:off + 19] == b"\x08\x00\x05":
        return stride
    return None


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("cap")
    ap.add_argument("--endpoint", type=int, default=2)
    a = ap.parse_args()

    buf = load(a.cap, a.endpoint)
    print(f"stream {len(buf):,} bytes\n")

    off = 0
    recs = 0
    skipped = 0
    skips = []          # (start, length)
    markers = Counter()
    images = Counter()
    runs = []
    cur_skip = None

    while off + 16 <= len(buf):
        stride = plausible(buf, off)
        if stride is None:
            if cur_skip is None:
                cur_skip = off
            off += 16
            skipped += 16
            continue
        if cur_skip is not None:
            skips.append((cur_skip, off - cur_skip))
            cur_skip = None
        size = int.from_bytes(buf[off + 2:off + 4], "little")
        sub = int.from_bytes(buf[off + 8:off + 10], "little")
        aux = int.from_bytes(buf[off + 10:off + 12], "little")
        body = buf[off + 16:off + 20]
        if len(body) >= 4 and body[2:4] == CODEC_SYNC:
            images[sub] += 1
            head = sub & 0x0F
            if runs and runs[-1][0] == head:
                runs[-1][1] += 1
            else:
                runs.append([head, 1])
        elif size == 44 and aux == 0x000A:
            markers[sub] += 1
        recs += 1
        off += stride
    if cur_skip is not None:
        skips.append((cur_skip, off - cur_skip))

    print(f"records parsed:  {recs:,}")
    print(f"bytes skipped:   {skipped:,} ({100*skipped/len(buf):.3f}% of the stream)")
    print(f"skip regions:    {len(skips)}")
    print(f"\nframe markers per sub: {dict(markers)}")
    print(f"image records per sub: {dict(images)}")

    print(f"\nvideo run lengths (records before the other head appears): {len(runs)} runs")
    short = [r for r in runs if r[1] < 50]
    print(f"  runs under 50 records: {len(short)} of {len(runs)} "
          f"({100*len(short)/max(1,len(runs)):.1f}%)")
    print(f"  min {min(r[1] for r in runs)}  max {max(r[1] for r in runs)}")

    print("\nlargest skip regions (start, bytes):")
    for start, n in sorted(skips, key=lambda s: -s[1])[:8]:
        print(f"  {start:>12,}  {n:>8,} B")


main()
