# v4 pre-send TODO

What is left before the six series go out as v4.

Audited 2026-09-01 against `vino` @ `c63829528980..`, base `c63829528980`
(`Merge tag 'v7.3-rc1' into drm-rust-next`). Worked the same day; see
**Done 2026-09-01** at the bottom for what moved and what that corrected.

Status keys: `[ ]` open, `[~]` in progress, `[x]` done, `[?]` needs a decision from Mike.

---

## Verified good -- do not re-check

- [x] No evdi/revdi contamination. Four prose mentions survive, all deliberate.
- [x] No cross-series Makefile/Kconfig context leakage. The v3 bug where
  `drm-evdi/0001` carried the Vino Makefile entry as context is gone.
- [x] `release_driver` dropped from `rust-usb`, as promised to Krummrich.
- [x] Generic akcipher wrapper replaced by an RSA-only API (`crypto/rsa.rs`).
- [x] `aes_enckey_zero` helper gone; one safe `crypto::zeroize()` remains.
- [x] `RUST_CRYPTO_LIB_AES`/`_SHA256` declared in the patch that adds the code.
- [x] hrtimer `restart()` restored -- it is called from `enable_vblank`.
- [x] Driver hygiene: no `unsafe`, no `bindings::`, no `TODO`/`FIXME`, no non-ASCII.
- [x] MAINTAINERS entry present.
- [x] `make LLVM=1 rustfmtcheck` clean, checkpatch clean apart from Rust
  string-literal line lengths.
- [x] Branch builds warning-free; `vino.ko` produced.

---

## 1. Blockers

### 1.1 `[x]` Drop `rust-drm` "pin the owner while DRM files remain open" -- DONE

### 1.2 `[x]` Version labels and changelogs -- DONE

### 1.3 `[x]` Cover letters misdated the base tree -- DONE

### 1.4 `[x]` README dependency column contradicted the cover letters -- DONE

### 1.5 `[x]` `tools/check-series.sh` was dead -- DONE, and it now passes

### 1.6 `[x]` ~~`integration/base-20260901` does not exist~~ -- **WAS WRONG**

It exists, as a **tag**. `git branch -a --list` does not list tags, which is how
the original audit missed it. Nothing to fix. Recorded because the same mistake
is easy to repeat: check `git for-each-ref`, not `git branch`.

---

## 2. Open review comments from v3 -- owed to reviewers

### 2.1 `[x]` Use `aes_ctr()` instead of hand-rolled CTR loops (Eric Biggers) -- DONE

Done 2026-09-01. `kernel::crypto::aes_ctr_128()` binds the library `aes_ctr()`
through a helper (it takes the same transparent union as `aes_encrypt()`), and
both `cp.rs` loops are gone. `RUST_CRYPTO_LIB_AES` selects `CRYPTO_LIB_AES_CTR`;
the declaration lives in `crypto/aes-ctr.h`, not `crypto/aes.h`. The rust-crypto
0001 commit is retitled to name CTR, and both covers say so.

WARNING -- one semantic note, commented in `cp.rs` rather than worked around:
`aes_ctr()` advances a full 128-bit counter, while the protocol counts in a
32-bit field with zeroes above it. Proved by enumerating the counter blocks:
identical for every reachable value, diverging only once `seq + nblocks` crosses
2^32, which is 64 GiB of control traffic in one session. The decrypt side
verifies the Dl3Cmac before decrypting, so a wire-supplied `seq` cannot reach it.

WARNING -- **`seal_livemac_roundtrip` would not have caught a keystream change**:
seal and open share the model, which is the round-trip trap. The counter-block
enumeration is the real evidence. **A hardware run is still wanted:** this is
byte-exactness with a dock, and it has not been on hardware.

### 2.2 `[x]` The bare-block-cipher caller -- ANSWERED by Biggers, 2026-08-31

⚠ The answer had been sitting unread in the mailbox. Biggers replied on
2026-08-31 (`<20260831170546.GA239479@google.com>`):

> I'm not currently planning to make the crypto library expose AES functions
> that take raw keys, since computing the AES round keys is fairly slow and
> most users use their AES keys multiple times. So in this case I guess keep
> planning to use the sequence that is already supported: aes_prepareenckey()
> + aes_encrypt() + memzero_explicit(). For CTR mode, use aes_prepareenckey()
> + aes_ctr() + memzero_explicit().
>
> But if you're using either key multiple times you should call
> aes_prepareenckey() just once and cache the result, as that is what it is
> for.

