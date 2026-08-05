# Windows capture notes — WAVLINK DL7400 (Navarro, `17e9:7000`)

Session date: **2026-08-02**. Host `DESKTOP-RD09JSQ`, Windows 11 Home 10.0.26200.

## Files

| file | what |
|---|---|
| `cap1-usbpcap1.pcap` | 69 MB — **main choreography**, cold boot → plug → drags → cable walk 1→3, 2→4. SDR. **All four connectors.** |
| `cap1-usbpcap2.pcap` | 0.7 MB — other root hub, no dock traffic. Proof of a negative only. |
| `cap2-full-usbpcap1.pcap` | 244 MB — **full payload / codec reference.** Known test pattern painted into a running capture. |
| `screen-ref.png` | pixel-exact source for the socket-3 (connector 2) screen in `cap2` |
| `screen-ref-2.png` | pixel-exact source for the socket-4 (connector 3) screen in `cap2` |
| `cap3-hdr-usbpcap1.pcap` | 230 MB — HDR toggled per-connector on a live link. **Falsifies the `0x20` = HDR theory.** |
| `cap4-modesweep-usbpcap1.pcap` | 30 MB — **25 mode changes in 78 s.** Densest control-plane capture. |
| `mode-sweep-log.txt` | exact timestamp + result for each of the 25 mode changes |
| `preflight.pcap` | 4.8 MB — viability test, **pre-reboot**, HDR on, monitors in sockets 1+3. The *only* capture showing `0x20` tags. |
| `pnp-before.txt` / `pnp-after.txt` | dock PnP state at session start and end |
| `usbpcap-hubs.txt` | root hub topology |
| `tools\` | the capture harness, test-pattern/animation generators, mode sweeper and `phase-tags.py` |

**State at end of session:** monitors in dock sockets **3 and 4**, both 2560×1440 @ 60 Hz, HDR
**off**, `bcdDevice` **3922** (unchanged). Nothing was left mid-transition.

**Runbook phases:** 0–5 done (1 satisfied via Windows Update rather than a manual install; 3 and 5
completed with corrections noted below). **Phase 6 (frida session keys) skipped — impossible here,
there is no DisplayLink user-mode process to hook.** Phases 7–8 done.

---

## ⭐ Headline findings

### 1. NO FIRMWARE FLASH — `bcdDevice` unchanged at 3922

Hardware ID reads `USB\VID_17E9&PID_7000&REV_3922`, i.e. `bcdDevice = 3922`, identical to the value
recorded from Linux before this session. **The Windows driver did not flash the dock.** No DFU
transaction occurred.

Caveat on how to read this: the dock had **already been plugged in before any capture was running**
(see "Deviations" below), so the very first driver-contact moment is not on tape. But `REV_3922` is
read from the live device *after* that first contact, so the conclusion holds regardless — whatever
happened on first plug, it did not change the firmware revision.

### 2. ⚠ Frame records carry an extra `0x20` bit that Linux does not expect

This is the most significant protocol difference found. Linux's model is `sub = connector << 3` for
frame records. Windows sends the same values **with bit `0x20` set**:

| connector | stream-open `sub` | frame `sub` (Windows) | frame `sub` (Linux model) |
|---|---|---|---|
| 0 | `0x07` ✅ matches | **`0x20`** | `0x00` |
| 2 | `0x17` ✅ matches | **`0x30`** | `0x10` |

So `frame sub = (connector << 3) | 0x20` on Windows. The **stream-open** encoding
(`(connector << 3) | 7`) matches Linux exactly — only frame records differ.

### ❌ NOT HDR — hypothesis tested and FALSIFIED. Cause still unknown.

⚠ **An earlier draft of this file claimed `0x20` was a confirmed HDR flag. That was wrong and has
been retracted.** The correction is recorded here rather than deleted, because the negative result
is itself useful.

The hypothesis was: `0x20` marks HDR, since the pre-flight (HDR on) showed `0x20`/`0x30` where
`cap1` (HDR off) showed `0x00`/`0x10`. It was tested directly in `cap3-hdr.pcap`, which toggles HDR
**on one connector at a time** on a live link — giving one HDR connector and one SDR connector
streaming simultaneously down the same USB link, the cleanest possible A/B.

**Result: `cap3-hdr.pcap` contains no `0x20` or `0x30` tags at all.** Only `0x10` and `0x18`, across
the whole file, despite HDR being enabled on one or both dock screens for ~108 s of the 206 s
capture. If `0x20` marked HDR it would be unmissable in that file. It is not there.

`cap4-modesweep.pcap` reinforces this: 25 mode changes spanning 640×480@60 through 2560×1440@180,
every one of them producing only plain `connector << 3` tags.

**So `sub = connector << 3` (Linux's model) held in every post-reboot capture — cap1, cap2, cap3 and
cap4 — under SDR, HDR, and every supported resolution and refresh rate.**

What actually differs about the pre-flight, as candidate explanations for the Linux side:

1. **⭐ Most likely: it was taken before the Windows restart.** The pre-flight is the *only* capture
   from the pre-reboot boot session, where the driver had bound the dock automatically at first plug
   and had been running an unknown length of time. Every capture showing plain tags is post-reboot.
2. It is the only capture where the monitors sat in sockets **1 and 3** (connectors 0 and 2), so
   both streams shared endpoint `0x08`. If `0x20` relates to two connectors multiplexing on one
   endpoint, nothing since has reproduced that layout — **worth retesting deliberately.**
3. It is the only capture taken with the dock having been hot-plugged into an already-running
   driver rather than brought up from a cold boot.

**That follow-up run was done — see `cap5-sockets13-usbpcap1.pcap`. Explanation (2) is also
falsified.** With the monitors moved back to sockets **1 and 3** (connectors 0 and 2, both
multiplexed onto `0x08`, `0x0a` completely idle — the exact pre-flight layout), the tags were
**plain in SDR and plain in HDR**:

```
sdr-sockets13 :  sub=0x0000 x6611 [plain]   sub=0x0010 x7293 [plain]
hdr-sockets13 :  sub=0x0000 x4835 [plain]   sub=0x0010 x5778 [plain]
```

**⭐ Conclusion: `0x20` is attributable to the pre-reboot driver state and nothing else.** Every
other variable — HDR, endpoint sharing, socket layout, resolution, refresh rate — has now been
tested directly and reproduces plain `connector << 3` tags. The pre-flight remains the only capture
from the boot session in which Windows Update bound the driver and enumerated the dock
automatically for the first time.

Best remaining guess for the Linux side: on that first automatic bring-up the driver had negotiated
some different stream/format state which a clean reboot discarded. **It could not be reproduced
after reboot in this session.** If it matters, reproducing it likely means a fresh driver
installation against an unbound dock, which is the one thing this session could not stage.

`check-capture.py` reports "no connector tags decoded" for this reason — its classifier only accepts
`sub % 8 == 0 && (sub >> 3) < 4`, and `0x20 >> 3 == 4` falls outside that. The tags ARE there; the
tool's decode rule is what is out of date.

### 3. The driver streams video continuously even on a completely idle desktop

893 MB of video crossed `0x08` in 25 seconds (~35 MB/s) with **no user interaction at all** — no
window dragging, no pointer movement, static desktop. This contradicts the runbook's assumption that
the still gaps in the choreography are informative silence. On this platform there is no silence.

---

## Environment as found

| thing | state |
|---|---|
| DisplayLink driver | **already installed by Windows Update — not installed by hand** |
| driver version | **11.5.6380.0**, provider `DisplayLink`, dated 2024-12-18 |
| INF section | `dlidusb4_Install.NT`, service `WUDFRd` (user-mode driver framework) |
| DisplayLinkManager process | **none running** — no user-mode host process exists |
| Wireshark | 4.6.6, `C:\Program Files\Wireshark\` |
| USBPcap | `C:\Program Files\USBPcap\USBPcapCMD.exe` |
| Python | 3.12, `C:\Program Files\Python312\python.exe` |
| monitors | 2 × `MAG 27CQ6F`, both enumerated as children of the dock |

Note the version mismatch vs the Linux side, which runs DisplayLink **6.8.1.0** /
`DisplayLinkManager` 3.4.26. Windows' `11.5.6380.0` is a different product/versioning line
(Windows DisplayLink Graphics driver), not directly comparable, but it is a **December 2024** driver.

**There is no `DisplayLinkManager`-equivalent user-mode process on Windows with this driver.** The WU
package is entirely UMDF-side. This makes runbook phase 6 (frida key extraction) impossible as
written — there is no host process to attach to. Skipped.

## USB topology

Dock is on **`\\.\USBPcap1`**, USB device address **5**, sitting under a Generic SuperSpeed USB Hub.
`\\.\USBPcap2` carries only keyboard / mouse / Bluetooth and is irrelevant.

```
[1] Generic SuperSpeed USB Hub
 └─ [5] USB Composite Device            <-- the dock, device address 5
     ├─ Universal DP Quad Display Docking 16G
     │   ├─ Generic Monitor (MAG 27CQ6F)
     │   └─ Generic Monitor (MAG 27CQ6F)
     ├─ Universal DP Quad Display Docking 16G
     └─ DL USB Aduio  ──> Speakers (DL USB Aduio)
