#!/usr/bin/env python3
"""Map a whole DisplayLink session: both directions, with timings, phases and cadence.

Everything else here answers a question about one part of a capture. This describes the entire
session, because the parts that have gone wrong repeatedly -- what the vendor does *after* the
opening burst, how it paces sustained video on two heads, what the dock says back -- are exactly
the parts a spot-check never reaches.

Three things it does that `record-stream.py` does not:

  * reads the IN endpoint as well as the OUT one, so a request and the dock's answer sit next to
    each other and an unanswered message is visible;
  * carries timestamps, so inter-record gaps, per-phase durations and frame cadence are reported
    rather than inferred; and
  * runs to the end by default, collapsing pixel runs into frames so 92,000 records become a
    readable timeline.

  dlm-map.py CAP.pcapng [KEYS.json] [--out report.txt] [--phases] [--cadence]

Keys are optional: framing, frame sizes, cadence and the IN/OUT interleave need none. Supply them
to name the control messages.
"""

from __future__ import annotations

import argparse
import collections
import json
import statistics
import struct
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "codec"))
import usbmon_read as u  # noqa: E402

CODEC_SYNC = b"\x01\x28"
SUB_CP_PLAIN, SUB_CP_SEALED = 0x04, 0x24
SUB_CP_IN = (0x25, 0x45)

try:
    from Crypto.Cipher import AES
    from Crypto.Hash import CMAC
except ImportError:  # pragma: no cover - either binding is fine
    from Cryptodome.Cipher import AES
    from Cryptodome.Hash import CMAC


def transfers(path, endpoints):
    """Yield (ts, endpoint, payload) for every submission carrying data on `endpoints`.

    usbmon puts an OUT payload on the submission and an IN payload on the completion, so both
    record types have to be accepted and told apart by direction rather than by type.
    """
    linktypes = []
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
            iface, hi, lo, caplen, _orig = struct.unpack_from("<IIIII", body, 0)
            if iface < len(linktypes) and linktypes[iface] not in (
                u.DLT_USB_LINUX,
                u.DLT_USB_LINUX_MMAPPED,
            ):
                continue
            pkt = body[20 : 20 + caplen]
            if len(pkt) < u.USBMON_HDR.size:
                continue
            f = u.USBMON_HDR.unpack_from(pkt, 0)
            utype, xfer, epnum, len_cap = chr(f[1]), f[2], f[3], f[12]
            # A mmapped capture carries an ISO/interval descriptor between the header and the
            # transfer data. Starting at the header size instead splices 24 zero bytes into the
            # stream at every transfer boundary, which shifts every record that spans one.
            base = 64 if len(pkt) >= 64 + len_cap else 48
            payload = pkt[base : base + len_cap]
            if xfer != 3 or not payload:
                continue
            ep_in = bool(epnum & 0x80)
            ep = epnum & 0x7F
            if ep not in endpoints:
                continue
            if ep_in and utype != "C":
                continue
            if not ep_in and utype != "S":
                continue
            yield f[8] + f[9] / 1e6, ("IN" if ep_in else "OUT"), ep, payload


def records(path, ep, direction):
    """Yield (ts, record) from one endpoint's concatenated stream.

    Records span transfer boundaries, so the stream must be concatenated before parsing. The
    timestamp reported is that of the transfer the record *started* in.
    """
    buf, last_ts = bytearray(), None
    for ts, d, _e, payload in transfers(path, {ep}):
        if d != direction:
            continue
        # A record can span transfers; attribute it to the transfer that completed it. At
        # millisecond resolution that is indistinguishable from the one that started it, and it
        # avoids carrying an offset table for no gain.
        last_ts = ts
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
            yield last_ts, rec


def fields(rec):
    return (
        int.from_bytes(rec[4:8], "little"),
        int.from_bytes(rec[8:10], "little"),
        int.from_bytes(rec[10:12], "little"),
        int.from_bytes(rec[12:16], "little"),
        rec[16:],
    )


