# Upstream status and review disposition

## 2026-09-21 re-check: rebase is cheap, and nothing upstream is ready to take

Rechecked against `drm-rust-next`, Lyude's tree and the mailbox, and the branch was
rebased onto the result. Three-line summary: **the rebase cost one conflict; no
prerequisite has landed; one patch we carried had gone stale and is now dropped.**

`vino` is `6bd5eaeb9827` + **102** commits (was 103). Pre-rebase branch kept as
`backup/vino-20260921-pre-rebase`.

### The base moved, and following it was nearly free

`drm-rust-next` went `c63829528980` -> `6bd5eaeb9827` (59 commits, tip 2026-09-20).
⚠ **No `drm-next` merge in that range** -- it is driver-core's `rust-io-7.4-rc1` and
`rust-dma-7.4-rc1`, plus nova-core's MM/page-table work. So the DRM content under us is
unchanged; what moved is `rust/kernel/io.rs`.

Test-rebased all 103 of our commits onto the new tip:

- **one conflict**, in `rust/kernel/io.rs`. Upstream's `rust: io: perform conversions
  using AsRepr` added `io_view_as_repr()` in the same place our
  `rust: io: add checked offset copy helpers` adds `io_byte_slice()`. Both are new
  free functions next to `io_view()`; the resolution is to keep both.
- `git range-diff` reports **two patches changed, both context-only**: the io one above,
  and `rust: helpers/drm.c: Split into its own folder`, whose context shifted because
  upstream added `dma_fence.c` to `helpers.c`.
- The whole collision surface is **six files**: `rust/kernel/io.rs`, `rust/kernel/lib.rs`,
  `rust/bindings/bindings_helper.h`, `rust/helpers/helpers.c`, `MAINTAINERS` and
  `drivers/gpu/drm/tyr/driver.rs`. Only the first needed a human.
- The rebased branch builds **warning-clean**: `make LLVM=1` and `make LLVM=1 modules`
  both exit 0 with zero warnings, and `vino.ko` is produced. (⚠ `make modules` alone
  fails at modpost with `vmlinux.o is missing` and a wall of undefined symbols -- that is
  the missing vmlinux, not the rebase. Build the kernel first.)

### Prerequisites: none landed, one is close

| Dependency | State on 2026-09-21 | Action |
|---|---|---|
| Lyude Paul, Rust KMS | Still no `rust/kernel/drm/kms*` upstream | Keep carrying |
| Colin Braun, URB | Unchanged since the 2026-07-12 v1; no replies | Keep carrying |
| Danilo Krummrich, workqueue | **Still v2, but reviewed** | Wait for v3 |
| `wait_for_completion_timeout` | Upstream `completion.rs` still has only `wait_for_completion()` | Ours is load-bearing |
| Boqun et al., interrupt prerequisites | **Resolved** | Watch item closed |
| Igor Korotin, Rust I2C | **Landed in v7.3-rc1** -- but consumer side only | Disposition unchanged |

**Lyude's branch has been rebuilt.** `rvkms-slim` no longer has our imported
`25bc8cc7e97f` as an ancestor; its tip is `46f373d35b78` (2026-09-01),
literally `WIP: DOES NOT COMPILE`, 13 files, +379/-75, reworking `drm/kms.rs`,
`kms/crtc.rs`, `kms/framebuffer.rs`, `kms/plane.rs` and `kms/encoder.rs`.
⛔ **Nothing to take**: it does not build, and it is a WIP branch, not a posted
revision. The 17 `fixup!` commits under it are dated 2026-07-17..24 -- they *predate*
our last check -- and all 14 targets are commits we carry. Spot-checked the most
substantive one (`fixup! rust: drm/kms: Add drm_connector bindings`, which narrows
`Send`/`Sync` from any `DeviceContext` to the initialised one): **we already carry the
tightened form**, in connector, crtc, plane and encoder alike. So the fixups are folded
and only the 2026-09-01 WIP is new. Watch it; do not import it.

**The workqueue series is the one to watch.** Danilo's
`[PATCH v2 0/6] workqueue: OwnedQueue, ScopedQueue and ScopedWork` is still v2
(2026-08-07), but it is no longer quiet: Daniel Almeida gave `Reviewed-by` on 1/6, 2/6,
3/6 and 4/6 on 2026-08-27, Alice Ryhl and Gary Guo reviewed 5/6 and 6/6 on 2026-09-02,
and Onur Ozkan answered both on 2026-09-12. A v3 is likely imminent. The plan is
unchanged: take it when v3 lands, in **one** commit that swaps the prerequisites and
migrates the seven `Work::cancel_sync()` call sites together.

**The interrupt watch item is closed.** `rust/kernel/interrupt.rs` is present in our own
base (it arrived with v7.3-rc1) and Boqun's `e901c1510e24` is an ancestor of
`drm-rust-next`. The series carries exactly one interrupt-adjacent commit now,
`rust: hrtimer: expose interrupt state in hard callbacks`. The 2026-08-23 instruction to
"re-check the whole group before cutting v3" has been discharged: the group is gone.

**I2C landed, and it does not help us.** Igor Korotin's work merged for v7.3-rc1 (via
`i2c-7.3-part2` and `driver-core-7.3-rc1`) and is therefore already under us. ⚠ It is the
**consumer** side -- `I2cClient`, `I2cAdapter::get(index)`, a client `Registration`, and
an `Adapter<T>` that is the driver-matching adapter, not a bus one. There is no
`i2c_add_adapter`, no `i2c_algorithm`, no `master_xfer`. The **adapter-provider** binding
vino would need to publish a DDC/CI channel still does not exist, so the v1/v2 decision
not to register a downstream adapter stands -- but its stated reason ("Igor's
provider-lifetime issue is unresolved") is no longer the live one, and the disposition
text should say so when v4 goes out.

### ⛔ `sched-fair` dropped: its premise was false

`045c0a4a859f sched/fair: stop reading cfsb_guard.flags after the guard drops it`
desugars one `CLASS(raw_spinlock_irqsave, cfsb_guard)` call site back to plain
`raw_spin_lock_irqsave()`, because the locking-guard series we used to carry removed the
`flags` field from that guard.

**We do not carry that series any more.** Our branch does not touch
`include/linux/spinlock.h` at all, and both our tree and `drm-rust-next` still define the
guard as `DEFINE_LOCK_GUARD_1(raw_spinlock_irqsave, ..., unsigned long flags)`.
`kernel/sched/fair.c:7494` reads `cfsb_guard.flags` upstream today and compiles.

So the patch worked around a removal that no longer happens: a no-op, and an unrelated
change to `kernel/sched/fair.c` sitting in the branch whose commit message argued from a
premise that is false against mainline.

**Dropped.** `kernel/sched/` now matches upstream byte for byte, `carried_order` is just
`drm-tyr` in both `tools/regenerate-patches.sh` and `tools/check-series.sh`,
`patches/not-posted/sched-fair/` is gone, and the cover letters say "one commit" rather
than two. The open item about sending it to Peter Zijlstra and Ingo Molnar is closed.

### Folded while rebasing

Three comments in the driver described code that is not there, which is the one style rule
this tree breaks most easily. Each fix went into the commit that owns the line
(`git commit --fixup` + `--autosquash`), not a follow-up cleanup patch:

- `drm_sink/presence.rs` explained which repair strategy was tried before and why it was
  abandoned. It now states what the repair does and what dropping the connector instead
  would cost.
- `session/setup.rs` described the timing of an eight-drain reply loop the code no longer
  has. It now states the rule and keeps the 90 ms that constrains it.
- `cp.rs` said "beyond the old catalogued range", implying a superseded catalogue.

⚠ Non-ASCII survives in `rust/kernel/drm/kms/crtc.rs`, `rust/kernel/usb.rs` and
`rust/kernel/usb/ch9.rs` -- ellipses, an em-dash and `§`. All are **Lyude's and Colin
Braun's own lines**, so they stay: third-party commits keep their content. Our own
sources and all 58 of our commit messages are clean of non-ASCII, dates, out-of-tree
paths and AI tells.

### Two pre-send checks had been verifying nothing

Both were pinned to bases that stopped being current two rebases ago, so they
looked like they passed (or failed noisily) without checking what they claim to.

- **`tools/validate.sh` defaulted to `integration/base-20260809`.** Everything it
  derives from `$kernel_base..$kernel_head` was therefore computed over thousands of
  unrelated upstream commits: its `git diff --check` was reporting trailing whitespace in
  `drivers/gpu/drm/amd`, `drivers/mtd`, `fs/ntfs3` and `io_uring`. Now
  `integration/base-20260921`. ⭐ With the right base it immediately found three real
  defects it had been unable to see: `color.rs:22` (112 columns, and `*every*` markdown
  emphasis, which upstream style forbids in doc comments), `cp.rs:787` (102) and
  `drm_sink/scanout.rs:621` (112) -- all outside string literals, so all genuine. Reflowed
  and folded into their owning commits.
- **`tools/regenerate-patches.sh` was putting a false paragraph in all six cover
  letters**: "The reference branch also carries Boqun Feng's counted interrupt disabling
  series". It does not -- the branch's only non-Mike authors are Lyude Paul, Colin Braun,
  Alice Ryhl and Onur Ozkan, and `e901c1510e24` is an ancestor of our own base. Removed;
  that also cleared all six `checkpatch` ERRORs, which were the bare `as e901c1510e24`
  reference failing the `commit <sha> ("title")` rule.

### Mannered emphasis removed from the driver's comments

The driver had 56 single-asterisk italics in its comments. 35 introduce a term
(`the *wire*`, `*encoded*`) and are ordinary typography, kept. The other 22 emphasised
function words -- `*is*`, `*and*`, `*that*`, `*first*`, `*before*` -- which is the
house-style tic the upstream rules mean by shouty emphasis, and reads as somebody's notes
rather than kernel code. All 22 are gone, across 12 files, folded into the eight commits
that own the lines. No `**bold**` was present. `color.rs`'s `*every*` went with the
reflow above.

⚠ **`checkpatch` must be run from inside the kernel tree.** Run from `vino/` it cannot
find the tree root, gives up on parsing, and reports every blank context line in a hunk as
`ERROR: trailing whitespace` -- 194 of them, all false. From `linux/` the same patches give
**0 errors**.

Where the series now stands on the standard checks: `rustfmtcheck` clean, `checkpatch
--strict` 0 errors, and the remaining warnings are the expected ones -- 17 "does
MAINTAINERS need updating?" for new files, Rust 100-column notes on string literals, and
cover-letter prose wrapped at 80 rather than 75.

### ⚠ For Mike: four of Lyude's commits are not patch-identical

`tools/validate.sh` compares the third-party commits against
`backup/vino-usb-unsplit-20260728`, a branch of ours from before both the rust-drm recut
and the rust-usb split, so that comparison has been failing on noise since 2026-08-24.
**Not caused by the rebase** -- it fails identically on `backup/vino-20260921-pre-rebase`
against the old base.

Comparing against the real source instead (`reference/lyude-rvkms-slim`, the imported
`25bc8cc7e97f`) gives a cleaner answer. Of her 37 commits, **29 are byte-identical**, and
of the 8 that are not, 4 differ only in rebase context -- the fourcc commit, for instance,
differs solely by a `pub mod gpuvm;` context line that upstream added since.

**Four have genuinely modified content**, small but real:

| commit | changed lines |
|---|---:|
| `rust: drm: Add traits for registering KMS devices` | 75 |
| `rust: drm: Add Device::event_lock()` | 15 |
| `rust: helpers/drm.c: Split into its own folder` | 10 |
| `rust: drm/kms: Add VblankSupport` | 9 |

The cover letters and the v1/v2 disposition both say her commits are "patch-identical to
the imported source", with adaptations kept as separate Mike-authored commits. For these
four that is not true: the adaptation is folded into her commit.

⛔ **Left alone deliberately.** Splitting an adaptation back out of someone else's commit
is a change to attribution, and the decision is yours: either move those hunks into
Mike-authored follow-ups so the claim holds, or soften the claim in the cover letters to
say her commits are carried with minimal adaptation where the current DRM API required it.
Whichever way, `validate.sh`'s `external_reference` should point at
`reference/lyude-rvkms-slim` rather than one of our own old backups, and the comparison
should ignore context lines, or it can never pass across a rebase.

### Mailbox notes

- **Randy Dunlap replied to `[PATCH v3 13/13]`** (2026-08-26,
  `rdunlap@infradead.org`) with comments on `Documentation/gpu/vino.rst`. Human review,
  not yet dispositioned in `todo.md`.
- Miguel Ojeda replied to `[PATCH 0/9] rust: core abstractions for a USB display driver`
  on 2026-08-26.
- `sashiko-bot` reviewed most of the `drm-vino` patches; its recurring high-severity claim
  across 6/13, 7/13, 8/13, 9/13, 11/13 and 12/13 is **sleeping in atomic KMS callbacks**.
  Worth answering once, centrally, rather than six times.
- ⚠ Several v3 Cc addresses **bounced**: `airlied@gmail.com` and
  `nick.desaulniers+lkml@gmail.com` (Gmail policy), `daniel.almeida@collabora.com`
  (remote policy), plus kernel.org SRS bounces. Fix the Cc list before v4 or those
  maintainers never see it.
- Someone else is in this space now: `[PATCH] drm/trigger5: Add MCT Trigger 5 USB display
  driver` (Ho Jie Feng, 2026-09-10, v2 2026-09-14) -- a C driver for MCT Trigger 5
  adapters, on dri-devel. Not a dependency; useful precedent for how dri-devel receives a
  reverse-engineered USB display driver.

---

## v3, as cut on 2026-08-26

**Six series are posted, and two of our own commits are deliberately not.**
The branch is `vino-v3` in `linux/`, 116 commits on
`integration/base-20260809`, of which 55 are ours.

| group | patches | version | list |
|---|---:|---|---|
| `rust-core` | 9 | new | rust-for-linux + lkml |
| `rust-crypto` | 2 | v3 | linux-crypto + rust-for-linux |
| `rust-usb` | 5 | v3 | linux-usb + rust-for-linux |
| `rust-drm` | 23 | v3 | dri-devel + rust-for-linux |
| `rust-firmware` | 1 | new | lkml + rust-for-linux |
| `drm-vino` | 13 | v3 | dri-devel |

Not posted, exported to `patches/not-posted/`: `sched-fair` (1) and `drm-tyr`
(1). Both are build fixes the reference tree needs; neither enables any part of
Vino.

**What was dropped from the posting, and why.** Everything that does not enable
Vino came out, which is a sharper rule than "everything that compiles":

- **`drm-evdi`.** A second consumer is worth having, but it is a separate
  argument with its own UAPI-documentation work, and carrying it doubles the
  review surface of the whole chain.
- **`rust-core`: runtime platform-device creation, root-device attribute
  groups, the kernel-FPU section guard.** `git grep` over `drivers/gpu/drm/vino`
  finds no caller for any of the three: the first two were EVDI's, and the FPU
  guard's consumer (an AVX2 transform) was measured at parity-or-slower and
  removed. Rust-for-Linux does not take an abstraction with no in-tree user, and
  it would have been right not to.
- **`rust-drm`: typed RAII event channels, private ioctl compat translations.**
  Both EVDI's. Vino declares `declare_drm_ioctls! {}` and delivers vblank events
  through KMS.
- **`rust-usb`: the topology walk and the device-removal notifier.** Also
  EVDI's, and never reviewed by anyone. They were half of one commit whose
  other half (the device descriptor accessors and `can_send_n`) Vino does use,
  so that commit was split and retitled `rust: usb: expose device descriptor
  fields and queue readiness`.

**Threading.** No series is sent `In-Reply-To` its v2.
`Documentation/process/submitting-patches.rst` says not to attach a new revision
of a multi-patch series to the old thread, because multiple versions become an
unmanageable forest of references. Each cover letter carries a `v2:` lore link
instead.

**Cross-links.** The cover letters name the whole posting and link the series
already sent, so they must be generated one at a time: send, record the
Message-Id in `tools/v3-message-ids.txt`, re-run
`tools/regenerate-patches.sh`, send the next. `tools/send-series.sh` prints that
reminder after every `--send`.

**No RFC prefix.** v1 and v2 went out as `RFC PATCH`. v3 does not: the driver
works on three generations of hardware, and the remaining question is review,
not whether the approach is viable.

**Fixed while cutting v3:**

- `rust/kernel/crypto.rs` and `rust/kernel/drm/kms/framebuffer.rs` each had an
  import that is unused when the feature it serves is configured off. Both are
  now `#[cfg]`-gated; the tree builds warning-clean with the features on and
  with them off.
