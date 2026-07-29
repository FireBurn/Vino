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

⚠ Only `0x4300` is covered by the archived transcript. `0x4301` and any single-bulk-endpoint device are
**unverified** and may genuinely be pre-DL3 (see the `udl` note in §3(a)).

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

### (b) A DLM capture designed to catch the firmware update ★★

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

### (c) Re-run the archived implementation, if you have it

If it still reaches `H values matched` / `L values matched` on that hardware, that is a live
independent oracle and by far the strongest evidence available. If it now fails where it once passed,
the firmware has moved and the transcript is historical.

---

## 4. What to send

```bash
D=dl-capture-$(date +%Y%m%d); mkdir -p $D
mv lsusb-verbose.txt interfaces.txt vino-identity.txt run*.pcapng run*.log run*.txt $D/ 2>/dev/null
cat > $D/NOTES.txt <<'EOF'
device:        <product name and PID>  -- WHICH physical unit, if you have several
same unit as the archived transcript?   <yes / no / unsure>
has DLM ever been run against it?       <yes / no / unsure>   <- decides if firmware may have moved
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

## Appendix — capture troubleshooting

| symptom | cause |
|---|---|
| `dumpcap` cannot initiate capture on `usbmon<N>` | `sudo modprobe usbmon` |
| capture empty | wrong bus; `busnum` changes across replugs |
| DLM does nothing | `systemctl status displaylink-driver.service`; needs `evdi` loaded |
| device grabbed before DLM sees it | blacklist not applied — reboot after writing it |
| mode-set capture looks empty | DLM only reprograms timing at **connect**; a runtime change makes it scale |
| `kscreen-doctor` set the wrong mode | mode **indices are renumbered** between calls — use `WxH@rate` |
