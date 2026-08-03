# Handover — 2026-08-03 (second session)

Supersedes `handover-2026-08-03.md`. The DL7400 still shows no picture, but the failure is now
localised much more tightly, and several of the previous session's leading candidates are dead by
measurement rather than by argument.

All work is one commit on `vino/linux` branch `vino`:
`53bb9075fac3 drm/vino: give the DL7400 its own keystream, cadence and bring-up`
(warning-free). It wants splitting on the next rebase — this environment has no `git rebase -i`
and no `git add -p`, and the changes interleave inside `drm_sink.rs`.

## The failure, stated precisely

With every fix below in place, on a cold `modprobe vino`:

* the full Navarro bring-up choreography runs, **head 1 first then head 0**, as DLM does;
* each head's first transfer is submitted as four URBs (65536 ×3 + 7600);
* **the first 65536-byte URB completes with status 0 on each endpoint, and nothing after it ever
  completes**;
* about 1.05 s later every outstanding URB completes `-ESHUTDOWN` and the dock stops answering the
  **control plane too**, then re-enumerates.

The dock takes exactly one URB's worth into its FIFO and never drains again. That is what an
unarmed pipe looks like: the bytes are accepted by the USB controller and nothing consumes them.

## ⛔ Killed by measurement this session — do not re-chase

### 1. The dock is not authenticating vino's sealed prologue at all

The previous handover listed this as candidate 2, untested. It is now tested, with the experiment
it asked for. `break_mac=1` XORs `0xff` into the last byte of the sealed pipe descriptor's
Dl3Cmac, so the record cannot possibly authenticate.

**The behaviour is identical**: one 65536-byte URB accepted, the rest jam, `-ESHUTDOWN` at
+1.04 s against +1.07 s with a valid tag.

⇒ Whatever the dock is doing with that record, it is not checking its MAC and acting on the
result. **Every remaining theory that turns on the sealed video content — the key, the nonce, the
plaintext, `compute_eks`, `V` — cannot explain this failure**, because the dock behaves the same
when the record is provably corrupt.

### 2. The per-head video key, both ways

`video_key_raw=1` seals with the raw per-head SKE key instead of `cp_session_key()`'s whitened
one. Identical failure. Combined with (1), the per-head keying is not what is stopping this.

### 3. The pipe descriptor's plaintext

vino's `navarro_pipe_descriptor()` was reconstructed in Python from the constants in `cp.rs` and
compared against DLM's *decrypted* descriptor from
`captures/navarro-dlm-modeset-20260802-005453`. **All 290 deterministic bytes agree**, for both
connectors, across two separate prologues. (The last 14 bytes are host-random and are not
compared.) 27 of 27 sealed video records in that capture authenticate under vino's own
`seal_livemac`/`dl3cmac_tag` algorithm, so the primitives are right too.

### 4. Prologue record layout

vino's prologue and DLM's now agree record for record on offsets, sizes, `type`, `sub`, `aux` and
`seq`:

```
off    size  type sub     aux     seq
0      28    2    stream  0       0     stream marker
32     28    2    other   0       0     the OTHER connector on this endpoint, not stream|0x10
64     332   4    stream  0x000e  0     pipe descriptor      (304 B plaintext)
400    28    2    frame   0       0     frame marker
432    28    4    frame   0x0004  0     ring record
464    1132  4    stream  0x000e  19    decoder configuration (1104 B plaintext)
1600   4044  4    frame   0x0000  0     first image record
```

The two leading `type=2` markers are **the two connectors that share the endpoint** (`0x07`/`0x17`
on ep `0x08`, `0x0f`/`0x1f` on ep `0x0a`), which happens to equal `stream | 0x10` for connectors 0
and 1 only. The previous handover's open question about `0x17`/`0x1f` is settled: in both Linux
DLM captures those subs carry the 16-byte `aux=0x0002` open exactly once, on the connectors with
**no monitor**, while pixels went to connectors 0 and 1.

### 5. Image records and strips

vino's first image record body and DLM's agree over the bytes compared: 54-byte strips on a 56-byte
stride, `[u16 len][01 28][x u16][y u16][00 00 00 00]`, x stepping 128 across a 2560-wide row.

### 6. `aux` on an image record is not padding

Every image record in the DLM capture has a 16-aligned stride, so `aux` cannot be a pad count. Its
values are the even numbers 0..0xe, uncorrelated with size, and **`aux` is 0 for every record of
the first frame** — so it cannot be what the dock objects to on the opening frame. Its meaning is
still unknown and still matters for damage updates later.

