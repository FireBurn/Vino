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
| DL7400 (Navarro) draws a picture | ✅ |
| **DL7400 corruption** | ✅ **FIXED, user-confirmed clean** (`6e3b2862e40d`) |
| Codec geometry shared between docks | ✅ fixed -- passed as a value, statics deleted |
| D6000 (Ridge) control session | ✅ fixed -- completes, EDID reads, connector comes up |
| D6000 frame framing | ✅ root-caused + fixed; frame is byte-exact to the proven size again |
| **D6000 picture** | ⛔ dock takes the first frame, then stops draining EP08 |
| D6000 warm re-attach | ⛔ **one bring-up per power cycle**; nothing in software clears it |
| Two docks bound at once | ⚠ D6000's control session loses the bring-up overlap |

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

### 0. ⭐ ROOT-CAUSED: Ridge frames were reordered by the DL7400's permutation

`139bf929a013`. **Fix committed and confirmed by byte count.** With it in, the D6000's first training frame is
**205,696 bytes** -- exactly the size `docs/` records for a proven Ridge ARM+all-black frame. It was
**207,072** before, and the 1,376-byte difference was the permutation reordering records across
band boundaries. ⛔ The dock nevertheless still stops draining EP08 after that frame, so the
permutation was necessary and is not sufficient.

`NAVARRO_PROLOGUE_ROWS` / `NAVARRO_ORDINARY_ROWS` are DLM's measured producer completion order for
a 2560x1440 *Navarro* surface -- 20 strips across x 180 bands, because Navarro strips are 128x8.
`frame_records_with_boundary` selected them on `strips.len() == 3600` alone.

⭐ **Ridge at 2560x1440 also has exactly 3600 strips**: 40 across x 90 bands, its strips being
64x16. So every D6000 frame at its usual mode was reordered by the other dock's permutation,
indexed `y * STRIPS_ACROSS + x` with `STRIPS_ACROSS` hardcoded to **20** against a **40**-wide
grid. Identical byte count, thoroughly scrambled coordinates -- which is why no length or record
check ever caught it.

Both black training carriers reach that code on **every** dock: `black_frame_ep08` passes
`Some(false)` and `black_frame_ep08_ordinary` passes `Some(true)`, unconditionally. The strip count
was the only guard and Ridge's most common mode walks straight through it. The guard is now the
Navarro layout itself (interlaced bands + a 128-px strip).

This is the "no pixels at all" half of the bisected regression, and `498a10040294` is exactly the
commit that introduced these tables -- consistent with the bisect and with the parent still
resetting for the unrelated phantom-head-1 reason.

**How it presented** (D6000 alone, `debug=1`):



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

Only 70 us separate "queue opened" from "startup frame submitted", so those URBs were *queued*, not
completed; 36 ms later `can_send_n(4)` is false against an eight-deep queue, so **none of the first
frame's four URBs had completed**. The dock took the first transfer and then stopped, exactly as it
would for a frame whose records name coordinates it cannot place.

⚠ Ridge's 3600-strip coincidence is only at 2560x1440. Any other Ridge mode has a different strip
count, misses the guard and is framed correctly -- which is why this looked mode-specific and why
a dock driven at a fallback mode behaved differently.

⚠ Do **not** re-chase codec geometry (now a value, corruption impossible), the `send_init!` burst
(gated), `open_in`'s Dl3Cmac (gated), `send_cp_reply`'s counter loop (gated), or Ridge's EP84 queue
depth (already a profile field, 4).

### 0a. ⛔ The D6000's dock refuses EVERY video byte -- and `a13775e0cdc5` is a working reference

⭐⭐⭐ **Measured 2026-08-04, and it collapses the search: at `a13775e0cdc5` -- the user's bisect
"good" anchor, built with `tools/hardware/vino-at.sh` so bindings, DRM core and the HDR work stay at
HEAD -- the D6000 comes up `connected` **and** `enabled`, with zero re-enumerations and zero
`stopped accepting video`.** The dock and the monitor are fine. The regression is in the driver, and
there is a revision on this very kernel that demonstrates it.

