# TV runbook — settle the 10-bit AC codebook ceiling

**One question, one capture.** Everything else in this directory is context.

The DL7400's 10-bit **DC** escape ceiling is settled at 12. The **AC** ceilings are not, and vino
currently ships the 8-bit values because guessing upward desynchronises the dock while guessing
downward only clips detail. This capture settles them.

⏱ About 25 minutes including setup. The capture itself is 3 minutes.

---

## Why the last attempt failed, so it is not repeated

`cap9` produced **zero** AC coefficients over 3181 strips. Not a small number — zero. Its
codec-stress segment used a **16 px** checkerboard, and the codec transforms **8 px** blocks, so
every block sat entirely inside one cell and was perfectly flat.

⛔ **It was not the monitor's brightness.** That was the first guess and a forward model of the
codec says it was wrong: a **2 px grating** demands a category-11 coefficient on *any* HDR sink,
including the 302 cd/m² panels already on the desk. The TV adds headroom and a 4K timing; it does
not make the measurement possible. If the TV fights you at any point, **phase 5 runs the same
experiment on the MSI panels and is just as valid.**

## What "settled" looks like

`hdr-content\ac\` holds twelve pictures designed against the codec's actual block size, in an HDR
(PQ / BT.2020 / 10-bit) version and a **pixel-matched SDR twin**. The pair is the whole experiment:

| | max luma AC the picture demands | 8-bit codebook ceiling |
|---|---|---|
| `ac-hdr.webm`, segment 01 | **1538 — category 11** | — |
| `ac-sdr.webm`, segment 01 | 510 — category 9 | 9 |

Verified against the real encoded files, not just intended. So afterwards:

* if the HDR half carries coefficients above **511** and its strips still decode coherently, the
  AC ceiling scales with depth;
* if its largest magnitudes **pile up at exactly 511**, it does not, and vino's current conservative
  choice is correct.

Either answer closes the question. A null result is a result here.

---

## Phase 0 — physical and preconditions

**Everything in this phase is a gate. Do not start capturing with any of it unresolved.**

1. **The TV must be on a dock socket, not the laptop.** Note which socket number — connector index
   is socket − 1, and `ep 0x08` carries connectors 0 and 2 while `ep 0x0a` carries 1 and 3.

2. ⚠ **If the TV is on a DP→HDMI adapter, it must be an *active* one supporting HDMI 2.0 or
   later.** A passive adapter is HDMI 1.4: 4K30, 8-bit, no HDR — and it will fail quietly by simply
   never offering the HDR toggle.

3. ⚠ **Turn on the TV's own deep-colour setting for that input.** It is off by default on most
   sets and is the single most common reason Windows shows no HDR switch. It is called
   *HDMI UHD Colour* (Samsung), *HDMI Ultra HD Deep Colour* (LG), *HDMI Signal Format → Enhanced*
   (Sony), *HDMI Mode → 2.0* (others). Set it for the input the dock is on, then replug.

4. **Mode: 3840×2160 @ 60 Hz.**
   ```
   powershell -ExecutionPolicy Bypass -File .\tools\rescue-refresh.ps1 -List
   powershell -ExecutionPolicy Bypass -File .\tools\rescue-refresh.ps1 -Device '\\.\DISPLAYnn' -Width 3840 -Height 2160 -Hz 60
   ```
   If 4K60 will not hold, 4K30 is fine — this experiment does not care about refresh. What it
   cannot tolerate is a resolution that is not the panel's own.

5. ⛔ **Scaling on that display must be 100%.** Windows defaults a 4K TV to 300%. At anything else
   the 2 px grating is resampled into mush and the capture answers nothing.
   ```
   powershell -ExecutionPolicy Bypass -File .\tools\dpiscale.ps1 -List
   powershell -ExecutionPolicy Bypass -File .\tools\dpiscale.ps1 -Display '\\.\DISPLAYnn' -Percent 100
   ```

6. **HDR on for that display**, and read it back rather than trusting the Settings toggle:
   ```
   powershell -ExecutionPolicy Bypass -File .\tools\hdr.ps1 -List
   powershell -ExecutionPolicy Bypass -File .\tools\hdr.ps1 -Display '\\.\DISPLAYnn' -On
   ```
   It must report `bpc 10`. If it reports `bpc 8`, go back to steps 2 and 3.

7. **Record the sink.** Thirty seconds, and without it a decoded value cannot be turned back into
   a luminance:
   ```
   powershell -ExecutionPolicy Bypass -File .\tools\edid.ps1 -OutDir .\out\edid-tv
   ```
   Note the TV's declared peak from the output. Keep the blobs — `edid-decode` on Linux reads far
   more of them.

## Phase 1 — bring the harness up

```
:: elevated, once for the whole session -- one UAC prompt, not one per capture
powershell -ExecutionPolicy Bypass -File .\tools\capture-runner.ps1