- Vino's Kconfig help named two docks when three are supported.
- Two commit messages wrapped past 75 columns.

`checkpatch --strict` is clean across all six series apart from Rust 100-column
notes on 13 string literals and trailing comments, which rustfmt does not
reflow, and the usual `MAINTAINERS needs updating?` for new files.

---

## Everything below is the history that led to the cut above

Superseded where it disagrees. The nine-series table, in particular, is what v3
looked like before the "does it enable Vino" rule was applied.

Status was rechecked on 2026-07-28 against the public patch threads and the
available remote branch tips. Only v2 has been posted, so the next posting is v3.

⭐ **v3 is nine subsystem series, and the branch is ordered to match.** They were interleaved
across twenty runs before, so none could be posted on its own; the reorder left the tree
byte-identical. `tools/regenerate-patches.sh` exports them to `patches/<group>/`, each numbered from
0001 with its own cover letter:

| group | patches | list |
|---|---|---|
| `sched-fair` | 1 | lkml |
| `rust-core` | 12 | rust-for-linux |
| `rust-crypto` | 2 | linux-crypto + rust-for-linux |
| `rust-usb` | 5 | linux-usb + rust-for-linux |
| `rust-drm` | 25 | dri-devel |
| `rust-firmware` | 1 | linux-kernel + rust-for-linux |
| `drm-tyr` | 1 | dri-devel |
| `drm-vino` | 13 | dri-devel |
| `drm-evdi` | 1 | dri-devel |

