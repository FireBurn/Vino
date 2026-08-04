# Handover

Single current handover. Replaces `handover-2026-08-02.md`, `handover-2026-08-03.md`, `-03b`,
`-03c` and `-03d`, which are deleted: everything below is either still true or a dead end worth
not repeating. Anything those files said that is not repeated here was either done, superseded, or
retracted.

Last updated 2026-08-04.

---

## State

| | |
|---|---|
| DL7400 (Navarro) picture | ✅ clean, user-confirmed |
| D6000 (Ridge) picture | ✅ driving video, user-confirmed |
| Both docks bound at once | ✅ one connector each, real EDID, native 2560x1440, 0 resets |
| Phantom connector | ✅ fixed -- presence reads the status word on both docks |
| Control-plane idle load | ✅ 0 unanswered CP messages per 45 s (was 22) |
| DL7400 refresh ceiling | ✅ 165 Hz offered; 180 Hz excluded, it fails on Windows too |
| Time from dock power-on to pixels | ⚠ still long; driver-side bind→connector is now ~2 s |

---

## ⚠ Measurement correction: usbmon completions

`tools/hardware/capture-usbmon-session.py` records **`S` (submit) for OUT endpoints and `C`
(callback) for IN endpoints only**. "Zero completions on EP08" is therefore what a *healthy* dock
looks like through this tool, and any earlier conclusion drawn from it is void.

⛔ Do not use completion counts to decide whether video is flowing. Use **bytes submitted on the
video endpoints over a window with forced damage** -- the protocol is damage-driven, so a static
desktop legitimately sends kilobytes.

---

## ✅ Fixed on 2026-08-04

### The codec geometry is a value now, not module state

`a799158705f8`. `STRIP_W_SHIFT`, `STRIP_H_SHIFT`, `INTERLACED_BANDS`, `BAND_PARITY_BIT`,
`AUX_IS_PAD_COUNT`, `HEAD_SUB_SHIFT`, `STREAM_ID_MASK` and `DOCK_BUFFERS` are gone. A `Copy`
eight-byte `video::wht::Geometry` is threaded into every codec entry point, sourced from
`VinoDrmData::geometry()` at runtime and `DockProfile::geometry()` during CP setup. `EncodeChunk`
carries it, so two docks encode concurrently again. `ENCODE_BUSY`/`EncodeGeometryGuard` deleted --
they only made the docks take turns and did **not** stop the D6000's reset loop (re-measured).

⚠ This was the previous handover's "do this before anything else". It is done, and it was **not**
the cause of either dock's remaining symptom. Both docks still fail exactly as they did with the
stopgap in place. Its value is that geometry corruption is now unrepresentable, so it can be struck
off every future hypothesis.

### The D6000's control session -- the ETIMEDOUT half of the bisected regression

`d082d0912dcb` (cherry-picked onto the branch) + `fffce4ef5d8d`.

⛔ **The previous handover's "`cde9a2c9e430` is NOT sufficient" is retracted.** That was measured on
the detached bisect build, not on the branch. On the branch, gating the `send_init!` sequence
(`0x14/0x30`, `0x15/0x0b`, `0x16/0x2a` per connector -- three messages the parent of
`498a10040294` never sends) **does** fix it. Measured, D6000 alone:

```
encrypted control setup complete (16 messages)
link ready after 25 status polls
encrypted control session ready
head 0 EDID 384 B, vendor MSI product 0x3cd9
head 0 monitor connected after sink re-engagement
```

`fffce4ef5d8d` gates the other two ungated CP changes from that commit behind new profile fields,
so Ridge is decoded the way it always was:

* `cp_authenticated_in` -- `open_in()` verifying the trailing Dl3Cmac over the whole body, and its
  callers dropping the `id < 0x400` / `is_known_sub` / zero-word-at-6..7 plausibility tests.
* `cp_reply_counter_match` -- `send_cp_reply()` looping up to 64 ms with `cp_link` held until a
  reply's inner counter echoes the request.

Both are `true` for Navarro (it needs them) and `false` for Ridge.

### ⛔ RETRACTED: "the bisect's parent is good for the D6000"

