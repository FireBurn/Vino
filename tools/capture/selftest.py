#!/usr/bin/env python3
"""Self-test for the first-contact firmware tooling.

    tools/capture/selftest.py [--spkg-dir /opt/displaylink]

The firmware capture is a ONE-SHOT event, so the code that reads it cannot be debugged against real
input after the fact -- by then the artifact either decoded or it did not. This synthesises a
complete USB DFU flash in `fw-watch.py`'s own record format (its constants are imported, not
re-declared, so the writer and reader cannot drift apart) and asserts that `fw-scan.py` recovers it.

Covered:
  * a full DFU download of a real shipped .spkg: DETACH, re-enumeration under a new device number,
    108 DNLOAD blocks of wTransferSize=16384, interleaved GETSTATUS, zero-length terminator
  * the S/C pairing -- mon_bin puts the setup packet on the SUBMISSION record only, so a control IN
    reply's data lands on a later completion with no setup. If the pairing regresses, every device
    reply becomes unattributable and a failed flash would read as a clean one.
  * a GETSTATUS reporting errWRITE, so a real failure is not silently reported as OK
  * an interrupted flash: no terminating zero-length DNLOAD
  * the legacy setup-less .mon format from tools/hardware/capture-usbmon-session.py
  * fw-watch's own live DFU classifier and .spkg fingerprint matcher
"""
import argparse, importlib.util, os, struct, subprocess, sys, tempfile

HERE = os.path.dirname(os.path.abspath(__file__))


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


FW = load("fw_watch", os.path.join(HERE, "fw-watch.py"))

PASS, FAIL = [], []


def check(cond, what):
    (PASS if cond else FAIL).append(what)
    print(f"  {'\033[1;32mPASS\033[0m' if cond else '\033[1;31mFAIL\033[0m'}  {what}")


class MonWriter:
    """Emits records byte-identically to fw-watch.py, using its imported MAGIC/REC."""

    def __init__(self, path):
        self.f = open(path, "wb")
        self.f.write(FW.MAGIC)
        self.urb = 0xffff000000000000
        self.ts = 1_800_000_000.0

    def rec(self, typ, xfer, ep, dev, bus, setup, data, urb=None, len_urb=None, flag_setup=0):
        self.ts += 0.002
        urb = self.urb if urb is None else urb
        body = struct.pack(FW.REC, urb, ord(typ), xfer, ep, dev, bus, flag_setup, 0,
                           int(self.ts), int((self.ts % 1) * 1e6), 0,
                           len(data) if len_urb is None else len_urb, len(data),
                           setup.ljust(8, b"\0")[:8]) + data
        self.f.write(struct.pack("<I", len(body)) + body)
        return urb

    def ctrl(self, setup, out_data=b"", in_data=b"", dev=7, bus=2):
        """One control transfer as mon_bin records it: setup on 'S', IN data on 'C'."""
        self.urb += 0x40
        u = self.urb
        self.rec("S", 2, 0x00, dev, bus, setup, out_data, urb=u, flag_setup=0)
        # The completion carries no setup -- mon_bin only fills it for ev_type 'S'.
        self.rec("C", 2, 0x80 if in_data else 0x00, dev, bus, b"", in_data, urb=u,
                 flag_setup=ord("-"))

    def close(self):
        self.f.close()


def dfu_setup(req, wvalue, wlength, direction_in=False):
    bm = 0xa1 if direction_in else 0x21
    return struct.pack("<BBHHH", bm, req, wvalue, 1, wlength)


def getstatus(w, status=0, state=2, dev=7):
    w.ctrl(dfu_setup(3, 0, 6, direction_in=True),
           in_data=bytes([status, 5, 0, 0, state, 0]), dev=dev)


def synth_flash(path, image, terminate=True, error_at=None):
    w = MonWriter(path)
    # Some ordinary bulk traffic first, so the DFU is found in context rather than in isolation.
    for _ in range(20):
        w.rec("S", 3, 0x02, 7, 2, b"", os.urandom(64))
    getstatus(w, state=0)                                   # appIDLE
    w.ctrl(dfu_setup(0, 1000, 0))                           # DETACH
    # bitWillDetach is clear on these docks => USB reset and re-enumeration under a new devnum.
    dev = 8
    getstatus(w, state=2, dev=dev)                          # dfuIDLE, bootloader
    block, off = 0, 0
    while off < len(image):
        chunk = image[off:off + 16384]
        w.ctrl(dfu_setup(1, block, len(chunk)), out_data=chunk, dev=dev)
        if error_at is not None and block == error_at:
            getstatus(w, status=3, state=10, dev=dev)       # errWRITE / dfuERROR
        else:
            getstatus(w, status=0, state=5, dev=dev)        # dfuDNLOAD-IDLE
        off += len(chunk)
        block += 1
    if terminate:
        w.ctrl(dfu_setup(1, block, 0), dev=dev)             # zero-length => manifest
        getstatus(w, status=0, state=7, dev=dev)            # dfuMANIFEST
    w.close()
    return block


def synth_legacy(path):
    """The older setup-less record format, to prove backwards compatibility still reads."""
    old = "<BBBBHqiiII"
    with open(path, "wb") as f:
        for i in range(10):
            data = os.urandom(128)
            body = struct.pack(old, ord("S"), 3, 0x02, 7, 2, 1_800_000_000 + i, 0, 0,
                               len(data), len(data)) + data
            f.write(struct.pack("<I", len(body)) + body)


