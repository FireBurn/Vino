# Handover

Single current handover. Replaces the 2026-08-04 one entirely: everything below is either still
true or a dead end worth not repeating. Anything the old file said that is not repeated here was
done, superseded, or retracted.

Last updated 2026-08-08. ⭐ Read the **2026-08-07 (third session)** block at the bottom first: every
socket pairing works now, and the reason they did not invalidates every per-head encoding in the
corpus.

⭐ **`linux:vino` is now v3, reordered into five contiguous subsystem series** — core (18), crypto
(2), usb (6), drm (61), vino (6) — regenerated into `patches/v3/`. The reorder applied all 93
patches with **zero conflicts** and the tree is byte-identical to what it was folded from; the
driver builds warning-clean and the module hashes the same as the one HW-verified this session
(`2b55001b6c4027f9`).

⛔ **The driver's 33 patches of development history are now 6.** That history carried a revert pair,
a module parameter added and later deleted, selftest corrections and fixes to earlier patches in
the same series. The six introduce the driver in the order it is understood: control protocol,
codec, KMS engine, USB driver, docs — plus the one KMS binding it needs, moved into the `drm`
series where it belongs. ⚠ The cover letters are `format-patch` stubs; write them before posting.

---

## State

| | |
|---|---|
| DL7400 (Navarro) idle stability | ✅ 4-minute soak: 0 resets, 0 connector teardowns, 0 re-activations, 0 flip timeouts |
| DL7400 live mode change (incl. 165 Hz) | ✅ applied with both heads lit, no re-enumeration — **one trial** |
| DL7400 cold plug | ✅ both heads armed from one dual activation, 6.67 s plug-to-pixels |
| DL7400 across a module reload | ✅ both heads back, **4 trials** — was 0 of 2 before the latch fix below |
| Panels actually lit | ✅ **user-confirmed by eye 2026-08-07**: **every pairing** — 1+2, 2+3, 3+4 (cross-endpoint) and 2+4, 1+3 (shared endpoint) |
| Hardware cursor | ✅ user-confirmed on **both** heads 2026-08-07; the selector is `1 << head` |
| HDR | ⭐ **all three wire fields decoded and sent, and now advertised.** KWin reads the dock heads as HDR-capable-but-disabled. Nobody has looked at a panel in PQ |
| D6000 (Ridge) | untouched this session |
| Kernel selftests | ✅ `pass:60 fail:0` |

Module as left installed: `2b55001b6c4027f9`.

### What changed on 2026-08-06 (second session)

1. ⭐⭐ **HDR's missing field is found, and it did not need a capture.** The transfer function is
   **offset-42 bit 6**, `ST2084 colorspace used (HDR)` in DLM's own `setupVideo` log decode. The
   same function also names the DMA formats (`off23 = 3` is `NM30`, closing `hdr_dma_format`) and
   splits offset 68 into two bytes (off69 is the depth code, 30 bpp → 3, so the word is `0x0300`).
   Full write-up in `docs/hdr.md` §0.4a and `docs/protocol/navarro-decoded.md`.
   ⛔ **Cancel the planned keyed HDR capture** — it cannot beat reading the serializer, and it
   cannot settle fields that never vary.
2. ⭐ **A module reload could lose a head for the rest of the session.** The EDID-recovery loop
   stood down permanently on a head's *first* negative presence probe. But this dock reports a lit
   sink absent routinely — that is what `PRESENCE_REMOVE_MS` exists for — so whichever monitor was
   slow at bring-up was latched out and never asked for its EDID again. A negative probe now has to
   outlast the same 5 s window a removal does. See "The reload trap" below.
3. The SDR wire is **byte-identical** to before: nothing offers a 10-bit plane format, so
   `st2084` and `ten_bit` are both false on every mode set vino sends today. The HDR paths are
   pinned by selftests, not by hardware.
4. ⭐ **The experiment switches are gone** — 20 module parameters down to 4. See "Module
   parameters" below for what survived and why, and for the one that was not inert.

---

## ⛔ The trap that cost most of a session

**Correct pixels on the wire are not evidence that a panel is lit.** A fix was reported three times
on the strength of byte-perfect `wire-render.py` reconstructions (3600/3600 strips, 0 decode
failures), both CRTCs `active=1` with framebuffers attached, 60–100 MB flowing under forced damage,
and a stable `devnum`. **Both panels were black throughout.** This dock will accept a complete,
correct frame and never start its downstream pixel clock — the failure mode most of this driver's
history is about.

There is currently **no local oracle**. No vino I²C adapter is registered, so `ddcutil` cannot see
the dock heads, and the presence status word reads `0x00271105` whether the panel is lit or dark.
⇒ **Ask, for "is there a picture". Use the wire for "is the picture correct".** Both tools are
good; they answer different questions.