**Measured at `c57634406a47` = `498a10040294^`, D6000 alone: it re-enumerates there too.**

```
encrypted control session ready
1920x1440@60 has no decrypted DLM profile; inferring off42=0x0400 off66=0x0800
head 0 video submit took 0 ms   /   head 1 video submit took 0 ms
usb 2-2.1: USB disconnect, device number 87
```

`1920x1440@60` is the **no-EDID fallback**, and head 1 has no monitor: at the parent the dock
reports `2/2 head(s) authenticated` and both heads `connected`, so vino drives a phantom head 1 and
the dock resets. At HEAD head 1 is correctly `no downstream sink`, so this particular reset is
already gone.

⇒ The user bisect distinguished *"pixels while restarting a lot"* from *"no pixels at all"*. The
**restarting predates the bad commit**; only the pixels regressed. Do not expect any further
gating of `498a10040294` to produce a stable D6000, and do not treat its parent as a working
reference.

Also checked and clean: `498a10040294` changes **nothing** in Ridge's record generation. Every
video.rs hunk is behind `navarro_ordinary` (`None` for Ridge) or `interlaced_bands` (false for
Ridge); the only other edit is a doc comment.

---

## ✅ Also fixed on 2026-08-04

### Ridge's frames were reordered by the DL7400's producer permutation

