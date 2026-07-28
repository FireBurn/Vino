# revdi

A from-scratch **Rust** reimplementation of DisplayLink's
[`evdi`](https://github.com/DisplayLink/evdi) — both the kernel module *and* the
userspace library — targeting DisplayLink DL3 docks (e.g. the Dell D6000). It is
a drop-in replacement for the stock C `evdi` + `libevdi`: the closed-source
DisplayLinkManager (DLM) loads it unchanged.

The module is written against the in-progress
[Rust DRM/KMS bindings](https://lore.kernel.org/dri-devel/) (Lyude Paul's safe
mode-object layer) and drives a virtual DRM display that DLM pulls pixels from,
exactly like the C `evdi` does through its own private ioctl ABI.

`chimera/` replaces DisplayLinkManager's dock authentication, control, mode
programming, video encoding, and USB submission. It connects the compositor
frames supplied by Revdi to the same Vino protocol and codec sources used by the
kernel driver.

> **Naming.** The kernel module identifies itself to userspace as `evdi` (DRM
> driver name, `/dev/dri/cardN`, the `/sys/devices/evdi` control node) because
> DLM and `libevdi` hard-code that name. Only the *project* is called revdi.

## Layout

| Path          | What it is | License |
|---------------|------------|---------|
| `module/`     | The Rust EVDI kernel module (`evdi.ko`). Virtual DRM/KMS display, EVDI private ioctls, USB-dock pairing, and damage-tracked GRABPIX. | GPL-2.0 |
| `library/`    | `librevdi`: a Rust `cdylib` exposing the exact `evdi_lib.h` C ABI, installed as `libevdi.so.1`. | LGPL-2.1-or-later |
| `chimera/`    | The DLM-replacement daemon and offline oracle. It compiles Vino's kernel protocol and codec sources in userspace. See `chimera/README.md`. | GPL-2.0-or-later |
| `vino-driver/`| libusb transport for the dock, a frame oracle, and a focused traffic probe. | GPL-2.0-or-later |
| `vino-hdcp/`  | HDCP 2.2 reference helpers used by the reverse-engineering tools. | GPL-2.0-or-later |
| `kernel-shim/`| A userspace stand-in for the kernel prelude, so the vendored kernel sources compile unmodified. | GPL-2.0-or-later |
| `LICENSES/`   | Full license texts. | — |

The module/library split mirrors upstream evdi: the module is GPL-2.0
(`MODULE_LICENSE("GPL")`), the library is LGPL-2.1-or-later so a proprietary
client may link it.

## Vendored kernel sources

This project is **standalone**: it builds and its tests pass with no kernel tree
checked out anywhere. It manages that by keeping its own copies of two sets of
files that are *edited in the kernel tree*:

| Vendored copy      | Upstream source              |
|--------------------|------------------------------|
| `module/*.rs`      | `drivers/gpu/drm/evdi/*.rs`  |
| `chimera/vino/*.rs`| `drivers/gpu/drm/vino/*.rs`  |

The kernel tree is authoritative. Refresh the copies with:

```sh
make sync KSRC=/path/to/drm-next       # defaults to ../linux
make check-sync                        # report drift, change nothing, exit 1 if any
```

Do not hand-edit a vendored file. `chimera`'s proofs are only worth anything
because it compiles the *literal* driver sources — a local edit here would make
them silently stop tracking the real driver, which is exactly the drift the rig
exists to rule out. Edit the kernel tree, then re-run `make sync`.

## Building

### Kernel module

The module consumes Rust DRM/KMS bindings that are not yet in a released kernel,
so it must be built against a kernel tree that has them (this project is
developed against `drm-next` + the vino bindings series):

```sh
export PATH=/usr/lib/llvm/22/bin:$PATH      # kernel Rust toolchain (clang/rustc)
make -C module KDIR=/path/to/drm-next
sudo make -C module modules_install KDIR=/path/to/drm-next
```

`modules_install` runs `depmod` for the target kernel (KDIR's release) itself,
so a separate `depmod -a` — which would target the *running* kernel, not the
one you installed into — is neither needed nor correct.

`LLVM=1` is passed automatically; a stock distro kernel without the bindings
will not build it.

### Library

Builds with the system Rust toolchain (1.77+), no kernel tree required:

```sh
make -C library
sudo make -C library install       # installs libevdi.so.{1.15.0,1} + libevdi.so
```

### Chimera

Builds with the system Rust toolchain, no kernel tree required (it uses the
vendored sources above). Needs libusb development headers for the `live` feature:

```sh
make chimera
make test
```

### Everything

```sh
make          KDIR=/path/to/drm-next   # module + library
sudo make install KDIR=/path/to/drm-next
```

## Using it with DisplayLinkManager

1. Install the module and library as above.
2. Make sure the stock C `libevdi.so.1` is not shadowing ours (this replaces it
   at the same path/SONAME).
3. `modprobe evdi`, then start the DisplayLinkManager service. Plug in the dock;
   DLM opens `/dev/dri/cardN`, connects with the monitor's EDID, and drives the
   virtual display — revdi tracks damage and hands back only the changed
   rectangles per frame.

## Emergency stop / unloading the module

If something goes wrong (a wedged client, a card that will not tear down on
unplug, a suspected driver bug), the safe order is:

```sh
sudo systemctl stop displaylink-driver      # or otherwise stop DLM cleanly
echo 1 | sudo tee /sys/devices/evdi/remove_all
sudo rmmod evdi
```

`remove_all` unplugs every card in one shot without holding the registry lock
across the sleeping DRM teardown, and it is safe even while clients still
hold the DRM nodes: the cards are unplugged, not freed under them, so every
subsequent ioctl returns `ENODEV` and in-kernel waiters are drained. Once
the last client closes its file descriptors the module refcount drops to zero
and `rmmod evdi` succeeds. (`rmmod` while a client still has the node open is
refused with `EBUSY` by design — that is the kernel protecting you from a
use-after-free, not a hang.)

The same drain-then-drop path runs automatically when a dock is unplugged
(`USB_DEVICE_REMOVE` notifier) and when a single platform device unbinds.

## Status

**Module + library:** the virtual output, live frame updates with delta damage,
and clean dock-paired teardown have been exercised on a Dell D6000 under
KWin/Wayland. The compatibility cursor and DDC/CI ioctls remain accepted, but
the module advertises no cursor plane and no virtual I2C adapter in this
revision.

**Chimera:** the service connects Revdi frames to the Vino control, mode, codec,
USB, DPMS, dynamic-topology, and session-recovery paths. It retains a
userspace-only DDC/CI tunnel for research. Its optimized build and offline tests
pass; this cleanup revision has not been exercised against hardware.

## License

Dual-licensed by component, matching upstream evdi: the kernel module under
**GPL-2.0**, the library under **LGPL-2.1-or-later**. Each file carries an
`SPDX-License-Identifier`. Full texts are in `LICENSES/`.
