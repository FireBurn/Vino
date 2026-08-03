# Handover

Single current handover. Replaces `handover-2026-08-02.md`, `handover-2026-08-03.md`, `-03b`,
`-03c` and `-03d`, which are deleted: everything below is either still true or a dead end worth
not repeating. Anything those files said that is not repeated here was either done, superseded, or
retracted.

Last updated 2026-08-03, end of the two-dock session.

---

## State

| | |
|---|---|
| DL7400 (Navarro, `17e9:7000`) draws a picture | ✅ but see below |
| DL7400 codec correctness | ✅ **proven**, twice, independently |
| DL7400 DRM connector, both docks bound | ⛔ reads `disconnected`; **no output** |
| D6000 (Ridge, `17e9:6006`) reset loop | ✅ fixed and root-caused |
| D6000 DRM connector | ✅ `connected`, enabled as an output |
| **D6000 picture** | ⛔ **nothing on the panel** |
| Two docks bound at once | ⛔ **only one displays at a time** |

⚠ **The two docks have swapped places over one session.** That is the signature of shared state,
not of two independent bugs. Fix the state and re-measure before chasing either symptom.

---

## ⛔⛔ The core defect: codec geometry is module-global

`STRIP_W_SHIFT`, `STRIP_H_SHIFT`, `INTERLACED_BANDS`, `BAND_PARITY_BIT`, `AUX_IS_PAD_COUNT`,
`HEAD_SUB_SHIFT`, `STREAM_ID_MASK` (video.rs) and `DOCK_BUFFERS` (drm_sink.rs) are module-wide
statics written once per probe. **Ridge lays a strip's sixteen blocks 8 across x 2 down over
64x16 px; Navarro lays them 16 across x 1 down over 128x8.** Whichever dock probes last wins and
the other encodes with the wrong layout.

Measured consequences:

* The D6000 was fed Navarro-shaped records, answered `head=0 endpoint=0x08 stopped accepting
  video: GET_STATUS=0x0000 halt=0`, and **reset itself** — `usb 2-2.1: USB disconnect` — looping
  its whole bring-up every ~8 s. 13 re-enumerations in one window.
* The DL7400's delivered pixels, scored against the source after fitting out brightness, went from
  a residual of **0.47** with one dock bound to **22.21** with a D6000 alongside, and 544 of 3600
  strip positions were never sent.

**Stopgaps in tree** (`dfc52a1856ab`, `9c9f2113b8a1`): each device mirrors its geometry and
restores it before encoding, and `EncodeGeometryGuard` serialises restore+encode across devices.
This stopped the D6000's reset loop.

⛔ **These are not the fix.** They make two docks *correct* by making them take turns, and they
have not made either dock display properly. **The real change is to pass a geometry struct into
the codec entry points** — `colour_frame_ep08*`, `frame_records*`, `colour_strip*`,
`damage_strip_coords`, `all_strip_coords`, `navarro_strip_params` — and delete the statics. Then
no serialisation is needed and no probe can corrupt another device.

**Do this before anything else.** Every symptom that moved during the last session moved because
of this state.

---

## Open, in priority order

### 1. Neither dock puts correct pixels on a panel with both bound

* DL7400: `card2-DP-2` reads `disconnected` after a forced detect, so no output exists. Measured
  cause: its head 0 presence **genuinely flips**. The dock answers `status=0x00271105` (bit 0x1000
  set, present) and later `status=0x00200105` (clear), with the monitor physically attached
  throughout, so `presence_from_status` correctly reports it gone and DRM follows. Find what makes
  the dock retract presence.
  `detect()` returns Connected on `cached_edids[head].is_some() || heads_present & (1<<head)`, and
  vino *does* log `vino 2-1.3:1.0: ... head 0 monitor connected after sink re-engagement`, whose
  code path calls `set_connected(h)`. So something clears it afterwards — most likely the presence
  watcher calling `set_disconnected`.
* D6000: `card3-DP-6` reads `connected` and kscreen enables it as `DP-6`, but **the panel shows
  nothing**. Its presence is stable — `runtime_connector()` returns true for Ridge, so it *is*
  being probed — so this is a **video** problem, not a presence one. Check EP08/EP0b byte flow;
  connector state is not pixels.
  ⚠ The presence log fires **only when the reply changes**. An absence of lines for a dock means
  its answers are steady, NOT that it is unprobed. That inference was made and was wrong.
* ⭐⭐ **Known-good anchor for the D6000: `a13775e0cdc5`** (user-confirmed working there).
  **40 commits** touch `drivers/gpu/drm/vino` between it and HEAD, +4576/-595 lines. That is ~6
  build-and-boot cycles with `git bisect`, and it is the cheapest remaining route to the Ridge
  regression -- cheaper than more instrumenting, which has now eliminated four candidate
  mechanisms without finding the cause.

  ⚠ Bisect on the **D6000 alone, unbound from the DL7400**, or the shared-state bug will
  contaminate every verdict. Mark good/bad on "does the D6000 put a picture on its panel", not on
  any log line -- `encrypted control session ready` and a `connected` connector have both been
  observed on a dock showing nothing.

⚠ The presence log was untagged (`pr_info!`, no device prefix) for most of the session, so lines
like `head 0 presence reply … present=true` **could not be attributed to a dock** and were read as
the wrong one. It now carries the dock's connector count (Navarro 4, Ridge 2). Re-read any earlier
conclusion drawn from those lines.

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