```

### Endpoint inventory observed

| endpoint | transfers | bytes | what it is |
|---|---|---|---|
| `0x08` | 2786 | 893,054,320 | **video** — carried everything |
| `0x09` | 382 | 362,880 | **USB audio isochronous OUT** (1920 B/transfer). Not a dock control/video endpoint — it belongs to the `DL USB Aduio` function on the same composite device. `check-capture.py` flags it as unexpected because it only models interface 0. **Benign.** |
| `0x02` | 98 | 16,448 | control OUT |
| `0x84` | 118 | 4,336 | control IN |
| `0x80` | 16 | 589 | ep0 control |
| `0x00` | 14 | 52 | ep0 setup — benign |

⚠ **`0x0a` carried no traffic at all** in the pre-flight — because both monitors were in sockets 1
and 3 (connectors 0 and 2), which per the Linux model both multiplex onto `0x08`. This is an
independent confirmation of the Linux endpoint→connector mapping from a second implementation.

---

## Deviations from RUNBOOK.md, and why

1. **Phase 1 (install driver with dock unplugged) did not happen as written.** The dock was already
   plugged in and both screens were already lit when the session started; Windows Update had bound
   the DisplayLink driver automatically. Consequence: **the first driver-contact plug is not
   captured.** Per the runbook this is the moment a one-time DFU flash could occur — but `bcdDevice`
   is still 3922, so no flash occurred.
2. **Runbook flag error:** it gives `USBPcapCMD.exe -d \\.\USBPcapN --extcap-config`. The extcap
   protocol requires `--extcap-interface`, not `-d`; with `-d` the command silently prints nothing
   and looks like a permissions failure. Correct form:
   `USBPcapCMD.exe --extcap-interface \\.\USBPcap1 --extcap-config`.
3. **`USBPcapCMD` requires elevation.** Not mentioned in the runbook. Unelevated it produces empty
   output rather than an error.
4. **`-I / --init-non-standard-hwids` was NOT needed** despite the dock being SuperSpeed+. USBPcap's
   own help says that registry key is "needed for USB 3.0 capture", but capture worked without it.
   No reboot required.
5. **Phase 6 (frida) skipped** — no user-mode DisplayLink process exists to hook. See above.
6. Captures also pass `--inject-descriptors`, so descriptors of the already-connected dock are
   embedded in each file. Necessary because the dock was enumerated before capture started.

---

## Capture log

All times are local wall clock, `HH:MM:SS`.

### `preflight.pcap` — viability test, **HDR ON**, snaplen 4096

Purpose: prove USB 3.0 capture works at all before spending a full choreography on it. Kept because
it is the only **HDR** evidence of the `0x20` frame bit.

| time | event |
|---|---|
| 11:26:28 | capture start |
| 11:26:28 → 11:26:54 | **idle** — desktop untouched, no dragging, pointer still |
| 11:26:54 | capture stop |

Result: 4,819,422 B file, 893 MB wire video volume. Monitors in **sockets 1 and 3**. HDR **on**.

```
dev/ep        transfers          bytes   role
5/0x08             2786    893,054,320   VIDEO (connectors 0 and 2)
5/0x09              382        362,880   unexpected -- report this
5/0x02               98         16,448   control OUT
5/0x84              118          4,336   control IN
5/0x80               16            589   ep0 control
5/0x00               14             52   unexpected -- report this

--- connector tags in video records (sub = connector << 3) ---
  ep 0x08:
     sub=0x0007  x78      connector 0 STREAM-OPEN
     sub=0x0017  x98      connector 2 STREAM-OPEN
     sub=0x0020  x583
     sub=0x0030  x605

VERDICT: PASS -- 893,054,320 bytes of video captured.
VERDICT: no connector tags decoded. [see finding 2 above — tool decode rule is out of date]
VERDICT: UNEXPECTED endpoints on the dock: ['0x00', '0x09']  [benign — see endpoint table]
```

### `cap1-usbpcap1.pcap` / `cap1-usbpcap2.pcap` — main choreography, **SDR**, snaplen 4096

Taken after a **clean Windows restart with the dock powered off**, so this capture contains the full
cold enumeration and driver bring-up from scratch. Both USB root hubs were captured simultaneously
(`USBPcap1` and `USBPcap2`) rather than gambling on which one the dock would land on; the dock landed
on **USBPcap1** at device address **4**. `cap1-usbpcap2.pcap` (665 KB) is keyboard/mouse/Bluetooth
only and can be ignored — it is kept solely to prove nothing dock-related went to the other hub.

HDR **off** on both screens. Monitors started in sockets **1 and 2**.

| time | step | notes |
|---|---|---|
| 11:35:42 | capture start | dock still unplugged |
| 11:35:42–11:36:0x | `idle-before` | dock absent, no dock traffic |
| ~11:36:0x | `plug-dock` | upstream cable connected, cold enumeration + bring-up |
| 11:36:38 | both screens lit, settled | `bcdDevice` re-checked: still `REV_3922` |
| 11:36:38–11:37:54 | `settle` 20 s, then `drag-screen1` 10 s / still 10 s, `drag-screen2` 10 s / still 10 s | sub-step times within this block are **nominal**; the block is bracketed exactly |
| 11:37:54 | drags complete | file at 33 MB |
| 11:38:34 | `move-1-to-3` complete | socket 1 → socket 3 (10 s unplugged, 20 s settle) |
| 11:38:57 | `move-2-to-4` complete | socket 2 → socket 4. Both monitors now on **different endpoints** (connectors 2 and 3) |
| 11:38:57–11:39:58 | `drag-both` 15 s, then `idle-after` 20 s | both connectors driven simultaneously |
| 11:39:59 | capture stopped (clean, via sentinel) | |

Final: `cap1-usbpcap1.pcap` **69,250,280 B**, `cap1-usbpcap2.pcap` 665,770 B.

```
=== cap1-usbpcap1.pcap ===
dock device address: 4

