# revdi design notes

revdi has two halves that talk over a private DRM ioctl ABI, exactly like the
upstream C evdi:

```
  DisplayLinkManager (proprietary)
        │  evdi_lib.h C ABI  (dlopen libevdi.so.1)
        ▼
  library/  librevdi  (Rust cdylib)
        │  evdi_drm.h ioctls + drm_event stream  (/dev/dri/cardN)
        ▼
  module/   evdi.ko    (Rust DRM/KMS driver)
        │  atomic KMS, virtual connector, GEM-shmem dumb buffers
        ▼
  the compositor (KWin/Wayland, X, …) renders into the virtual output
```

The compositor renders a normal DRM output; the driver captures each committed
frame and the userspace client (DLM) pulls the pixels out and sends them over
USB to the dock. Nothing is scanned out to local hardware.

## Kernel module (`module/`, GPL-2.0)

Single-file crate rooted at `evdi.rs`, with submodules:

| File | Responsibility |
|------|----------------|
| `evdi.rs` | Platform driver + the `/sys/devices/evdi` control device: `add`/`remove_all`, on-demand card creation, and USB-dock pairing via a `usb_register_notify` hook so a card is destroyed when its dock is unplugged. `module!` entry point (registers as `evdi`). |
| `kms.rs` | The DRM driver: `INFO`, `EvdiDrmData`, atomic-KMS output (CRTC + primary + cursor plane + virtual connector), EDID-driven mode list, and the `atomic_update` hook that records damage and signals the client. Declares the driver-private ioctls. |
| `ioctl.rs` | The five libevdi ioctls: `CONNECT`, `REQUEST_UPDATE`, `GRABPIX` (damage-tracked pixel readback), `DDCCI_RESPONSE`, `ENABLE_CURSOR_EVENTS`. |
| `painter.rs` | Per-device connection/event state and the `drm_event` senders (UPDATE_READY, DPMS, MODE_CHANGED, CRTC_STATE, CURSOR_SET/MOVE, DDCCI_DATA); the `Damage` accumulator. |
| `ddcci.rs` | The in-kernel I2C/DDC-CI adapter bridged out to the client. |
| `uapi.rs` | `#[repr(C)]` mirror of `evdi_drm.h`, pinned by `size_of` assertions. |

### Frame path and damage

On every atomic flip, `EvdiPlane::atomic_update` walks the plane's
`FB_DAMAGE_CLIPS` (via the new `for_each_damage_clip` binding), pushes each
changed rectangle into a bounded `Damage` set (coalescing to a bounding box on
overflow), marks the frame dirty, and sends `UPDATE_READY`. `GRABPIX` then reads
the scanout framebuffer through a safe CPU vmap and copies out **only** the
accumulated rectangles, reporting them back as `drm_clip_rect`s so DLM transmits
just the deltas. This is what keeps a 2560×1440 output responsive over USB.

### Binding work this driver required

The driver is the first virtual `KmsDriver`, and shook out four fixes/helpers in
the shared safe KMS layer (`rust/kernel/drm/…`), shipped as patch 1 of the
kernel-tree series (`patches/revdi/`):

* **`MODE_CONFIG_OPS` as `&'static`** — the mode-config vtable was installed from
  a stack local in `probe_kms`, dangling after return; the DRM core later jumped
  through a garbage `mode_valid` slot (RIP=0x9). Now a const-promoted `'static`.
* **CPU-access coherency** — the framebuffer vmap now brackets reads with
  `drm_gem_fb_begin/end_cpu_access`, so `GRABPIX` never sees a stale cached frame
  (the "stuck on two frames" bug).
* **`for_each_damage_clip`** — iterate individual damage clips, not just their
  merged bounding box.
* **`drm_atomic_helper_shutdown` at teardown** — disable the pipeline before
  `drm_dev_unplug`, so connectors aren't left pinned (the `mode_config_cleanup`
  "leaked connector" warning).

## Library (`library/`, LGPL-2.1-or-later)

`librevdi` is a Rust `cdylib` that exports the exact `evdi_lib.h` symbol set and
installs as `libevdi.so.1` (SONAME set in `build.rs`), so DLM loads it in place
of the C library. It is a faithful port of upstream `evdi_lib.c` /
`evdi_procfs.c`:

* device discovery/creation under `/sys/bus/platform/devices` and
  `/sys/devices/evdi/add`, including the `usb:<syspath>` dock-attach form;
* DRM-master drop / slave-open handshake (and the `Xorg_running` wait);
* the ioctl wrappers, buffer registry, `GRABPIX` dirty-rect plumbing;
* the `drm_event` reader in `evdi_handle_events` that demultiplexes to the
  client's callbacks, including mapping the cursor dumb buffer out for
  `CURSOR_SET`.

Only the handful of libc entry points with no ergonomic `std` equivalent (raw
`ioctl`/`mmap` on the DRM fd) are declared in `ffi.rs`; everything else uses
`std`. The `uapi.rs` layouts are asserted byte-identical to the module's.
