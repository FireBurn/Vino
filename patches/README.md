# Generated kernel patches

[`kernel/`](kernel/README.md) is generated from
`linux:integration/base-20260728..vino` by
[`../tools/regenerate-patches.sh`](../tools/regenerate-patches.sh). Do not edit
the patch files by hand; amend the source history and regenerate.

The export contains:

- numbered `git format-patch` output;
- `series`, in application order;
- `manifest.tsv`, mapping each patch to its commit, author, and subject;
- `groups/*.series`, one per subsystem, each independently reviewable and contiguous:

| group | patches | list |
|---|---|---|
| `rust-core` | 34 | rust-for-linux |
| `rust-crypto` | 2 | linux-crypto + rust-for-linux |
| `rust-usb` | 6 | linux-usb + rust-for-linux |
| `rust-drm` | 61 | dri-devel |
| `vino` | 6 | dri-devel |

They apply in that order, each depending only on the ones before it. ⚠ The branch is kept in this
order deliberately: the groups were interleaved across twenty runs before 2026-08-08 and none could
be posted on its own. The group boundaries are pinned by starting line in
`tools/regenerate-patches.sh`, which errors rather than silently mis-tiling if they drift.

Revdi and Chimera are source directories in the top-level repository, not a
second generated patch series.