**And a single trial proves nothing here.** A bisect that "confirmed" a regression was one
observation per build on hardware that re-enumerated ~115 times that night. Two later builds
contradicted it outright, including one confirmed working an hour earlier coming up dark. Repeat
every trial, and prefer a power-cycled dock between them.

---

## ⭐ The reload trap, and how to get a head back

`vino-cycle.sh` unbinds the USB *interface*. That runs `disconnect()` and re-probes, but it does
**not** make the dock re-run its own downstream sink discovery. A sink that goes quiet while vino
is unloaded is therefore never rediscovered, and — before the 2026-08-06 fix — vino made that
permanent by standing its EDID recovery down on the head's first negative presence probe.

Measured: two consecutive reloads, one of two monitors gone each time, flat absent with **zero**
presence transitions over four minutes while its sibling flapped absent-and-back four times a
minute. Three reloads after the fix, both heads back every time.

⭐ **The recovery is a full device re-enumeration, and `authorized` is the safe way to do it:**

```sh
D=$(for d in /sys/bus/usb/devices/*/; do [ "$(cat $d/idProduct 2>/dev/null)" = 7000 ] && echo $d; done)
echo 0 | sudo tee $D/authorized; sleep 3; echo 1 | sudo tee $D/authorized
```

Both heads came back within 25 s. Prefer this to `port/disable` (environment trap 3): deauthorising
cannot latch, because the path stays put whatever the device does.

⚠ **Two diagnostic traps met while chasing this.** The presence status word looks like it encodes
three states — `0x00200105` on empty sockets, `0x00210105` on the lost head, `0x00271105` on the
live one — and it does not: the *lit* head flaps through `0x20` and `0x21` alike, so the low bit
means nothing. And the presence log is **change-only**, so a head that is being probed every second
and answering the same thing every time prints nothing at all; silence there is not absence of
probing.

---

## ✅ What made the DL-7400 stable

Four changes, and the causal chain matters more than any one of them:

> presence flap → connector teardown → compositor re-enables → **single-head mode set while the
> sibling is lit** → dock re-enumerates → 20 s of flip timeouts → 3 s keyframe blast

That loop was the instability, the slow plug-to-pixels, *and* the thing that made a refresh-rate
change look like a 165 Hz problem. It has to be broken in more than one place at once.

### 1. A mode set is dock-wide

`b17d458c489f`. Reconfiguring one connector while any other is lit re-enumerates the dock about
100 ms after the next video write. Measured five ways: 120 → 165, 165 → 120 and 120 → 60 on a live
head; waking a second head one second after the first; and reconfiguring a head whose sibling had
been lit and idle for three minutes. All reset. The same changes with the sibling **disabled** are
clean, and so is the simultaneous `activate_dual_wake` path — which is why cold bring-up always
worked.

This is DLM's own behaviour: its log says `[Profile change] Recreating device`, and it re-runs a
bring-up-shaped burst rather than reconfiguring one connector in place.

Fix: fold every already-active head into the batch and zero its mode generation, which routes them
into `activate_dual_wake`. Gate on the batch containing a **ModeSet** (`cmd_heads != 0`), not
`has_stream()` — a `Blank`-only batch must not re-activate everything.

⛔ **165 Hz was never the problem.** DRM's `2560x1440@165` is byte-identical to DLM's own decrypted
set-mode, down to the 8-line vsync width (5 at 60/120). Only off70 differs, 69950 against DLM's
rounded 69949.

⛔ `unanswered id=0x0048 sub=0x0022` after a mode set is **benign** — a single-head change goes
unanswered too and still works. Same for the `id=0x16 sub=0x2e` after it. Do not chase either.

### 2. Absorb a flap, do not act on it

`5d0899e76d74` → reverted by `856dd105649c` → landed properly in `76e6f4b155be`.

The dock reports a **lit** sink absent for 0.11–2.5 s at a time: 29 runs over three minutes idle,
reaching 2.46 s around a mode change. Vino tore the DRM connector down on each one.

⛔ The old rule ("two consecutive contrary probes") was never a debounce. Every `id=0x44` reply sets
the downstream-event flag, and a presence probe's own reply *is* an `id=0x44`, so the watcher kept
pulling itself forward to `PRESENCE_MIN_GAP` and fired **132 ms** after the first negative.

⛔⛔ **Debouncing alone leaves the panel dark for good.** The teardown was the only *repair*: the
compositor re-enabled the output and that mode set relit the panel. Remove it and vino streams
byte-perfect pixels into a sink nothing is driving.

⛔⛔ **But repairing every flap is worse.** Re-driving the lit heads whenever a sink returns costs a
full dock-wide re-activation — four seconds of cold choreography across both panels — and one fires
every 5–15 s. Measured: a permanent re-activation loop, neither panel ever staying lit.

⇒ The answer is to **absorb** a blip that heals itself (`PRESENCE_REMOVE_MS = 5000`) and act only on
a drop that outlasts it. `repair_flapped_head()` is kept, unused and `#[expect(dead_code)]`, for the
sink that stays down — see "Open" below.

