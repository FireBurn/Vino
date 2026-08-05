#!/usr/bin/env python3
"""Set a mode on one connector and hold it, with no libdrm, no modetest and no compositor.

    sudo tools/hardware/drm-setmode.py --card /dev/dri/card2 --connector DP-3 --list
    sudo tools/hardware/drm-setmode.py --card /dev/dri/card2 --connector DP-3 --mode 1920x1080@60
    sudo tools/hardware/drm-setmode.py --card /dev/dri/card2 --connector DP-3   # preferred mode

Written because "does this panel light up" must not be answered through KWin. A compositor that
declines to open a card, or that re-probes on its own schedule, turns a driver question into a
session question -- and this card has a history of exactly that. This takes DRM master itself,
paints a recognisable pattern into a dumb buffer, sets the mode, and holds it until the timeout so
the picture can be looked at.

It refuses to run if anything else holds the card, because taking master from a live compositor is
not a thing to do by accident.

Read-write, obviously: it drives real hardware. The mode is dropped when the process exits, which
also frees the framebuffer and disables the pipe.
"""
import argparse, ctypes, fcntl, mmap, os, struct, sys, time

DRM_IOCTL_BASE = ord("d")


def _io(nr):
    return (DRM_IOCTL_BASE << 8) | nr


def _iowr(nr, size):
    return (3 << 30) | (size << 16) | (DRM_IOCTL_BASE << 8) | nr


class CardRes(ctypes.Structure):
    _fields_ = [("fb_id_ptr", ctypes.c_uint64), ("crtc_id_ptr", ctypes.c_uint64),
                ("connector_id_ptr", ctypes.c_uint64), ("encoder_id_ptr", ctypes.c_uint64),
                ("count_fbs", ctypes.c_uint32), ("count_crtcs", ctypes.c_uint32),
                ("count_connectors", ctypes.c_uint32), ("count_encoders", ctypes.c_uint32),
                ("min_width", ctypes.c_uint32), ("max_width", ctypes.c_uint32),
                ("min_height", ctypes.c_uint32), ("max_height", ctypes.c_uint32)]


class ModeInfo(ctypes.Structure):
    _fields_ = [("clock", ctypes.c_uint32),
                ("hdisplay", ctypes.c_uint16), ("hsync_start", ctypes.c_uint16),
                ("hsync_end", ctypes.c_uint16), ("htotal", ctypes.c_uint16),
                ("hskew", ctypes.c_uint16),
                ("vdisplay", ctypes.c_uint16), ("vsync_start", ctypes.c_uint16),
                ("vsync_end", ctypes.c_uint16), ("vtotal", ctypes.c_uint16),
                ("vscan", ctypes.c_uint16),
                ("vrefresh", ctypes.c_uint32), ("flags", ctypes.c_uint32),
                ("type", ctypes.c_uint32), ("name", ctypes.c_char * 32)]


class GetConnector(ctypes.Structure):
    _fields_ = [("encoders_ptr", ctypes.c_uint64), ("modes_ptr", ctypes.c_uint64),
                ("props_ptr", ctypes.c_uint64), ("prop_values_ptr", ctypes.c_uint64),
                ("count_modes", ctypes.c_uint32), ("count_props", ctypes.c_uint32),
                ("count_encoders", ctypes.c_uint32), ("encoder_id", ctypes.c_uint32),
                ("connector_id", ctypes.c_uint32), ("connector_type", ctypes.c_uint32),
                ("connector_type_id", ctypes.c_uint32), ("connection", ctypes.c_uint32),
                ("mm_width", ctypes.c_uint32), ("mm_height", ctypes.c_uint32),
                ("subpixel", ctypes.c_uint32), ("pad", ctypes.c_uint32)]


class GetEncoder(ctypes.Structure):
    _fields_ = [("encoder_id", ctypes.c_uint32), ("encoder_type", ctypes.c_uint32),
                ("crtc_id", ctypes.c_uint32), ("possible_crtcs", ctypes.c_uint32),
                ("possible_clones", ctypes.c_uint32)]


class CreateDumb(ctypes.Structure):
    _fields_ = [("height", ctypes.c_uint32), ("width", ctypes.c_uint32),
                ("bpp", ctypes.c_uint32), ("flags", ctypes.c_uint32),
                ("handle", ctypes.c_uint32), ("pitch", ctypes.c_uint32),
                ("size", ctypes.c_uint64)]


class MapDumb(ctypes.Structure):
    _fields_ = [("handle", ctypes.c_uint32), ("pad", ctypes.c_uint32),
                ("offset", ctypes.c_uint64)]


