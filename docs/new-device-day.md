# New-device day — WAVLINK DL7400

A runbook for the one day the hardware is new. Everything here exists so that tomorrow is a
sequence of commands rather than a sequence of decisions.

The general onboarding guide is `new-device-capture.md`; it explains the *why* of each capture and
is what you would hand a stranger. This document is the *order*, for this device, on this machine.

---

## What the device almost certainly is

**WAVLINK DL7400** — DisplayLink quad-4K dock, 4×DP 1.4, 2.5G LAN, 100 W PD, USB 3.2.

| | |
|---|---|
| silicon | **DL-7400**, i.e. the **DL-7000** generation |
| platform codename | **Navarro** — identity blob tail `NavaDock`, package `navarro-dock-release.spkg` |
| installed image | `52aef616`, 2026-03-27, **1,758,192 B**, container magic `ELLA` + LE length |
| heads | 4 (the D6000 is 2) |
| why it matters | Navarro is **the only one of the four shipped images still iterating**. Ella has been frozen since DLM 6.8.1.0 and is byte-identical across macOS 16.1/16.2; Ridge and Firefly are static too. A dock built before 2026-03-27 is therefore very likely to be flashed on first contact. |

Confirm rather than assume — `dl-identity.py` prints the codename in ten seconds, with no driver and
no DLM.

### The flash will be standard USB DFU, and we know its shape in advance

The D6000 exposes this, and the DL7400 will almost certainly match:

```
iface 1  class=fe sub=01 proto=01          <- USB DFU, RUNTIME mode
  Device Firmware Upgrade Interface Descriptor:
    bmAttributes 0x01     Download Supported, Upload Unsupported,
                          Manifestation Intolerant, Will NOT Detach
    wDetachTimeout  200 ms
    wTransferSize   16384 bytes
    bcdDFUVersion   1.01
```

So a flash is **class control transfers on interface 1**, not bulk:

| step | on the wire |
|---|---|
| `DFU_DETACH` | `bmRequestType 0x21  bRequest 0` |
| USB reset | `bitWillDetach` is **clear**, so the host resets the port and the device **re-enumerates**, possibly under a different product id |
| `DFU_DNLOAD` × N | `0x21 / 1`, `wValue` = block number, ≤ **16384 B** each |
| `DFU_GETSTATUS` | `0xa1 / 3`, 6 bytes back: `bStatus`, `bwPollTimeout`, `bState` |
| terminating `DFU_DNLOAD` | zero length ⇒ manifestation, then a self-reset |

`1,758,192 / 16384 = 108` blocks for the Navarro image. **That is the number to watch for.**

Two consequences that the scripts already encode: capture the **whole bus**, never a device or PID
filter, because the re-enumeration is mid-transaction; and record the **setup packet**, because a
control transfer without it is an anonymous blob.

---

## The night before — once, ~5 minutes

```bash
cd ~/Downloads/dl-scripts/vino
sudo tools/capture/preflight-newdevice.sh --fix
```

It checks and, with `--fix`, repairs: usbmon loaded *and* autoloading, `dumpcap` proven able to open
a usbmon interface *right now* **and to write into a root-created directory under `$HOME`**, DLM
masked and hash-matched against the build the frida AES offset was derived for, `udl`/`udlfb`
neutralised, `evdi` present, disk headroom, frida importable under root, and — the important one —
**it runs `selftest.py`**, which synthesises a complete DFU flash of a real shipped `.spkg` and
asserts the decoder recovers it. Syntax is not the risk; a capture that decodes into nothing is.

A reboot is demanded only if the blacklist actually suppresses something. On a kernel where `udl`
and `udlfb` are not built, they cannot autoload, the blacklist is belt-and-braces, and **no reboot
is needed** — the script says which case you are in rather than always insisting.

Fix every `FAIL` tonight. There is no second attempt at the flash.