`139bf929a013`. `NAVARRO_PROLOGUE_ROWS`/`NAVARRO_ORDINARY_ROWS` were selected on
`strips.len() == 3600` alone, and Ridge at 2560x1440 is *also* exactly 3600 strips (40x90, against
Navarro's 20x180), so every D6000 frame was reordered against a grid half its real width with no
change in byte count. Fixing it put the frame back to 205,696 B, the proven Ridge ARM+black size.

### Ridge's H' computation hold

`e6171c1f60e7`. `498a10040294` gated the post-`AKE_No_Stored_km` hold and drain on
`per_head_onehot`, so Ridge sent `LC_Init` while the receiver was still computing H'.

    before   1/2 head(s) authenticated, head 1 "no downstream sink", one connector
    after    2/2 head(s) authenticated, both connectors, zero re-enumerations

### The phantom connector, and one presence rule for both docks

`bd7cf0f48e42`. Both platforms answer the presence probe with the same status word --
`0x00271105` occupied, `0x00200105` empty, bit `0x1000` being presence. Ridge keyed on *which
handler* replied instead, which is `id=0x44` either way, so its empty head read as present and came
up as an enabled `DP-3` with no EDID and a `1920x1440` fallback list. `presence_from_status` and the
silence-counting removal path are gone with it.

The absent-head re-engage now stands down once the probe has answered for a connector. It exists to
recover a deferred discovery, not to poll an empty socket, and each attempt is seven CP messages an
empty socket largely never answers:

    unanswered CP messages per 45 s   22 -> 0

### The mode ceilings are per dock, and the DL7400 gets 165 Hz

`64e6ae791baa`. `max_refresh_hz`, `max_head_clock_khz` and `pixel_budget` were single constants
derived from the D6000 and applied to every dock. Each is now a profile field carrying DLM's own
behaviour on that dock: Ridge 120 Hz / 655.35 MHz, DL7400 165 Hz / 699.50 MHz. The binding limit
turned out to be the *clock* ceiling, not the refresh one -- the old 655.35 MHz was the range of the
low half of the offset-70 `u32`, which only Ridge's captures ever fill.

    card2-DP-2 (D6000)  36 modes, 3 at 2560x1440
    card3-DP-6 (DL7400) 37 modes, 4 at 2560x1440

⛔ **2560x1440@180 is a known-bad mode on the vendor stack too, not merely untested by us.** Under
Windows, delivered frame records fell from 5,462 to 344 over a comparable window -- ~230/s to ~15/s,
a 16x drop at three times the nominal rate -- while ep0 control transfers rose from 4 to 1,724, and
the dock then entered a disconnect/reconnect loop needing a manual power cycle.
`ChangeDisplaySettingsEx` returned `DISP_CHANGE_SUCCESSFUL` and the mode read back as 180 Hz
throughout. **Mode acceptance is not evidence of deliverable bandwidth.** Its 714.81 MHz timing is
above the new clock ceiling, so it is pruned on both tests.

### The DL7400's intermittent blank

`f56461774810`. `probe_connector_present()` reaped one EP84 read and decoded it as the answer.
Connectors are probed back to back, so a late reply was consumed by the *next* connector's probe and
attributed to the wrong head -- head 0 and head 1 traded `present`, the live output was dropped, and
a sink re-engagement brought it back seconds later. It now waits for the reply whose inner counter
echoes the probe.

---

## Open, in priority order

### 1. Time from dock power-on to pixels

Driver-side bind to connector is now ~2 s and the idle control plane is quiet, so what remains is
before or after vino: dock firmware boot, USB enumeration, and userspace re-enabling the output.
Measure the whole path before changing anything in the driver.

### 2. Both docks bound: the D6000's control session can lose the overlap

With the DL7400 also probing, the D6000 has been seen going straight to
`control session failed after 3 attempts (ETIMEDOUT)` while Navarro reaches `4/4 head(s)
authenticated` in the same window. It is not shared driver state -- the only module-level statics
left are read-only tables, the two `ColdTimeline`s and the encode workqueue -- so look at the USB
layer, or stagger the two bring-ups. The eight-attempt backoff makes it recoverable rather than
fatal.

⚠ Both docks change bus path on every re-enumeration. Resolve them from `idProduct` (`7000`/`6006`)
under `/sys/bus/usb/devices/`. `tools/hardware/vino-hold-off.sh <6006|7000>` keeps vino off one dock
across re-enumerations so the other can be measured alone.

### 3. Ring-slot shortfall (DL7400) — fix committed, **never verified**

`029c2bd6c747`. The dock rotates **three** slots (`ring_phase` = `seq0 % 3`), but the keyframe
presentation count was hardcoded 2 ("must reach both dock buffers") and `DAMAGE_REPEATS` was 3,
matching the ring depth by luck with zero margin. A keyframe reached two of three slots and then
called `debt.fill(0)`, so nothing ever repaired the third.

Measured before the fix, counting transmissions of each position's **final** payload:

```
1x :   4 positions
2x :  76 positions      <- at least one ring slot left stale
3x : 301 positions      <- correct
```

24 were a permanent shortfall (~2% of touched strips) — the density of a few ghosted text labels.
Both counts now derive from a per-profile `dock_buffers` (Ridge 2, DL7400 3).

⛔ **Invisible to wire decoding**: the bytes are correct; reconstruction models one framebuffer.
Measure by counting **transmissions per distinct payload per position** and requiring >= the ring
depth. Verification needs one delta-heavy capture; three attempts were confounded (two-dock race,
uniform on-screen content, stale sysfs).

### 4. D6000 EP02 queue flush

`610754e7a62c`, `dbe004a2d6be`'s revert. `send_cp_setup` ended with
`queue.flush(dev.io(), timeout())?`; `timeout()` is 1000 ms, matching the measured 1.06 s between
`N/2 head(s) authenticated` and failure exactly. With one connector empty an outstanding EP02 write
is never drained, and the flush failed a session that was otherwise complete. Now logged and
non-fatal.

⛔ Eliminated by measurement first, each with an instrument built and run on hardware:
`send_cp_reply` match-loop expiry (never fires), unqueued EP02 NAK retry exhaustion (never fires),
queued EP02 submit failure (never fires), `read_ep84`'s ETIMEDOUT (handled by `match` at all three
callers, never propagated).

---

## ✅ Settled — do not re-chase

### The DL7400 codec is correct, including finest detail

Two independent proofs; a round trip through our own decoder proves nothing on its own.

1. **The decoder is a true inverse of DLM's encoder.** Reconstructing `~/dlm-today-124144/wire.pcapng`
   renders the boats wallpaper with rigging and hull trim intact — 3600/3600 positions, **0 decode
   failures**.
2. **vino's bytes round-trip exactly.** Against a purpose-built pattern, mean absolute error:
   colour bars 0.06, greyscale ramp 0.05, 8-px checkerboard **0.00**, **1-px checkerboard 0.00**,
   diagonals 0.00. A 1-pixel checkerboard is the worst input this codec can be given.