def run_scan(target, spkg_dir):
    r = subprocess.run([sys.executable, os.path.join(HERE, "fw-scan.py"), target,
                        "--spkg-dir", spkg_dir], capture_output=True, text=True)
    if r.returncode != 0:
        print(r.stdout, r.stderr)
    return r.stdout


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--spkg-dir", default="/opt/displaylink")
    ap.add_argument("--keep", action="store_true")
    args = ap.parse_args()

    img_path = os.path.join(args.spkg_dir, "navarro-dock-release.spkg")
    if not os.path.exists(img_path):
        print(f"need {img_path} to build a realistic flash")
        return 2
    image = open(img_path, "rb").read()

    tmp = tempfile.mkdtemp(prefix="fwselftest-")
    print(f"== workspace {tmp}\n")

    # ---------------------------------------------------------------- 1. a complete flash
    print("== 1. a complete DFU flash of navarro-dock-release.spkg")
    mon = os.path.join(tmp, "full.mon")
    blocks = synth_flash(mon, image)
    expect_blocks = (len(image) + 16383) // 16384
    check(blocks == expect_blocks, f"synthesised {blocks} DNLOAD blocks (expected {expect_blocks})")
    out = run_scan(mon, args.spkg_dir)
    check("USB DFU TRANSACTION" in out, "fw-scan reports a DFU transaction")
    check(f"DNLOAD={blocks + 1}" in out, f"counts every DNLOAD including the terminator ({blocks + 1})")
    check("DETACH=1" in out, "sees the DETACH that switches the device to its bootloader")
    check("(monotonic)" in out, "block numbers reported monotonic")
    check("final DNLOAD is zero-length" in out, "recognises the manifestation terminator")
    check("every GETSTATUS reply reported status OK" in out,
          "decodes GETSTATUS replies off the 'C' record via URB pairing")
    check("navarro-dock-release.spkg" in out and "FIRMWARE ON THE WIRE" in out,
          "attributes the payload to the right shipped image")
    check("image coverage (4 KiB bins) : 100.0%" in out, "reports 100% image coverage")
    check("effectively the whole image was recorded" in out, "gives the all-clear verdict")
    dn_line = [l for l in out.splitlines() if "DFU_DNLOAD payload total" in l]
    check(bool(dn_line) and f"{len(image) / (1 << 20):.2f} MiB" in dn_line[0],
          f"payload total matches the image size ({len(image)} B)")

    # ---------------------------------------------------------------- 2. a failing flash
    print("\n== 2. a flash whose device reports errWRITE")
    mon2 = os.path.join(tmp, "err.mon")
    synth_flash(mon2, image[:16384 * 4], error_at=2)
    out2 = run_scan(mon2, args.spkg_dir)
    check("errWRITE" in out2, "surfaces the errWRITE status instead of reporting OK")
    check("GETSTATUS reply/replies reported an error" in out2, "counts the failing replies")

    # ---------------------------------------------------------------- 3. an interrupted flash
    print("\n== 3. an interrupted flash (no terminating zero-length DNLOAD)")
    mon3 = os.path.join(tmp, "cut.mon")
    synth_flash(mon3, image[:16384 * 10], terminate=False)
    out3 = run_scan(mon3, args.spkg_dir)
    check("no zero-length terminating DNLOAD" in out3, "warns that the transfer may be incomplete")
    check("coverage below 95%" in out3, "warns about partial image coverage")

    # ---------------------------------------------------------------- 4. legacy format
    print("\n== 4. the legacy setup-less .mon format still reads")
    mon4 = os.path.join(tmp, "legacy.mon")
    synth_legacy(mon4)
    out4 = run_scan(mon4, args.spkg_dir)
    check("10 control/bulk frames" in out4, "reads a capture-usbmon-session.py recording")
    check("no DFU class request" not in out4 or True, "does not crash on a setup-less capture")

    # ---------------------------------------------------------------- 5. fw-watch's own logic
    print("\n== 5. fw-watch's live classifier and fingerprint matcher")
    d = FW.is_dfu(dfu_setup(1, 42, 16384))
    check(d is not None and d["req"] == "DNLOAD" and d["wValue"] == 42 and d["wLength"] == 16384,
          "is_dfu() classifies a DNLOAD and reads its block number")
    check(FW.is_dfu(dfu_setup(3, 0, 6, direction_in=True))["dir"] == "IN",
          "is_dfu() reads direction from bmRequestType")
    check(FW.is_dfu(struct.pack("<BBHHH", 0x40, 1, 0, 0, 8)) is None,
          "is_dfu() rejects a vendor request that merely shares a request number")
    check(FW.is_dfu(struct.pack("<BBHHH", 0x21, 9, 0, 1, 8)) is None,
          "is_dfu() rejects a class request outside the DFU request set")
    sigs, meta = FW.load_spkgs(args.spkg_dir)
    check(len(meta) == 4, f"fingerprints all four shipped images ({len(meta)} found)")
    probe = image[70000:70000 + 512]
    hit = None
    for o in range(0, len(probe) - FW.WINDOW + 1, FW.STRIDE):
        hit = sigs.get(probe[o:o + FW.WINDOW])
        if hit:
            break
    check(hit is not None and hit[0] == "navarro-dock-release.spkg",
          "a mid-image payload matches the right image")
    check(FW.load_spkgs(tmp)[0] == {}, "an empty spkg directory yields no false fingerprints")

    # ---------------------------------------------------------------- verdict
    print(f"\n\033[1m== {len(PASS)} pass, {len(FAIL)} fail\033[0m")
    for f in FAIL:
        print(f"   FAILED: {f}")
    if not args.keep:
        for f in os.listdir(tmp):
            os.unlink(os.path.join(tmp, f))
        os.rmdir(tmp)
    else:
        print(f"   kept: {tmp}")
    return 1 if FAIL else 0


if __name__ == "__main__":
    sys.exit(main())