⚠ At `c57634406a47` (the bad commit's parent) it already re-enumerates, for the unrelated
phantom-head-1 reason above. So there are **two** Ridge regressions between the anchor and HEAD, and
the user's bisect separated only the second: "pixels while restarting a lot" vs "no pixels at all".

**What HEAD does, on the wire.** A usbmon capture of a bring-up, filtered to the D6000's devnum:

```
S st=-115 len= 65536   02000000 08000000 00000000 08000600 ...   <- frame 0, ARM record first
S st=-115 len= 65536
S st=-115 len= 65536
S st=-115 len=  9088                                             (205,696 B total)
S st=-115 len= 65536   frame 1 ...
S st=-115 len=  6528
```

**Eight submits, zero completions.** Not one video byte is accepted. This is not a byte count and
not a transfer count -- `video_xfer`/`video_records` cannot bisect something that never starts. The
stream is simply never open, and `GET_STATUS` says `halt=0`, so the endpoint is not stalled either.

⭐ And the control plane says so: `send_cp_reply` now names the inner sub of an unanswered message,
and the one the dock ignores is **`id=0x16 sub=0x2e`** -- a stream/display marker from the mode-set
bracket (`modeset_bracket_post_open` sends `2f(1)`, `2e(3)`, `2f(1)`, `2e(3)`, `2f(1)`, `2e(0)`).
The dock answers the earlier markers and then goes silent.

**The method that will converge**: diff HEAD against `a13775e0cdc5` for everything that touches
Ridge's *video open* -- the per-head HDCP sequencing is the largest ungated candidate
(`wait_perhead_push` replaced `drain_ep84` at four sites in `send_cp_setup`; two of them, the
`AKE_SEND_H_PRIME` and pairing-info waits, run for **both** docks), then
`b837e4ea9333` "serialize queues by physical endpoint" and `40568f66f3ed` "Navarro authenticates the
link once, not once per head".

⛔ Do not simply run `git bisect`: see 0c. A wedged dock fails at *every* revision, and it wedges
after roughly two bring-ups, so a bisect silently reports "bad" for revisions that are good. Two
steps were run this way and both readings are worthless -- `a13775e0cdc5` itself scored
`sessions=0` on the third pass. Each bisect step needs a dock that has just re-enumerated.

### 0c. ⛔ The D6000 accepts exactly ONE bring-up per power cycle

`ef6cef0a4945`. The first bring-up after power succeeds; every one after it NAKs `init_0` forever
(`control-session attempt 1/3 failed (ETIMEDOUT)` ~1 s after bind, before any session exists). The
dock answers every control request throughout and reports `interface state` as 28 zero bytes; it
simply will not drain EP02. **Only unplugging its power clears it.**

This was invisible while the dock re-enumerated every few seconds -- that cleared it as a side
effect. It makes every D6000 experiment cost one power cycle, so plan them accordingly.

⛔ Tried and does **not** work, each built and measured: driving vendor request `0x24` back to
wValue 0 before claiming (`vendor_state_reset`); CLEAR_FEATURE(ENDPOINT_HALT) on EP02/EP84 at
bring-up (`ctrl_clear_halt`); and `USBDEVFS_RESET` on the whole device followed by a rebind. The
port reset completes and `init_0` still times out ⇒ this is dock **firmware** state, not host-side
endpoint or enumeration state.

⭐ The remaining candidate: vino never tells the dock its session is over. `disconnect()` reaches
`shutdown()` and sends no CP teardown. One frida attach on a DLM **shutdown** would name the
message.

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


### 1. ✅ FIXED: the DL7400's intermittent blank was a mis-attributed presence reply

`f56461774810`. `probe_connector_present()` reaped exactly one EP84 read after its write and decoded
it as the answer. The connectors are probed back to back, so a reply that arrived a moment late --
or any unprompted push -- was consumed by the *next* connector's probe and its status word
attributed to the wrong head. From the journal, monitor on socket 1 only:

```
01:18:02  head 1 -> present=true     (head 1 is EMPTY)
01:18:22  head 0 -> present=false    (head 0 has the monitor)
01:18:22  head 0 monitor disconnected        <- live output dropped
01:18:23  head 1 -> present=true
01:18:27  head 0 monitor connected after sink re-engagement
```

Exactly one of the two is ever "present", which is the true state; **which head it lands on
alternates**. The dock never changed its mind. It now waits for the reply whose inner counter echoes
the probe, and a round that never sees its own echo returns `None` -- which the caller already
treats as "this poll learned nothing" rather than as an unplug.

⇒ This is the "disconnects with the new dock". ⚠ Not the same thing as the three genuine
re-enumerations of `usb 2-1.3` earlier in the session; those stopped once the D6000's reset loop
did, and none has occurred since.

### 1b. The DL7400's connector, when both docks are bound

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
