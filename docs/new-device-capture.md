# Onboarding a DisplayLink device vino cannot yet drive

For anyone with DisplayLink hardware other than the Dell D6000. **You run the captures and send the
files; no reverse engineering needed at your end.**

A report against `17e9:4300` — control session `ETIMEDOUT` after the DL3 AKE, no EDID — plus an
archived 2014 libusb implementation and a decoded transcript of a *working* session on that hardware
were enough to locate the bug. That diagnosis is in §2, and it makes the ask in §3 much smaller than a
blind capture campaign would be.

---

## ⛔ 0. READ THIS BEFORE YOU PLUG THE DEVICE INTO DLM

**DLM ships firmware for these docks and will very likely flash yours.** From the installed DLM:

```
/opt/displaylink/ella-dock-release.spkg        928464     <- Ella platform
/opt/displaylink/ridge-dock-release.spkg       888688     <- Ridge  (= D6000)
/opt/displaylink/navarro-dock-release.spkg    1758192
/opt/displaylink/firefly-monitor-release.spkg  364048
```

and from `strings` on the `DisplayLinkManager` binary itself:

```
FirmwareCompatibilityEnforcerImpl must have a valid FindBestFirmwareForElla and GetFirmware
DfuGetFirmware ... Unknown DFU requ...
Firmware sent / Firmware size / firmwareId = / FirmwareType =
Invalid firmware: beginning of TLV / body of TLV / wrong length of TLV
```

So there is a **firmware compatibility enforcer** with a platform-specific selector, a **DFU** path to
push an image, and a TLV-structured package per platform.

**Each package carries its own version, in plaintext.** The container is `"ELLA"` + a `u32` LE total
length (the format is named after the first platform to use it, not the contents), and somewhere inside
sits a build descriptor: the platform codename, an 8-hex build id, and a build date. Read it with:

```bash
strings -n 6 /opt/displaylink/ella-dock-release.spkg \
  | grep -E '^[0-9a-f]{8}$|^20[0-9]{2}-[0-9]{2}-[0-9]{2}$' | head -2
```

The codenames match the vendor identity blob tails exactly — `EllaDockOW`, `RidgeDocOW` — so a
device's blob names the package that targets it.

Measured across four shipped releases:

| source | ella-dock | ridge-dock (= D6000) | navarro-dock | firefly-monitor |
|---|---|---|---|---|
| Linux 6.4.24 | `99fc983d` 2023-09-21 | `ad23f7e3` 2024-02-02 | `0f416ea2` 2024-01-31 | `0f416ea2` 2024-01-31 |
| Linux 6.8.1 | `57bed729` 2025-12-03 | `3d85201b` 2026-03-23 | `52aef616` 2026-03-27 | `57bed729` 2025-12-03 |
| macOS 16.1 | `57bed729` 2025-12-03 | `3d85201b` 2026-03-23 | `7785f9fb` 2026-05-14 | `57bed729` 2025-12-03 |
| macOS 16.2 | `57bed729` 2025-12-03 | `3d85201b` 2026-03-23 | `914777de` 2026-06-18 | `57bed729` 2025-12-03 |

**Ella has been static since Linux 6.8.1** and is byte-identical in macOS 16.1 and 16.2
(sha256 `b44ba56e…`); only **Navarro** is iterating. So the image an Ella dock would be flashed with is
`57bed729` / 2025-12-03 regardless of which recent DLM you use — there is no newer one to chase, and
no reason to prefer 16.2 over 6.8.1 for this purpose.

⚠ Extracting the macOS `.pkg` takes three layers: xar → `pbzx` (chunked xz — `7z` stops after the
first 16 MB chunk, so decode the chunk list properly) → cpio. The packages land in
`DisplayLink Manager.app/Contents/Resources/`.

**Why this matters more than it looks.** A dock that has been in a drawer for a decade is almost
certainly running firmware older than the shipped image. If DLM decides to bring it up to date, that
is **irreversible**, and it may move the device *away* from the protocol the archived trace documents
— destroying the single most useful property the hardware has for this work.

### …and that makes it a capture we actively want

**No DisplayLink firmware update has ever been observed on the wire here.** The DFU path is visible in
the binary but has never been seen running, so a capture of one is a genuinely new artifact — worth
more than any of the protocol runs below. §3(b) is built specifically to catch it.