dev/ep        transfers          bytes   role
4/0x08            24070  6,752,678,784   VIDEO (connectors 0 and 2)
4/0x0a            19466  5,260,501,296   VIDEO (connectors 1 and 3)
4/0x09              652        610,560   unexpected -- report this
4/0x02             1866        122,544   control OUT
4/0x84             2123        104,133   control IN
4/0x80              372          3,993   ep0 control
4/0x00               60            208   unexpected -- report this
4/0x83               10             18   audio interrupt IN

--- connector tags in video records (sub = connector << 3) ---
  ep 0x08:
     sub=0x0000  x5354    connector 0  (physical socket 1)
     sub=0x0007  x894     connector 0 STREAM-OPEN
     sub=0x0010  x4883    connector 2  (physical socket 3)
     sub=0x0017  x814     connector 2 STREAM-OPEN
  ep 0x0a:
     sub=0x0008  x4204    connector 1  (physical socket 2)
     sub=0x000f  x706     connector 1 STREAM-OPEN
     sub=0x0018  x4062    connector 3  (physical socket 4)
     sub=0x001f  x686     connector 3 STREAM-OPEN

VERDICT: PASS -- 12,013,180,080 bytes of video captured.
VERDICT: connectors seen driving video: [0, 1, 2, 3] (sockets [1, 2, 3, 4])
         >2 connectors -- this is new ground; Linux has only ever seen two at once.
VERDICT: UNEXPECTED endpoints on the dock: ['0x00', '0x09']   [benign - audio, see endpoint table]
```

**⭐ All four connectors were seen driving video** — each of the four sockets was driven in turn
within one continuous recording, because the cables were walked 1→3 and 2→4 mid-capture.

⚠ **Read this carefully: that is NOT the same as four connectors driven *simultaneously*.** The
Linux wishlist item is "three or four connectors driven **at once**", and this session could not
deliver it — **only two monitors exist**, so at most two connectors were ever live at the same
moment. What `cap1` gives is all four connectors exercised **sequentially**, which does confirm the
tag encoding and the endpoint mapping for every connector, but says nothing new about how three or
four streams interleave on a shared endpoint under simultaneous load.

`check-capture.py` prints ">2 connectors -- this is new ground" for this file, but that message is
driven by the set of connectors seen anywhere in the capture, not by concurrency. **Do not read it
as evidence of simultaneity.** Answering the real question needs a third and fourth monitor.

**⭐ The Linux endpoint→connector mapping is independently confirmed** by a second implementation:

```
endpoint 0x08  carried connectors 0 and 2   (sockets 1 and 3)   -- as Linux predicted
endpoint 0x0a  carried connectors 1 and 3   (sockets 2 and 4)   -- as Linux predicted
```

Note that after `move-2-to-4` both monitors were on **different** endpoints (connectors 2 and 3),
which is why `0x0a` carries heavy volume here where it carried nothing at all in the pre-flight.

### `cap2-full-usbpcap1.pcap` — full payload, **SDR 60 Hz**, snaplen 4 MB

**This is the codec reference capture.** 244,087,983 B, 34.4 s, 11k packets,
11:51:11.538 → 11:51:45.938.

Reference images — **pixel-exact sources for what was on screen**:

| file | display | monitor position | connector |
|---|---|---|---|
| `screen-ref.png` | `\\.\DISPLAY85` | origin `+2560,0` | **connector 2 (socket 3)** |
| `screen-ref-2.png` | `\\.\DISPLAY86` | origin `-2560,0` | **connector 3 (socket 4)** |

Both are 2560×1440 generated test patterns, not screenshots — the exact bitmap that was blitted to
the display was saved to PNG in the same operation, so they are ground truth, not an approximation.
Pattern content, top to bottom:

1. eight saturated colour bars (R, G, B, yellow, cyan, magenta, white, black) — large flat areas with hard vertical edges
2. 16-step greyscale ramp — tests quantisation / bit depth
3. left half: 8-pixel checkerboard (worst case for block compression); right half: flat mid-grey `#808080` on the same rows — flat-vs-detail contrast at identical Y
4. white diagonals on black, plus 64×64 pure corner markers: **red = top-left, green = top-right, blue = bottom-left, white = bottom-right** — unambiguous origin, orientation and stride check
5. a burned-in label with the device name, resolution and timestamp

Sequence: capture started 11:51:11, patterns painted at **11:51:15.9**, held static 18 s, then closed
(revealing the desktop again). Both dock screens painted simultaneously.

```
dev/ep        transfers          bytes   role
4/0x08             1286    223,848,416   VIDEO (connectors 0 and 2)
4/0x0a             1074     19,705,120   VIDEO (connectors 1 and 3)
4/0x84               36          1,248   control IN
4/0x02               24            768   control OUT
4/0x80                4            523   ep0 control
4/0x00                2              8   unexpected -- report this

--- connector tags in video records ---
  ep 0x08:
     sub=0x0010  x527     connector 2  (physical socket 3)
     sub=0x0017  x105     connector 2 STREAM-OPEN
  ep 0x0a:
     sub=0x0018  x396     connector 3  (physical socket 4)
     sub=0x001f  x80      connector 3 STREAM-OPEN

VERDICT: PASS -- 243,553,536 bytes of video captured.
VERDICT: connectors seen driving video: [2, 3] (sockets [3, 4])
```

⚠ Note the strong asymmetry: `0x08` carried 223.8 MB but `0x0a` only 19.7 MB, for what should have
been two identical simultaneous full-screen paints of the same-sized pattern. Worth a look on the
Linux side — it may be scheduling, or the two connectors may be compressing differently.

---

## ⭐ Finding: the protocol is damage-driven — a static screen sends almost nothing

This cost two attempts at `cap2` and is important for anyone planning a capture.

A first attempt showed the test pattern **first** and then started a 30 s capture over a completely
static screen. Result: **28 transfers, 9.2 MB in 30 seconds** — essentially nothing, despite two lit
1440p monitors. The dock is not being continuously refreshed over USB; the driver sends only changed
regions.

The successful capture starts the recording **first** and paints the pattern **into** it, capturing
the full-screen update as it happens: **2360 transfers, 243 MB**, a 26× increase from the same
hardware in the same state.