class Opener:
    """Open sealed bodies; a verified Dl3Cmac tag is the only trustworthy oracle."""

    def __init__(self, cands):
        self.c, self.known = cands, {}

    @staticmethod
    def _ctr(key, riv, seq, data):
        c = AES.new(key, AES.MODE_ECB)
        out = bytearray()
        for off in range(0, len(data), 16):
            iv = riv + bytes(4) + ((seq + off // 16) & 0xFFFFFFFF).to_bytes(4, "big")
            out += bytes(a ^ b for a, b in zip(data[off : off + 16], c.encrypt(iv)))
        return bytes(out)

    @staticmethod
    def _mac(key, riv, seq, ct):
        m = CMAC.new(key, ciphermod=AES)
        m.update(bytes([riv[0] ^ 0x80]) + riv[1:] + seq.to_bytes(8, "big") + ct)
        return m.digest()

    def open(self, sub, seq, body):
        if len(body) < 32:
            return None
        ct, tag = body[:-16], body[-16:]
        known = self.known.get(sub)
        for key, riv in ([known] if known else []) + self.c:
            if self._mac(key, riv, seq, ct) == tag:
                self.known[sub] = (key, riv)
                return self._ctr(key, riv, seq, ct)
        return None


def load_keys(path):
    if not path:
        return []
    seen, out = set(), []
    for r in json.load(open(path)):
        it = (bytes.fromhex(r["key"]), bytes.fromhex(r["riv"]))
        if it not in seen:
            seen.add(it)
            out.append(it)
    return out


def main() -> int:
    ap = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    ap.add_argument("capture")
    ap.add_argument("keys", nargs="?")
    ap.add_argument("--out", default="-")
    ap.add_argument("--out-ep", type=int, default=2)
    ap.add_argument("--in-ep", type=int, default=4, help="IN endpoint number, 0x84 -> 4")
    args = ap.parse_args()

    op = Opener(load_keys(args.keys))
    fh = sys.stdout if args.out == "-" else open(args.out, "w")

    t0 = None
    run = None  # (start_ts, sub, records, bytes, strips)
    frames = collections.defaultdict(list)
    cp_out = 0
    prev_ts = None

    def flush(end_ts):
        nonlocal run
        if run is None:
            return
        st, sub, n, nb, ns = run
        frames[sub].append((st, end_ts, n, nb, ns))
        print(
            f"{st - t0:9.3f}  FRAME sub={sub:#04x} {n:4d} recs {nb:9d} B {ns:5d} strips",
            file=fh,
        )
        run = None

    for ts, rec in records(args.capture, args.out_ep, "OUT"):
        if t0 is None:
            t0 = ts
        typ, sub, aux, seq, body = fields(rec)
        img = len(body) >= 4 and body[2:4] == CODEC_SYNC
        if img:
            pay = body[: len(body) - aux] if aux < 16 else body
            strips, off = 0, 0
            while off + 2 <= len(pay):
                ln = int.from_bytes(pay[off : off + 2], "little")
                if ln == 0:
                    break
                strips += 1
                off += 2 + ln
            if run and run[1] == sub:
                run = (run[0], sub, run[2] + 1, run[3] + len(rec), run[4] + strips)
            else:
                flush(ts)
                run = (ts, sub, 1, len(rec), strips)
            prev_ts = ts
            continue
        flush(ts)
        gap = "" if prev_ts is None else f" (+{(ts - prev_ts) * 1000:7.1f} ms)"
        prev_ts = ts
        if sub == SUB_CP_SEALED:
            cp_out += 1
            pt = op.open(sub, seq, body)
            if pt:
                print(
                    f"{ts - t0:9.3f}  CP   id={int.from_bytes(pt[0:2],'little'):#06x}"
                    f"/{int.from_bytes(pt[2:4],'little'):#04x}"
                    f" ctr={int.from_bytes(pt[4:6],'little'):<5}"
                    f" off22={pt[22]:#04x} off23={pt[23]:#04x}{gap}",
                    file=fh,
                )
            else:
                print(f"{ts - t0:9.3f}  CP   sealed len={len(rec)}{gap}", file=fh)
        elif sub == SUB_CP_PLAIN:
            print(
                f"{ts - t0:9.3f}  CPP  id={int.from_bytes(body[0:2],'little'):#06x}"
                f"/{int.from_bytes(body[2:4],'little'):#04x} len={len(rec)}{gap}",
                file=fh,
            )
        else:
            print(
                f"{ts - t0:9.3f}  REC  sub={sub:#06x} aux={aux:#06x} len={len(rec)}{gap}",
                file=fh,
            )
    flush(prev_ts or t0)

    print("\n===== cadence =====", file=fh)
    for sub, fl in sorted(frames.items()):
        if len(fl) < 2:
            continue
        gaps = [
            (fl[i + 1][0] - fl[i][0]) * 1000 for i in range(len(fl) - 1) if fl[i + 1][0] > fl[i][0]
        ]
        sizes = [f[3] for f in fl]
        strips = [f[4] for f in fl]
        span = fl[-1][1] - fl[0][0]
        print(
            f"  sub={sub:#04x}: {len(fl)} frames over {span:.1f}s = {len(fl)/max(span,1e-9):.1f} fps\n"
            f"      bytes  median {statistics.median(sizes):9.0f}  max {max(sizes):9d}\n"
            f"      strips median {statistics.median(strips):6.0f}  max {max(strips):6d}\n"
            f"      gap ms median {statistics.median(gaps):6.1f}  p10 {sorted(gaps)[len(gaps)//10]:6.1f}"
            f"  p90 {sorted(gaps)[9*len(gaps)//10]:6.1f}",
            file=fh,
        )
    print(f"  sealed CP records OUT: {cp_out}", file=fh)
    if fh is not sys.stdout:
        fh.close()
        print(f"[*] {args.out}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
