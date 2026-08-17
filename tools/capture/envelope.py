#!/usr/bin/env python3
"""Peak bytes a sender puts on an endpoint in a sliding window, at several timescales.

"The dock cannot take this much" keeps being proposed and keeps being tested against the wrong
number.  Bytes bucketed by calendar second hide a burst that straddles a bucket boundary -- a run
measured that way read 9.9/13.8/11.3 MB in consecutive seconds when its worst *sliding* second was
23.3 MB.  And a single figure cannot describe either sender: both burst a frame at over
150 MB/s and then go quiet, so the envelope has to be read as a curve.

Point it at the vendor capture and at a driver capture and compare the rows.  Anything the driver
does that the vendor also does is not what stopped the dock.

  envelope.py CAP.pcapng [--ep 2]
"""

from __future__ import annotations

import argparse
import bisect
import struct
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "codec"))
import usbmon_read as u  # noqa: E402

WINDOWS = (0.05, 0.1, 0.25, 0.5, 1.0, 2.0, 3.0, 5.0)


def submissions(path: str, ep: int):
    """Yield `(timestamp, bytes)` for every OUT submission on `ep`."""
    linktypes: list[int] = []
    with open(path, "rb") as fh:
        for btype, body in u._blocks(fh):
            if btype == 0x0A0D0D0A:
                linktypes = []
                continue
            if btype == 0x00000001:
                linktypes.append(struct.unpack_from("<H", body, 0)[0])
                continue
            if btype != 0x00000006:
                continue
            iface, _hi, _lo, caplen, _orig = struct.unpack_from("<IIIII", body, 0)
            if iface < len(linktypes) and linktypes[iface] not in (
                u.DLT_USB_LINUX,
                u.DLT_USB_LINUX_MMAPPED,
            ):
                continue
            pkt = body[20 : 20 + caplen]
            if len(pkt) < u.USBMON_HDR.size:
                continue
            f = u.USBMON_HDR.unpack_from(pkt, 0)
            utype, xfer, epnum, sec, usec, length = chr(f[1]), f[2], f[3], f[8], f[9], f[11]
            if xfer != 3 or epnum & 0x80 or (epnum & 0x7F) != ep or utype != "S":
                continue
            yield sec + usec / 1e6, length


def main() -> int:
    ap = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    ap.add_argument("capture")
    ap.add_argument("--ep", type=int, default=2, help="OUT endpoint number (default 2)")
    args = ap.parse_args()

    ev = sorted(submissions(args.capture, args.ep))
    if len(ev) < 2:
        print(f"no OUT traffic on endpoint {args.ep:#04x}")
        return 1
    ts = [t for t, _ in ev]
    cum = [0]
    for _, n in ev:
        cum.append(cum[-1] + n)
    span = ts[-1] - ts[0]
    print(
        f"{len(ev)} transfers, {cum[-1]:,} bytes over {span:.1f} s "
        f"= {cum[-1] / max(span, 1e-9) / 1e6:.2f} MB/s mean"
    )
    for w in WINDOWS:
        best = 0
        for i, t in enumerate(ts):
            j = bisect.bisect_right(ts, t + w)
            best = max(best, cum[j] - cum[i])
        print(f"  peak over {w:5.2f} s: {best / 1e6:8.2f} MB = {best / w / 1e6:7.2f} MB/s")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