Two actions fall out, both **open**:

- `[ ]` Cut the rest of `Aes128` from rust-crypto 0001. The HDCP 2.2 dKey
  derivation keeps `aes_prepareenckey()` + `aes_encrypt()` +
  `memzero_explicit()` directly. No lib/crypto change is being asked for.
- `[ ]` **`aes_ctr_128()` prepares the key on every call, and that is wrong
  for this driver.** The control-plane key is fixed for the life of a
  session and every sealed message uses it, so the schedule is rebuilt per
  message. Hold the prepared key with the session instead. This is a change
  to the binding's shape, not just its caller: the helper takes a raw key
  today and needs to take a prepared one.

### 2.3 `[?]` Answer Krummrich on the revocable I/O window

Genuinely open: can the lifetime and higher-ranked machinery express a window
that **closes and reopens several times within one bind** (suspend/resume,
pre_reset/post_reset), given revocation is one-way? The answer decides whether
the type stays USB-specific or is rewritten on Devres. The hand-rolled
open/closed flag and wait-for-quiescence should go regardless.

The rust-usb v4 cover now states this openly rather than presenting the design
as settled.

### 2.4 `[~]` Send the outstanding replies -- DRAFTED 2026-09-02, awaiting Mike

⛔ **The `X-Status` lines in `outgoing/v3-replies/` were wrong in both
directions.** Checked against `[Gmail]/Sent Mail`, which is the only thing
that actually knows. What was really sent on 2026-08-31:

| draft | sent | as |
|---|---|---|
| 01 hindborg 7/9 | 17:44 | **multipart/alternative** -- lore rejected it |
| 02 hindborg 2/9 | 17:45 | **multipart/alternative** -- lore rejected it |
| 04 biggers 1/2  | 17:51 | **multipart/alternative** -- lore rejected it |
| 06 correction   | 18:01 | ⛔ **not the correction** -- the 18:01 message is only "Fudge / I have to keep remembering to enable plain text in gmail" |
| 03 krummrich    | 18:04 | text/plain, real content ✅ |
| 05 biggers 2/2  | 18:04 | text/plain, real content ✅ |

So 05 was **not** unsent, and the correction **was** still unsent -- and it
matters: Andreas's own reply quotes "I will drop this patch rather than
carry an API with no user", and v4 keeps it.

**Three plain-text drafts are now in `[Gmail]/Drafts`**, threaded on the real
Message-IDs read out of the mailbox:

1. `Re: [PATCH 7/9] rust: hrtimer: expose interrupt state in hard callbacks`
   -- verbatim resend, so lore gets it at all.
2. `Re: [PATCH 2/9] rust: hrtimer: add ArcHrTimerHandle::restart` -- the
   correction. **Send this one first.**
3. `Re: [PATCH v3 1/2] rust: crypto: ...` -- new, answering Biggers's
   2026-08-31 reply and conceding the cached-key point (2.2).

⚠ **02 is deliberately not resent.** Its content ("I will drop this patch")
is now known to be wrong, and neither it nor the correction ever reached
lore, so the archive would gain a false statement followed by its
retraction. Draft 2 replaces it. ⚠ There is also an **older multipart/mixed
draft on the same thread** left in place -- do not send that one.

⚠ 04 is not resent either: Biggers answered it on 31 Aug, so resending would
re-ask a question already answered. Draft 3 replaces it.

### 2.5 `[?]` Ask Biggers where the RSA API should live

A narrow Rust wrapper over the existing `rsa` transform, or something in
lib/crypto? Only operation needed is RSAES-OAEP-SHA256 public-key encryption
with a caller-supplied seed. The lib/-Kconfig-vs-rust/-code placement follows
Miguel's thread.

### 2.6 `[x]` `device_release_driver()` -- ANSWERED by Gary Guo, 2026-08-31

Also unread until now (`<DL3AO2K8IS07.3QB2IM7YJL8WT@garyguo.net>`), on the
`remove_all` question in the sent reply to Krummrich:

> EVDI is not an upstream driver, so the fact that it supports something
> isn't really a justification of adding a new API interface that is already
> covered by sysfs unbind.

That settles it. The binding stays dropped, and the "should it come back
later with its user attached" question is closed -- do not reopen it.
`/sys/bus/usb/drivers/vino/unbind` is the answer.

