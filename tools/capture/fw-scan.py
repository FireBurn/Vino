#!/usr/bin/env python3
"""Find and characterise a firmware flash in a first-contact capture.

    tools/capture/fw-scan.py <capture-dir-or-file> [--spkg-dir /opt/displaylink]

Accepts a directory produced by capture-firstcontact.sh (it will read every .mon in it), or a
single .mon file. It does NOT need tshark: the .mon recording written by fw-watch.py is the
authoritative input, and pcapng files are left for Wireshark.

What it reports, in the order it matters:

  1. whether any payload matches a shipped *-release.spkg, and how much of the image was seen
  2. the endpoint and request shape carrying it -- the DFU transport
  3. the chunking: payload size distribution and whether image offsets advance monotonically
  4. gaps: image ranges never seen, which is how you know whether the capture is complete
  5. bulk/control byte totals per endpoint, so a big transfer that matches NOTHING still surfaces
     (a device may be sent a build that is not the one installed here)

A DisplayLink flash has never been recorded, so treat "no match" as a finding rather than a
failure, and read it next to the bcdDevice diff.
"""
import argparse, glob, os, struct, sys
from collections import defaultdict

MAGIC = b"VFW2"
REC = "<QBBBBHBBqiiII8s"
REC_SZ = struct.calcsize(REC)
REC_OLD = "<BBBBHqiiII"
REC_OLD_SZ = struct.calcsize(REC_OLD)
WINDOW = 32
STRIDE = 16

DFU_REQ = {0: "DETACH", 1: "DNLOAD", 2: "UPLOAD", 3: "GETSTATUS", 4: "CLRSTATUS",
           5: "GETSTATE", 6: "ABORT"}
DFU_STATE = {0: "appIDLE", 1: "appDETACH", 2: "dfuIDLE", 3: "dfuDNLOAD-SYNC",
             4: "dfuDNBUSY", 5: "dfuDNLOAD-IDLE", 6: "dfuMANIFEST-SYNC", 7: "dfuMANIFEST",
             8: "dfuMANIFEST-WAIT-RESET", 9: "dfuUPLOAD-IDLE", 10: "dfuERROR"}
DFU_STATUS = {0: "OK", 1: "errTARGET", 2: "errFILE", 3: "errWRITE", 4: "errERASE",
              5: "errCHECK_ERASED", 6: "errPROG", 7: "errVERIFY", 8: "errADDRESS",
              9: "errNOTDONE", 10: "errFIRMWARE", 15: "errUNKNOWN"}


def parse_setup(setup):
    if len(setup) < 8:
        return None
    bm, br = setup[0], setup[1]
    return {"bm": bm, "br": br,
            "class_iface": (bm & 0x60) == 0x20 and (bm & 0x1f) == 0x01,
            "dir_in": bool(bm & 0x80),
            "wValue": int.from_bytes(setup[2:4], "little"),
            "wIndex": int.from_bytes(setup[4:6], "little"),
            "wLength": int.from_bytes(setup[6:8], "little")}


def load_spkgs(d):
    imgs = {}
    for path in sorted(glob.glob(os.path.join(d, "*-release.spkg"))):
        imgs[os.path.basename(path)] = open(path, "rb").read()
    return imgs


def build_index(imgs):
    """window bytes -> (image name, offset). First writer wins; collisions are vanishingly rare."""
    idx = {}
    for name, blob in imgs.items():
        for off in range(0, max(0, len(blob) - WINDOW), STRIDE):
            idx.setdefault(blob[off:off + WINDOW], (name, off))
    return idx


def locate(blob, data):
    """Exact offset of `data` inside `blob`, or None. Anchored by the first window hit."""
    if len(data) < WINDOW:
        return None
    i = blob.find(data)
    return i if i >= 0 else None


def read_mon(path):
    """Read a fw-watch.py recording, or an older setup-less one from capture-usbmon-session.py."""
    with open(path, "rb") as f:
        head = f.read(4)
        new = head == MAGIC
        if not new:
            f.seek(0)
        while True:
            hdr = f.read(4)
            if len(hdr) < 4:
                return
            (n,) = struct.unpack("<I", hdr)
            body = f.read(n)
            if new:
                if len(body) < REC_SZ:
                    return
                (urb_id, typ, xfer, epnum, devnum, busnum, flag_setup, _pad, ts_sec, ts_usec,
                 status, len_urb, stored, setup) = struct.unpack(REC, body[:REC_SZ])
                off = REC_SZ
            else:
                if len(body) < REC_OLD_SZ:
                    return
                (typ, xfer, epnum, devnum, busnum, ts_sec, ts_usec, status,
                 len_urb, stored) = struct.unpack(REC_OLD, body[:REC_OLD_SZ])
                urb_id, setup, flag_setup, off = 0, b"", 0xff, REC_OLD_SZ
            yield {
                "id": urb_id,
                "type": chr(typ), "xfer": xfer, "ep": epnum, "dev": devnum, "bus": busnum,
                "ts": ts_sec + ts_usec / 1e6, "status": status, "len_urb": len_urb,
                "setup": setup, "flag_setup": flag_setup,
                "data": body[off:off + stored],
            }


