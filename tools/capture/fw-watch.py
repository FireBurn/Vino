#!/usr/bin/env python3
"""Live firmware-flash watcher and independent usbmon recorder.

Two jobs, deliberately in one process:

  1. RECORD every control and bulk frame in FULL, straight off /dev/usbmonN via mon_bin. This is a
     second, independent capture path alongside dumpcap: different code, different failure modes.
     A firmware flash is a one-shot event, so one capture backend is one point of failure.

  2. TELL YOU LIVE whether the flash is happening, while the device is still on the desk and a
     retry is still possible. It counts bytes per endpoint and watches for the container magic
     "ELLA" and for byte windows that match the shipped *-release.spkg images. Discovering a
     miss the next morning is the failure mode this exists to prevent.

    sudo tools/capture/fw-watch.py --bus 2 --out fw.mon --spkg-dir /opt/displaylink [--secs 1800]

--bus 0 follows ALL buses, which is what you want if the device may re-enumerate onto another bus
in bootloader mode.

Record format: file magic "VFW2", then per frame
    u32 rec_len | u64 urb_id | u8 type | u8 xfer | u8 epnum | u8 devnum | u16 busnum |
    u8 flag_setup | u8 pad | i64 ts_sec | i32 ts_usec | i32 status | u32 len_urb | u32 len_stored |
    u8 setup[8] | data

Two fields here are not in tools/hardware/capture-usbmon-session.py's format, and both are needed
because a DisplayLink dock exposes a standard USB DFU interface (class 0xfe / subclass 0x01), so
the flash is CLASS CONTROL TRANSFERS:

  * `setup` -- without the setup packet a control transfer is an anonymous blob: no bRequest, no
    block number, no direction.
  * `urb_id` -- mon_bin fills `setup` ONLY on the 'S' (submission) record
    (`usb_endpoint_xfer_control(epd) && ev_type == 'S'` in mon_bin.c). A control IN reply, such as
    the DFU_GETSTATUS that reports bStatus/bState, carries its DATA on the 'C' record, which has no
    setup at all. Without the URB id there is nothing to pair the two by, and every device reply in
    the flash would be unattributable.
"""
import argparse, ctypes, errno, fcntl, glob, os, select, signal, struct, sys, time

MON_IOCT_RING_SIZE = (0x92 << 8) | 4
MON_IOCX_GET = (1 << 30) | (24 << 16) | (0x92 << 8) | 6
BUFF_MAX = 1228800  # mon_bin's maximum ring size; ask for all of it, drops are unrecoverable here

WINDOW = 32   # bytes of a spkg used as a fingerprint
STRIDE = 16   # sampling stride; any contiguous run of WINDOW+STRIDE-1 image bytes is caught

MAGIC = b"VFW2"
REC = "<QBBBBHBBqiiII8s"

# USB DFU 1.1 class requests, on bmRequestType 0x21 (OUT) / 0xa1 (IN), recipient = interface.
DFU_REQ = {0: "DETACH", 1: "DNLOAD", 2: "UPLOAD", 3: "GETSTATUS", 4: "CLRSTATUS",
           5: "GETSTATE", 6: "ABORT"}


def is_dfu(setup):
    """A class request addressed to an interface, with a DFU request number."""
    if len(setup) < 8:
        return None
    bm, br = setup[0], setup[1]
    if (bm & 0x60) != 0x20 or (bm & 0x1f) != 0x01 or br not in DFU_REQ:
        return None
    windex = int.from_bytes(setup[4:6], "little")
    # USB Audio Class SET_CUR is also 0x21/0x01 -- byte-for-byte a DFU_DNLOAD except that audio
    # puts (entity << 8) | interface in wIndex while DFU puts a bare interface number. A dock with
    # UAC interfaces generates a stream of these, and without this test they read as a firmware
    # download with nonsense block numbers.
    if windex >> 8 or windex > 8:
        return None
    # DETACH/DNLOAD/CLRSTATUS/ABORT are OUT; UPLOAD/GETSTATUS/GETSTATE are IN.
    if bool(bm & 0x80) != (br in (2, 3, 5)):
        return None
    return {"dir": "IN" if bm & 0x80 else "OUT", "req": DFU_REQ[br], "num": br,
            "wValue": int.from_bytes(setup[2:4], "little"),
            "wIndex": windex,
            "wLength": int.from_bytes(setup[6:8], "little")}


class MonHdr(ctypes.Structure):
    _fields_ = [("id", ctypes.c_uint64), ("type", ctypes.c_ubyte), ("xfer", ctypes.c_ubyte),
                ("epnum", ctypes.c_ubyte), ("devnum", ctypes.c_ubyte), ("busnum", ctypes.c_uint16),
                ("flag_setup", ctypes.c_char), ("flag_data", ctypes.c_char),
                ("ts_sec", ctypes.c_int64), ("ts_usec", ctypes.c_int32), ("status", ctypes.c_int32),
                ("len_urb", ctypes.c_uint32), ("len_cap", ctypes.c_uint32),
                ("setup", ctypes.c_ubyte * 8)]


