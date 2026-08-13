#!/usr/bin/env python3
"""Describe what a sender does on a DisplayLink dock's OUT endpoint, in order and in time.

The other tools answer one question each: `sequence-diff.py` finds the first record the driver
gets wrong, `stall-point.py` finds the transfer a dock refused.  This one builds the whole
picture the vendor's driver presents to the dock -- the record stream, the frames it groups into,
the USB transfers it is chopped into, and when each went out -- so a driver can be checked against
the vendor's choreography rather than against a guess about it.

Three views, each answering a question the wire alone can settle:

  --frames      how large is each frame, how many records, how far apart in time, and which
                non-image records ride with it
  --transfers   where the sender puts its USB transfer boundaries relative to frame boundaries
  --records     the record stream itself, images collapsed

The dock is a state machine driven by this stream; a record in the wrong frame or a boundary in
the wrong place is as real a fault as a wrong byte, and neither is visible in dmesg.

  choreography.py CAP.pcapng [--dev N] [--frames] [--limit BYTES] [--head H]
"""

from __future__ import annotations

import argparse
import struct
import sys

DLT_USB_LINUX = 189
DLT_USB_LINUX_MMAPPED = 220
USBMON_HDR = struct.Struct("<QBBBBHccqiiII")

SUB_CP = (0x04, 0x24, 0x25, 0x45)
CODEC_SYNC = b"\x01\x28"
# A record that opens a frame: 48 bytes on a video sub, aux naming the subtype.
AUX_STREAM_OPEN = 0x0008
AUX_FRAME_OPENER = 0x000A


def blocks(fh):
    while True:
        head = fh.read(8)
        if len(head) < 8:
            return
        btype, blen = struct.unpack("<II", head)
        if blen < 12:
            return
        body = fh.read(blen - 12)
        fh.read(4)
        yield btype, body


def events(path, device=None, endpoint=2):
    """Yield `(ts, kind, status, payload)` for one endpoint, submissions and completions alike."""
    linktypes = []
    with open(path, "rb") as fh:
        for btype, body in blocks(fh):
            if btype == 0x0A0D0D0A:
                linktypes = []
                continue
            if btype == 0x00000001:
                linktypes.append(struct.unpack_from("<H", body, 0)[0])
                continue
            if btype == 0x00000006:
                iface, _hi, _lo, caplen, _origlen = struct.unpack_from("<IIIII", body, 0)
                if iface < len(linktypes) and linktypes[iface] not in (
                    DLT_USB_LINUX, DLT_USB_LINUX_MMAPPED
                ):
                    continue
                pkt = body[20:20 + caplen]
            elif btype == 0x00000003:
                pkt = body[4:]
            else:
                continue
            if len(pkt) < USBMON_HDR.size:
                continue
            (_uid, utype, xfer, epnum, devnum, _bus, _fs, _fd,
             ts_sec, ts_us, status, _length, len_cap) = USBMON_HDR.unpack_from(pkt, 0)
            # `endpoint` carries its direction bit, so 2 is bulk-OUT 2 and 0x84 is bulk-IN 4.
            if epnum != endpoint:
                continue
            if device is not None and devnum != device:
                continue
            # mmapped captures add an ISO/interval descriptor before the data.
            base = 64 if len(pkt) >= 64 + len_cap else USBMON_HDR.size
            payload = pkt[base:base + len_cap]
            yield ts_sec + ts_us / 1e6, chr(utype), status, payload


def transfers(path, device, endpoint, limit):
    """Yield `(index, offset, length, ts)` for each OUT submission, in stream order."""
    off = idx = 0
    for ts, kind, _status, payload in events(path, device, endpoint):
        if kind != "S" or not payload:
            continue
        yield idx, off, len(payload), ts, payload
        off += len(payload)
        idx += 1
        if off >= limit:
            return


class Record:
    __slots__ = ("off", "size", "sub", "aux", "seq", "image", "body", "xfer", "ts")

    def __init__(self, off, size, sub, aux, seq, image, body, xfer, ts):
        self.off, self.size, self.sub, self.aux = off, size, sub, aux
        self.seq, self.image, self.body = seq, image, body
        self.xfer, self.ts = xfer, ts

    @property
    def kind(self):
        if self.image:
            return "IMAGE"
        return "CP" if self.sub in SUB_CP else "VIDEO"

    def label(self):
        return (f"{self.kind:5s} sub=0x{self.sub:04x} aux=0x{self.aux:04x} "
                f"len={self.size} seq={self.seq}")


