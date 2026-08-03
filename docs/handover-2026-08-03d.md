# Handover — 2026-08-03 (fourth session)

Supersedes `handover-2026-08-03c.md`, whose premise is gone: **the DL7400 draws a picture.** That
handover's "the video jam" section is history now, and its recommended next step (a frida hook on
DLM's video submit path) is not needed.

## What the state actually is

| | |
|---|---|
| DL7400 picture | ✅ draws the desktop, hardware cursor correct |
| DL7400 encoder correctness | ✅ **proven byte-perfect**, see below |
| DL7400 stability | ⛔ re-enumerated unprovoked mid-run; `stopped accepting video` at every bring-up |
| DL7400 drag/damage path | ⛔ user-visible artifacts on moving detailed content |
| DL7400 bring-up latency | ⛔ ~24 s bind → picture, 16 s of it between session-ready and first EDID |
| DL7400 socket moves | ⛔ moving a monitor to another socket does not relight it |
| D6000 | ⛔ `control session failed after 3 attempts (ETIMEDOUT)` |

## ⭐⭐⭐ The encoder is exonerated. Stop looking there.

Two independent proofs, because a round trip through our own decoder proves nothing on its own:

**The decoder is a true inverse of DLM's encoder.** Reconstructing `~/dlm-today-124144/wire.pcapng`
renders the user's boats wallpaper with rigging, ropes and hull trim intact -- 3600/3600 positions,
**0 decode failures**. Coherent fine detail out of DLM's real bytes is what makes the model
trustworthy; merely parsing without overrun is not.

**vino's bytes round-trip through that decoder exactly.** `tools/codec/navarro-render.py`
reconstructs the dock's whole framebuffer from vino's own captured EP08 bytes. Against the source
image, with a purpose-built pattern on screen:

| region | mean abs error |
|---|---|
| colour bars (flat) | 0.06 |
| greyscale ramp (flat) | 0.05 |
| 8-px checkerboard (busy) | **0.00** |
| 1-px checkerboard (busy) | **0.00** |
| white diagonals (busy) | 0.00 |

A 1-pixel checkerboard is the worst input this codec can be given. The transform, quantiser,
entropy coder, strip geometry and record framing are all correct on full frames. Any remaining
visual defect is in *which* strips get sent and *when*, or on the dock side — not in how a strip is
encoded.

## ⭐⭐⭐ The delta path is correct too, and the panel runs at 59% amplitude

Reproduced without a human: cycle the frames of a generated animation as wallpapers
(`plasma-apply-wallpaperimage`). vino hashes per strip, so only the moving sprite's strips change
and the *delta* path is what gets exercised. mpv cannot be used — a Wayland client cannot choose
its output, and `--fs-screen-name=DP-2` silently lands on the laptop panel.

Reconstructing the 1088 strips the delta path actually sent, against the frame that was on screen:

```
mean abs error raw            39.46
best-fit gain 0.5893  offset +0.37
mean abs error after affine    0.47
```

The sprite is in the right place with its edges, label boxes and per-pixel noise all exact. The
whole image is simply scaled to **59% amplitude**. ⇒ **the damage/delta path is pixel-accurate**;
what is wrong is a uniform gain applied to the pixels *before vino sees them*.

⭐ 0.589 is the same figure Ridge produced in
`project_wire_correct_and_59pct_brightness_20260727` (0.609 / 0.589 / 0.565), where it was traced
to KWin's pipeline and not to vino. `kscreen-doctor` reports brightness "set to 100% and dimming to
100%" while this is happening, and no `GAMMA_LUT` is programmed. A washed-out low-amplitude image
is exactly what makes flat fills look banded and detail look poor, so this is a strong candidate
for what the artifact *looks* like even though the wire is exact.

⇒ **Neither the encoder nor the damage path is the defect.** Look at the reset loop and at the
compositor-side gain.

## ⭐⭐ `kind=0x200f` decoded: it is a per-strip size class

Committed as `e91d77a134b1`. The value is the strip's byte length in 512-byte units:

```
value == strip_byte_length >> 9      68,347 pairs, 0 disagreements
  0 → 54..510 B    1 → 512..1022    2 → 1024..1498    3 → 1594..1670
```

vino sent the all-zero map, i.e. it declared every strip over 511 bytes to be under 512 — wrong for
exactly the detailed strips and right for every flat one. It is now derived from the framed records
themselves so it cannot drift from the wire.

