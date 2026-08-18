# Handover

Single current handover. Last updated **2026-08-17**, rewritten from scratch. Everything here is
either still true, still open, or a trap worth not repeating. Anything an earlier handover said and
this one does not is done, superseded or retracted; the previous version is in git history.

---

## START HERE

**All three dock generations now run under one `vino`, at the same time, without interfering.**
That was the open question and the answer is clean. What remains are three independent bugs, one
per area, none of them caused by the others.

⭐⭐ **The DL7400 paints a desktop again** (user-confirmed 2026-08-17, 2560x1440). That was the
worst of the three and it is closed; what is left is the EPIPE's *cause* on Ella, and hardware runs
for two fixes that have not been exercised.

| dock | USB | state |
|---|---|---|
| **Ella** HP 3005pr, DL-3900 | `17e9:430a` | Lights both panels and drives the desktop. Dies under sustained load, and now recovers by itself -- see [The EPIPE](#3-the-epipe----ellas-open-bug). |
| **Navarro** DL-7400 | `17e9:7000` | ✅ **Drives a 2560x1440 desktop.** See [The EP08 wall](#1-the-ep08-wall----solved-2026-08-17-hw-verified). |
| **Ridge** D6000, DL-6xxx | `17e9:6006` | Holds a control session and a keepalive. Not exercised with a monitor recently. ⚠ It shares the carrier loop the EP08 fix touched. |

Concurrency evidence, so nobody re-opens it: all three bind as separate cards with distinct DRM
minors; the D6000 was hot-plugged into a live two-panel Ella session and came up clean without
disturbing it; Navarro failed on its first attempt, before the D6000 was plugged in at all, and
reproduces with vino unbound entirely; Ella's EPIPE struck while Navarro was physically unplugged.
The only driver-global mutable state is `static ENCODE_WQ` in `drm_sink/scanout.rs`, one encode
workqueue shared by every dock -- a fairness question under simultaneous scanout, not a correctness
one.

---

## Open work, in priority order

### 1. The EP08 wall -- ✅ SOLVED 2026-08-17, HW-verified

`be8d71890581`. The DL7400 paints a desktop at 2560x1440: user-confirmed, with the wire showing
full 3,936,656-byte keyframes accepted where none had ever been.

⭐ **A ring slot was spent on a frame the dock never saw.** The carrier loop counted ring slots
above the send, and its deferral path -- taken when the endpoint cannot yet accept a whole frame --
`continue`s to the top without sending. Each deferred pass therefore advanced the frame counter.
Since the ring phase is that counter modulo the ring depth, a step equal to the depth leaves the
phase unchanged, so from the fifth frame on **every frame told a three-buffer dock it was filling
slot 0** -- the buffer it was scanning out. The dock stopped taking bytes. The fix counts the slot
once the presentation has gone out, beside the frame count that already worked that way.

Introduced by `f72590773852`, which replaced `repeat` (incremented after a successful send) with
`named` (incremented before it).

⚠ **Why this resisted every byte comparison.** No single frame is malformed. Record grammar,
trailer, decoder configuration and the whole control plane compare *identical* against a working
revision -- only the sequence across frames is wrong. And it is timing-sensitive: whether the
deferral fires depends on how fast the dock drains, so adding log lines around the counter was by
itself enough to make a failing build pass. ⛔ Do not conclude anything from a single passing run,
and do not conclude "the bytes are the same" means "the stream is the same".

⭐⭐ **The method is the reusable part, and it is cheap.** The failure is machine-detectable, so it
needs no eyes and no judgement:

```
load with debug=1, wait 40 s, then read dmesg:
  "strip map 3600 strip(s)"        the 2560x1440 frame was attempted
  "pipeline submit at off=... failed"   the dock refused it
```

That drove a `git bisect run` over 35 commits unattended (~2 min per step), then a within-commit
split by reverting file groups, then this. ⚠ Unplug the other dock first: swapping the module means
unloading it, which takes every dock down.

⭐ **The decisive instrument was the ring sequence itself**, read off the frame trailer of each
EP08 frame group -- `slot` at offset 19, `ring` at 22, `frame_no` at 25. Good cycles
`0,2,4,0,2,4` with `frame_no` stepping by one; broken sticks at slot 0. Reference captures:
`~/vinocap/nav-good.pcapng`, `nav-bad.pcapng`, `nav-fixed.pcapng` (not committed).

### 2. `off48` is wrong on every mode set -- ✅ FIXED, needs a hardware run

`88e0ab5de5e6`. The DL-3x00 allocation is now derived rather than looked up, so every resolution
gets the row count the vendor sends instead of a family default. The values are unchanged at
1920x1080, the one resolution the old table covered, so a mode sweep is what tests this.

⭐ The vendor's own serializer settles the general rule, and it agrees with the closed form:
`rows = allocation_bytes / (render_stride * bytes_per_pixel)`. A DL-3x00 partitions **48 MiB** per
head, and `48 MiB / 3` is the `2^24` in the formula below. The depth belongs in the division, so
`timing_from_drm_mode` now takes it as an argument rather than having the caller patch it on
afterwards -- a 30 bpp head is told three quarters of the rows a 24 bpp one is.

⚠ **Navarro does not fit this and keeps its measured table.** Its two captured values imply
allocations of 202.5 MiB at 2560 wide and 58.5 MiB at 640, which is not one constant; Ridge keeps
its device-level override. Do not unify the three.

`off48` depends on **hactive alone** -- unchanged by vactive (1280x1024, 1280x960 and 1280x720 all
give 11915), by refresh (1280x1024 at both 60.02 and 75.02 gives 11915), and by sink (identical in
both sweep captures). Closed form, exact for all six measured widths:

```
off48 = 16777216 // (round_up_128(hactive) + 128)
```

| hactive | padded | off48 |
|---|---|---|
| 2560 | 2560 | 6241 |
| 1920 | 1920 | 8192 |
| 1280 | 1280 | 11915 |
| 1024 | 1024 | 14563 |
| 800 | **896** | 16384 |
| 640 | 640 | 21845 |

The padding is what gives the shape away -- 800 is the only width that is not a multiple of 128 and
it behaves as 896. So it is `2^24` over a padded line width, a fixed-point step per pixel. The
driver's name for the field ("measured row count") is wrong and should change with the value.

### 3. The EPIPE -- Ella's open bug

Reproduced three times in twenty minutes on 2026-08-17 (t=5200, 6539, 7310), always under
sustained repaint, never on a settled desktop. The session dies with:

```
dock has answered nothing for 90141 ms; abandoning the session
   -- or --
shared video/control pipe failed (EPIPE); abandoning the session
```

⛔⛔ **vino's own recovery is what leaves it dead** -- ✅ **root-caused, fixed and HW-VERIFIED**
(`53b8fa4cd159`). It used to reset the dock and then never re-run bring-up; the card sat there with
connectors still reporting `connected`, KWin still flipping into a dock that was not listening, and
only a manual driver rebind brought it back. Measured on 2026-08-17, the whole cycle now runs
unattended in about six seconds:

```
shared video/control pipe failed (EPIPE); abandoning the session
resetting the dock to recover the control session
reset complete; rebinding for a fresh session
DL-3x00 dock (Ella, DL-3900)   [re-probe]  ...  socket 1 monitor connected
```

⭐ **A reset does not re-probe when the driver supplies `pre_reset` and `post_reset`.** Supplying
the pair is how a driver tells the USB core it can carry its state across a reset, and the core
then leaves it bound and just calls them. The Rust `usb::Driver` adapter installs both callbacks
for *every* driver, and their default body returns success -- so vino was claiming a session
survived the reset that destroys it. `usb_reset_device()` rebinds an interface whose `post_reset`
returns non-zero (`drivers/usb/core/hub.c`, and `hid_post_reset` is the in-tree precedent), which
is the manual rebind, done by the driver at the moment it knows one is needed.

⚠ **This is the "never comes back" half only.** What causes the EPIPE in the first place is still
open, and the cross-head budget below is still the lead.

⭐⭐ **Half the bytes were going to a socket with no monitor on it, and that is what EPIPE'd.**
Measured 2026-08-17: Ella offers both connectors, and DP-9 reports `connected` with **zero** bytes
of EDID where DP-8 carries 256. The compositor enables and paints the empty one, and the session
that died did so on `head=1` -- the phantom -- pushing a frame onto the pipe the control plane
shares. `0948e647efd0` stops painting a head with no EDID; the connector is still offered, because
hiding it was measured to stop the panel lighting at all (`a3a153182547`).

✅ **The phantom is gone, HW-verified 2026-08-17** (`af03f95fb956`). KWin now sees exactly the
sockets with a monitor in them: `DP-6` connected with 256 B of EDID, `DP-7` disconnected.

⭐ It took three attempts because the empty connector was **load-bearing**, not slack. This dock
activates as one transaction over every connector it has, and the transaction is assembled from
what the compositor enabled -- so hiding the empty socket left one timing where two are needed, the
dock fell to the per-head path, and the panel did not light (`a3a153182547`).

⭐⭐ **A synthesised head needs a mode generation, not just a timing.** The second attempt gave it
a timing alone. A head whose requested mode is zero is a head the activation waits on and never
gets:

```
KMS batch -- dual timings 2, requested [<gen> 0 0 0], active [0 0 0 0]
atomic multihead KMS batch deferred
```

It deferred again several times a second for as long as the dock was up, and that retry churn on
the shared pipe ended in EPIPE, a reset, and **both** connectors going away -- including the one
with a monitor. Publishing `timing_key()` beside the timing fixes both halves: the activation
accepts the head, and the head stops looking unasked-for, so the synthesis is once-only without a
separate guard.

**The vendor reference now exists** (`captures/ella-coldplug-load-20260817` and
`captures/ella-twohead-load-20260817`, both from a cold plug, both with their own keys):

| heads | DLM sustains |
|---|---|
| 1 | ~59 MB/s |
| 2 | ~75 MB/s |

⭐ **Two heads get 1.27x, not 2x.** On a dock where video and control share one bulk pipe, DLM holds
the total near a ceiling and gives each head less rather than letting each draw what it would draw
alone. vino has no cross-head budget at all -- each head encodes and submits independently, which
is exactly the shape that ends in EPIPE.

⚠ A lead, not a proof. What is measured is DLM's *rate*; how it paces to achieve that rate, and
whether matching it prevents the EPIPE, are not established. The wire in those captures settles it.
⚠ Both MB/s figures are capture-file growth, so they include usbmon overhead and are approximate --
same recorder and method for both, so the ratio holds, but quote `usb-session-stats.py` for bytes.

---

## Landed on 2026-08-17

Each builds warning-clean on its own, so a bad hardware result is bisectable. Selftests read
`pass:88 fail:0` on hardware.

- `be8d71890581` **Spend a ring slot only on a frame the dock received.** ✅ HW-verified -- the
  DL7400 paints a desktop. Open bug 1.
- `af03f95fb956` **Stop offering a DL-3x00 socket with nothing plugged into it.** ✅ HW-verified --
  the phantom screen is gone and both panels keep working.
- `53b8fa4cd159` **Come back from the reset that recovers a wedged dock.** ✅ HW-verified.
- `0948e647efd0` **Stop painting a DL-3x00 socket with nothing plugged into it.** ✅ HW-verified:
  `scanout head=1 deferred: no monitor has described this socket`, once per repaint, on the
  connector with no EDID.
- `6470d5ff7d9d` **Open a stream with the carrier frames the vendor sends.** ⚠ Not the EP08 fix,
  and not independently validated -- it was in the tree when that fix was measured.
  The carrier was bounded by a 400 ms window rather than a count, so the *same* DL7400 at the same
  mode opened a stream with **four** carrier frames when its endpoint was draining slowly and
  **852** when it was not -- and every one of them walks the dock's ring and steps its frame
  counter. Now profile data: 5 for the DL7400, 1 for Ella, and the DL-6xxx keeps its measured
  window.
- `88e0ab5de5e6` **Derive a DL-3x00's framebuffer row count from the width.** ⚠ Not exercised --
  the values are unchanged at 1920x1080, which is what Ella ran. A mode sweep is what tests it.
- `6112303b251d` **Put a DL7400's parameter map among its records.** ⚠ Did **not** fix the EP08
  wall; see open bug 1.
- `9c391ecae94c` Sort the KUnit re-exports -- the tree was not `rustfmt`-clean at HEAD.

⚠ A build gotcha worth keeping: an ordinary `/` on a value the compiler cannot prove non-zero adds
a panic path that objtool reports as `falls through to next function`. `checked_div` removes both
the warning and the trap.

## Landed earlier on 2026-08-17

- `5ba58eae7cdd` **Build as many heads as the dock has sockets.** `create_objects` used `HEADS` (4,
  the maximum any dock has) as the number of KMS objects to build, so two-connector docks published
  four outputs. Not cosmetic: with the phantom connector reporting `connected`, KWin enabled Ella's
  empty socket and the driver encoded and pushed a **1,491,824-byte keyframe at a socket with no
  monitor**, onto the same shared pipe that then EPIPE'd. The count now arrives through the
  constructor, because `create_objects` runs inside registration before probe publishes the
  profile -- which is already why the ten-bit and cursor flags travel that way. Verified live: 2/2/4
  connectors for Ridge/Ella/Navarro. Selftests `pass:86 fail:0`.
- `a4b7fee4aead` + `495275b53ae7` **Power the sinks down on unbind.** New `SOFT_UNBIND` binding in
  `rust/kernel/usb.rs` plus `park_sinks()` from `quiesce()`. Without it the dock kept scanning out
  its last decoded frame and both monitors stayed lit on a frozen desktop after unbind. It cannot
  be done from `disconnect()`: that runs after I/O is revoked, and a transfer issued into a
  disconnect deadlocks `usb_hub_wq`.
- `4d0ac6a` + `044f6a1` The mode-sweep captures and their analysis (see below).

---

## What the 2026-08-17 captures established

Two `kscreen-doctor` sweeps under DLM, ten `id=0x48 sub=0x22` records each, on two different sinks.
The timing block is fully decoded and **self-validating**: all ten pixel clocks match the VESA
standard for their mode exactly, and refresh recomputed as `clk / (htotal * vtotal)` matches the
independently decoded `off44` every time.

```
off 26 hactive   off 28 hblank   off 30 hfront   off 32 hsync
off 34 vactive   off 36 vblank   off 38 vfront   off 40 vsync
off 70 pixel clock, units of 10 kHz        htotal = hactive + hblank
```

⛔ **Timing follows the SINK. Never build a static per-mode timing table.** Same dock, same mode,
two sinks: the HDMI monitor got `2720x1474 @ 200.25 MHz = 49.95 Hz`; the DisplayPort monitor got
`2720x1481 @ 241.50 MHz = 59.95 Hz`. An earlier version of this document read that as "the DL-3900
clamps 1440p to 50 Hz, like the DL-6xxx clamps 180 to 120" -- **retracted**, it was the HDMI sink's
own ceiling. A table built from the first capture would have programmed 200 MHz into a monitor
asking for 241. Derive timing from the mode and the EDID, the ordinary DRM way.

⭐ Constant in all twenty records: `off22 = 0`, `off23 = 2`, `off68 = 0x0200`, `off69 = 2`,
`off72 = 0`. `off42` is sync polarity and follows the mode: high-byte bit 0 is hsync-negative,
bit 1 is vsync-negative. Its low byte was `0x80` only on the single reduced-blanking mode -- one
sample, a hypothesis, not a fact.

⚠ **Read the resolution out of the record's timing block, never off what was requested.** Three of
ten DisplayPort steps were silently substituted (1680x1050 and 1440x900 became 1920x1080;
1280x960 became 1280x1024@75). ⚠ DLM **synthesises** the advertised mode list -- byte-identical for
both sinks, and capping 1440p at 59.95 on a panel that does 165.

---

## Watch list -- observed, not explained

### ⚠ A DL7400 sink flap, and a dark panel that a dock restart cleared

Seen 2026-08-17 with both docks bound. The DL7400's presence probe flapped every ~20 s --
`present=false ready=false` then `present=true ready=true` -- each one absorbed by the debounce
(`socket 1 sink flap healed on its own`), so the connector was never dropped. Meanwhile the panel
was dark and the scanout path reported `no keyframe owed and no strip content changed` for three
minutes: the driver believed the dock's framebuffer was current.

⛔ **Do not "fix" this by owing a keyframe on every healed flap.** It is an appealing theory -- a
sink that went away and came back holds nothing, so the shadow this side diffs against describes a
framebuffer that no longer exists -- but it is unproven, and the evidence against acting on it is
that **a dock restart cleared the darkness with no code change**. The flaps themselves are
long-standing and normally benign; the surrounding comment records that re-driving the head per flap
costs a full dock-wide re-activation and puts the dock in a permanent loop. A keyframe is cheaper
than that, but it is still a 3.9 MB frame every twenty seconds bought on a hunch.

⇒ What would settle it: catch the dark state again and check whether forcing a repaint
(a mode toggle, or anything that calls `owe_keyframe`) lights the panel. If it does, the
invalidation is right and belongs at the heal site.

## Traps -- physical and procedural

### ⛔⛔ The dock latches its connector set at power-on

Established by elimination on the same dock and cables. A socket the dock did not see when it
powered on does not become usable by re-running anything in software:

| action | outputs DLM created |
|---|---|
| monitor plugged into a running dock | 1 |
| ... then a fresh session (`authorized` 0 -> 1) | 1 |
| ... then a full DLM restart with both attached | 1 |
| **power cycle with both attached** | **2** |

This matters beyond captures: vino was suspected of a presence bug on Ella's second socket, and the
vendor stack behaves identically, so "socket 2 reports absent" is not by itself a driver fault.

### ⛔⛔ Navarro can refuse `CLEAR_FEATURE(ENDPOINT_HALT)`, and only a power cycle clears it

Seen on 2026-08-17: standard `CLEAR_FEATURE(HALT)` on EP `0x08` and `0x0a` timed out at ~2 s each
while every other EP0 request answered in 0.1 ms. Reproduced **with vino unbound**, straight from
usbfs, so it is the dock, not the driver. A `USBDEVFS_RESET` did **not** clear it. The physical
power cycle did. Interface 0 alt 0 declares all of `0x08 0x0a 0x84 0x02`, so the request was always
legitimate.

### ⛔⛔ Hold vino off before a replug meant for DLM

vino autoloads by modalias and will take the display function the instant the dock enumerates,
racing DLM and spending the bring-up. Blacklist it first:

```
/etc/modprobe.d/zz-temp-vino-blacklist.conf:
    blacklist vino
    install vino /bin/false
```

⚠ **Remember to remove it.** Nothing comes back under vino while that file exists.

### ⛔ Docks change USB bus between plugs

Ella ran as `4-2.1` for most of one boot and as `2-2.1` later the same boot. Never hardcode a
sysfs path; re-derive it from `idVendor:idProduct` every time.

### ⛔ Start recorders before any physical action, and prove they are writing

A cold plug is the most expensive event to reproduce and cannot be recovered afterwards. One was
lost on 2026-08-17 by starting the recorders after the power cycle. The order is: start recorders,
confirm the wire file is *growing*, then ask for the physical action.

---

## Capture recipe, as actually used

```
sudo modprobe -r vino          # plus the blacklist above
sudo modprobe evdi             # DLM does nothing without it
( cd /opt/displaylink && sudo ./DisplayLinkManager )     # unit stays MASKED
sudo tools/capture/capture-newdevice.sh <outdir> <seconds>
```

⚠ `capture-newdevice.sh` attaches frida to a **running** DLM -- start DLM first or the run is
wire-only and the CP is opaque.
⚠ **CP crypto is dormant on a warm dock.** The capture must span a real connect or there is no AKE
and no keys. A physical replug is not required: USB `authorized` 0 -> 1 forces a full
re-enumeration and a real AKE. Use a physical power cycle only when the dock's own power-on
behaviour is the thing being measured.
⛔ `capture-firstcontact.sh` is for the one-shot **firmware flash**, not for ordinary captures.

Reading a capture:

```
tools/capture/decrypt-dlm-cp.py wire.pcapng keys.candidates.json --full
tools/capture/usb-session-stats.py wire.pcapng        # exact endpoint byte accounting
tools/capture/setmode-table.py <decrypted.txt> <journal.tsv>
tools/capture/setmode-diff.py  <dec-a> <journal-a> <dec-b> <journal-b>
tools/capture/ella-modesweep.sh <output> <journal.tsv>  # run as the DESKTOP USER
```

`wire.pcapng` files are **not** committed -- no capture in this repo carries one. Keep the summary,
the journal and the keys.

---

## Captures on disk from 2026-08-17

| directory | what it holds |
|---|---|
| `ella-socket1-20260817` | Cold session, HDMI sink. 15 key candidates, 56 sealed frames decrypted. |
| `ella-modesweep-20260817` | Ten set-mode records, HDMI sink. ⚠ **Rides socket1's keys -- not standalone.** |
| `ella-modesweep-dp-20260817` | Ten set-mode records, DisplayPort sink. Own AKE, decrypts standalone. |
| `ella-coldplug-load-20260817` | Cold plug + 100 s load, **one** head. ~59 MB/s. 26 keys. |
| `ella-twohead-load-20260817` | Cold plug + load, **two** heads. ~75 MB/s. 19 keys. |

---

## Environment

```
Kernel tree:   vino/linux   (branch `vino`, CONFIG_RUST=y)
Driver:        drivers/gpu/drm/vino/{vino,cp,drm_sink,video,ake,hdcp,proto,crypto,rng,profile,session,usb_link}.rs
PATH:          export PATH=/usr/lib/llvm/22/bin:$PATH

Build only:    make LLVM=1 -j16 M=drivers/gpu/drm/vino modules
Place it:      sudo cp drivers/gpu/drm/vino/vino.ko /lib/modules/$(uname -r)/kernel/drivers/gpu/drm/vino/
               sudo depmod -a
```

⛔ **Never put `M=` on a `modules_install` line** -- it installs to `updates/`, which `depmod`
prefers, so a stray copy silently shadows the real module and every later reinstall appears to do
nothing.
⛔ Always `make LLVM=1`, never `LLVM=/path` -- Kbuild records the literal string and mixing the two
forms rebuilds the whole tree.
⚠ A green build can mean nothing was built: require a real `RUSTC [M]` / `LD [M]` line.
⚠ Use `modprobe` / `modprobe -r` and the `remove_all` sysfs attribute, never raw `insmod`/`rmmod`.
⚠ `vino.ko` only loads on the matching kernel; the user builds, installs and reboots kernels.
