# Handover

Single current handover. Last updated **2026-08-17**. Everything below is either still true or a
trap worth not repeating; anything an earlier handover said and this does not is done, superseded
or retracted. The DL7400/Navarro-era handover is in git history.

---

## START HERE -- the state on 2026-08-17

⛔ **The panels are dark and the dock still halts.** vino loads, both connectors configure,
selftests pass, then EP02 halts partway through a large frame (run 61: 35 MB, `-32` at 8.9 s).

### ⛔⛔ Every number the last handover led with was wrong -- the tool was misreading the capture

`tools/capture/dlm-map.py` had its own usbmon reader, and it took the transfer payload from
`USBMON_HDR.size` (40) instead of the 64-byte offset a mmapped capture actually uses. That splices
**24 zero bytes into the record stream at every transfer boundary**. The parser resynchronises by
scanning, so the opening of a capture reads fine and the numbers look plausible -- but every record
that spans a transfer is corrupted, and a busy stream desynchronises into invented records with
50,000-byte lengths. Fixed; `usbmon_read.iter_transfers` was always right and every tool built on
it (`record-stream`, `sequence-diff`, `ring-openers`, `stall-point`, `choreography-diff`,
`ella_decode`) was unaffected.

Re-measured with the reader fixed:

| | old (wrong) | measured |
|---|---|---|
| vendor head 0 | 3233 frames, 10.5 fps, median 18,048 B / 64 strips | **6473 frames, 20.0 fps, median 16,416 B / 64 strips** |
| vendor head 1 | 116 frames, 0.4 fps | **859 frames, 2.9 fps, median 18,464 B / 25 strips** |
| vendor peak second | "never above 13.8 MB" | **34.4 MB** |
| vendor peak 100 ms | "never above 6.14 MB" | **11.9 MB (119 MB/s)** |
| run 61 peak second | "9.95 / 13.75 / 11.35 MB" (calendar buckets) | **23.35 MB sliding** |
| run 61 content frames | "died before producing one" | **25 of them, every one 2040/2040 strips** |

⭐ The one thing the old numbers got right is the shape of the gap: **DLM's median frame is 64
strips of 2040, about 3% of the surface. vino sent 2040 of 2040 on every frame in run 61 and on
every frame in run 52.** A full surface is the vendor's maximum, not its habit.

### ⛔ Rate is dead, and this time with the right numbers

`tools/capture/envelope.py` reports peak bytes in a *sliding* window at eight timescales -- calendar
buckets are what hid this. vino is inside the vendor at every scale:

```
                0.05s   0.10s   0.25s   0.50s   1.00s   2.00s   3.00s   5.00s
vendor  MB       7.91   11.86   23.73   33.27   34.44   36.24   38.48   42.80
run 61  MB       8.27   11.64   11.87   15.59   23.35   35.04   35.04   35.05
```

The vendor bursts a frame at 158 MB/s and then goes quiet; its curve is a ~32 MB bucket refilling
at ~2.2 MB/s, which is what `PROFILE_ELLA.stream_pacing` now states. The previous 16 MB/s figure
permitted 80 MB in five seconds where the vendor's own worst five seconds is 42.8 MB, and its
stated justification ("no second above 13.8 MB", "17.5 MB inside 0.81 s halts it") came from the
broken reader. ⛔ Do not re-open rate: the vendor does everything vino did and more.

### ⛔ Four more mechanisms measured dead on run 61

1. **A malformed bitstream.** All **49,591** strips vino put on the wire decode cleanly under
   `tools/codec/ella_decode.py` -- 37,341 with AC rows, 12,250 flat, zero failures.
2. **Framing.** Every frame is N x 65,536 plus one short transfer; no record exceeds 4084 bytes;
   no record is split.
3. **The ring.** `ring-openers.py`: `(0,1,1) (1,2,2) (2,3,0) ...` on both heads, the vendor's walk.
4. **Frame shape.** A vendor full surface is 530 records / 1,882,144 B; vino's is 538 records /
   1,920,272 B. When both send everything, they send the same thing.

### ⭐ What is left: vino never sends a delta

Neither capture in the corpus exercises damage selection, which is why this has never been settled:

* **run 52** (the run that lit a panel) sent **12 frames, all 1,442,960 B, all 2040 strips** -- three
  keyframe presentations per head per stream open, and then nothing. Its desktop never changed, so
  the delta path never ran. It survived because it only ever sent 13.4 MB.
* **run 61** sent 25 frames, all 2040 strips, and consecutive frames share **0.0%** of their strip
  bytes -- the content genuinely moved everywhere. That is a fresh output's start-up animation, not
  a selection bug, and it also cannot settle the question.

So the next run has to catch a **settled** desktop. `scanout.rs` now logs one line per frame:

```
vino: scanout head=0 delta 336/2040 strips from 3 rect(s), 210 moved
```

`moved` is strips whose pixels changed; the selected count is what goes on the wire. If they are
close, selection is working and the desktop really is busy. If `moved` is small and the selection is
the surface, the widening is the retransmit debt (`damage_frames`) or the macro-tile rounding.

Two amplifiers were fixed on inspection before that run, because both are wrong regardless:

* `changed_strip_rects` built its rectangle list on the **strip** grid and gave up on the whole
  surface past 128 rectangles. Damage is transmitted at macro-tile granularity anyway, so the list
  is now built there: scattered damage collapses about sixteen-fold and stays inside the ceiling.
* The keepalive queried status every **250 ms** on a dock where the vendor queries every **2.5 s**
  and sends nothing at all during its long silences. Now `DockProfile::status_period_ms`; the
  dedicated-pipe docks keep 250 ms, where a query contends with nothing.

### ⭐ Tooling that now exists (use it before inferring)

* `tools/capture/dlm-map.py` -- whole session, both directions, timings, cadence.
* `tools/capture/choreography-diff.py` -- every ordering divergence, not just the first.
* rr replay **works**: record **without** the `rr-norrguard` LD_PRELOAD shim, which was itself the
  divergence (rr git master handles `MADV_GUARD_INSTALL`). Verified Ella trace in
  `captures/rr-ella2` (56946 events). `scripts/rr-find-tail-writer.sh`.
* `tools/capture/hook-cp-prewire.js` -- a DL3 message before it is sealed.
  ⛔⛔ **Never hook `0x1cf436` or `0x269dd0`** -- they segfault DLM, not merely stall it.
* ⛔⛔ DLM is **idle in steady state**: a hook that sees nothing has proved nothing unless control
  records went out in the same window. Force a session with USB `authorized` 0->1.

### ⭐ The opaque message tail, closed

It is a **u32 connector selector at off22, a flag byte at off26, an HDCP message id at off27 and its
payload at off28** -- read out of the vendor's own assembler at `0x586af0`, payloads confirmed live
(`02`+rtx, `04`+Ekpub(128), `09`+rn, `0b`+edkey+riv, `0f`+V, `10`+Stream_Manage). Bytes past the
message end are untouched allocation (`operator new(sz=36)`). vino already matches; the one defect
found was introduced and reverted the same day. ⛔ Do not re-open -- it has been re-derived at least
four times.

---

## DLM's choreography on a DL-3x00, as measured

Derived from `captures/ella-video-evdi-20260810/wire.pcapng` (290 MB, 92,072 records, both planes
decrypted). This is the specification vino is being held to; each line is a measurement, not a
reading of the code.

**Bring-up.** 45 records, fixed order, identical to vino's for the first 42. Both connectors are
engaged and mode-set before any pixels. Timeline: first record at t+0, plaintext burst to t+9 ms,
sealed setup from t+382 ms, per-head blocks at t+775 ms and t+1161 ms, set-modes at t+2361 ms,
decoder configuration at t+2428 ms, first pixels at t+2438 ms. Status polls run at a steady 16 ms
while it waits.

**Frames.**

- A frame is **closed** by a 48-byte ring-slot record (`aux=0x000a`) naming the slot it filled, a
  one-based frame counter, and the slot the next frame will fill. It is the **last record of the
  frame's final USB transfer**: 772 of the 774 short-terminated transfers that carry pixels end on
  one, and only 2 begin one.
- Transfers are **65,536 bytes**, and a frame's last one is short -- so the short packet is the
  frame delimiter and the closing record is the last thing before it.
