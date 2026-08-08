# v3, split by subsystem

Five series, each contiguous on `linux:vino` and applying in this order. Generated from
`v3-reorder`; the tree at the tip is byte-identical to the branch it was folded from, and the
driver builds warning-clean at the tip of every series.

| series | patches | goes to | depends on |
|---|---|---|---|
| `core/` | 18 | rust-for-linux | — |
| `crypto/` | 2 | linux-crypto + rust-for-linux | core |
| `usb/` | 6 | linux-usb + rust-for-linux | core |
| `drm/` | 61 | dri-devel | core |
| `vino/` | 6 | dri-devel | all of the above |

⚠ Each cover letter is a stub: `git format-patch --cover-letter` fills in the subject and the
diffstat but not the rationale. Write those before posting.

## What changed from the branch's development history

The vino driver was **33 patches of development history and is now 6**. That history contained a
revert pair, a module parameter added and later deleted, selftest corrections, and fixes to patches
earlier in the same series -- none of which a reviewer should have to read. The 6 introduce the
driver in the order it is understood: control protocol, codec, KMS engine, USB driver, docs, plus
the one KMS binding it needs, which now sits in the `drm` series where it belongs rather than
inside the driver's.

⛔ The remaining 87 are Lyude's KMS series and the core/USB/crypto bindings, reordered but
otherwise untouched.
