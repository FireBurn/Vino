#!/usr/bin/env python3
"""Whole-session usbmon recorder for a DLM (or vino) hotplug capture.

Reads /dev/usbmonN via mon_bin directly (dumpcap/tshark are killed in the agent shell) and appends
every bulk/control frame to a binary log.  CP frames (<=~480 B) are stored in full; video payloads
are truncated to --snap bytes (metadata -- endpoint/length/status/timestamp -- is always kept), so a
multi-minute session with video stays small while every control/CP frame is complete for later
decryption with scripts/decrypt-dlm-cp.py.

Record format (little-endian): for each frame,
    u32 rec_len | u8 type('S'/'C'/...) | u8 xfer | u8 epnum | u8 devnum | u16 busnum |
    i64 ts_sec | i32 ts_usec | i32 status | u32 len_urb | u32 len_cap_stored | bytes(data[:snap])
plus a MARKER record (type='M') carrying an ASCII label when --mark is written to the fifo.

Usage:
    sudo python3 scripts/capture-usbmon-session.py --bus 2 --out captures/dlm-hotplug-XXXX.mon \
        [--snap 512] [--secs 900] [--markfifo /tmp/capmark]
Write a label to the markfifo to timestamp an event:  echo "plug-mon1" > /tmp/capmark
Stop cleanly:  echo "STOP" > /tmp/capmark   (or SIGINT)
"""
import argparse, ctypes, fcntl, os, struct, sys, time, select, errno

MON_IOCT_RING_SIZE = (0x92 << 8) | 4
MON_IOCX_GET = (1 << 30) | (24 << 16) | (0x92 << 8) | 6

class MonHdr(ctypes.Structure):
    _fields_ = [("id", ctypes.c_uint64), ("type", ctypes.c_ubyte), ("xfer", ctypes.c_ubyte),
                ("epnum", ctypes.c_ubyte), ("devnum", ctypes.c_ubyte), ("busnum", ctypes.c_uint16),
                ("flag_setup", ctypes.c_char), ("flag_data", ctypes.c_char),
                ("ts_sec", ctypes.c_int64), ("ts_usec", ctypes.c_int32), ("status", ctypes.c_int32),
                ("len_urb", ctypes.c_uint32), ("len_cap", ctypes.c_uint32),
                ("setup", ctypes.c_ubyte * 8)]

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--bus", type=int, default=2)
    ap.add_argument("--out", required=True)
    ap.add_argument("--snap", type=int, default=512)
    ap.add_argument("--secs", type=float, default=1800)
    ap.add_argument("--markfifo", default="/tmp/capmark")
    args = ap.parse_args()

    try:
        os.mkfifo(args.markfifo)
    except FileExistsError:
        pass
    mf = os.open(args.markfifo, os.O_RDONLY | os.O_NONBLOCK)

    fd = os.open(f"/dev/usbmon{args.bus}", os.O_RDONLY)
    try:
        fcntl.ioctl(fd, MON_IOCT_RING_SIZE, 4 << 20)
    except OSError:
        pass

    hdr = MonHdr()
    buf = ctypes.create_string_buffer(1 << 16)
    get = struct.pack("PPn", ctypes.addressof(hdr), ctypes.addressof(buf), len(buf))
    out = open(args.out, "wb")
    n = 0
    marks = []
    end = time.monotonic() + args.secs
    print(f"# recording bus {args.bus} -> {args.out} (snap={args.snap}) mark: echo LABEL > {args.markfifo}", flush=True)
    try:
        while time.monotonic() < end:
            # drain marker fifo
            r, _, _ = select.select([fd, mf], [], [], 0.2)
            if mf in r:
                try:
                    data = os.read(mf, 4096)
                    for lbl in data.split(b"\n"):
                        lbl = lbl.strip()
                        if not lbl:
                            continue
                        if lbl == b"STOP":
                            raise KeyboardInterrupt
                        ts = time.time()
                        rec = struct.pack("<IBBBBHqiiII", 0, ord('M'), 0, 0, 0, 0,
                                          int(ts), int((ts % 1) * 1e6), 0, 0, len(lbl)) + lbl
                        rec = struct.pack("<I", len(rec) - 4) + rec[4:]
                        out.write(rec)
                        marks.append((ts, lbl.decode('ascii', 'replace')))
                        print(f"# MARK {lbl.decode('ascii','replace')} @ {ts:.3f}", flush=True)
                except OSError:
                    pass
                # reopen fifo (writer closed)
                try:
                    os.close(mf)
                except OSError:
                    pass
                mf = os.open(args.markfifo, os.O_RDONLY | os.O_NONBLOCK)
            if fd not in r:
                continue
            try:
                fcntl.ioctl(fd, MON_IOCX_GET, get)
            except OSError as e:
                if e.errno in (errno.EINTR, errno.EAGAIN):
                    continue
                raise
            if hdr.xfer not in (2, 3):  # control, bulk
                continue
            store = min(hdr.len_cap, args.snap)
            data = bytes(buf[:store])
            rec = struct.pack("<IBBBBHqiiII", 0, hdr.type, hdr.xfer, hdr.epnum, hdr.devnum,
                              hdr.busnum, hdr.ts_sec, hdr.ts_usec, hdr.status, hdr.len_urb, store) + data
            rec = struct.pack("<I", len(rec) - 4) + rec[4:]
            out.write(rec)
            n += 1
            if n % 20000 == 0:
                out.flush()
                print(f"# {n} frames, {out.tell()//1024} KiB", flush=True)
    except KeyboardInterrupt:
        pass
    finally:
        out.flush()
        out.close()
        os.close(fd)
        print(f"# DONE {n} frames -> {args.out}", flush=True)
        print("# marks:")
        for ts, lbl in marks:
            print(f"#   {ts:.3f}  {lbl}", flush=True)

if __name__ == "__main__":
    main()
