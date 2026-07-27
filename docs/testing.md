# Testing and regeneration

All commands in this document are build-only or operate on disposable Git
worktrees. They do not install or load modules, install a kernel, alter a
bootloader, start or stop DisplayLink services, access hardware, or reboot.

## Regenerate patches

From the repository root:

```sh
./tools/regenerate-patches.sh
```

Override a path or range when reproducing elsewhere:

```sh
KERNEL_TREE=/work/linux \
KERNEL_BASE=integration/base-20260727 \
KERNEL_HEAD=series/vino-upstream \
REVDI_TREE=/work/revdi \
./tools/regenerate-patches.sh
```

The script writes kernel and Revdi patches plus `series` and `manifest.tsv`
files under `patches/`.

## Verify patch application

```sh
./tools/check-series.sh
```

The script creates temporary detached worktrees, applies the generated patches
with `git am`, compares the resulting tree object with the named source head,
and removes the worktrees.

The combined validator also runs strict `checkpatch.pl` checks over the final
EVDI and Vino consumer patches. The generic prerequisite series is left to its
respective subsystem review and is not silently reformatted.

## Kernel build

The focused build used during cleanup is:

```sh
make -C kernel LLVM=1 -j16 \
  rust/kernel.o \
  drivers/gpu/drm/evdi/evdi.o \
  drivers/gpu/drm/vino/vino.o
```

An external-module `modpost` requires a complete matching kernel build and
`Module.symvers`; its absence is not a Rust compile failure.

## Revdi and Chimera

```sh
make -C revdi check-sync KSRC=../kernel
make -C revdi test
cargo test --manifest-path revdi/Cargo.toml \
  -p vino-chimera --all-features
make -C revdi chimera
```

The first command ensures the standalone copies match the kernel. Tests cover
the workspace, the ABI-compatible library, protocol fixtures, mode profiles,
video-arm construction, DDC/CI response bounds, pixel conversion, cursor
repacking, padding, and codec framing. The all-features run also compiles and
tests the daemon integration path.

## Combined validation

```sh
./tools/validate.sh
```

Use `SKIP_BUILD=1` for policy, patch, and synchronization checks only:

```sh
SKIP_BUILD=1 ./tools/validate.sh
```

## Documentation

The kernel document can be rendered through the normal kernel Sphinx target
when Sphinx is already available:

```sh
make -C kernel htmldocs SPHINXDIRS=gpu
```

This cleanup did not install Sphinx or TeX packages. Missing documentation
tooling should be reported as an environment prerequisite, not worked around by
installing packages without permission.

## Hardware boundary

Hardware testing is intentionally separate. A release candidate should later
cover cold plug, warm plug, both heads, each supported mode, mode changes,
DPMS, cursor, DDC/CI, monitor removal/reappearance, USB reset, suspend/resume,
module unload with open and closed DRM files, and sustained damage traffic.
That procedure may stop services or load modules and is therefore never run by
the validation scripts.