⚠ This contradicts what the pre-flight seemed to show (893 MB in 25 s of an "idle" desktop). The
pre-flight desktop was not actually idle — a normal Windows desktop has a clock, taskbar animation
and cursor blink, all of which are damage. A *genuinely* static screen is near-silent.

**Practical rule for future captures: the interesting bytes are at moments of change.** Start the
capture before the change you care about. The runbook's phase-5 advice ("leave the pointer still",
"put something with large flat colour areas on screen") produces an almost empty file if followed
literally with the content already on screen.

## ⭐ Finding: mode changes re-create the display object (and rename it)

Changing refresh rate causes the DisplayLink displays to be torn down and re-created with **new
Windows device names**: `\\.\DISPLAY9`/`\\.\DISPLAY10` → `\\.\DISPLAY37`/`\\.\DISPLAY38` →
`\\.\DISPLAY85`/`\\.\DISPLAY86` across three mode changes in ten minutes. The names are volatile and
must never be cached across a mode change — target displays by role/position instead.

On the wire this should correspond to a stream close and re-open rather than an in-place re-tune,
i.e. fresh `stream-open` records. `cap4` records this directly.

## ⭐ Finding: 1440p @ 180 Hz is accepted over DisplayLink

`ChangeDisplaySettingsEx` returned `DISP_CHANGE_SUCCESSFUL` for 2560×1440 @ **180 Hz** on a dock
display, and the mode read back as 180 Hz. Uncompressed that is ~2.0 GB/s, far beyond USB 3.0's
~1 GB/s ceiling, so the driver must be compressing adaptively rather than shipping raw framebuffer.
Whatever `vino` ends up implementing has to account for that.

---

## ⚠ Tooling corrections (beyond the runbook errors listed above)

These cost real time; anyone repeating this should start from them.

5. **`USBPcapCMD` does not accept `-s 0`.** Unlike `tcpdump`, `0` is not "unlimited" — the process
   exits immediately, silently, leaving no output file. The runbook's phase-5 command
   (`-s 0 -A`) therefore captures nothing at all. Omit `-s` for the default (65535), or pass an
   explicit large value. **4194304 (4 MB) was used here**, comfortably above the ~280 KB largest
   observed URB.
6. **⭐ The default capture buffer is far too small and drops silently.** `USBPcapCMD`'s default
   `-b` is 1 MB. At ~280 KB per URB that is about three transfers of headroom, and a lit dock
   overruns it instantly. A full-payload capture with the default buffer recorded **22 transfers
   averaging 54 KB** — truncated and mostly dropped — with **no error, no warning, and a
   healthy-looking `VERDICT: PASS`**. Raising it to the maximum `-b 134217728` (128 MB) gave
   transfers averaging 330 KB, i.e. complete ones.

   This is a *new instance of the exact failure class the runbook warns about*: a capture that looks
   fine and is quietly worthless. `check-capture.py` passes it, because it counts bytes rather than
   checking whether transfers are whole. **Always pass `-b 134217728` for full-payload captures.**
7. `Get-Process`-style name matching for DisplayLink processes gives a false positive on `Idle`
   (case-insensitive `dl` matches `I-d-l-e`). There genuinely is no DisplayLink user-mode process.

### `cap3-hdr-usbpcap1.pcap` — HDR toggled on a live link, snaplen 4096

229,521,434 B. **This is the capture that falsified the HDR hypothesis** (see headline finding
above). Designed as an A/B: HDR was enabled on one connector at a time, so that for ~38 s one dock
screen was HDR and the other SDR, both streaming simultaneously.

Both dock screens ran a controlled animation throughout (a moving colour block at ~30 fps plus
static R/G/B/white reference bars down the left edge) — necessary because a static screen produces
almost no traffic on this protocol.

Monitors in sockets **3 and 4** (connectors 2 and 3) for this and all later captures.

| time | phase | state |
|---|---|---|
| 11:54:02 | capture start | |
| 11:54:08 | `sdr-baseline` | both dock screens SDR |
| 11:55:17 | `hdr-right` | **HDR ON, right screen only** (connector 2 / socket 3). Left stays SDR |
| 11:55:55 | `hdr-both` | HDR ON for left screen too (connector 3 / socket 4) |
| 11:57:05 | `sdr-return` | HDR OFF on both |
| 11:57:28 | capture stopped (clean, via sentinel) | |

```
dev/ep        transfers          bytes   role
4/0x0a            96260  1,620,762,464   VIDEO (connectors 1 and 3)
4/0x08            89608  1,411,886,224   VIDEO (connectors 0 and 2)
4/0x84              558         41,344   control IN
4/0x02              450         14,784   control OUT
4/0x80               28            691   ep0 control
4/0x83                8             24   audio interrupt IN

--- connector tags in video records ---
  ep 0x08:
     sub=0x0010  x29812   connector 2  (physical socket 3)
     sub=0x0017  x4971    connector 2 STREAM-OPEN
  ep 0x0a:
     sub=0x0018  x34234   connector 3  (physical socket 4)
     sub=0x001f  x5708    connector 3 STREAM-OPEN

VERDICT: PASS -- 3,032,648,688 bytes of video captured.
VERDICT: connectors seen driving video: [2, 3] (sockets [3, 4])
```

**No `0x20` bit in any phase.** Tags are identical before, during and after HDR. Phase-sliced
(via `phase-tags.py`, kept alongside this file):

| phase | ep `0x08` (connector 2) | ep `0x0a` (connector 3) |
|---|---|---|
| `sdr-baseline` | `0x0010` ×11789 **[plain]**, 446.0 MB | `0x0018` ×14843 **[plain]**, 598.0 MB |
| `hdr-right-only` — **conn 2 HDR, conn 3 SDR** | `0x0010` ×5112 **[plain]**, 191.6 MB | `0x0018` ×6630 **[plain]**, 248.9 MB |
| `hdr-both` | `0x0010` ×9167 **[plain]**, 343.3 MB | `0x0018` ×9029 **[plain]**, 338.1 MB |
| `sdr-return` | `0x0010` ×3270 **[plain]**, 122.1 MB | `0x0018` ×3264 **[plain]**, 122.0 MB |

The `hdr-right-only` row is the decisive one: connector 2 was in HDR and connector 3 in SDR **at the
same moment on the same wire**, and both emitted plain `connector << 3` tags. There is no
host-side-HDR signal in the frame `sub` field.

⚠ Note also that HDR made no dramatic difference to data volume — bytes per frame record are broadly
similar across phases. If HDR were changing the wire pixel format to 10-bit, some volume change
would be expected. It is possible **HDR is being handled entirely host-side** and the dock is fed
the same format regardless; that would be worth confirming, and would explain the null result.

⭐ The `|7` record ratio is **exactly 5.997:1 against frame records in every single phase**
(11789/1966, 14843/2475, 9167/1529, 3270/545). That level of consistency across phases with very
different volumes means these records are structural — see the finding below.

### `cap4-modesweep-usbpcap1.pcap` — every supported mode, back to back, snaplen 4096

