# Hardware tools

For the manual validation described in
[`docs/testing.md`](../../docs/testing.md#hardware-boundary).

| Script | Purpose |
|---|---|
| `vino-cycle.sh` | Reload the module without physically unplugging: unbind, wait for clients to let go, unload, load, rebind. |
| `drm-fd-holders.py` | Lists processes holding a DRM node open. Used by the above; useful alone before any unload. |
| `vino-perf.py` | Per-head frames/s, MB/s, frame-interval distribution and machine CPU, from usbmon URB bursts. |

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