⛔ **The driver was 33 patches of development history and is now 13 plus its documentation.** That
history carried a revert pair, a module parameter added and later deleted, selftest corrections and
fixes to patches earlier in the same series. What is exported introduces the driver in the order it
is understood -- wire framing, USB transport, crypto and HDCP, control plane, dock profiles, codec,
bring-up, KMS, activation and scanout, firmware, the USB frontend, the build, the docs -- plus the
KMS bindings it needs, which live in the `rust-drm` group where they belong.

⭐ **A fix to a commit that is not upstream yet belongs *in* that commit** (2026-08-26). The sample-
depth work landed as six follow-ups on top of the series -- reporting the depth, publishing the
plane modifier, driving the link from `max bpc`, scaling the entropy coder's ceilings, keying the
strip cache, pricing the shared budget -- and every one of them was a fix to a commit the same
series introduces. They are folded into those commits: each file's change goes to the commit that
added the file, so a reviewer sees the driver as it is meant to be read rather than the order it
was debugged in. The KMS binding they need moved ahead of the driver, into `rust-drm`.

## Current base

Rebased 2026-08-09 onto the `drm-rust-next` tip. The base is now that branch
head itself rather than the merge commit inside it: `drm-rust-next` has not
merged a newer `drm-next` since, so the `drm-next` content is unchanged and
building our own merge would only invent an integration point the DRM Rust
maintainers do not test.

