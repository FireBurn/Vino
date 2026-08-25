# Handover

## 2026-08-25 -- 30 bpp drives both DL7400 panels; every escape ceiling is stated by a code table

Both DL7400 panels are confirmed by eye at 2560x1440p120 in PQ at 30 bpp, reported as `10 Bit` by
the monitors. What stood between the wire being right and the picture being right was the decoder
code tables.

`CODE_TABLES` are not opaque constants: each is the series `2^n * (2^(n+1) - 1)` truncated at a
category, and a table *is* one plane's escape codebook with `naturals = cmax - 1`. A coefficient is
four times the sample, so **every** depth-sensitive ceiling gains two categories at 30 bpp -- luma
AC nine to eleven, chroma AC and DC ten to twelve -- and each has to be raised in its own table as
well as in the encoder. The mapping is table 0 luma AC, table 1 chroma AC, table 2 DC; the last two
are indistinguishable at 24 bpp because they share a ceiling there. Mechanism, evidence and the
two very different failure shapes are in [`hdr.md`](hdr.md) §0.2a.

⛔ **Do not use the repository decoder to check this.** It takes `cmax` from `Depth`, not from the
tables in the stream, so vino's own 30 bpp wire decodes perfectly against a screenshot while the
dock cannot decode it at all. Round-tripping proves nothing; only vendor bytes or a panel do.

Three other things settled on hardware the same day:

* **1440p165 is a 24 bpp mode on this dock.** Two connectors at that refresh sit exactly on the
  pixel budget, and 30 bpp is priced against three quarters of it. Raising the budget to the rated
  four-connector 4K60 load admits the pair and both sinks power off with nothing logged.
* **A warm plug can be answered from the dock's own bridge descriptor** -- `NVT` / `0x079c`, a
  1920x1080 panel, valid magic and checksum. It is now refused on identity. Gating on the presence
  reply's readiness bit is necessary but not sufficient, and that bit is a different property from
  `shared_edid_handler`; see [`navarro.md`](navarro.md) §4a.
* **A bring-up can report complete success and leave the panels dark**, cleared by a second mode
  set. Unexplained, and the open item most worth taking next.

⚠ `modprobe vino debug=1` fills the kernel ring buffer in about four minutes -- 40,000 lines, 4 MB
-- and `dmesg` then shows only the last couple of minutes. Read `journalctl -k -o short-monotonic`
instead, and check `dmesg | head -1` before believing that an event is absent.


## 2026-08-22 -- Ridge lights the panel; the EDID it was reading described the dock

A like-for-like DLM reference for this dock now exists: `captures/ridge-dlm-ref-20260822`, 841 s,
1362 control records decrypted, video endpoint included, current firmware, MSI MAG 27CQ6F on socket
2. vino's matching session is `captures/ridge-vino-ref-20260822`. Full write-up in
[`ridge.md`](ridge.md); the cross-generation table it belongs to is
[`protocol/generations.md`](protocol/generations.md).

⛔ **Two things this document previously asserted are wrong and are now corrected.**

* "There is currently no obtainable like-for-like DLM reference for this dock; the only DLM corpus
  is `captures/max-cold-20260721-235609`, on older firmware." Both halves are false. The archive
  holds dozens of D6000 DLM captures, every recorded `pid=6006` is `bcdDevice=3159`, and no `.spkg`
  for this family ships in `/opt/displaylink`, so nothing has ever flashed it. **The firmware has
  not moved.** DLM run by hand drives this dock; the run that failed had simply hit the wedge.
* "This dock no longer sends its DISPLAY-CAP push ... 596 frames and not one `id=0x78`." It sends
  three per session, `id=0x78 sub=0x30`, 160 B, byte-identical to each other. They are **dock-wide**,
  not per-head, and carry no presence information -- presence is `0x44/0x20` alone.

### ⭐⭐ The cause: one EDID handler, shared between the heads

A `0x15/0x21` fetch does not read the monitor named at offset 22. It reads whichever head the dock's
handler is engaged for, and engaging it for one head disengages it for the other. The presence reply
says which answer is coming: **offset 26 bit 7** is set once the downstream DDC read has completed.

| socket | inner 22..26 | off26 | the next fetch returns |
|---|---|---|---|
| empty | `05 01 20 00` | `00` | the other head's monitor, or nothing |
| occupied, not engaged | `05 11 27 00` | `00` | the dock's own 1920x1080 block |
| occupied, engaged | `05 11 27 00` | `80` | the monitor |

Four cases, two independent captures, agreeing every time. `cp::probe_reply_status` already decoded
both fields; nothing acted on `ready`.

That one fact produced both symptoms. Cold boot took the dock's 256-byte NOVATEK 1920x1080
descriptor and drove the panel at a timing it never advertised. At runtime the keepalive spent a
blind re-engage on the empty socket, which stole the handler and returned the *other* socket's
monitor -- so a single MSI panel ping-ponged between sockets, tearing its connector down each time.

⚠ **Why it was intermittent:** on a warm rebind the handler is still engaged from the previous
session, so the first fetch returns the real EDID and everything looks correct. **Only a cold dock
reproduces it.** Several earlier measurements were taken after a rebind and proved nothing.

