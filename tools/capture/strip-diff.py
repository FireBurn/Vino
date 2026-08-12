#!/usr/bin/env python3
"""Diff the coded strip bytes two senders produce for the same picture.

The record stream can be byte-for-byte right and the picture still never appear, because the
strip payload inside the records is a separate grammar with its own tables. This pulls the OUT
record stream out of a usbmon capture, walks it, and reports the distinct coded strip payloads.
Point it at a DLM capture and a vino capture of the same content and the answer is one line:
either the payloads match, or the encoder is speaking a dialect the dock does not read.

A flat frame is the sharpest probe available -- every strip encodes identically, so a whole frame
collapses to one payload and any difference is unambiguous.

  ./strip-diff.py dlm.pcapng --at 16:39:12 --frames 15
  ./strip-diff.py dlm.pcapng vino.pcapng          # compare two captures

Records are `[u16 0][u16 size][u32 type][u16 sub][u16 aux][u32 seq][body]`, stride `size+4`
rounded up to 16. Image records carry `0x2801` at body offset 2 and pack strips as
`[u16 len][strip]`; the strip's own 16-byte header holds x, y and the two AC row offsets.
"""

import argparse
import collections
import datetime
import struct
import sys

STRIDE_CAP = 0x1000
STRIP_MAGIC = 0x2801


def pcapng_out_stream(path, t0=None, t1=None, endpoint=0x02):
    """Concatenate the payloads of every bulk-OUT submission, in order.

    The dock parses one continuous record stream per endpoint, so the transfer boundaries carry
    no meaning and must be joined before anything is walked.
    """
    endian = "<"
    tsresol = {}
    iface = 0
    chunks = []
    with open(path, "rb") as f:
        while True:
            head = f.read(8)
            if len(head) < 8:
                break
            btype, blen = struct.unpack(endian + "II", head)
            if btype == 0x0A0D0D0A:
                body = f.read(blen - 8)
                if len(body) < blen - 8:
                    break
                magic = struct.unpack("<I", body[0:4])[0]
                endian = "<" if magic == 0x1A2B3C4D else ">"
                iface = 0
                continue
            if blen < 12 or blen > 100_000_000:
                break
            body = f.read(blen - 8)
            if len(body) < blen - 8:
                break
            if btype == 1:  # interface description
                res = _option(body[8:-4], 9, endian, b"\x06")[0]
                tsresol[iface] = 10**res if not res & 0x80 else 2 ** (res & 0x7F)
                iface += 1
            elif btype == 6:  # enhanced packet
                idx, hi, lo, caplen, _ = struct.unpack(endian + "IIIII", body[0:20])
                ts = ((hi << 32) | lo) / tsresol.get(idx, 1_000_000)
                if (t0 is not None and ts < t0) or (t1 is not None and ts > t1):
                    continue
                pkt = body[20 : 20 + caplen]
                if len(pkt) < 64 or chr(pkt[8]) != "S" or pkt[10] != endpoint:
                    continue
                dlen = struct.unpack(endian + "I", pkt[36:40])[0]
                if dlen:
                    chunks.append(pkt[64 : 64 + dlen])
    return b"".join(chunks)


def _option(buf, want, endian, default):
    i = 0
    while i + 4 <= len(buf):
        code, ln = struct.unpack(endian + "HH", buf[i : i + 4])
        val = buf[i + 4 : i + 4 + ln]
        i += 4 + ((ln + 3) & ~3)
        if code == 0:
            break
        if code == want:
            return val
    return default


def strips(stream, limit_records=None):
    """Yield (x, y, coded) for every strip in the stream's image records."""
    off = 0
    seen = 0
    while off + 16 <= len(stream):
        zero, size = struct.unpack("<HH", stream[off : off + 4])
        stride = ((size + 4) + 15) & ~15
        if zero != 0 or size < 12 or stride > STRIDE_CAP:
            print(f"desync at offset {off}: {stream[off:off+16].hex()}", file=sys.stderr)
            return
        if size >= 28 and struct.unpack("<H", stream[off + 18 : off + 20])[0] == STRIP_MAGIC:
            seen += 1
            if limit_records is not None and seen > limit_records:
                return
            pos, end = off + 16, off + 4 + size
            while pos + 2 <= end:
                slen = struct.unpack("<H", stream[pos : pos + 2])[0]
                if slen == 0 or pos + 2 + slen > end:
                    break
                body = stream[pos + 2 : pos + 2 + slen]
                x, y = struct.unpack("<HH", body[2:6])
                yield x, y, body[16:]
                pos += 2 + slen
        off += stride


def census(path, args):
    stream = pcapng_out_stream(path, args.t0, args.t1, args.endpoint)
    payloads = collections.Counter()
    total = 0
    for _x, _y, coded in strips(stream, args.frames):
        payloads[coded] += 1
        total += 1
    print(f"{path}: {len(stream)} OUT bytes, {total} strips, {len(payloads)} distinct payloads")
    for coded, count in payloads.most_common(args.show):
        print(f"   x{count:<6d} {coded.hex()}")
    return payloads


def _timestamp(value):
    """Accept a bare time for today's capture, or a full date for an older one."""
    for fmt, dated in (
        ("%Y-%m-%d %H:%M:%S", True),
        ("%Y-%m-%d %H:%M:%S.%f", True),
        ("%H:%M:%S", False),
        ("%H:%M:%S.%f", False),
    ):
        try:
            parsed = datetime.datetime.strptime(value, fmt)
        except ValueError:
            continue
        if dated:
            return parsed.timestamp()
        return datetime.datetime.combine(datetime.date.today(), parsed.time()).timestamp()
    raise SystemExit(f"unparseable time: {value!r}")


def main():
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("capture", nargs="+", help="usbmon pcapng file(s)")
    ap.add_argument("--endpoint", type=lambda s: int(s, 0), default=0x02,
                    help="bulk OUT endpoint carrying the record stream (default 0x02)")
    ap.add_argument("--frames", type=int, default=None,
                    help="stop after this many image records (one flat frame is enough)")
    ap.add_argument("--show", type=int, default=3, help="distinct payloads to print")
    ap.add_argument("--from", dest="since", help="start time, 'HH:MM:SS' or 'YYYY-MM-DD HH:MM:SS'")
    ap.add_argument("--until", help="end time, same forms as --from")
    args = ap.parse_args()

    args.t0 = args.t1 = None
    for name, value in (("t0", args.since), ("t1", args.until)):
        if value:
            setattr(args, name, _timestamp(value))

    censuses = [census(path, args) for path in args.capture]
    if len(censuses) == 2:
        left, right = (set(c) for c in censuses)
        shared = left & right
        print(f"\n{len(shared)} payload(s) in common, "
              f"{len(left - right)} only in the first, {len(right - left)} only in the second")
        if not shared:
            print("no coded strip is shared: the two encoders do not agree on the bitstream")


if __name__ == "__main__":
    main()