---

## 3. Reviewability

### 3.1 `[x]` Split the five largest `drm-vino` commits -- MEASURED, NO ACTION

**Do not propose this again.** It was never asked for: no reviewer raised commit
size in v3, and the only hit for "size" across all six reply drafts is a `size_t`
in a quoted hunk. The v3 complaint was patches being out of *order*, which is
§3.2, and content that should not have been posted at all, which is §1.

Vino's sizes are ordinary for a new DRM driver. Added lines in the introducing
series, measured in this tree:

| driver | commits | total | largest |
|---|---|---|---|
| xe | 1 | 40575 | 40575 |
| imagination | ~20 | ~33000 | 6531 regs, then 4756 / 4015 / 3832 / 3438 |
| **vino** | **13** | **24119** | **5431 / 5130 / 3421 / 3083 / 2348** |
| panthor | 11 | ~12825 | 3552 / 2870 / 1865 |
| tyr, nova | 1 | 650, 288 | skeleton, grown in-tree afterwards |

Vino is between panthor and imagination on totals and peaks, and far more
granular than xe. Its largest commit is 14% above imagination's largest logic
commit, which is not a different category. Splitting further would make it the
most finely divided new DRM driver in the tree.

What does still matter is that each commit is a coherent unit a reviewer can
hold in their head and that bisect can test -- that is §3.2 and §3.5, not a line
count.

### 3.2 `[x]` Reorder `rust-drm` logically -- DONE

22 commits -> 27, reordered into groups. Verified at each step by comparing the
applied tree against the pre-work branch: every split produced a byte-identical
tree, and the reorder differs only by `enable_fb_damage_clips` moving earlier
within `plane.rs` (21 lines out, the same 21 in), which follows its commit
moving. Builds warning-clean, `rustfmtcheck` passes, `check-series.sh` applies
all six series in send order and reproduces the branch.

The order now runs: fixes to Lyude's series (4) -- registration data -- modes
(3) -- connector (5) -- CRTC (6) -- plane (5) -- framebuffer -- generic DRM (2),
with the carried `drm-tyr` patch at the end instead of buried in the middle.

**Four mistitled or mixed commits found in the audit, all fixed:**

- `add typed color and rotation properties` put `ColorLut` (CRTC) and
  `Rotation`/`BlendModes` (plane) in one commit. This was *why* the order could
  not be fixed by moving commits alone -- `expose CRTC mode changes` depends on
  the CRTC half. Split into `add CRTC colour lookup table entries` and `add the
  plane rotation property`.
- `BlendModes` was defined eight commits away from `create_blend_mode_property`,
  its only user. Moved into that commit.
- `expose CRTC mode changes` also added a whole CRTC colour-management API
  (3.3). Split.
- `add common state and connector helpers` was a grab-bag of five unrelated
  things, and worse, a chunk of it was **rustfmt and doctest fixes to Lyude's
  files** hidden under that title. Split four ways, with the formatting fixes
  now stated as such and sitting next to the adapt commit.
- `expose a connector's requested link depth` also introduced
  `FORMAT_MOD_LINEAR`, used only by the framebuffer commit. Moved there.

⚠ **A pure reorder is not conflict-free.** Six conflicts, and the dangerous ones
were where git absorbed *context* from a commit that now comes later: taking
"theirs" wholesale silently pulled `create_rotation_property` and
`enable_fb_damage_clips` into the wrong commits, and one resolution dropped a
closing brace that only the build caught. Check every "both sides are
additions" resolution against what actually belongs at that point.

### 3.3 `[x]` Split `rust: drm: expose CRTC mode changes` -- DONE

Now `expose CRTC mode changes` (just `mode_changed()`) and `add CRTC colour
management` (`ColorCtm`, `enable_color_mgmt`, `degamma_lut`, `ctm`).

The subject-vs-diff audit ran over all 27 rust-drm commits, not just this one;
the results are in 3.2 above. One body restated its subject (`adapt Lyude's KMS
series`) and was reworded.

### 3.4 `[~]` Split the two large `rust-usb` patches -- CHECKED, one half blocked

Checked against the feedback the way 3.1 should have been. Nobody asked for a
split here either, but unlike 3.1 the two commits do each introduce two
abstractions at once, and 698 and 650 added lines are roughly twice the in-tree
norm for a Rust abstraction commit (pci 315, auxiliary 303, platform 215, devres
191, dma 389, drm gem 351; configfs at 1057 is the outlier). So the concern is
real, but it is coupling, not line count.