- series base: `4c9ba407018e8deb06dbc643112bac8f40404f95` (`drm-rust-next`,
  2026-08-06);
- `drm-next` parent reached through that tree:
  `ea97ab2759506d9a818ffed1009bde01062b4091` (unchanged);
- previous base: `0755a4e3e809610a14befc9ad28d35e2e460da68`, 28 commits back;
- Lyude Paul's `rvkms-slim`: no newer complete revision was found after the
  imported `25bc8cc7e97fd292bea4b77354aaac7eba6c5385`.

All 114 patches replayed with zero conflicts and `git range-diff` reports no
patch changed, so the rebase moved the base and nothing else.

⚠ `drm-next` itself has moved on to `e4d41a34eedb808d423d73cae8d8601be32f307e`.
We deliberately do not follow it directly: the KMS layer this series is built on
lives only in `drm-rust-next`, and that tree picks up `drm-next` on its own
schedule.

## 2026-08-23 re-check, and the prerequisite message-ids

Every message-id below was fetched and confirmed to resolve (HTTP 200 on
`lore.kernel.org/all/<id>/raw`) rather than reconstructed from a numbering pattern.

| Dependency | Commits we carry | `prerequisite-message-id:` |
|---|---:|---|
| Lyude Paul, `[RFC v3 00/33] Rust bindings for KMS + RVKMS` | 43 | `20250305230406.567126-1-lyude@redhat.com` |
| Colin Braun, `[RFC PATCH 0/4] rust: usb: add usb request block abstractions` | 3 | `20260712-urb-abstraction-v1-v1-0-9fa011634ead@gmail.com` |
| Danilo Krummrich, `[PATCH v2 0/6] workqueue: OwnedQueue, ScopedQueue and ScopedWork` | 3 + 1 + 1 | `20260807165252.3849875-1-dakr@kernel.org` |