:: Edge on the TV, with the debugging port the choreographer drives
"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe" ^
  --remote-debugging-port=9222 --user-data-dir=%TEMP%\edgecap ^
  --new-window "file:///C:/Users/Mike/navarro-wincap/hdr-content/player.html"
```

Drag the window to the TV, press `f` for fullscreen, then `i` and confirm **all four**:

```
devicePixelRatio  1
1:1 mapping       yes
dynamic-range     high
screen depth      30
```

Press `9`. The AC-stress clip should play; the picture will look like fine vertical stripes.
⚠ If it looks like flat grey, the browser is scaling it — go back to phase 0 step 5.

## Phase 2 — the capture

```
powershell -ExecutionPolicy Bypass -File .\tools\choreograph.ps1 -Phase ac ^
    -Head '\\.\DISPLAYnn' -Other '\\.\DISPLAYmm'
```

That is the whole thing. It refuses to start if scaling or 1:1 mapping is wrong, toggles HDR
itself, plays the HDR clip and then the pixel-matched SDR twin, and timestamps every step into
`out\cap16-ac-ceiling.phaselog.txt`.

⚠ **Full payload, and these are the worst-case pictures for this codec.** Leave **20 GB** free.
If the disk is tight, `-MaxSeconds 120` still covers both halves of segments 00–05, which includes
the decisive ones.

## Phase 3 — verify before you reboot

```
python check-capture.py out\cap16-ac-ceiling-usbpcap1.pcap
```

It must show video bytes on the TV's endpoint. A capture with a healthy control plane and no video
is a real, repeated failure mode that is invisible from the desktop — it has cost three runs.

Then the one check that is specific to this experiment: in the phase log, confirm
`after HDR ON` reported `"hi":true` and `"depth":30`. If either is wrong the run is void and it is
quicker to redo it now than to discover it from Linux.

## Phase 4 — notes

Append to `out\NOTES.md` (never replace it):

* which socket the TV is on, and the connector index that implies
* the TV's model and its declared peak from phase 0 step 7
* the mode actually achieved, and whether 4K60 held
* whether the adapter was active or direct DP
* anything the TV's own picture menu was set to

## Phase 5 — optional cross-check, and the fallback

The same experiment on an MSI panel at 2560×1440. Worth doing either way — two sinks with very
different declared peaks either agree about the ceiling or they do not, and that is informative.
**If the TV cannot be made to do HDR at all, do this instead and the session still succeeds.**

The clips are 4K, so on a 1440p panel they would be resampled. Regenerate at the panel's size
first, on Linux:

```sh
scratchpad/venv-np/bin/python tools/make-ac-patterns.py --out hdr-content/ac1440 --size 1440p
```

then point the player's sources at `ac1440/` and repeat phase 2 as `-OutPrefix out\cap17-ac-1440p`.

## Phase 6 — leave it tidy

* Dock heads back to **2560×1440 @ 60 Hz**, HDR as you found it.
* ⚠ The **SDR content brightness slider** on `DISPLAY29` was left at minimum by the last session
  and has no public setter. Settings → System → Display → HDR → SDR content brightness; it was at
  3000 (~240 nits) before.
* ⚠ **Power settings from the last session are still changed:** monitor-timeout-ac/dc → 0,
  standby-timeout-ac → 0, `ScreenSaveActive` → 0. Originals: AC 300 s, DC 180 s, `ScreenSaveActive` 1.
* Leave `bcdDevice` alone — it should still read **3922**. If a DFU transfer ever starts, let it
  finish.

---

## If something goes wrong

| symptom | cause | do |
|---|---|---|
| no HDR toggle for the TV | passive adapter, or the TV's deep-colour setting is off | phase 0 steps 2–3 |
| `hdr.ps1` reports `bpc 8` with HDR on | link cannot carry 10-bit at this mode | try 4K30, or 1080p60 |
| `1:1 mapping no` | display scaling | phase 0 step 5 |
| picture looks flat grey, not striped | the same, or the browser zoomed — press `Ctrl+0` | phase 0 step 5 |
| dock re-enumerates repeatedly | bandwidth; the other head is competing | unplug the other monitor for this capture |
| capture is enormous | expected — worst-case content | `-MaxSeconds 120` |
| `check8_control` decodes with AC | the picture was resampled after all | the run is void; fix scaling and redo |

The last row is worth understanding: segment 08 is an 8 px checkerboard aligned to the codec's
block grid, so a correctly delivered picture makes **no** AC there at all. It is a self-check built
into the content, and it is more trustworthy than any setting you can read back.