def fmt(n):
    if n >= 1 << 20:
        return f"{n / (1<<20):.2f} MiB"
    if n >= 1 << 10:
        return f"{n / (1<<10):.1f} KiB"
    return f"{n} B"


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("target")
    ap.add_argument("--spkg-dir", default=None,
                    help="default: the capture dir itself, then /opt/displaylink")
    args = ap.parse_args()

    if os.path.isdir(args.target):
        mons = sorted(glob.glob(os.path.join(args.target, "*.mon")))
        spkg_dir = args.spkg_dir or (args.target if glob.glob(
            os.path.join(args.target, "*-release.spkg")) else "/opt/displaylink")
    else:
        mons = [args.target]
        spkg_dir = args.spkg_dir or "/opt/displaylink"

    if not mons:
        print(f"no .mon recording under {args.target}")
        return 2

    imgs = load_spkgs(spkg_dir)
    idx = build_index(imgs)
    print(f"== firmware images from {spkg_dir}")
    for name, blob in imgs.items():
        print(f"   {name:<34} {len(blob):>9} B  magic {blob[:8].hex(' ')}")
    if not imgs:
        print("   (none -- payload attribution disabled; byte totals only)")

    ep_bytes = defaultdict(int)
    ep_frames = defaultdict(int)
    hits = defaultdict(list)       # image name -> [(ts, ep, img_off, length)]
    magic_frames = []
    dfu = []                       # [ts, req, wValue, wIndex, wLength, dir_in, data]
    pending = {}                   # urb_id -> the dfu row awaiting its 'C' reply data
    total_frames = 0
    t0 = None

    for path in mons:
        print(f"\n== reading {path}")
        for r in read_mon(path):
            total_frames += 1
            if t0 is None and r["ts"]:
                t0 = r["ts"]
            key = (r["bus"], r["dev"], r["ep"], r["xfer"])
            ep_bytes[key] += r["len_urb"]
            ep_frames[key] += 1
            # mon_bin puts the setup packet on the 'S' record only, so a control IN reply's DATA
            # arrives on a later 'C' record with no setup at all. Pair them by URB id, or every
            # device reply -- including the DFU_GETSTATUS that would report a write error -- is
            # unattributable.
            if r["xfer"] == 2 and r["type"] == "S" and r["setup"]:
                s = parse_setup(r["setup"])
                # USB Audio Class SET_CUR is also 0x21/0x01, identical to DFU_DNLOAD; audio puts
                # (entity << 8) | interface in wIndex where DFU puts a bare interface number, and
                # the request directions are fixed. Without both tests a dock with UAC interfaces
                # reports a firmware download with nonsense block numbers.
                if s and s["class_iface"] and s["br"] in DFU_REQ \
                        and not (s["wIndex"] >> 8) and s["wIndex"] <= 8 \
                        and s["dir_in"] == (s["br"] in (2, 3, 5)):
                    rec = [r["ts"], DFU_REQ[s["br"]], s["wValue"], s["wIndex"],
                           s["wLength"], s["dir_in"], r["data"]]
                    dfu.append(rec)
                    if s["dir_in"]:
                        pending[r["id"]] = rec
            elif r["xfer"] == 2 and r["type"] == "C" and r["id"] in pending:
                pending.pop(r["id"])[6] = r["data"]
            d = r["data"]
            if len(d) < WINDOW:
                continue
            if b"ELLA" in d:
                magic_frames.append((r["ts"], r["ep"], d.find(b"ELLA"), len(d)))
            hit = None
            for off in range(0, len(d) - WINDOW + 1, STRIDE):
                hit = idx.get(d[off:off + WINDOW])
                if hit:
                    break
            if not hit:
                continue
            name, _approx = hit
            exact = locate(imgs[name], d)
            hits[name].append((r["ts"], r["ep"], exact, len(d)))

    print(f"\n== {total_frames} control/bulk frames")

    print("\n== bytes per endpoint (bus.dev ep, xfer)")
    for (bus, dev, ep, xfer) in sorted(ep_bytes, key=lambda k: -ep_bytes[k])[:20]:
        kind = {2: "ctrl", 3: "bulk"}.get(xfer, str(xfer))
        d = "IN " if ep & 0x80 else "OUT"
        print(f"   {bus}.{dev:<3} ep {ep:#04x} {d} {kind}  {ep_frames[(bus,dev,ep,xfer)]:>8} frames  "
              f"{fmt(ep_bytes[(bus,dev,ep,xfer)])}")
    big = [k for k in ep_bytes if ep_bytes[k] > 500_000 and not (k[2] & 0x80)]
    if big:
        print("   ^ endpoints above ~500 KB outbound are firmware-image scale; the shipped images")
        print("     are 364 KB - 1.7 MB.")

    if dfu:
        print(f"\n== ★ USB DFU TRANSACTION -- {len(dfu)} class request(s)")
        counts = defaultdict(int)
        dn_bytes = 0
        for _ts, req, _wv, _wi, wl, _di, _dat in dfu:
            counts[req] += 1
            if req == "DNLOAD":
                dn_bytes += wl
        print("   " + ", ".join(f"{k}={v}" for k, v in sorted(counts.items())))
        print(f"   DFU_DNLOAD payload total: {fmt(dn_bytes)}")
        dn = [x for x in dfu if x[1] == "DNLOAD"]
        if dn:
            sizes = sorted({x[4] for x in dn})
            blocks = [x[2] for x in dn]
            print(f"   block sizes: {sizes[:6]}{' …' if len(sizes) > 6 else ''}")
            print(f"   block numbers: {blocks[0]} .. {blocks[-1]}"
                  f"  ({'monotonic' if all(b >= a for a, b in zip(blocks, blocks[1:])) else 'NOT monotonic -- retries'})")
            print(f"   wall time: {dn[-1][0] - dn[0][0]:.1f} s")
            if dn[-1][4] == 0:
                print("   final DNLOAD is zero-length => the image was completed and manifested")
            else:
                print("   ⚠ no zero-length terminating DNLOAD: the transfer may be INCOMPLETE in")
                print("     the capture, or the flash was interrupted")
        # GETSTATUS replies carry bStatus/bState and are how a failure would announce itself.
        errs = []
        for ts, req, _wv, _wi, _wl, di, dat in dfu:
            if req == "GETSTATUS" and di and len(dat) >= 6:
                st, state = dat[0], dat[4]
                if st != 0:
                    errs.append((ts, DFU_STATUS.get(st, st), DFU_STATE.get(state, state)))
        if errs:
            print(f"   ⚠ {len(errs)} GETSTATUS reply/replies reported an error:")
            for ts, st, state in errs[:8]:
                print(f"      {ts - (t0 or ts):8.3f}s  {st} in state {state}")
        else:
            print("   every GETSTATUS reply reported status OK")
        print("   => this is the DFU transport, decoded. It is the first one recorded here.")

    if magic_frames:
        print(f"\n== container magic 'ELLA' seen in {len(magic_frames)} frame(s)")
        for ts, ep, off, n in magic_frames[:5]:
            rel = f"{ts - t0:8.3f}s" if t0 else f"{ts:.3f}"
            print(f"   {rel}  ep {ep:#04x}  at payload offset {off} of {n} B")
        print("   => a firmware container crossed the wire. This is the artifact.")

    if not hits and dfu:
        print("\n== DFU RAN, BUT NO PAYLOAD MATCHED A SHIPPED IMAGE")
        print("   The transport is decoded above, which is most of the value. The bytes not")
        print("   matching means DLM transformed the image on the way out -- a chunk header, a")
        print("   re-wrap, or a build that is not the one installed here. Compare the DNLOAD total")
        print("   against the .spkg sizes; a close match points at framing, a wild one at a")
        print("   different image.")
        return 0

    if not hits:
        print("\n== NO PAYLOAD MATCHED A SHIPPED IMAGE")
        print("   Read this together with the bcdDevice diff:")
        print("     bcdDevice unchanged  => the enforcer accepted the device's existing build.")
        print("                             No flash happened; nothing was missed.")
        print("     bcdDevice CHANGED    => a flash happened but its bytes are not the installed")
        print("                             .spkg -- transformed on the wire (chunk headers,")
        print("                             re-encryption) or a different build. Keep the capture:")
        print("                             that is a more interesting result, not a failure.")
        return 0

    print("\n== ★ FIRMWARE ON THE WIRE")
    for name, rows in hits.items():
        blob = imgs[name]
        known = [r for r in rows if r[2] is not None]
        covered = set()
        for _ts, _ep, off, n in known:
            if off is not None:
                covered.update(range(off // 4096, (off + n) // 4096 + 1))
        eps = sorted({r[1] for r in rows})
        sizes = sorted({r[3] for r in rows})
        first, last = rows[0][0], rows[-1][0]
        print(f"\n   {name}  ({len(blob)} B)")
        print(f"     frames carrying image bytes : {len(rows)}")
        print(f"     endpoints                   : {', '.join(f'{e:#04x}' for e in eps)}")
        print(f"     payload sizes               : {sizes[:8]}{' …' if len(sizes) > 8 else ''}")
        print(f"     wall time                   : {last - first:.1f} s")
        pct = 100.0 * len(covered) * 4096 / max(1, len(blob))
        print(f"     image coverage (4 KiB bins) : {min(pct,100.0):.1f}%")
        if known:
            offs = [r[2] for r in known]
            mono = all(b >= a for a, b in zip(offs, offs[1:]))
            print(f"     offsets advance monotonically: {'yes' if mono else 'NO (retries or interleave)'}")
            print(f"     first offset {min(offs)}  last offset {max(offs)}")
        if pct < 95:
            print("     ⚠ coverage below 95% -- either the capture missed part of the transfer, or")
            print("       the image is sent in a transformed form and only some chunks match")
            print("       verbatim. Check the endpoint byte total above against the image size.")
        else:
            print("     ✅ effectively the whole image was recorded.")
    print("\n== this is the first DisplayLink firmware update ever recorded here. Keep every file.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
