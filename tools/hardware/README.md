# Hardware tools

For the manual validation described in
[`docs/testing.md`](../../docs/testing.md#hardware-boundary).

| Script | Purpose |
|---|---|
| `vino-cycle.sh` | Reload the module without physically unplugging: unbind, wait for clients to let go, unload, load, rebind. |
| `drm-fd-holders.py` | Lists processes holding a DRM node open. Used by the above; useful alone before any unload. |
| `drm-setmode.py` | Set a mode on one connector and hold it, with no libdrm, no modetest and no compositor. Answers "does this panel light up" without it becoming a KWin question. |
| `vino-perf.py` | Per-head frames/s, MB/s, frame-interval distribution and machine CPU, from usbmon URB bursts. |
| `vino-bringup-trials.sh` | Repeat a **cold** bring-up N times and report, per head, whether pixels actually flowed. One trial proves nothing on this dock, so a claim about bring-up should always be a count. Resets by USB re-authorise, not an interface unbind, because an unbind does not make the dock re-run its downstream sink discovery. |
| `vino-edid-override.sh` | Describe a head's sink with an EDID read elsewhere, when a converter's broken DDC stops the dock reading it. See [`docs/device.md`](../../docs/device.md#sinks-the-dock-cannot-read). Blobs in `edid/`. |

## Installing a rebuilt module

```sh
make -C ../../linux LLVM=1 -j16 modules && sudo make -C ../../linux modules_install && sudo depmod -a
```

⛔ **Never add `M=` to a `modules_install` line** — it installs into `updates/`, which `depmod`
prefers over `kernel/`, so the stray copy wins and later reinstalls appear to do nothing. Check with
`ls /lib/modules/$(uname -r)/updates/` (must not exist) and `modinfo -n vino` (must be under
`kernel/...`).

⚠ `vino-cycle.sh` reloads whatever is **installed**, not what is merely built. Verify the resident
module, not the file: hashing `$(modinfo -n vino)` tells you about the file on disk, which can differ
from what is loaded in memory.

## Reloading safely

⚠ **Unloading while a DRM file is open frees the fops under the compositor and hangs the machine.**
`vino-cycle.sh` exists to make that impossible: USB unbind runs the driver's `disconnect()`, which
calls `drm_dev_unplug()`, so clients get `-ENODEV` and close of their own accord. Only once nothing
holds the card does it unload. It refuses rather than forcing.

⚠ The module refcount trails the last file close by a moment, so `modprobe -r` immediately after
reports "Module vino is in use". The script polls for it.

```sh
sudo tools/hardware/vino-cycle.sh              # unbind, unload, load, rebind
sudo tools/hardware/vino-cycle.sh --unload     # leave unloaded, for DLM/evdi work
```

## Measuring

```sh
sudo modprobe usbmon                           # NOT autoloaded
sudo tools/hardware/vino-perf.py --secs 30
```

⚠ Compare runs only against the same `--load` setting, on an otherwise-quiet desktop.

⚠ Judge absolute cost from the **machine busy** figure, which comes from `/proc/stat`. The
per-kworker line is a **lower bound**: vino's scanout worker runs on the shared `system()` queue,
whose kworkers are named `-events` and cannot be attributed to a driver. An earlier version of this
script matched every `events_unbound` kworker and so billed unrelated subsystems to vino, reporting
641% where the real cost was ~267%.

## Counting bring-ups

```sh
sudo LOGDIR=$HOME tools/hardware/vino-bringup-trials.sh 5
```

⚠ The verdict is **bytes under forced damage**, not dmesg and not "frame ok": a static desktop
legitimately sends nothing, so an idle head and a jammed one are indistinguishable until damage is
forced. And bytes still are not "lit" -- this dock will accept a complete, correct frame and never
start its downstream pixel clock. What the script measures is whether the dock kept *accepting*
video; a person still has to look at the panel.

⭐ A failing trial writes its `dmesg` to `$LOGDIR/vino-trial-fail-*.log`. The script clears the log
each trial to count frames, and these failures are intermittent -- roughly one bring-up in five at
the time of writing -- so without that a failure is a number with nothing behind it.