⚠ **Not established as the artifact's cause.** DLM sends 1816-byte strips in frames carrying no map
at all, so an absent map is tolerated and a wrong one being fatal does not follow. The static test
pattern renders correctly with the fix in, but that is consistent with several explanations.

## ⛔⛔ The real top priority: the DL7400 resets and jams

Two things are solidly established:

```
vino 2-1.3:1.0: vino: disconnected                      <- mid-run, unprovoked, t=72978
vino: vino: head=0 endpoint=0x08 stopped accepting video: GET_STATUS=0x0000 halt=0
```

The dock **re-enumerated on its own** during a run, and `stopped accepting video` — the 65,536-byte
wall from the previous handover — still fires at essentially every bring-up. The user separately
reported the panel freezing with only the hardware cursor moving, immediately after moving a
monitor between docks.

⚠ **Do not measure this by capture volume alone.** The protocol is damage-driven: a static desktop
legitimately sends **kilobytes**, and a run that captured 49 KB in 90 s was a static screen, not a
jam. This handover originally called those runs jammed; they were not. Distinguish the two by
*forcing damage* and re-measuring:

```sh
dumpcap -i usbmon2 -s 0 -a duration:12 -w flow.pcapng &
sleep 2; plasma-apply-wallpaperimage blank.png; sleep 3; plasma-apply-wallpaperimage pattern.png
python3 tools/codec/usbmon_read.py flow.pcapng | grep 0x08
```

A healthy DL7400 answers that with ~62 MB. Silence under forced damage is a jam; silence on a
static desktop is correct behaviour.

⚠ Still confounding: a genuinely jammed head is *silent*, not *wrong*, so a visual defect and a
stalled pipe look the same on the panel. Run the flow check before judging any artifact.

## ⭐⭐⭐ THE ARTIFACT IS REAL, AND IT IS A RING-SLOT SHORTFALL

`VID20260803200730.mp4` shows it plainly: on the DL7400 panel a Dolphin window's **icons are
clean while the filename text is garbled**, and the garbling reads as *ghosting* -- two versions of
the glyphs overlaid -- not as noise. A generated test pattern behind the window is perfect.

⚠ **Wire analysis cannot see this**, which is why three separate "the wire is correct" results are
all true and all beside the point. Reconstruction models **one** framebuffer. The dock has
**three** ring slots (`ring_phase` = `seq0 % 3`), and a strip whose bytes are perfect still ghosts
if it only lands in two of them.

Measured on `wall-200012.pcapng`, counting how many times each position's **final** payload was
transmitted:

```
1x :   4 positions
2x :  76 positions      <- at least one ring slot left stale
3x : 301 positions      <- correct: DAMAGE_REPEATS = 3 == the ring depth
```

Of the 80 short positions, 56 are the capture ending mid-debt; **24 are a permanent shortfall** --
last transmitted at frame 780/1198/1203/1204 of 4569 and never again. About 2% of touched strips,
which is exactly the density of "a few ghosted labels" rather than a broken screen.

⛔ **Do not re-measure this by decoding the wire.** The bytes are right. Count *transmissions per
distinct payload per position* and require >= 3.

`DAMAGE_REPEATS` is 3 and the ledger wipes are all paired with `owe_keyframe()`, so the mechanism
is correct in outline -- the leak is in how the debt is decremented or cancelled. Note the
decrement at the end of `encode_and_send_wht` runs over **every** strip with debt, not only the
strips the frame carried, and a full keyframe does `debt.fill(0)` while being presented **twice**
against a **three**-slot ring. Both are candidates; neither is proven.

## ⭐⭐ There is a SECOND strip encoding, and HDR is not it

| capture | strip header word at **offset 14** | decodes with our model |
|---|---|---|
| `cap2-full-usbpcap1.pcap` (SDR 60, **test pattern**) | `0x9249` on 3821/4001 | **1636 fail** |
| `cap6-hdr-fullpayload-usbpcap1.pcap` (**HDR**) | `0` on all 4001 | **0 fail** |
| `cap7-sdr-fullpayload-usbpcap1.pcap` (SDR) | `0` on all 4001 | **0 fail** |
| `~/dlm-today-124144/wire.pcapng` (Linux DLM) | `0` always | 0 fail |

