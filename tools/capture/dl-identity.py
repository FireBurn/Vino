#!/usr/bin/env python3
"""Place an unfamiliar DisplayLink device without loading a driver or running DLM.

vino logs the vendor identity blob at probe, but its USB id table is a single exact match
(17e9:6006), so it never binds anything else -- on new hardware that log is unavailable exactly
when it would be most useful. This does the same reads from userspace over usbfs, with no
third-party modules: descriptors and endpoints from sysfs, and the 16-byte vendor identity blob
via the same control request vino issues (bRequest 0xfe, bmRequestType 0xc1, wIndex 1, 16 bytes).

The blob's ASCII tail names the platform, and the platform names the firmware package that
targets it:

    "RidgeDoc" -> ridge-dock-release.spkg    (Dell D6000, 17e9:6006)
    "EllaDock" -> ella-dock-release.spkg     (17e9:4300 / 4301)
    "NavaDock" -> navarro-dock-release.spkg  (DL-7400 class, e.g. WAVLINK DL7400)

    sudo tools/capture/dl-identity.py [--vid 17e9] [--json out.json]

Read-only apart from claiming interface 0 for the duration of one control transfer, which is
required by usbfs for an interface-recipient request and is released immediately.
"""
import argparse, ctypes, fcntl, json, os, sys

VID_DEFAULT = "17e9"

# usbfs ioctls. usbdevfs_ctrltransfer is {u8,u8,u16,u16,u16,u32,ptr} = 24 bytes with alignment.
USBDEVFS_CONTROL = (3 << 30) | (24 << 16) | (ord('U') << 8) | 0
USBDEVFS_CLAIMINTERFACE = (2 << 30) | (4 << 16) | (ord('U') << 8) | 15
USBDEVFS_RELEASEINTERFACE = (2 << 30) | (4 << 16) | (ord('U') << 8) | 16


class CtrlTransfer(ctypes.Structure):
    _fields_ = [("bRequestType", ctypes.c_uint8), ("bRequest", ctypes.c_uint8),
                ("wValue", ctypes.c_uint16), ("wIndex", ctypes.c_uint16),
                ("wLength", ctypes.c_uint16), ("timeout", ctypes.c_uint32),
                ("data", ctypes.c_void_p)]


def rd(path, default=""):
    try:
        with open(path) as f:
            return f.read().strip()
    except OSError:
        return default


def control_in(devnode, brequest, brequesttype, wvalue, windex, length, iface=0, timeout=2000):
    """One vendor control IN. Returns bytes, or raises OSError."""
    fd = os.open(devnode, os.O_RDWR)
    claimed = False
    try:
        # usbfs requires the interface to be claimed for an interface-recipient request.
        if (brequesttype & 0x1f) == 0x01:
            try:
                fcntl.ioctl(fd, USBDEVFS_CLAIMINTERFACE, ctypes.c_uint(iface))
                claimed = True
            except OSError as e:
                raise OSError(f"cannot claim interface {iface}: {e} "
                              f"(a kernel driver or DLM is holding it)") from e
        buf = ctypes.create_string_buffer(length)
        req = CtrlTransfer(brequesttype, brequest, wvalue, windex, length, timeout,
                           ctypes.cast(buf, ctypes.c_void_p))
        n = fcntl.ioctl(fd, USBDEVFS_CONTROL, req)
        return bytes(buf[:max(0, n)])
    finally:
        if claimed:
            try:
                fcntl.ioctl(fd, USBDEVFS_RELEASEINTERFACE, ctypes.c_uint(iface))
            except OSError:
                pass
        os.close(fd)


def ascii_tail(blob):
    return "".join(chr(b) if 0x20 <= b < 0x7f else "." for b in blob)


def walk_descriptors(path):
    """Yield (bDescriptorType, raw) over the device's whole descriptor set.

    /sys/bus/usb/devices/*/descriptors is the cached device + config descriptors, readable with no
    driver interaction at all. Both things we most want live in there as class/vendor descriptors:
    the DFU functional descriptor (type 0x21) and DisplayLink's 16-byte identity blob (type 0x40).
    Reading them here rather than over a control transfer means it works even when a driver already
    owns the interface -- which is exactly the case that defeated the control-transfer path.
    """
    try:
        with open(path, "rb") as f:
            buf = f.read()
    except OSError:
        return
    i = 0
    while i + 2 <= len(buf):
        blen = buf[i]
        if blen < 2 or i + blen > len(buf):
            break
        yield buf[i + 1], buf[i:i + blen]
        i += blen


