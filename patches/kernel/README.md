# Kernel patch series

<!-- The block below is rewritten by ../../tools/regenerate-patches.sh. -->
```text
base: 4c9ba407018e8deb06dbc643112bac8f40404f95
head: 78bedbe6845471b380f0c66191f082ce2ba19c79
range: integration/base-20260809..vino
```

Apply the exact integration series in recorded order:

```sh
git am $(sed 's#^#/path/to/vino/patches/kernel/#' series)
```

`../../tools/check-series.sh` performs this in a disposable worktree and
compares the resulting tree object with the source branch.

## Review groups

The full branch is useful for dependency and build testing. The manifests in
`groups/` identify contiguous pieces that belong in separate subsystem
discussions:

| Group | Patches | Ownership |
|---|---:|---|
| `interrupt-prerequisites` | 18 | scheduler, locking, architecture, Rust |
| `kms-lyude` | 37 | Lyude Paul's original Rust KMS work |
| `drm-crypto-platform` | 18 | DRM, crypto, driver core |
| `usb` | 7 | USB and Rust |
| `rust-runtime-drm` | 22 | Rust core, timer/workqueue, FPU, time, DRM |
| `evdi` | 1 | DRM |
| `vino` | 5 | DRM and USB |

Lyude's commits remain individual and patch-identical to the imported source;
current-tree adaptations are later Mike-authored commits. Colin Braun's first
three USB RFC patches, Alice Ryhl's v4 workqueue series, and Onur Özkan's
`cancel_sync` patch are likewise retained as their authors' work.

`../../tools/send-series.sh GROUP --version 3` rerolls a selected group with
correct per-group numbering and a cover letter. It does not send by default.
