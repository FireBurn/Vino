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
| DL7400 (Navarro, `17e9:7000`) draws a picture | ✅ connector `connected` + `enabled`, EP08 draining |
| DL7400 codec correctness | ✅ **proven**, twice, independently |
| Codec geometry shared between docks | ✅ **fixed** -- passed as a value, statics deleted |
| D6000 (Ridge) control session | ✅ **fixed** -- completes, EDID reads, connector comes up |
| D6000 picture | ⛔ takes ONE video frame, then the endpoint stops draining |
| D6000 dock health | ⛔ **wedged; needs a physical power cycle** (see below) |
| Two docks bound at once | ⛔ D6000's control session times out while Navarro brings up |

---

## ⛔ Do this first: power-cycle the D6000

After ~20 firmware resets in one session the dock stopped answering at all. It now fails
`control-session attempt 1/3 (ETIMEDOUT)` about a second after bind, *before*
`plaintext session initialized`, i.e. in `bring_up()`'s first control transfer -- a failure mode it
does not otherwise show. Unplug its power for ten seconds. Any D6000 measurement taken without
doing this is measuring the wedge, not the driver.

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

## Open, in priority order

### 0. The D6000 takes exactly one video frame, then the endpoint stops

Measured at HEAD with all four gates off, D6000 alone, `debug=1`:

```
KMS CRTC enable -- head 0 display ON, mode 2560x1440@120 (scanout begins)
head=0 prompt-training parameter map 0 B
head=0 endpoint=0x08 persistent video queue opened by prompt training
head 0 startup frame submitted after 0 ms (207072 bytes)
head=0 training complete (1 presentations, 0 ms)
   ... 36 ms ...
head=0 prompt-training parameter map 0 B
head=0 endpoint=0x08 stopped accepting video: GET_STATUS=0x0000 halt=0
   ... 92 ms ...
usb 2-2.1: USB disconnect
```

Only 70 us separate "queue opened" from "startup frame submitted", so those URBs were *queued*, not
completed. 36 ms later the next presentation cannot fit a whole frame -- `can_send_n()` false -- and
the dock re-enumerates. `halt=0`: the endpoint is not stalled, the dock simply stops consuming.

⭐ **207,072 bytes.** `docs/` records the proven Ridge ARM+all-black size as **205,696**. A 1,376-byte
difference is the first thing to check: dump the wire parts (arm / opener / report / params / image
/ trailer) for that first frame and account for every byte against the DLM capture. This is the same
shape as Navarro's "accepts exactly 65,536 bytes then NAKs", which was an *opening-sequence* fault,
not a pixel fault.

⚠ Do **not** re-chase codec geometry (now a value, corruption impossible), the `send_init!` burst
(gated), `open_in`'s Dl3Cmac (gated), `send_cp_reply`'s counter loop (gated), or Ridge's EP84 queue
depth (already a profile field, 4).

### 0b. Both docks bound: the D6000's control session loses the overlap

With the DL7400 also probing, the D6000 goes straight to
`control session failed after 3 attempts (ETIMEDOUT)` while Navarro reaches
`4/4 head(s) authenticated` in the same window. Alone (with the DL7400 held unbound) it completes.

⛔ It is **not** shared driver state: after `a799158705f8` the only remaining module-level statics in
the driver are read-only tables, the two `ColdTimeline`s and the encode workqueue. Look at the USB
layer -- both docks are on bus 2 behind the same xHCI -- or stagger the two bring-ups.

⚠ Both docks change bus path on every re-enumeration. Resolve them at runtime from `idProduct`
(`7000`/`6006`) under `/sys/bus/usb/devices/`. `tools/hardware/vino-hold-off.sh <6006|7000>` keeps
vino off one dock -- re-unbinding across re-enumerations -- so the other can be measured alone.


### 1. The DL7400's connector, when both docks are bound

⚠ **Superseded in part on 2026-08-04.** With both docks bound and the geometry fix in, the DL7400's
`DP-2` reads `connected` **and** `enabled`, the control keepalive runs, EP08 drains and no
`stopped accepting video` appears. So the "reads `disconnected`, no output" symptom did not
reproduce. What is still unconfirmed is whether the panel shows the *right* pixels -- that needs a
forced-damage measurement (trap 3 below), which a headless shell cannot produce.

The old measurement, kept because the mechanism is real and may return: its head 0 presence
genuinely flips. The dock answers `status=0x00271105` (bit 0x1000 set, present) and later
`status=0x00200105` (clear) with the monitor physically attached throughout, so
`presence_from_status` correctly reports it gone and DRM follows. `detect()` returns Connected on
`cached_edids[head].is_some() || heads_present & (1<<head)`, and the re-engage path does call
`set_connected(h)`, so something clears it afterwards -- most likely the presence watcher calling
`set_disconnected`.

⚠ The presence log fires **only when the reply changes**. An absence of lines for a dock means its
answers are steady, NOT that it is unprobed. That inference was made and was wrong.

⚠ Card numbering is not stable and is not a dock identity. Across today's reloads the D6000 was
`card2` then `card3` then `card2`. Resolve a card to a dock through
`/sys/class/drm/cardN/device` -> the USB path -> `idProduct`.

### 2. Ring-slot shortfall (DL7400) — fix committed, **never verified**

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

### 3. D6000 EP02 queue flush

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
