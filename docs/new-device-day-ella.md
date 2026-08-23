# New-device day — HP 3005pr (Ella / DL-3900)

A runbook for the one day this hardware is new. The general onboarding guide is
`new-device-capture.md`, which explains the *why* of each capture and is what you would hand a
stranger; `new-device-day.md` is the same document for the DL7400. This one is the *order*, for
this device, on this machine.

Everything here exists so that the day is a sequence of commands rather than a sequence of
decisions, because two of those decisions are irreversible.

---

## What the device is

**HP 3005pr USB 3.0 Port Replicator** — DisplayLink **DL-3900**, announced August 2012, dual head
(HDMI + DisplayPort), GbE, audio, its own PSU.

| | |
|---|---|
| silicon | **DL-3900**, i.e. the **DL-3x00** generation -- the oldest DLM still supports |
| platform codename | **Ella** -- identity blob tail `EllaDock`, package `ella-dock-release.spkg` |
| packaged image | `57bed729`, 2025-12-03, **928,464 B** -- **57 DFU blocks** of 16,384 (56 full, tail 8,208) |
| heads | 2, 1920x1200 each, or one 2560x1600 |
| product id | **read it, do not guess** -- `dl-identity.py` prints it. Expect `17e9:43xx` |
| what vino does with it today | **declines it by name.** `profile::for_family` maps `Ella => None`; there is no third generation in the driver yet |

Two things make this device worth more than another dock:

1. **It is the family the archived 2014 implementation targets.** `dl3dev/` is a working libusb DL3
   session against `17e9:4300` "EllaDock", and `new-device-capture.md` §2 already located vino's
   divergence from it: the transcript bundles `AKE_INIT` into the `init_4` write (84 bytes, two
   frames), vino sends an 80-byte capability probe there and posts `AKE_INIT` later under its own
   sub. The device ACKs either way and then does nothing, so nothing on the wire says no. **That
   diagnosis has never been tested against hardware.** This dock tests it.
2. **A 2012 dock has almost certainly never been flashed.** The enforcer flashes on a version
   mismatch, so first contact with DLM is a near-certain firmware update -- something never yet
   observed on the wire here.

Those two facts pull in opposite directions, which is the whole shape of the day.

---

## ⛔ Before it is plugged in: vino will flash it on sight

This is new since the DL7400 and it invalidates what the older runbook says.

vino used to match two exact product ids. Since `4f7551789675` it binds the **function**:

```
usb:v17E9p*d*dc*dsc*dp*icFFisc00ip03in*      <- any 17e9 DL3 display interface
usb:v17E9p*d*dc*dsc*dp*icFEisc01ip01in*      <- any 17e9 DFU interface
```

The product id is a wildcard, so vino claims this dock, and on the **DFU** interface `probe()` calls
`firmware::update_if_newer()` before the family check ever declines Ella. With
`/lib/firmware/vino/ella-dock-release.spkg` installed and a dock from 2012, that comparison is not
close: **vino flashes it the moment it enumerates.** The DFU interface does not implement upload, so
the running image cannot be read back and there is nothing to restore from.

That would spend the first contact on an untested code path, and destroy the pre-flash firmware.

**The flash capture is the priority that cannot be retaken, so it gets three independent
interlocks, all already applied on this machine (2026-08-10):**

1. **The images are held back**, so probe has nothing to write:
   ```bash
   sudo mkdir -p /lib/firmware/vino/held-back
   sudo mv /lib/firmware/vino/*-release.spkg /lib/firmware/vino/held-back/
   ```
   `request_nowarn` now fails and probe logs `no vino/ella-dock-release.spkg available; leaving the
   dock on <version>`. The D6000 is unaffected -- it already runs the packaged Ridge image.
2. **vino cannot autoload.** `/etc/modprobe.d/zz-dl-capture.conf` now carries `blacklist vino`
   alongside `udl`/`udlfb`. Verified by resolving the alias, not by reading the file:
   ```bash
   sudo modprobe -n -v 'usb:v17E9p4306d0257dc00dsc00dp00icFFisc00ip03in00'   # must print nothing
   ```
   An explicit `modprobe vino` still works, which is what Phase 0c needs.
