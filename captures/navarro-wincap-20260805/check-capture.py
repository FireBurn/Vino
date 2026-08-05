#!/usr/bin/env python3
"""Verify a Windows USBPcap capture of a DL7400 (Navarro) dock BEFORE you reboot out of Windows.

A capture that looks healthy from the driver's side and contains no video at all is a real and
repeated failure mode -- it happened twice on the Linux side, where the control plane came up, EDID
was read and a correct mode list was published while the panels stayed dark and not one video byte
crossed the wire. There is no way to tell from the desktop. So check the bytes, here, while the
dock is still plugged in and the run can be repeated.

    python check-capture.py out\\cap1.pcap

Uses tshark, which ships with Wireshark; no Python packages needed.
"""
import collections
import os
import subprocess
import sys

TSHARK_CANDIDATES = [
    r"C:\Program Files\Wireshark\tshark.exe",
    r"C:\Program Files (x86)\Wireshark\tshark.exe",
    "tshark",
]

# Interface 0 of a DL7400. Anything outside this set on the dock's device address is a surprise
# worth reporting rather than hiding.
EP_ROLE = {
    "0x02": "control OUT",
    "0x84": "control IN",
    "0x08": "VIDEO (connectors 0 and 2)",
    "0x0a": "VIDEO (connectors 1 and 3)",
    "0x80": "ep0 control",
    "0x83": "audio interrupt IN",
}
VIDEO_EPS = ("0x08", "0x0a")


def find_tshark():
    for c in TSHARK_CANDIDATES:
        if os.path.exists(c):
            return c
    return "tshark"


def run(path):
    tshark = find_tshark()
    fields = ["usb.device_address", "usb.endpoint_address", "usb.data_len", "usb.capdata"]
    cmd = [tshark, "-r", path, "-T", "fields"]
    for f in fields:
        cmd += ["-e", f]
    try:
        p = subprocess.run(cmd, capture_output=True, text=True, check=False)
    except FileNotFoundError:
        sys.exit(f"tshark not found -- looked in {TSHARK_CANDIDATES}")
    if p.returncode != 0 and not p.stdout:
        sys.exit(f"tshark failed:\n{p.stderr[:2000]}")
    return p.stdout.splitlines()


def main():
    if len(sys.argv) < 2:
        sys.exit(__doc__)
    path = sys.argv[1]
    if not os.path.exists(path):
        sys.exit(f"no such file: {path}")

    rows = run(path)
    per = collections.defaultdict(lambda: [0, 0])          # (dev, ep) -> [count, bytes]
    subs = collections.defaultdict(collections.Counter)    # ep -> Counter(sub)
    for line in rows:
        f = line.split("\t")
        if len(f) < 3:
            continue
        dev, ep, dlen = f[0], f[1], f[2]
        cap = f[3] if len(f) > 3 else ""
        if not ep:
            continue
        try:
            n = int(dlen) if dlen else 0
        except ValueError:
            n = 0
        slot = per[(dev, ep)]
        slot[0] += 1
        slot[1] += n
        # The video record header is PLAINTEXT: type at bytes 4..8, sub at 8..10, little-endian.
        # Only type==4 records are real frame records; mid-transfer continuations decode as noise
        # and are dropped by that test.
        if ep in VIDEO_EPS and cap:
            h = cap.replace(":", "")
            if len(h) >= 20:
                d = bytes.fromhex(h[:20])
                if int.from_bytes(d[4:8], "little") == 4:
                    subs[ep][int.from_bytes(d[8:10], "little")] += 1

    # A capture that spans a replug contains the dock under SEVERAL device addresses -- USB assigns
    # a new one each enumeration -- so picking the first match silently reports a fraction of the
    # run. Take every address that carries video, and lead with the busiest.
    video_by_dev = collections.Counter()
    for (dev, ep), (_c, b) in per.items():
        if ep in VIDEO_EPS:
            video_by_dev[dev] += b
    docks = [d for d, b in video_by_dev.most_common() if b > 0]
    if not docks and per:
        docks = [max(per.items(), key=lambda kv: kv[1][1])[0][0]]
    dock = docks[0] if docks else None

    print(f"=== {path} ===")
    if len(docks) > 1:
        print(f"dock device addresses: {docks}  "
              f"(several = the dock re-enumerated; a replug or a firmware flash)")
    else:
        print(f"dock device address: {dock or '(none found)'}")
    print()
    print(f"{'dev/ep':<12} {'transfers':>10} {'bytes':>14}   role")
    total_video = 0
    for (dev, ep), (c, b) in sorted(per.items(), key=lambda kv: -kv[1][1]):
        if dev not in docks:
            continue
        role = EP_ROLE.get(ep, "unexpected -- report this")
        print(f"{dev+'/'+ep:<12} {c:>10} {b:>14,}   {role}")
        if ep in VIDEO_EPS:
            total_video += b

    print("\n--- connector tags in video records (sub = connector << 3) ---")
    seen_connectors = set()
    for ep in VIDEO_EPS:
        if not subs[ep]:
            continue
        print(f"  ep {ep}:")
        for s, n in sorted(subs[ep].items()):
            if s % 8 == 0 and (s >> 3) < 4:
                seen_connectors.add(s >> 3)
                note = f"connector {s >> 3}  (physical socket {(s >> 3) + 1})"
            elif s % 8 == 7 and (s >> 3) < 4:
                note = f"connector {s >> 3} STREAM-OPEN"
            else:
                note = ""
            print(f"     sub=0x{s:04x}  x{n:<7} {note}")

    print("\n" + "=" * 68)
    ok = True
    if total_video == 0:
        print("VERDICT: FAIL -- no video bytes on 0x08/0x0a.")
        print("         The screens were not actually being driven. This capture cannot answer")
        print("         anything about video. REDO the choreography before rebooting.")
        ok = False
    else:
        print(f"VERDICT: PASS -- {total_video:,} bytes of video captured.")
    if seen_connectors:
        print(f"VERDICT: connectors seen driving video: {sorted(seen_connectors)} "
              f"(sockets {[c + 1 for c in sorted(seen_connectors)]})")
        if len(seen_connectors) > 2:
            print("         >2 connectors -- this is new ground; Linux has only ever seen two at once.")
    else:
        print("VERDICT: no connector tags decoded. If there IS video volume above, the record")
        print("         framing differs from Linux -- that is itself a finding. Note it and keep")
        print("         the capture.")
    unexpected = [ep for (dev, ep) in per if dev in docks and ep not in EP_ROLE]
    if unexpected:
        print(f"VERDICT: UNEXPECTED endpoints on the dock: {sorted(set(unexpected))}")
        print("         Linux has only ever seen 0x02/0x84/0x08/0x0a/0x80/0x83. Report this.")
    print("=" * 68)
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
