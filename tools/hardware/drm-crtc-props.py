#!/usr/bin/env python3
"""List the KMS properties a driver exposes on each CRTC, with no libdrm and no modetest.

    tools/hardware/drm-crtc-props.py [--card /dev/dri/card2] [--driver vino]

Written because neither `modetest` nor `drm_info` is installed here, and "does the CRTC actually
advertise CTM and GAMMA_LUT" is the one question a colour-management change has to answer on real
hardware. Read-only: it opens the node, enumerates, and closes. It does not become DRM master, so
it is safe to run against a card a compositor is driving.
"""
import argparse, ctypes, fcntl, glob, os, struct, sys

DRM_IOCTL_BASE = ord("d")


def _iowr(nr, size):
    return (3 << 30) | (size << 16) | (DRM_IOCTL_BASE << 8) | nr


class CardRes(ctypes.Structure):
    _fields_ = [("fb_id_ptr", ctypes.c_uint64), ("crtc_id_ptr", ctypes.c_uint64),
                ("connector_id_ptr", ctypes.c_uint64), ("encoder_id_ptr", ctypes.c_uint64),
                ("count_fbs", ctypes.c_uint32), ("count_crtcs", ctypes.c_uint32),
                ("count_connectors", ctypes.c_uint32), ("count_encoders", ctypes.c_uint32),
                ("min_width", ctypes.c_uint32), ("max_width", ctypes.c_uint32),
                ("min_height", ctypes.c_uint32), ("max_height", ctypes.c_uint32)]


class ObjGetProps(ctypes.Structure):
    _fields_ = [("props_ptr", ctypes.c_uint64), ("prop_values_ptr", ctypes.c_uint64),
                ("count_props", ctypes.c_uint32), ("obj_id", ctypes.c_uint32),
                ("obj_type", ctypes.c_uint32)]


class GetProperty(ctypes.Structure):
    _fields_ = [("values_ptr", ctypes.c_uint64), ("enum_blob_ptr", ctypes.c_uint64),
                ("prop_id", ctypes.c_uint32), ("flags", ctypes.c_uint32),
                ("name", ctypes.c_char * 32),
                ("count_values", ctypes.c_uint32), ("count_enum_blobs", ctypes.c_uint32)]


class Version(ctypes.Structure):
    _fields_ = [("version_major", ctypes.c_int), ("version_minor", ctypes.c_int),
                ("version_patchlevel", ctypes.c_int),
                ("name_len", ctypes.c_size_t), ("name", ctypes.c_uint64),
                ("date_len", ctypes.c_size_t), ("date", ctypes.c_uint64),
                ("desc_len", ctypes.c_size_t), ("desc", ctypes.c_uint64)]


class CreateBlob(ctypes.Structure):
    _fields_ = [("data", ctypes.c_uint64), ("length", ctypes.c_uint32),
                ("blob_id", ctypes.c_uint32)]


class Atomic(ctypes.Structure):
    _fields_ = [("flags", ctypes.c_uint32), ("count_objs", ctypes.c_uint32),
                ("objs_ptr", ctypes.c_uint64), ("count_props_ptr", ctypes.c_uint64),
                ("props_ptr", ctypes.c_uint64), ("prop_values_ptr", ctypes.c_uint64),
                ("reserved", ctypes.c_uint64), ("user_data", ctypes.c_uint64)]


class ClientCap(ctypes.Structure):
    _fields_ = [("capability", ctypes.c_uint64), ("value", ctypes.c_uint64)]


def _iow(nr, size):
    return (1 << 30) | (size << 16) | (DRM_IOCTL_BASE << 8) | nr


DRM_IOCTL_VERSION = _iowr(0x00, ctypes.sizeof(Version))
DRM_IOCTL_SET_CLIENT_CAP = _iow(0x0D, ctypes.sizeof(ClientCap))
DRM_IOCTL_MODE_CREATEPROPBLOB = _iowr(0xBD, ctypes.sizeof(CreateBlob))
DRM_IOCTL_MODE_ATOMIC = _iowr(0xBC, ctypes.sizeof(Atomic))
DRM_CLIENT_CAP_ATOMIC = 3
DRM_IOCTL_MODE_GETRESOURCES = _iowr(0xA0, ctypes.sizeof(CardRes))
DRM_IOCTL_MODE_OBJ_GETPROPERTIES = _iowr(0xB9, ctypes.sizeof(ObjGetProps))
DRM_IOCTL_MODE_GETPROPERTY = _iowr(0xAA, ctypes.sizeof(GetProperty))
DRM_MODE_OBJECT_CRTC = 0xCCCCCCCC

# The property flags that say what kind of value this is.
DRM_MODE_PROP_RANGE = 1 << 1
DRM_MODE_PROP_BLOB = 1 << 4


def driver_name(fd):
    v = Version()
    fcntl.ioctl(fd, DRM_IOCTL_VERSION, v)
    if not v.name_len:
        return "?"
    buf = ctypes.create_string_buffer(v.name_len + 1)
    v.name = ctypes.cast(buf, ctypes.c_void_p).value
    v.date = 0
    v.desc = 0
    v.date_len = 0
    v.desc_len = 0
    fcntl.ioctl(fd, DRM_IOCTL_VERSION, v)
    return buf.value.decode("ascii", "replace")


def crtc_ids(fd):
    res = CardRes()
    fcntl.ioctl(fd, DRM_IOCTL_MODE_GETRESOURCES, res)
    n = res.count_crtcs
    if not n:
        return []
    arr = (ctypes.c_uint32 * n)()
    res.crtc_id_ptr = ctypes.cast(arr, ctypes.c_void_p).value
    res.count_fbs = res.count_connectors = res.count_encoders = 0
    res.fb_id_ptr = res.connector_id_ptr = res.encoder_id_ptr = 0
    fcntl.ioctl(fd, DRM_IOCTL_MODE_GETRESOURCES, res)
    return list(arr)[: res.count_crtcs]


