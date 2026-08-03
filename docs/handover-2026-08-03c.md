# Handover — 2026-08-03 (third session)

Supersedes `handover-2026-08-03b.md`. The DL7400 still shows no picture. What changed is that the
protocol is now decoded rather than guessed (`docs/protocol/navarro-decoded.md`), three more
candidates are dead by measurement rather than by argument, and a **D6000 regression** was found
and fixed.

## ⛔ The D6000 was broken. Fixed, but NOT verified.

`de9521207d12`. The Ridge dock regressed to `dual-head activation failed (ENODEV)` because the
presence probe degenerates to loop rate: `probe_head_present()` sets `downstream_event` whenever the
dock moves a head's status word, and the keepalive loop answers with `next_presence = now`.
`PRESENCE_SILENT_LIMIT` counts *probes*, so three land inside 30 ms — the dmesg lines are 26 ms apart
against a 1000 ms period — and a live connector is torn down mid-modeset. Silence is now measured in
time (3 s) as well as probes, and an event-forced probe is floored at 50 ms.

⚠ **The D6000 was not enumerated at any point during this session**, so this fix has never run
against hardware. That is the first thing to do.

⚠ **Process**: every DL7400 commit had been validated on the DL7400 only. Ridge shares the keepalive
loop, the probe and the whole control path.

## ⭐⭐ The dock accepts exactly 65,536 bytes, measured two ways

This is the hardest number in the whole hunt, and it is **a byte count, not a transfer count**.
`video_sync=1` writes through `usb_bulk_msg` one transfer at a time; `video_xfer` sets the size.

```
video_xfer=65536:   1 transfer  x 65536 completes, the next times out after 1.05 s
video_xfer=16384:   4 transfers x 16384 complete,  the 5th times out after 1.06 s
                    ------------------------------
                    65,536 bytes accepted either way
```

⚠ A first 16 KiB run showed only one transfer and looked like "one transfer regardless of size".
That was a truncated capture, not a result -- vino had stopped, not the dock. Re-running with a
longer window gave the four completions above. **Do not build on a capture that ends at the
interesting moment.**

64 KiB is a landing FIFO that never drains, so the dock's video decoder never starts consuming.
Every record vino sends is *accepted* and none is *processed*, which is why corrupting the sealed
prologue's MAC changed nothing and why none of the record-level fixes below moved it.

### The pipe descriptor's slot header is a buffer descriptor

`NAVARRO_SLOT_HEADER`, carried in every slot record of the pipe descriptor, read as `u16`:

| off | value | meaning |
|---|---|---|
| 0 | `0x1000` = 4096 | image records are a 4048-byte stride |
| 2 | `0x00b4` = **180** | **bands** = 1440 / 8 |
| 4 | `0x0014` = **20** | **strips per band** = 2560 / 128 |
| 6 | `0x4000` = 16384 | the dock stops after 65536 = **4 x** this |
| 8 | `0x0001` | |

⭐ 180 and 20 are exactly the geometry derived independently from the `kind=0x200f` parameter map
(§3.4 of `docs/protocol/navarro-decoded.md`). Two unrelated records agreeing on the same numbers is
a strong cross-check that the strip geometry is right.

⚠ vino ships this leader as a **fixed constant** measured at 2560x1440. It encodes the mode's
geometry, so it is wrong for any other mode -- `navarro_pipe_descriptor()` says so, and nothing
enforces it.

## The failure, restated with the sharpest measurement available

`video_sync=1` writes video through `usb_bulk_msg` one transfer at a time — DLM's own shape, and the
first time vino has been made strictly serial. With no queue involved at all:

```
t=+0.0000  S ep0a len=65536
t=+0.0001  C ep0a len=65536  status 0
t=+0.0001  S ep0a len=65536
t=+1.0548  C ep0a len=0      status -2     <- timed out having moved nothing
```

**The dock accepts exactly one 65536-byte transfer and then refuses to take another byte.** It is
not backpressure from a slow consumer, and it is not a queueing artefact: a strictly serial writer
gets the same wall.

## ⭐⭐ Offset 23 of the mode set is an operation code — a wrong field assumption, found and fixed