3. **`capture-firstcontact.sh` refuses to proceed** with vino loaded (it unloads it, or stops and
   tells you to unplug the other dock first) or with packaged images still installed.

A blacklist does **not** unload a running module. vino is loaded right now, so on the day:

```bash
# unplug the D6000 first -- vino's refcount is its bound interfaces
sudo modprobe -r vino
```

**Restore when the day is over:** put the images back and drop the `blacklist vino` line.

```bash
sudo mv /lib/firmware/vino/held-back/*-release.spkg /lib/firmware/vino/
sudo sed -i '/^blacklist vino$/d' /etc/modprobe.d/zz-dl-capture.conf
```

Re-run the preflight before starting; it reads the driver's actual alias rather than asserting an id
table, and fails on each of the exposures above:

```bash
cd ~/Downloads/dl-scripts/vino && sudo tools/capture/preflight-newdevice.sh
```

### What was fixed in the tooling after the DL7400 run

The DL7400 first contact on 2026-08-01 **did** catch its flash — `fw-scan` reported 100% coverage of
`navarro-dock-release.spkg` across 430 frames with monotonic offsets. Three things went wrong around
it, and all three are fixed:

| what happened | cause | fix |
|---|---|---|
| `keys.log` came back **0 bytes** | the script started DLM under **systemd**, and `99-displaylink.rules` bounces the unit on every 17e9 add/remove -- so plugging the dock in restarted DLM 42 s after frida attached. The hook stayed bound to the dead pid and still looked healthy. The journal shows `Stopping`/`Started` at 18:09:42/45. | `capture-firstcontact.sh` now runs DLM **by hand with the unit masked**, the pattern `capture-portmap.sh` has used since 2026-08-02. It also compares the DLM pid at the end and says outright whether keys and wire came from one session. |
| the script never noticed the flash had **finished** | `fw-watch.py` only ever exited on `--secs` or Ctrl-C | it now detects manifestation (the zero-length `DFU_DNLOAD`), counts down `--settle` seconds (45) of DFU silence and stops by itself, printing `the flash is COMPLETE`. A second image restarts the wait, so it cannot stop early on a two-image update. |
| Ctrl-C left the script hanging for the full timeout | cleanup did a bare `wait` on the key extractor, which had been started with `--secs` covering the whole window | bounded shutdown: `SIGINT`, 10 s to write `keys-raw.json`, then `SIGTERM`/`SIGKILL`. The "wire still busy" loop is bounded too, so continuous video traffic can no longer block the finish. |

---

## The decision that cannot be undone

| goal | needs |
|---|---|
| **preserve** the pre-flash firmware, which is what the 2014 transcript was taken against | never let DLM or vino see the device |
| **capture a firmware update**, never once observed here | let DLM see it fresh, accepting that it rewrites the device |

With one unit you cannot have both *in the end* — but you can have both *in sequence*, because
everything the pre-flash firmware can tell you is a read-only session, and the flash is at the end.
**That is why the phases below are in this order and must not be reordered.** Phase 0 is one-shot;
after Phase 1 it can never be run again on this dock.

**Never interrupt a flash.** No unplug, no suspend, no killing DLM mid-write. That is how these get
bricked, and here there is no recovery image to fall back to.

---

## Phase 0 · the pre-flash session ★★★ — one shot, no DLM, no vino (20 min)

**Nothing in this phase can trigger a flash**, which is why it is safe to put in front of the
capture that matters most. The two things that flash a dock are DLM (masked) and vino's DFU probe
(blacklisted from autoload, with its images held back). Prove both before you plug anything in:

```bash
systemctl is-enabled displaylink-driver.service        # masked
ls /lib/firmware/vino/*-release.spkg 2>/dev/null       # nothing
lsmod | grep '^vino '                                  # nothing
```

If any of those three is wrong, stop and fix it. Everything else here is descriptor reads and one
read-only libusb session.