### 3. Wait for a late head before publishing (`76e6f4b155be`)

On a cold plug this dock can answer a head's `id=0x15` EDID fetch **~4 s** after the control session
comes up. The initial recovery asked once and gave up in well under one, so bring-up published a
partial topology — one monitor connected, the other arriving on its own hotplug seconds later — and
userspace mode-set them one at a time, which is exactly what item 1 says the dock will not take.

Now retried to a 6 s deadline, so every monitor lands in the single initial hotplug the bring-up is
built around. Only heads the presence probe has **not** already called empty get there, so an empty
socket still costs one message.

### 4. Release the compositor's flips on unplug (`dfe084622edf`)

`shutdown()` stops the software vblank clock with page flips still armed, and DRM only noticed when
its own deadlines expired: **110 pairs** of `flip_done timed out` / `commit wait timed out` and 73
`vblank wait timed out` in one boot, ten seconds each, all of them between a dock returning and
pixels reappearing. `drm_crtc_vblank_off()` refuses further references *and* sends every event still
queued on the device's vblank list, which is where `PendingVblankEvent::arm` put ours.

---

## Measurements worth keeping

**Plug-to-pixels: 6.67 s**, cold plug, both heads:

```
enumerate                t=0
monitors connected      +2.34 s
control session ready   +2.37 s
both heads armed        +6.67 s     <- plug to pixels
```

The 4.3 s tail is `activate_dual_wake`'s cold choreography. `NAVARRO_REAL_MODE_H0_MS` is 2978 —
DLM's captured offset — but DLM's own prelude finishes at **2016 ms**, so ~960 ms is dead air.
Setting that constant to 2016 should reclaim it; the one attempt to measure that was contaminated
(see the bus-migration trap).

**The post-mode-set training window costs 1.07 GB in 12 s** across two heads: `sustain_until` holds
full keyframes at `FRAME_PERIOD_MS` (5 ms) for three seconds. Cold activation needs it — the dock
will not program its pixel clock without a sustained stream — but a *repair* does not, and
`sustain_window()` skips it for heads flagged in `repair_heads`. Sustained bandwidth is the
documented way to destabilise this dock, so never spend it twice.

**Presence flap shape**, idle and lit: absent 0.11–2.29 s (n=29 over 3 min, mean 0.9 s), present
1.1–19.7 s between them.

---

## ⚠ Environment traps

1. ⭐⭐ **Check `kscreen-doctor -o` for `HDR:` and `Wide Color Gamut:` before investigating any
   colour or brightness complaint.** KWin persisted a single HDR test toggle onto **both** dock
   outputs and kept it there for hours: it was encoding PQ/BT.2020 into monitors still in SDR, and
   the whole desktop looked grey. ⚠ **Vino now advertises HDR unconditionally**, so this can
   recur and there is no module parameter to withdraw it with — the guard is the persisted
   `highDynamicRange: false` in `~/.config/kwinoutputconfig.json`, which was verified false on
   every dock output before the properties were turned on. Check that file, not just the driver.
2. ⭐ **The dock migrates between USB buses.** It appeared as both `2-1.3` and `1-1.3` in one
   session. Never hard-code the bus — resolve by `idProduct` (`7000`/`6006`) — and remember
   `capture-usbmon-session.py --bus` has to follow it.
3. ⭐ `/sys/bus/usb/devices/<port>/port/disable` is a usable cable-level unplug (write 1, then 0),
   but it **latches**: if the device disappears while it is set, the path vanishes and the port
   stays off. Re-enable through the root hub (`usb<N>-port<M>/disable`) and always re-enable from a
   trap or a background safety net.
4. ⭐ **KWin loses the card across repeated module reloads.** It reports both outputs `enabled` while
   every CRTC is `enable=0` with no framebuffer. Check the CRTC state before blaming the driver;
   recover with `kscreen-doctor output.DP-4.disable output.DP-5.disable` then `.enable`.
5. ⛔ **Do not leave `debug=1` on.** The scanout gate logs per head per wake and buries the journal.
   ⚠ Several useful markers (`dual-head activation complete`, `initial ARM+keyframe accepted`) are
   `vino_debug!`, so counting them without `debug=1` silently yields zero.

---

## Module parameters

Four, and none of them is an experiment:

| Parameter | Default | What it does |
|---|---|---|
| `debug` | 0 | Verbose protocol and scanout diagnostics. ⛔ Do not leave it on |
| `trace_crypto` | 0 | Disclose session keys for one decryptable capture. ⛔ Never in normal use |
| `rtc_utc_offset_minutes` | 0 | Local UTC offset for the Navarro RTC message; `vino-cycle.sh` derives it |
| `edid_override` | 0 | Bitmask of heads described by DRM's EDID override, for a sink the dock cannot read |

