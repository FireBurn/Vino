# Windows capture runbook — WAVLINK DL7400 (Navarro, `17e9:7000`)

**Read this file, then do it.** It is written to be executed with minimal further prompting. Work
top to bottom; each phase ends with a check you must actually run before moving on.

You are on Windows. The Linux side of this project (an in-kernel Rust DRM driver called `vino` that
replaces DisplayLink's binary driver) already has this dock working for control, EDID, modes and
hotplug, but **not video** — vino cannot yet put a picture on a DL7400. The point of this session is
to record Windows' DisplayLink driver doing it, because Windows is a third independent
implementation and its bytes are readable (see "Why wire-only is enough" below).

Everything you produce goes in `C:\Users\Mike\navarro-wincap\out\`. Do not analyse deeply here —
capture well, verify the capture is good, and leave it for the Linux side.

---

## 0. Ground truth about the environment (verified from Linux, 2026-08-02)

| thing | state |
|---|---|
| Wireshark | installed, `C:\Program Files\Wireshark\` (`tshark.exe`, `dumpcap.exe`, `capinfos.exe`) |
| USBPcap | installed, `C:\Program Files\USBPcap\USBPcapCMD.exe` |
| Python | installed, `C:\Program Files\Python312\python.exe` |
| **DisplayLink driver** | **NOT INSTALLED** — phase 1 installs it |

The dock as it currently stands: `17e9:7000`, `bcdDevice = 3922`, SuperSpeed+ 10 Gbps, identity tail
`NavaDock`, serial `754D3380000000209`. **Four DisplayPort sockets, two monitors available.**

---

## 1. Install the DisplayLink Windows driver — but capture the first plug

⚠ **The dock is unplugged for this phase. Do not plug it in until phase 3 says so.** The first
contact between a freshly installed DisplayLink driver and this dock can trigger a **USB DFU
firmware flash**, which happens exactly once and is worth having on tape.

1. Download "DisplayLink USB Graphics Software for Windows" from
   <https://www.synaptics.com/products/displaylink-graphics/downloads/windows>.
2. **Record the version you downloaded** into `out\NOTES.md`. The Linux side runs DisplayLink
   **6.8.1.0** (`DisplayLinkManager` 3.4.26). If the Windows version differs, say so — a newer
   Windows package may flash the dock to a firmware the Linux driver then wants to change back.
3. Install it. Reboot if it asks.
4. Do **not** plug the dock in yet.

---

## 2. Find the USBPcap interface for the port you will use

USBPcap captures a **whole root hub**, not a device, so you must pick the right one. Run:

```
"C:\Program Files\USBPcap\USBPcapCMD.exe" --extcap-interfaces
```

That lists `\\.\USBPcap1`, `\\.\USBPcap2`, … Then, for each, list the devices currently on it:

```
"C:\Program Files\USBPcap\USBPcapCMD.exe" -d \\.\USBPcap1 --extcap-config
```

⚠ You cannot tell which hub the dock will land on while it is unplugged. Two options, in order of
preference:

- **Preferred:** briefly plug the dock in, note which root hub gains a `17e9` device
  (`Get-PnpDevice -PresentOnly | Where-Object InstanceId -match 'VID_17E9'`), unplug it again, and
  capture that hub. You lose the very first enumeration but you get the right hub with certainty —
  and a firmware flash, if there is one, is triggered by the *driver*, so it will still occur on the
  captured plug in phase 3 if it did not already occur here. **Record in `out\NOTES.md` whether the
  dock had already been plugged in at this point**, because it changes how to read a missing flash.
- **Fallback:** capture *every* `USBPcap` interface at once, one process per interface. Cheap, and
  it cannot miss.

Write the chosen interface(s) into `out\NOTES.md`.

---

## 3. Capture 1 — the important one (bring-up, hotplug, port moves)

This one is **snaplen-limited on purpose**. A lit DL7400 streams video at hundreds of MB/s, and a
full-payload capture of a long session reaches tens of gigabytes (measured on Linux: 50 GB in about
three minutes). Everything phase 3 is trying to learn lives in the first few hundred bytes of each
transfer, so cap it:

```
mkdir C:\Users\Mike\navarro-wincap\out
"C:\Program Files\USBPcap\USBPcapCMD.exe" -d \\.\USBPcapN -o C:\Users\Mike\navarro-wincap\out\cap1.pcap -s 4096 -A
```

`-s 4096` keeps whole control-plane frames (the largest seen is 1056 B) and the header of every video
record, while discarding the pixel bulk. `-A` captures all devices on the hub.

**Leave it running for the whole of the following choreography.** Keep a log of wall-clock times in
`out\NOTES.md` as you go — one line per step, `HH:MM:SS  what you did`. That is what makes the
capture sliceable afterwards; without it the file is an undifferentiated wall of frames.

With **both monitors in sockets 1 and 2**:

1. `idle-before` — wait 15 s, touch nothing.
2. `plug-dock` — plug the dock in. Wait until Windows finishes installing and both screens light.
   **If a firmware flash starts, do not unplug anything.** Wait it out.
3. `settle` — 20 s, leave alone.
4. `drag-screen1` — drag a window around vigorously on the socket-1 monitor for 10 s, then stop for 10 s.
5. `drag-screen2` — same on the socket-2 monitor, 10 s, then stop for 10 s.
6. `move-1-to-3` — unplug the cable from socket 1, wait 10 s, plug it into socket 3, wait 20 s.
7. `move-2-to-4` — unplug the cable from socket 2, wait 10 s, plug it into socket 4, wait 20 s.
8. `drag-both` — drag a window across both dock screens for 15 s.
9. `idle-after` — 20 s, touch nothing.

Then stop the capture (Ctrl-C).

⚠ Steps 4 and 5 matter more than they look: driving **one screen at a time** is what attributes a
video endpoint to a specific connector. Do not skip them, and do leave the still gaps — the silence
is as informative as the traffic.

---

## 4. Verify capture 1 BEFORE going any further

```
"C:\Program Files\Python312\python.exe" C:\Users\Mike\navarro-wincap\check-capture.py C:\Users\Mike\navarro-wincap\out\cap1.pcap
```

This prints an endpoint tally and a connector-tag histogram, and ends with `VERDICT:` lines. What
you need to see:

- traffic on the dock's **`0x02`** (control OUT) and **`0x84`** (control IN);
- **real video volume on `0x08` and/or `0x0a`**;
- **connector tags** in the video records — `sub = connector << 3`, so `0x00`, `0x08`, `0x10`, `0x18`
  for connectors 0..3. Connector index is **physical socket number − 1**.

If there is no video on `0x08`/`0x0a`, the screens were not actually being driven — **redo phase 3**.
A control-plane-only capture is a wasted run; on Linux this exact failure mode happened twice and
looked completely healthy from the driver's side.

Paste the tool's output into `out\NOTES.md`.

---

## 5. Capture 2 — full payload, short (for the codec)

Only after capture 1 verifies. This one gets **complete pixel bytes**, which is the part vino cannot
currently produce at all.

```
"C:\Program Files\USBPcap\USBPcapCMD.exe" -d \\.\USBPcapN -o C:\Users\Mike\navarro-wincap\out\cap2-full.pcap -s 0 -A
```

Run it for **no more than 30 seconds**, and during that time:

1. Put something with large flat colour areas and sharp edges on a dock screen — a maximised
   solid-colour window, or a browser on a plain page. State exactly what was on screen, and which
   socket, in `out\NOTES.md`. A screenshot saved as `out\screen-ref.png` is even better: it gives the
   Linux side a reference image to decode the captured pixels against.
2. Leave the pointer still.
3. Stop after 30 s. **Check the file size** — if it is over ~20 GB something is wrong; note it and
   move on.

---

## 6. Optional stretch — session keys

Skip unless everything above is done and verified.

The dock's control plane is AES-CTR sealed. On Linux the keys are lifted live with frida from
`DisplayLinkManager`'s AES core. **The Linux offsets are useless here** — that is an ELF build and
Windows ships a different PE, so a hook at the Linux address reads garbage and yields a silently
keyless capture. Do **not** guess offsets.

If you want to try: `pip install frida-tools`, find the DisplayLink user-mode process, and locate the
AES block function in the PE first. Treat any key you get as unverified until it decrypts a frame.
This is genuinely optional — see below for why the capture is valuable without it.

---

## 7. Why wire-only is enough here

On this platform the interesting parts are **not** encrypted:

- **The video record header is plaintext.** `type` and `sub` sit at byte offsets 4..8 and 8..10 of
  every bulk-OUT transfer, unsealed. That is how connector attribution works without any key.
- **Navarro's pixels are plaintext too.** Unlike the older D6000, the DL7400's frame payload is not
  sealed — only the short per-stream "stream-open" message is. So a full-payload capture is directly
  decodable image data.

What is lost without keys is the sealed control plane: set-mode, EDID and the setup burst. Those are
already well understood from the Linux side, so they are not what this run is for.

---

## 8. Finish

Leave everything in `C:\Users\Mike\navarro-wincap\out\`:

- `cap1.pcap`, `cap2-full.pcap`
- `NOTES.md` — driver version, chosen USBPcap interface, timestamped choreography log, the
  `check-capture.py` output, what was on screen for capture 2, and anything surprising
- `screen-ref.png` if you took one
- `pnp-before.txt` / `pnp-after.txt` from:
  `Get-PnpDevice -PresentOnly | Where-Object InstanceId -match 'VID_17E9' | Format-List * > out\pnp-after.txt`

⭐ **Report the `bcdDevice` before and after.** It is `3922` right now. If it changed, the Windows
driver flashed the dock's firmware, and that is a significant finding in its own right — say so
loudly at the top of `NOTES.md`.

The Linux side reads `/mnt/windows/Users/Mike/navarro-wincap/out/` directly after reboot, so nothing
needs copying back.