> **Run as of 2026-08-01 on this machine: 28 pass, 0 warn, 0 fail.** `udl`/`udlfb` are not built on
> `7.2.0-rc2-drm+`, so no reboot was required; `evdi` is loaded and its `add`/`count`/`remove_all`
> control interface is present.

---

## The day — phase by phase

### Phase 0 · clear the decks (2 min)

**Unplug the D6000.** It is already on the current Ridge build, it will not flash, and its traffic
is pure noise in a capture whose whole point is a single large transfer. `vino` can stay loaded —
its USB id table is `17e9:6006` exactly, so it cannot claim the new dock.

Have **no monitors** connected to the new dock for the firmware phase.

### Phase 1 · the firmware capture ★★★ (20–30 min, mostly waiting)

```bash
sudo tools/capture/capture-firstcontact.sh ~/dlcap-firstcontact 25
```

This is your plan, with one addition. The order it runs:

1. **Prescan.** You plug the dock in *once* while DLM is still masked, purely to read descriptors,
   then unplug it. Enumeration cannot flash anything, and it converts two guesses into facts: the
   **bus number**, and the **before** firmware revision to diff against. (`--no-prescan` skips it.)
2. **Recorders, all verified writing before anything else happens:** `dumpcap` on the device's bus,
   `dumpcap` on `usbmon0` (every bus — the DFU re-enumeration may land elsewhere), `fw-watch.py`
   reading `mon_bin` directly as an independent backend *and* the live meter, xHCI tracepoints for
   port resets and slot lifecycle, and `dmesg -w`. Sleep, idle and lid are inhibited.
3. **DLM starts with no device attached, and frida attaches to it while it is idle.** This is the
   ordering that makes keys safe to take. The documented hazard is that frida can stall DLM into a
   watchdog restart, and a watchdog restart *during a flash* is how these get bricked — so the
   risky moment is deliberately placed when there is nothing to corrupt. Never `--spawn`.
4. **Then you plug the dock in**, and the whole first contact is inside every recorder.

While it runs, `fw-watch.py` prints a live line and shouts in magenta on:

```
★ DFU DETACH … the device is being switched into its BOOTLOADER
★ DFU_DNLOAD: THE FLASH HAS STARTED
★ DFU_DNLOAD block 64, 1.00 MiB written so far
★ payload matches navarro-dock-release.spkg (image offset 524288) — FLASH IN PROGRESS
```

**Rules while it runs:** do not unplug anything, do not suspend, do not Ctrl-C twice. The script
refuses to stop while the wire is still moving, and refuses to conclude "no flash" before five
minutes.

At the end it prints the verdict itself: the `bcdDevice` diff, the identity-blob diff, the
re-enumerations, the decoded DFU transaction and the image coverage.

> **Keys note.** The `.spkg` payload key is **dock-side** — no host binary can decrypt it, which is
> already established. DLM therefore pushes the container opaquely, so the image should be
> recognisable on the wire *without any key at all*. If frida fails to attach, the run is still
> good; let it continue.

### Phase 2 · protocol and features, keyed (15 min)

Attach **one monitor**, and put it on the **last DP port**, not the first.

> With four heads, that single choice is worth a whole experiment: on the D6000 the head selector is
> a single byte (probe `byte22`, EDID engage `off23`, cursor `off22`) and with two heads a **0/1
> index** and a **`1<<head` bitmask** are indistinguishable. Head 3 tells them apart immediately —
> index gives `0x03`, bitmask gives `0x08`.

```bash
sudo tools/capture/capture-newdevice.sh ~/dlcap-keyed
```

The guided choreography (idle / connect / cursor move / cursor shape / cursor off / drag / video /
idle / mode change / DPMS / monitor hotplug / dock unplug) timestamps every step into
`journal.tsv`, so the wire can be sliced by action afterwards. Every step in that list is there
because its absence once cost real time.

### Phase 3 · the mode matrix ★★★ (15 min)