So there are two legitimate goals in tension, and they want different things:

| goal | what to do |
|---|---|
| **preserve** old firmware matching the archived trace | never let DLM see the device; do §1 + §3(a) only |
| **capture a firmware update** (never yet seen) | §3(b), accepting that it rewrites the device |

**If you have two units of the same product, you can have both** — reference one, sacrifice the other.
That is the ideal case and worth checking before anything else.

**Whichever you pick: never interrupt a flash.** No unplug, no suspend, no killing DLM mid-write.
That is how these get bricked. §3(b) is written to make that easy to honour.

---

## 1. `0x4300` is DL3-family — the timeout is a real vino gap

An early reading of this — that `43xx` parts are pre-DL3 and the timeout was therefore expected — was
**wrong**. The archived tree says otherwise:

* it targets `PID = 0x4300` and drives bulk **`0x02` OUT / `0x84` IN**, the same control pair vino uses;
* its HDCP header is the standard HDCP 2.2 message set, and it builds `AKE_INIT`,
  `AKE_NO_STORED_KM`, `LC_INIT` and `SKE_SEND_EKS` inside the DL3 `wsub=0x25` wrapper, verifying
  H′ and L′;
* the transcript's second line is the vendor identity blob `10 40 08 09 21 06 02 02 "EllaDock"` —
  the same `10 40` header and 16-byte shape as the D6000's `"RidgeDoc"`, and the codename matches
  `ella-dock-release.spkg` above.

So this is DL3 hardware DLM itself supports, and the `EPIPE` on vino's vendor preamble is an optional
request this firmware does not implement — a side issue, not the cause.

### `0x4301` is confirmed DL3 too

A `17e9:4301` (Plugable USB3-HDMI-DVI) reporting through vino's identity logging:

```
USB 17e9:4301 firmware (bcdDevice) 2.57 bcdUSB 3.00 speed super (5 Gbps)
DisplayLink Plugable USB3-HDMI-DVI
  ep 0x02 bulk-out maxp 1024        <- DL3 control OUT
  ep 0x84 bulk-in  maxp 1024        <- DL3 control IN
  ep 0x08 bulk-out maxp 1024        <- video
  ep 0x85 int-in   maxp 8
device identity = [10, 40, 08, 07, 0d, 06, 03, 03] "EllaDock"
```

That is **the D6000's control shape** — bulk `0x02` OUT + bulk `0x84` IN + video `0x08` — with one
video endpoint rather than four, matching a single head. It is a **USB 3.0 SuperSpeed** part, not an
old full-speed one, and its identity blob is byte-identical to the `0x4300`'s. So `4300` and `4301`
are the same platform, both DL3, and both vino's problem rather than `udl`'s.

It still fails identically (`ETIMEDOUT` ×3), which is exactly what §2 predicts.

⚠ Still unverified: any device with **only** a bulk endpoint and no `0x84`. That one may genuinely be
pre-DL3 — the `udl` check in §3(a) settles it.

---

## 2. Where vino diverges

Lining the working transcript against the failing vino log:

| step | transcript (works) | vino | |
|---|---|---|---|
| `init_0` | 16 B | 16 B | ✅ |
| `init_25` | 32 B | 32 B | ✅ |
| `init_4+probe` | **84 B** | **80 B** | ❌ |
| ACK | 38 B | 38 B | ✅ |
| `AKE_SEND_CERT` | **546 B** | never arrives | ❌ |

Those 84 bytes are two frames concatenated into one bulk write. **Frame A is byte-identical to
vino's.** Frame B is a different message:

| | `sub_len_dw` | frame B body | meaning |
|---|---|---|---|
| transcript | `0x00` | `1f 00 10 00 … 30 00 02` + Rtx | **`AKE_INIT`** (marker `0x30`, msg-id `0x02`) |
| vino | `0x0a` | `14 00 90 00 …` | a capability **probe** |

vino sends `AKE_INIT` much later as its own frame under sub `0x0022`; the transcript bundles it into
the `init_4` write under sub `0x04`, padded to a 4-byte boundary (`32 + 49 → 84`).

