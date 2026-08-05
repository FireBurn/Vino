# HDR capture runbook — WAVLINK DL7400 (Navarro, `17e9:7000`)

**Read this file, then do it.** Work top to bottom. Each phase ends in a check you must actually
run. Everything you produce goes in `C:\Users\Mike\navarro-wincap\out\`.

This is the **second** Windows session. The first one (2026-08-02) captured bring-up, hotplug, the
codec and a mode sweep; its results are in `out\NOTES.md` and are already understood on the Linux
side. Do not redo them.

---

## Why this session exists

The Linux driver (`vino`) drives the DL7400 as an ordinary dual-head SDR KMS device. It has no HDR
path, and we do not yet know what one would have to do.

What we know:

- DisplayLink's binaries on **all three** host platforms carry the same pixel-format enum, including
  **`NM30` (10-bit packed), `FP16` and `YU10`**. A driver with no deep-colour path does not need
  those.
- The Windows stack has a `setHdrMetadata` IPC method sitting next to `setGammaRamp`, per view. That
  maps onto DRM's `HDR_OUTPUT_METADATA` connector property.
- HDR10 on DisplayLink is documented as **DL-7000 series on Windows 11 only**. This dock, this OS.

What the 2026-08-02 session did **not** settle, and this one must:

> ⛔ `cap6` (HDR on) and `cap7` (HDR off) played **the same SDR animation**. Windows was in HDR
> *mode* with nothing of HDR *range* or wide gamut ever on screen. An SDR picture in an HDR
> container has SDR bit depth and SDR gamut, so an identical wire is exactly what you would expect
> **even if the dock has a full 10-bit path**. The comparison could not have detected a difference.
>
> The old "the two captures are 0.4% apart, so HDR is host-side and there is nothing for vino to do"
> conclusion inherits the same flaw — 0.4% is a *bandwidth* comparison, not a *format* one, and this
> codec is lossy and content-driven.

So: this session puts **genuinely HDR content** on the dock — light above SDR range, colour outside
BT.709 — and A/Bs it against the identical picture in SDR.

---

## 0. FIRST: get the dock out of 180 Hz

The dock is currently at (or will come up at) **2560×1440 @ 180 Hz**, which knocks it into a
reconnect loop — the whole hub re-enumerates every few seconds and the dock screens never settle.
Windows persists the refresh rate per monitor, so rebooting does not clear it.

⚠ **Do this from the laptop's own screen.** If the dock screens are looping you cannot see a window
that is on them. If they are not usable at all, `Win`+`P` → **PC screen only** first.

```
cd C:\Users\Mike\navarro-wincap\tools
powershell -ExecutionPolicy Bypass -File .\rescue-refresh.ps1 -List
powershell -ExecutionPolicy Bypass -File .\rescue-refresh.ps1 -Hz 60
```

`rescue-refresh.ps1` cuts output to the dock (`DisplaySwitch /internal`), rewrites the **stored**
mode for every non-primary display while nothing is contending for it, commits, and re-extends. It
retries for 90 s because the dock may be mid-reconnect, and it prints what to try next if it fails.

**Check:** `-List` afterwards must show both dock displays at 60 Hz, attached. Record the before and
after in `out\NOTES.md`.

⭐ **60 Hz for the whole session.** The mode ceiling is already known from `cap4-modesweep`; refresh
is not what we are studying here, and a link at its limit is one more reason for a capture to fail
in a way that looks like an HDR result.

---

## 1. Preconditions

| thing | required state |
|---|---|
| both dock monitors | plugged into sockets **1 and 3** (connectors 0 and 2 — different video endpoints) |
| mode | 2560×1440 @ 60 Hz on both |
| display scaling | **100%** on the dock screens — Settings → System → Display → Scale. A scaled desktop resamples the test pattern and every decoded pixel becomes a blend of two |
| Wireshark / USBPcap | already installed, `C:\Program Files\USBPcap\USBPcapCMD.exe` |
| Python | `C:\Program Files\Python312\python.exe` |

Sockets 1 and 3 matter: `ep 0x08` owns connectors {0,2} and `ep 0x0a` owns {1,3}, so one monitor per
endpoint keeps the two streams separable.

### Check the HDR toggle actually works

Settings → System → Display → **HDR**, select a dock monitor, turn **Use HDR** on. If the toggle is
missing or greyed out for the dock screens, stop and record that in `out\NOTES.md` — it is a
finding, and phases 3–6 cannot run.

⭐ **It should be there.** Both dock monitors were read from Linux on 2026-08-05 (EDIDs and full
decodes in `hdr-content\sink-edid\`) and both are **MSI MAG 27CQ6F** advertising, in the CTA-861
HDR Static Metadata Data Block:

| | value | raw EDID byte |
|---|---|---|
| EOTFs | traditional SDR, traditional HDR, **SMPTE ST 2084** | `0x07` |
| static metadata descriptors | type 1 | `0x01` |
| desired content max luminance | **301.8 cd/m²** | `0x53` (83) |
| desired content max frame-average | **301.8 cd/m²** | `0x53` (83) |
| desired content min luminance | **0.221 cd/m²** | `0x45` (69) |
| colorimetry | xvYCC601/709, **BT2020YCC, BT2020RGB** | `0xc3` |

⭐ **Those raw bytes are worth grepping for in every capture.** If the host tells the dock about the
sink's HDR capability at all, this is the most likely thing it says — `53 53 45`, or `83`/`302`
(`0x012e`) after unit conversion, or the whole `e6 06 07 01 53 53 45` block forwarded verbatim.
Finding them settles where vino would have to get its `HDR_OUTPUT_METADATA` values from.

⚠ **The panel peak is only ~302 cd/m².** Content above that gets rolled off somewhere between the
compositor and the glass, so `grey1000` and `grey4000` may well land at similar values on the wire —
that is tone mapping, not a codec bug. **The discriminating pair is `grey100` vs `grey1000`**
(100 is inside SDR range, 1000 is not), with `steps8` bracketing the panel peak at 203 and 400.

Note the **SDR content brightness** slider position. Do not touch it until phase 7.

---

## 2. The test content

`C:\Users\Mike\navarro-wincap\hdr-content\` holds everything. See `SEGMENTS.md` for exactly what
each picture contains and `manifest.json` for the machine-readable version.

| file | what |
|---|---|
| `hdr-pattern.webm` / `.mp4` | 14 static segments, 6 s each. PQ, BT.2020, 10-bit, **lossless** |
| `sdr-pattern.webm` / `.mp4` | the same 14 pictures in BT.709 8-bit, 203 cd/m² mapped to SDR white |
| `hdr-motion.webm` / `.mp4` | 1000 cd/m² block sweeping a 100 cd/m² field, 30 s — continuous damage |
| `sdr-motion.webm` / `.mp4` | the SDR twin |
| `probes\probe-*.mp4` | 7 clips, **identical pixels**, differing only in HDR10 metadata |
| `ref\*.png` | the exact picture that went into the encoder |
| `ref\decoded\*.png` | the same after the codec round trip — **compare wire output against these** |
| `player.html` | opens in Edge; picks a clip, steps segments, keeps a timestamped log |

**Use `player.html` in Edge.** Chromium has its own VP9 and AV1 decoders, so `.webm` plays with no
OS codec installed. `.mp4` is HEVC and needs the Windows HEVC extension, which may not be present —
if the mp4s do not play, that is fine for phases 3–5, but it **does** block phase 6.

How to run it:

1. Open `hdr-content\player.html` in **Edge**.
2. Drag the window onto the **socket-1** dock monitor.
3. Press `f` for fullscreen, then `i` for the info panel.
4. ⭐ The info panel must read **`dynamic-range  high`**, **`devicePixelRatio 1`** and
   **`1:1 mapping  yes`**. If any of those is wrong, fix it before capturing — an HDR capture taken
   through a window that Edge thinks is SDR, or a picture that got resampled, is a wasted run.
5. Press `h` to hide the panels before the capture proper.

Keys: `1`-`8` source, `←`/`→` segment, space pause, `m` add a mark, `l` show the log to copy into
`NOTES.md`.

---

## 3. Capture H1 — the A/B that the last session could not do

This is the whole point. Same picture, same mode, same everything; **HDR on, then HDR off**.

Start a full-payload capture (the codec is what we are reading, so no snaplen):

```
cd C:\Users\Mike\navarro-wincap\tools
powershell -ExecutionPolicy Bypass -File .\capture-both.ps1 `
  -OutPrefix C:\Users\Mike\navarro-wincap\out\cap9-hdr-ab `
  -Snaplen 0 -MaxSeconds 420 -BufferLen 134217728 `
  -FlagDir C:\Users\Mike\navarro-wincap\out
```