⭐ `0x9249` marks a **second strip encoding we have not decoded**. It is not a Windows trait and
not an HDR trait -- only `cap2` uses it. `0x9249` is bits 0,3,6,9,12,15, every third bit of a
16-bit word, which looks like a per-plane (Y/Cb/Cr) flag replicated across blocks. `cap2` is the
**test pattern** capture (8-px and 1-px checkerboards) while cap6/cap7 are an animation with median
strip 58 B, so the mode may be selected by content complexity -- which would make it directly
relevant to the finest-detail rendering on the panel.

### ⛔ TESTED AND REFUTED: the second encoding is not content-selected

The obvious worry was that DLM switches to the `0x9249` encoding on high-frequency content and
vino never does, which would put it exactly where the finest-detail artifact lives. **Tested
directly**: Linux DLM was run by hand against the DL7400 with the same 8-px and 1-px checkerboard
pattern up (`dlm-pattern-202144.pcapng`, 292 MB on ep08 from device 57):

```
strips           1,486,800
offset-14 word   0 on EVERY one
decode failures  0
max strip        2,322 bytes   (larger than Windows cap2's 2,062)
```

⇒ **Linux DLM encodes worst-case high-frequency content with exactly the encoding vino uses, and
never sets `0x9249`.** The mode is not selected by content complexity; it is a Windows-driver
trait. Nothing for vino to copy, and the finest-detail artifact is **not** a missing encoding.

⇒ That leaves the ring-slot shortfall above as the standing explanation for the ghosting.

### ⛔ RETRACTED: "HDR does not change the strip encoding"

`cap6` (HDR on) decodes with the existing model at zero failures, identical in shape to `cap7`
(HDR off) -- but **both ran the same SDR test animation**. `out/NOTES.md` line 582: *"Both: sockets
1+3, same animation... Only HDR differs."* Windows was in HDR **mode** with nothing of HDR range or
wide gamut on screen, so an identical wire is exactly what you would see **even if the dock has a
full 10-bit path**.

⚠ The same flaw invalidates the older *"0.4% apart => HDR is host-side, nothing for vino to
implement"* conclusion. 0.4% is a **bandwidth** comparison, not a format one, and this codec is
lossy and content-driven: 10-bit and 8-bit encodings of the same low-dynamic-range picture compress
to nearly the same size. "It decodes with our 8-bit model" does not rescue it either -- a 10-bit
variant could share the strip grammar and differ only in quantiser tables, parsing cleanly while
producing wrong values.

⭐ Counter-evidence that should have been weighed: the DLM binaries carry **`NM30` (10-bit),
`FP16` and `YU10` format enums on all three platforms**. A driver with no HDR path does not need
those.

**The capture that would settle it:** genuinely HDR content -- real HDR video, or a specular
highlight beyond SDR range plus saturated wide-gamut colour -- on screen while in HDR mode, A/B'd
against the same clip in SDR. Compare *decoded pixel values and any format field*, not byte
counts.

⚠ The reasoning trap that produced the wrong call: matching *Linux DLM* is not the same as matching
*the dock's full capability*. vino's codec agreeing with Linux DLM byte-for-byte says nothing about
modes Linux DLM never uses.

## ⚠ Two measurement traps that cost real time this session

### 1. The Windows corpus is a different strip profile — it is NOT codec ground truth

`captures/navarro-wincap-20260802/out/cap2-*` has the pixel-exact `screen-ref*.png`, so it is the
obvious thing to validate the codec against. **1449 of its 3600 busy strips fail to decode**, which
looks precisely like the on-panel artifact. It is not. The Linux control run decodes **1640 busy
strips with zero failures**.

⭐ The tell is the strip header word at **offset 14**: `0x9249` on 3420 of 3600 Windows strips, and
**always 0** on Linux and in vino. Use `~/dlm-today-124144/wire.pcapng` as ground truth.

### 2. Filter captures by device, not just endpoint

A D6000 and a DL7400 on the same bus **both use endpoint `0x08`**. An endpoint-only filter
interleaves their records, and since Ridge strips are 64 px wide and Navarro's 128 px, the mixture
looks exactly like a driver emitting strips on the wrong grid. That produced a confident wrong
conclusion here before the device filter was added.

### 3. The reconstruction describes the END of the capture