A same-day keyed DLM capture (`captures/navarro-dlm-today-124144/keys-raw.json`, wire at
`~/dlm-today-124144/wire.pcapng`, 322 MB with **every byte accepted and both panels driven**)
contains **six** `id=0x48 sub=0x22` messages, not two:

```
-3.100 s  conn 0   off23=0x00   all timings zero, off42=0x8000
-3.098 s  conn 1   off23=0x00
-1.278 s  conn 0   off23=0x00
-1.170 s  conn 1   off23=0x00
-0.122 s  conn 0   off23=0x02   2560x1440@120
-0.112 s  conn 1   off23=0x02   2560x1440@120
```

vino called offset 23 a *"fixed generation/type value"* and always wrote 2. It is an **operation
code**: `0` tears a connector's pipe down, `2` sets its mode. DLM tears down before it configures.
`cp::clear_mode` (`058268aef460`) now does the same.

⚠ It does **not** lift the jam. It is committed because the field is decoded correctly now either
way, and because it is precisely the class of bug — a constant that was never a constant — that
this project has lost weeks to before.

⭐ The same capture **independently confirms the measured mode words** from `5eb36090793c`:
`off46=0x0a80`, `off48=0x66db`, `off66=0x0800`, pixel clock 49775 at 2560x1440p120, against
hardware running at that moment rather than against the previous day's capture.

## ⭐ The top remaining lead: DLM brackets the video start with a marker burst, vino does not

From the same-day capture, DLM's `id=0x0016 sub=0x2f` / `sub=0x2e` messages around the first video
byte. The payload is `[connector][value]` at offset 22:

```
-0.1155  2f 00 01     -0.0874  2e 00 00     +0.0087  2f 00 01     +0.1059  2f 00 00
-0.1095  2e 00 03     -0.0462  2e 01 03     +0.0140  2e 00 00     +0.1081  2e 00 00
-0.1021  2f 01 01     -0.0178  2f 01 01     +0.0463  2f 01 01     +0.1807  2f 01 00
-0.1009  2f 00 01     +0.0060  2e 01 03     +0.0588  2e 01 00     +0.1817  2e 01 00
```

**Sixteen markers in 300 ms**, per connector, opening `(2f=1, 2e=3)` and closing `(2f=0, 2e=0)`
*after* the video is already flowing.

vino, in the same window, sends **four** control frames — its 100 ms status metronome — and one
mode set at +0.70 s. Its cadence is a flat 100 ms; DLM's is a burst.

⚠ This is not the same as "vino is blocked". It is not: its control plane keeps running the whole
time the video pipe is jammed, with the dock answering on EP84 throughout. That rules out the
obvious deadlock (dock waits for closing markers, vino waits for the dock to drain) — **tested and
refuted this session**.

But it is a concrete, measured cadence difference at exactly the moment the dock decides whether to
start draining, and the user flagged cadence and timing from the outset. It is the best remaining
lead.

## ⛔ Killed by measurement this session

### 1. The per-strip parameter map (`kind=0x200f`)

Identified this session — see `docs/protocol/navarro-decoded.md` §3.4. It is the only record kind
in DLM's video stream vino never emitted: 180 bands x 20 strips at 2560x1440, one byte per strip.
Implemented, verified byte-exact against the capture, **and the jam is unchanged**.

⚠ It took two attempts to get it on the wire at all: it was first wired into
`encode_and_send_wht`, but the frames that matter go out through `submit_prompt_training`, a
*second* submit path with its own scatter/gather cursor. Anything added to one must be added to the
other. That trap cost a hardware cycle here.

### 2. Startup pacing

The previous handover's candidate ⭐⭐. `video_sync=1` reproduces DLM's serialisation exactly. No
change; see the trace above.

### 3. Two records vino sent that DLM does not

The full-payload capture finally allows a record-by-record diff. **Records 0 through 5 are
identical** — both stream markers, the pipe descriptor, the frame marker, the ring record, the
decoder configuration. vino then inserted a sealed stream report and the parameter map. DLM sends
neither on the frame carrying the prologue: its first frame goes straight from the configuration to
image records, and the map appears about 45 image records in. Both are now suppressed there
(`302f0d983046`). No change.