29,880,306 B, 12:00:34 → 12:02:00. **25 mode changes in 78 seconds**, 3 s dwell each, driven
programmatically via `ChangeDisplaySettingsEx` so every transition has a sub-second timestamp.
Per-mode timings are in **`mode-sweep-log.txt`**.

Modes swept (all 32bpp, on the socket-3 / connector-2 display):

```
640x480    @ 60, 72          1280x720   @ 50, 60
720x480    @ 60              1280x1024  @ 75
720x576    @ 50              1920x1080  @ 50, 60, 120
800x600    @ 56, 60, 72, 75  1920x1440  @ 60
1024x768   @ 60, 70, 75      2048x1152  @ 60
                             2560x1440  @ 50, 60, 85, 120, 165, 180
```

**Every single one returned `DISP_CHANGE_SUCCESSFUL`**, including 2560×1440 @ 180 Hz.

```
dev/ep        transfers          bytes   role
4/0x08            11506  2,884,780,832   VIDEO (connectors 0 and 2)
4/0x0a             9152  1,258,806,448   VIDEO (connectors 1 and 3)
4/0x84             2578        210,362   control IN
4/0x02             2146         67,648   control OUT
4/0x80              720          8,422   ep0 control
4/0x83               24             36   audio interrupt IN

--- connector tags in video records ---
  ep 0x08:
     sub=0x0010  x4854    connector 2  (physical socket 3)
     sub=0x0017  x809     connector 2 STREAM-OPEN
  ep 0x0a:
     sub=0x0018  x3631    connector 3  (physical socket 4)
     sub=0x001f  x889     connector 3 STREAM-OPEN

VERDICT: PASS -- 4,143,587,280 bytes of video captured.
VERDICT: connectors seen driving video: [2, 3] (sockets [3, 4])
```

⭐ **This is the densest control-plane capture of the session** — 2578 control-IN and 2146
control-OUT transfers, roughly 5× `cap3` in under half the time. If the sealed set-mode exchange is
of interest, this file has 25 of them with exact timestamps and known target modes, which should
make them comparatively easy to align even without keys.

⚠ The final automatic restore to 2560×1440 @ 60 reported `READ-FAILED` — the display was renamed in
the gap between the last mode change and the restore call (see the renaming finding above). The
displays were restored manually straight afterwards at 12:02:16 and both dock screens ended the
session at **2560×1440 @ 60 Hz**. This is *after* the capture stopped, so it is not in the file.

---

## ⚠ Finding: `sub & 7 == 7` records are NOT once-per-stream

`NAVARRO-PROTOCOL.md` states that a stream-open `sub` "appears **exactly once** per stream, on the
endpoint owning that connector". On Windows they are routine and periodic:

| capture | connector 2 frames | connector 2 `|7` records | ratio |
|---|---|---|---|
| `cap3-hdr` | 29,812 | **4,971** | ~6 : 1 |
| `cap4-modesweep` | 4,854 | **809** | ~6 : 1 |
| `cap1` | 5,354 | 894 | ~6 : 1 |

A consistent ~6:1 ratio across captures of very different length and activity is not a
once-per-stream event — it looks like a periodic per-frame or per-group header. Either the Windows
driver uses this record differently, or the Linux interpretation of `|7` needs revisiting. Cheap to
check on the Linux side against an existing capture, and it would change how a decoder frames the
stream.

### `cap5-sockets13-usbpcap1.pcap` — shared-endpoint layout, SDR→HDR, snaplen 4096

97,056,230 B, 12:07:37 → 12:09:22. Monitors moved to sockets **1 and 3** (connectors 0 and 2, both
on `0x08`) to reproduce the pre-flight layout post-reboot. Animation on both screens throughout.

| time | phase |
|---|---|
| 12:07:43 | `sdr-sockets13` — both SDR |
| 12:08:40 | `hdr-sockets13` — HDR on for both (left then right) |
| 12:09:22 | stopped |

Result: **plain tags in both phases** (see the falsification section at the top). `0x0a` carried
**nothing at all** for the whole capture — a third independent confirmation that connectors 0 and 2
both live on `0x08`.

### `cap6-hdr-fullpayload` / `cap7-sdr-fullpayload` — the HDR A/B at full payload

Both: sockets 1+3, same animation, ~23.4 s, snaplen 4 MB, buffer 128 MB. Only HDR differs.

| | transfers | video bytes | frame records | **bytes / frame record** |
|---|---|---|---|---|
| `cap6` **HDR on** | 17,200 | 214,240,704 | 5,319 | **40,278** |
| `cap7` **HDR off** | 16,780 | 219,206,096 | 5,462 | **40,133** |

⭐ **0.4% apart — HDR makes no measurable difference to the wire.** Combined with the absence of any
`sub` flag, the conclusion is that **HDR is composited host-side and the dock is fed the same format
regardless.** There is nothing for a dock-side driver to do about HDR, and nothing for `vino` to
implement. This is now measured rather than inferred.

⚠ Note on transfer shape: these average **12–13 KB per transfer**, where `cap2` averaged **330 KB**.
Both are full-payload captures. The difference is the damage pattern — a small moving block produces
many small transfers; a full-screen paint produces few enormous ones. **A decoder must handle both
shapes.** Both are on tape.

### `cap8-180hz-fullpayload-usbpcap1.pcap` — ⭐ the codec under pressure

132,230,046 B, ~23.6 s. Both dock screens at **2560×1440 @ 180 Hz**, animation pushed to 120 fps.
Everything else identical to `cap7`.

| | video transfers | bytes/transfer | frame records | control IN | control OUT | ep0 control |
|---|---|---|---|---|---|---|
| `cap7` @ 60 Hz | 16,780 | 13 KB | 5,462 | 18 | 12 | 4 |
| `cap8` @ **180 Hz** | **1,102** | **119 KB** | **344** | **2,064** | **1,658** | **1,724** |

⭐ **This is the most significant behavioural finding of the session.**

- **Delivered frame rate collapses.** 344 frame records in ~23.6 s is roughly **15 frames/sec
  actually reaching the dock** — at a mode nominally running 180 Hz, and well below the 60 Hz case's
  ~230/s. Tripling the refresh rate cut delivered frames by **16×**.
- **Transfers grow ~10×** (13 KB → 119 KB), so the driver switches to shipping far fewer, far larger
  units.
- **The control plane explodes.** ep0 control goes from **4** transfers to **1,724**, and control
  IN/OUT rise ~100×. That is not normal steady-state operation; it looks like sustained error
  recovery or renegotiation.
- `0x0a` shows 190 transfers carrying **560 bytes total** — essentially empty, consistent with the
  layout, but the transfer count is non-zero where `cap7` had none.

**⭐⭐ 1440p @ 180 Hz DESTABILISES THE DOCK — operator-observed.**

Shortly after this capture the dock **began disconnecting and reconnecting in a loop** and had to be
**powered off manually** to stop it. That is a first-hand observation by the operator, not an
inference from the bytes. **Treat 2560×1440 @ 180 Hz on this dock as a known-bad mode.**

Be precise about what the capture does and does not show, because the two are easy to conflate:

**Measured in `cap8`:** frame delivery collapsed ~16×, transfers grew ~10×, and control-plane
traffic rose ~100× (ep0 control 4 → 1,724).

**NOT visible in `cap8`:** any re-enumeration. The dock holds USB device address **4** throughout,
exactly as in `cap7`, and **no new device addresses appear**. A search for `GET_DESCRIPTOR`
(`bRequest==6`) returns **zero** hits in both captures — USBPcap did not decode enumeration detail
here, so that avenue is inconclusive rather than negative.

**Most likely reading:** the disconnect/reconnect loop began *after* `cap8`'s 23.6 s window closed.
What `cap8` captures is therefore the **onset** — the link degrading and the control plane thrashing
— rather than the loop itself. The loop is **not on tape.**

**⭐ Implication for `vino`: mode acceptance is not evidence of deliverable bandwidth.**
`ChangeDisplaySettingsEx` returned `DISP_CHANGE_SUCCESSFUL` for 1440p@180, and the mode read back
correctly, yet the link cannot carry it — and pushing it appears able to knock the dock into a
reconnect loop. A driver that publishes every EDID-advertised mode will let a user select one that
does this. Note `cap4` set this same mode for 3 s during the sweep with no observed ill effect, so
**sustained** operation seems to be what matters, not the mode set itself.

⚠ **Reproduce with care.** This was observed once. Anyone repeating it should expect to have to
power-cycle the dock. No firmware flash is involved (`bcdDevice` unchanged throughout the session),
so the failure appears transient rather than damaging.

---

## Session log corrections

- `cap7` was captured **twice** — 12:17:32 (320,913,645 B) and 12:17:55 (220,201,745 B) — because a
  stale result file made the first run look like it had not happened, and the second run overwrote
  it. **The surviving file is the 12:17:55 one**, and it is the one all figures above are computed
  from. Both were valid captures of the same configuration.
- `cap9` (known test pattern painted on the shared-endpoint layout) was **attempted and did not
  happen** — the elevated capture runner had died, so the patterns painted with nothing recording.
  There is no `cap9` file. This would have given known-image pixels with two connectors interleaved
  on one endpoint; `cap2` provides known-image pixels but with the connectors on *separate*
  endpoints, and `cap6`/`cap7` provide shared-endpoint pixels but of an animation rather than a
  reference image. **That specific combination is the one gap left.**

<!-- end of capture log -->

---
---

# Session 2 — 2026-08-05 — HDR with genuinely HDR content

Executed `HDR-RUNBOOK.md`. All phases captured and verified except one, called out below.
Everything here is from this session; the 2026-08-02 material above is untouched.

**The headline: the dock is told when HDR turns on, and the OS really does hand it a 10-bit
surface.** The 2026-08-02 conclusion ("HDR is host-side, nothing for vino to do") does not
survive — as the runbook predicted, because that comparison played SDR content in both halves.
What the wire does with the 10-bit pixels is now decidable from `cap9`, which is the point.

## What made this session different

`cap6`/`cap7` compared HDR-mode-on against HDR-mode-off **with the same SDR animation on screen**.
This session put real PQ / BT.2020 / 10-bit content on the panel and A/B'd it against the same
pictures in BT.709 8-bit. Crucially the preconditions were *read back*, not eyeballed:

| check | HDR off | HDR on |
|---|---|---|
| `matchMedia('(dynamic-range: high)')` | false | **true** |
| `screen.colorDepth` | 24 | **30** |
| display config `bitsPerColorChannel` | 8 | **10** |
| display config `colorEncoding` | RGB | RGB |

Windows really did switch the head to a 10-bit RGB surface. That is what the previous session
never established, and it is what makes the pixel comparison meaningful.

⭐ **Phase 7.3 answered from the API, not a screenshot.** "Bit depth" and "Colour format" in
Settings → Advanced display are `bitsPerColorChannel` and `colorEncoding` from
`DISPLAYCONFIG_GET_ADVANCED_COLOR_INFO`. The values above are read straight from it, which beats
OCR'ing a picture. Screenshots are in `out\advanced-display-hdr-{on,off}.png` regardless, but the
numbers to trust are in the table.

## ⛔ The physical setup was NOT what the runbook assumed — read before analysing

**At session start only ONE monitor was on the WAVLINK.** The second was plugged into the old
**Dell D6000** (`17e9:6006`), which is also connected. Evidence: that monitor's PnP parent was
`USB\VID_17E9&PID_6006&MI_00`, and the head reported `HDR supported = False` — correct and
expected, since HDR10 is DL-7000-series only. It also capped at 120 Hz where the DL7400 head
offered 180.

The operator moved the cable mid-session. **From `preflight2` onward both monitors are on the
DL7400** (`USB\VID_17E9&PID_7000&MI_00`, UID256 and UID257). Every `cap9`–`cap14` capture has both
heads on the WAVLINK. The D6000 stayed plugged in over USB throughout but drove **no displays**
from `preflight2` on — separate USB device address, separable, but present in the captures.

⚠ **The runbook's socket advice is self-contradictory, and we did not follow it.** It says to use
"sockets 1 and 3 (connectors 0 and 2 — different video endpoints)", but by its own mapping
`ep 0x08` owns connectors {0,2} and `ep 0x0a` owns {1,3} — so connectors 0 and 2 are *both* on
`0x08`. The layout actually used was **sockets 1 and 2 = connectors 0 and 1**, which does put one
monitor on each endpoint, i.e. it achieves the stated *goal*. Confirmed from the wire by
`check-capture.py`:

- **connector 0 → socket 1 → `ep 0x08` → `\\.\DISPLAY29`** (UID256) — the main capture head
- **connector 1 → socket 2 → `ep 0x0a` → `\\.\DISPLAY30`** (UID257)

## Phase 0 — the 180 Hz rescue

The dock was **not** at 180 Hz on arrival: `DISPLAY29` was at 60 Hz, the other head at 120 Hz.
Both set to **2560×1440 @ 60 Hz** and left there. (`cap14` deliberately visits 120 Hz and returns.)

⛔ **`rescue-refresh.ps1` did not work as shipped and had to be fixed.** Three real bugs:

1. `EnumDisplayDevicesW($null, ...)` — PowerShell converts `$null` to `""` when marshalling to a
   `[string]` P/Invoke parameter, and `EnumDisplayDevices("")` fails. `Get-Devices` returned an
   empty list, so **`-List` printed nothing and the rescue silently did nothing at all.** Fixed
   with `[NullString]::Value`.
2. The final commit `ChangeDisplaySettingsExW($null, [IntPtr]::Zero, ...)` had the same fault, so
   the registry writes were never applied.
3. It retried the ~12 phantom detached GPU heads for the full 90 s timeout. Now skips displays with
   no stored mode.

Also added `-Device` so one head's refresh can be changed alone (7b.2 holds the other as a control).

## Tooling written this session (all in `tools\`, all used for every capture)

The runbook's choreography is "toggle HDR, wait N seconds, change clip". Done by hand that is a
stopwatch, a keyboard and a notepad — which is how `cap6`/`cap7` ended up mislabelled. So it was
automated, with every step timestamped to the millisecond.