⭐ **The other sixteen were deleted on 2026-08-06** (`d96a7c8d79e0`). Ten were diagnostics for
closed questions; `flap_repair`, `dock_wide_modeset` and `cursor_enabled` are now simply how the
driver behaves; `hdr_dma_format` and `navarro_mode_offset_ms` had values since read out of DLM's
code; `hdr_advertise` is on permanently. ⇒ **Taking a stability change back out now needs a
rebuild.** That is the trade: they were how this was debugged, and keeping them is how it stops
being reviewable.

⚠⚠ **One was not inert, and this is the lesson.** `idle_opens` defaulted *off* and gated a
cold-activation burst that names the streams vino will not drive. On Navarro `uses_arm_burst()` is
false, so the body had **never once run** — dropping the gate would have enabled it for the first
time. It showed up immediately as ~22 scanout submit failures across a bring-up that had been
clean, and zero after removing the block. Driving a stream at an empty head is a documented way to
re-enumerate this dock. ⇒ **When deleting a gate, check what the default actually did.** A
default-off gate around never-executed code is not dead weight; it is the only thing keeping that
code from running.

⛔ Deleting `simd_transform` orphaned `simd.rs` entirely (nothing else could set `USE_SIMD`), so the
714-line AVX2 module went with it. Not a loss — it was byte-exact but measured parity-to-slower
in-kernel, ~18% more CPU on a live encode. The measurement survives in `docs/simd.md` and
`tools/simd/`; only the unreachable code is gone.

---

## Open, in priority order

### 1. Confirm the picture on the final build

Nothing else is worth doing first. Panels, hardware cursor and a live 165 Hz change each need one
look at the screens on `eac0f8f17c5b5eb6`, on a dock that has not just been power-cycled repeatedly.

### 2. HDR: turn it on and look at the screen

✅ **The protocol side is finished.** All three fields are decoded from DLM's own code and sent:

| field | value | how it was settled |
|---|---|---|
| off42 bit 6 | `0x0040` = ST2084/PQ | DLM's `setupVideo` flag decode at `0x576b26` |
| off23 | `3` = `NM30` | the format-name helper at `0x62ecb0` vs the bpp table at `0x8dc320` |
| off69 | `3` for depth 30 ⇒ off68 word `0x0300` | DLM's `depth` switch |

`Timing::st2084` comes from the connector's `HDR_OUTPUT_METADATA` EOTF, read in `atomic_enable`
through the new `AtomicState::new_connector_state_for_crtc` binding; `Timing::ten_bit` already came
from the committed framebuffer's fourcc. Both are pinned by
`set_mode_carries_depth_and_transfer_function`, which also asserts the SDR bytes are unchanged.

**What is left is one experiment, and it needs a person at the machine.** The properties are
attached now — `kscreen-doctor -o` reads the dock heads as `HDR: disabled` rather than `incapable`,
and `Color resolution: automatic (10), range: [8; 10]` — so nothing has to be rebuilt to try it:

```sh
kscreen-doctor output.DP-4.hdr.enable      # then look at the panel
kscreen-doctor output.DP-4.hdr.disable     # ...and this is the way back
```

⚠ Expect one of three outcomes, told apart by eye and not by the wire: a correct HDR picture; a
*grey* desktop, meaning the sink stayed in SDR while the compositor encoded PQ (the 2026-08-06
morning failure); or a dark panel. ⛔ **There is no longer a module parameter to withdraw the
properties** — if HDR has to go away entirely it is a rebuild. `kscreen-doctor …hdr.disable` is the
first thing to reach for.

⚠ Still unsettled, and neither blocks the experiment: the 10-bit **AC** ceilings (leave them at the
8-bit values — `esc` saturates an over-range magnitude safely, an over-sized ceiling desynchronises
the decoder) and whether `color.rs`'s 8-bit CTM/GAMMA_LUT tables behave at 10 bits.

⛔ **The planned keyed HDR capture is cancelled.** Reading the serializer beat it, and it settles
fields that never vary, which no capture can. The reusable method is
`tools/re/string-store-offsets.py` — the obfuscated string store dumped *in address order*, so a
function's literal run reads out as its argument list and the blob addresses are xref anchors.

### 3. Repair a sink that stays down

`repair_flapped_head()` is written and unused. It re-queues every lit head in one batch, flags them
in `repair_heads` so they skip the training window, and leaves the connector alone. It must **not**
run on a flap that heals — that was the re-activation loop — but it is the right answer for a drop
that outlasts `PRESENCE_REMOVE_MS`, where today the connector still disappears.

### 4. Reclaim the cold-plug dead air

Set `NAVARRO_REAL_MODE_H0_MS` to 2016 (it is a constant now, not a parameter), then re-measure
plug-to-pixels with `debug=1`. Expect ~5.7 s.
⚠ Measure across a real enumeration and check the dock did not change bus mid-test.