### 0a. Place the device (2 min)

```bash
cd ~/Downloads/dl-scripts/vino
sudo tools/capture/dl-identity.py | tee ~/ella-before-identity.txt
lsusb -v -d 17e9: > ~/ella-before-lsusb.txt 2>/dev/null
lsusb -d 17e9: -t | tee ~/ella-before-tree.txt
```

What to read out of it, and why each one decides something:

* **`bInterfaceProtocol` on the display interface.** `03` ⇒ DL3-family and vino's problem. `00` ⇒
  this is `udl` hardware, already driven by an in-tree driver, and the right answer is to say so
  rather than grow a second one. The DL-3900 should be `03`; confirm it rather than assuming.
* **The identity blob.** `EllaDock` confirms the platform and therefore which `.spkg` targets it.
  The archived transcript's blob is `10 40 08 09 21 06 02 02`; a `17e9:4301` reporting today reads
  `10 40 08 07 0d 06 03 03`. **Which one this dock matches decides whether the archived transcript
  is ground truth for it.** Record it exactly.
* **`bcdDevice`** — the firmware revision, and the "before" half of the flash proof.
* **The endpoint inventory** — `video_eps` for the profile, and the head count. The one-head `4301`
  exposes only `0x08`; a dual-head Ella should expose a second video endpoint, and *which address*
  it uses is a profile field vino cannot guess.
* **The DFU functional descriptor** — `wTransferSize` and `bmAttributes`. The D6000's is 16384 with
  `Will NOT Detach`, which is what makes 57 blocks the number to expect. If Ella's differs, the
  block count differs with it.

### 0b. Run the archived implementation — the live oracle ★★★ (10 min)

This is the highest-value thing available today and it is **only** available today. `dl3dev/` is a
complete DL3 session that once worked on this family. If it still reaches `H values matched` /
`L values matched` on this dock, you have an independent, executable oracle for the wire that vino
can be diffed against frame by frame — no DLM, no keys, no decryption.

It is ported and building (OpenSSL 3: `EVP_CIPHER_CTX`/`HMAC_CTX` are opaque now, `RSA_set0_key`
replaces the struct writes) and takes the product id on the command line, since it hardcoded
`0x4300`:

```bash
cd ~/Downloads/dl-scripts/dl3dev && make          # already built
sudo modprobe usbmon
sudo dumpcap -i usbmon<BUS> -s 0 -w ~/ella-dl3-preflash.pcapng &
sudo ./dl3 <PID-from-0a> 2>~/ella-dl3-libusb.log | tee ~/ella-dl3-preflash.txt
sudo pkill dumpcap
grep -iE 'H value|L value|matched|Claiming|rx:|tx:' ~/ella-dl3-preflash.txt | head -40
```

It calls `libusb_set_auto_detach_kernel_driver`, so it takes the interface off whatever holds it —
but keep vino unloaded anyway, so nothing re-binds behind it.

Read the result as:

| outcome | meaning |
|---|---|
| `H values matched` **and** `L values matched` | the transcript is live ground truth. The `init_4` fix can be developed and verified against this without DLM at all. |
| it gets further than vino did (a 546-byte `AKE_SEND_CERT` arrives) | `init_4` is confirmed as the divergence even if the AKE later fails on the hardcoded keys |
| it fails where it once passed | the firmware has already moved at some point in this unit's life; the transcript is historical, and `ella-before-identity.txt` will say which blob it carries |

Any of the three is a result. Keep the pcap regardless — a clear-text DL3 session on pre-flash
firmware is not reproducible after Phase 1.

### 0c. Let vino try, with flashing disarmed (5 min)

```bash
sudo modprobe vino
sleep 20
dmesg | grep -i vino | tee ~/ella-before-vino.txt
sudo modprobe -r vino
```

Expect: the identity log, `no vino/ella-dock-release.spkg available` from the DFU interface, and
the display interface **declining by name** — `for_family(Ella) => None`. That decline is the
correct behaviour and is the "before" for Phase 4. If instead it attempts a session and times out
three times, that is the §2 `init_4` failure and the log is worth keeping next to `dl3`'s.