### Landed, HW-verified

`DockProfile::shared_edid_handler` (Ridge `true`, Navarro and Ella `false`, so both are inert).

* EDID acceptance gated on the readiness bit in `session.rs`. On hardware:
  `socket 2 discarding an EDID offered before the downstream read completed` followed by
  `cached socket 2 EDID (384 bytes)`.
* No blind re-engage of a socket the probe reports absent. Zero
  `monitor connected after sink re-engagement` across a full session, where there were six.
* Ridge now runs **2560x1440** on the real MSI EDID; Ella is unchanged at 1920x1080 with its second
  connector correctly disconnected. Module `d71fb371803f4228...`.
* `frame_period_ms` 5 -> 8 for Ridge, matching the vendor's measured floor. ⚠ This fixed nothing;
  it is kept only because undercutting the vendor by three times was never justified.

### ✅ SOLVED: the video endpoint stopped accepting because a frame was split in two

`never end a frame on a full packet` sends a frame whose length `N` is a multiple of the 1024-byte
packet as `N - 16` then `16`. `N - 16` is 1008 modulo 1024, so the *first* transfer ends short too:
the dock sees the frame end sixteen bytes early and a stray sixteen-byte frame behind it. The split
cannot do what it is named for -- a multiple of the packet size cannot be divided into two transfers
where only the last is short; that needs a zero-length packet.

Three captures, the failure cascade beginning 49.0 / 46.8 / 50.7 ms after the first such transfer,
one of them containing exactly one transfer and one cascade. DLM emits none. vino's own known-good
July capture predates the change.

Now `DockProfile::split_full_packet_frame` -- `true` for DL-3x00 where it was measured, `false`
elsewhere. ✅ HW-verified: the reproducer no longer reproduces, and the wire shows 0 sixteen-byte
transfers and 0 failing completions over 2865 frames.

⭐⭐ **Method, and it cost five wrong fixes:** the report said "this used to work". That makes it a
regression, and a regression is found by diffing against the last good version of *this* driver, not
by decoding the vendor harder. Throughput, ring slots, frame rate, the record `sub` bit and a
coverage floor were each a real measured difference from DLM, each tested on hardware, and none was
the change that broke the dock. Full table in [`ridge.md`](ridge.md).

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

### ⭐ The D6000: EDID fixed, video still dark

**EDID -- ✅ fixed** (`56b57895d5cc`, `c841c1336109`). The keepalive skips its sink re-engage
whenever the presence probe answers negative, but this family reports a head absent *while its EDID
handler is not engaged for that head* -- so the loop waited for a positive that only the skipped
call could produce. Blind re-engagements are now spent on a socket that has **never** answered,
bounded to ten and only while nothing on that dock is lit.

⚠ **This was never the regression.** At `4f7551789675` -- the last commit before the DL-3x00 series
-- the D6000's cold discovery fails *identically* (`socket 1/2 -- cap:no edid:no`) and is rescued by
the same re-engage path, which fires there only because the socket has not answered yet. The fix
makes a rescue that was always load-bearing reliable; it does not restore anything.

⛔ **Video -- OPEN. The dock takes every byte and does not light the sink.** Measured across a
fresh load: **1464 submissions on `ep 0x0b`, 87,981,136 bytes, 1456 completions status 0**, no
halts, no errors, the eight `-108`s being the unbind. The connector is up, the CRTC is enabled at
the panel's native mode, keyframes report `frame ok`, and the panel stays dark. So the codec, the
ring accounting, the record grammar and the transport are all eliminated.

⛔⛔ **Three false signals, all from reading absence in a short sample. Do not repeat them.**
This dock's firmware trace and its bus traffic are both activity-dependent, and a quiet window
looks exactly like a broken one:

| claimed | what it actually was |
|---|---|
| "reports a successful write with nothing on the wire" | the capture window caught an idle moment; a capture across a real send has all 88 MB |
| "five msgids appear only on the pre-Ella build" | the HEAD capture was 4x shorter; a longer one has all five |
| "`366c51` is the discriminator" | a bisect harness built on it returned **GOOD at HEAD** |

⇒ **Validate any candidate signal at both endpoints before building on it.** That is what made the
DL7400 hunt work and skipping it cost most of a session here.

⛔⛔ **There is no in-band lit/dark discriminator for this dock, and that is now measured.**
Its output was cycled with `kscreen-doctor output.DP-9.disable/enable` -- confirmed in dmesg as
`socket 2 downstream sink powered down` and back -- while its EP84 stream was captured and
decrypted. Across all 409 frames: every `0x8x sub=0x0c` push is just the firmware trace ASCII
(`7c 32 34..` = `|24..`), differing only because the tick counter advances, and the presence reply
`id=0x44 sub=0x20` carries exactly two payloads -- `0501200000000000` for the empty socket and
`0511270080010100` for the one with a monitor -- **identical while the sink was powered down**.
⇒ Nothing the dock sends reports sink power. Do not go looking for one again; it costs an hour.