### 5. Hardware cursor, second opinion

The selector was a two-entry table on a four-head dock — every message for sockets 3 and 4 returned
`EINVAL` and `cmd_work` dropped it, so no cursor byte had ever reached this hardware. `head + 1` is
correct and was seen working once.

⚠ If the pointer ever needs to go back to software, withdraw the **plane**, not just the messages: a cursor plane whose commit succeeds stops the compositor drawing its own,
so starving it loses the pointer entirely.

⭐ Verifying it without a person at the machine: `/dev/uinput` is not built and KWin exposes no
pointer-move DBus call, so `kscreen-doctor output.eDP-1.disable` is the way to force the pointer
onto a dock head. Then look for **16448-byte** writes on EP02 (32-byte header + 64×64×4 BGRA +
seal); everything else on that endpoint is 64 or 112 bytes.

### 6. Idle/Wake Bug (Screens staying on / blanking and reinitializing)

User-confirmed 2026-08-07: the dock panels stay lit when the laptop screen blanks, and re-run a
bring-up when it comes back. `blank_head()` returns immediately on Navarro (`is_navarro()` early
return) because replaying Ridge's close bracket re-enumerates the dock seven times out of seven, so
today vino sends the dock **nothing at all** on DPMS-off — the dock simply keeps showing its last
frame. When the host wakes and pushes video on the same pipes, the dock rejects the new frames
(`stopped accepting video`) and recovers only via a presence flap (`id=0x0044`).

⚠ **Decoded, implemented, and then deliberately switched back off — read all of this before
re-enabling it.** The blank is HW-verified: both panels went dark on the two markers and the stream
stopped. **The wake is not.** Closing the bracket and re-running the cold choreography left both
panels dark and needed a module reload; that reload then discovered only one head, and a USB
re-authorise did not recover the second — it took a physical dock power-cycle. So `blank_head`
returns early again on Navarro (`4d428b9b49f2`), and `close_blank_bracket` stays in place, harmless
while no bracket is ever opened. ⇒ **Finish the wake before re-enabling the blank**: there is a
captured vino wake to diff against DLM's, which is the method that found all four socket bugs.

✅ **Recorded and decoded 2026-08-07** (`~/vino-dpms-ports-1916/`, window `dpms-fast-1`). DLM's
blank is **four messages and then silence** — per head, `id=0x16 sub=0x2f off23=1` immediately
followed by `id=0x16 sub=0x2e off23=3`, with `off22` the head selector. Nothing else: no video, no
mode set, no close bracket, and no further traffic for the whole 20 s the outputs were down.

⚠ That is the *same* pair `modeset_bracket_pre` sends, and pointedly **not** Ridge's close bracket
(`2f=0`, `2e=0`) — which is the thing measured to re-enumerate this dock seven times out of seven.
So `blank_head()`'s Navarro early return can be replaced by exactly those two markers.

✅ **No connector teardown across a blank** (`83398b4a5528`). The self-blanked guard covered only a
*negative* probe, and this dock flaps a blanked sink back to **present**: the positive branch
re-engaged it and `reengage_head` clears `self_blanked` on entry, so the next sustained negative
tore the connector down. KWin showed every dock output removed and added again across a DPMS blank
and re-laid out the session's windows behind it. Nothing the probe says about a blanked head is
news in *either* direction. Measured after: 0 `presence CLEARED`, 0 `monitor disconnected`, both
connectors up throughout.

⭐ **The wake is a full bring-up in DLM too** — EDID probe (`0x15/0x20`), fetch (`0x15/0x21`),
engage (`0x16/0x23`), set-mode (`0x48/0x22`) and the bracket, ~2.5 s of it. So "the screens do the
bring-up again when the laptop wakes" is not a vino defect; the vendor pays the same cost.

⛔ Not yet cross-checked against the second and third windows (`dpms-fast-2`, `dpms-idle`) — do that
before implementing, single trials have been wrong here repeatedly.

### 7. Ports 3 & 4 — ⭐ the cold timeline is indexed by transcript slot, not by head

**Found by reading, 2026-08-07: this is a driver bug and does not need a capture to explain.**
`activate_dual_wake` builds `slots`/`remap` (`drm_sink.rs:2295`) and then applies it to
**`NAVARRO_COLD_PRELUDE` only**. Everything after the prelude still uses the literal head numbers
of `COLD_NAVARRO`, which was captured with DLM's panels in sockets 1 and 2 and therefore names
heads 0 and 1 throughout. With monitors in sockets 3 and 4 (heads 2 and 3):

* **no video is ever submitted.** `timeline.video` is `[(0, 122), (0, 124), …, (1, 272), …]`; the
  loop skips each entry on `sent & (1 << head) == 0`, and `sent` only ever has bits 2 and 3. The
  cold ARM+carrier never goes out, so the dock never programs its pixel clock — which is exactly
  the reported symptom.