- **0001, revocable typed interface I/O -- BLOCKED, do not split yet.**
  Krummrich has asked for exactly this code to be rewritten on Devres and
  higher-ranked lifetimes, or else justified as USB-specific (2.3). Splitting a
  commit that may be substantially rewritten is wasted work. Wait for the answer.
- **0002, reusable URBs and persistent bulk queues -- split it.** Two
  abstractions, and its own commit message is already two paragraphs saying so.
  The queues build on the URBs, so the order is forced and the split is clean.
  Independent of the 2.3 question.

### 3.5 `[?]` Move the Kconfig/Makefile earlier in `drm-vino`

0012 of 13 adds `CONFIG_DRM_VINO`, so patches 0001-0011 add ~20k lines that no
normal kernel build compiles, and `git bisect` has nothing to test. Do not
contort the design with stub scaffolding just to satisfy the rule -- if it needs
fake stubs, leave it and say why in the cover.

---

## 4. Dead API -- `[x]` DONE, with three corrections to the original audit

Removed, each folded into the commit that introduced it: `add_cvt_mode()`,
`any_mode()`, `UnregisteredCrtc::enable_gamma()`, `Framebuffer::to_aref()`,
`Io::interrupt_recv()`, `Endpoint::max_packet_size()`, the `InterruptIn`
endpoint kind and the `Endpoint::max_packet` field it left write-only.

**Three things the original audit got wrong, all caught by checking before cutting:**

1. ⚠ "`rust-drm` 0020 -- drop the entire patch" was **wrong**. It also adds
   `attach_hdr_output_metadata_property`, `attach_colorspace_property` and
   `attach_max_bpc_property`, all used by the driver -- `max_bpc` is what makes
   10 bpc work on Navarro. Only `add_cvt_mode` was dead. The commit was retitled
   instead.
2. ⚠ The isoc and interrupt **pipe constructors are Colin Braun's**, not ours;
   our patch only generalises their signature. Deleting three of his
   constructors while keeping the rest would have been worse than leaving them.
   Only `interrupt_recv` and `max_packet_size` were ours.
3. ⚠ `FramebufferVMap` is **not** dead -- it is `vmap()`'s return type, and
   `vmap()` is used by vino and tyr. Only `to_aref()` was dead.

The lesson for the next sweep: a reference count of 2 is not proof of life and a
count of 1 is not proof of death. Check the call, and check the author.

