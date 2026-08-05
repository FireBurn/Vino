# Navarro Windows capture — session entry point

**Your task this session: execute `TV-RUNBOOK.md` in this directory, top to bottom.** Read it
first, then do it. It is written to need no further prompting, and it is short — one capture,
about 25 minutes.

One-line summary: a 4K HDR TV is now on the WAVLINK DL7400 dock (`17e9:7000`). Play content that
varies **inside** the codec's 8×8 block and capture it in HDR and again in SDR, so the Linux side
can settle the last open number in the 10-bit wire format: the AC escape codebook's ceiling.

⛔ **`HDR-RUNBOOK.md` is DONE — do not redo it.** Its captures are in `out\` and its result is
written up on the Linux side. The one thing it could not settle is what `TV-RUNBOOK.md` settles.

## Why the last session could not answer this

Its codec-stress segment used a **16 px** checkerboard. The codec transforms **8 px** blocks, so
every block sat inside one cell and was perfectly flat: 3181 strips, **zero** AC coefficients.

⛔ It was blamed on the monitors being too dim. That was wrong — a forward model of the codec says
a 2 px grating demands a category-11 coefficient on any HDR sink, including the 302 cd/m² panels.
So if the TV fights you, **phase 5 of the TV runbook runs the same experiment on the MSI panels and
is just as valid.**

## ⚠ Refresh rate

180 Hz on a dock head puts it into a reconnect loop, and Windows persists refresh per monitor. The
rescue is still here if you need it — run it from the **laptop's own screen**:

```
powershell -ExecutionPolicy Bypass -File .\tools\rescue-refresh.ps1 -List
powershell -ExecutionPolicy Bypass -File .\tools\rescue-refresh.ps1 -Hz 60
```

## Read in this order

| file | what it is |
|---|---|
| `TV-RUNBOOK.md` | **the instructions for this session** — six short phases, do them in order |
| `hdr-content\ac\manifest.json` | what each of the twelve AC-stress pictures is, and the AC category it demands |
| `HDR-RUNBOOK.md` | the **previous** session's runbook (2026-08-05 am) — done, for context only |
| `hdr-content\SEGMENTS.md` | what is in that session's test content |
| `RUNBOOK.md` | the 2026-08-02 runbook, for context only — do not redo it |
| `out\NOTES.md` | what that session found. Append to it; never replace it |
| `NAVARRO-PROTOCOL.md` | what Linux already knows; how to read a video record with no key |
| `navarro-full.md` | the complete Linux write-up, if you need depth |
| `check-capture.py` | run this to verify a capture before rebooting |

## What is already done, and must not be redone

The 2026-08-02 session captured bring-up, hotplug, the port map, a codec reference and a 25-mode
sweep. All of it is understood on the Linux side. In particular:

- `sub = connector << 3` for frame records, and connector index = physical socket − 1. Settled.
- No firmware flash; `bcdDevice` stayed 3922.
- The mode ceiling per resolution is known from `cap4-modesweep`.

⛔ **The one thing that session got wrong**, and the reason this one exists: `cap6` (HDR on) and
`cap7` (HDR off) played **the same SDR animation**. Windows was in HDR *mode* with nothing of HDR
*range* or wide gamut ever on screen, so an identical wire was the expected result either way — the
comparison could not have detected a difference even if the dock has a full 10-bit path. The
follow-on conclusion that "HDR is host-side, nothing for vino to do" rests on a bandwidth
comparison, not a format one, and does not stand.

## Non-negotiables

- ⛔ **Verify every capture with `check-capture.py` before rebooting out of Windows.** A capture with
  a healthy-looking control plane and zero video bytes is a real, repeated failure mode that is
  invisible from the desktop. It has cost three runs now.
- ⛔ **`-BufferLen 134217728` on every full-payload capture.** USBPcap's default 1 MB kernel ring
  silently drops almost everything from a lit dock.
- ⚠ **Check `player.html`'s info panel before each HDR capture.** It must say `dynamic-range high`,
  `devicePixelRatio 1` and `1:1 mapping yes`. A capture taken through a window Edge thinks is SDR,
  or of a picture that display scaling resampled, is a wasted run.
- ⛔ **100% display scaling on the TV.** Windows defaults a 4K set to 300%, and at anything but 100%
  the 2 px grating this session depends on is resampled into mush. `check8_control` (segment 08) is
  the built-in detector: delivered 1:1 it makes NO AC coefficients at all.
- ⚠ **Capture the whole root hub, not one device.** The dock re-enumerates on replug and takes a new
  USB address each time; a device-filtered capture loses exactly the moments worth having.
- ⚠ **Snaplen matters.** A lit dock streams hundreds of MB/s — 50 GB in three minutes was measured.
  Full payload for the codec captures, `-s 4096` for the control-plane ones.
- ⚠ Keep a timestamped log of what you physically did in `out\NOTES.md`. `player.html` keeps one for
  you — press `l` and copy it out. Without it a capture is an undifferentiated wall of frames.
- ⛔ **Never unplug anything during a firmware flash.** If `bcdDevice` starts changing or a DFU
  transfer appears, let it finish.

## Things that are NOT your job

- Don't try to decrypt the control plane. There is **no user-mode DisplayLink process on Windows** —
  the driver is `dlidusb4.dll` inside `WUDFHost.exe` — so the frida method that works on Linux does
  not apply, and the Linux ELF offsets are useless against a PE. The capture is valuable without
  keys: Navarro's pixels and record headers are plaintext, and the metadata probes are designed to
  be decisive on **framing alone** — a message that appears, or changes length, only when the
  metadata changes is the metadata message.
- Don't analyse deeply. Capture well, verify, write notes. The Linux side reads
  `/mnt/windows/Users/Mike/navarro-wincap/out/` directly after reboot; nothing needs copying back.
- Don't touch `C:\Users\Mike\dl-scripts\` — that is a stale June checkout from the older D6000 work
  and its assumptions do not apply to this dock.

## Layout

```
CLAUDE.md            this file
HDR-RUNBOOK.md       this session's instructions
RUNBOOK.md           the previous session's, for context
check-capture.py     capture verifier -- run it before you reboot
hdr-content\         the test content (generated on Linux, 2026-08-05)
  player.html          open this in Edge; it drives the whole session
  SEGMENTS.md          what every picture contains
  manifest.json        the machine-readable version
  hdr-pattern.{webm,mp4}  sdr-pattern.{webm,mp4}
  hdr-motion.{webm,mp4}   sdr-motion.{webm,mp4}
  probes\              7 clips, identical pixels, different HDR10 metadata
  ref\                 the exact source pictures
  ref\decoded\         the same after the codec -- compare wire output against THESE
  sink-edid\           both dock monitors' EDIDs + decodes, read from Linux 2026-08-05
  ac\                  ⭐ THIS session: 4K AC-stress content, HDR + pixel-matched SDR twin
    ac-hdr.{webm,mp4}    12 segments, PQ/BT.2020 10-bit, 3840x2160
    ac-sdr.{webm,mp4}    the same twelve pictures in BT.709 8-bit
    manifest.json        per-segment description and expected AC category
    ref\                 source crops
tools\
  rescue-refresh.ps1   phase 0: get the dock off 180 Hz
  capture-both.ps1     capture every USBPcap root hub at once
  displaymode.ps1      read/set modes
  mode-sweep.ps1       the previous session's mode sweeper
  test-pattern.ps1     the previous session's SDR test pattern
  animate.ps1          simple moving-block damage generator
  phase-tags.py        slice a capture by phase
  make-hdr-patterns.py the generator for hdr-content\ (needs Linux + ffmpeg)
  make-ac-patterns.py  the generator for hdr-content\ac\ -- read its header, it explains the design
  hdr.ps1              read/set per-display HDR; prints bit depth and colour encoding
  dpiscale.ps1         read/set per-display scaling -- 100% is mandatory for this session
  cdp.ps1              drive/interrogate the player over the DevTools protocol
  choreograph.ps1      run a whole capture phase with every step timestamped (-Phase ac)
  capture-runner.ps1   one elevated capture process for the session
  sdr-whitelevel.ps1   poll the SDR-content-brightness level (no public setter)
  edid.ps1             ⭐ dump every monitor's EDID and decode its declared HDR peak
out\                 everything you produce, plus the 2026-08-02 captures
```
