# AVX2 in the encoder — what it would take

Feasibility notes, established 2026-08-01. **Nothing is implemented yet.**

The encoder is the driver's dominant cost: fullscreen video is ~2.7 cores of irreducible codec work,
and `PixelSource::px` alone is the third-hottest symbol in the kernel (13.8% of the machine on a 4K
clip). The last big win came from removing per-coefficient branch dispatch in favour of const
tables (2.65 → ~2.05 cores). Vectorising the transform and quantise passes is the obvious next
lever.

## It is feasible, with three constraints

**1. The kernel disables SIMD for Rust globally.** `arch/x86/Makefile`:

```
KBUILD_RUSTFLAGS += -Ctarget-feature=-sse,-sse2,-sse3,-ssse3,-sse4.1,-sse4.2,-avx,-avx2
```

This mirrors what C does, and for the same reason: the kernel does not preserve FPU/vector state
across context switches unless told to. `#[target_feature(enable = "avx2")]` on an `unsafe fn` is
**additive per function**, so it still compiles under that global disable — this is the supported
way in, not a workaround.

**2. Vector registers need an explicit FPU section.** Both halves are already in the generated
bindings, so no new C helper is required:

```
kernel_fpu_begin_mask(kfpu_mask: c_uint)
kernel_fpu_end()
```

What is missing is a safe Rust wrapper. An RAII guard — `begin` on construction, `end` on drop —
belongs in `rust/kernel/` and therefore in `patches/`, not in the driver. It must be
non-`Send`/non-`Sync` and must not allow sleeping while held; the section has to be short and
straight-line.

**3. The codec is byte-exact against DLM and must stay that way.** Any vectorised path needs the
scalar one kept as the oracle, and a differential test that runs both over the same input and
compares output bytes. `revdi/chimera` already compiles vino's codec verbatim in userspace, which
is where that comparison should live — no dock required, and userspace has AVX2 unconditionally.

## Suggested order

1. Add the `FpuGuard` binding in `patches/`, with the safety contract documented.
2. Prototype the vectorised transform **in chimera first**, where it can be measured and diffed
   against the scalar output without a kernel build or hardware.
3. Only once byte-exactness holds, port it behind a runtime `boot_cpu_has(X86_FEATURE_AVX2)` check
   with the scalar path retained as the fallback.
4. Measure with `tools/hardware/vino-perf.py`.

⚠ Measure in **cores from `/proc/stat`**, not relative percentages — relative figures have twice
produced conclusions that did not survive checking.

## Where the time actually goes

Worth re-measuring before optimising, rather than assuming. The last profile put the cost in
`transform` and `colour_block`, with the encoded-strip retransmit cache already at its ceiling
(68–69% reuse against a 66.7% ideal), so it is the per-strip arithmetic that is left.