## ⭐ New protocol facts

### `seq` is a per-stream AES-CTR block counter, and it chains

This is the one that produced a real bug. `seal_livemac` builds the IV as `riv ‖ 0000 ‖ BE32(seq +
i)` and passes `seq` to the Dl3Cmac, so the wire `seq` **is** the block index. Across a whole DLM
session it advances by exactly `ceil(plaintext_len / 16)` for every sealed record on a stream and
never rewinds — `0 → 19 → 88 → 94 → 101 → 108 …`, continuing straight through a re-arm that sends
a second pipe descriptor at 133 rather than at 0.

vino sealed a standalone open at block 0 and then the pipe descriptor at block 0 as well, reusing
its first keystream block. Fixed: the counter lives in `video_seal_seq[head]` and is reserved from,
and is reset only when new video keys arrive.

Windows does the same thing a different way and arrives at the same place: it sends the 16-byte
open at seq 0 and a **288**-byte descriptor at seq 1, so its chain is `0 → 1 → 19 → 88` where
Linux DLM's is `0 → 19 → 88`.

### The per-frame sealed stream report

DLM pairs one sealed record on the *stream* sub with every frame on the frame sub. Two forms:

| aux | plaintext | content |
|---|---|---|
| `0x000c` | 96 B | 84-byte report body + 12 host-random |
| `0x0002` | 112 B | 26-byte mode header + the same body + 2 host-random |

`0x000c` is the overwhelming majority (159 of 164 and 304 of 306); `0x0002` restates the mode and
appears a handful of times, around a mode change. The 26-byte mode header is byte-for-byte the one
that opens the decoder configuration, so `video_arm::mode_header()` now builds both.

The body is `[len=0x0052][kind=0x000a]` and 40 more `u16`: a fixed `1, 1, 0, 64, 64` preamble, a
scalar, then three blocks of three `(a, a, b)` triples separated by `(1, 1, 1)`, then a zero. On a
quiescent stream the values are identical on both connectors in both captures; under load `a` and
`b` grow with the frame's cost. **The mapping from a frame to those numbers is not established** —
vino sends the quiescent set.

### The dock tears the link down over a silent video endpoint

Measured twice with very different transfer shapes (a 204 KB frame and a single 4 KB image
record): every outstanding URB completes `-ESHUTDOWN` 1.06 s and 1.10 s after the last video byte,
and the dock goes deaf on EP84 at the same instant. `SETTLE_REPAINT_MS` was 1200 ms and one-shot,
i.e. deliberately outside that window.

⚠ The keep-alive added for this **cannot currently be observed to help**, because by the time it is
due the video queue is already jammed with eight uncompleted URBs and `queue.send()` cannot get
through. It is correct and it is what DLM does, but it is untested in anger.

### DLM's startup cadence is nothing like vino's

Per 100 ms bucket over the first two seconds of DLM's first video:

```
  0-100 ms   627,344 B   6.3 MB/s
100-200 ms   627,360 B   6.3 MB/s
200-300 ms   625,920 B   6.3 MB/s
...
1000-1100 ms 4,868,192 B  48.7 MB/s
```

DLM sends one ~210 KB frame, waits 6.8 ms, another, waits 32.8 ms, another — and does not touch
the *other* endpoint until +129 ms. It ramps to full rate only at about t=1 s. vino fires ~470 KB
in 7 ms from a standing start with eight URBs queued at once. This has not been addressed and is
the most obvious remaining behavioural difference.

### The dual-head activation was never running

`activate_dual_wake` holds the entire cold choreography — markers, status polls, mode sets, video
ordering. It was not running at all. A dual-head atomic commit calls `atomic_enable` once per head;
each call queues its own `ModeSet` and wakes the KMS worker, and the worker was reliably scheduled
*between* the two:

```
vino: KMS batch -- stream cmds 1, dual timings 1, dual_wake false, requested [7205821271455012 0 0 0]
```

So vino took the single-head path, skipped the dock-wide activation, and **never drove head 1 at
all**. Fixed by waiting, bounded at 20 ms, for the mode sets of the other heads that have a
monitor. After the fix:

```
vino: KMS batch -- stream cmds 2, dual timings 2, dual_wake true, requested [720582127145501295 720582127145501295 0 0]
head 1 startup frame submitted ...
head 0 startup frame submitted ...
```

⚠ This means **every measurement in the previous handover was taken with the choreography
bypassed**, including the ones that concluded the cold timeline was "weaker evidence than it first
looked". They should be treated as untrustworthy, not as refutations.