### 4. The CP-blocked-behind-video deadlock

The theory: the dock waits for the closing markers, vino waits for the dock to drain, and vino
cannot send the markers because it is blocked in `submit_prompt_training`. **Refuted.** vino's
control plane runs throughout — 64-byte frames every 100 ms with EP84 replies, from before the
first video byte to well past the `-ESHUTDOWN`. `PROMPT_TRAINING_OPEN_MS` is 0, so the submit
returns after one frame without waiting.

### 5. The pipe descriptor's layout and the mode-set operation code

Both were genuinely wrong (see above) and both are now fixed. Neither changed the jam.

### 6. Everything in `docs/protocol/navarro-decoded.md` §4

The control plane is **complete** — vino and DLM agree exactly on every outbound frame size carrying
an HDCP, mode-set or stream record before the first video byte. `RepeaterAuth_Stream_Manage` names
streams 7/15/23/31. The frame bracket is byte-exact. `aux` is legitimately 0. URB record alignment
is not required. Frames are not a fixed size.

## ⭐ What is now understood rather than assumed

`docs/protocol/navarro-decoded.md` is the reference. The headlines:

* **Nothing on this wire is "host-random."** Every sealed plaintext is a self-terminating TLV
  chain, and walking it leaves exactly the pad to a 16-byte AES block in all 29 sealed video records
  and all 5 mode sets. Boundaries are measured, not decided.
* **The per-connector video keys are transmitted**, as HDCP `SKE_Send_Eks` (`id=0x0032 sub=0x10`,
  key at off28, riv at off44). This refutes the standing claim that they are derived locally and
  need Ghidra.
* **The mode set is ordinary DRM timings**, and **offsets 70..73 are a u32 pixel clock**. vino had
  it as a u16 and rejected every mode past 655.35 MHz; it also sent Ridge's off46/off48 words on
  every DL7400 mode set. Both fixed in `5eb36090793c`, both untested.
* **`aux` is a producer lane**, not a record kind.

## Where to go next

0. **The DLM reference now exists.** `captures/navarro-dlm-today-124144/` holds the keys and
   journal; the 308 MB wire is at `~/dlm-today-124144/wire.pcapng` (kept out of the repo for size).
   DLM drove both panels on this dock minutes before the last vino run, so any future "is it the
   hardware?" question is already answered: it is not.

1. **⭐⭐ Verify the D6000 fix.** It is the only regression, it is fixed, and the dock has not been
   plugged in. Do this before anything else.
2. **⭐⭐ Question whether the video jam is even the primary failure.** Every run ends
   `dual-head activation failed (ETIMEDOUT)`, and one ended `head 0 monitor disconnected` first.
   `activate_dual_wake` is timing out on something; the video stall may be a consequence of vino
   tearing its own transaction down mid-frame rather than the cause. Nothing this session tested
   that, and it reframes the whole problem if true.
3. **⭐ A keyed DLM capture in today's configuration**, via `tools/capture/capture-portmap.sh`, to
   diff DLM's control plane against vino's message for message rather than against a capture from
   the day before. This is now the only remaining source of new information about the control plane,
   and it has been the top recommendation for two sessions running.
4. The `kind=0x200f` **values**. vino sends the all-zero map, which is what DLM sends on a quiescent
   frame. Their meaning is unestablished; a busy frame's map is a mix of 0..3.

## Runtime state

```
vino: unloaded    evdi: unloaded    displaylink-driver.service: masked, inactive
module installed  e791ff2cf7b955de7a8ab26b897bfd12297d3174aa61ee10f1347578f2d9baae
no /lib/modules/*/updates/vino.ko shadow
17e9:7000 present on bus 2;  17e9:6006 NOT present
```

Captures from this session, full payload (`--snap 70000`, so vino's frames are diffable against
DLM's for the first time): `captures/navarro-vino-parammap-121954`, `navarro-vino-parammap3`,
`navarro-vino-sync`, `navarro-vino-frame0`.

⚠ `debug=1` floods printk hard enough to make dual-head activation time out before video is
attempted. Use it for control-plane questions only, never for a video run.