class FbCmd(ctypes.Structure):
    _fields_ = [("fb_id", ctypes.c_uint32), ("width", ctypes.c_uint32),
                ("height", ctypes.c_uint32), ("pitch", ctypes.c_uint32),
                ("bpp", ctypes.c_uint32), ("depth", ctypes.c_uint32),
                ("handle", ctypes.c_uint32)]


class Crtc(ctypes.Structure):
    _fields_ = [("set_connectors_ptr", ctypes.c_uint64), ("count_connectors", ctypes.c_uint32),
                ("crtc_id", ctypes.c_uint32), ("fb_id", ctypes.c_uint32),
                ("x", ctypes.c_uint32), ("y", ctypes.c_uint32),
                ("gamma_size", ctypes.c_uint32), ("mode_valid", ctypes.c_uint32),
                ("mode", ModeInfo)]


IOCTL_SET_MASTER = _io(0x1e)
IOCTL_DROP_MASTER = _io(0x1f)
IOCTL_GETRESOURCES = _iowr(0xA0, ctypes.sizeof(CardRes))
IOCTL_SETCRTC = _iowr(0xA2, ctypes.sizeof(Crtc))
IOCTL_GETENCODER = _iowr(0xA6, ctypes.sizeof(GetEncoder))
IOCTL_GETCONNECTOR = _iowr(0xA7, ctypes.sizeof(GetConnector))
IOCTL_ADDFB = _iowr(0xAE, ctypes.sizeof(FbCmd))
IOCTL_CREATE_DUMB = _iowr(0xB2, ctypes.sizeof(CreateDumb))
IOCTL_MAP_DUMB = _iowr(0xB3, ctypes.sizeof(MapDumb))

# Only the types this driver uses; anything else prints as its number.
CONNECTOR_TYPE = {10: "DP", 11: "eDP", 14: "HDMI-A", 15: "HDMI-B", 16: "Virtual", 17: "DSI"}


def connector_name(c):
    return "%s-%d" % (CONNECTOR_TYPE.get(c.connector_type, str(c.connector_type)),
                      c.connector_type_id)


def get_connector(fd, cid):
    """Two-pass ioctl: the first call reports the counts, the second fills the arrays."""
    c = GetConnector(connector_id=cid)
    fcntl.ioctl(fd, IOCTL_GETCONNECTOR, c)
    modes = (ModeInfo * c.count_modes)()
    encoders = (ctypes.c_uint32 * c.count_encoders)()
    c.modes_ptr = ctypes.addressof(modes) if c.count_modes else 0
    c.encoders_ptr = ctypes.addressof(encoders) if c.count_encoders else 0
    # Asking for modes without also asking for properties is fine, but the counts must be exact
    # or the kernel refuses the call.
    c.count_props = 0
    c.props_ptr = 0
    c.prop_values_ptr = 0
    fcntl.ioctl(fd, IOCTL_GETCONNECTOR, c)
    return c, list(modes), list(encoders)


def mode_label(m):
    return "%dx%d@%d" % (m.hdisplay, m.vdisplay, m.vrefresh)


