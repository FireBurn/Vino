# Handover

Single current handover. Last updated **2026-08-11**. Everything below is either still
true, or a trap worth not repeating. Anything an earlier handover said that is not repeated here
was done, superseded, or retracted; the DL7400/Navarro-era handover is in git history.

**Read "Method: how this session went wrong" before changing code.** Three confident conclusions
were retracted in one session, all from the same cause.

## ★★★★★ 2026-08-12: THE RING WAS ONE AHEAD, AND KWIN WAS NEVER DRIVING THE CARD

Two separate faults, both measured, both fixed. Read this before anything below it.

### 1. Nothing had a compositor attached -- a Mesa build, not vino

KWin logged `kmsro: driver missing` -> `Failed to create gbm device for "/dev/dri/card2"` and
discarded the card. `gbm_create_device()` must succeed on every DRM device KWin manages, and on a
display-only KMS device Mesa needs a **software gallium driver**; `/usr/lib64/dri/` had no swrast at
all. `VIDEO_CARDS` in `/etc/portage/make.conf` had `llvmpipe` commented out at 21:37 on 2026-08-10,
six minutes before a Mesa rebuild, and every boot from 2026-08-11 05:32 carries the error while
earlier boots with heavy vino use carry none. Restored `llvmpipe`, rebuilt Mesa, and
`gbm_create_device` on card2 now returns `drm`; after a relogin KWin holds nine fds on card2 and
drives it.

⚠ **Every hardware run between those dates was testing the codec with no compositor attached**,
which is why only synthetic frames ever reached the dock. The handover note below claiming "KWin
does not pick up the hot-added GPU" was this, and is withdrawn.

⭐ The diagnostic is three lines and needs no dock: open each `/dev/dri/cardN`, print
`drmGetVersion`, `DRM_CAP_DUMB_BUFFER`, `DRM_CAP_PRIME`, then `gbm_create_device`. vino advertised
`DUMB=1` and `PRIME=import+export` correctly throughout -- the kernel side was never at fault.

### 2. ⭐⭐ The frame counter was one ahead from the first frame

Measured by walking both concatenated EP02 streams and censusing every non-image video record.
DLM's frame openers, on **both** heads, start at slot 0 with frame counter 1 and walk
`(cur, cnt, next)` = `(0,1,1) (1,2,2) (2,3,0) (0,4,1) ...`. vino's **first** opener was `(1,2,2)`:
it sends DLM's second, third and fourth openers where DLM sends its first, second and third.

```
DLM   #129 @121648  080005000000000001010a000401000000010000000000000000000000000000
vino  #60  @116944  080005010000000101020a000402000000020000000100000000000000000000   <- DLM's #2
```

The builder was right -- `ella_frame_opener` is pinned byte-for-byte for `seq0` 0, 1 and 2 -- and
the counter handed to it was wrong. The stream prologue frame carries **no** opener and, on this
dock, no trailer either, yet it consumed a sequence number. So from frame one the host wrote slot 0
and told the dock slot 1, and every later frame stayed one ahead of the buffer that had the pixels.

⚠ **This un-retracts the "ring off-by-one".** The earlier retraction was correct *at the time*:
with `presentations = 3` the `cur=1` opener really was the same frame's second presentation. Once
content frames went to a single presentation that reasoning stopped applying, and the wire is
unambiguous.

Fixed by stating the rule the wire already implies: **the frame counter counts records that name a
ring slot, not frames.** `names_ring_slot(opener, trailer)` decides it in one place for all three
generations, and both submission paths advance their counter through it. Ridge and the DL7400 close
every frame with a trailer, so they always name a slot and are bit-for-bit unchanged; the DL-3x00
prologue names neither and no longer consumes a slot. Pinned by
`only_a_presentation_that_names_a_ring_slot_advances_the_frame_counter`; selftests read
**`pass:70 fail:0`**.

⭐ Also removed en route: `colour_frame_ep08*` returned `seq0 + 1` alongside its records and
nobody needed it. Two owners of one counter is what let this drift; there is now one.

### 3. ⭐⭐⭐ The second head's setup burst stopped three messages in

Found by aligning both bring-ups record for record from byte zero on shape alone -- record class,
`sub`, `aux`, length -- which needs no keys, because a sealed record's shape is as fixed as its
contents. `tools/capture/sequence-diff.py` does it in one command and exits non-zero:

```
 #31   CP sub=0x24 aux=0x0004 len=188   |  CP sub=0x24 aux=0x0004 len=188    <- both heads' AKE_No_Stored_km
*#32   CP sub=0x24 aux=0x000c len=76    |  CP sub=0x24 aux=0x0009 len=60     <- vendor: LC_Init.  vino: EDID probe
```

Thirty-two records agree exactly, covering the whole plaintext burst and **head 0's complete
nine-message block plus its stream announce**. Then head 1's block simply stops after
`AKE_No_Stored_km`. The vendor sends all nine per-head messages for both connectors; vino sent
three for the second, skipping `LC_Init`, **the key exchange that gives the dock a key for that
head's content stream**, `LC_Send_L_prime`, the stream restatement, the stream announce and both
stream-open control messages.

⭐ **On the head that has the monitor.** Socket 2 is head 1; socket 1 is empty. The empty head
completed its burst and the occupied one did not, which is what makes this a timing fault rather
than a presence one.

Two causes, one each side of the same decision, both fixed:

- **The rrx was sampled, not waited for.** `i >= 3 && !rrx_applied` declares a head sink-less on
  whatever a fixed 10 ms `drain_ep84` happened to see, so a receiver answering a little late reads
  as no receiver at all. Now `wait_perhead_push(AKE_SEND_RRX, 30 ms)` -- which the four-connector
  path has always used, and which the surrounding code already uses for H', L' and Stream_Ready.
- **Naming a head's content stream is not part of authenticating it.** A dock whose video shares
  the control pipe has no video endpoint to open a stream on afterwards, so a head skipped here
  could never be driven at all -- and that is exactly the head a monitor plugged in later arrives
  on. Both exits now go through `announce_stream`, a no-op on a dock that carries its announcement
  ahead of the first frame instead.

⚠ Only the second is certain to be right. The first is well motivated -- it makes the two platforms
agree and only lengthens a wait that currently ends in abandoning the head -- but whether 30 ms is
enough is a hardware question. If a head still reports `no downstream sink`, that is where to look,
and the announce means the stream is named either way.

⛔ **This supersedes the note below** claiming the setup burst opens both heads' streams correctly.
The sealed stream *open* and the plane announce were fixed; the stream *announce* was not, and the
key exchange behind it was never sent at all.

### ✅ HW RESULT 2026-08-12: both fixes confirmed on the wire, and the stall is gone

`~/vinocap/run17.pcapng`, module `c38c41a2`, one bring-up after a physical power cycle.

- ⭐ **The ring walk now matches the vendor exactly.** `ring-openers.py` against the DLM capture:
  `(0,1,1) (1,2,2) (2,3,0) (0,4,1)` on both, `walks agree at the first opener`. It was
  `(1,2,2) ...` before.
- ⭐ **The opening sequence now agrees for 43 records**, up from 32 -- through the whole plaintext
  burst, **both** heads' complete nine-message per-head blocks and **both** stream announces.
  `encrypted control setup complete (33 messages)`, was 27.
- ⭐⭐ **The endpoint stall is gone.** No `-32`, no `EPIPE`, no `video endpoint halt cleared`
  anywhere in the run. The only non-zero completions are `-2`, vino unlinking its own timed-out
  writes during teardown.
- The dock consumes the **entire** prologue-plus-carrier frame (115,104 B, offsets 9,952..124,672)
  and then stops answering. Control writes queued after it time out at one second each.

⚠ Each failed run costs a physical power cycle: the dock hard-wedges off USB afterwards. Budget
one experiment per cycle and make it count.

### ⭐ The next divergence, from the same alignment

The three records the dock does not take are the post-close markers and a mode-set retry. Placed
side by side with the vendor's, the ordering differs in one place:

```
DLM   #86 set-mode  #87 #88 markers  #89 set-mode  #90 #91 #92 markers
      #93 ring descriptor  #95 decoder configuration
      #96 #97 #98 markers          <- the last three, AFTER the configuration
      ... 30 image records, no control at all ...
      #129 first opener

vino  #133 set-mode  #135-#141 six markers
      #145 ring descriptor  #146 decoder configuration
      ... 30 image records ...
      #177 #178 markers            <- the last two, AFTER the whole frame
      #180 a SECOND set-mode       <- the retry; the dock is already silent by here
      #182 first opener
```

So the vendor's final `2e(head, 0)` -- the sink-up -- is the last thing before pixels, and vino's
lands after the first frame has already gone out. That is the standing "move the prologue inside
the bracket" lead, from the other direction: it is the **bracket's tail** that has to precede the
strips, not the prologue that has to move back.

⚠ **That change was tried on 2026-08-11 and called neutral.** It was measured on a baseline where
the ring was one ahead and the second head had no video key at all, so "neutral" said nothing. The
precondition the note attached to it -- "do not land this until the ring off-by-one has had a clean
run" -- is now met.

⭐ **Get a keyed capture before changing it.** vino's markers cannot be read off the wire without
`trace_crypto=1`, and the vendor's decrypted sequence is per-head (`2e(1,3) 2f(0,1) 2e(0,0)`), so
which head each of vino's six markers names is unknown. `record-stream.py` with the disclosed keys
turns the next change from a guess into a diff, which is what produced both of today's fixes.

### Where the dock actually stops, on the wire

`~/vinocap/run14.pcapng`, EP02, one continuous stream:

```
2.177292  S 65536 + 49568   frame 1 (prologue + flat carrier, 115104 B)
2.182316  C 65536           consumed
2.186097  C 49568           consumed
2.189881  S 64  -> reply    control record, answered
2.192879  S 64  -> reply    control record, answered
2.193096  S 65536 + 49296   frame 2, opener (1,2,2) at its head   <- NEVER completes
2.204780  S 65536 x2 + 59632  frame 3
2.354899  C status=-32      the dock stalls, 162 ms later
```

The dock consumes frame 1 in full, answers control immediately after it, and then stops dead at the
first frame that carries an opener. That is the sharpest evidence available that the opener is what
it objected to, and it is what sent this session at the ring rather than at the codec again.

⚠ **HW-unvalidated.** The dock wedged off USB before the fixed module could be tested -- see
below. This is a wire-derived fix with a selftest, not a hardware result.

### ⛔ The dock is now hard-wedged and needs a physical power cycle

Not the recoverable display-function wedge: the DisplayLink silicon does not answer `SET_ADDRESS` at
all (`device not accepting address N, error -71`, `unable to enumerate USB device`), and only the
TI hub `0451:8040` is left on bus 4. **Everything in software was tried and none of it works** --
device-scoped `USBDEVFS_RESET` on the function (`ENODEV`), the same reset on the dock's own hub, the
kernel's own automatic port power cycle, and `4-2:1.0/4-2-port1/disable` 1 then 0. Someone has to
unplug it.

---

---

## Immediate physical state

- The **HP 3005pr (Ella, DL-3900, `17e9:430a`)** is the only dock attached, on bus 4. `bcdDevice`
  **3157**, firmware **12.2.15**. It re-enumerates on a different device number constantly --
  resolve it by `idProduct`, never a path.
- The **DL7400 is not attached.** Everything in this document is Ella.
- ⛔ **Video does not work.** vino brings the dock up, publishes a connector and streams frames the
  dock accepts, and **both panels stay dark** (user-confirmed). What remains is the endpoint stall,
  not the codec -- see "Where the stall actually is".
- ⛔ **Withdrawn: "KWin does not pick up the hot-added GPU".** That was Mesa missing a software
  gallium driver, not hotplug -- see the 2026-08-12 block at the top. KWin adopts card2 on a manual
  bind. `tools/light-head.c` still earns its place for driving a head with no compositor at all; it
  paints a pattern whose bars land on strip boundaries, so a mis-placed strip shows as a bar of the
  wrong width rather than a vague smear.
- vino is **blacklisted** in `/etc/modprobe.d/zz-dl-capture.conf` between experiments; DLM +
  the **C evdi** (`dl-scripts/evdi/module/evdi.ko`, `insmod` from the build dir) drives the screens
  otherwise. ⚠ The installed `/lib/modules/.../evdi.ko` is **revdi**; tell them apart by module
  parameters (stock has `initial_device_count`/`initial_loglevel`, revdi has none).
- ⭐⭐ **A wedged display function is recoverable in software**: a device-scoped `USBDEVFS_RESET`
  (`ioctl(fd, ord('U')<<8|20)` on `/dev/bus/usb/004/005`) brings it back in **5 seconds** --
  `plaintext session initialized`, setup complete, connector up. Verified twice. This removes the
  one-physical-power-cycle-per-experiment blocker; iterate freely.
  ⚠ `authorized` cycling, driver unbind/rebind and hub unbind/rebind all fail -- only the port reset
  works. Check `lsusb -t` first: when the device is still enumerated (audio keeps working under
  `snd-usb-audio`) only the display function is stuck, which is the case a reset fixes.
  ⛔ **Never reset the xHCI controller `0000:08:00.4` or a hub.** The user declined that, and they
  are often remote. A *device* reset is safe here and a controller reset is not: `lsusb -t` shows
  the dock alone on bus 4, while the mass storage and Bluetooth are on bus 3, and the dock's own NIC
  (interfaces 5/6) has no driver bound.
- Captures, both root-owned: `~/vinocap/run5.pcapng` (9.4 GB, 2026-08-10 23:11:41--23:12:03, module
  `cc853a14fb33`, log in `journalctl -b -2`) and `~/vinocap/run6.pcapng` (570 MB, 2026-08-11 06:22
  onward, module `cb579af0`, the first run with the corrected encoder) and `~/vinocap/run7.pcapng`
  (85 MB, 2026-08-11 06:34, module `fd7c6cbb`, both streams opened in setup -- the ring evidence).
- ⭐ The dock survived every run on 2026-08-11 -- it never re-enumerated, so unbinding vino promptly
  when a run window closes really does keep it alive.

---

## What was built

The dock accepted frames and presented none of them because nothing told it there was a stream to
show. Every record it was missing is now built, and each was checked against the capture offline
before a line of it reached hardware.

⭐ **The whole per-head stream setup turned out to be Ridge's `VIDEO_ARM_BURST`, redistributed in
time.** Same records, same builders, different moments -- so almost none of it is new code. What
made it look like a new protocol is that a dock with no video pipe cannot open a stream from inside
a frame, so the opening records live in the CP setup burst instead.

