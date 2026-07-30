# Testing, regeneration, and submission

The commands here are build-only or use disposable Git worktrees. They do not
install or load modules, install or boot a kernel, change the bootloader, stop
the desktop, access the dock, or reboot.

## Regenerate and verify patches

```sh
./tools/regenerate-patches.sh
./tools/check-series.sh
```

To reproduce from another kernel checkout:

```sh
KERNEL_TREE=/work/linux \
KERNEL_BASE=integration/base-20260728 \
KERNEL_HEAD=vino-upstream-rebuild \
./tools/regenerate-patches.sh
```

The first command writes the kernel patches, manifest, full `series`, and
review-group manifests under `patches/kernel/`. The second applies the complete
series with `git am` in a disposable worktree and compares its tree object with
the source branch.

Revdi and Chimera are directly tracked source, not a generated patch set.

## Combined validation

```sh
./tools/validate.sh
```

For policy, patch-application, formatting, and source-sync checks without
compilation:

```sh
SKIP_BUILD=1 ./tools/validate.sh
```

The build uses a temporary out-of-tree kernel output unless `KBUILD_OUTPUT` is
provided. It compiles `rust/kernel.o`, `evdi.o`, and `vino.o`, then runs the
Revdi/Chimera workspace, all-feature, library, and protocol-oracle tests.

Focused source commands are:

```sh
make -C revdi check-sync KSRC=../linux
cargo test --manifest-path revdi/Cargo.toml --workspace --all-features
cargo test --manifest-path revdi/library/Cargo.toml
```

## Prepare a v3 posting

Review groups are listed in `patches/kernel/groups/`. Generate a correctly
renumbered v3 Vino draft:

```sh
./tools/send-series.sh vino --version 3
```

Edit `outgoing/vino-v3/v3-0000-cover-letter.patch`, then verify recipients and
mail formatting without sending:

```sh
./tools/send-series.sh vino --version 3 --dry-run \
  --to dri-devel@lists.freedesktop.org \
  --cc rust-for-linux@vger.kernel.org
```

The helper delegates per-patch recipient discovery to the kernel's
`scripts/get_maintainer.pl`. Transmission requires the separate, explicit
`--send` option:

```sh
./tools/send-series.sh vino --version 3 --send \
  --to dri-devel@lists.freedesktop.org \
  --cc rust-for-linux@vger.kernel.org
```

The script refuses to dry-run or send while the generated cover letter still
contains placeholders.

## Kernel documentation

When Sphinx is already installed:

```sh
make -C linux htmldocs SPHINXDIRS=gpu
```

No documentation packages are installed by the scripts.

## Hardware boundary

Hardware validation remains deliberately manual. A release candidate should
cover cold and warm plug, both heads, every advertised mode, rotations and
reflections, mode changes, DPMS, cursor, monitor removal and reappearance, USB
reset, suspend/resume, control-plane recovery, unload with open and closed DRM
files, and sustained damage traffic.

Building and installing the modules for a hardware run:

```sh
make -C linux LLVM=1 -j16 modules       # no `M=`
sudo make -C linux modules_install
sudo depmod -a
```

⛔ **Never put `M=` on a `modules_install` line.** `M=` sets `KBUILD_EXTMOD`, and
`scripts/Makefile.modinst` then installs to `/lib/modules/$(uname -r)/updates/` rather than
`kernel/...`. `depmod` *prefers* `updates/`, so a stray copy there silently shadows the real module
and every later reinstall appears to do nothing — a failure that survives reboots and is invisible
unless you look for it:

```sh
ls /lib/modules/$(uname -r)/updates/ 2>/dev/null   # must not exist
modinfo -n vino                                     # must be under kernel/...
```

`make -C linux LLVM=1 M=drivers/gpu/drm/vino modules` is a safe, much faster **build-only**
shortcut, but it only produces the `.ko` — place it yourself with `cp` into the matching
`kernel/...` path and re-run `depmod -a`. ⚠ `make LLVM=1 drivers/gpu/drm/vino/` compiles objects
only and yields no `.ko` at all.

[`tools/hardware/`](../tools/hardware/README.md) supports that work:

```sh
sudo tools/hardware/vino-cycle.sh      # safe unbind/unload/load/rebind cycle
sudo tools/hardware/vino-perf.py --secs 30
```

⚠ Never `modprobe -r` while a DRM file is open — it frees the fops under the
compositor and hangs the machine. `vino-cycle.sh` refuses instead of forcing.

⚠ `usbmon` is not autoloaded; `vino-perf.py` needs it.

Capturing an unfamiliar device driving under DLM, rather than under Vino, is a
separate exercise: see [`new-device-capture.md`](new-device-capture.md) and
[`tools/capture/`](../tools/capture/README.md).