```bash
sudo tools/capture/capture-modematrix.sh ~/dlcap-modes DP-4
```

This is the highest-value *protocol* work the device enables, and it is worth understanding why
before you run it.

`id=0x48 sub=0x22` is DLM's set-mode. Three of its words are unresolved on the D6000 for a
structural reason rather than a hard one — **the D6000 corpus cannot separate the variables**:

| word | state on the D6000 | what this device changes |
|---|---|---|
| `off42` | resolution-keyed (`0x0600` at 1440p, `0x0400` at 1080p) at every measured refresh — but `VIDEO.md` reads it as a **DP link tier** (1024 = HBR, 1536 = HBR2). Both readings agree on every D6000 mode and disagree on exactly one: 1080p165. That is why 1080p165 is blocked. | two refreshes at **one** resolution separate them: same value ⇒ resolution-keyed, different ⇒ link tier |
| `off66` | moves with refresh at fixed resolution (`0x2810` at 1080p60, `0x083f` at 1080p120) but 1440p is measured at exactly **one** refresh, so the mapping above 1080p is a guess | a second resolution measured at two refreshes |
| `off72` | **zero in every capture ever taken.** Believed to be a pixel-clock overflow field; no clock above 655.35 MHz has ever been on the wire, so DLM literally cannot settle it | a DL-7000 part should not clamp to 120 Hz the way DLM clamps the D6000 to its declared 442,368,000 px/s |

**What reaches 655.35 MHz:** not 4K60 (~561 MHz with blanking — it just misses). **2560×1440@180 is
~750 MHz** and does it comfortably; 1440p165 (~690 MHz) also does. Your panels already offer those
modes, so **no new monitor is needed** — the question is only whether this dock still offers them
once DLM has applied its own clamp. The script prints the estimated clock for the top mode it finds
and says outright whether it reaches the threshold.

Two mechanics the script encodes, both of which have burned time before:

* **DLM only reprograms the dock's timing at CONNECT.** A runtime resolution change makes it
  *scale*, and emits no set-mode at all. So the script **replugs the dock** between modes.
* It replugs rather than restarting DLM **because a restart kills the frida session**, and every
  mode after the first would come back sealed and unreadable. One frida session covers the matrix.
* Modes are addressed as `WxH@rate`, never by index — `kscreen-doctor` renumbers indices between
  calls and a stale index silently sets the wrong mode while still returning 0.

### Phase 4 · what only a 4-head, 2.5G, DL-7000 part can answer (10 min)

| question | how | why it is open |
|---|---|---|
| head id encoding beyond 0/1 | monitor on ports 1, 3, 4 in turn (Phase 2/3 covers most of it) | index vs bitmask are indistinguishable with two heads |
| how many video endpoints | `dl-identity.py` — free, already run | D6000 has four (`0x08`,`0x0a`,`0x0b`,`0x0c`) for two heads; the ratio tells you whether endpoints are per-head |
| does DLM clamp this dock too | compare what `kscreen-doctor` offers against what the decoded set-mode actually programs | the D6000's 120 Hz ceiling is DLM's `pixel_per_second_limit`, and whether that is per-platform is unknown |
| `DISPLAY-CAP` per head | count `id=0x78 sub=0x30` pushes in the connect window | two on the D6000; four here would confirm it is per-head |
| a new codec or framing | the `video` step in Phase 2, then `usb-session-stats.py` | DL-7000 is a new generation; record stride, `aux` padding and sub-band coordinates are all checkable against `WHT-CODEC.md` |
| non-DisplayLink functions | `lsusb -t`, `before-lsusb.txt` | 2.5G LAN, audio and the hub are standard-class and belong to existing kernel drivers; worth documenting, not vino's problem |

### Phase 5 · HDR groundwork — costs nothing extra

This dock is **DL-7000 class**, which is the generation DisplayLink says HDR10 requires. Two pieces
of HDR evidence fall out of captures you are already taking, so take them deliberately rather than
hoping they are in there:

