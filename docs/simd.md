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

Four runs, 131,072 blocks each. Full-lane AVX2 lands either side of parity across runs, which is
itself the finding: the effect is smaller than the run-to-run spread.

| | ns/block | vs scalar |
|---|---|---|
| scalar (the current code) | 80–83 | — |
| AVX2, 8 lanes fed, FPU section per call | 81–88 | **0.93–1.02x** |
| AVX2, 8 lanes fed, one FPU section for the whole run | 81–85 | 0.96–1.02x |
| AVX2, the encoder's real batch of 3 | 218–232 | **0.35–0.37x** |

`kernel_fpu_begin()`/`kernel_fpu_end()` with an empty body: **4 ns per section.**

## The transform is 9% of a strip encode

The same benchmark times a whole strip through `colour_strip_at` — pixel gather, transform,
quantiser, entropy coder — against the transform alone:

```
strip encode 40025 ns (1907 B avg), of which 48 transforms = 3936 ns, 9%
```

A strip is 16 blocks and `colour_block` transforms three planes per block, so a strip pays 48
transforms. **Everything the transform can win or lose is bounded by that 9%**, which settles the
question before any of the rows below matter:

| | ns/strip | live encode CPU |
|---|---|---|
| today | 40,025 | — |
| a *perfect*, zero-cost transform | 36,089 | **−10%** (the ceiling) |
| AVX2 at the encoder's batch of 3 | 47,225 | **+18%** |

⇒ Yes, it would be more CPU with live video: roughly a fifth more encode CPU, against a best case of
a tenth less that no implementation can reach.

⚠ Not measured on live frames. The AVX2 path is benchmark-only and is deliberately not wired into
the encoder, so no displayed frame has gone through it. The +18% is the measured per-strip cost
applied to the measured strip mix, not an end-to-end capture.

## Where a strip encode actually goes

The same benchmark times each stage. This is the answer to "is there a more efficient way to write
the codec for AVX2":

| stage | ns/strip | share | vectorises? |
|---|---|---|---|
| entropy coder | 29,603 | **72%** | **no** — bit-serial, variable-length, data-dependent |
| transform | 4,368 | 11% | poorly — needs a transpose (see below) |
| quantise + `chroma_last` | 3,920 | 10% | yes, elementwise |
| `colour()` conversion | 1,024 | 2% | yes, elementwise |
| pixel gather, allocation | 2,126 | 5% | — |
| **total** | **41,041** | | |

⇒ **The codec is dominated by the one stage SIMD cannot help.** Building a variable-length bitstream
is inherently serial: each symbol's length depends on its value and on how many bits are already in
the accumulator. Vectorising every elementwise stage perfectly — transform, quantise and colour
together, 23% — caps the whole encoder at a 23% win, and the realistic share of that is far less.

## The transform written the right way: 2x, and it is in the tree

Vectorising **within** a block instead of across blocks removes the transpose that ate the gain. A
row of an 8x8 `i32` block is exactly one `__m256i`, so the column pass is whole-vector add/sub with
no shuffle and only the row pass needs one permute. Levels 2 and 3 stay scalar: 20 of the 84
butterflies, on data narrower than a vector.

`transform_inblock_avx2` is byte-exact against scalar over 4096 blocks and transforms **one** block
per call, so unlike the across-blocks form it has no lane-utilisation penalty — the encoder's batch
of three costs three calls, not eight idle lanes.

| transform | ns/block | vs scalar |
|---|---|---|
| scalar | 98 | — |
| across-blocks, 8 lanes fed | 91 | 1.07x |
| across-blocks, encoder's batch of 3 | 266 | **0.36x** |
| **within-block, FPU per block** | **53** | **1.84x** |
| **within-block, FPU per strip** | **48** | **2.04x** |

⚠ The FPU section is per block in the current integration, which the table shows costs ~10%.
Hoisting it to strip level — one section around all 16 blocks — is the obvious next step.

### What that is worth to the encoder

`simd_transform=1` puts it in `colour_block`. Measured on a quiesced machine, two runs each, with
the deterministic strip benchmark:

| | scalar | avx2 | |
|---|---|---|---|
| `colour_block` | 7,992 ns | 6,752 ns | **−15.5%**, consistent across runs |
| strip encode | 43,423 ns | 40,012 ns | −7.9%, but the spread is −2.7%…−13% |
| entropy coder | 31,320 ns | 28,279 ns | −9.7% — **and it uses no SIMD** |

⛔ **The entropy coder moved 9.7% between configurations that cannot have changed it.** That is the
noise floor of the whole-strip figure, so only the `colour_block` number is solid: **−15.5%**, which
is the transform's ~53% share of that function speeding up 2x.

⇒ Expect single-digit percent off encode CPU. Not demonstrable on live video: two 60 s windows of
the same clip differed in frames, bytes and CPU all at once, because the content varies. Comparing
runs there needs a deterministic loop, not a music video.

⇒ The transform is worth 2x and about 15% of `colour_block`, and that is the whole of what SIMD can
reach here. The 72% that is left is the entropy coder, and the lever there is not wider arithmetic
but fewer unpredictable branches per symbol — the same shape of fix as replacing per-coefficient
branch dispatch with const tables.

## What that says

**The FPU section is not the obstacle.** At 4 ns against a transform of ~80 ns it is noise, and
hoisting one section around the whole run instead of opening one per call changes nothing
measurable. The concern that motivated the original feasibility note turns out not to be the
deciding factor.

**Full-lane AVX2 is parity, not a speedup** — it lands either side of 1.00x across runs. Every
block still has to be gathered into lane-major order before any vector arithmetic happens: 64
pixels x 8 lanes is 512 scalar loads per call, to feed roughly 200 vector operations. The transpose
is scalar, does not vectorise, and costs about what the vectorised arithmetic saves.

⚠ **A userspace benchmark of the same arithmetic reported 1.29x.** It is `tools/simd/haar-bench.rs`,
and it is not wrong about the arithmetic — it is measuring a different baseline. Trust the in-kernel
number: the kernel builds Rust with `-Ctarget-feature=-sse,…,-avx2`, links differently, and runs the
real `video::wht::transform` rather than a copy.

**At the encoder's real shape it is 2.7x slower.** `colour_block` transforms exactly three blocks
together — `cr`, `cb`, `y` — so five of eight lanes idle and the call costs the same as a full one.
Filling the lanes means batching across strips, i.e. restructuring the encode loop, for a ceiling
that has now been measured at 1.02x.

⇒ **Do not vectorise the transform.** Both the ceiling and the realistic case are measured in the
kernel rather than inferred, and the stage breakdown above shows why it was never the right target.

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