⇒ Transform, quantiser, entropy coder, strip geometry and record framing are all correct. **The
encoder is not the artifact.**

### `kind=0x200f` is a per-strip size class

`e91d77a134b1`. `value == strip_byte_length >> 9`, over 68,347 pairs with **zero** disagreements
(0 → 54..510 B, 1 → 512..1022, 2 → 1024..1498, 3 → 1594..1670; every boundary a multiple of 512).
vino sent all zeros, declaring every strip over 511 bytes to be under 512. Now derived from the
framed records themselves. ⚠ Not established as *the* artifact's cause — DLM sends 1816-byte
strips in frames carrying no map at all.

### The `0x9249` second strip encoding is Windows-only

`0x9249` at strip header **offset 14** marks a second encoding, present only in the Windows `cap2`
capture. **Tested and refuted as content-selected**: Linux DLM driving the DL7400 with the same
8-px/1-px checkerboard pattern wrote **0 on all 1,486,800 strips, 0 decode failures, max strip
2322 B** — larger than cap2's 2062, so the codec was certainly stretched. Nothing for vino to copy.

### Ridge's EP84 queue depth

`40f8da12fa5c`. It had been 4 since the driver was written; the DL7400 work dropped it to 1 through
a bare `const` and took Ridge with it. Now a profile field.

---

## ⚠ Measurement traps that cost real time

1. **Filter captures by USB device, not just endpoint.** Both docks use endpoint `0x08`. An
   endpoint-only filter interleaves their records, and since Ridge strips are 64 px and Navarro's
   128 px the mixture looks exactly like a driver emitting strips on the wrong grid. Produced a
   confident wrong conclusion.
2. **The Windows corpus is not codec ground truth.** `cap2` fails ~40% of its busy strips through
   `colour_decode`. Ground truth is `~/dlm-today-124144/wire.pcapng`.
3. **A static desktop sends kilobytes, legitimately.** The protocol is damage-driven. Distinguish a
   jam from an idle screen by *forcing damage*: a healthy DL7400 answers a wallpaper flip with
   ~62 MB on EP08. Runs of 49 KB over 90 s were called jams and were not.
4. **Backwards strip reconstruction describes the END of the capture.** If the content exits before
   the capture stops, you reconstruct the desktop that repainted after it.
5. **`/sys/class/drm/*/status` is cached.** Write `detect` to it first, or you read a stale value
   and conclude connectors are flapping when they are not.
6. **HDR was never actually tested.** `cap6` (HDR on) and `cap7` (HDR off) ran the **same SDR
   animation**, so an identical wire proves nothing about a 10-bit path. The older "0.4% apart ⇒
   HDR is host-side" call has the same flaw — that is a *bandwidth* comparison, not a format one.
   ⭐ Counter-evidence: the DLM binaries carry `NM30` (10-bit), `FP16` and `YU10` format enums on
   all three platforms. Settling it needs real HDR content A/B'd against the same clip in SDR,
   compared on decoded pixel values.
7. **Read the lines either side of the one that matches your hypothesis.** A D6000 "success" —
   `encrypted control session ready` — sat directly beneath `0/2 head(s) authenticated` and
   `0/4 head(s) authenticated`. Four commits were built on that before it was caught.

---

## Tooling

`tools/codec/` — `colour_decode.py` (the codec model), `usbmon_read.py` (Linux pcapng),
`usbpcap_read.py` (Windows USBPcap), `navarro-render.py` (record walker, frame splitter, surface
compositor, scorer), and a README carrying traps 1–4. Both readers walk the capture directly rather
than shelling out to tshark, which emits the whole payload as hex.

```sh
python3 tools/codec/usbmon_read.py wire.pcapng                 # per device+endpoint inventory
python3 tools/codec/navarro-render.py wire.pcapng --ep 8 --sub 0 --ref ref.png --out run
```

`tools/hardware/vino-cycle.sh` reloads the module and derives `rtc_utc_offset_minutes` from the
host timezone. `trace_crypto=1` discloses session key material for a decryptable capture — never
leave it on.
