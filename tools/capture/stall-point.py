#!/usr/bin/env python3
"""Find the last record a dock consumed before it stalled its endpoint.

The driver's own submit-failure warning reports where the error *surfaced*, which on a pipelined
queue is several transfers after the dock objected.  usbmon knows better: an OUT transfer whose
completion never arrives is one the dock stopped consuming, and the record sitting at that byte
offset is the one to look at.

`usbmon_read.iter_transfers` keeps only OUT submissions, which is right for parsing a record stream
and wrong for finding a stall -- the stall is on a completion.  This walks the raw blocks so both
halves are visible.

  stall-point.py CAP.pcapng --dev 7 [--endpoint 2]
"""

from __future__ import annotations

import argparse
import struct

USBMON_HDR = struct.Struct("<QBBBBHccqiiII")
USB_LINKTYPES = (189, 220)  # DLT_USB_LINUX, DLT_USB_LINUX_MMAPPED
SUB_CP = (0x04, 0x24, 0x25, 0x45)
CODEC_SYNC = b"\x01\x28"


def blocks(fh):
    while True:
        head = fh.read(8)
        if len(head) < 8:
            return
        btype, blen = struct.unpack("<II", head)
        if blen < 12:
            return
        body = fh.read(blen - 12)
        fh.read(4)  # trailing length
        yield btype, body


def events(path):
    """Yield every usbmon record, submissions and completions alike."""
    linktypes = []
    with open(path, "rb") as fh:
        for btype, body in blocks(fh):
            if btype == 0x0A0D0D0A:
                linktypes = []
            elif btype == 1:
                linktypes.append(struct.unpack_from("<H", body, 0)[0])
            elif btype == 6:
                iface, _hi, _lo, caplen, _orig = struct.unpack_from("<IIIII", body, 0)
                if iface < len(linktypes) and linktypes[iface] not in USB_LINKTYPES:
                    continue
                pkt = body[20:20 + caplen]
                if len(pkt) < USBMON_HDR.size:
                    continue
                (uid, utype, _xfer, epnum, devnum, _bus, _fs, _fd,
                 sec, usec, status, length, len_cap) = USBMON_HDR.unpack_from(pkt, 0)
                off = 64 if len(pkt) >= 64 + len_cap else 48
                yield dict(id=uid, kind=chr(utype), ep=epnum, dev=devnum,
                           ts=sec + usec / 1e6, status=status, length=length,
                           data=pkt[off:off + len_cap])


def describe(buf: bytes, target: int) -> str:
    """Name the record covering `target` in the concatenated stream."""
    off = 0
    while off + 16 <= len(buf):
        size = int.from_bytes(buf[off + 2:off + 4], "little")
        stride = size + 4
        if stride < 16 or stride % 16 or off + stride > len(buf):
            return f"stream does not parse as far as {target}"
        if off + stride > target:
            sub = int.from_bytes(buf[off + 8:off + 10], "little")
            aux = int.from_bytes(buf[off + 10:off + 12], "little")
            body = buf[off + 16:off + stride]
            if sub in SUB_CP:
                kind = "control"
            elif len(body) >= 4 and body[2:4] == CODEC_SYNC:
                kind = "image"
            else:
                kind = f"body {body[:16].hex()}"
            return (f"record @{off} size={size} sub=0x{sub:04x} aux=0x{aux:04x} {kind}"
                    f"  (target sits {target - off} B in)")
        off += stride
    return f"stream ends before {target}"


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("cap")
    ap.add_argument("--dev", type=int, help="usbmon device number; required on a busy bus")
    ap.add_argument("--endpoint", type=int, default=2)
    args = ap.parse_args()

    stream = bytearray()
    inflight: dict[int, tuple[int, float]] = {}   # urb id -> (stream offset, submit time)
    stalled = None      # the transfer the dock refused
    unconsumed = None   # the earliest transfer outstanding when it did
    t0 = None

    for e in events(args.cap):
        if args.dev is not None and e["dev"] != args.dev:
            continue
        if t0 is None:
            t0 = e["ts"]
        if (e["ep"] & 0x7F) != args.endpoint or e["ep"] & 0x80:
            continue
        if e["kind"] == "S":
            inflight[e["id"]] = (len(stream), e["ts"])
            stream += e["data"]
            continue
        start = inflight.pop(e["id"], None)
        if e["status"] in (0, -115) or stalled is not None:
            continue
        # The first refusal. Everything still outstanding was queued behind it, so the earliest
        # of those is where the dock stopped reading rather than where it complained.
        stalled = (e["ts"], e["status"], start)
        unconsumed = min((off for off, _ts in inflight.values()), default=None)

    print(f"stream {len(stream)} bytes")
    if stalled is None:
        print("every transfer completed -- no stall in this capture")
        return 0

    ts, status, start = stalled
    print(f"first refusal: status={status} at t+{ts - t0:.6f} s")
    if start is not None:
        off, submitted = start
        print(f"  the refused transfer was submitted at offset {off}, "
              f"{ts - submitted:.3f} s earlier")
        print("  " + describe(bytes(stream), off))
    if unconsumed is not None and (start is None or unconsumed < start[0]):
        print(f"\nearliest transfer still outstanding: offset {unconsumed}")
        print("  " + describe(bytes(stream), unconsumed))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