1. **The set-mode, diffed against the D6000's.** Phase 3 produces `id=0x48 sub=0x22` for several
   modes. Diff a DL-7000 set-mode against a Ridge one at the *same* resolution and refresh: any
   word that is structurally new, rather than just numerically different, is a candidate for bit
   depth, pixel format or colourimetry.
2. **The `DISPLAY-CAP` push, `id=0x78 sub=0x30`.** The dock sends this unprompted, per head, during
   connect — so it is already inside the Phase 2 window. It is the per-head capability descriptor,
   and a deep-colour or HDR capability bit would most naturally live there. Count them too: two on
   the D6000, and four here would confirm the descriptor is per-head.

Both are free. Neither requires HDR to be *enabled* anywhere.

What is **not** obtainable tomorrow: HDR10 is a Windows-only feature on this silicon, so the
decisive HDR-off/HDR-on differential capture needs a Windows host. Background, evidence and the full
plan are in **`hdr.md`** — including the finding that the 10-bit wire format (`NM30`) and the FP16
input format are present in the *Linux* DLM binary too, so the dock side of 10-bit is not
Windows-specific.

---

## If the flash does not happen — you get more shots than it looks

This matters, so it is worth being precise: **missing the first contact is recoverable.** The
enforcer flashes on a *mismatch* between the device's build and the image DLM carries, so a
mismatch can be manufactured.

1. **Downgrade the image.** `/opt/displaylink/navarro-dock-release.spkg.6.4.24.0.bak` is right
   there: build `0f416ea2`, 2024-01-31, 1,608,448 B. Whether the enforcer will flash *backwards*
   is unknown — `FindBestFirmwareForElla` suggests it picks a best rather than any difference — but
   it costs one file swap to find out.
2. **Upgrade past the installed image.** macOS DisplayLink Manager 16.2 carries Navarro
   `914777de` (2026-06-18), *newer* than the 6.8.1.0 Linux image `52aef616` (2026-03-27). Dropping
   that in creates a guaranteed forward mismatch. Extraction is xar → **pbzx** → cpio; `7z` stops
   after the first 16 MB chunk, so decode the chunk list properly. The packages land in
   `DisplayLink Manager.app/Contents/Resources/`.
3. Either way, keep a copy of the original `/opt/displaylink/*.spkg` before swapping, and **never
   interrupt a flash** you have deliberately triggered.

A "no flash" result is still a result: read it next to the `bcdDevice` diff. Unchanged means the
enforcer accepted the device's existing build and nothing was missed.

---

## Files to keep

Everything, unedited. `capture-firstcontact.sh` already collects the before/after identity, both
`lsusb` trees, the DLM journal, `dmesg`, the xHCI trace, the four `.spkg` images it compared
against, and its own `run.log`. The wire cannot be recaptured; keys can always be re-extracted
later from the recorded DLM build hash.

```bash
tar czf dlcap-$(date +%Y%m%d).tar.gz ~/dlcap-firstcontact ~/dlcap-keyed ~/dlcap-modes
```

---

## Quick reference

| symptom | cause |
|---|---|
| `dumpcap` cannot initiate capture on `usbmon<N>` | `sudo modprobe usbmon` — not autoloaded unless preflight wrote `modules-load.d` |
| capture empty | wrong bus; `busnum` changes across replugs |
| 0 key candidates | warm dock (no AKE inside the window), or DLM is not the 6.8.1.0 build the offset was derived for |
| keys present, nothing decrypts | keys and wire came from different sessions — they must overlap |
| DLM does nothing | `evdi` not loaded, or the service is still masked |
| mode-set capture looks empty | DLM only reprograms at connect; replug, do not switch live |
| `kscreen-doctor` set the wrong mode and returned 0 | a mode **index** was used; address modes as `WxH@rate` |