---

## Phase 1 · first contact with DLM, and the flash ★★★ (30 min, mostly waiting)

Only after Phase 0 is safely on disk.

**Unplug the D6000** — it is on the current Ridge image, will not flash, and its traffic is noise in
a capture whose whole point is one large transfer. **No monitors on the new dock** for this phase:
no video traffic keeps the pcap in the tens of MB and the flash unambiguous.

```bash
cd ~/Downloads/dl-scripts/vino
sudo tools/capture/capture-firstcontact.sh ~/dlcap-ella-firstcontact 25
```

The script's ordering is the point: prescan with DLM masked to learn the bus and the "before"
identity; start five independent recorders (`dumpcap` on the device bus, `dumpcap` on `usbmon0` in
case the re-enumeration lands elsewhere, `fw-watch.py` reading `mon_bin` directly, xHCI tracepoints,
`dmesg -w`) and prove each is writing; start DLM with **no device attached** and attach frida while
it is idle — so the one moment frida could stall DLM into a watchdog restart happens when there is
nothing to corrupt; and only then do you plug the dock in.

What to expect on screen, for this image:

```
★ DFU DETACH … the device is being switched into its BOOTLOADER
★ DFU_DNLOAD: THE FLASH HAS STARTED
★ DFU_DNLOAD block 32, 512.00 KiB written so far
★ payload matches ella-dock-release.spkg (image offset 524288) — FLASH IN PROGRESS
```

**57 blocks** is the number to watch for: 56 of 16,384 B plus a tail of 8,208, then a zero-length
`DFU_DNLOAD` for manifestation and a self-reset.

**You do not have to judge when it is over.** After manifestation the watcher counts down 45 s of
DFU silence and stops on its own:

```
★ zero-length DFU_DNLOAD = end of image, manifestation phase…
  will stop automatically after 45s of DFU silence; a second image restarts the wait.
  … 12043 frames  28.4 MB  quiet 6s  DFU dnload=57 (928.5 KiB)  MANIFESTED, finishing in  31s
★ no DFU activity for 45s after manifestation: the flash is COMPLETE. Stopping.
```

Rules while it runs: do not unplug, do not suspend, do not Ctrl-C twice. The script refuses to stop
while the wire is moving and refuses to conclude "no flash" before five minutes. If nothing has
happened, let it run the full 5-10 minutes — 928 KB over a control pipe is minutes, and the enforcer
may verify, flash, reset and re-verify.

At the end it prints its own verdict: the `bcdDevice` diff, the identity-blob diff, the
re-enumerations, the decoded DFU transaction and image coverage. **A changed `bcdDevice`, or changed
middle bytes in the identity blob, is the proof.**

### If no flash happens

Not a failure, and recoverable — the enforcer flashes on a *mismatch*, and a mismatch can be
manufactured. Ella has been frozen since DLM 6.8.1.0, so there is no newer image to chase upward;
what is available here is a **downgrade** to the 6.4.24.0 build:

```
/opt/displaylink/ella-dock-release.spkg.6.4.24.0.bak   920,416 B
```

Whether the enforcer will flash backwards is unknown — `FindBestFirmwareForElla` suggests it picks a
best rather than any difference — but it costs one file swap to find out. Keep the original.

**And there is now a second trigger DLM cannot give you:** vino's own DFU path. `force_flash=1`
writes the packaged image whatever version the dock runs, and `/sys/class/firmware/vino-dock` takes
an arbitrary image. That is the deliberate way to produce a flash — but it exercises *vino's* DFU
implementation, not DLM's, so it answers a different question. Capture DLM's first, always.

### After the flash — re-run the oracle

```bash
sudo ./dl3 <PID> | tee ~/ella-dl3-postflash.txt
diff ~/ella-dl3-preflash.txt ~/ella-dl3-postflash.txt
```

A pre/post pair across a known firmware change is a rare artifact: it says directly whether the DL3
init sequence moved between 2012-era firmware and 2025-12-03.

---