* **no stream markers go out**, for the same reason (`sent & (1u32 << head)` at line 2364).
* **both mode sets take the same reserved counter.** `let slot = if head == 0 { 0 } else { 3 }` —
  heads 2 and 3 both take slot 3. Navarro NAKs from the first flattened counter onward.
* **the 757 ms spacing between the two mode sets is skipped**, because it is gated on `head == 1`.

✅ **Done** (`5fbd2802ffd2`): `slots`/`remap` are hoisted and the whole timeline is indexed by slot
position, resolving to a real head only at the point of send. Heads 0 and 1 map to themselves, so
Ridge and a first-two-sockets Navarro are bit-identical.

⭐ **Validated against DLM**, same monitors, same sockets, an hour apart: DLM streams the
lower-numbered **connector** first whatever endpoint it sits on — ep `0x0a` at +35.641 s then ep
`0x08` at +35.779 s for sockets 2+3 — and vino now reproduces that, 150 ms apart against DLM's 138.

⛔ **But the panels still do not light on sockets 2+3, and this is now the open problem.** Measured
2026-08-07 with the fix in:

* Both heads get their full choreography, both EDIDs (`MSI 0x3cd9` on each), both mode sets, and —
  for the first time — video: **890 MB down ep `0x0a` and 1.79 GB down ep `0x08`** in one session.
* Forced damage answers with ~45 MB per head and no errors, which is the healthy-dock signature.
* **User-confirmed by eye: dark.** And user-confirmed that the same build lights sockets 1+2. ⇒ the
  remaining gate is head-indexed and is *not* the cold timeline, the endpoint pairing, or the
  interleaving (sockets 2+3 sit on different endpoints, so nothing is multiplexed there).
* `tools/hardware/vino-bringup-trials.sh` over 4 cold cycles: **3/4** reached sustained video, one
  hit `stopped accepting video` on both endpoints and never re-armed. So there is a reliability
  problem *as well*, but it is not what keeps the panels dark.

⇒ **Next step, and everything needed for it is already on disk:** a CP message-by-message diff of
vino's own bring-up on sockets 2+3 (`~/vino-ports23-selfcap-1949/wire.pcapng`, taken with
`trace_crypto=1`; the disclosed per-head keys are in the journal at 19:49:54–55) against DLM's on
the *same two sockets* (`~/vino-dpms-ports-1916/`, window `cold-ports23`). This is the method that
named the D6000's gate. ⭐ Start with `id=0x16 sub=0x23`, whose **off23 is a head selector, not a
flag** — DLM's own wake trace sends `off22=0 off23=0` and `off22=1 off23=1`, and a wrong off23 was
exactly what kept the D6000 dark while every ack said yes.

### 8. Implement In-Kernel Firmware Flashing (USB DFU)

To perform on-the-fly firmware updates, `vino` needs to:
1. **Check the Dock Type & Firmware Version:** The standard `bcdDevice` does *not* bump when the firmware updates. Instead, `vino` must read the proprietary DisplayLink **USB descriptor `0x40`** (which DLM calls the "device identity"). 
   - This 16-byte descriptor contains a 3-byte version tuple at offset 2 (e.g., `0x0b 0x05 0x17` for `11.5.23`, updating to `0x0c 0x02 0x1a` for `12.2.26`).
   - It also contains the dock family string at offset 8 (e.g., `NavaDock` for Navarro/DL-7400, or `Ridge` for DL-6000).
2. **Compare against available `.spkg`:** Check `/lib/firmware/vino/` for a matching image (e.g. `navarro-dock-release.spkg` for `NavaDock`). Compare the embedded version.
3. **Run the Flashing (DFU over EP00):** If the `.spkg` is newer, request the firmware via the kernel's `request_firmware()` API. Then, perform a standard USB Device Firmware Upgrade (DFU) over `EP00`.
   - The transfer is standard USB DFU class requests (`DETACH`, `DNLOAD`, `GETSTATUS`).
   - Chunk the `.spkg` file (e.g., in 4096-byte blocks) and upload it using `DFU_DNLOAD` control requests on `EP00`.
   - After each block, poll using `DFU_GETSTATUS` until the dock transitions from `dfuDNBUSY` to `dfuDNLOAD-IDLE` (and ensure the status is `OK`).
   - Send a final zero-length `DFU_DNLOAD` request to manifest the image. The dock will then reboot and re-enumerate with the new firmware!

---

## ✅ Settled — do not re-chase

### The DL7400 codec is correct, including finest detail

Two independent proofs; a round trip through our own decoder proves nothing on its own.

1. **The decoder is a true inverse of DLM's encoder.** Reconstructing `~/dlm-today-124144/wire.pcapng`
   renders the boats wallpaper with rigging and hull trim intact — 3600/3600 positions, **0 decode
   failures**.