| DLM record | what it is | where vino sends it now |
|---|---|---|
| `type=2` len 32 on `0x08\|head`, body `[sub][6]` (#26/#36) | `cp::stream_announce` -- the same record Navarro's prologue sends | in the per-head CP block, between the `0x26` restatement and `0x14/0x30`, exactly where DLM has it |
| `type=2` len 32 on the video `sub`, body `[sub][0]` (#49/#54) | the same builder, marker 0 | after the sinks are engaged |
| sealed len 48, `aux=0x000a`, seq **0** (#48/#53) | `cp::stream_open(kind)` -- Ridge's arm entry #2 with byte 4 = **1** instead of 3 | same place, immediately before the plaintext one |
| unsealed len 48, `aux=0x0008`, video `sub` (#93) | `video::wht::ella_stream_open`, the ring descriptor | frame prologue, first |
| sealed len 336, `aux=0x0000`, seq **1** (#95) | ⭐ the **decoder configuration** -- `video_arm::build_config`, the same message Ridge and the DL7400 send | frame prologue, second |

⭐ **Record 3 is not a new message.** It is `mode_header(1920, **1088**, 0x1800)` + five code tables
+ the **byte-identical** quantiser table Ridge sends. Only two things differ: the layout word
(Ridge `0x4000`, DL7400 `0x2100`, DL-3x00 `0x1800`) and the code tables, which this dock states as
counted `u16` lists under record kind `0x09` rather than 47-entry `u32` ones under `0x0d`. Both are
now `DockProfile` fields (`layout_word`, `code_tables`), so `video_arm.rs` has one builder for all
three generations.

⭐ **1080 lines are stated as 1088.** The mode header names the *padded* surface the codec actually
produces. Every captured mode on the other two docks is already a whole number of strips, which is
why this never showed up before; `stream_mode_header` now rounds for all of them.

Four more things the wire says, which the old handover did not have:

1. ⛔ **There is no frame trailer, and vino was sending Navarro's after every frame.** Counted over
   60,000 records: 3,941 frame openers, zero trailers. `build_frame_trailer` now returns
   `FrameTrailer::none()` for this dock.
2. ⛔ **The per-frame stream report is not per frame.** DLM sends **14 across 6,812 frames**, always
   in a burst of about three right after a stream opens, never in steady state. vino sent one with
   every frame -- spending both the 0.86 MB/s budget and the stream's sealed block counter on a
   record the dock is not waiting for. Now bounded by `STREAM_REPORT_BURST`. Its content is the
   mode header and a six-byte token, nothing else (`cp::stream_report_mode_only`, 32 B, `aux=0x0006`).
3. ✅ **The frame opener was already right.** All **3,941** captured openers reproduce byte-for-byte
   from `ella_frame_opener`, once the frame counter is restarted at each stream re-open -- which the
   capture does five times and vino did not. `arm_stream_prologue` now resets it.
4. **The dock-wide init records `0x14/0x30`, `0x15/0x0b` and one `0x16/0x2a` per connector** were
   `is_navarro()`-gated and this dock takes them too. Now `DockProfile::dock_wide_init`.

**Sealed block accounting**, which is the easy thing to get silently wrong: the setup open takes
block 0, so the decoder configuration must take block 1 and the first report block 20. That is why
`set_video_keys` now takes a `blocks_used` argument -- resetting the counter to 0 after setup would
replay the keystream and the dock would drop the record without a word.

Also done: `max_frame_bytes` and the keyframe band-split are **gone** (they were built on the
retracted frame-size measurement), and discovery on this dock now walks both heads through each
phase -- probe/fetch each, then engage both, then `0x15/0x53` for both -- instead of taking one head
end to end, which is what left head 0 without an EDID.

## HW RESULTS, 2026-08-10 night -- the root cause is fixed and the failure moved

⭐ **Run with the stream-open fix (module `96c3e6d6`): the dock accepted video for the first time.**

```
head 1 startup frame submitted (115216 bytes)      <- the flat carrier, DLM's is 114,720
scanout head=1 frame ok (8 presentation(s), 1881888 B final write)
scanout head=1 frame ok (8 presentation(s), 1881888 B final write)
scanout head=1 frame ok (8 presentation(s), 1881888 B final write)
```

Three whole-surface keyframes at eight presentations each -- about **45 MB** -- where the same dock
had previously halted the endpoint 128 kB into the first frame. vino's own wire confirms head 1 now
gets `announce(0x09)`, a sealed open at block 0 and `announce(0x01)` before its ring descriptor and
configuration. ⛔ The panels stayed dark, and the dock then stopped answering: 45 MB in two seconds
is ~22 MB/s against the 0.86 MB/s DLM uses.

⚠ **The next run, with the presentation cap and pacing (`d54b8599`), regressed**: no frame reached
the dock at all, the mode set failed, and the dock stopped answering after 5 s. Whether that was
the change or the dock's state is **not established** -- the run before it had flooded a different
dock instance. Do not treat either as settled on one run each.

⛔ **A bug that hid the carrier ramp, found in that log.** `training complete (1 presentations,
0 ms)` -- the carrier is bounded by a frame *count* on this dock (`carrier_presentations`), but the
wall-clock check ran first and `carrier_ms` returns 0 here, so it ended after one frame whatever the
count said. The ramp DLM shows (seven flat frames before a full-detail one) was never actually
being sent. Fixed by skipping the time check when the window is zero.

⚠ **Socket 1 never recovers an EDID** even with a monitor attached, so head 0 has never been driven
in any run. Its presence probe answers `id=0x0044 status=0x00100104 -> present=false`. Whether that
status word decodes differently on this dock is untested; DLM's own replies to the same probe are
in the capture and have not been decoded.

⛔ **Each attempt costs a physical power cycle.** The dock's display function wedges after a failed
run -- `no identity descriptor` on probe, `can't set config #1, error -71` -- and *no* software path
recovers it. Tried and failed: `authorized` cycling, device-level unbind/rebind, and hub
unbind/rebind (which the kernel logs as a genuine `attempt power cycle`). Unbinding vino promptly
when the run window closes is what keeps the dock alive for a retry.

## ★★★★★ THE ENCODER SPEAKS THE WRONG DIALECT (2026-08-11) -- read this first

The record stream is right. The **strip payload inside it is not.** For the identical flat black
frame, at the identical declared length, in a record whose header is byte-for-byte DLM's, the two
encoders emit different coded bits:

```
vino:  fc00 7e00 3f80 1fc0 0fe0 07f0 03f8 01 ...   (54 B, repeats every 15)
DLM:   5415 aa0a 5585 aa42 55a1 aa50 55a8 2a ...   (54 B, repeats every 15)
```

⭐ **This is apples to apples and there is no room to argue with it.** Both frames are uniformly
flat: 1024 strips, **exactly one distinct payload each**. Both records are size 4044, both walk the
same x/y sequence (`0,0` -> `768,80` -> `1536,144` -> ...), both carry `A=C=D=54`, and the first 18
body bytes are equal. The difference starts at the first coded bit and never recovers.

Read LSB-first (the packer is LSB-first) the mechanism is visible:

```
vino:  00 111111 0000000000000 111111 ...     one unary run, category 6
DLM:   00 1 0 1 0 1 0 1 0 1 0 1 0 1 0 ...     repeated category-1 symbols
```

⛔ **`CodeTables` reaches the configuration record and never reaches the encoder.** `profile.rs`
sets `code_tables: Narrow` for this dock and `video_arm::build_config` faithfully states the narrow
tables to the dock -- then `video::wht` encodes with Ridge's ceilings regardless. The dock is told
one code and sent another, so every strip decodes to noise. That is exactly "accepts every frame
and presents none", and it is enough on its own to explain a decoder that eventually stalls its own
endpoint.

⚠ **Corrects a five-star claim.** "Ella needs almost no new code: framing is Ridge's byte-for-byte"
is true of the *record framing* and false of the *strip grammar*. The narrow tables are shorter than
the wide ones because the category ceilings are lower; they are not a restatement of the same code.
Confirmation from the other direction: `scripts/codec-re/colour_decode.py`, which implements Ridge's
grammar, decodes vino's strip to an all-zero significance field (a flat block, correct) and decodes
DLM's to `last = 61/63` on a strip whose own header says there is no AC data at all.

**Reproduce it in one command** -- `vino/tools/capture/strip-diff.py`, which joins a capture's OUT
transfers into the single record stream the dock actually parses, walks it, and censuses the coded
strip payloads:

```
tools/capture/strip-diff.py captures/ella-video-evdi-20260810/wire.pcapng run5.pcapng \
    --frames 15 --until '2026-08-10 23:11:46.75'
  ... 1024 strips, 1 distinct payloads   5415aa0a5585aa42...
  ... 1024 strips, 1 distinct payloads   fc007e003f801fc0...
  0 payload(s) in common
```

⭐ **This is a hardware-free oracle.** A flat frame collapses to one payload, so the encoder can be
corrected against DLM's 54 bytes in a KUnit test and iterated without spending a power cycle per
attempt. ⛔ The old selftest (`flat strip 54 B (DLM sends 54)`) only ever compared the **length**,
which is why this survived.

✅ **FIXED, and verified on the wire.** `DLM_FLAT_STRIP` in `tests.rs` pins DLM's 54 bytes and the
flat-strip test asserts against them; selftests read `pass:69 fail:0`, and a hardware run confirms
vino now puts DLM's exact bytes on the wire:

```
tools/capture/strip-diff.py ~/vinocap/run6.pcapng --frames 30
  2040 strips, 1 distinct payloads   5415aa0a5585aa4255a1aa5055a82a...
```

### The rule, and how it was derived

⭐ **Every unary run in a strip is spelled the same way, and the generations differ only in where
the payload sits.** Ridge and the DL7400 emit `1^c`, a `0` terminator, then `c` payload bits. A
DL-3x00 decoder expects **one payload bit immediately after each unary one, with the terminator
last**. Both spellings are the same length, which is why the records were always exactly the right
size and always decoded to noise.

In every field the payload is exactly `c` bits, which is what makes it one primitive rather than
four: the escape carries `offset(c-1) ++ sign`, the chroma node a `c`-bit offset, the luma node a
`c`-bit position, and a flat block the maximum category with an all-zero payload.

Derivation, in the order that worked:

1. A flat frame collapses to one strip payload, giving one symbol exactly: DLM's
   `00 1010101010100` against vino's `00 1111110000000` -- the same six ones and seven zeros,
   interleaved.
2. Harvesting strips where exactly one block is non-flat (anchor the tail against 15 known flat
   units) gave real non-flat symbols. Five distinct luma symbols and a 27-bit chroma one all
   decode exactly under the interleaved rule and none under Ridge's.
3. Scoring the whole corpus: the main section (16 significance units + 48 DC escapes) must fit
   inside `w18` with only zero padding. **Ridge's grammar 217/6000; the interleaved rule 6000/6000.**
   Whole strips including both AC rows: **81%**.

⚠ **The oracle has to tolerate padding.** An earlier pass scored "ends exactly at `w18`" and got
~6% for every hypothesis including the right one, because the main section is padded by two or
three bytes. That wasted a search; the fix is to require zero bits after the decode, not an exact
landing.

### ⭐⭐ DLM's frames decode to a picture -- the grammar is confirmed end to end

`tools/render-dc.py` reconstructs a frame from the **DC coefficients alone** and writes a PNG.
Pointed at the DLM capture it produces a recognisable desktop -- a window, a sidebar, a file list --
across three separate keyframes, with **2040 strips and 0 undecodable** each time
(`captures/ella-video-evdi-20260810/decoded/frame-dc.png`).

That is the strongest validation available without a dock, and it is a *positive* one rather than
another elimination: the interleaved unary rule, the three-plane significance tree, the DC DPCM and
the 8-across-by-2-down block layout are all right, because getting any of them wrong turns the
render into noise. A DC-only render is the sharp test precisely because the main section decodes at
100% while the AC section does not -- the picture cannot be coming from anywhere else.

⚠ vino's own frames cannot be rendered from any capture yet: every strip it has put on the wire is
the 54-byte flat one, because its content frame never survived the stall.

### The AC section, narrowed (offline, no dock needed)

Whole-strip decoding is at **15% of AC-bearing strips**, up from 3.5%. What the corpus settles:

- ⛔ **The AC coefficient count is `last`.** A landing sweep says `last + 1` scores 4.5x better;
  that is an artifact. Fitting the count directly on strips whose AC row holds exactly one
  significant block *and* one significant plane gives `count = last` with **zero slack** for chroma
  (Cr `last=2` -> 2, Cb `last=6` -> 6), and for luma `count - last` is exactly the trailing padding
  (15, 15, 6, 0 bits). A greedy counter cannot tell a zero coefficient from a padding bit -- both
  are a single `0` -- so it over-counts by the padding and makes `last + 1` look better.
- ✅ **AC escapes use the same interleaved form** as every other field, 3.5x better than the grouped
  spelling. One rule really does cover the whole strip.
- ⛔ **Plane order is not determinable from a landing oracle**: every ordering scores identically,
  because the three counts come off one bit reader and only their total moves the landing point.
- ⛔ **Nor are the ceilings**: `(10,10,9)`, `(9,9,9)`, `(10,10,10)` and `(9,9,10)` give *identical*
  scores, so no AC magnitude in this corpus reaches a ceiling.

⚠ 85% still fail. With the count settled at `last` and the escape shared with every other field,
what is left is most likely a **position-dependent ceiling** -- Ridge's decoder already varies the
quantiser step by coefficient index (`step_bias`, `chroma_ac_step`), and a ceiling that moves the
same way would mis-parse only the coefficients that reach it. That is invisible to every oracle used
so far, because a constant-cmax sweep scores identically when the corpus rarely saturates.

⛔ **The landing oracle is exhausted** -- it sees only the *total* coefficient count, so plane order,
per-block versus per-plane grouping and the ceilings all score byte-identically. The next pass needs
the pixel decoder (`tools/render-dc.py` extended through the AC section) checked against a rendered
reference, not another sweep.

⛔ **Retracted en route**: the AC category ceilings are *not* the residue -- sweeping `ac_cmax` and
`chroma_ac_cmax` over 6..11 is flat at 77%. The remaining 19% is something else in the AC section.

### Shared, not duplicated

The dialect is `DockProfile::code_tables`, the field that already chose which tables the
configuration record states -- so the code vino emits and the code it declares cannot drift apart
again. `Bits::unary` is the single primitive; `esc`, `chroma_base` and `sync_unit_after` all route
through it, replacing four hand-rolled bit loops. Ridge and the DL7400 keep `Wide` and their bytes
are unchanged, which their existing byte-exact tests pin.

### ⛔ Still dark: a second cause, independent of the codec

With byte-correct strips on the wire the dock **still stalls**, 264 ms after the first frame:

```
KMS CRTC enable -- head 1 display ON, mode 1920x1080@60
head 1 startup frame submitted after 0 ms (115216 bytes)
head=1 endpoint=0x02 stopped accepting video: GET_STATUS=0x0000 halt=0
scanout head=1 pipeline submit failed; 29 records, largest stride 4048, largest strip 54
```

⭐ **The control session is fine until video starts.** A run that loaded vino and sent *no* video
held for the full window -- `CP keepalive finished (971 polls)`, no stall. Video is what kills it,
and now provably not because of what the strips contain. The steady frame is 114,752 bytes, exactly
DLM's; the startup frame is 464 bytes larger, which is the prologue.

✅ **One real bug found and fixed here anyway.** The setup burst opened streams only for heads that
had answered with an EDID, so on a dock where socket 1 is empty it opened **head 0** -- the head
with no monitor -- and never head 1. vino's own wire says so plainly:

```
setup:  announce 0x0008, sealed open 0x0008, announce plane 0x0000     <- head 0, no monitor
frame:  announce 0x0009, sealed open 0x0009, announce plane 0x0001     <- head 1, far too late
```

DLM opens **both**. The gate is gone (`control setup complete (27 messages)`, was 25), and with it
the prologue-in-frame fallback that existed only to paper over it -- a dock with no video pipe
cannot open a stream from inside a frame, which is exactly what that fallback was doing.

### ⭐ Where the stall actually is: the dock's ring, not its parser

With both streams opened correctly the failure is unchanged, and the wire now pins it exactly
(`~/vinocap/run7.pcapng`, 06:34:05.6 onward):

```
05.667 .688 .709 .731 .752 .773 .794   7 training presentations, ~21 ms apart, all complete
05.804 .825                            complete
05.846 .867 .888 .909                  submitted, NEVER complete
05.931                                 GET_STATUS on ep80 -> 0000, endpoint NOT halted
06.003                                 C ep02 status=-32, the dock finally stalls
```

⭐ **The dock accepts about nine frames and then stops consuming**, and only stalls 170 ms later,
after vino has queued four more. The 264 ms in the log is vino's own write timeout, not a dock
event -- `GET_STATUS` reporting `halt=0` at 05.931 proves the endpoint was healthy while writes were
already going unanswered.

⛔ **Four flow-control explanations were then measured dead**, so do not spend another session on
pacing:

| theory | measurement | verdict |
|---|---|---|
| the dock acknowledges each frame and vino ignores it | DLM: **7113 frame openers, 312 EP84 IN messages total** (0.017 per frame) | there is no per-frame signal; DLM never waits |
| vino queues too many transfers ahead | peak in flight on ep02: **DLM 8, vino 9** | comparable |
| vino's frame rate is too high | DLM inter-frame gap **median 16.5 ms, minimum 0.2 ms, 5570 of 6421 under 20 ms** | DLM runs ~60 fps per head, *faster* than vino's 21 ms |
| vino mis-rotates the ring | openers walk `cur/next` 0->1, 1->2, 2->0 with the frame counter advancing, **byte-identical to DLM's** | correct |

⚠ **"DLM never starts a frame within 20 ms of the last" is RETRACTED** -- it was an average
mistaken for a floor. The real floor is 0.2 ms.

⛔ **The frame-1 prologue matches too** -- checked last. DLM's is exactly two records before its
first image record, and vino's is the same two, in the same order, on the same subs, with a
byte-identical ring-descriptor body:

```
DLM   sub=0x0000 aux=0x0008 seq=0 len 44   0a00040000000000000000020a000400...
      sub=0x0008 aux=0x0000 seq=1 len 332  (decoder configuration)
vino  sub=0x0001 aux=0x0008 seq=0 len 44   same body, plane sub of its own head
      sub=0x0009 aux=0x0000 seq=1 len 332
```

Neither sends an `aux=0x000a` opener for frame 1 -- an earlier note here claimed DLM did, which was
an artifact of scanning from the middle of a stream. Frame 2 onward carries one in both.

⭐ **So the dock takes 7113 of DLM's frames at 60 fps and stops after nine of vino's, with framing,
strip content, slot rotation, cadence and the frame-1 prologue all byte-identical.** Whatever is
left is cumulative and structural.

⛔ **Also checked, also not it**: the sink is brought up. `modeset_bracket_post_open` ends with
`2e(head, 0)` at +26 ms on this dock (`reopen = 0` when `video_on_ctrl_pipe()`), so the head is not
left powered down. ⚠ What has *not* been reproduced is DLM's **interleaving**: it brackets both
heads together (`set-mode h0, 2f(0,1), 2e(0,3), set-mode h1, 2f(0,1), 2e(0,0), 2f(1,1)`, then
h0's ring descriptor and configuration, then `2e(1,3), 2f(0,1), 2e(0,0)`, then h0's strips), where
vino runs one head's whole bracket then the other's. On a dock this sensitive to a head that is not
attached, that ordering is worth reproducing exactly.

⛔ **The sealed decoder configuration matches too.** Decrypted from DLM with
`tools/capture/record-stream.py` (record #95, `sub=0x08 aux=0x0000 seq=1`, 304 B of content) it is
exactly what `video_arm::build_config` produces: `mode_header(1920, 1088, 0x1800)`, the five narrow
tables (`1,0,2,0,...,256,512` / `...,512,1024` twice / `1,0,2,0,4,0,8,15,2` / `...,64,127,2`) and
the quantiser table. Nothing to find there.

### ⛔ RETRACTED: the "ring counter off-by-one"

An earlier pass here claimed vino's first opener named slot 1 where DLM named slot 0, and shipped a
fix. **That was wrong and is reverted.** `repeat_seq = seq0 + repeat` in the scanout loop, so the
`cur=1` opener being compared against DLM's `cur=0` is vino's *second presentation of the same
frame*, not the next frame's opener -- unlike things compared. The existing code already lands the
following frame on slot 0. Kept here only so it is not rediscovered as a lead.

<details><summary>the retracted reasoning</summary>

The decrypted sequence did show one real difference. DLM's frame-1 prologue puts that frame in
**slot 0** (its ring descriptor body is `0a 00 04 00 ... 02 0a 00 04 00`), and the first opener
after it carries **`seq0 = 0`, `cur = 0`** -- an opener trails the frame it describes. vino's first
opener carried **`seq0 = 1`, `cur = 1`**, contradicting its own ring descriptor: it claimed the
prologue frame had landed in slot 1. From frame one, host and dock disagreed about every slot.

Fixed by not consuming a sequence number for the frame that carries the prologue, **scoped to
`video_on_ctrl_pipe()`** so Ridge and the DL7400 -- whose prologue is the arm burst and whose ring
maths is working -- are untouched.

⛔ The run that appeared to test it was also invalid: the machine was at load **21.9** with a 31 GB
Gradle tree filling `/tmp`, and this driver starves its scanout worker under CPU load. **Check
`uptime` and `df -h /tmp` before believing any hardware result on this box.**

</details>

### ⭐ FIXED: the carrier was seven frames where the vendor sends one

`carrier_presentations()` returned `COLD_TRAINING_PRESENTATIONS - 1` = **7** for this dock, on a
comment claiming DLM sends "seven on the head whose next frame is the whole surface in detail".
The capture says otherwise: classify every frame by mean strip size and **every stream opens with
exactly one** 54-byte-per-strip frame and then goes to content -- never a run of them.

```
strips=2040 mean=  54 B  FLAT      <- one, then straight to content
strips=2040 mean= 174 B  content
strips=2040 mean= 174 B  content
```

Set to 1, and the failure moved: `endpoint stopped accepting video` is **gone**, the flat carrier is
accepted, and vino now reaches a real content frame (`first=190704, max_strip=154` -- the test
pattern). Holding the endpoint through six extra full-surface presentations was silencing the
control plane, which is exactly what the surrounding comment warns a wall-clock window would do.

### ⭐⭐ A CONTENT FRAME IS NOW ACCEPTED

Second fix, the same class as the carrier: vino presented a full keyframe `dock_buffers` = **3**
times where the vendor sends every content frame **once**. On a dock whose frame opener names the
ring slot, the dock rotates its own buffers -- a keyframe does not have to be repeated to reach
them. Set to 1 and:

```
head=1 chunks=12 first=190704 presentations=1 records=48 max_strip=154
scanout head=1 frame ok (1 presentation(s), 190704 B final write)
```

**190,704 bytes of real picture accepted end to end.** Every earlier run died inside the first
content frame; this is the first that did not.

### The failure now: the control plane dies, then the next frame

```
scanout head=1 CP status poll failed (EPROTO)      <- control breaks FIRST
scanout head=1 frame ok (1 presentation(s), 190704 B final write)
head=1 chunks=12 first=190640 presentations=1
scanout head=1 pipeline submit at off=131072/190640 failed
dock has answered nothing for 5014 ms; abandoning the session
```

⛔ **The 128 KiB offset is a coincidence, not a boundary** -- across runs the failing offset is
0 or 131072 for the same frame, and DLM's uninterrupted image runs reach **1,882,144 bytes** (453
over 128 KiB). Do not chase it.

⭐ **`CP status poll failed (EPROTO)` fires before the video failure**, so the control channel is
what breaks and video follows. The wire shows vino queueing six URBs (~295 KB) back-to-back across
a frame boundary, where the frames that succeed complete before the next is submitted.

⛔ **TESTED: a shallow video queue is NOT the answer.** Depth 2 instead of 8 for this dock -- on the
theory that a control record queued behind eight 64 KiB transfers waits for half a megabyte and
times out -- made it strictly worse: the *first* content frame then failed at offset 0. Reverted.
Whatever the dock needs, it is not less data in flight.

⚠ **Frames accepted per run varies: 1 or 2 with the same build**, so treat the count as noisy and
do not read a small change in it as a result.

### The control poll was running 33x the vendor's rate

Measured over DLM's whole capture: **225 host-to-dock control records in 326 s = 0.69/s**, of which
the `0x14/0x0c` status poll family is 129 -- one every **2.5 seconds**. vino sent one on *every*
pass of the keepalive loop, about 13/s (`CP keepalive finished (1476 polls)` in one session). On a
dock whose video shares this endpoint each poll is bytes queued against a frame and a reply the dock
must produce mid-scanout.

Now bounded to 250 ms (`STATUS_PERIOD`), a 19x reduction and still six times the vendor's rate, kept
conservative so the 5 s no-answer watchdog still sees several replies per window.

⚠ **Measured, but not individually decisive**: the run after it still accepts one content frame and
then fails the same way. It is kept because matching the vendor is what produced both fixes above,
not because this run proved it. Tightening it further toward DLM's 2.5 s is untested.

⚠ **Nobody can see the panels right now** (the user is remote), so "accepted" is as far as the wire
can take this. A run with eyes on the monitor is needed to close it.

### Superseded: a content frame dies at 128 KiB

```
head 1 startup frame submitted (115104 bytes)     <- accepted
training complete (1 presentations)
head=1 chunks=12 first=190704 presentations=3 records=48 max_strip=154
scanout head=1 CP status poll failed (EPROTO)
scanout head=1 pipeline submit at off=131072/190640 failed
```

The flat frame is 115,104 B and lands; the content frame is 190,640 B and dies at **exactly
131072 = 2 x 65536**, two of its twelve chunks in.

⛔ **Not a dock receive budget.** DLM's uninterrupted image runs reach **1,882,144 bytes**, with 453
runs over 128 KiB and 837 over 64 KiB, so the dock will take 1.88 MB of pure image records with
nothing interleaved. Something about vino's third chunk is the problem, not its size.

⚠ Also still divergent, and the same class of error as the carrier: vino presents a full keyframe
`dock_buffers` = **3** times; DLM presents content frames **once**. That is worth fixing next, though
it is not what broke this frame -- the failure is inside the first presentation.

### ⛔ TESTED AND DISPROVEN: the prologue inside the bracket

Implemented from the decrypted DLM sequence: `send_stream_prologue` writes the ring descriptor and
decoder configuration on the control pipe between the `+20 ms 2f(1)` and `+26 ms 2e(0)` markers, so
the sink-up is the last thing the dock sees before pixels. On a dock with no video pipe
`build_stream_prefix_buf` now yields nothing -- the prologue has already gone out -- and the frame
still opens the generation, so it still carries no opener. No new flag and no second path: where a
generation's prologue goes is decided in one place. Ridge and the DL7400 keep theirs on the first
frame, which is where their own captures put it.

**Tested 2026-08-11 and reverted.** With the prologue moved into the bracket the dock stopped
accepting after the first frame; with it reverted, **the same thing happens** -- so the change is
neutral, not harmful, and the hypothesis that the final sink-up latches the configuration is dead.
⚠ The earlier "~9 frames accepted" figure does not reproduce either: after a USB reset the dock
takes **one** frame and stops, at 89-108 ms, in both builds. How many frames it accepts is dock
state, not a property of the code -- do not use it as a signal. Tree is back to `8e8d2a79`.

### What the wire says about the stall

The dock **STALLs bulk OUT** (`status=-32`) ~200 ms after video starts and never speaks on EP84
again -- last IN completion 23:11:46.649, then 146 MB of OUT accepted in silence. Because control
and video share the record stream on this dock, losing the video decoder loses the control channel
with it: `unanswered id=0x0014`, `socket 1 re-engage failed`, `dock has answered nothing for
5057 ms` and the abandoned session are all one event, not four.

⛔ **RETRACTED: rate, pacing and flooding.** Measured over DLM's whole 326 s capture: mean
**0.888 MB/s** but a peak of **82.9 MB/s** in a 100 ms window, minimum inter-submit gap **14 us**.
vino's peak was ~43 MB/s. The dock is not being over-driven, "never <20 ms between frames" was the
frame cadence and not a rate limit, and **DLM never once stalls EP02 in 290 MB** (all 23 non-zero
completions in its capture are `ep80` control probes before the session). The presentation cap and
the `frame_period_ms` pacing added for this are not load-bearing; leave them until pixels land, then
take them out.

⛔ **RETRACTED: the image record `aux` field.** DLM spreads image records over eight even values
(`0x0000`..`0x000e`) and vino sends `0`. It is **uninitialised sender-side garbage**: `(sub,x,y)`
does not determine it (3637 of 3896 keys see more than one value), the repeat-gap histogram is
geometric with p = 1/8 (i.e. uniform random), and DLM's own **first frame is all zeros** -- the
signature of fresh heap. The dock ignores it. vino's `0` is correct.

⛔ **RETRACTED: keystream replay in the frame opener.** The opener's header `seq` is `0` on every
one of vino's frames, which looked like the documented block-replay trap. It is not: DLM sends `0`
on all **3711** of its openers. The openers are plaintext; only `sub=0x0009` carries block
accounting, and vino's is right (open `0`, config `1`, reports `20, 22, 24` -- exactly as
documented).

✅ **vino's record framing is confirmed correct**, a second way: the training burst walks as 416
records ending *exactly* on the buffer boundary with zero remainder, and its record kinds, sizes and
seq fields line up with DLM's.

## ROOT CAUSE, from vino's own decrypted wire (2026-08-10 late)

⭐ **A head that gains a monitor after CP setup was driven on a stream that was never opened.**
Captured with `debug=1 trace_crypto=1` and decoded with `tools/capture/record-stream.py`, vino's own
EP02 stream reads:

```
#1 type=2 sub=0x08          announce         <- head 0's stream
#2 type=4 sub=0x08 aux=000a seq=0  open      <- head 0
#3 type=2 sub=0x00          plane announce   <- head 0
#4 type=4 sub=0x01 aux=0008 seq=0  ring desc  <- HEAD 1
#5 type=4 sub=0x09 aux=0000 seq=1  config     <- HEAD 1, sealed at block ONE
```

Head 1 is the head with the monitor. It has no announce, no plane announce and no sealed open --
and its configuration seals at **block 1 with block 0 never sent**. Head 0, which has nothing
plugged into it, got the whole opening instead.

The cause is the setup burst opening only the heads that authenticated *during* setup (`head_ok`),
while `set_video_keys` started **every** head's block counter at 1 on the assumption that it had.
A monitor plugged in afterwards comes up through runtime discovery, so its head is driven with a
sealed chain the dock accounts for as a keystream it never received.

Fixed: `set_video_keys` takes a per-head opened bitmask, and a head whose stream was not opened in
setup emits its own announce, block-0 open and plane announce at the head of its prologue, in the
same order the setup burst uses. ⚠ **HW-unvalidated** -- the dock wedged before the fix could run.

⛔ **Two more candidates died on that same run**, thanks to the frame-shape line the submit failure
now prints: `records=522 max_stride=4080 max_strip=1490`. The largest stride is exactly the cap, not
over it, and the largest strip is well under the 2036 DLM reaches. The frame was well formed. And
DLM sends contiguous image runs up to **1,882,144 bytes** with nothing interleaved -- 453 of them
over 128 kB -- so there is no 128 kB budget either.

## The live failure, measured three times (2026-08-10 late)

With the stream records in, the dock gets further -- `socket 2 monitor connected`, EDID, mode set --
and then **halts EP02 partway into the first full keyframe**, identically every time:

```
vino: scanout head=1 pipeline submit at off=655360/1881888 failed
vino: head 1 scanout frame failed (EPIPE) -- throttling
vino: dock has answered nothing for 5004 ms; abandoning the session
```

⚠ `off=655360` is **not** where the bad byte is. The video queue is 8 URBs deep and `send()` reaps
a completion when its slot is reused, so the error surfaces about eight transfers after the dock
actually halted -- somewhere in the first ~200 kB of the frame.

⛔ **Three explanations are refuted by measurement, do not re-chase them:**

- **Not frame size.** DLM's largest frame here is **1,882,144 B**, against vino's 1,881,888. It
  sends 31 frames over 655,360 B on one head. (This also confirms removing `max_frame_bytes` was
  right, for a better reason than the one it was removed for.)
- **Not rate.** DLM pushes a 1,882,208-byte frame in **11 ms = 178 MB/s**.
- **Not burst length or reply pacing.** DLM sends up to **3,982 consecutive OUT transfers** with no
  reply from the dock at all. Its max transfer is 65,536, the same as `VIDEO_XFER`.

### A latent framing bug, found with no dock attached

⭐ **A selftest can encode a worst-case strip and check it against the wire's own limits**, which
costs nothing and needs no hardware -- `an_encoded_strip_never_exceeds_the_decoder_input_bound`.
It found one immediately: `frame_records` took the *first* strip of a record at any size (the
`n > 0` guard), so a strip too big for a record produced a record whose **stride was over
`STRIDE_CAP`** -- a wire violation the captures show never happens once in 92,072 records, and which
a dock can only report by halting the endpoint, several transfers later, looking exactly like a
transport fault.

Measured on pseudo-random pixels, the worst case for an entropy coder:

| profile | worst encoded strip | fits a record (4062)? |
|---|---|---|
| 8 bits per channel | **3186 B** | yes, with margin |
| 10 bits per channel (DL-7000 HDR) | **4576 B** | **no** |

`frame_records` now refuses such a frame with `EOVERFLOW` and names the strip, so the encoder is
what gets looked at rather than the transport. ⚠ **This is not the DL-3x00 failure** -- that dock
runs 8 bits per channel and stays inside the bound. It is a real bug on the HDR path that the dark
panels happened to lead to.

⚠ For reference, DLM never exceeds **1758** bytes per strip on DL-6xxx, **1780** on the DL-7400 or
**2036** on DL-3x00, across roughly half a million strips. Those are content maxima, not a device
limit: the quantiser is fixed per stream and transmitted in the decoder configuration, so DLM has
no per-strip rate control either and cannot be bounding them deliberately.

⭐ **The one clear behavioural divergence left.** DLM's first frame on *every* fresh stream is
exactly **114,720 bytes** -- which is 2040 strips x (54 + 2) + 30 record headers, i.e. the whole
surface as *flat* strips. On head 0 it then goes to ~361 kB content frames; on head 1 it sends
**seven** flat frames before its first 1.88 MB one. vino sends one flat carrier and then goes
straight to a full-detail 1.88 MB keyframe, three times over (`repeat_count = dock_buffers`).

⭐ **vino's codec is not what inflates the frame.** Its flat strip encodes to **exactly 54 bytes**,
byte-identical to DLM's, so its carrier frame is DLM's 114,720 to the byte; a smooth gradient costs
424. The 1.88 MB keyframe is simply a busy desktop, and DLM's own largest frames average the same
917 bytes a strip. Pinned in `an_encoded_strip_never_exceeds_the_decoder_input_bound`.

⭐ **What vino does differently is the ramp.** `carrier_ms` returns 0 for a dock that shares its
control pipe, which collapses the carrier to a **single** presentation -- deliberately, because a
wall-clock carrier window holds the shared endpoint and silences the control plane. DLM presents
that same flat frame **once** on the head whose next frame is small, and **seven** times on the head
whose next frame is the whole surface in detail. It never hands this dock a large frame early in a
stream. The carrier is now bounded by a frame *count* (`carrier_presentations`) and paced at
`frame_period_ms` between frames, which restores the ramp without holding the endpoint.

Whether that is the cause is **not established** -- it is the best-supported candidate to test, not
a conclusion. The
capture that settles it needs vino's own wire: load with `debug=1 trace_crypto=1` so the per-head
video keys are disclosed, capture usbmon from before the replug, and run the record stream through
`tools/capture/record-stream.py`. ⚠ Each attempt costs a replug: the dock wedges off USB after the
failure (`can't set config #1, error -71`) and **no software path recovers it** -- an `authorized`
0/1 cycle was tried and does not.

### The next run diagnoses itself

⚠ `off=` in the submit-failure warning is where the error *surfaced*, not where the dock objected --
the queue is eight URBs deep and `send` reaps a completion when its slot is reused. So the warning
now also reports the frame's shape, read back off the records themselves by
`video::wht::record_stats`: record count, largest stride, largest strip. The same three numbers go
into the per-frame debug line on the success path.

That makes the next run answer the open question with no capture at all. Compare the largest strip
against what DLM reaches -- 1758 bytes on DL-6xxx, 1780 on the DL-7400, 2036 on DL-3x00 -- and the
largest stride against the 4080 cap. A stride over the cap is a malformed record and the dock is
right to halt; a strip well past 2036 with a legal stride says the frame is legal but larger than
anything the vendor's decoder has been asked to take.

## How to check it went out

⛔ Do not judge this from dmesg or from `enable=1 active=1`. Capture usbmon from the start and dump
the record stream:

```
tools/capture/record-stream.py CAP.pcapng KEYS.candidates.json --count 120
```

Expected, in order: two `ANNOUNCE marker=6` records inside the CP burst (one per head), a sealed
48-byte record at `seq=0` on each `0x08|head` followed by `ANNOUNCE marker=0` on the video sub, the
two set-modes, then per head an `aux=0x0008` 48-byte ring descriptor and a 336-byte sealed record at
`seq=1`, then strips. `--stats` should show **no** 32-byte trailer records on a video `sub`.

**Then ask whether the panels lit.** Pixels on the wire are not evidence.

## What the three generations now share

The record header is written in **one place** (`video::wht::record_header`) and used by every
record every dock sends -- image records, frame openers, trailers, plaintext markers. The captures
say the header is byte-identical across all three, and it was hand-rolled in six. The byte-exact
selftests are what make that refactor safe: they carry real captured bytes for all three docks, so
a moved byte fails the suite rather than a dock.

Also shared, where it used to be per-dock code:

- `video_arm::build_config` builds the decoder configuration for all three. The mode header and the
  quantiser table are identical; `layout_word` and `code_tables` are `DockProfile` fields.
- `cp::stream_announce` builds the plaintext record that names a stream or a video plane, for the
  DL7400 prologue and the DL-3x00 setup burst alike.
- `cp::stream_open` builds the six-byte sealed stream marker, with the one byte that differs
  between generations as `DockProfile::stream_marker_kind`.
- `DrmData::stream_mode_header` states the padded surface once, for the configuration and the
  per-frame report, on every dock.
- `dock_wide_init`, `arm_burst`, `sink_down_state`, `frame_period_ms` are profile data rather than
  `is_navarro()` branches.

⛔ Two things were deliberately **not** merged. `navarro_frame_opener` (32 bytes, ring addresses)
and `ella_frame_opener` (48 bytes, ring slots and a frame counter) are different wire formats, and
one builder spanning both would obscure each without removing anything real. The same goes for the
two trailers.

## The reference sequence -- build this before changing anything

⛔ The single biggest mistake of the session was spot-checking the capture instead of dumping it in
order. One systematic pass took minutes and produced both a retraction and the real lead.

⭐ **It is now a tool, so it does not have to be rebuilt again.** `tools/capture/record-stream.py`
parses the concatenated endpoint stream (records span USB transfers, so a per-transfer parse
silently overruns) and decrypts both the CP plane and each head's sealed video stream, finding the
per-stream key from `keys.candidates.json` by its Dl3Cmac tag.

```
tools/capture/record-stream.py captures/ella-video-evdi-20260810/wire.pcapng \
    captures/ella-video-evdi-20260810/keys.candidates.json --count 100
```

DLM's own sequence, which is what everything above was built against:

```
#1-#11   CP clear-text setup burst
#12-#25  CP sealed setup, through head 0's HDCP block
#26      ANNOUNCE sub=0x08 marker=6    <- head 0's stream, between 0x26 and 0x14/0x30
#27-#35  head 1's HDCP block
#36      ANNOUNCE sub=0x09 marker=6
#39-#42  0x15/0x20 + 0x15/0x21 per head, both heads before either engage
#44/#45  0x16/0x23 engage, off22 AND off23 = head
#48      sealed len 48 on 0x08, seq 0  <- head 0's stream open
#49      ANNOUNCE sub=0x00 marker=0    <- head 0's video plane
#53/#54  the same pair for head 1
#58/#59  0x15/0x53, off22 = 1 then 2
#86/#89  the two set-modes (0x48/0x22), each bracketed by 0x16/0x2f and 0x16/0x2e
#93      unsealed len 48, aux=0x0008   <- ring descriptor, video sub
#95      sealed len 336 on 0x08, seq 1 <- decoder configuration
#99+     strip records
```

⛔ **Frame delimiter, correctly**: a 48-byte record on a video sub whose body does *not* start with
`01 28`. On an image record `aux` is the **pad count**, so testing `aux == 0x000a` matches any
record with 10 bytes of padding. That error produced a retracted conclusion -- see below.

---

## What is solid (verified against captured bytes, or measured)

- **Wire framing is Ridge's, unchanged**: 92,072 records, 289,818,224 bytes, zero resync skips,
  zero non-16-byte strides, max stride 4080 = `STRIDE_CAP`.
- **One endpoint, two planes.** No video endpoint exists; EP02 carries both. `sub` at offset 8
  splits them: `0x00`/`0x01` video per head, `0x04` plaintext CP, `0x24` sealed CP OUT,
  `0x25`/`0x45` IN. ⛔ `ep 0x0a` on this dock is the **NIC**, not video.
- **Geometry 64x16**, from strip coordinates: x has 30 values step 64, y has 68 step 16.
  `strip_x` = `s[2..4]`, `strip_y` = `s[4..6]`.
- **Producer band order** (`ELLA_ROWS_1080P`) and **ordering by rank read from each strip's own
  coordinates**, which is what lets a partial frame arrive correctly. Both pinned in selftests.
- **Stream OPEN and frame opener** bytes, pinned in selftests. `aux` is a subtype on non-image
  records: `0x0008` open, `0x000a` opener. The opener carries a 3-slot ring (`cur`, `next`, `cur`).
- **Set-mode is unchanged from the existing field map**: 1920/1080 active, htotal 2200, vtotal
  1125, off42 `0x0400`, off44 = 60 refresh, off66 `0x2810`, off68 `0x0200`, off72 0.
- **Bandwidth**: DLM averages 0.86 MB/s and never starts a frame within 20 ms of the last. vino now
  measures 0.38 MB/s with the dock answering (1026 EP84 events), against 191 MB/s and *zero*
  replies before the gating.
- **`usb_clear_halt` on a stall.** Nothing cleared a halted endpoint, so one `EPIPE` killed the
  shared control plane too -- which is what "dock has answered nothing" actually was.
- **Mode limits**: DLM offers up to 1920x1080@60 but **75 Hz** at lower resolutions, so the pixel
  clock binds, not refresh.

---

## What is known-wrong and unfixed

1. **`sink_down_state`** is a profile field (DL-3x00 3, Ridge 1) and wired into `blank_head`, but
   the `0x2e`/`0x2f` sequence DLM brackets each set-mode with has not been reproduced. See "Next".
2. A head's EDID push carries **no head selector**, so discovery has to collect one head's answer
   before asking the next even though DLM issues both fetches back to back. If that push turns out
   to name its head, the whole discovery phase collapses into three loops with no waiting.

## Method: how this session went wrong

Three confident conclusions were retracted, all from acting on a spot-check instead of a reference.

1. **"vino is driving the displays."** Claimed on 246 clean frames and an active CRTC. Both panels
   were dark. ⛔ Pixels on the wire and `enable=1 active=1` are **not** evidence of a lit panel --
   this is written down elsewhere in this repo and was walked into anyway. **Ask.**
2. **"vino's frame is 10x too big."** Built on a frame delimiter that was matching pad counts. With
   the correct delimiter DLM's largest frame is **1,882,192 bytes** against vino's 1,882,048, and
   13 DLM frames are larger. Frame size was never the problem. A band-split was implemented on top
   of this before it was checked.
3. **Record 1's placement.** Bytes right, position wrong; it made the dock stop answering.

⭐ **What actually works: assert builders against captured bytes in a selftest.** Doing that for
`ella_frame_opener` caught a real bug in seconds (byte 37 is the *current* ring slot, not the
previous) that two hardware runs had missed. Each failed hardware run here can cost a replug or a
reboot. Do the offline check first, every time.

---

## ⛔ Verification traps

**A green build can mean nothing was built.** `make modules` re-runs `syncconfig` when the tree's
Kconfig differs from the one `.config` was generated against. With stdin at `/dev/null` it answers
*default* to every new symbol, silently dropping `CONFIG_DRM_VINO`. `make` then exits 0 having
compiled nothing. Assert the symbol, and assert a new symbol reached the `.ko`:
`strings vino.ko | grep -c <new_test_name>`.

**`make M=drivers/gpu/drm/vino` never rebuilds `rust/kernel`.** Fine as a fast syntax check; run a
full `make LLVM=1 -j16 modules` before believing any warning count. ⚠ It must be run from the tree
root -- from inside the driver directory it fails with "No rule to make target".

**A careless bulk edit can delete hundreds of lines.** A python slice-delete between two comment
anchors removed ~400 lines of `drm_sink.rs` mid-session. Recovered from `git show HEAD:`.
**Verify with a function-name set difference against HEAD**, not by eye:
`set(re.findall(r"\bfn (\w+)", head)) - set(same(current))` must be empty.

**dumpcap drops privileges.** Its output directory must be world-writable, or it fails with
"Permission denied" even under sudo.

---

## Firmware (deliberately parked until pixels work)

Implemented and cross-verified, **not tested on hardware**:

- ⭐ **The DFU payload is the entire `.spkg`, byte-identical from offset 0** -- 227 blocks
  (226 x 4096 + a 2768-byte tail) = 928,464 bytes, reassembled from `wire-bus4.pcapng` and matched
  against the file. **There is no container header to strip.** vino's `flash()` does
  `image.chunks(4096)` over the whole image, giving the same 227 blocks. Two independent
  derivations agree.
- **Downgrade is implemented**: `Upload::prepare` checks only the `ELLA` magic and the family, with
  no version gate on the manual sysfs path.
- ⭐ **An older image is in hand: 11.2.45** against the dock's 12.2.15.
  `/opt/displaylink/ella-dock-release.spkg.6.4.24.0.bak` and the 6.0.0-24 package's image are
  **byte-identical** -- no need to go back further. Read a version with
  `firmware::package_version`: tag `"VE"`, `u16` length 3, then three bytes, **searched for**
  because the offset moves between packages.
- **The upload node is now per-dock**: `vino-dock-<device>`. It was a fixed `vino-dock`, so a
  second dock's registration failed `-EEXIST` silently and the node said nothing about which dock
  it flashed -- a route to flashing the wrong one.
- ⚠ `ELLA` is the container magic on **every** DisplayLink package, not a family marker.
- ⚠ Twelve stray 2-byte `0x21` control writes are interleaved with the image blocks and must be
  excluded when reassembling. The payload field in tshark is **`usb.data_fragment`**.

---

## Next, in priority order

1. ⭐ **Re-test the ring off-by-one fix on an idle machine.** It makes vino's first opener match
   DLM's (`seq0 = 0`, slot 0) instead of contradicting its own ring descriptor, and it has never had
   a clean run. ⚠ Check `uptime` and `df -h /tmp` before believing any hardware result on this box.
2. **Move the prologue inside the bracket.** ⭐ This is the sharpest remaining lead, from the
   decrypted DLM sequence (records #86-#99):

   ```
   2f(0,1) 2e(0,3) [set-mode h1] 2f(0,1) 2e(0,0) 2f(1,1)
       <ring descriptor h0>  <decoder configuration h0>
   2e(1,3) 2f(0,1) 2e(0,0)
       <h0 strips>
   ```

   DLM's markers for head 0 are the same six vino sends at +5/+9/+12/+14/+20/+26 -- but the
   prologue lands **between the fifth and sixth**, so the last thing before pixels is
   `2e(head, 0)` *after* the configuration. vino sends all six markers, then the prologue and
   pixels together at `PROMPT_VIDEO_MS`. If the final sink-up latches the decoder configuration,
   vino brings the sink up while the decoder is still unconfigured -- which is a dock that accepts
   frames into a ring it never scans out, i.e. exactly the observed failure.

   ⛔ **Do not land this until the ring off-by-one above has had a clean run.** Two unvalidated
   changes in the activation path make the next result uninterpretable, which is how this dock has
   already cost several sessions.
2. **Close the last 19% of the strip grammar.** The main section is 100%; whole strips are 81%,
   and the residue is in the AC section and is *not* the category ceilings. `strip-diff.py` plus
   the corpus makes this a desk exercise -- no dock needed.
3. If the panels are still dark, the next unexplained records are the three `type=4 aux=0x0004`
   32-byte `0a 00 04 ...` records DLM sends mid-session (at capture indices 38357, 44733, 56708),
   each of which restarts the frame counter. They are Ridge's arm entries #6/#7 and vino does not
   send them. They are not a per-frame obligation.
4. **The `0x16/0x2e`/`0x2f` bracket still interleaves differently.** The pre-set-mode `0x2e` is
   now suppressed on this dock -- DLM sends none, and downing a sink that is about to be
   programmed left it down through the whole bracket. What remains is that DLM does not finish one
   head before starting the next: its order is `set-mode h0, 2f(0,1), 2e(0,3), set-mode h1,
   2f(0,1), 2e(0,0), 2f(1,1)`, ring descriptor h0, config h0, `2e(1,3)`, `2f(0,1)`, `2e(0,0)`,
   then h0's strips. vino runs the whole bracket per head.
5. Only then: test the firmware downgrade with the 11.2.45 image.
