#!/usr/bin/env python3
"""Read bulk-OUT payloads out of a Linux usbmon pcapng capture.

`video-record-stats.py` shells out to tshark for this, which is fine for a few thousand control
frames but emits the entire payload as hex for a 300 MB video capture.  This walks the pcapng
blocks directly so the same analysis can run over a full-payload capture in seconds.

Handles DLT_USB_LINUX (189) and DLT_USB_LINUX_MMAPPED (220); both put a 48- or 64-byte
`usbmon_packet` in front of the transfer data.
"""

import struct
import sys

DLT_USB_LINUX = 189
DLT_USB_LINUX_MMAPPED = 220

# id, type, xfer_type, epnum, devnum, busnum, flag_setup, flag_data, ts_sec, ts_usec,
# status, length, len_cap
USBMON_HDR = struct.Struct("<QBBBBHccqiiII")


def _blocks(fh):
    """Yield `(block_type, body)` for each pcapng block."""
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


def iter_transfers(path, endpoint=None, transfer_type=None, out_only=True, device=None):
    """Yield `(device, endpoint, payload)` for each submission carrying data.

    usbmon reports the OUT payload on the submission record (`type == 'S'`); the completion
    carries no data for an OUT transfer, so `out_only` filters on that rather than on direction.
    """
    linktypes = []
    with open(path, "rb") as fh:
        for btype, body in _blocks(fh):
            if btype == 0x0A0D0D0A:  # section header
                linktypes = []
                continue
            if btype == 0x00000001:  # interface description
                linktypes.append(struct.unpack_from("<H", body, 0)[0])
                continue
            if btype == 0x00000006:  # enhanced packet
                iface, _hi, _lo, caplen, _origlen = struct.unpack_from("<IIIII", body, 0)
                if iface < len(linktypes) and linktypes[iface] not in (
                    DLT_USB_LINUX, DLT_USB_LINUX_MMAPPED
                ):
                    continue
                pkt = body[20:20 + caplen]
            elif btype == 0x00000003:  # simple packet
                (_origlen,) = struct.unpack_from("<I", body, 0)
                pkt = body[4:]
            else:
                continue

            if len(pkt) < USBMON_HDR.size:
                continue
            (_uid, utype, xfer, epnum, devnum, busnum, _fs, _fd,
             _s, _us, _status, _length, len_cap) = USBMON_HDR.unpack_from(pkt, 0)

            if out_only and utype != ord("S"):
                continue
            ep = epnum & 0x7F if not (epnum & 0x80) else epnum
            if endpoint is not None and ep != endpoint:
                continue
            # Two docks of different generations can share endpoint numbers on one bus, so an
            # endpoint filter alone silently interleaves their record streams.
            if device is not None and devnum != device:
                continue
            if transfer_type is not None and xfer != transfer_type:
                continue
            # mmapped captures add an ISO/interval descriptor before the data.
            off = 64 if len(pkt) >= 64 + len_cap else 48
            data = pkt[off:off + len_cap]
            if not data:
                continue
            yield devnum, ep, data


def main():
    path = sys.argv[1]
    counts = {}
    for device, ep, data in iter_transfers(path, out_only=False):
        key = (device, ep)
        n, total = counts.get(key, (0, 0))
        counts[key] = (n + 1, total + len(data))
    for (device, ep), (n, total) in sorted(counts.items()):
        print(f"dev {device:3d} ep 0x{ep:02x}  {n:7d} records  {total:14,d} bytes")


if __name__ == "__main__":
    main()