## Phase 2 · the keyed feature capture ★★★ (20 min)

Now attach **both** monitors — HDMI on one head, DisplayPort on the other. Two heads is exactly
enough to expose the per-head selectors, and *not* enough to disambiguate them: `head`, `head + 1`
and `1 << head` coincide for heads 0 and 1, which is how four per-head selectors stayed wrong past
head 1 on the D6000 for months. Record the values; do not infer the encoding from two heads.

```bash
sudo tools/capture/capture-newdevice.sh ~/dlcap-ella-keyed
```

Guided by default: it prompts through `idle-before`, `connect`, `settle`, `cursor-move`,
`cursor-shape`, `cursor-off`, `window-drag`, `video`, `idle-after`, `mode-change`, `dpms`,
`monitor-unplug`, `dock-unplug`, timestamping each into `journal.tsv` so the wire slices by action:

```bash
awk -F'\t' '/dpms/' ~/dlcap-ella-keyed/journal.tsv
tools/capture/decrypt-dlm-cp.py ~/dlcap-ella-keyed/wire.pcapng \
    ~/dlcap-ella-keyed/keys.candidates.json --start <t> --end <t>
```

Every step in that list is there because its absence once cost real time. Run the cursor and mode
steps **once per head**.

⚠ Keys are per session and the AKE only runs on a cold connect, so the capture must span the plug.
⚠ A frida session ends if DLM restarts — replug the dock between modes, never restart DLM.

**Verify before moving on**, rather than discovering weeks later that nothing decrypts:

```bash
python3 -c 'import json;print(len(json.load(open("keys.candidates.json"))),"key candidates")'
tools/capture/decrypt-dlm-cp.py wire.pcapng keys.candidates.json | head -40
```

You want `id=0x48 sub=0x22` (set-mode), `id=0x194` (EDID) and `wsub=0x24`/`0x45` rendering as
structured plaintext.

---

## Phase 3 · DPMS and the mode matrix (20 min)

Both of these are on the user's list and both need care on this device.

**DPMS.** The D6000 corpus provably cannot settle the sink power-down: a DLM output toggle emits the
same `0x2e`/`0x2f` off23 sequence as a mode-set bracket, so the two cannot be told apart in a capture
that contains both. The fix is an *isolated* action, which is why `dpms` is its own journalled step
above. For a longer sitting with the real idle timeout, a single output disabled while its sibling
stays lit, and a cold power-cycle, use the paired runbook — it resolves outputs from the compositor,
so it works on two heads even though it was written for four sockets:

```bash
# terminal 1 (root)
sudo tools/capture/capture-portmap.sh --no-reauth --snap 4096 ~/vino-ella-dpms 3600
# terminal 2 (as the desktop user -- kscreen-doctor needs the Wayland session)
tools/capture/dpms-ports-runbook.sh ~/vino-ella-dpms
```

**Modes.** ⚠ DLM reprograms the dock's timing only at **connect**; a runtime resolution change makes
it *scale* and emits no set-mode at all. `capture-modematrix.sh` therefore replugs between modes
rather than restarting DLM, which would kill the key session:

```bash
sudo tools/capture/capture-modematrix.sh ~/dlcap-ella-modes <DOCK-OUTPUT>
```

⚠ Address modes as `WxH@rate`, never by index — `kscreen-doctor` renumbers indices between calls and
a stale index silently sets the wrong mode while still returning 0.

What the matrix is for **here** is different from the DL7400 run. This dock tops out at 1920x1200
per head (2560x1600 single), so it will not reach any of the high-clock questions. What it *can*
settle is the low end, and that is genuinely useful: `off42` was decoded as **sync polarity** packed
as `0x0400 | 0x0100*hSyncInv | 0x0200*vSyncInv`, and the evidence for the inverted-sync case is one
mode on one dock. An old dock driving 640x480p60 (`-h -v` ⇒ `0x0700`) and 1920x1200 CVT-RB
(`+h -v`) on **different silicon** is an independent confirmation of that reading, or a refutation.