**The device ACKs either way.** The ACK is contentless and merely echoes the sub it was handed — the
transcript gets `14 00 10 00` back, vino gets `14 00 90 00`. So nothing on the wire says "no": no AKE
is initiated, no certificate is sent, the control session times out three times and gives up. That
also explains the missing EDID, which sits behind an engaged session.

This is the same trap as a previous cold-activation bug: a malformed message the dock accepts and then
ignores.

⚠ **This is a reading of two traces, not a tested fix.** The D6000 is happy with vino's ordering, so
any change must be conditional on device generation and must not regress it.

---

## 2a. Prerequisites — do all of these once

```bash
# Wire capture. usbmon is NOT autoloaded; this is the single most common lost capture.
sudo modprobe usbmon
ls /dev/usbmon*                       # expect usbmon0..N
sudo apt install wireshark-cli        # or: tshark / wireshark, for dumpcap + tshark

# Key extraction. Install as your NORMAL user, not root -- see the PYTHONPATH gotcha below.
pip install --user frida pycryptodome

# DLM needs evdi loaded to do anything at all.
lsmod | grep evdi || sudo modprobe evdi
systemctl status displaylink-driver.service

# The harness and the offline decryptor.
git clone https://github.com/FireBurn/vino-scripts && cd vino-scripts
```

**Stop anything racing DLM for the interface.** DLM and the kernel drivers fight over the same
interface, and a half-bound device produces a capture that looks like a protocol bug:

```bash
sudo tee /etc/modprobe.d/zz-dl-capture.conf >/dev/null <<'EOF'
blacklist udl
blacklist udlfb
blacklist vino
EOF
```

then **reboot**, or unplug/replug the device, so nothing is still holding it.

**Disk.** A whole-bus capture also records video traffic if a panel lights up — hundreds of MB per
minute at speed. Have a few GB free, or pass `-b filesize:200000` to `dumpcap` for a 200 MB ring.

---

## 3. What would help

### (a) Confirm the platform — two minutes, no DLM, no risk ★

```bash
lsusb -v -d 17e9: 2>/dev/null | tee lsusb-verbose.txt

for d in /sys/bus/usb/devices/*/; do
  v=$(cat "$d/idVendor" 2>/dev/null) || continue
  [ "$v" = "17e9" ] || continue
  echo "== $(basename $d)  pid=$(cat $d/idProduct 2>/dev/null)  bcdDevice=$(cat $d/bcdDevice 2>/dev/null)"
  for i in "$d"*:*; do
    [ -d "$i" ] || continue
    printf '   %s class=%s sub=%s proto=%s driver=%s\n' \
      "$(basename $i)" \
      "$(cat $i/bInterfaceClass 2>/dev/null)" \
      "$(cat $i/bInterfaceSubClass 2>/dev/null)" \
      "$(cat $i/bInterfaceProtocol 2>/dev/null)" \
      "$(basename "$(readlink $i/driver 2>/dev/null)" 2>/dev/null)"
  done
done | tee interfaces.txt
```

Then vino's identity logging — it prints at **info**, so no debug build:

```bash
sudo dmesg -C
sudo modprobe vino        # still expected to ETIMEDOUT; the log is the point
sleep 20
dmesg | tee vino-identity.txt
sudo modprobe -r vino
```

What this answers:

* **`bInterfaceProtocol` places the part.** The D6000's control interface is
  `class=ff sub=00 proto=03`, while in-tree `udl` matches `17e9` + `class ff` + `sub 00` +
  **`proto 00`** and reads vendor descriptor `0x5f`. So `proto=03` ⇒ DL3-family and vino's problem;
  `proto=00` ⇒ `udl` already owns that hardware and a second driver would be the wrong answer.
* **`bcdDevice`** is the firmware revision. ⚠ The archived transcript's identity blob is
  `10 40 08 09 21 06 02 02` while a device reporting today reads `10 40 08 07 0d 06 03 03` — same
  platform, **different firmware or different unit**. Settling that decides whether the transcript can
  be trusted as ground truth for the device that is failing, so it is the single most valuable field
  here.
* The **endpoint inventory** distinguishes your units: a DL3 control device has bulk OUT `0x02` +
  bulk IN `0x84` + video `0x08`; a lone bulk pipe is a different, older shape.

Send `lsusb-verbose.txt`, `interfaces.txt`, `vino-identity.txt`. **That, plus the archived transcript,
may be enough to produce a patch to test** — no wire capture needed.