2. **vino's bytes round-trip exactly.** Mean absolute error: colour bars 0.06, greyscale ramp 0.05,
   8-px checkerboard **0.00**, **1-px checkerboard 0.00**, diagonals 0.00.

⇒ Transform, quantiser, entropy coder, strip geometry and record framing are all correct. **The
encoder is not the artifact.** Re-confirmed this session: renders of both heads' wire came back
3600/3600 with 0 failures while the panels were black.

### `kind=0x200f` is a per-strip size class

`e91d77a134b1`. `value == strip_byte_length >> 9`, over 68,347 pairs with zero disagreements.

### The `0x9249` second strip encoding is Windows-only

Tested and refuted as content-selected: Linux DLM driving the DL7400 with the same 8-px/1-px
checkerboard wrote **0** on all 1,486,800 strips, with larger strips than the Windows capture.

### 2560x1440@180 is known-bad on the vendor stack too

Under Windows, delivered frame records fell 5,462 → 344 over a comparable window while ep0 control
transfers rose 4 → 1,724, and the dock entered a disconnect/reconnect loop needing a manual power
cycle. `ChangeDisplaySettingsEx` returned success throughout. **Mode acceptance is not evidence of
deliverable bandwidth.** Its 714.81 MHz timing is above `max_head_clock_khz` and is pruned.

### The DL-7400's blank sequence is unknown

`6e8c98d6d8bb`. Replaying Ridge's close bracket makes the dock re-enumerate ~2 s later — seven for
seven, with and without the sink power-down. `blank_head()` does nothing on Navarro until a
transcript establishes how DLM disables an output. The scanout has already stopped by then, because
`atomic_disable` zeroed the head's mode generation first.

---

## ⚠ Measurement traps that cost real time

1. **Filter captures by USB device, not just endpoint.** Both docks use endpoint `0x08`; an
   endpoint-only filter interleaves their records and looks exactly like a driver on the wrong grid.
2. **The Windows corpus is not codec ground truth.** `cap2` fails ~40% of its busy strips through
   `colour_decode`. Ground truth is `~/dlm-today-124144/wire.pcapng`.
3. **A static desktop sends kilobytes, legitimately.** Distinguish a jam from an idle screen by
   forcing damage — `kscreen-doctor output.<name>.brightness.55` then `.100` is reliable, and a
   healthy DL7400 answers with tens of MB.
4. **Backwards strip reconstruction describes the END of the capture.**
5. **`/sys/class/drm/*/status` is cached.** Write `detect` to it first.
6. **`capture-usbmon-session.py` records `S` for OUT endpoints and `C` for IN only.** "Zero
   completions on EP08" is what a *healthy* dock looks like through this tool. Use **bytes
   submitted** over a window with forced damage.
7. **Read the lines either side of the one that matches your hypothesis.**

---

## Tooling

`tools/codec/` — `colour_decode.py` (the codec model), `usbmon_read.py`, `usbpcap_read.py`,
`navarro-render.py`, `depth-probe.py`. `scripts/codec-re/wire-render.py` renders a captured frame
back to a PNG. ⚠ its `--ep` takes a **decimal** int (`10`, not `0x0a`).

```sh
sudo python3 tools/hardware/capture-usbmon-session.py --bus <N> --out cap.mon --snap 65536 --secs 20
python3 scripts/codec-re/wire-render.py cap.mon 2560 1440 --ep 8 --min-strips 1500 --blocks-x 16
```

`tools/capture/capture-portmap.sh` + `tools/capture/dpms-ports-runbook.sh` are the two halves of one
DLM recording sitting — recorder as root in one terminal, guided steps as the desktop user in
another, one DLM process and therefore one set of keys for the whole run:

```sh
sudo tools/capture/capture-portmap.sh --no-reauth --snap 4096 ~/vino-dpms-ports 3600   # terminal 1
     tools/capture/dpms-ports-runbook.sh ~/vino-dpms-ports                             # terminal 2
```

⚠ `--no-reauth` because the panels are already lit and a USB re-authorise brings the control plane
up without relighting them; the runbook records a real dock power-cycle instead. `--snap 4096` keeps
every CP frame whole and the head of every video URB — full capture has reached 50 GB in one run.

`tools/hardware/vino-cycle.sh` reloads the module and derives `rtc_utc_offset_minutes` from the host
timezone; trailing arguments are passed through as module parameters.
`tools/hardware/vino-hold-off.sh <6006|7000>` keeps vino off one dock across re-enumerations.
`trace_crypto=1` discloses session key material for a decryptable capture — never leave it on.

---

## 2026-08-07 (third session): the far sockets work, and why they never had

⭐ **User-confirmed by eye: sockets 2+3 light, both panels, with the hardware cursor on both.**
Three bugs, all the same family — a per-head selector that was read from a capture where every
possible encoding agreed.