Walking frames backwards keeping the newest strip per `(x, y)` gives the dock's current
framebuffer. If the thing being displayed exits before the capture stops, you reconstruct the
desktop that repainted afterwards. Keep the content up until the capture ends.

## The D6000

`40f8da12fa5c` restores Ridge's `EP84_QUEUE_DEPTH` of 4 — it had been 4 since the driver was
written and the DL7400 series dropped it to 1 through a bare `const`. Correct, but **not
sufficient**: the dock still fails, now reaching

```
vino: AKE: bad AKE_Send_Cert (id=0x7, 32 B)
```

on the retry. `id=0x7` with a 32-byte body is not a certificate, so the inbound classification is
handing the AKE parser the wrong frame. The suspects are all in `498a10040294`: `open_in()` now
verifies the Dl3Cmac instead of requiring inner bytes 6..7 to be zero, and `decode_in_lenient` was
widened for Navarro's session-varying reply ids. Both are shared with Ridge.

⭐ A third suspect was found and **tested negative**: `498a10040294` changed `decode_in_lenient`
from decoding `wire[16..32]` to decoding `wire[16..]`, and `open_in` now *verifies* the Dl3Cmac
over it -- so any Ridge reply not carrying the tag in that layout is silently dropped. Making the
strict full-body decode fall back to the historical header-only one **does not fix the D6000**
(still ETIMEDOUT), so the change was reverted rather than shipped unproven. The semantic change is
real and worth knowing; it is just not the cause.

### What the wire says (measured 2026-08-03, `ridge-205339.pcapng`)

⭐ **The D6000 fails ALONE.** With the DL7400 unbound from vino it still reports
`control session failed after 3 attempts (ETIMEDOUT)`. So this is a genuine Ridge regression, not
contention with the DL7400 — the two halves of the "both docks at once" goal are independent.

The plaintext bring-up is **clean**: the whole AKE runs to completion (events 0..28 of the capture),
the sealed session opens, and the dock answers. Across the attempt:

```
H->D  wsub=0x24 x42   wsub=0x04 x21
D->H  wsub=0x45 x50   wsub=0x25 x40
```

⇒ **the dock replies throughout — 50 sealed replies — while vino times out.** vino is failing to
*match* replies, not being starved of them. `AKE: bad AKE_Send_Cert (id=0x7, 32 B)` only appears on
the *second* attempt, i.e. it is the dock left in a bad state by the first failure, not the cause.

⛔ **Tested and NOT the cause:** the `open_in` change. `498a10040294` turned it from a plain decrypt
taking `ct` into a decrypt that splits a trailing 16-byte Dl3Cmac and *verifies* it, and
`decode_in_lenient` went from `wire[16..32]` to `wire[16..]`. That is a real semantic change and a
plausible way to drop every Ridge reply. Both a strict-then-lenient fallback and a profile-gated
`INBOUND_REQUIRES_MAC=false` for Ridge were implemented, built and run on hardware: **still
ETIMEDOUT**. Both were reverted rather than shipped unproven. ⚠ An earlier attempt at the fallback
was a **no-op** and must not be counted as a test — it passed a 16-byte slice into the *new*
`open_in`, which leaves zero ciphertext after splitting off the tag.

### ⭐⭐⭐ ROOT-CAUSED AND PARTLY FIXED (`dbe004a2d6be`)

`send_cp_reply` reads EP84 until it sees the reply whose inner counter echoes its request, and it
identifies that reply through `decode_in_lenient` -- the very function the series made
tag-verifying. **Every Ridge reply became invisible to the matcher.** That is why the dock sends 50
sealed replies while vino reports ETIMEDOUT.

Restoring the plain decode as a per-frame fallback brings the D6000 up:

```
vino 2-2.1:1.0: vino: encrypted control session ready     <- with the DL7400 UNBOUND
```

after four sessions of nothing but ETIMEDOUT. The DL7400 is unaffected: 4/4 heads authenticated,
session ready, monitor connected.

⚠ **Necessary but not sufficient, and NOT for the reason first recorded.** `dbe004a2d6be` changed
only `decode_in_lenient`. Ridge then fails **even alone**. The run that succeeded had the plain
decode forced *inside `open_in` itself*, so all **eight** of its call sites got it —
`verify_in_ack`, `perhead_hdcp_push`, the EDID parsers and the rest. Ridge sends no tag on any of
them, so fixing only the matcher leaves the other seven rejecting every frame:
`1/2 head(s) authenticated` then ETIMEDOUT.