DFU_ATTR = [(0x08, "Will Detach (no USB reset needed after DFU_DETACH)"),
            (0x04, "Manifestation Tolerant"),
            (0x02, "Upload Supported"),
            (0x01, "Download Supported")]


def parse_dfu(raw):
    if len(raw) < 9:
        return None
    attrs = raw[2]
    return {
        "bmAttributes": attrs,
        "attrs": [t for bit, t in DFU_ATTR if attrs & bit],
        "wDetachTimeout": int.from_bytes(raw[3:5], "little"),
        "wTransferSize": int.from_bytes(raw[5:7], "little"),
        "bcdDFUVersion": f"{raw[8]}.{raw[7]:02x}",
    }


# Tails measured from the shipped packages' own build descriptors: RidgeDocOW, EllaDockOW,
# NavaDockOW, FflyMoniOW. The blob carries the first eight characters.
PLATFORM = {"RidgeDoc": "ridge-dock-release.spkg     (Dell D6000 class, DL-6xxx)",
            "EllaDock": "ella-dock-release.spkg      (17e9:4300/4301 class)",
            "NavaDock": "navarro-dock-release.spkg   (DL-7400 class, DL-7000 generation)",
            "FflyMoni": "firefly-monitor-release.spkg (integrated monitor)"}


def describe_platform(text):
    for k, v in PLATFORM.items():
        if k in text:
            return k, v
    return None, None


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--vid", default=VID_DEFAULT)
    ap.add_argument("--json", help="also write the findings as JSON")
    args = ap.parse_args()

    found = []
    for d in sorted(os.listdir("/sys/bus/usb/devices")):
        base = f"/sys/bus/usb/devices/{d}"
        if rd(f"{base}/idVendor").lower() != args.vid.lower():
            continue
        dev = {
            "sysfs": d,
            "idVendor": rd(f"{base}/idVendor"),
            "idProduct": rd(f"{base}/idProduct"),
            "bcdDevice": rd(f"{base}/bcdDevice"),
            "bcdUSB": rd(f"{base}/version"),
            "speed": rd(f"{base}/speed"),
            "manufacturer": rd(f"{base}/manufacturer"),
            "product": rd(f"{base}/product"),
            "serial": rd(f"{base}/serial"),
            "busnum": rd(f"{base}/busnum"),
            "devnum": rd(f"{base}/devnum"),
            "interfaces": [],
        }
        for i in sorted(os.listdir(base)):
            ip = f"{base}/{i}"
            if ":" not in i or not os.path.isdir(ip):
                continue
            drv = ""
            try:
                drv = os.path.basename(os.readlink(f"{ip}/driver"))
            except OSError:
                pass
            eps = []
            for e in sorted(os.listdir(ip)):
                if not e.startswith("ep_"):
                    continue
                eps.append({
                    "addr": rd(f"{ip}/{e}/bEndpointAddress"),
                    "type": rd(f"{ip}/{e}/type"),
                    "maxp": rd(f"{ip}/{e}/wMaxPacketSize"),
                    "dir": rd(f"{ip}/{e}/direction"),
                })
            dev["interfaces"].append({
                "name": i,
                "class": rd(f"{ip}/bInterfaceClass"),
                "subclass": rd(f"{ip}/bInterfaceSubClass"),
                "protocol": rd(f"{ip}/bInterfaceProtocol"),
                "driver": drv,
                "endpoints": eps,
            })
        found.append((base, dev))

    if not found:
        print(f"no {args.vid}:* device attached")
        return 1

    out = []
    for base, dev in found:
        print(f"\n== {dev['sysfs']}  {dev['idVendor']}:{dev['idProduct']}  "
              f"bcdDevice {dev['bcdDevice']} (firmware revision)  bcdUSB {dev['bcdUSB']}  "
              f"speed {dev['speed']} Mbps")
        print(f"   {dev['manufacturer']} {dev['product']}"
              + (f"  serial {dev['serial']}" if dev["serial"] else ""))
        dl3 = False
        for i in dev["interfaces"]:
            # udl matches 17e9 + class ff + sub 00 + proto 00; DL3 control is proto 03.
            tag = ""
            if i["class"] == "ff" and i["subclass"] == "00":
                if i["protocol"] == "03":
                    tag = "   <- DL3 control interface (vino's problem)"
                    dl3 = True
                elif i["protocol"] == "00":
                    tag = "   <- pre-DL3: in-tree udl already matches this"
            print(f"   iface {i['name']} class={i['class']} sub={i['subclass']} "
                  f"proto={i['protocol']} driver={i['driver'] or '-'}{tag}")
            for e in i["endpoints"]:
                note = {"02": "  <- DL3 control OUT", "84": "  <- DL3 control IN",
                        "08": "  <- video", "09": "  <- video", "0a": "  <- video",
                        "0b": "  <- video"}.get(e["addr"].lower().lstrip("0x").zfill(2), "")
                print(f"      ep {e['addr']} {e['type']:<9} maxp {e['maxp']} {e['dir']}{note}")

        # ---- DFU. This decides the SHAPE of a firmware flash before it ever happens.
        dev["dfu"] = None
        for dtype, raw in walk_descriptors(f"{base}/descriptors"):
            if dtype == 0x21 and len(raw) == 9:
                dev["dfu"] = parse_dfu(raw)
                break
        if dev["dfu"]:
            d = dev["dfu"]
            print(f"   ★ DFU functional descriptor: version {d['bcdDFUVersion']}, "
                  f"wTransferSize {d['wTransferSize']} B, detach timeout {d['wDetachTimeout']} ms")
            print(f"     attributes {d['bmAttributes']:#04x}: {', '.join(d['attrs']) or 'none'}")
            proto = [i["protocol"] for i in dev["interfaces"]
                     if i["class"] == "fe" and i["subclass"] == "01"]
            mode = {"01": "RUNTIME (normal operation; DFU_DETACH switches it)",
                    "02": "DFU MODE -- the device is in its bootloader RIGHT NOW"}.get(
                        proto[0] if proto else "", "unknown")
            print(f"     interface protocol {proto[0] if proto else '?'} => {mode}")
            print("     a flash will therefore be CLASS control transfers on that interface:")
            print("       bmRequestType 0x21  bRequest 1 (DNLOAD)  wValue = block number")
            print(f"       up to {d['wTransferSize']} B per block, polled with GETSTATUS (0xa1/3)")
            if not (d["bmAttributes"] & 0x08):
                print("     bitWillDetach is CLEAR: the host issues a USB RESET after DFU_DETACH,")
                print("     so the device RE-ENUMERATES, possibly under a different product id.")
                print("     => capture the whole bus. A device-filtered capture loses the flash.")

        # ---- the vendor identity blob, from the descriptor set (no claim, always works)
        dev["identity_hex"] = None
        dev["identity_text"] = None
        dev["platform"] = None
        blob = None
        for dtype, raw in walk_descriptors(f"{base}/descriptors"):
            if dtype == 0x40 and len(raw) == 16:
                blob = raw
                break
        src = "config descriptor type 0x40"
        if blob is None:
            # Fall back to the control read vino performs. Needs the interface free.
            node = f"/dev/bus/usb/{int(dev['busnum']):03d}/{int(dev['devnum']):03d}"
            try:
                blob = control_in(node, 0xfe, 0xc1, 0, 1, 16)
                src = "control read 0xc1/0xfe"
            except OSError as e:
                print(f"   device identity unavailable ({e})")
        if blob is not None:
            text = ascii_tail(blob)
            dev["identity_hex"] = blob.hex(" ")
            dev["identity_text"] = text
            print(f"   device identity = [{blob.hex(' ')}] \"{text}\"   [{src}]")
            key, pkg = describe_platform(text)
            if key:
                dev["platform"] = key
                print(f"   platform: {key}  ->  DLM would flash it from {pkg}")
                print("   ** compare bcdDevice before/after: that is the proof a flash happened **")
            else:
                print("   platform: UNKNOWN tail -- record it, this is a new codename")

        if dl3:
            print("   => DL3-family. vino territory; udl is not the answer for this part.")
        out.append(dev)

    if args.json:
        with open(args.json, "w") as f:
            json.dump(out, f, indent=1)
        print(f"\nwrote {args.json}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
