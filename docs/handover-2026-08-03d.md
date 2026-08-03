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

`tools/codec/navarro-render.py` reconstructs the dock's whole framebuffer from vino's own captured
EP08 bytes. Against the source image, with a purpose-built pattern on screen:

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

**Next step:** load with `trace_crypto=1`, capture bus 2, and decrypt vino's own control dialogue to
see which message it is waiting on. Guessing has now failed twice.

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