⚠ **Historic note:** The presence status word
reads `present=true ready=true` while the panel is dark, so `ready` is not it. No periodic
scanout heartbeat is identifiable in the trace. Getting one needs a labelled lit-vs-dark pair on
the current firmware, which needs one human look.

⭐ **Reading the dock's own firmware trace** (the instrument, now working):

```
sudo modprobe vino debug=1 trace_crypto=1
sudo python3 scripts/dock-trace-live.py --bus <N> --seconds 95 --save /tmp/rt.bin
# then, with the key vino printed for THAT dock:
python3 scripts/dock-trace-live.py --decode /tmp/rt.bin --key <key> --riv <riv>
```

⚠ The tool scrapes an old `CPKEYS` line that no longer exists; pass `--key`/`--riv` by hand from
`vino-crypto: control key=[..] riv_out=[..]`. **The IN nonce is `riv_out` with byte 7 XOR 0x01.**
With several docks bound the key lines are not device-tagged -- try each against the dock's bus and
keep the one that decodes.
⛔ `captures/vino-freshboot-20260726-1100/dock-trace-decoded.txt` is **not** a usable reference:
its msgid space is completely disjoint from today's (220 distinct against 60, zero overlap). This
dock now runs firmware 10.3.56, so any byte-level reference must be recaptured.

⛔ **`bracket_reopen_state` is a dead end, and the reasoning behind it was wrong.** The theory was
that Ridge is sent a second sink-*down* mid-bracket. Diffed against `4f7551789675`, the bracket's
markers are **byte-identical** for this dock -- the `reopen` expression evaluates to exactly the
`3` the old code sent -- so changing it moves Ridge *away* from known-good. The
`send_stream_prologue` call added to that bracket is gated on `video_on_ctrl_pipe()` and is inert
here too.

⭐⭐ **vino sends this dock materially the same thing at HEAD as it did pre-Ella.** Captured on
its own bus across a full load on both revisions and compared:

| | pre-Ella `4f7551789675` | HEAD |
|---|---|---|
| video to the dock | 1216 URBs / **77.8 MB** | 1268 URBs / **80.6 MB** |
| control records, by type | identical | identical |
| first 723 control records, in order | \- | **2 differ**, and only as a reorder of two `sub=0x24` |
| mode-set bracket markers | identical (`reopen` is `3` on both) | |

The single real delta is the **status-poll count**: `len=64 sub=0x24` appears **3049** times
pre-Ella against **697** at HEAD, and every other message type matches exactly. ⚠ Read that as a
symptom, not a cause: both the keepalive period (250 ms) and the scanout poll floor
(`STATUS_POLL_MIN_MS`) are unchanged for this family, and the scanout poll is still gated in for it,
so the extra polls are most likely extra *activation attempts* rather than a different policy.

⇒ Because the bytes are materially the same on both revisions, a regression is the *less* likely
reading, and "this dock has not been driven by this tree for some time" is the more likely one.
⭐⭐ **The concrete lead: this dock no longer sends its DISPLAY-CAP push.** In the capture from
when it drove panels (`captures/vino-replug-validate-20260726-190920`, in the archive root) presence
came from the dock itself:

```
per-head monitor presence (DISPLAY-CAP id=0x78): [true, true]
```

Today it reports `cap:no` on both sockets and an EDID has to be prised out of it with a blind
re-engage. ⭐ **Verified on the wire, not inferred**: decrypting its EP84 stream across a full
bring-up gives **596 frames and not one `id=0x78`** -- while the dock is otherwise talkative, with
97 `0x44/0x20` presence replies and a spread of `0x82/0x85/0x88/0x89/0x8c/0x8d/0x8f/0x91/0x92`
`sub=0x0c` async pushes. The handler still exists (`id == 0x78 && sub == 0x30` in `session.rs`) and
vino's control sequence to this dock is byte-identical to the pre-Ella one, so **the dock's own
behaviour differs from the era when it worked**, not vino's request. Why is the open question.

⛔ A fifth hypothesis died here too: `312a2f73a066` swapped a general `drain_ep84` for a targeted
`wait_perhead_push`, gated on `!perhead_onehot()` -- false for the DL7400, **true for this dock** --
and the wait counts a CP acknowledgement and discards it where the drain recorded
`display_cap_ctr`. It explained the split perfectly. Patching the wait to record the push changes
nothing, because the push never arrives.

⚠ Unproven but worth checking first: the dock's firmware. Its trace msgid space is completely
disjoint from the July reference, and vino is known to flash an older dock on enumeration when an
`.spkg` is present. The July captures cannot settle it -- back then vino could not even read the
version (`device-open 0xfd(firmware-version) non-fatal (EREMOTEIO)`); it reads 10.3.56 now.

⛔ **Four hypotheses tried and disproved. Do not re-run them.**

| tried | why it is wrong |
|---|---|
| parameter map placement | fixed on both paths, wall unchanged |
| `bracket_reopen_state` | the bracket's markers are byte-identical to pre-Ella for this dock |
| activation must describe every connector | single-port operation on this dock is **proven**, deliberately, including monitor removal |
| the three trace/wire "signals" | all sampling artifacts; see the table above |

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
