# Hardware tools

For the manual validation described in
[`docs/testing.md`](../../docs/testing.md#hardware-boundary).

| Script | Purpose |
|---|---|
| `vino-cycle.sh` | Reload the module without physically unplugging: unbind, wait for clients to let go, unload, load, rebind. |
| `drm-fd-holders.py` | Lists processes holding a DRM node open. Used by the above; useful alone before any unload. |
| `vino-perf.py` | Per-head frames/s, MB/s, frame-interval distribution and machine CPU, from usbmon URB bursts. |

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
