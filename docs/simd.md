# Vectorising the transform — measured in the kernel

**Result: not worth it.** The FPU section is cheap; the transform is the problem.

`drivers/gpu/drm/vino/simd.rs` carries optional AVX2 and AVX-512 Haar transforms and an in-kernel
benchmark for them. The scalar transform is always present, is the default, and is the oracle: the
benchmark checks byte-exactness against it before reporting any timing, and refuses to report a
speedup if a single block differs. Run it with:

```sh
sudo tools/hardware/vino-cycle.sh simd_bench=1
dmesg | grep vino-simd
```

## Measured, AMD Ryzen 9 5900HX, 7.2.0-rc2-drm+

Three runs, 131,072 blocks each; the spread across runs is 1–3 ns.

| | ns/block | vs scalar |
|---|---|---|
| scalar (the current code) | 80–83 | — |
| AVX2, 8 lanes fed, FPU section per call | 81–82 | **1.02x** |
| AVX2, 8 lanes fed, one FPU section for the whole run | 81–82 | 1.02x |
| AVX2, the encoder's real batch of 3 | 218–221 | **0.37x** |

`kernel_fpu_begin()`/`kernel_fpu_end()` with an empty body: **4 ns per section.**

## What that says

**The FPU section is not the obstacle.** At 4 ns against a transform of ~80 ns it is noise, and
hoisting one section around the whole run instead of opening one per call changes nothing
measurable. The concern that motivated the original feasibility note turns out not to be the
deciding factor.

**Full-lane AVX2 is parity, not a speedup.** Every block still has to be gathered into lane-major
order before any vector arithmetic happens: 64 pixels x 8 lanes is 512 scalar loads per call, to
feed roughly 200 vector operations. The transpose is scalar, does not vectorise, and costs about
what the vectorised arithmetic saves.

⚠ **A userspace benchmark of the same arithmetic reported 1.29x.** It is `tools/simd/haar-bench.rs`,
and it is not wrong about the arithmetic — it is measuring a different baseline. Trust the in-kernel
number: the kernel builds Rust with `-Ctarget-feature=-sse,…,-avx2`, links differently, and runs the
real `video::wht::transform` rather than a copy.

**At the encoder's real shape it is 2.7x slower.** `colour_block` transforms exactly three blocks
together — `cr`, `cb`, `y` — so five of eight lanes idle and the call costs the same as a full one.
Filling the lanes means batching across strips, i.e. restructuring the encode loop, for a ceiling
that has now been measured at 1.02x.

⇒ **Do not vectorise the transform.** Both the ceiling and the realistic case are now measured in
the kernel rather than inferred.

## Implementation notes worth keeping

**Bounds checks cost 10%.** The scratch buffers started as `KVec` and the transform ran at 89% of
scalar; the same code with fixed-size arrays in a `KBox` runs at 102%. Sizes must be compile-time
constants for the indexing in the inner loops to be free — but the several KB they occupy must stay
off the stack, because the encode path already runs deep in a 16 KB kernel stack and has
`#[inline(never)]` markers specifically to keep it there.

**`#[target_feature]` is additive.** The kernel's global `-Ctarget-feature=-avx2` does not stop a
per-function `#[target_feature(enable = "avx2")]` from compiling. That is the supported way in.

**Feature detection** reads `boot_cpu_data.x86_capability`, which bindgen places behind an anonymous
union (`__bindgen_anon_3`).

## AVX-512, on a machine that has it

The AVX-512 path is written, compiles, and is skipped at runtime on any CPU without `avx512f` — this
machine reports `avx2=true avx512f=false`. On a machine with it, the same command reports an
`avx512` row beside the `avx2` one.

Expect it to look worse, not better, for this workload: 16 lanes against an encoder batch of three
leaves thirteen idle, and the transpose that already dominates the AVX2 case doubles in width. The
interesting number is the full-lane row — whether wider lanes beat the transpose — and the licence
behaviour, since sustained AVX-512 can drop core frequency on some parts and that would show up as
the *scalar* baseline changing between runs.