- **`hdr.ps1`** — read/set per-display HDR via `DisplayConfigSetDeviceInfo` /
  `DISPLAYCONFIG_SET_ADVANCED_COLOR_STATE`, the same API Settings drives. Prints an ISO timestamp,
  before/after state, bit depth and colour encoding for every toggle. **This is what makes the
  captures sliceable.**
- **`cdp.ps1`** — evaluates JS in the running `player.html` over the DevTools protocol. Used to
  *verify* the runbook's preconditions (`dynamic-range high`, `devicePixelRatio 1`, `1:1 mapping`)
  instead of trusting a glance at the info panel, and to drive the player.
- **`choreograph.ps1`** — runs a whole phase (`h1`, `h2`, `probes`, `axes`, `bandwidth`), refusing
  to start if dpr ≠ 1 or the picture is not 1:1. Writes `<prefix>.phaselog.txt`.
- **`phase-mode120.ps1`** — phase 7b.2.
- **`capture-runner.ps1`** — one elevated process for the whole session (USBPcap needs admin), taking
  jobs from `out\jobs\`. One UAC prompt instead of one per capture, so no consent dialog lands in
  the middle of a timed sequence.
- **`dpiscale.ps1`** — reads/sets per-display scaling.

⭐ **Two PowerShell traps worth knowing, both of which silently yield zeroes rather than errors:**
`$null` passed to a `[string]` P/Invoke parameter becomes `""`; and `$struct.header.type = 1`
mutates a *copy*, because reading a value-type field boxes it — every `DisplayConfigGetDeviceInfo`
call failed until headers were built whole and assigned in one go. Both are commented in the scripts.

⚠ **Display scaling was a false alarm.** An early measurement said the dock screens were at 150%.
That was wrong: the measuring process was only *system*-DPI-aware, so `GetDpiForMonitor` reported
the laptop's 144 for every monitor. The dock heads were at **100%** all along — `dpiscale.ps1 -List`
and the player's `devicePixelRatio 1` / `1:1 mapping yes` both confirm it. Nothing was resampled.

## Captures — all PASS on `check-capture.py`

Verified **before** rebooting, as required. Phase logs sit beside each capture as
`<prefix>.phaselog.txt`; the player's own segment/probe log is `out\player-log-session2.txt`.

| file | phase | snaplen | size | video bytes | verdict |
|---|---|---|---|---|---|
| `preflight2-usbpcap1.pcap` | plumbing check | 4096 | 17.7 MB | 215 MB | PASS |
| `cap9-hdr-ab-usbpcap1.pcap` | **H1 — the A/B** | 0 (full) | **962 MB** | 1,280 MB | PASS |
| `cap10-hdr-ab-ep0a-usbpcap1.pcap` | H2 — other endpoint | 0 (full) | 471 MB | 868 MB | PASS |
| `cap11-metadata-probes-usbpcap1.pcap` | H3 — 7 metadata probes | 4096 | 13.2 MB | 173 MB | PASS |
| `cap12-axes-usbpcap1.pcap` | H4 — HDR on static desktop | 4096 | 7.7 MB | 520 MB | PASS |
| `cap13-bandwidth-usbpcap1.pcap` | H5 — both heads | 4096 | 34.5 MB | 956 MB | PASS |
| `cap14-mode120-usbpcap1.pcap` | 7b.2 — 60/120 Hz × HDR/SDR | 4096 | 41.4 MB | 972 MB | PASS |
| `cap15-sdrbrightness-usbpcap1.pcap` | **7.2 — SDR brightness slider** | 4096 | 74.9 MB | 5,035 MB | PASS |

`-BufferLen 134217728` on every one. The `usbpcap2` files are ~1–6 KB (the dock is on hub 1); kept
for completeness. Dock was USB device address **5** throughout, with no re-enumeration mid-capture.

### H1 (`cap9`) — the capture this session exists for

Same 14 pictures, same mode, same connector 0, same player. Wall clock from the phase log:

```
15:41:35.714  HDR OFF  (start from SDR)      bpc 8
15:41:36        idle 15 s
15:41:52.776  HDR ON                          bpc 10   <- dynamic-range high, colorDepth 30
15:41:57.503  hdr-pattern.webm  (14 segs, 92 s)
15:43:30.012  hdr-motion.webm   (32 s)
15:44:02        settle 15 s
15:44:18.156  HDR OFF                         bpc 8    <- dynamic-range standard, colorDepth 24
15:44:22.935  sdr-pattern.webm  (14 segs, 92 s)
15:45:55.414  sdr-motion.webm   (32 s)
15:46:27        idle 15 s
```

Per-segment boundaries for both halves are in `out\player-log-session2.txt` — e.g. HDR `grey1000`
at 15:42:15.641, SDR `grey1000` at 15:44:41.051. **`grey100` vs `grey1000` is the discriminating
pair** (100 is inside SDR range, 1000 is not). The panel peak is ~302 cd/m², so `grey1000` and
`grey4000` may both be tone-mapped and land similarly — expected, not a bug.

### H3 (`cap11`) — probes ran; HEVC is present

⭐ **The HEVC decoder IS installed** — `canPlayType('hvc1.2.4.L153.B0')` = `probably`, and
`probe-A-baseline.mp4` loaded at 2560×1440. Phase 6 ran properly; it was not skipped. All seven
clips played with **no decode errors**. Exact start/end per clip, 6.5 s each with 15 s of black
between, in `player-log-session2.txt`:

```
15:55:54.329 A-baseline   15:56:15.838 B-peak4000   15:56:37.360 C-peak605
15:56:58.872 D-nometa     15:57:20.377 E-p3prim     15:57:41.895 F-hlg
15:58:03.409 G-bt709tag   15:58:24.930 sequence complete
```

### `cap14` — 60 Hz vs 120 Hz × HDR vs SDR in one file

Four states, other head held at 60 Hz SDR as a control: `16:07:34` 60 Hz SDR → `16:08:05.883`
60 Hz **HDR** → `16:08:38` **120 Hz HDR** (verified `dynamic-range high`, `colorDepth 30` at
120 Hz) → `16:09:18.288` 120 Hz SDR → `16:09:51` back to 60 Hz. Nothing above 120 Hz was attempted.

## ⭐ New observations for the Linux side

1. **`ep 0x09` appears, and only in `cap13`** — 32 transfers, 30,720 bytes, exclusively in the
   both-heads capture. It is in no other capture this session, and Linux has never seen it.
   `cap13` is the only capture where **two views were in different HDR states at once**. Look here
   first: if HDR metadata is per-view rather than per-device, this is where it shows its hand.
2. **`ep 0x00` shows 2 transfers / 8 bytes in every capture** (12 in `cap13`). Consistent, tiny,
   present in the 2026-08-02 captures too. Probably a USBPcap artefact rather than a real endpoint,
   but `check-capture.py` flags it every time.
3. **Control-plane volume tracks HDR activity, not pixels.** `cap9` (two toggles, 962 MB of video)
   636 control-IN / 504 control-OUT; `cap12` (four toggles, **static desktop, no content at all**)
   **732 IN / 624 OUT** — more control traffic than the big content capture. ⭐ That is the phase
   7.1 result: **toggling HDR alone generates substantial control traffic with nothing on screen**,
   so the dock is definitely being told something when HDR changes.
4. **Both heads negotiate 10-bit independently.** `cap13` holds head-A-only-HDR, both-HDR, and
   head-B-only-HDR. Each toggle reported `bpc 8 ↔ 10` for that head alone while the other stayed at
   8 — so at the OS level it is per-connector, not per-device. Whether the *wire* agrees is what
   `cap13` plus the new `ep 0x09` should settle.
5. **`(video-dynamic-range: high)` stayed `false` even while `(dynamic-range: high)` was true.**
   Noted in case it means Edge never took an HDR video overlay path and composited in software.
   It does not change the pixels reaching the dock — the surface was 10-bit either way — but it is
   a difference from a "proper" HDR video playback path, worth knowing when reading the codec.

## Not done, and why

- ✅ **Phase 7.2 was completed after all** — see the dedicated section below. Nothing in the runbook
  is now outstanding.
- ⛔ Session keys / `WUDFHost.exe` attach — deliberately out of scope, as the runbook says.
- ⛔ The dock's own `id=0x78 sub=0x30` DISPLAY-CAP push — Linux-side job, needs keys.
- ⚠ **The `53 53 45` / `e6 06 07 01 53 53 45` EDID-forwarding grep was not run.** No deep analysis
  was done here, per the runbook's instruction; the captures are on disk for it.
- ⚠ Gamma/CTM under HDR — not exercised, still a gap.

## Machine state left behind

- **Dock at 2560×1440 @ 60 Hz on both heads.** ✅ Not at 180.
- HDR left **on** for `DISPLAY29` (socket 1) and **off** for `DISPLAY30`, matching how the dock was
  found at session start. ⚠ Its **SDR content brightness slider is at minimum** after phase 7.2 and
  needs putting back by hand — see that section.
- `bcdDevice` = **REV_3922, unchanged.** No firmware flash, no DFU traffic. `pnp-after.txt` written.
- ⚠ **Power settings were changed and left changed:** monitor-timeout-ac/dc → 0, standby-timeout-ac
  → 0, `ScreenSaveActive` → 0. **Originals: AC 300 s, DC 180 s, ScreenSaveActive 1.** The display
  blanked once at ~16:09, *after* every capture had finished — the longest no-video stretch inside
  any capture was under 3 minutes, so no capture is contaminated. Restore these.
- The D6000 (`17e9:6006`) is still plugged in over USB, driving nothing.

<!-- end of session 2 capture log -->

## Phase 7.2 — the SDR content brightness slider (`cap15-sdrbrightness`)

Done last, by hand, because the slider has **no public setter**. It does have a getter, though:
`DISPLAYCONFIG_DEVICE_INFO_GET_SDR_WHITE_LEVEL` (device info type **11** — note 12 returns success
with a zero level, which reads like a slider at minimum rather than like the error it is). So
`tools\sdr-whitelevel.ps1` polled the value at 5 Hz throughout and logged every change. The result
is not "the operator moved a slider at some point" but an exact list of instants.

Setup: HDR **on** for `\\.\DISPLAY29` (connector 0, socket 1, `ep 0x08`), `sdr-pattern.webm`
playing on that head — i.e. genuinely SDR content inside an HDR container, which is what the slider
acts on. Other head SDR at 60 Hz. Capture 16:17:38 → 16:20:35, snaplen 4096.

**The operator swept the slider five times**, 16:18:38.393 → 16:19:04.331:

- **99 discrete changes**, all logged with millisecond timestamps in `cap15-sdrbrightness.phaselog.txt`
- range **1000 → 6000**, i.e. **~80 → ~480 nits** (`nits = level / 1000 × 80`)
- five full min→max→min passes; each pass takes ~5 s of wall clock and ~20 changes
- `bitsPerColorChannel` stayed **10** across every single change — the slider does not drop the
  surface out of 10-bit

⭐ **The wire moves when the slider moves, and it moves a lot.** `cap15` carries **38,178 transfers /
637 MB on `ep 0x08`** with **15,036 connector-0 frame records** in a ~170 s window — against a
`sdr-pattern` source that is *static within each 6 s segment* and would otherwise produce almost
nothing. For comparison, `cap9`'s entire 92 s HDR pattern pass plus 32 s of motion produced 24,321
connector-0 records. So slider movement alone is generating full-screen repaints at a high rate.

**Reading, stated carefully:** this is consistent with **Windows compositing SDR content into the
HDR surface host-side** and pushing the recomposited result to the dock — the dock is not being
told "the SDR white level is now X" and doing the mapping itself, or the traffic would be a small
control message rather than a flood of pixels. ⚠ That is an inference from volume and timing, not
from decoded control messages; the phaselog gives 99 exact instants to check it against, and the
control plane in this capture is notably *quiet* (132 control-IN / 88 control-OUT — far less than
`cap12`'s 732/624 for four HDR toggles). **A metadata-carrying design would look like the opposite
of this.** Confirming it is a Linux-side job.

⚠ **`ep 0x0a` carries 4,398 MB in this capture** — the largest single figure of the session, on the
head that was *not* being changed and was showing a static desktop. Almost certainly the reported
URB buffer length rather than real pixels (snaplen 4096 means payload is truncated, and
`check-capture.py` sums the reported transfer length), but it is out of line with every other
capture and worth not taking at face value.

⚠ **The slider was left at minimum (level 1000, ~80 nits). It was at 3000 (~240 nits) before this
phase.** There is no public setter, so it needs putting back by hand in Settings → System →
Display → HDR → SDR content brightness, for the socket-1 monitor.

## Addendum — the `0x20` frame-tag question from session 1

Session 1 found `frame sub = (connector << 3) | 0x20` in the pre-reboot `preflight.pcap` **and
nowhere else**, and concluded it was attributable to the pre-reboot driver state after HDR,
endpoint sharing and socket layout had each been falsified as causes.

**Session 2 adds another seven captures' worth of negative evidence.** Every capture here decodes
plain `connector << 3` tags — `sub=0x0000` for connector 0 and `sub=0x0008` for connector 1, with
`0x0007` / `0x000f` stream-opens — under conditions session 1 never tested:

- HDR **on**, with the surface genuinely at **10 bits per channel** and PQ / BT.2020 content
- HDR toggled repeatedly on a live link, on either head independently, and on both at once
- 60 Hz and 120 Hz
- HEVC Main 10 and VP9 profile 2 sources

So the `0x20` bit does **not** track HDR, bit depth, colour space, refresh rate or codec. Session
1's conclusion stands and is now much better supported: **`0x20` belongs to that first
automatically-bound, pre-reboot driver session and nothing that has been reproduced since.**

⚠ `check-capture.py`'s classifier still only accepts `sub % 8 == 0 && (sub >> 3) < 4`, so it would
still mis-report a `0x20` tag as "no connector tags decoded". That did not bite this session
because no `0x20` appeared, but the decode rule is still out of date and should be widened before
anyone relies on that tool to detect the case.