### The interrupt prerequisites have started landing upstream

Boqun Feng's `irq,spin_lock: Add counted interrupt disabling/enabling` was merged
into the **tip `locking/core`** branch by Peter Zijlstra on 2026-08-10, as commit
`e901c1510e24726dcbd6340ee927b3ac8b992043`.

That is one patch of the group we carry as "interrupt prerequisites" (Boqun's nine, plus
Joel Fernandes' NMI counter, Heiko Carstens' s390 enablement, and Lyude's openrisc include
and KUnit test). It postdates our base by a day, so it is not in `drm-rust-next` yet.

⭐ This is the outcome worth watching for: when it reaches mainline and `drm-rust-next`
rebases, **those commits should be dropped from our series rather than carried**, and no
prerequisite trailer is needed for them at all. Re-check the whole group before cutting
v3; do not assume the rest of the series landed just because this patch did.

## 2026-08-09 re-check

Lists and prerequisite trees were rechecked on 2026-08-09.

**No new replies to v1 or v2.** The last message on any of the five posted
threads is 2026-07-07. Everything on them is already dispositioned below.

**Still not upstream, still carried:** Lyude Paul's KMS layer is not in
`drm-rust-next` (no `rust/kernel/drm/kms*`), and she has posted no newer Rust
KMS series, so the 43 imported commits stay. Colin Braun's URB RFC is unchanged
since its 2026-07-12 v1. Igor Korotin's I2C adapter work is unchanged since
2026-01-31.

