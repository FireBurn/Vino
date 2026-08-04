# Vino upstream workspace

This repository is the review and reproducibility workspace for Vino, a Rust
DRM/KMS driver for DisplayLink DL3 USB display devices. It contains the working
kernel tree, the Revdi/Chimera source, generated kernel patches, protocol and
device documentation, and the scripts used to reproduce the review artifacts.

## Source authority and pins

The `vino` branch in `../drm-v3` is the implementation source of truth from
which this cleanup started. Its imported tip is preserved as
`linux:reference/drm-v3-vino`; it is not modified. Upstream-facing work is
`linux:vino` itself: development history is folded into the review series rather
than kept beside it, and the pre-fold branch is retained as
`linux:backup/vino-pre-v3-fold-20260804-2051`.

| Item | Revision |
|---|---|
| `drm-next` parent checked 2026-07-28 | `ea97ab2759506d9a818ffed1009bde01062b4091` |
| `drm-rust-next` parent checked 2026-07-28 | `93b9511a3bba7f31d95502e5f912f0a476b0cf4a` |
| integration base | `0755a4e3e809610a14befc9ad28d35e2e460da68` |
| imported `drm-v3` Vino tip | `19a91f95f35785f5f15ba57c6efffc855c47cc76` |
| kernel review branch | `vino` |
| kernel review head | `7614ff24714925a661df9bcde40e5a987eb8795a` |

The integration base is the existing `drm-rust-next` merge of the current
`drm-next` parent. The series does not create an artificial merge or pretend
that unmerged Rust DRM dependencies are already in `drm-next`.

## Layout

- [`linux/`](linux/) — kernel source and authoritative review history;
- [`revdi/`](revdi/README.md) — standalone Revdi, `librevdi`, Chimera, and
  userspace protocol oracle source;
- [`docs/`](docs/README.md) — curated architecture, protocol, reverse-
  engineering, upstream, and test documentation;
- [`patches/kernel/`](patches/kernel/README.md) — generated, mail-ready kernel
  patch export and review-group manifests;
- [`tools/capture/`](tools/capture/README.md) — device-capture toolkit: wire plus
  session keys from a live DLM, and the offline CP decryptor;
- [`tools/hardware/`](tools/hardware/README.md) — safe module reload and the
  scanout performance harness;
- [`tools/regenerate-patches.sh`](tools/regenerate-patches.sh) — deterministic
  patch export;
- [`tools/check-series.sh`](tools/check-series.sh) — apply and tree-identity
  check in a disposable worktree;
- [`tools/validate.sh`](tools/validate.sh) — source-policy and build-only checks;
- [`tools/send-series.sh`](tools/send-series.sh) — prepare a rerolled review
  group, dry-run `git send-email`, and send only when explicitly requested.

Revdi is maintained directly in this repository. It is not represented by a
second generated patch archive.

## Quick start

```sh
./tools/regenerate-patches.sh
./tools/check-series.sh
./tools/validate.sh
```

Prepare the five Vino patches as a v3 mail series:

```sh
./tools/send-series.sh vino --version 3
```

The send helper stops after creating a cover-letter draft. After editing it,
use `--dry-run --to ADDRESS`; only `--send --to ADDRESS` transmits mail.

These scripts do not install or load modules or kernels, change the bootloader,
access the dock, or reboot.

## Validation status

The kernel Rust core, in-tree EVDI object, and Vino object build. The generated
106-patch series reapplies to the pinned base with an identical tree. Revdi
source synchronization, Cargo workspace tests, library tests, the byte-exact
DLM protocol proof, formatting, and Chimera builds pass. This cleanup did not
install or load a module, install or boot a kernel, or perform a live dock test.