Also record what DLM *offers* against what it *programs*: the 120 Hz ceiling on the D6000 is DLM's
declared `pixel_per_second_limit`, and whether that limit is per-platform is unknown.

---

## Phase 4 · what vino needs before it can drive this dock

vino declines Ella by name today. Turning that into a driven dock means filling one
`DockProfile` and deciding how much of `Generation` is genuinely new code. **The point of the table
below is that every row is answered by a capture above** — so take them knowing what each one is
for, rather than going back for a second sitting.

Measured from `~/dlcap-ella-keyed` on the HP 3005pr at dual 1920x1080@60:

| `DockProfile` field | value | how it was read |
|---|---|---|
| `video_eps` | ⚠ **`0x02` -- the control pipe** | interface 0 has one alternate setting with only `0x02` OUT / `0x84` IN. 42.6 MB of video went out on `0x02`; `0x84` carried 39.6 KB back. **There is no video endpoint.** |
| `head_sub_shift` | **0** (bare index) | video record wire sub is `0x0000` / `0x0001`; set-mode off22 is `0x00` / `0x01` |
| `band_parity_bit` | **false** | only `0x0000`/`0x0001` appear as record subs -- never Ridge's `0x10`/`0x11` parity forms |
| `connectors` | **2** | two record subs, two set-modes, two evdi devices |
| `hdr_capable` | **false** | DL-3900; HDR10 is DL-7000 only |
| `max_head_clock_khz` | >= **148,500** | set-mode off70..73 = 14850 in 10 kHz units |
| `generation` | **Ridge's** | see below |
| `strip_blocks_x` | **8** (64x16 px strips) | 30 strips per y band at 1920 px, x stepping 64; y stepping 16 |
| `interlaced_bands` | **false** | head 1 sends y=16 complete, then y=32 -- consecutive bands, not two passes |
| `strm2_marker` | **0x10** | off24..27 is `10 00 04` / `10 04 04`, the `<marker> [head*4] 04` triple |
| `ep84_queue_depth` | **1** | 625 EP84 URB events, never more than one outstanding |
| `stream_id_mask` | **not needed** | it only feeds the video content nonce, and Ella video is plaintext |
| `dock_buffers` | **still unmeasured** | needs a controlled damage experiment: change one small region and count how many times its strip is sent |

⭐ **Ella video is plaintext.** 560,638 of 560,646 strips parse straight off the wire with valid
`0x2801` magic and coherent coordinates. There is no video encryption, so the per-head SKE video key,
`cp::stream_content_nonce` and the arm burst have no role here -- a large simplification against
Ridge.

⭐ **The sink power-down, isolated for the first time.** Disabling one output while its sibling stayed
lit produced the pair with no `0x48/0x22` anywhere in the window:

| action | sequence (off22 = head) |
|---|---|
| sink **down** | `0x16 sub=0x2f` off23=**1**, then `0x16 sub=0x2e` off23=**3** |
| sink **up** | `0x16/0x2f` off23=1, `0x16/0x2e` off23=**0**, `0x16/0x2f` off23=**0** |

⛔ `kscreen-doctor --dpms off` does not reach the dock at all: zero control messages, and video
*quadruples* (14.0 MB idle to 62.7 MB) because the compositor paints black and DLM streams it. Use
`output.<name>.disable` to capture a power-down, never `--dpms`.

⭐ **A runtime resolution change reprograms this dock**, unlike the D6000: `0x48/0x22` appears in both
the mode-change and mode-restore windows, so a mode matrix here needs no replug. At 1280x1024@60 the
set-mode reads htotal 1688 / vtotal 1066, clock 108.00 MHz, off42 `0x0400`, off48 11915, and
**off66 `0x0800` against `0x2810` at 1920x1080@60** -- so off66 moves with resolution, not only with
refresh as the D6000 corpus suggested.