| message | was | is | how it was caught |
|---|---|---|---|
| the whole cold timeline | transcript head numbers taken literally | indexed by **slot** | reading `activate_dual_wake` (`5fbd2802ffd2`) |
| `id=0x16 sub=0x23` engage | off22 remapped, **off23 left as the slot** | `edid_sink_state(head, head)`, both bytes | diff vs DLM on the same sockets (`1e351f4f4ab4`) |
| `id=0x15 sub=0x53` post-EDID | `head + 1` | **`1 << head`** | DLM sends 4 for head 2, not 3 (`1e351f4f4ab4`) |
| cursor selector | `head + 1` | **`1 << head`** | exactly one head drew a cursor — the one where they agree (`1e139a98fcd7`) |

### The shared-endpoint pair (sockets 1+3 or 2+4) — one panel lights

⭐ **Diagnosed, fixed, HW-unvalidated.** Four connectors are multiplexed onto two video bulk
endpoints (`0x08` owns {0, 2}, `0x0a` owns {1, 3}), so sockets 2 and 4 put both monitors on `0x0a`.
Measured on vino's own wire: the tagging is **correct** — `sub=0x08` and `0x18` for connectors 1
and 3, stream opens `0x0f` and `0x1f`, both streams accepted, ~150 frames each. One panel lit.

⛔ It is not a bandwidth limit and not the multiplexing. The 2026-08-02 reference capture
(`captures/navarro-pair-ports13-20260802-120220`, connectors 0 and 2 on `0x08`, both lit) carries
**304,356 records at `sub=0x0000` and 240,011 at `sub=0x0010`** with a stream open for each, so the
dock plainly does drive two independent streams on one endpoint. ⚠ Note the first 126 MB of that
capture contains connector 0 *only* — sample the whole file before concluding anything from it.

⭐ **What was missing is a declaration.** DLM's own `setupVideo` flag decode names **offset-42 bit
2 `Dual NIVO`** — the same name as the `TiledNivoViewer` / "Dual NIVO" strings in its binary. vino
set it on neither head. `Timing::dual_nivo` + `endpoint_is_shared()` now do (`df39f5673371`), and
it is inert in every cross-endpoint pairing, so sockets 1+2, 2+3 and 3+4 are byte-identical.

✅ **HW-verified, user-confirmed by eye: sockets 2+4 AND sockets 1+3 both light** — that is both
shared-endpoint pairs, on `0x0a` and on `0x08` respectively. So every pairing of the four sockets
now works.

⚠ **The flag has to be recomputed at send time** (`b0527949c03b`). Whether an endpoint is shared is
a property of the *other* head and changes while this head's timing sits cached, so deciding it in
`atomic_enable` told whichever monitor came up first "not shared" forever — sockets 1+3 lit one
panel until this landed. Confirmed on the wire
too — under forced damage endpoint `0x0a` carries image records at **both** `sub=0x08` and
`sub=0x18`, each with its own stream control (`0x0f`, `0x1f`).
⚠ Needed a dock power-cycle to get there: socket 2's sink had dropped out earlier and a USB
re-authorise did not recover it.
⊙ Still worth a keyed DLM capture in a shared-endpoint configuration — none exists, and it would
say whether the offset-48 allocator rows change when an endpoint carries two streams.

### ⚠ Cold bring-up is not yet reliable: roughly 8 in 10

Counted with `tools/hardware/vino-bringup-trials.sh` (USB re-authorise, then forced damage, then
frames per head — never dmesg alone, and never a single trial):

| pairing | result |
|---|---|
| sockets 3+4 | 3/3 |
| sockets 2+3 | 3/4 — one hit `stopped accepting video` on both endpoints and never re-armed |
| sockets 1+3 | 2/3 — one came up with **both connectors present and no frames at all** |

Two distinct failure shapes, both intermittent, neither yet root-caused. The "no frames" one is the
more interesting: the connectors are there, the activation runs, and nothing is submitted.
⭐ The harness now dumps `dmesg` to `$LOGDIR/vino-trial-fail-*.log` on a failure, because it clears
the log each trial to count frames and these failures are too rare to catch by watching.

⛔ **The lesson, and it is worth more than the fixes.** `head`, `head + 1` and `1 << head` are the
same byte for heads 0 and 1. Every capture in this project's corpus until 2026-08-07 had DLM's two
panels in the first two sockets, so **no per-head encoding in it is evidence for anything beyond
head 1.** Treat every remaining `head + 1` as unmeasured: `cp::connector_marker`'s non-onehot
branch still has one.

⚠ **The dock's `6990d<c>` trace line is a pixel-clock-start oracle for DLM and NOT for vino.** It
appears per lit connector in DLM's transcript (`6990d0`+`6990d1` on sockets 1+2, `6990d1`+`6990d2`
on sockets 2+3) and is absent from vino's log **even when the panels are lit** — vino does not
surface that part of the trace. It was used here to declare a working build dark. Ask a human.