### (b) The capture that actually adds support: wire **plus keys** ★★★

**This is the important one.** The init sequence and the whole HDCP AKE go over the wire in the clear
— which is why the archived transcript was enough to find the `init_4` bug. But everything after
`SKE_SEND_EKS` is **AES-CTR sealed**, and that is exactly where display support lives:

* `id=0x48 sub=0x22` — set-mode (timings, the mode profile words)
* `id=0x194` — the real EDID
* the post-msg0 setup burst, per-head restatement, stream markers

So a plain usbmon capture of DLM gives us sealed bytes we cannot read. **The keys have to be captured
from DLM in the same session**, and that is a solved problem — it is how the D6000's control plane was
decoded.

#### Two captures, simultaneously, one session

Keys are per-session, so the wire capture and the key capture must overlap. Run both:

| what | tool | why |
|---|---|---|
| the wire | **usbmon** | authoritative bytes, all endpoints, timing |
| `(ks, riv)` | **frida** on live DLM | the session key; hooks DLM's AES core |

⚠ Do **not** rely on frida for the wire — its USB hook drops bulk transfers (measured: 0 against
usbmon's 249 for the same traffic). usbmon is the source of truth for bytes; frida supplies only keys.

#### Version matching — please read, it decides whether this works

The key hook attaches at a **build-specific offset** inside DLM. Ours is derived for:

```
DLM package 6.8.1.0
/opt/displaylink/DisplayLinkManager  sha256 d3584c4369a594e9bcac…   (stripped PIE, BuildID 27c7a1f2…)
AES core @ 0x269dd0        round key INLINE at rdi+16 (not a pointer at rdi+8)
```

**Easiest path: install DLM 6.8.1.0** and the harness works unchanged.

If you must use a different version, send the binary's `sha256sum` (or the package version) and the
offset can be re-derived at this end — DLM uses a **software** AES, not AES-NI, so the anchor is the
Rijndael S-box (single occurrence, file offset `0x8ba5a0` in 6.8.1.0) and the function that references
it. Please don't guess the offset; a wrong hook yields an empty key and a silently keyless capture,
which has cost a hardware run here before.

#### Running it

```bash
git clone https://github.com/FireBurn/vino-scripts && cd vino-scripts
sudo modprobe usbmon                                  # NOT autoloaded
lsusb -d 17e9: -t                                     # note the BUS

# 1) wire, whole bus, full snaplen, in the background
sudo dumpcap -i usbmon<BUS> -s 0 -w keyed-wire.pcapng &

# 2) keys -- attach to the LIVE DLM (never --spawn: it wedges the dock)
sudo env PYTHONPATH=$HOME/.local/lib/python3*/site-packages \
  python3 vino-re/frida/decode-modeset-live.py --secs 120 --out keyed-keys.json \
  | tee keyed-keys.log

# 3) while both are running, make DLM do the things we need to see:
#    - let the device connect and reach a picture if it can
#    - change resolution, so a set-mode is emitted
sleep 5
sudo pkill dumpcap
```

⚠ `--secs` must cover the whole window, and **only one frida session at a time**.

⚠ **Never hook DLM's hot AES paths** — the harness deliberately hooks the block function, not the
per-round helpers, because hooking those stalls DLM into a watchdog restart.

⚠ CP crypto is **dormant on a warm dock**: if DLM already has an established session, no AKE and no
fresh keys will appear. Start the capture, then **connect the device** (or restart DLM) so a real
session initialises inside the capture window.

⚠ For a resolution change to emit a set-mode, note that DLM only reprograms the dock's timing at
**connect** on the D6000 — a runtime change makes it scale instead. If nothing appears, restart DLM
between modes rather than switching live.

#### Verify it before sending — this is the whole point

Prove the sealed traffic actually decrypts, rather than discovering weeks later that it doesn't:

```bash
python3 -c 'import json;print(len(json.load(open("keys.candidates.json"))),"key candidates")'
scripts/decrypt-dlm-cp.py wire.pcapng keys.candidates.json | head -40
```

You are looking for decoded inner messages — `id=0x48 sub=0x22` (set-mode), `id=0x194` (EDID),
`wsub=0x24`/`0x45` sealed traffic rendered as structured plaintext. If you get plausible message ids
and lengths, the capture is good.

**Even a keyless run is worth sending.** The wire capture cannot be recaptured later; keys can be
re-extracted from the recorded DLM build hash. Send it either way and say which you got.

### (c) A capture designed to catch the firmware update ★★

Read §0 first — this run may rewrite the device. What follows is arranged so that if a flash happens,
we get all of it, and so that we can *prove* it happened.

**Maximise the chance of triggering one.** The enforcer flashes when the device image does not match
what DLM carries, so: use the **newest** DLM you can, and let it see the device **fresh** (service
started, then device plugged) rather than mid-session.

#### Step 1 — record the "before" state, so a flash is provable

```bash
mkdir -p fwcap && cd fwcap
# The firmware revision and the vendor identity blob are what change across a flash.
for d in /sys/bus/usb/devices/*/; do
  [ "$(cat $d/idVendor 2>/dev/null)" = "17e9" ] || continue
  echo "$(basename $d) pid=$(cat $d/idProduct) bcdDevice=$(cat $d/bcdDevice)"
done | tee before-ids.txt
lsusb -v -d 17e9: > before-lsusb.txt 2>/dev/null

# vino's identity log (the 16-byte blob). Expected to still ETIMEDOUT; the log is the point.
sudo dmesg -C; sudo modprobe vino; sleep 20
dmesg | grep -E 'vino: (USB|device identity|  ep)' | tee before-vino.txt
sudo modprobe -r vino

# The images DLM would push, so wire bytes can be compared against the package.
sha256sum /opt/displaylink/*.spkg | tee spkg-sha256.txt
cp /opt/displaylink/*-release.spkg .        # ~4 MB total, worth having alongside
```

#### Step 2 — capture the whole bus, not the device

⚠ **Do not filter by device or PID.** A DFU commonly re-enumerates under a **different product ID**
while in bootloader mode, and a device-filtered capture loses exactly the interesting part. Capture the
whole bus for the entire window.

```bash
sudo modprobe usbmon                    # NOT autoloaded -- the most common lost capture
lsusb -d 17e9: -t                       # note the BUS number

# -s 0: no snaplen. A flash carries the payload in the data stage; truncation destroys it.
sudo dumpcap -i usbmon<BUS> -s 0 -w fw-flash.pcapng &
```

If the device might land on a different bus after re-enumerating, capture **usbmon0** (all buses) as
well or instead — noisier, but it cannot miss a bus change:

```bash
sudo dumpcap -i usbmon0 -s 0 -w fw-flash-allbus.pcapng &
```

#### Step 3 — let it run, and give it time

```bash
# In a second terminal, watch for the flash live:
sudo dmesg -w | grep -iE 'usb|displaylink' &

sudo systemctl restart displaylink-driver.service
sleep 10
#  >>> plug the device in now <<<
```

⚠ **Do not stop at 60 seconds.** The Ella image is ~928 KB; over a full-speed control pipe that is
minutes, and the enforcer may verify, flash, reset and re-verify. **Let it run 5–10 minutes**, and
watch for a re-enumeration in `dmesg` followed by the device settling. Only then:

```bash
sudo pkill dumpcap
journalctl -u displaylink-driver.service --since '-15 min' > fw-dlm.log
dmesg > fw-dmesg.txt
```

⚠ Disk: a whole-bus capture also records video traffic if a panel lights up, which can reach hundreds
of MB/min. If space is tight, add a ring buffer — but size the files generously so the flash is not
split awkwardly: `-b filesize:200000` (200 MB each).

#### Step 4 — record the "after" state

```bash
for d in /sys/bus/usb/devices/*/; do
  [ "$(cat $d/idVendor 2>/dev/null)" = "17e9" ] || continue
  echo "$(basename $d) pid=$(cat $d/idProduct) bcdDevice=$(cat $d/bcdDevice)"
done | tee after-ids.txt
lsusb -v -d 17e9: > after-lsusb.txt 2>/dev/null
sudo dmesg -C; sudo modprobe vino; sleep 20
dmesg | grep -E 'vino: (USB|device identity|  ep)' | tee after-vino.txt
sudo modprobe -r vino

echo "--- did the firmware move? ---"
diff before-ids.txt after-ids.txt
diff before-vino.txt after-vino.txt
```

**A changed `bcdDevice`, or changed middle bytes in the identity blob, is proof of a flash** — and the
before/after pair is what makes the capture interpretable rather than just a big pcap.

#### Step 5 — did it actually contain one?

```bash
# Large control transfers are the usual DFU signature.
tshark -r fw-flash.pcapng -Y 'usb.transfer_type == 0x02 && usb.data_len > 64' \
       -T fields -e frame.number -e usb.data_len 2>/dev/null | head -40

# Bulk writes big enough to be image chunks.
tshark -r fw-flash.pcapng -Y 'usb.transfer_type == 0x03 && usb.data_len >= 512' \
       -T fields -e frame.number -e usb.endpoint_address -e usb.data_len 2>/dev/null | head -40

# Total bytes moved per endpoint -- an image push shows up as a ~1 MB outlier.
tshark -r fw-flash.pcapng -T fields -e usb.endpoint_address -e usb.data_len 2>/dev/null \
  | awk 'NF==2 {s[$1]+=$2} END {for (e in s) printf "%s  %d bytes\n", e, s[e]}' | sort -k2 -rn | head

# Re-enumerations, and any new PID appearing (bootloader mode):
grep -E 'new .*-speed USB device|idProduct' fw-dmesg.txt | head -20
```

**Roughly 900 KB–1 MB moving in one direction on a single endpoint is the thing to look for.** If you
see it, say so — that alone is the result, and I can work from the pcap.

If nothing like it appears, that is still a useful answer: it means the enforcer decided this device's
firmware was already acceptable, and the `bcdDevice` diff in step 4 will corroborate that nothing
changed.

### (d) Re-run the archived implementation, if you have it

If it still reaches `H values matched` / `L values matched` on that hardware, that is a live
independent oracle and by far the strongest evidence available. If it now fails where it once passed,
the firmware has moved and the transcript is historical.

---

## 4. What to send

```bash
D=dl-capture-$(date +%Y%m%d); mkdir -p $D
mv lsusb-verbose.txt interfaces.txt vino-identity.txt $D/ 2>/dev/null
# the keyed capture directory from scripts/capture-newdevice.sh, whole:
mv ~/dlcap-keyed $D/keyed 2>/dev/null
mv run*.pcapng run*.log run*.txt fwcap $D/ 2>/dev/null
cat > $D/NOTES.txt <<'EOF'
device:        <product name and PID>  -- WHICH physical unit, if you have several
same unit as the archived transcript?   <yes / no / unsure>
has DLM ever been run against it?       <yes / no / unsure>   <- decides if firmware may have moved
key candidates captured?                <number, from the verify step>
did decrypt-dlm-cp.py show plaintext?   <yes / no / not tried>
DLM build sha256:                       <from dlm-sha256.txt, needed to re-derive the key offset>
monitor:       <make/model, native mode>, connected via <VGA/DVI/HDMI>
through a hub? <yes/no>
DLM version:   <package version>
did a picture ever appear?              <yes/no>
EOF
tar czf $D.tar.gz $D/
```

Send to **mike@fireburn.co.uk**, or a link.

`NOTES.txt` matters more than it looks. "Same unit as the transcript" and "has DLM ever been run
against it" decide whether the transcript is usable as ground truth — several past dead ends came from
not knowing that much about a capture.

---

## 5. What happens next

1. A **device-generation split**, so `init_4` frame B can carry `AKE_INIT` on gen-1 parts while the
   D6000 keeps its current ordering — gated so a D6000 regression is impossible by construction.
2. Replay the rest of the archived transcript against vino's builders frame by frame. It is a complete
   session, so it should pin the remaining differences with no new captures.
3. If a part turns out to be pre-DL3 and already handled by `udl`, that will be said plainly rather
   than growing a second driver for hardware that already works.
4. If §3(b) catches a firmware update, that gets documented on its own account — the DFU request
   shapes, the framing of the image on the wire, and how it lines up against the shipped `.spkg` TLV
   container. Nothing here has ever seen one, so it would be new ground independent of vino.

---

## Appendix — every gotcha, each of which has cost a real run

### Wire capture

| gotcha | consequence | do this |
|---|---|---|
| **`usbmon` is not autoloaded** | `dumpcap` cannot open the interface | `sudo modprobe usbmon` before anything |
| capture is empty | wrong bus | `busnum` **changes across replugs**; re-check after every plug |
| **default snaplen truncates** | payloads cut; a sealed frame or firmware chunk is unusable | always `dumpcap -s 0` |
| filtering by device or PID | a **DFU re-enumerates, possibly under a different product ID**, and you lose exactly the interesting part | capture the whole **bus**; `usbmon0` (all buses) if it may move bus |
| usbmon `read()` caps at ~61 KB/URB | a >61 KB URB loses its tail, desyncing a record walk | fine for control traffic; for byte-exact video use `MON_IOCX_MFETCH` + mmap |
| stopping too early | a ~1 MB firmware image over a control pipe takes **minutes** | run 5–10 min for a flash watch; watch `dmesg -w` |
| whole-bus capture fills the disk | run dies mid-flash | few GB free, or `-b filesize:200000` |

### Key extraction (frida)

| gotcha | consequence | do this |
|---|---|---|
| **`sudo python3` cannot see user-site frida** | `ModuleNotFoundError: frida` | `sudo env PYTHONPATH=$HOME/.local/lib/python3.X/site-packages python3 …` |
| **the frida USB hook drops bulk transfers** | measured 0 against usbmon's 249 for the same traffic | usbmon is the source of truth for **bytes**; frida supplies **keys** only |
| **build-specific AES offset** | a wrong hook yields an **empty key** and a silently keyless capture | match DLM 6.8.1.0, or send the binary hash so it can be re-derived. Never guess |
| the round key is **inline at `rdi+16`** | the `rdi+8`-as-pointer form reads null and yields empty keys | already handled in the harness; do not "fix" it |
| **hooking the hot AES helpers** (`0x269e69`, `0x1cf436`) | stalls DLM into a **watchdog restart** | hook only the block function, as the harness does |
| **more than one frida session** | they interfere; captures come back empty | one at a time |
| **`--spawn`** | **wedges the dock firmware** — blank screen, needs replug or reboot | never on hardware you care about; attach to the live DLM |
| decrypting before persisting | a decrypt bug throws away a hardware run | the harness writes raw **first**; keep `keys-raw.json` |
| `--secs` shorter than the window | keys captured, session missed, or vice versa | cover the whole capture |

### Getting DLM to do the thing

| gotcha | consequence | do this |
|---|---|---|
| **CP crypto is dormant on a warm dock** | no AKE, no fresh keys, nothing to decrypt with | start the capture, **then** connect the device or restart DLM |
| **DLM reprograms the dock timing only at connect** | a runtime resolution change makes it **scale**; no set-mode on the wire | restart DLM between modes rather than switching live |
| **`kscreen-doctor` mode indices are renumbered** between `-o` calls | a stale index silently sets the **wrong mode** and still returns 0 | address modes as `WxH@rate`, never by number |
| `udl` / `udlfb` / `vino` loaded | they race DLM for the interface; the capture looks like a protocol bug | blacklist all three, then **reboot** |
| `evdi` not loaded | DLM does nothing at all | `modprobe evdi`; check the service status |
| **interrupting a firmware flash** | **bricks the device** | never unplug or suspend mid-write; if in doubt, wait |

### Analysis, if you look yourself

| gotcha | consequence | do this |
|---|---|---|
| **numpy 1.26.4 on Python 3.14 corrupts large arrays** | silently overwrites live array locals ≥256 KiB **inside functions** | use a venv with a matched numpy; treat any large-array result from that combo as suspect |
| a "contentless ACK" | the device ACKs malformed messages and then ignores them, so **nothing on the wire says no** | never treat an ACK as proof a message was understood — that is exactly the `init_4` bug |
| comparing against a forced-key run | a forced key can hide a real key bug for weeks | re-diff with a fresh random session |

### Quick reference

| symptom | cause |
|---|---|
| `dumpcap` cannot initiate capture on `usbmon<N>` | `sudo modprobe usbmon` |
| capture empty | wrong bus number |
| `ModuleNotFoundError: frida` under sudo | missing `PYTHONPATH` |
| 0 key candidates | warm dock (no AKE in window), or wrong AES offset for your build |
| keys present but nothing decrypts | keys from a different session than the wire — they must overlap |
| DLM does nothing | `evdi` not loaded, or service not running |
| mode-set capture looks empty | DLM only reprograms at connect |