- The prologue frame is closed the same way. It is the flat surface as 2040 strips of 54 bytes,
  114,720 bytes exactly, and vino's is byte-identical.
- **Cadence 16.5 ms median** per head (p10 15.6, floor 1.3 ms), 20.0 fps on head 0 and 2.9 on
  head 1. Peak **118.7 MB/s in a 100 ms window** and 34.4 MB in the worst second; mean 0.89 MB/s
  over the whole capture. **Idle sends nothing.**
- Frame sizes follow content: **median 16 kB**, 361 kB where a head is busy, 1,882,144 bytes
  largest. The vendor's first head goes flat frame -> 361 kB content immediately.
- ⭐ The two heads' records **interleave inside a single 64 kB transfer** when both are busy, at
  record granularity. vino serialises a whole frame per head under the queue mutex. Neither is
  known to matter, but they are not the same transport model.
- **No record ever exceeds 4084 bytes**, and a record is **never split**: control records sit
  between records, never inside one.

**Control plane while streaming.**

- The dock answers **nothing** for stretches of up to **79 s**, and the vendor sends nothing either
  -- ~225 control records in 326 s, none at all during the long silences.
- **312 EP84 IN messages for 7115 frames**: there is no per-frame acknowledgement, and one IN
  transfer is kept posted, not four.
- The sealed stream report (`aux=0x0006`, 60 bytes) appears **14 times in 7115 frames**, never two
  together, the first with the **third** frame of a fresh stream, riding at the head of that
  frame's first transfer.

**Set-mode.** `0x48/0x22`, 80 bytes. 1920/1080 active, htotal 2200, vtotal 1125, off42 `0x0400`,
off44 = 60, off46 = **`0x0800`** (stride), off48 = **`0x2000`** (rows), off66 `0x2810`, off68
`0x0200`, off72 0. ⭐ Offsets 46 and 48 are the dock's own framebuffer allocation and are **not**
Ridge's device override -- sending Ridge's `0x4000`/`0x6000` is accepted, and the dock then has
nowhere to put the frame after the first. Pinned by `ella_set_mode_matches_the_dlm_capture`.

### The tools, and what each one answers

All four run offline against a capture and need no dock. Point them at the vendor reference
`captures/ella-video-evdi-20260810/wire.pcapng` and they are the fastest way to turn "the dock
stopped" into a named record.

| tool | question |
|---|---|
| `tools/capture/sequence-diff.py` | where does our bring-up first differ from the vendor's? (record shape, no keys) |
| `tools/capture/ring-openers.py` | do the two ring walks start at the same slot? (exit code says) |
| `tools/capture/stall-point.py` | which transfer did the dock refuse, and what record is at its offset? |
| `tools/codec/ella_decode.py` | does every captured strip decode? anything under 100% is a grammar hole |
| `tools/render-dc.py` | the pixel oracle -- the only check that can see an escape payload's bit order |
| `tools/capture/record-stream.py` | the decrypted reference sequence, with keys |
| `tools/capture/choreography.py` | what a sender does and when: frames and their riders (`--frames`), where its USB transfer boundaries fall relative to frame boundaries (`--transfers`), the record stream with images collapsed (`--records`) |
| `tools/capture/dlm-map.py` | the whole session as one timeline: both planes, timings, frames, per-head cadence |
| `tools/capture/envelope.py` | peak bytes in a sliding window at eight timescales -- the only honest way to compare two senders' rates |

### Confirmed on hardware, do not re-chase

| | |
|---|---|
| ring counter starts at slot 0 | `walks agree at the first opener` |
| both heads' setup bursts complete | 43 records agree with the vendor; 33 setup messages |
| the endpoint stall | **gone** -- no `-32`, no `EPIPE`, no halt-clear |
| stream prologue inside the bracket | first frame is 114,720 B, the vendor's exactly |
| bracket close before pixels | `failed mode set` **gone**; a 1.5 MB keyframe is reached |
| strip grammar | 100% of captured strips decode (offline, three oracles) |

### Untested

- `CARRIER_RAMP_FRAMES = 5` -- the experiment above.
- `KMS_RETRY_LIMIT` -- bounds the deferral storm; its own deadlock is fixed but the bound itself has
  not been exercised.