⇒ **The fix must be at `open_in`, and it must be per device.** Forcing it globally is not an
option: it costs Navarro its per-head HDCP (`0/4 head(s) authenticated`), measured. `open_in` is a
free function with no device context, so this needs a `verified: bool` (or equivalent) threaded
through it and through the cp.rs helpers that call it — the call sites are cp.rs lines 589, 637,
679, 731, 808, 1380, 1432 and 1473, all reachable from callers that do have the profile.

**That is the next piece of work, and it is mechanical rather than exploratory.**

⛔ **Do not make this a global flag.** That was tried first and is the wrong shape: the DL7400
overwrote Ridge's setting, and forcing it the other way cost the DL7400 its own per-head HDCP
(`0/4 head(s) authenticated`). Per frame works because the strict pass can only succeed on a
genuine authenticated frame.

### The remaining blocker for "both docks at once"

Two candidates for the second interaction, neither tested:

1. **Codec geometry is module-global**: `STRIP_W_SHIFT`, `STRIP_H_SHIFT`, `INTERLACED_BANDS`,
   `BAND_PARITY_BIT`, `AUX_IS_PAD_COUNT`, `DOCK_BUFFERS` are all set from whichever profile probed
   last. Ridge needs 64x16 strips and 2 buffers; Navarro needs 128x8 and 3. These must move onto
   `VinoDrmData`.
2. **Bring-up contention** -- and the timestamps say the two bring-ups **overlap** rather than
   queue, so this is contention, not serialisation:

   ```
   79957.306  2-2.1 bound          79957.306  2-1.3 bound      <- together
   79960.733  2-2.1 attempt 1 FAILED (ETIMEDOUT)
   79960.855  2-1.3 encrypted control session ready            <- 0.12 s later
   ```

   Ridge gives up ~0.12 s before Navarro finishes. Both docks *have* reached
   `encrypted control session ready` in the same boot (79864.3 and 79868.5), just never at the
   same time. Look for what a bring-up holds across its long blocking USB waits.

**Historical, superseded:** `trace_crypto=1` is implemented and a capture was
taken (`ridge-trace-205751.pcapng` plus the keys in `trace-dmesg.txt`), but the decrypt was **not**
achieved this session — neither logged key produced sensible plaintext for the device filtered as
the Ridge. Two things to get right that were not: which of the two logged control keys belongs to
which dock (both docks load together, so both are logged), and the exact inbound nonce
(`byte7 ^= 0x04` for OUT then `^= 0x01` for IN, per the proven CP contract — a sweep of
0x00/0x04/0x05 did not land, so the device mapping is the more likely error). With that decrypt,
the stalling message is read off directly instead of guessed. **Guessing has now failed four
times; do not add a fifth.**

⚠ `de9521207d12` (presence silence measured in time, not probes) is **still unverified** — the D6000
has never got far enough to exercise it.

## Tooling added

`tools/codec/` — `colour_decode.py`, `usbmon_read.py`, `usbpcap_read.py`, `navarro-render.py` and a
README carrying the traps above. Copied out of the retired `dl-scripts/scripts/codec-re/` archive so
the live tree does not depend on it. The readers walk pcap/pcapng directly instead of shelling out
to tshark, which emits a 244 MB payload as hex.

## Where to go next, in order

1. **The unprovoked re-enumeration, and `stopped accepting video` at every bring-up.** Run the
   forced-damage flow check first, so a jam is told apart from an idle desktop before anything
   visual is chased.
2. **The drag path.** Reproduce without a human: `mpv --fs --fs-screen=0 mf://anim/f*.png` over a
   generated animation gives a repeatable moving-detail workload. Reconstruct and diff against the
   frame that was on screen — but only after (1), or the capture is empty.
3. **Bring-up latency.** 16 s sits between `encrypted control session ready` and the first EDID
   read. Nothing has looked at that gap yet.
4. **The D6000**, via `trace_crypto=1` as above.
5. Socket moves — connector index is socket − 1, `ep 0x08` owns {0,2} and `ep 0x0a` owns {1,3};
   `runtime_connector()` currently keeps 2 and 3 out of the runtime path entirely.