def paint(buf, width, height, pitch):
    """Colour bars with a white frame and a centre cross.

    Chosen so a photograph answers three questions at once: whether anything arrived, whether the
    geometry is right (the frame touches all four edges), and whether the colour channels are in
    the order the driver thinks (bars run R, G, B, ...).
    """
    bars = [0xFF0000, 0x00FF00, 0x0000FF, 0xFFFF00, 0x00FFFF, 0xFF00FF, 0xFFFFFF, 0x000000]
    bw = max(1, width // len(bars))
    row = bytearray(pitch)
    for y in range(height):
        edge = y < 4 or y >= height - 4
        for x in range(width):
            if edge or x < 4 or x >= width - 4:
                px = 0xFFFFFF
            elif abs(x - width // 2) < 2 or abs(y - height // 2) < 2:
                px = 0xFFFFFF
            else:
                px = bars[min(x // bw, len(bars) - 1)]
            struct.pack_into("<I", row, x * 4, px | 0xFF000000)
        buf[y * pitch:y * pitch + pitch] = row


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--card", default="/dev/dri/card2")
    ap.add_argument("--connector", help="e.g. DP-3; default is the first connected one")
    ap.add_argument("--mode", help="WxH@R, or an index into --list; default is preferred")
    ap.add_argument("--list", action="store_true", help="list connectors and modes, set nothing")
    ap.add_argument("--seconds", type=float, default=30.0, help="how long to hold the mode")
    args = ap.parse_args()

    fd = os.open(args.card, os.O_RDWR | os.O_CLOEXEC)

    res = CardRes()
    fcntl.ioctl(fd, IOCTL_GETRESOURCES, res)
    crtcs = (ctypes.c_uint32 * res.count_crtcs)()
    conns = (ctypes.c_uint32 * res.count_connectors)()
    encs = (ctypes.c_uint32 * res.count_encoders)()
    res.crtc_id_ptr = ctypes.addressof(crtcs)
    res.connector_id_ptr = ctypes.addressof(conns)
    res.encoder_id_ptr = ctypes.addressof(encs)
    res.count_fbs = 0
    res.fb_id_ptr = 0
    fcntl.ioctl(fd, IOCTL_GETRESOURCES, res)

    chosen = None
    for cid in conns:
        c, modes, encoders = get_connector(fd, cid)
        name = connector_name(c)
        state = {1: "connected", 2: "disconnected"}.get(c.connection, "unknown")
        if args.list:
            print("%s (id %d): %s, %d mode(s)" % (name, cid, state, len(modes)))
            for i, m in enumerate(modes):
                print("  %2d  %-14s %6d kHz  %d/%d/%d/%d %d/%d/%d/%d%s"
                      % (i, mode_label(m), m.clock, m.hdisplay, m.hsync_start, m.hsync_end,
                         m.htotal, m.vdisplay, m.vsync_start, m.vsync_end, m.vtotal,
                         "  [preferred]" if m.type & 8 else ""))
        if args.connector:
            if name == args.connector:
                chosen = (c, modes, encoders)
        elif chosen is None and c.connection == 1 and modes:
            chosen = (c, modes, encoders)
    if args.list:
        return 0
    if not chosen:
        sys.exit("no such connector, or nothing connected with modes")

    conn, modes, encoders = chosen
    if not modes:
        sys.exit("%s has no modes" % connector_name(conn))

    if args.mode is None:
        mode = next((m for m in modes if m.type & 8), modes[0])
    elif args.mode.isdigit():
        mode = modes[int(args.mode)]
    else:
        want = args.mode if "@" in args.mode else args.mode + "@60"
        mode = next((m for m in modes if mode_label(m) == want), None)
        if mode is None:
            sys.exit("no mode %s; try --list" % want)

    # possible_crtcs is a bitmask of indices into the resources' CRTC list, so a head can only be
    # driven by its own pipe. Picking the first free CRTC instead would silently drive head 0.
    crtc_id = None
    for eid in encoders:
        e = GetEncoder(encoder_id=eid)
        fcntl.ioctl(fd, IOCTL_GETENCODER, e)
        for i, cid in enumerate(crtcs):
            if e.possible_crtcs & (1 << i):
                crtc_id = cid
                break
        if crtc_id:
            break
    if crtc_id is None:
        sys.exit("no CRTC can drive %s" % connector_name(conn))

    print("%s: setting %s (%d kHz) on CRTC %d"
          % (connector_name(conn), mode_label(mode), mode.clock, crtc_id))

    try:
        fcntl.ioctl(fd, IOCTL_SET_MASTER, 0)
    except OSError as e:
        sys.exit("cannot become DRM master (%s) -- something else is driving this card" % e)

    dumb = CreateDumb(width=mode.hdisplay, height=mode.vdisplay, bpp=32)
    fcntl.ioctl(fd, IOCTL_CREATE_DUMB, dumb)
    fb = FbCmd(width=mode.hdisplay, height=mode.vdisplay, pitch=dumb.pitch,
               bpp=32, depth=24, handle=dumb.handle)
    fcntl.ioctl(fd, IOCTL_ADDFB, fb)

    mp = MapDumb(handle=dumb.handle)
    fcntl.ioctl(fd, IOCTL_MAP_DUMB, mp)
    buf = mmap.mmap(fd, dumb.size, mmap.MAP_SHARED, mmap.PROT_READ | mmap.PROT_WRITE,
                    offset=mp.offset)
    paint(buf, mode.hdisplay, mode.vdisplay, dumb.pitch)

    cid = ctypes.c_uint32(conn.connector_id)
    req = Crtc(set_connectors_ptr=ctypes.addressof(cid), count_connectors=1,
               crtc_id=crtc_id, fb_id=fb.fb_id, mode_valid=1, mode=mode)
    fcntl.ioctl(fd, IOCTL_SETCRTC, req)
    print("mode set. Holding for %.0f s -- look at the screen." % args.seconds)

    try:
        time.sleep(args.seconds)
    except KeyboardInterrupt:
        pass
    print("dropping the mode")
    buf.close()
    fcntl.ioctl(fd, IOCTL_DROP_MASTER, 0)
    os.close(fd)
    return 0


if __name__ == "__main__":
    sys.exit(main())
