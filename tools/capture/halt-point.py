#!/usr/bin/env python3
"""Find where the dock halted the endpoint, and what it had been sent up to that point.

`iter_transfers` yields only submissions carrying data, so it cannot see a completion's status --
which is the one field that says the dock objected. This walks the usbmon records directly and
reports, in submission order, the running OUT byte offset of every URB, then the first completion
that came back with an error and which submitted URB it belongs to (matched by usbmon id).

    halt-point.py CAP.pcapng [--ep 2]
"""
import argparse
import struct
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "codec"))
from usbmon_read import _blocks, USBMON_HDR, DLT_USB_LINUX, DLT_USB_LINUX_MMAPPED  # noqa: E402


def events(path):
    linktypes = []
    with open(path, "rb") as fh:
        for btype, body in _blocks(fh):
            if btype == 0x0A0D0D0A:
                linktypes = []
                continue
            if btype == 0x00000001:
                linktypes.append(struct.unpack_from("<H", body, 0)[0])
                continue
            if btype == 0x00000006:
                iface, _hi, _lo, caplen, _o = struct.unpack_from("<IIIII", body, 0)
                if iface < len(linktypes) and linktypes[iface] not in (
                        DLT_USB_LINUX, DLT_USB_LINUX_MMAPPED):
                    continue
                pkt = body[20:20 + caplen]
            elif btype == 0x00000003:
                pkt = body[4:]
            else:
                continue
            if len(pkt) < USBMON_HDR.size:
                continue
            (uid, utype, xfer, epnum, devnum, busnum, _fs, _fd,
             ts_s, ts_us, status, length, len_cap) = USBMON_HDR.unpack_from(pkt, 0)
            yield dict(id=uid, type=chr(utype), xfer=xfer, ep=epnum, dev=devnum,
                       ts=ts_s + ts_us / 1e6, status=status, length=length,
                       cap=len_cap, data=pkt[USBMON_HDR.size:USBMON_HDR.size + len_cap])


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("cap")
    ap.add_argument("--ep", type=int, default=2)
    a = ap.parse_args()

    out_ep, in_ep = a.ep, a.ep | 0x80
    submits = {}        # usbmon id -> (seq, offset, length)
    off = seq = 0
    errors = []
    t0 = None
    n_out = n_in = 0

    for e in events(a.cap):
        if t0 is None:
            t0 = e["ts"]
        if e["ep"] == out_ep and e["type"] == "S":
            submits[e["id"]] = (seq, off, e["length"])
            off += e["length"]
            seq += 1
            n_out += 1
        elif e["ep"] == out_ep and e["type"] == "C":
            s = submits.get(e["id"])
            if e["status"] != 0:
                errors.append((e["ts"] - t0, s, e["status"], e["length"]))
        elif e["ep"] == in_ep:
            n_in += 1

    print(f"{a.cap}")
    print(f"OUT submissions on ep{out_ep}: {n_out}, total {off:,} bytes")
    print(f"IN events on ep{in_ep|0:#x}: {n_in}")
    print(f"errored OUT completions: {len(errors)}")
    if not errors:
        print("\nno errored completion -- the endpoint was never halted in this capture")
        return
    print("\nfirst 12 errors (t, submit-seq, byte offset of that URB, status, len):")
    for ts, s, st, ln in errors[:12]:
        if s:
            print(f"  t={ts:8.3f}s  urb #{s[0]:<6} at byte {s[1]:>12,}  "
                  f"(len {s[2]:>6})  status {st}")
        else:
            print(f"  t={ts:8.3f}s  urb #?      (no matching submission)  status {st}")

    first = errors[0][1]
    if first:
        print(f"\nThe dock stopped accepting at OUT byte {first[1]:,}.")
        print(f"Bytes submitted after that point but never accepted: "
              f"{off - first[1]:,}")


main()