**Superseded, migration pending:** Danilo Krummrich has taken over the workqueue
work and posted `[PATCH v2 0/6] workqueue: OwnedQueue, ScopedQueue and
ScopedWork` (2026-08-07). It supersedes three commits we carry — Alice Ryhl's
three creation patches, our own `rust: workqueue: make OwnedQueue thread-safe`
(v2 adds `Send + Sync` for `OwnedQueue` directly), and Onur Özkan's `cancel_sync`
support, which v2 drops in favour of `ScopedWork`.

The swap is not mechanical: Vino calls `Work::cancel_sync()` in seven places
across `drm_sink.rs` and `vino.rs`, and `ScopedWork` replaces that idiom with a
work item that cancels synchronously on drop. It is the better fit for what our
teardown does by hand, but it is a driver change that needs hardware validation.
The series is also still moving — Danilo self-reported fixes to v2 4/6 (wrong
default flag, `WQ_PERCPU` vs `WQ_UNBOUND`) and 6/6 (a missing `drop_in_place`)
within a day of posting, so a v3 is likely. Take it when v3 lands, in one commit
that swaps the prerequisites and migrates the call sites together.

**Watch, do not act yet:** Alvin Sun's `[PATCH v9 00/10] Fix missing fops.owner
in Rust DRM/misc abstractions` fixes the same bug as our `rust: drm: pin the
owner while DRM files remain open`, from the other end: it gives
`ModuleMetadata` a `THIS_MODULE` const and drops the `module!` static, rather
than threading an owning module through `UnregisteredDevice::new()` the way we
do. It is at v9 and being reviewed by Petr Pavlu, Miguel Ojeda and Gary Guo, but
it is not in `drm-rust-next` — upstream `UnregisteredDevice::new()` still takes
no module — so our patch is still load-bearing. Drop ours when theirs lands; do
not post ours into the same area meanwhile.

Joel Fernandes / John Hubbard's
`rust: sync: completion: add wait_for_completion_timeout()` (2026-08-07, patch
1/17 of a nova-core interrupt series) adds the same API as our
`rust: sync: completion: add single-shot and timed operations`. Unmerged, and
inside a series about something else; drop ours if and when theirs lands.

`rust: firmware: add request_into_buf()` reached `drm-rust-next` this cycle and
is *not* a duplicate of our firmware-upload abstraction — that binds
`firmware_upload_register()`, the push direction, which upstream still lacks.

**Nothing else is coming this cycle.** Danilo's `[GIT PULL] DRM Rust changes for
v7.3-rc1` (2026-08-08) is exactly the base we rebased onto: `RegistrationData`
and `RegistrationGuard`, TLV firmware and GSP consolidation for nova-core, and
firmware/MCU boot for Tyr. It mentions no KMS, workqueue, completion or
`fops.owner` work at all, so none of the four collisions above resolve
themselves in v7.3-rc1.

The full integration branch is not a proposed single-list posting. Its group
manifests separate existing work and independently owned subsystem APIs from
the EVDI and Vino consumers.

## v1/v2 feedback carried forward

### Rust DRM/KMS

The v2 series obscured Lyude Paul's attribution and mixed her work with
unrelated adaptations. The rebuilt history preserves her 37 KMS commits in
order and patch-identical to the imported source. Her messages and original
tags are untouched, and no Mike or assistant trailer appears on them.
Adaptations to the current DRM APIs and all later safety extensions are
separate Mike-authored commits.

Consumer code uses the safe KMS object layer and validated shmem scanout views.
It has no raw C KMS calls, direct bindings, or reconstructed object lifetimes.

### USB

The v2 review distinguished a bound interface from the narrower interval in
which I/O is legal. The rebuilt code models that interval as an adapter-owned,
revocable interface capability across probe, suspend, reset, resume, and
disconnect.

Colin Braun's current URB RFC overlaps this work. Its first three commits are
retained unchanged, followed by USB-owned additions for typed revocable I/O,
reusable persistent queues, topology lookup, and removal notification. Vino
does not carry a private URB implementation.

### Crypto

The current implementation does not add a Vino-specific RSA primitive. It uses
the kernel crypto implementations for AES, CMAC, SHA-256, HMAC-SHA256, and RSA,
with safe Rust wrappers in crypto-owned patches. RSA-OAEP padding and HDCP key
handling use a memory-wiping secret type and the existing `crypto_akcipher`
facility.

### I2C

Igor Korotin's active Rust I2C adapter work was checked. Its provider-lifetime
issue is not solved privately in Vino. The kernel driver therefore does not
register a downstream I2C adapter in this revision. Chimera retains its
userspace-only DDC/CI vendor transaction so protocol research can continue
without creating a competing kernel API.

### Vino and EVDI

Automated findings from the previous Vino posting were rechecked and corrected,
but are not treated as human acceptance. EVDI now uses a conventional C UAPI
header, generated Rust bindings, safe compat translation, normal DRM plane
geometry, and the shared owned shmem view. The Vino match table lists the two profiles
actually validated on hardware: the Dell D6000 `17e9:6006` and the DL-7400
`17e9:7000`. Both are described by data the driver reads rather than by
open-coded model checks, and a third device needs its own profile and its own
evidence.

## Reused external work

The integration history deliberately retains:

- Lyude Paul's Rust DRM/KMS commits;
- Colin Braun's current USB URB RFC foundation;
- Alice Ryhl's v4 owned-workqueue series;
- Onur Özkan's `cancel_sync` workqueue patch;
- scheduler, locking, architecture, and preemption prerequisites under their
  original authors.

No newer revisions of those selected series were found during the 2026-07-28
check. They must still be posted and reviewed through their own maintainers;
their presence in the integration branch is not a claim of acceptance.

## Patch authorship

Third-party commits retain their authors, messages, and trailers. Every
Mike-authored kernel patch names the assistants that worked on it and then, last,
the only sign-off:

```text
Assisted-by: Claude:claude-opus-5
Assisted-by: Codex:gpt-5
Signed-off-by: Mike Lothian <mike@fireburn.co.uk>
```

This follows `Documentation/process/coding-assistants.rst`'s
`AGENT_NAME:MODEL_VERSION` form: the assistant and model are identified, while
only Mike supplies the DCO sign-off. The model version legitimately differs
between patches written months apart, so `tools/validate.sh` checks the shape of
the block and that the sign-off is last, not a fixed string.

## Series shape

The branch contains 108 commits in seven contiguous review groups, exported to
`patches/kernel/` with a manifest per group:

| Group | Patches | Ownership |
|---|---:|---|
| `interrupt-prerequisites` | 18 | scheduler, locking, architecture, Rust |
| `kms-lyude` | 37 | Lyude Paul's original Rust KMS work |
| `drm-crypto-platform` | 18 | DRM, crypto, driver core |
| `usb` | 7 | USB and Rust |
| `rust-runtime-drm` | 22 | Rust core, timer/workqueue, FPU, time, DRM |
| `evdi` | 1 | DRM |
| `vino` | 5 | DRM and USB |

The five Vino patches are control protocol, codec, KMS/scanout, the USB driver,
and the documentation. Each introduces its subject once, in the state it is in:
the development history — bring-up chronology, experimental switches,
reversions, temporary workarounds — is folded away, and the branch it was folded
from is kept as `backup/vino-pre-v3-fold-20260804-2051` rather than published as
review material.

Generic facilities are introduced in their owning subsystem rather than hidden
in the driver. Two were added this round: a safe kernel-FPU section guard, which
the optional AVX2 transform needs, and `ktime_get_real_seconds()`, which replaced
the driver's one remaining raw `bindings::` call.

## References

- [Rust crypto v2](https://patchew.org/linux/20260703030056.2763-1-mike%40fireburn.co.uk/)
- [Rust KMS v2](https://patchew.org/linux/20260703030123.2814-1-mike%40fireburn.co.uk/)
- [Vino v2](https://patchew.org/linux/20260617151249.2937-1-mike%40fireburn.co.uk/)
- [Colin Braun's USB URB RFC](https://patchew.org/linux/20260712-urb-abstraction-v1-v1-0-9fa011634ead%40gmail.com/)
- [Alice Ryhl's v4 workqueue series](https://patchew.org/linux/20260312-create-workqueue-v4-0-ea39c351c38f%40google.com/)
- [Igor Korotin's Rust I2C adapter RFC](https://patchew.org/linux/20260131-i2c-adapter-v1-0-5a436e34cd1a%40gmail.com/)