- ⚠ **The rrx wait touches Ridge.** `wait_perhead_push(AKE_SEND_RRX)` replaced a fixed drain on the
  generic path, which is Ridge's as well as this dock's, and **the D6000 is not attached to test
  it**. `wait_perhead_push` does not take `edid_out`, so a reply `drain_ep84` would have routed to
  EDID collection is dropped instead. No EDID is expected at that point in the burst, but this is
  the one change in the tree that could regress another dock. Check it before the D6000 next runs.

---

## Immediate physical state

- The **HP 3005pr (Ella, DL-3900, `17e9:430a`)** is the only dock attached, on bus 4. `bcdDevice`
  **3157**, firmware **12.2.15**. It re-enumerates on a different device number constantly --
  resolve it by `idProduct`, never a path.
- The **DL7400 is not attached.** Everything in this document is Ella.
- ⛔ **Video does not work yet.** vino loads, both connectors configure and selftests pass, then
  EP02 halts partway through the first large frame (run 61: 35 MB, `-32` at 8.9 s). Nobody has seen
  a picture on this dock from vino.
- **The dock has two wedges.** Soft: still enumerated, display function unresponsive -- a
  device-scoped `USBDEVFS_RESET` (`ioctl(fd, ord('U')<<8|20)` on `/dev/bus/usb/BBB/DDD`, resolved by
  `idProduct`) fixes it in ~5 s, followed by a vino unbind/bind. Hard: refuses `SET_ADDRESS` at all
  (`device not accepting address N, error -71`), and **nothing in software recovers it** -- device
  reset, hub reset, the kernel's own port power cycle and `port/disable` were all tried. Someone has
  to unplug it.
  ⛔ **Check for D-state tasks before believing the hard wedge** -- a deadlock inside vino wedges
  `usb_hub_wq` and produces byte-for-byte the same symptom. `ps -eo pid,stat,comm | awk '$2 ~ /D/'`.
  ⛔ **Never reset the xHCI controller `0000:08:00.4` or a hub.** The user declined that and is
  often remote. Bus 4 carries only the dock (wlan0 is PCI at `0000:05:00.0`; storage and Bluetooth
  are bus 3), so a *device*-scoped reset there is safe and a controller reset is not.
- ⭐ Unbinding vino promptly when a run window closes keeps the dock alive; leaving it bound after a
  failure is what ends in a wedge.
- **KWin drives card2** since Mesa was rebuilt with `llvmpipe`; see "Nothing had a compositor
  attached". ⚠ A running KWin keeps the old Mesa mapped, so that fix needed one relogin.
- `tools/light-head.c` drives a head with no compositor at all; it paints a pattern whose bars land
  on strip boundaries, so a mis-placed strip shows as a bar of the wrong width rather than a smear.
- Captures worth keeping: `~/vinocap/run23.pcapng` (the mode set succeeding and a real keyframe
  reached), `~/vinocap/run17.pcapng` (the ring and setup-burst fixes confirmed), and the vendor
  reference `captures/ella-video-evdi-20260810/wire.pcapng`.

## Next, in priority order

1. ⭐ **One run on a settled desktop, captured.** Let the output come up, wait for the start-up
   animation to finish, then leave the desktop alone and move one window. Read the per-frame
   `N/2040 strips from R rect(s), M moved` lines: that single line says whether damage selection
   works, and no capture in the corpus answers it.
2. **Map that capture** with `tools/capture/dlm-map.py` and profile it with
   `tools/capture/envelope.py`, and put both blocks beside the vendor's in START HERE.
3. **Close whatever the gap turns out to be.** If `moved` is already the whole surface on a settled
   desktop, the snapshot or the hash is at fault, not the selector. If `moved` is small and the
   selection is not, `damage_frames` (3 on this dock) and the macro-tile rounding are the two
   multipliers, in that order.
4. **Explain the vendor's own head asymmetry** -- 6473 frames to head 0 against 859 to head 1 over
   the same 300 s. vino drives both heads symmetrically, and each of its full surfaces costs what
   the vendor's largest frame costs.
5. Only then: the firmware downgrade with the 11.2.45 image.