def parse(path, device, endpoint, limit):
    """Parse the concatenated OUT stream into records, tagging each with its transfer."""
    stream = bytearray()
    starts = []  # (offset, ts) per transfer
    for _idx, off, length, ts, payload in transfers(path, device, endpoint, limit):
        starts.append((off, ts, length))
        stream += payload

    recs = []
    off = 0
    ti = 0
    while off + 16 <= len(stream):
        size = int.from_bytes(stream[off + 2:off + 4], "little")
        stride = size + 4
        if stride < 16 or stride % 16 or off + stride > len(stream):
            break
        sub = int.from_bytes(stream[off + 8:off + 10], "little")
        aux = int.from_bytes(stream[off + 10:off + 12], "little")
        seq = int.from_bytes(stream[off + 12:off + 16], "little")
        body = bytes(stream[off + 16:off + stride])
        image = sub not in SUB_CP and len(body) >= 4 and body[2:4] == CODEC_SYNC
        while ti + 1 < len(starts) and starts[ti + 1][0] <= off:
            ti += 1
        recs.append(Record(off, size, sub, aux, seq, image, body, ti, starts[ti][1]))
        off += stride
    return recs, starts, len(stream)


def is_opener(rec):
    return not rec.image and rec.sub not in SUB_CP and rec.aux == AUX_FRAME_OPENER


def is_stream_open(rec):
    return not rec.image and rec.sub not in SUB_CP and rec.aux == AUX_STREAM_OPEN


def frames(recs):
    """Group records into frames.

    A frame starts at a stream-open or frame-opener record and runs to the next one on the same
    head, so the records that ride with a frame -- reports, control traffic -- are attributed to
    it.  Image records carry the head in `sub`; the opener carries it there too.
    """
    out = []
    cur = None
    for rec in recs:
        if is_opener(rec) or is_stream_open(rec):
            if cur:
                out.append(cur)
            cur = {"head": rec.sub & 0x0F, "start": rec.off, "recs": [rec],
                   "images": 0, "bytes": 0, "ts0": rec.ts, "opener": rec}
            continue
        if cur is None:
            continue
        cur["recs"].append(rec)
        if rec.image:
            cur["images"] += 1
            cur["bytes"] += rec.size + 4
    if cur:
        out.append(cur)
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("capture")
    ap.add_argument("--dev", type=int, default=None)
    ap.add_argument("--endpoint", type=int, default=2)
    ap.add_argument("--limit", type=int, default=8 << 20, help="bytes of stream to read")
    ap.add_argument("--frames", action="store_true")
    ap.add_argument("--transfers", action="store_true")
    ap.add_argument("--records", action="store_true")
    ap.add_argument("--count", type=int, default=40)
    args = ap.parse_args()

    recs, starts, total = parse(args.capture, args.dev, args.endpoint, args.limit)
    print(f"{args.capture}: {total} bytes, {len(recs)} records, {len(starts)} transfers")

    if args.records or not (args.frames or args.transfers):
        run = 0
        run_bytes = 0
        shown = 0
        for rec in recs:
            if rec.image:
                run += 1
                run_bytes += rec.size + 4
                continue
            if run:
                print(f"      [{run} image records, {run_bytes} B]")
                run = run_bytes = 0
            print(f"  @{rec.off:<9} x{rec.xfer:<5} {rec.label()} {rec.body[:24].hex()}")
            shown += 1
            if args.count and shown >= args.count:
                break
        if run:
            print(f"      [{run} image records, {run_bytes} B]")

    if args.frames:
        print("\n== frames (a frame is opener .. next opener)")
        print("  idx head    bytes  imgs  riders                        gap_ms  first_ts")
        prev = {}
        for i, fr in enumerate(frames(recs)[:args.count or None]):
            riders = [r for r in fr["recs"][1:] if not r.image]
            rid = ",".join(f"{r.kind[0]}{r.sub:02x}/{r.aux:04x}" for r in riders[:6])
            gap = (fr["ts0"] - prev[fr["head"]]) * 1e3 if fr["head"] in prev else float("nan")
            prev[fr["head"]] = fr["ts0"]
            print(f"  {i:<4}{fr['head']:<5}{fr['bytes']:>9}{fr['images']:>6}  {rid:<30}"
                  f"{gap:>8.1f}  {fr['ts0']:.6f}")

    if args.transfers:
        print("\n== transfers, with the record boundary each one starts on")
        by_off = {r.off: r for r in recs}
        for i, (off, ts, length) in enumerate(starts[:args.count or None]):
            rec = by_off.get(off)
            what = rec.label() if rec else "(mid-record)"
            print(f"  x{i:<5} @{off:<9} len={length:<7} {ts:.6f}  {what}")


if __name__ == "__main__":
    sys.exit(main())