**The control plane is Ridge's, and that is the big result.** DLM's Ella session carries exactly the
message set this driver already builds -- `0x15/0x20`, `0x15/0x21`, `0x16/0x23` (EDID engage),
`0x16/0x2e`, `0x16/0x2f`, `0x19/0x31`, `0x48/0x22` -- and the set-mode decodes cleanly against the
existing field map: off22 head selector, off42 `0x0400` sync polarity (both positive, correct for
CEA 1080p60), off44 refresh 60, off66 `0x2810` which is **the same word the D6000 sends for
1080p60**, off68 `0x0200`, off70..73 = 148.50 MHz. The timing block is 1920+88+44+148 /
1080+4+5+36. Record framing matches too: every record is 16-byte aligned and <= 4080 bytes, which is
`video::haar` `STRIDE_CAP` exactly.

So Ella does not need a new `Generation`. What it needs is the transport:

> **Video shares the control pipe.** On Ridge and Navarro the control plane and the scanout path
> write to different endpoints and never interact. On Ella they are two writers on `0x02`, and a
> control message submitted while video URBs are queued lands inside a record. DLM avoids this by
> emitting one ordered record stream, slotting its 64-byte sealed control records *between* whole
> video records.
>
> `DockProfile::video_on_ctrl_pipe()` names this shape, and a KUnit assertion holds that no profile
> the driver hands out has it -- so the serialisation has to land in the same change as the profile,
> rather than being remembered.

**The generation question is the one that matters.** `Generation` names the split that is real code
rather than data: initialisation sequence, per-head HDCP framing, stream open, mode description.
Ella already differs from both in the first of those — the `init_4` bundling in §2 — so a third
variant is likely, and the honest order is:

1. Land the `init_4` split **first**, gated so a D6000 regression is impossible by construction, and
   verify it against `dl3`'s own transcript rather than by guessing.
2. Only then add `Generation::Ella` + `PROFILE_ELLA` with fields measured above, never inferred.
   A guessed profile is worse than no driver: the way a dock rejects a guess is to reset itself.

Do not write `PROFILE_ELLA` before Phase 2 exists on disk.

---

## Files to keep

Everything, unedited. The wire cannot be recaptured; keys can always be re-extracted later from the
recorded DLM build hash.

```bash
tar czf ella-$(date +%Y%m%d).tar.gz \
    ~/ella-before-*.txt ~/ella-dl3-*.txt ~/ella-dl3-*.pcapng ~/ella-dl3-libusb.log \
    ~/dlcap-ella-firstcontact ~/dlcap-ella-keyed ~/dlcap-ella-modes ~/vino-ella-dpms
```

Then put `/lib/firmware/vino/held-back/*.spkg` back.

---

## Quick reference

| symptom | cause |
|---|---|
| the dock flashed before you captured anything | vino's DFU probe -- the images were not held back, or vino was left able to autoload |
| `keys.log` is 0 bytes although frida attached | DLM was restarted under it. The unit must be MASKED and DLM run by hand; udev bounces the service on every 17e9 add/remove |
| the watcher sat there long after the flash finished | pre-2026-08-10 `fw-watch.py`; it now stops 45 s after manifestation |
| Ctrl-C and the script hung for minutes | pre-2026-08-10 cleanup waited on the key extractor's full `--secs` |
| `dumpcap` cannot initiate capture on `usbmon<N>` | `sudo modprobe usbmon` -- not autoloaded unless preflight wrote `modules-load.d` |
| capture empty | wrong bus; `busnum` changes across replugs and across a DFU re-enumeration |
| `dl3` prints `no 17e9:xxxx found` | wrong product id -- take it from `dl-identity.py`, in hex, no `0x` |
| `dl3` fails to claim the interface | something re-bound it; `sudo modprobe -r vino` |
| 0 key candidates | warm dock (no AKE inside the window), or DLM is not the 6.8.1.0 build the AES offset was derived for |
| keys present, nothing decrypts | keys and wire came from different sessions -- they must overlap |
| DLM does nothing | `evdi` not loaded, or the service is still masked |
| mode-set capture looks empty | DLM only reprograms at connect; replug, do not switch live |
| `kscreen-doctor` set the wrong mode and returned 0 | a mode **index** was used; address modes as `WxH@rate` |