⛔ Closed, do not re-open: the ring off-by-one, the prologue inside the bracket, the strip grammar
(100%), the `0x16/0x2e`/`0x2f` interleave, and the opaque message tail (see START HERE).
## ★★★★★ 2026-08-12 late: THE MODE SET SUCCEEDS AND A REAL KEYFRAME IS REACHED

`~/vinocap/run23.pcapng`, with the post-close markers removed:

```
encrypted control setup complete (33 messages)
socket 2 monitor connected
link ready after 81 status polls
head=1 stream prologue sent inside the bracket (384 B)
head 1 startup frame submitted after 0 ms (114720 bytes)
head=1 chunks=105 first=1524320 presentations=1 records=432
```

⭐ **`socket 2 left open after a failed mode set` is gone.** The session survives the carrier, and
for the first time the driver builds and submits a real 1.5 MB KWin desktop keyframe on this dock.
The failure moved again: the dock consumes the **whole** carrier and then never completes the
transfer that carries the next frame's opener (`stall-point.py`: earliest outstanding transfer at
offset 124,608, which is that opener). The ring walk is `(0,1,1) (1,2,2)` -- the vendor's.

### ⭐⭐ Why it stops there: the vendor ramps, and vino did not

Frame sizes measured between consecutive openers on a fresh vendor stream:

```
head 1:  114768  114768  114832  114768  837840  115024  1882128 ...
head 0:  361328  361568  361376  361376  361424      48   361472 ...
```

⭐ **The head that goes on to send the whole surface in detail is fed five flat frames first** --
the prologue carrier and four more. vino sent one and then a megabyte and a half.

⛔ **This corrects "a stream opens with exactly one flat frame and then goes to content"**, which
was read off head 0: its first content frame is 361 kB, a third of the size, and it really does go
straight to it. `carrier_presentations` is now `CARRIER_RAMP_FRAMES = 5`. ⚠ HW-untested -- the dock
wedged before it could run.

### ⭐ The retry storm, which was also destroying the evidence

A KMS command that fails because the dock stopped answering was re-queued every 50 ms forever. On a
dead session that is a reprogramming attempt twenty times a second: it filled the kernel ring buffer
twice before the failure that caused it could be read, and it keeps writing to a dock that has
already stopped listening -- a plausible route from the recoverable soft wedge to the hard one.
Bounded by `KMS_RETRY_LIMIT`, reset whenever a batch gets through.

## ★★★★★ 2026-08-12 THE STRIP GRAMMAR IS CLOSED -- 77% -> 100%

**A DL-3x00 decoder takes the payload bit that follows each unary one as the LEAST significant.**
The driver emitted it most-significant-first, which is the same number of bits, so every
length-based check passed and every strip decoded to noise.

Settled three ways against 8000 captured strips, offline, no dock:

| check | most significant first | least significant first |
|---|---|---|
| whole strips decoding cleanly | 6153/8000 = **77%** | 8000/8000 = **100%** |
| recovered luma DC range | **-3205..996** (impossible; luma DC cannot be negative) | **0..1020** (exactly the codec's bound) |
| frame rebuilt from DC | streaks | **a legible desktop** |

⭐ **The landing oracle alone could not have found this.** It sees the significance fields (whose
value sets how many coefficients follow) but is blind to an escape's payload, where reversing the
bits changes values and not lengths. What settles the escape is pixels:
`tools/render-dc.py` renders a frame from DC coefficients alone, and
`captures/ella-video-evdi-20260810/decoded/frame-dc.png` is the result -- a settings window with a
sidebar, list rows and buttons.

⛔ **The flat strip that pins this dock's encoding byte for byte is blind to it**: every field of a
flat strip carries an all-zero payload, so both orders produce the identical 54 bytes. That is why
`DLM_FLAT_STRIP` never caught it, and why a gradient strip is now pinned alongside.

⛔ **Retract "the residue is a position-dependent ceiling."** The ceilings were never involved. So
is "whole strips are 81%": with the order corrected there is no residue at all.

Ridge and the DL7400 group their payload after the terminator and are untouched; their byte-exact
tests still pass. Tools: `tools/codec/ella_decode.py` (the grammar as a decoder plus a whole-corpus
check -- anything short of 100% is a hole), `tools/render-dc.py` (the pixel oracle).

## 2026-08-12: the ring, the setup burst, and the compositor

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

✅ **Confirmed on hardware**: 43 records now agree with the vendor and setup runs 33 messages
instead of 27. ⚠ Only the second was certain in advance. The first is well motivated -- it makes the two platforms
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

## The bracket, decrypted end to end

`record-stream.py` on the vendor capture settles the mode-set sequence exactly, and it matches the
placement derived from source:

```
#86  0x48/0x22 set-mode   off22=00      head 0's mode
#87  0x16/0x2f off22=00 off23=01        2f(h0, 1)
#88  0x16/0x2e off22=00 off23=03        2e(h0, 3)   sink down
#89  0x48/0x22 set-mode   off22=01      head 1's mode
#90  0x16/0x2f off22=00 off23=01        2f(h0, 1)
#91  0x16/0x2e off22=00 off23=00        2e(h0, 0)   sink up
#92  0x16/0x2f off22=01 off23=01        2f(h1, 1)
#93  ring descriptor h0
#95  decoder configuration h0
#96  0x16/0x2e off22=01 off23=03        2e(h1, 3)
#97  0x16/0x2f off22=00 off23=01        2f(h0, 1)
#98  0x16/0x2e off22=00 off23=00        2e(h0, 0)   the last record before pixels
#99+ strips ... then the next frame's opener, with NOTHING in between
```

⭐ **The bracket is complete before any pixels**, so the closing pair vino sent after prompt video
put two records into the one gap the vendor leaves empty -- and one of them is `2f(head, 0)`, a
marker state the vendor never uses on this generation at all: every `2f` in its bring-up carries
state 1. `modeset_bracket_post_close` now returns early on a dock without a video pipe. ⚠ HW-untested.

## ⛔⛔ A held guard is not a re-entrant lock -- and it takes USB hotplug with it

Introduced and fixed in the same session, worth writing down because the symptom points at the
dock and not at the driver. `kms_worker` already holds `pending_kms` when it decides to drop the
batch:

```rust
let mut pending = data.pending_kms.lock();
...
data.pending_kms.lock().clear();   // the guard above is still alive
```

The kernel names it exactly -- `task kworker/4:0 is blocked on a mutex likely owned by task
kworker/4:0` -- but only after `hung_task_timeout_secs`, two minutes later. Before that the machine
just looks like it has a dying dock:

```
kworker/4:0+events     D   <- the KMS worker, holding vino's state
kworker/5:2+usb_hub_wq D   <- everything USB behind it
sh                     D   <- an unbind that will never return
```

⚠ **`usb_hub_wq` in D state means every enumeration on the machine fails**, so the dock reads as
`device not accepting address N, error -71 / unable to enumerate USB device` -- indistinguishable
from the dock's own hard wedge. Two physical power cycles were spent on this before the hung-task
report appeared. **Check `ps -eo pid,stat,comm | awk '$2 ~ /D/'` before concluding the dock is
dead**, and `cat /proc/<pid>/stack` to name the offender. Nothing but a reboot clears it.

## ⛔ A replug is spent before you can use it -- hold vino off first

**vino autoloads by modalias**, so the moment the dock is replugged it binds, runs a bring-up, fails
and wedges the dock -- before a capture is running and before the newly built module is the one
being tested. Two power cycles were burnt that way. Hold it off, replug, then bind deliberately:

```
sudo tee /etc/modprobe.d/zz-vino-manual.conf <<< 'blacklist vino'   # remove when done
   ... physical replug ...
sudo dumpcap -i usbmon4 -w ~/vinocap/runN.pcapng -q -B 256 &
sleep 12                       # dumpcap probes every device's descriptors on start
sudo modprobe vino debug=1
```

⚠ **Wait for dumpcap before loading.** It issues `GET_DESCRIPTOR` to every device on the bus when
it starts, and those requests time out against this dock
(`usbfs: USBDEVFS_CONTROL failed cmd dumpcap ... ret -110`); a bind racing them reads
`no identity descriptor and no quirk entry; declining`.

⚠ Give the run at least 55 s of capture: a slow bring-up put the first frame 0.3 s past the end of
a 38 s window once, which cost the whole capture.

---

---

# Reference

## The strip grammar, and how it was derived

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
   Whole strips including both AC rows: 81% -- which is where this stopped until the payload
   order was corrected, taking it to 100%. See "THE STRIP GRAMMAR IS CLOSED" above.

⚠ **The oracle has to tolerate padding.** An earlier pass scored "ends exactly at `w18`" and got
~6% for every hypothesis including the right one, because the main section is padded by two or
three bytes. That wasted a search; the fix is to require zero bits after the decode, not an exact
landing.

## Shared, not duplicated

The dialect is `DockProfile::code_tables`, the field that already chose which tables the
configuration record states -- so the code vino emits and the code it declares cannot drift apart
again. `Bits::unary` is the single primitive; `esc`, `chroma_base` and `sync_unit_after` all route
through it, replacing four hand-rolled bit loops. Ridge and the DL7400 keep `Wide` and their bytes
are unchanged, which their existing byte-exact tests pin.

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
- **`usb_clear_halt` on a stall.** Nothing cleared a halted endpoint, so one `EPIPE` killed the
  shared control plane too -- which is what "dock has answered nothing" actually was.
- **Mode limits**: DLM offers up to 1920x1080@60 but **75 Hz** at lower resolutions, so the pixel
  clock binds, not refresh.

---

## ⛔ Do not re-chase -- refuted or closed by measurement

Each of these cost at least a session. The evidence is in git history if the reasoning is ever
needed again; what matters here is that they are settled.

**About the transport**

- **Rate, pacing and flooding.** Over the vendor's whole 326 s capture: mean 0.888 MB/s but a peak
  of **82.9 MB/s** in a 100 ms window, minimum inter-submit gap **14 us**, and up to **3,982
  consecutive OUT transfers with no reply at all**. It never once stalls its endpoint in 290 MB.
  ⛔ "Never less than 20 ms between frames" was a mistaken average -- the real floor is 0.2 ms.
- **A per-frame acknowledgment.** 7113 frame openers against **312 EP84 IN messages** in total.
  There is no per-frame signal and the vendor never waits for one.
- **Queue depth.** Peak in flight on EP02: vendor 8, vino 9. A *shallower* queue (2) made it
  strictly worse.
- **A dock receive budget.** The vendor sends uninterrupted image runs up to **1,882,144 bytes**,
  453 of them over 128 kB. ⛔ The failing `off=` in a submit warning is where the error *surfaced*,
  eight transfers late -- never where the dock objected. The recurring 128 KiB was a coincidence.
- **Frame size.** The vendor's largest frame is 1,882,192 B against vino's 1,882,048.

**About the records**

- **The image record `aux` field.** Uninitialised sender-side garbage: `(sub,x,y)` does not
  determine it, the repeat-gap histogram is uniform random, and the vendor's own first frame is all
  zeroes. vino's `0` is correct.
- **Keystream replay in the frame opener.** The openers are plaintext and the vendor sends `seq=0`
  on all 3711 of them.
- **The sealed decoder configuration.** Decrypted and compared: exactly what `video_arm::build_config`
  produces. Nothing to find there.
- **Ring rotation.** The openers walk `cur/next` 0->1, 1->2, 2->0 byte-identically to the vendor's.

**About the codec**

- **The AC category ceilings.** Sweeping them 6..11 is flat. ⛔ So is "the residue is a
  position-dependent ceiling" and "whole strips are 81%" -- with the payload order corrected there
  is no residue at all.
- **A landing oracle that demands an exact landing.** The regions are byte-padded, so requiring an
  exact stop scores ~6% for every hypothesis including the right one. Require zero bits after the
  decode, not an exact landing.
- **Plane order and per-block grouping** are invisible to a landing oracle -- only the total
  coefficient count moves the landing point. Use the pixel oracle.

## A latent framing bug, found with no dock attached

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

**A record-walk "desync" can be the checker's own stride cap.** Image records are capped at 4080
bytes, control records are not: the vendor's own stream carries none over 4084, but vino's cursor
upload is a single 16,448-byte record. A validator that caps every record at the image limit reports
a desync at the first big control record. Cap by plane, and confirm a desync by finding where the
next valid header actually is.

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
