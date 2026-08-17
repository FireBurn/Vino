# Handover

Single current handover. Last updated **2026-08-17**, rewritten from scratch. Everything here is
either still true, still open, or a trap worth not repeating. Anything an earlier handover said and
this one does not is done, superseded or retracted; the previous version is in git history.

---

## START HERE

**All three dock generations now run under one `vino`, at the same time, without interfering.**
That was the open question and the answer is clean. What remains are three independent bugs, one
per area, none of them caused by the others.

⭐ **Each of the three now has a fix in the tree and none of them has been tested on hardware.**
The next session's job is one run per dock, in the order below; see
[Landed on 2026-08-17](#landed-on-2026-08-17-all-three-unvalidated-on-hardware).

| dock | USB | state |
|---|---|---|
| **Ella** HP 3005pr, DL-3900 | `17e9:430a` | Lights both panels and drives the desktop. Dies under sustained load -- see [The EPIPE](#the-epipe-ellas-open-bug). |
| **Navarro** DL-7400 | `17e9:7000` | Session, EDID and mode-set all good. First content frame stalls EP08 forever -- see [The EP08 wall](#the-ep08-wall-navarros-open-bug). |
| **Ridge** D6000, DL-6xxx | `17e9:6006` | Holds a control session and a keepalive. Not exercised with a monitor recently. |

Concurrency evidence, so nobody re-opens it: all three bind as separate cards with distinct DRM
minors; the D6000 was hot-plugged into a live two-panel Ella session and came up clean without
disturbing it; Navarro failed on its first attempt, before the D6000 was plugged in at all, and
reproduces with vino unbound entirely; Ella's EPIPE struck while Navarro was physically unplugged.
The only driver-global mutable state is `static ENCODE_WQ` in `drm_sink/scanout.rs`, one encode
workqueue shared by every dock -- a fairness question under simultaneous scanout, not a correctness
one.

---

## Open work, in priority order

### 1. The EP08 wall -- Navarro's open bug

⭐ **A candidate cause is identified and fixed in the tree; it needs one hardware run.** See
[what the code review found](#what-the-code-review-of-the-ep08-wall-found) below -- the short
version is that the per-strip parameter map goes part-way through a frame's image records on the
carrier path and after all of them on the scanout path, and the carrier path's own comment records
that the second placement is what leaves the endpoint un-drained. `6112303b251d` gives both paths
the same placement.

⛔ **It is NOT a DL-3x00 regression, and the search space in the previous handover was wrong.** See
below; do not spend hardware time bisecting the Ella series for it.

Navarro reaches a **live, correct** state and then cannot move pixels: control session up, EDID
read (MSI, 2560x1440), mode-set sent, training complete, `KMS CRTC enable`. Then every content
frame fails identically:

```
head=0 endpoint=0x08 persistent video queue opened (depth=8, 65536 B URBs)
scanout head=0 pipeline submit at off=524288/2212608 failed
head=0 retired failed physical video queue (ETIMEDOUT)
```

`524288 = 8 x 65536` -- the full URB queue. Not one of the eight ever completes. Across 81
observed failures: 72 at `off=524288`, 5 at `65536`, 4 at `0`.

What this rules in and out:

- ⛔ **Not a halt.** vino's own `GET_STATUS` sample reads `0x0000 halt=0`.
- ⛔ **Not a decode or framing fault.** A dock choking on bad records still drains the endpoint and
  completes the transfer, then shows corruption. Never completing means the bytes are not being
  accepted at the USB level, before any decode.
- ⭐ **The dock is otherwise fully alive.** With the panel black, the **hardware cursor still moves
  on it**. The cursor is a CP record on EP02; the desktop is EP08 bulk. So mode-set landed, the
  downstream sink is powered, the dock's scanout engine is running and compositing, and the control
  session is healthy. The failure is isolated to the EP08 bulk path alone.
- ⭐ **Flat frames pass, content frames do not.** The startup/training frame (13 chunks, all class 0,
  longest 54 B) goes through. The first real desktop frame (147 chunks, classes
  `[679, 2833, 76, 12]`, longest 1888 B) does not.

#### What the code review of the EP08 wall found

⛔ **"This is a regression from the DL-3x00 work" is RETRACTED.** Three things say otherwise:

- `a13775e0cdc5`, called the known-good revision, is from **2026-07-28** -- before the DL7400 was
  ever driven and before the parameter map existed. It is a Ridge reference and says nothing about
  Navarro.
- The same symptom is recorded on **2026-08-05**, days before the Ella series began: *"`edid_override`
  WORKS but the first CONTENT frame stalls EP08."*
- Both leading candidates are **no-ops for this dock**, by inspection:
  - `a6411f11524b` changed `full && !video_on_ctrl_pipe()` to `full`. Navarro is not on the control
    pipe, so it took the `dock_buffers` branch before the commit and takes it after. Zero change.
  - `f52d2ff30699` is gated on a stream budget, and Navarro's profile is `StreamPacing::UNMETERED`,
    which returns before the ledger is touched. Zero change.

⭐ **What the dmesg actually shows**, read against the code (`captures/ella-socket1-20260817/dmesg.txt`
around t=6197, which recorded the Navarro dock as well):

- The five carrier frames are **consumed** -- 210,048 B each, presented over 400 ms, the endpoint
  draining between them. The dock is alive and taking bulk video.
- The first content frame opens a **fresh** queue, so its eight 64-KiB transfers are all new
  submissions and none of them completes: the dock accepts **nothing at all** of it.
- So the discriminator is the frame, not the endpoint and not the rate.

⭐ **The difference between the two frames is where the parameter map sits.** `submit_prompt_training`
splices it `NAVARRO_PARAM_IMAGE_OFFSET` (115,168) bytes into the image records, as the vendor does,
and carries a comment recording what the other placement cost: *"putting the same valid records
after all pixels lets two presentations complete, then leaves the first transfer of frame three
permanently pending."* `encode_and_send_wht` built exactly that frame -- its scatter list was
`[chunks][parameter map][trailer]`. The fix gives both paths the same placement, rounded back to a
chunk boundary on a content frame because a chunk holds whole records and the raw byte offset does
not.

⚠ **Unproven until it runs.** What is established is that the two paths disagreed and that the
working one is the one that matches the vendor. If the panel is still black afterwards, the next
thing to look at is the map's *contents* on a content frame: the vendor's quiescent frames carry an
all-zero map, `docs/protocol/navarro-decoded.md` §3.4 warns "do not invent values", and this is the
first frame on which vino sends non-zero ones (`classes [3400, 200, 0, 0]`).

⛔ **`07124a5a3ca1` ("never end a frame on a full packet") is NOT the cause**, though it looks like
it. It only modifies a frame's *last* transfer, and Navarro dies on the first eight URBs. Its
condition never fires on the failing frame anyway: the final chunk is 49,920 B, which is
`48 x 1024 + 768`, not a multiple of the packet size. Eliminated -- do not re-chase it.

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

⛔⛔ **vino's own recovery is what leaves it dead** -- ✅ **root-caused and fixed** (`53b8fa4cd159`),
needs a hardware run. It correctly resets the dock and then never re-runs bring-up; the card sits
there with connectors still reporting `connected`, KWin still flipping into a dock that is not
listening, and only a manual driver rebind brings it back.

⭐ **A reset does not re-probe when the driver supplies `pre_reset` and `post_reset`.** Supplying
the pair is how a driver tells the USB core it can carry its state across a reset, and the core
then leaves it bound and just calls them. The Rust `usb::Driver` adapter installs both callbacks
for *every* driver, and their default body returns success -- so vino was claiming a session
survived the reset that destroys it. `usb_reset_device()` rebinds an interface whose `post_reset`
returns non-zero (`drivers/usb/core/hub.c`, and `hid_post_reset` is the in-tree precedent), which
is the manual rebind, done by the driver at the moment it knows one is needed.

⚠ **This is the "never comes back" half only.** What causes the EPIPE in the first place is still
open, and the cross-head budget below is still the lead.

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

## Landed on 2026-08-17, all three unvalidated on hardware

These are one commit per open bug above, each building warning-clean on its own so a bad hardware
result is bisectable. **Nothing here has been near a dock.** Both docks were attached and vino was
unloaded while they were written, and loading it would have spent Ella's bring-up.

- `6112303b251d` **Put a DL7400's parameter map among its records.** Open bug 1.
- `53b8fa4cd159` **Come back from the reset that recovers a wedged dock.** Open bug 3's second half.
- `88e0ab5de5e6` **Derive a DL-3x00's framebuffer row count from the width.** Open bug 2.
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