Sweep command, for the respin:

    for nm in $(grep -oP '^\+\s*pub (const |unsafe )*(fn|struct|enum|trait|type) \K\w+' patches/*/0*.patch | sort -u); do
        [ "$(grep -rhow "$nm" rust/kernel/ | wc -l)" -le 1 ] && echo "candidate: $nm"
    done

---

## 5. Decisions for Mike

### 5.1 `[x]` `trace_crypto` -- DONE: now just the `debug` parameter

Mike's call: the key dump is not its own module parameter and not its own
Kconfig. The driver's existing `debug` parameter is the only switch, both sites
are plain `vino_debug!` like every other diagnostic, and the cover says nothing
about it.

WARNING -- **Rust's `pr_debug!` is not C's**, so do not reach for it here. Both
`pr_debug!` (print.rs:398) and `dev_dbg!` (device.rs:376) are
`if cfg!(debug_assertions)`; there is no `dyndbg=` control and no `_ddebug`
descriptor behind any Rust logging macro. Using them would tie the key dump to
the global `CONFIG_RUST_DEBUG_ASSERTIONS`, with no runtime way to turn it off.
`vino_debug!` is `pr_info!` under `debug_enabled()`, so nothing in vino's
logging depends on that config either way.

### 5.2 `[x]` Automatic firmware flash at probe -- DECIDED: keep it

Mike's call, 2026-09-01: probe-time flashing stays. The cover now carries the
argument instead of leaving it to be challenged -- why probe rather than
userspace (a dock whose shipped firmware cannot enumerate its connectors has no
display to prompt on and looks output-less to userspace, so deferring to
userspace means the hardware that most needs the update cannot ask for it), what
bounds it (forward-only, no downgrade, no rewrite of the running version,
per-dock attempt counter surviving re-enumeration, nothing at all without an
image in /lib/firmware/vino), and what does not (DFU here has no upload, so
there is no readback and nothing to roll back to).

### 5.3 `[?]` Send strategy

Do not fire all six at once again. Get the infrastructure moving first,
coordinate the Lyude fixes with Lyude, let the driver follow. Consider labelling
drm-vino **RFC v4** rather than a merge candidate.

---

## 6. Small

- [x] KUnit count is computed from the tree, not typed (was 97, is 98).
- [x] The "no module parameters" claim now says what it means: no parameter
  selects a profile or a code path.
- [x] The one body restating its subject was `adapt Lyude's KMS series`, now
  reworded. Checked all 27 rust-drm commits, not just the one.
- [x] ~~Wrap the commit-message lines over 75 columns.~~ **Nothing to do.** The
  only two over-length lines in the whole branch are a `Fixes:` trailer and a
  `Link:` trailer, which are exempt and must not be wrapped.
- [ ] Send `sched-fair` separately to Peter Zijlstra / Ingo Molnar once v4 is out.
- [ ] Keep `Assisted-by: Claude:claude-opus-5` consistent; never strip one from a
  commit Claude actually wrote.

### 6.1 `[x]` The push guards from the WORKLOG are off -- deliberate, leave them

`WORKLOG.md` records `git remote set-url --push <remote> DISABLED` on every
remote. They are off, and Mike confirmed on 2026-09-01 that they stay off: they
were a precaution while the Synaptics question was open, and it no longer is.
The `sendemail.smtpServer` guard is still in place and `send-series.sh` still
refuses `--send` without `--smtp-server`, which is the one that matters.

Still true that `gitlab Vino:main` is protected against force-push, so never
amend an already-pushed superproject commit.

---

## Suggested order of work

1. §5.1 / §5.2 decisions, so the code settles before the split.
3. §3 splits and reordering, including §3.3 and the subject-vs-diff audit.
4. Re-run `tools/check-series.sh --build` after each step. Nothing goes out
   until it passes.
5. Regenerate and re-read the covers.
6. §2.4 send the outstanding replies -- ideally before v4 lands.

---

## Done 2026-09-01

All verified by `tools/check-series.sh --build`: the six series apply in send
order over `integration/prereqs-20260901`, reproduce the branch tree exactly, and
the result builds and produces `vino.ko`. Branch builds warning-free directly too.

**Series shrank from 98 commits to 97; rust-drm from 23 patches to 22.**

- Dropped `rust: drm: pin the owner while DRM files remain open`. Upstream's
  `OwnerModule` (via `#[vtable]`, auto-inserting `type OwnerModule = LocalModule`)
  already stamps `file_operations::owner` at `device.rs:199`. Vino was passing
  `&<LocalModule as ModuleMetadata>::THIS_MODULE` by hand -- the same module, the
  same way. The v3 cover had said this should go the moment Alvin Sun's fix
  landed; it has. The call-site fix is folded into the frontend commit.
- Removed seven dead public API items (see §4), each folded into its own commit.
- Retitled `add synthesized CVT connector modes` to `attach the connector colour
  properties`, which is what it does now.
- Fixed the version machinery: `previous_ver()` derives the prior revision from
  the reroll count, so covers now read `Changes since v3:` / `v3: <link>` for the
  four at v4 and `Changes since v1:` / `v1: <link>` for the two at v2. The
  hardcoded `printf 'v2: ...'` is gone.
- Wrote real v3->v4 changelogs for all six, naming the reviewer each change
  answers, and stating openly what is still unsettled (2.1, 2.3) rather than
  leaving it to be found.
- `depends()` now names the third-party prerequisite per series, matching what
  the covers already declared.
- Corrected the base date: `c63829528980` is the drm-rust-next tip of
  **2026-08-31** merging v7.3-rc1, not 2026-08-06.
- KUnit counts computed from the tree; README points at `v4-message-ids.txt`.
- Rewrote `tools/check-series.sh`: applies each series in send order with `am -3`,
  then the two carried-but-not-posted ones, compares the tree against the branch,
  and with `--build` compiles the result and checks `vino.ko` exists. Added the
  `integration/prereqs-20260901` tag (base + the 44 third-party commits, which
  cherry-pick cleanly) as the reference a reviewer with the prerequisites has.
- Fixed two rustfmt regressions my own removals introduced, folded into the
  commits that caused them.

Backup of the pre-cleanup branch: `backup/vino-pre-v4-cleanup-20260901`.
