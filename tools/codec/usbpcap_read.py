#!/usr/bin/env python3
"""Read bulk-OUT payloads out of a USBPcap capture.

The Windows corpus (`captures/navarro-wincap-20260802/`) is recorded with USBPcap, whose link
type the repository's usbmon readers do not understand.  Going through `tshark -T fields` works
but emits the whole 244 MB payload as hex, so this walks the file directly instead.

USBPcap's per-packet header is documented at https://desowin.org/usbpcap/captureformat.html and
is fixed-layout: everything this needs is in the first 27 bytes, and `headerLen` skips whatever a
transfer-specific suffix adds after that.
"""

import datetime
import struct
import sys

PCAP_GLOBAL = struct.Struct("<IHHiIII")
PCAP_REC = struct.Struct("<IIII")
# headerLen, irpId, status, function, info, bus, device, endpoint, transfer, dataLength
USBPCAP_HDR = struct.Struct("<HQIHBHHBBI")

DLT_USBPCAP = 249


def iter_transfers(path, endpoint=None, transfer_type=None, out_only=True,
                   device=None, since=None, until=None):
    """Yield `(device, endpoint, payload)` for each captured USB transfer carrying data.

    `info` bit 0 is USBPCAP_INFO_PDO_TO_FDO -- set on the completion (device->host direction of
    the IRP).  Submissions carry the OUT payload, so that bit must be clear for bulk OUT.

    `since`/`until` are seconds since local midnight, matching how the Windows harness timestamps
    its phase logs -- a capture of a scripted session is only useful sliced by phase, and the
    phase boundaries are wall-clock.  `device` filters by USB address: two docks on one bus both
    use endpoint 0x08, and an endpoint-only filter interleaves their record streams.
    """
    with open(path, "rb") as f:
        head = f.read(PCAP_GLOBAL.size)
        magic, vmaj, vmin, tz, sigfigs, snaplen, network = PCAP_GLOBAL.unpack(head)
        if magic not in (0xA1B2C3D4, 0xA1B23C4D):
            raise ValueError(f"{path}: not a little-endian pcap (magic {magic:#x})")
        if network != DLT_USBPCAP:
            raise ValueError(f"{path}: link type {network}, expected USBPcap ({DLT_USBPCAP})")

        while True:
            rec = f.read(PCAP_REC.size)
            if len(rec) < PCAP_REC.size:
                return
            ts, tus, caplen, _origlen = PCAP_REC.unpack(rec)
            pkt = f.read(caplen)
            if len(pkt) < caplen or caplen < USBPCAP_HDR.size:
                return
            if since is not None or until is not None:
                lt = datetime.datetime.fromtimestamp(ts + tus / 1e6)
                secs = lt.hour * 3600 + lt.minute * 60 + lt.second + lt.microsecond / 1e6
                if since is not None and secs < since:
                    continue
                if until is not None and secs > until:
                    return
            (hlen, _irp, _status, _func, info, _bus, dev,
             ep, xfer, dlen) = USBPCAP_HDR.unpack_from(pkt, 0)
            if hlen > caplen:
                continue
            if device is not None and dev != device:
                continue
            if out_only and (info & 0x01):
                continue
            if endpoint is not None and ep != endpoint:
                continue
            if transfer_type is not None and xfer != transfer_type:
                continue
            data = pkt[hlen:hlen + dlen]
            if not data:
                continue
            yield dev, ep, data


def main():
    path = sys.argv[1]
    counts = {}
    for device, ep, data in iter_transfers(path, out_only=False):
        key = (device, ep)
        n, total = counts.get(key, (0, 0))
        counts[key] = (n + 1, total + len(data))
    for (device, ep), (n, total) in sorted(counts.items()):
        print(f"dev {device:3d} ep 0x{ep:02x}  {n:6d} transfers  {total:14,d} bytes")


if __name__ == "__main__":
    main()