_last_ids = []


def props_for(fd, obj_id):
    _last_ids.clear()
    g = ObjGetProps(obj_id=obj_id, obj_type=DRM_MODE_OBJECT_CRTC)
    fcntl.ioctl(fd, DRM_IOCTL_MODE_OBJ_GETPROPERTIES, g)
    n = g.count_props
    if not n:
        return []
    ids = (ctypes.c_uint32 * n)()
    vals = (ctypes.c_uint64 * n)()
    g.props_ptr = ctypes.cast(ids, ctypes.c_void_p).value
    g.prop_values_ptr = ctypes.cast(vals, ctypes.c_void_p).value
    fcntl.ioctl(fd, DRM_IOCTL_MODE_OBJ_GETPROPERTIES, g)
    out = []
    for i in range(g.count_props):
        p = GetProperty(prop_id=ids[i])
        try:
            fcntl.ioctl(fd, DRM_IOCTL_MODE_GETPROPERTY, p)
        except OSError:
            continue
        kind = "blob" if p.flags & DRM_MODE_PROP_BLOB else (
            "range" if p.flags & DRM_MODE_PROP_RANGE else "enum/other")
        out.append((p.name.decode("ascii", "replace"), kind, vals[i]))
        _last_ids.append(ids[i])
    return out


def set_ctm(fd, crtc_id, prop_id, matrix):
    """Attach a CTM blob to `crtc_id` through a real atomic commit."""
    cap = ClientCap(capability=DRM_CLIENT_CAP_ATOMIC, value=1)
    fcntl.ioctl(fd, DRM_IOCTL_SET_CLIENT_CAP, cap)
    raw = struct.pack("<9Q", *matrix)
    buf = ctypes.create_string_buffer(raw, len(raw))
    blob = CreateBlob(data=ctypes.cast(buf, ctypes.c_void_p).value, length=len(raw))
    fcntl.ioctl(fd, DRM_IOCTL_MODE_CREATEPROPBLOB, blob)
    objs = (ctypes.c_uint32 * 1)(crtc_id)
    counts = (ctypes.c_uint32 * 1)(1)
    props = (ctypes.c_uint32 * 1)(prop_id)
    vals = (ctypes.c_uint64 * 1)(blob.blob_id)
    a = Atomic(flags=0, count_objs=1,
               objs_ptr=ctypes.cast(objs, ctypes.c_void_p).value,
               count_props_ptr=ctypes.cast(counts, ctypes.c_void_p).value,
               props_ptr=ctypes.cast(props, ctypes.c_void_p).value,
               prop_values_ptr=ctypes.cast(vals, ctypes.c_void_p).value)
    fcntl.ioctl(fd, DRM_IOCTL_MODE_ATOMIC, a)
    return blob.blob_id


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--set-ctm", action="store_true",
                    help="commit a CTM blob to each CRTC and read it back (needs DRM master)")
    ap.add_argument("--card", help="a /dev/dri/cardN node; default: every card")
    ap.add_argument("--driver", help="only report cards whose driver name matches")
    args = ap.parse_args()

    cards = [args.card] if args.card else sorted(glob.glob("/dev/dri/card*"))
    want = {"CTM", "GAMMA_LUT", "GAMMA_LUT_SIZE", "DEGAMMA_LUT", "DEGAMMA_LUT_SIZE"}
    rc = 0
    for path in cards:
        try:
            fd = os.open(path, os.O_RDWR)
        except OSError as e:
            print(f"{path}: {e}")
            continue
        try:
            name = driver_name(fd)
            if args.driver and name != args.driver:
                continue
            print(f"\n== {path}  driver={name}")
            ids = crtc_ids(fd)
            if not ids:
                print("   (no CRTCs)")
            for cid in ids:
                p = props_for(fd, cid)
                names = {n for n, _, _ in p}
                print(f"   CRTC {cid}: {len(p)} properties")
                for n, kind, v in p:
                    mark = "  <-- colour management" if n in want else ""
                    print(f"      {n:<20} {kind:<10} = {v}{mark}")
                if args.set_ctm and "CTM" in names:
                    pid = next(i for i, (n, _, _) in zip(
                        [x for x in _last_ids], p) if n == "CTM")
                    # Identity except green halved: S31.32, 1.0 = 1<<32.
                    m = [1 << 32, 0, 0, 0, 1 << 31, 0, 0, 0, 1 << 32]
                    try:
                        bid = set_ctm(fd, cid, pid, m)
                        after = {n: v for n, _, v in props_for(fd, cid)}
                        ok = after.get("CTM", 0) == bid and bid != 0
                        print(f"      set CTM -> blob {bid}, reads back {after.get('CTM')}"
                              f"  {'✅ accepted' if ok else '❌ not retained'}")
                        if not ok:
                            rc = 1
                    except OSError as e:
                        print(f"      set CTM FAILED: {e}")
                        rc = 1

                missing = {"CTM", "GAMMA_LUT", "GAMMA_LUT_SIZE"} - names
                if missing:
                    print(f"      MISSING: {', '.join(sorted(missing))}")
                    rc = 1
                else:
                    print("      ✅ CTM + GAMMA_LUT + GAMMA_LUT_SIZE all present")
        finally:
            os.close(fd)
    return rc


if __name__ == "__main__":
    sys.exit(main())
