# Kernel patch series

The series is generated from:

```text
base: integration/base-20260727
head: series/vino-upstream
```

The base is the 2026-07-27 `drm-next` tip
`ea97ab2759506d9a818ffed1009bde01062b4091` merged with the matching
`drm-rust-next` tip `6dcbb4b1320fa91fee349462a52bb69135f2e45e`.
The resulting local integration commit is
`90e13d487b3b828669dab730cfdf72d417825869`.

Apply in the order recorded by `series`:

```sh
git am /path/to/vino/patches/kernel/*.patch
```

For an exact local check, use `../../tools/check-series.sh`.

The series preserves the prerequisite and Lyude KMS commits as individual
authored patches. Shared Rust additions follow their subsystem boundaries.
Revdi is the first safe virtual-KMS consumer. Vino is introduced in control,
video, KMS, USB/lifecycle, and documentation patches without the historical
bring-up experiments.

[`COVER_LETTER.md`](COVER_LETTER.md) is a maintained submission draft rather
than generated `git format-patch` output.