⚠ `-BufferLen 134217728` is not optional. USBPcap's default 1 MB kernel ring silently drops almost
everything from a lit dock.

Then, logging every step with wall-clock times (the player's `l` log does this for you):

1. `idle` — 15 s, nothing moving.
2. **HDR ON** for the socket-1 monitor.
3. `hdr-pattern` — play `hdr-pattern.webm` fullscreen, all the way through (90 s). Do not touch the
   mouse; the segments advance themselves.
4. `hdr-motion` — source `5`, let it run 30 s.
5. `settle` — 15 s idle.
6. **HDR OFF** for the socket-1 monitor.
7. `sdr-pattern` — source `3`, all the way through (90 s).
8. `sdr-motion` — source `7`, 30 s.
9. `idle` — 15 s.

Stop the capture (`echo x > C:\Users\Mike\navarro-wincap\out\stop.flag`).

⭐ **The reason this ordering works:** steps 3 and 7 are the same 14 pictures at the same resolution
and refresh, from the same player, on the same connector. Anything that differs between them is
either the HDR mode or the content's dynamic range — and the per-segment structure tells you which,
because `grey100` is inside SDR range and `grey1000` is not.

---

## 4. Verify BEFORE going further

```
"C:\Program Files\Python312\python.exe" C:\Users\Mike\navarro-wincap\check-capture.py C:\Users\Mike\navarro-wincap\out\cap9-hdr-ab-usbpcap1.pcap
```

You need **real video volume on `0x08`** and connector tags (`sub = connector << 3`). A
control-plane-only capture is a wasted run — it has happened twice on the Linux side and it looks
completely healthy from the driver's side. If there is no video, redo phase 3.

Paste the output into `out\NOTES.md`.

---

## 5. Capture H2 — the same A/B on the *other* endpoint

Repeat phase 3 exactly, but with the player on the **socket-3** monitor (connector 2, endpoint
`0x0a`), as `cap10-hdr-ab-ep0a`. Shorter is fine: pattern only, no motion.

This exists because a per-connector difference and a per-endpoint difference look identical in a
single-connector capture, and the Linux side has been caught by exactly that shape of ambiguity
before.

---

## 6. Capture H3 — the metadata probes

⚠ Needs the HEVC decoder. If `hdr-pattern.mp4` would not play in phase 2, skip this phase and say so
in `NOTES.md`.

`hdr-content\probes\` holds seven 6-second clips. **The coded samples are byte-identical across all
seven** — same picture, same YUV matrix, same lossless encode. Only the HDR10 static metadata
differs, one axis at a time:

| clip | what is different |
|---|---|
| `probe-A-baseline.mp4` | mastering peak 1000, MaxCLL 1000, MaxFALL 400, BT.2020 primaries |
| `probe-B-peak4000.mp4` | mastering peak **4000**, min **0.005**, MaxCLL **4000**, MaxFALL **1234** |
| `probe-C-peak605.mp4` | mastering peak **600**, min **0.05**, MaxCLL **605**, MaxFALL **123** |
| `probe-D-nometa.mp4` | PQ + BT.2020 tagged, **no** mastering display, **no** MaxCLL at all |
| `probe-E-p3prim.mp4` | baseline, but **DCI-P3** mastering primaries |
| `probe-F-hlg.mp4` | **HLG** transfer instead of PQ |
| `probe-G-bt709tag.mp4` | **BT.709** primaries/transfer tags on byte-identical samples |

With HDR on, the player fullscreen and a capture running, **press `p`**. The page runs the whole
sequence itself — A B C D E F G, 6.5 s each with 15 s of black between — and logs the start and end
of every clip, so the ordering and the gaps cannot be got wrong by hand. Press `p` again to abort.
Capture as `cap11-metadata-probes`, snaplen 4096 (this phase is about control messages, not pixels).

The numbers are chosen to be findable as little-endian `u16` in a byte stream: `605` = `0x025d`,
`4000` = `0x0fa0`, `1234` = `0x04d2`, `50` = `0x0032`. CTA-861 units — mastering max luminance is
whole cd/m², min is 0.0001 cd/m², MaxCLL/MaxFALL are whole cd/m², chromaticities are 0.00002.

⚠ **A null result here is a real result.** If all seven produce an identical control plane, then
Windows composites everything into its own scRGB surface and hands the dock only *output* metadata
derived from the monitor's EDID — in which case vino's job is to set `HDR_OUTPUT_METADATA` from the
sink's capabilities and never from the content. Write that down; it is the answer to a question we
would otherwise keep re-asking.

---

## 7. Capture H4 — the non-content axes

Short, snaplen 4096, one capture `cap12-axes`, 15 s idle between each step, all times logged:

1. HDR **off** → **on** → **off** on the socket-1 monitor, with a **static desktop** (nothing
   playing). Isolates the mode change from any content.
2. HDR on, then move the **SDR content brightness** slider from minimum to maximum and back.
   If the wire changes, Windows is compositing SDR into HDR host-side — and the *amount* it changes
   tells you the composition is happening before the dock, not in it.
3. Settings → System → Display → Advanced display: note the reported **Bit depth** and **Colour
   format** for a dock screen with HDR on and with it off. Screenshot both into `out\`.
4. If Windows 11 offers **Automatic colour management** for SDR on this build, toggle it on and off
   with the SDR pattern playing. That is wide-gamut-without-HDR — a third point that separates the
   gamut axis from the luminance axis.

---

## 7b. Capture H5 — bandwidth and both-heads-at-once

Two things phases 3–7 leave open, both cheap. One capture, `cap13-bandwidth`, snaplen 4096, logged:

1. **HDR on both dock monitors at the same time**, pattern playing on socket 1 while socket 3 shows
   a static desktop. A per-connector difference and a per-endpoint difference look identical in a
   single-connector capture, and a metadata message that is per-view rather than per-device only
   shows its hand when two views disagree. Then swap: HDR on socket 3 only.
2. **HDR on at 2560×1440 @ 120 Hz**, pattern playing, ~30 s. 10-bit needs more link bandwidth than
   8-bit at the same mode, so if the dock's pixel budget is depth-aware the set-mode words move —
   and vino's bandwidth manager would need to know. Put it back to 60 Hz afterwards
   (`.\tools\rescue-refresh.ps1 -Hz 60`).

⚠ Do **not** go above 120 Hz for this. 180 is what broke the dock in the first place.

## 8. Finish

In `out\`:

- `cap9-hdr-ab-*.pcap`, `cap10-hdr-ab-ep0a-*.pcap`, `cap11-metadata-probes-*.pcap`, `cap12-axes-*.pcap`
- `NOTES.md` — appended to, not replaced. Timestamped log for every capture, the `check-capture.py`
  output for each, which socket/connector each phase used, and the HDR toggle's behaviour.
- screenshots from phase 7.3
- `pnp-after.txt` and the `bcdDevice` value — say so loudly if it changed.

⭐ **Leave the dock at 60 Hz.** Do not put it back to 180.

The Linux side reads `/mnt/windows/Users/Mike/navarro-wincap/out/` directly after reboot, so nothing
needs copying back.

---

## Coverage — what a vino HDR path needs, and where it comes from

Everything an in-kernel DRM driver would have to get right, and which phase (if any) pins it down.

| what vino needs | where it comes from | covered? |
|---|---|---|
| Does the dock take >8-bit pixels at all (`NM30`)? | phase 3, `near_black` + `pq_ramp` decoded off the wire | ✅ |
| Does the *codec* change for 10-bit (strip grammar, quantiser tables)? | phase 3, `detail` + `pq_ramp` | ✅ |
| Is the gamut BT.709-limited or wide? | phase 3, `gamut_ab` — gamut moved with luminance held fixed | ✅ |
| Does above-SDR luminance reach the wire, or is it tone-mapped host-side? | phase 3, `grey100` vs `grey1000` vs the ~302 cd/m² panel peak | ✅ |
| Which CP message carries the EOTF / colorimetry selection? | phase 6 `probe-F` (HLG) and `probe-G` (BT.709 tags on identical pixels) | ✅ framing; ⚠ fields need keys |
| Which carries HDR10 static metadata, and what is its layout? | phase 6 A/B/C/E — one field moved at a time, values chosen to be findable as `u16` | ✅ framing; ⚠ fields need keys |
| Does the set-mode message change when HDR is on? | phase 7.1 — HDR toggled with a static desktop, same mode | ✅ |
| Is it a per-connector or per-device setting? | phase 7b.1 — one head HDR, one head SDR, simultaneously | ✅ |
| Does 10-bit change the link/bandwidth budget? | phase 7b.2 — HDR at 120 Hz | ✅ |
| Does the host composite SDR into HDR, or does the dock? | phase 7.2 — the SDR-brightness slider | ✅ |
| What values should `HDR_OUTPUT_METADATA` carry? | the **sink EDID**, not the content — already read, in `hdr-content\sink-edid\` | ✅ (from Linux) |
| Does the dock advertise HDR capability of its own? | the dock's `id=0x78 sub=0x30` DISPLAY-CAP push — **Linux side, with keys** | ⛔ not this session |
| Does HDR change gamma/CTM programming? | not exercised — vino's software CTM+GAMMA_LUT path is untested under HDR | ⛔ gap, noted |
| HDR10+ / Dolby Vision dynamic metadata | out of scope — DisplayLink is HDR10 static only | n/a |

Two things are deliberately **not** in this session and should not be improvised into it: extracting
session keys from `WUDFHost.exe`, and anything about the dock's own capability push. Both are better
done from Linux, where the key method already works.

## What is out of reach here, and why that is fine

**There is no user-mode DisplayLink process on Windows to hook** — the driver is `dlidusb4.dll`
inside `WUDFHost.exe` — so the 2026-08-02 session could not lift session keys, and neither will this
one without a deliberate WUDFHost attach. That means the sealed control plane stays sealed.

That limits phase 6 to *framing*: message id, sub, length and timing, not decoded fields. It is
still decisive, because the probes are byte-identical in every other respect — **if a message
appears, or changes length, only when the metadata changes, that message is the metadata message**,
and the Linux side can then go looking for it with keys.

The pixel path is not limited at all: Navarro's video records and their headers are **plaintext**,
so phases 3–5 are fully decodable from the wire alone.