### Navarro's cold timeline is now implemented

`ColdTimeline` carries Ridge's table and Navarro's. Navarro's, from
`captures/navarro-dlm-modeset-20260802-005453`, anchored on head 0's mode set: head 1's mode at
757 ms (Ridge: 29), head 0's mode set **again** at 1129, head 1's video at 1116 and head 0's at
1245, an opening `(2f,0) (2e,0)` marker pair before the `(2f,1) (2e,3)` pair, and 19 status polls
spread from 178 to 1612 ms with no silent window.

## ⚠ One change made things worse and is now default-off

Sending the 16-byte idle open on the connectors with no monitor — which DLM does — **halts the
endpoint outright** (`-EPIPE` on the very first frame URB, on both endpoints) when it goes out
0.1 ms before the first frame. DLM sends them about 4 s earlier, during EDID and setup. It is
behind `idle_opens=1`; with it off there is no `-EPIPE`, just the jam. **If it is retried, move it
to the setup phase, not to just before video.**

## Where to go next

The MAC-corruption null result is the sharpest tool now available and it points away from the video
stream's content entirely. The dock is not acting on the sealed prologue, so the question is what
state it is missing that would make it start.

1. **⭐⭐ Pace the stream like DLM.** Untested, and the largest measured behavioural difference
   left: 6.3 MB/s ramping over a second, one frame per 7–33 ms, one endpoint for the first 129 ms,
   and far fewer URBs in flight. If the dock's ring genuinely has three slots and vino queues eight
   URBs into it with no handshake, this alone could be the jam. Cheap to try.
2. **⭐ Re-run the whole previous handover's eliminations now that the choreography actually
   runs.** Several of them — prologue in its own transfer, the stream-open's sub, transfer size,
   waiting 4 s after the open — were measured on a path that skipped `activate_dual_wake`.
3. **The per-connector stream finalize and `RepeaterAuth_Stream_Manage`.** Not examined this
   session. It is the remaining control-plane state that names a stream before pixels, and it is
   the kind of thing whose absence would leave a pipe unarmed while every video byte is silently
   accepted.
4. **A keyed DLM capture in today's configuration**, via `tools/capture/capture-portmap.sh`, to
   diff DLM's control plane against vino's message for message rather than against an older
   capture. This is now more valuable than another video-side experiment.

## Tools

`/tmp/.../scratchpad/nav/` (recreate as needed; the recipes are the durable part):

* `usbpcap.py` — USBPcap (`LINKTYPE_USBPCAP`) reader plus the Navarro record walker
  `[00 00][size u16][type u32][sub u16][aux u16][seq u32]`, stride `size + 4`. The Windows captures
  in `captures/navarro-wincap-20260802/out/` had no reader before this.
* `usbmon.py` — pcapng reader for Linux usbmon captures.
* `readmon.py` — reader for `tools/hardware/capture-usbmon-session.py`'s `.mon` format. ⚠ its
  `rec_len` **excludes** the 4-byte prefix; reading it as inclusive silently yields one record.
* `seal.py` — `dl3cmac_tag` / AES-CTR mirroring `cp.rs`, and `unseal()` which returns `None` on a
  MAC failure. Use the tag as the key oracle, never a guessed plaintext shape.
* `desc.py` — rebuilds the pipe descriptor from `cp.rs`'s constants and diffs it against DLM's
  decrypted one.

Recovered video keys for `captures/navarro-dlm-modeset-20260802-005453` (frida `krs` + MAC oracle;
the recorded RIV is already the `byte7 ^= stream_id` tweaked one):

```
ep 0x08 sub 0x0007  key 960b47bf9a9a8ca8dcd98e95471a12a3  riv c345fe5593613906
ep 0x0a sub 0x000f  key e51518790217968822c77cf053164d25  riv 9446c83da5fa39ec
```

## Runtime state

```
vino: unloaded    evdi: unloaded    displaylink-driver.service: masked, inactive
module installed at /lib/modules/7.2.0-rc2-drm+/kernel/drivers/gpu/drm/vino/vino.ko
no /lib/modules/*/updates/vino.ko shadow
```

Captures from this session are in `vino/captures/navarro-vino-*` (`.mon`, read with `readmon.py`).
`navarro-vino-breakmac-*` and `navarro-vino-keyraw-*` are the two null results above;
`navarro-vino-dualwake2-*` is the `-EPIPE` from the idle opens; `navarro-vino-noidleopen-*` is the
current behaviour.
