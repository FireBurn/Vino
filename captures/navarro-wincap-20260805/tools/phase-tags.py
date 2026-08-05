#!/usr/bin/env python3
"""Bucket video-record connector tags by time phase.

check-capture.py aggregates a whole file, which hides transitions. This slices the same data
into labelled wall-clock phases so a per-phase change in the `sub` tag is visible.

    python phase-tags.py cap3-hdr.pcap 11:54:08=SDR-both 11:55:17=HDR-right 11:55:55=HDR-both 11:57:05=SDR-both
"""
import collections
import datetime as dt
import os
import subprocess
import sys

TSHARK = r"C:\Program Files\Wireshark\tshark.exe"
VIDEO_EPS = ("0x08", "0x0a")


def main():
    if len(sys.argv) < 3:
        sys.exit(__doc__)
    path = sys.argv[1]
    if not os.path.exists(path):
        sys.exit(f"no such file: {path}")

    # phase boundaries: HH:MM:SS=label, applied to the capture's own date
    marks = []
    for a in sys.argv[2:]:
        hhmmss, label = a.split("=", 1)
        marks.append((hhmmss, label))

    cmd = [TSHARK, "-r", path, "-T", "fields",
           "-e", "frame.time_epoch", "-e", "usb.endpoint_address",
           "-e", "usb.data_len", "-e", "usb.capdata"]
    p = subprocess.run(cmd, capture_output=True, text=True, check=False)
    if p.returncode != 0 and not p.stdout:
        sys.exit(f"tshark failed:\n{p.stderr[:2000]}")

    rows = []
    for line in p.stdout.splitlines():
        f = line.split("\t")
        if len(f) < 4 or not f[1]:
            continue
        if f[1] not in VIDEO_EPS or not f[3]:
            continue
        try:
            ts = float(f[0])
        except ValueError:
            continue
        h = f[3].replace(":", "")
        if len(h) < 20:
            continue
        d = bytes.fromhex(h[:20])
        if int.from_bytes(d[4:8], "little") != 4:
            continue                      # not a video record
        sub = int.from_bytes(d[8:10], "little")
        try:
            nbytes = int(f[2]) if f[2] else 0
        except ValueError:
            nbytes = 0
        rows.append((ts, f[1], sub, nbytes))

    if not rows:
        sys.exit("no video records decoded")

    day = dt.datetime.fromtimestamp(rows[0][0]).date()
    bounds = []
    for hhmmss, label in marks:
        hh, mm, ss = (int(x) for x in hhmmss.split(":"))
        t = dt.datetime.combine(day, dt.time(hh, mm, ss)).timestamp()
        bounds.append((t, label))
    bounds.sort()

    def phase_of(ts):
        lab = None
        for t, label in bounds:
            if ts >= t:
                lab = label
            else:
                break
        return lab or "(before first mark)"

    agg = collections.defaultdict(lambda: collections.defaultdict(collections.Counter))
    vol = collections.defaultdict(lambda: collections.Counter())
    for ts, ep, sub, nbytes in rows:
        ph = phase_of(ts)
        agg[ph][ep][sub] += 1
        vol[ph][ep] += nbytes

    order = ["(before first mark)"] + [l for _, l in bounds]
    seen = []
    for o in order:
        if o in agg and o not in seen:
            seen.append(o)

    for ph in seen:
        print(f"=== {ph} ===")
        for ep in VIDEO_EPS:
            if ep not in agg[ph]:
                continue
            mb = vol[ph][ep] / 1e6
            print(f"  ep {ep}   ({mb:,.1f} MB)")
            for sub, n in sorted(agg[ph][ep].items()):
                conn = sub >> 3
                kind = ""
                if sub & 0x07 == 7:
                    kind = f"connector {(sub & 0x1f) >> 3} STREAM-OPEN"
                else:
                    base = sub & 0x1f
                    flag = sub & 0x20
                    kind = f"connector {base >> 3}" + ("   [+0x20 HDR flag]" if flag else "   [plain]")
                print(f"     sub=0x{sub:04x}  x{n:<7} {kind}")
        print()


if __name__ == "__main__":
    sys.exit(main())
