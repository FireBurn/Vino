# Colour management (`CTM` + `GAMMA_LUT`)

Both drivers advertise the CRTC's `CTM` and `GAMMA_LUT` properties and apply them **in software**,
because neither has colour hardware to program: vino sends already-encoded pixels to a dock, and
evdi hands framebuffer pixels to a userspace client.

Without this, a compositor that colour-corrects through the KMS properties has nowhere to put the
correction on these outputs while native outputs are corrected normally. GNOME's Night Light and
KDE's Night Colour both work that way rather than rewriting the framebuffer. The same gap in
upstream evdi is [DisplayLink/evdi#584](https://github.com/DisplayLink/evdi/pull/584).

## Shape

| | |
|---|---|
| advertised | `enable_color_mgmt(0, true, 256)` — CTM and a 256-entry gamma LUT, no degamma LUT |
| pipeline | **CTM, then gamma** (DRM's order, minus the degamma stage we do not advertise) |
| vino applies at | `PixelSource::px`, during the encoder's pixel walk |
| evdi applies at | the `GRABPIX` row copy, before the row reaches userspace |
| cached | per head, from `atomic_flush` — **not** `atomic_enable`, because a night-light corrector ramps its CTM continuously on an already-enabled CRTC, which never re-runs enable |
| cleared | `atomic_disable` |

`ColorPipeline::build` returns `None` when nothing is programmed **or when the CTM is the
identity** — a compositor turning a corrector off sends an identity matrix rather than removing the
blob, and collapsing that back to `None` is what lets vino's direct-scanout path and evdi's plain
copy path return.

⚠ The transform is applied to the framebuffer's encoded (typically sRGB) values, not to linear
light, because there is no degamma stage to linearise them first. That is the same simplification
every software implementation makes, and it is what a compositor expects from a CRTC advertising no
degamma LUT.

## Two representations, for cost

`px` is the third-hottest symbol in the kernel under fullscreen video (13.8% of the machine on a 4K
clip), so the per-pixel cost matters more than generality.

* **`Fused`** — one 8-bit table per channel. Covers a gamma ramp alone *and* a gamma ramp after a
  channel-independent CTM. Every colour-temperature corrector produces a diagonal matrix, so the
  common case stays at one table lookup per channel: the same cost as before CTM existed.
* **`Mixed`** — only for a matrix that genuinely mixes channels. Q16 fixed point, `i32 * i32` into
  an `i64`; S31.32 would need 128-bit intermediates, which are not available on every architecture
  the kernel builds for.

The fast path must agree with the general one exactly, or colour would change with the
*optimisation* rather than with the CTM. `color-selftest.sh` asserts that directly.

## One file, two drivers

`drivers/gpu/drm/vino/color.rs` and `drivers/gpu/drm/evdi/color.rs` are **byte-identical**. They
are separate modules and cannot share a crate, so the copies are kept in sync and
`tools/color-selftest.sh` fails if they drift. evdi declares the module `#[allow(dead_code)]`
because `tag()` serves only vino's encoded-strip cache. `revdi/module/color.rs` is vendored from
the in-tree copy by `revdi: make sync`.

## Testing

```bash
tools/color-selftest.sh        # drift check + runs the real arithmetic; needs only rustc
```

The in-tree KUnit tests (`CONFIG_DRM_VINO_KUNIT_TEST`) assert the same properties, but they are
gated behind a kernel built with `CONFIG_KUNIT=y` — and were silently **not compiled at all** on
this tree, so the maths went unrun. `color-selftest.sh` compiles the drivers' real `color.rs` with
plain `rustc` against a small shim, so the arithmetic is testable anywhere.

That mattered immediately: it caught `narrow()` dividing by **256** where `expand()` multiplies by
**257**, so `narrow(expand(v)) != v` and *merely enabling* colour management shifted every value
above about 128 — an identity gamma ramp was not a no-op. It also caught a systematic half-level
downward bias from truncating fixed-point multiplies, now rounded to nearest.

## Traps worth keeping

* **The CTM is S31.32 sign-magnitude, not two's complement** (bit 63 is the sign). Reading the
  `u64` as an `i64` turns every negative coefficient into a huge positive one — a darkening
  correction would saturate to white instead. Decoded once in the binding
  (`ColorCtm::coefficient`) so no driver repeats it.
* **Off-diagonal terms below Q16 resolution are treated as zero**, so a matrix with a
  2⁻³² off-diagonal entry legitimately takes the fused path.
* **A short LUT blob extends with identity, not zeroes** — zeroes would render the tail black.
* **A transform change must invalidate the encoded-strip cache.** It keys on a strip's source
  pixels, so a change that leaves those pixels alone would otherwise re-send stale bodies for the
  whole screen. `ColorPipeline::tag()` feeds that, and `update_color` owes a keyframe.

## Not implemented

`DEGAMMA_LUT` is not advertised (`degamma_size = 0`). Adding it would mean linearising before the
CTM and re-encoding after the gamma ramp, which is a real colour-accuracy change rather than a
plumbing one, and no compositor requires it to drive the other two.