def load_spkgs(d):
    """Fingerprint every shipped firmware image so wire bytes can be attributed to one."""
    sigs, meta = {}, []
    for path in sorted(glob.glob(os.path.join(d, "*-release.spkg"))):
        blob = open(path, "rb").read()
        name = os.path.basename(path)
        for off in range(0, max(0, len(blob) - WINDOW), STRIDE):
            sigs.setdefault(blob[off:off + WINDOW], (name, off))
        meta.append((name, len(blob), blob[:8].hex(" ")))
    return sigs, meta


def fmt(n):
    if n >= 1 << 30:
        return f"{n / (1<<30):.2f} GiB"
    if n >= 1 << 20:
        return f"{n / (1<<20):.2f} MiB"
    if n >= 1 << 10:
        return f"{n / (1<<10):.1f} KiB"
    return f"{n} B"


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--bus", type=int, default=0, help="usbmon bus; 0 = all buses")
    ap.add_argument("--out", required=True)
    ap.add_argument("--spkg-dir", default="/opt/displaylink")
    ap.add_argument("--secs", type=float, default=3600)
    ap.add_argument("--vid", default="17e9")
    ap.add_argument("--events", default=None, help="append flash events here (default <out>.events)")
    ap.add_argument("--interval", type=float, default=3.0, help="live report period")
    args = ap.parse_args()

    ev_path = args.events or (args.out + ".events")
    ev = open(ev_path, "a", buffering=1)

    def event(msg):
        line = f"{time.time():.3f}  {msg}"
        ev.write(line + "\n")
        print(f"\033[1;35m★ {msg}\033[0m", flush=True)

    sigs, meta = load_spkgs(args.spkg_dir)
    print(f"# fingerprinted {len(sigs)} windows from {len(meta)} firmware image(s):")
    for name, size, magic in meta:
        print(f"#   {name:<34} {size:>9} B  magic {magic}")
    if not sigs:
        print("# WARNING: no *-release.spkg fingerprints — payload attribution disabled")

    fd = os.open(f"/dev/usbmon{args.bus}", os.O_RDONLY)
    for want in (BUFF_MAX, 4 << 20, 1 << 20):
        try:
            fcntl.ioctl(fd, MON_IOCT_RING_SIZE, want)
            print(f"# usbmon ring size {want} B")
            break
        except OSError:
            continue

    hdr = MonHdr()
    buf = ctypes.create_string_buffer(1 << 16)
    get = struct.pack("PPn", ctypes.addressof(hdr), ctypes.addressof(buf), len(buf))
    out = open(args.out, "wb")
    out.write(MAGIC)

    # (busnum, devnum, epnum) -> [frames, bytes]
    counts = {}
    matched = {}          # spkg name -> bytes seen on the wire
    dfu_counts = {}       # DFU request name -> count
    dfu_bytes = 0
    seen_magic = False
    frames = 0
    stop = False

    def on_sig(_s, _f):
        nonlocal stop
        stop = True
    signal.signal(signal.SIGINT, on_sig)
    signal.signal(signal.SIGTERM, on_sig)

    end = time.monotonic() + args.secs
    last_report = 0.0
    last_traffic = time.monotonic()
    print(f"# recording bus {args.bus} -> {args.out}  (Ctrl-C to stop)", flush=True)
    print("# DO NOT UNPLUG ANYTHING WHILE A TRANSFER IS IN PROGRESS.", flush=True)

    try:
        while not stop and time.monotonic() < end:
            r, _, _ = select.select([fd], [], [], 0.25)
            now = time.monotonic()
            if now - last_report >= args.interval:
                last_report = now
                total = sum(v[1] for v in counts.values())
                top = sorted(counts.items(), key=lambda kv: -kv[1][1])[:6]
                bits = " ".join(f"{b}.{d}/ep{e:02x}={fmt(v[1])}" for (b, d, e), v in top)
                quiet = now - last_traffic
                dfu = f"  DFU dnload={dfu_counts.get('DNLOAD', 0)} ({fmt(dfu_bytes)})" if dfu_counts else ""
                print(f"\r\033[K  {frames:>7} frames  {fmt(total):>10}  quiet {quiet:4.0f}s{dfu}  {bits}",
                      end="", flush=True)
            if fd not in r:
                continue
            try:
                fcntl.ioctl(fd, MON_IOCX_GET, get)
            except OSError as e:
                if e.errno in (errno.EINTR, errno.EAGAIN):
                    continue
                raise
            if hdr.xfer not in (2, 3):   # control, bulk
                continue
            store = min(int(hdr.len_cap), len(buf))
            data = bytes(buf[:store])
            setup = bytes(hdr.setup)
            body = struct.pack(REC, hdr.id, hdr.type, hdr.xfer, hdr.epnum, hdr.devnum, hdr.busnum,
                               ord(hdr.flag_setup) if hdr.flag_setup else 0, 0,
                               hdr.ts_sec, hdr.ts_usec, hdr.status, hdr.len_urb, store,
                               setup) + data
            out.write(struct.pack("<I", len(body)) + body)
            frames += 1
            if store:
                last_traffic = time.monotonic()

            key = (int(hdr.busnum), int(hdr.devnum), int(hdr.epnum))
            c = counts.setdefault(key, [0, 0])
            c[0] += 1
            c[1] += int(hdr.len_urb)

            # DFU is the transport a DisplayLink flash uses; announce it the moment it starts.
            if hdr.xfer == 2 and hdr.type == ord('S'):
                d = is_dfu(setup)
                if d:
                    first = d["req"] not in dfu_counts
                    dfu_counts[d["req"]] = dfu_counts.get(d["req"], 0) + 1
                    if d["req"] == "DNLOAD":
                        dfu_bytes += d["wLength"]
                    if first:
                        event(f"DFU {d['req']} (bmRequestType {setup[0]:#04x} bRequest {d['num']}, "
                              f"iface {d['wIndex']}) on bus {hdr.busnum} dev {hdr.devnum}")
                        if d["req"] == "DETACH":
                            event("DFU_DETACH: the device is being switched into its BOOTLOADER. "
                                  "A re-enumeration follows. DO NOT UNPLUG.")
                        if d["req"] == "DNLOAD":
                            event("DFU_DNLOAD: THE FLASH HAS STARTED. Let it run to completion.")
                    elif d["req"] == "DNLOAD" and dfu_counts["DNLOAD"] % 16 == 0:
                        event(f"DFU_DNLOAD block {d['wValue']}, {fmt(dfu_bytes)} written so far")
                    if d["req"] == "DNLOAD" and d["wLength"] == 0 and dfu_counts["DNLOAD"] > 1:
                        event("zero-length DFU_DNLOAD = end of image, manifestation phase. "
                              "The device will reset itself. STILL DO NOT UNPLUG.")

            if store >= WINDOW:
                if not seen_magic and b"ELLA" in data:
                    seen_magic = True
                    event(f"container magic 'ELLA' on bus {hdr.busnum} dev {hdr.devnum} "
                          f"ep {hdr.epnum:#04x} — THIS IS A FIRMWARE IMAGE ON THE WIRE")
                if sigs:
                    hit = None
                    for off in range(0, store - WINDOW + 1, STRIDE):
                        hit = sigs.get(data[off:off + WINDOW])
                        if hit:
                            break
                    if hit:
                        name, imgoff = hit
                        first = name not in matched
                        matched[name] = matched.get(name, 0) + int(hdr.len_urb)
                        if first:
                            event(f"payload matches {name} (image offset {imgoff}) on bus "
                                  f"{hdr.busnum} dev {hdr.devnum} ep {hdr.epnum:#04x} — FLASH IN PROGRESS")
                            event("KEEP EVERYTHING PLUGGED IN. Let it finish.")
                        elif matched[name] // (256 * 1024) != (matched[name] - int(hdr.len_urb)) // (256 * 1024):
                            event(f"{name}: {fmt(matched[name])} of image bytes seen so far")
            if frames % 2000 == 0:
                out.flush()
    finally:
        print()
        out.flush()
        out.close()
        os.close(fd)
        total = sum(v[1] for v in counts.values())
        print(f"# DONE {frames} frames, {fmt(total)} -> {args.out}")
        print("# per-endpoint totals (bus.dev/ep):")
        for (b, d, e), v in sorted(counts.items(), key=lambda kv: -kv[1][1])[:20]:
            direction = "IN " if e & 0x80 else "OUT"
            print(f"#   {b}.{d:<3} ep {e:#04x} {direction} {v[0]:>8} frames  {fmt(v[1])}")
        print("#")
        if dfu_counts:
            print("# ★ DFU class requests seen: "
                  + ", ".join(f"{k}={v}" for k, v in sorted(dfu_counts.items())))
            print(f"# ★ DFU_DNLOAD payload total: {fmt(dfu_bytes)}")
            print("#   (the shipped images are 364 KB - 1.7 MB; a total in that range is a full flash)")
        else:
            print("# no DFU class request was seen. Whatever else happened, the standard firmware")
            print("#   download path did not run.")
        if matched:
            for name, n in matched.items():
                print(f"# ★ FIRMWARE SEEN: {name}  {fmt(n)} of matching payload on the wire")
            print("# ★ A flash was captured. Confirm with fw-scan.py and the bcdDevice diff.")
        elif seen_magic:
            print("# ★ 'ELLA' container magic seen but no image window matched — the device may")
            print("#   have been sent a DIFFERENT build than the installed .spkg. Keep everything.")
        else:
            print("# no firmware image matched. Either the enforcer accepted the device's existing")
            print("#   build, or the flash has not happened yet. Check the bcdDevice diff before")
            print("#   concluding, and see the 'second shot' section of docs/new-device-day.md.")
        ev.close()
        print(f"# events log: {ev_path}")


if __name__ == "__main__":
    sys.exit(main())
