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

### 2.1 `[ ]` Use `aes_ctr()` instead of hand-rolled CTR loops (Eric Biggers)

**The largest remaining item.** Mike's reply promised it; not done. The base has
`aes_ctr()` (`lib/crypto/aes.c:1119`, `EXPORT_SYMBOL_GPL` at `:1144`) and the
driver still hand-rolls two CTR loops:

- `drivers/gpu/drm/vino/cp.rs:648` (`seal_livemac`)
- `drivers/gpu/drm/vino/cp.rs:1129`

Both are textbook SP 800-38A: 8-byte nonce, four zero bytes, 32-bit big-endian
counter starting at `seq`. `aes_ctr()` replaces both outright.

Needs, in order:
1. A Rust binding for `aes_ctr()` in `rust/kernel/crypto.rs`.
2. Both `cp.rs` call sites converted.
3. Then decide the follow-up below, because it changes how much of `Aes128` survives.

The rust-crypto cover already flags this as open on-list, so the changelog does
not need rewriting when it lands -- just tightening.

### 2.2 `[?]` The one remaining bare-block-cipher caller (question is on-list)

Doing 2.1 leaves exactly one caller of the block cipher: the HDCP 2.2 dKey
derivation (`crypto.rs:17`), a single AES-128 ECB block. Either lib/crypto grows
a one-shot single-block encrypt, or that caller keeps `aes_prepareenckey()` /
`aes_encrypt()` directly. Biggers has not answered. Cut the rest of `Aes128`
from rust-crypto 0001 once he does.

### 2.3 `[?]` Answer Krummrich on the revocable I/O window

Genuinely open: can the lifetime and higher-ranked machinery express a window
that **closes and reopens several times within one bind** (suspend/resume,
pre_reset/post_reset), given revocation is one-way? The answer decides whether
the type stays USB-specific or is rewritten on Devres. The hand-rolled
open/closed flag and wait-for-quiescence should go regardless.

The rust-usb v4 cover now states this openly rather than presenting the design
as settled.

### 2.4 `[ ]` Send the outstanding replies

- `06-hindborg-restart-correction.txt` -- marked NOT SENT. **This one matters**:
  Andreas currently believes the `restart()` patch is being dropped, and v4
  keeps it. Send before v4 lands.
- `05-biggers-2of2-rsa-and-kconfig.txt` -- no `X-Status`, appears unsent.
- Resend 01, 02 and 04 in **plain text**; lore rejected them as HTML.

### 2.5 `[?]` Ask Biggers where the RSA API should live

A narrow Rust wrapper over the existing `rsa` transform, or something in
lib/crypto? Only operation needed is RSAES-OAEP-SHA256 public-key encryption
with a caller-supplied seed. The lib/-Kconfig-vs-rust/-code placement follows
Miguel's thread.

---

## 3. Reviewability

### 3.1 `[ ]` Split the five oversized `drm-vino` commits

| patch | added | proposed split |
|---|---|---|
| 0009 activation + scanout | **5439** | activation / streams / framebuffer+damage / encode+submit |
| 0008 KMS + atomic path | **5137** | mode objects / atomic state+check / properties+damage / vblank |
| 0006 video codec | **3428** | transforms / coding / record+framing |
| 0004 encrypted control plane | **3090** | envelope+sealing / messages+EDID / control+cursor |
| 0007 session bring-up | **2355** | parsers / state machine+setup phases |
| 0011 USB frontend | 1747 | probe+matching / lifecycle+recovery |

0001 is 72 lines and 0002 is 251, so the series is very lopsided. This is a
**commit** split, not a source-file split; the file layout is fine.

### 3.2 `[ ]` Reorder `rust-drm` logically

Still ordered by when it was written (authored dates run 22 Jul, 22 Jul, ...,
2 Jul x3, 22 Jul, ..., 27 Jul, 8 Aug, 23 Aug, 24 Aug). Proposed grouping:

1. Fixes to Lyude's unmerged series -- better still, folded into her next revision.
2. Core plumbing: registration data, mode objects.
3. Connector: detect/mode_valid, modes, colour properties, colorimetry/HDR, link depth.
4. CRTC: mode changes, colour management, vblank refs, atomic-commit walk.
5. Plane: geometry, damage clips, FB_DAMAGE_CLIPS, blend mode, colour/rotation.
6. Framebuffer: shmem scanout views.
7. Generic DRM: cross-device GEM, HDCP message ids.

Consider splitting into two or three postings; 22 in one series against a
still-moving KMS layer is a lot to ask.

### 3.3 `[ ]` Split `rust: drm: expose CRTC mode changes` -- NEW, found 2026-09-01

Its subject and message describe `mode_changed()`. It also adds an entire CRTC
colour-management API: `ColorCtm`, `ColorLut::new`, `coefficient`,
`coefficients`, `enable_color_mgmt`, `degamma_lut`, `ctm`. Should be two commits:

- `rust: drm: expose CRTC mode changes` -- just `mode_changed()`
- `rust: drm: kms: add CRTC colour management` -- the rest

Deferred to the §3.2 pass rather than done piecemeal, because it wants doing
alongside the reordering. Its sibling problem was already fixed: the commit
titled "add synthesized CVT connector modes" is now correctly titled "attach the
connector colour properties".

⚠ **Audit the other 95 commit subjects against their diffs while doing §3.2.**
Two of the first few checked were mistitled, which is not a good rate.

### 3.4 `[ ]` Split the two large `rust-usb` patches

- 0001 (754 added) introduces revocable ownership, interface I/O, typed
  endpoints, transfers and lifecycle at once. Separate the lifetime/revocation
  primitive from typed USB I/O.
- 0002 (650 added) should introduce the reusable URB separately from persistent
  bulk queues.

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

### 5.1 `[?]` `trace_crypto` -- keep or cut

`vino.rs:1671` defines it; `session.rs:527` and `session/setup.rs:745` print the
control key, delivered RIV, raw video key and nonce to dmesg.

The cover flags it honestly and argues the case. But a kernel config that
discloses key material draws a NAK rather than a discussion, and it can be
carried as a local debug patch at no cost to anybody outside this tree.

Recommendation: cut for v4, keep out-of-tree, say in the cover that it exists
and where to get it. Cheap to concede, expensive to argue about in the same
thread as a 24k-line driver.

### 5.2 `[?]` Automatic firmware flash at probe

`vino.rs:1329` calls `firmware::update_if_newer()` from `probe()`, so a
persistent firmware write can happen automatically with no rollback or readback.
The kernel's firmware-upload framework is built around userspace initiating
persistent updates, and the manual path already exists at
`/sys/class/firmware/vino-<dock>/`.

Recommendation: make the sysfs path the only one for the first posting. Does not
lose a capability -- moves who decides. If it stays, the cover needs a paragraph
on why probe-time is right for a dock too old to enumerate its connectors.

### 5.3 `[?]` Send strategy

Do not fire all six at once again. Get the infrastructure moving first,
coordinate the Lyude fixes with Lyude, let the driver follow. Consider labelling
drm-vino **RFC v4** rather than a merge candidate.

---

## 6. Small

- [x] KUnit count is computed from the tree, not typed (was 97, is 98).
- [x] The "no module parameters" claim now says what it means: no parameter
  selects a profile or a code path.
- [ ] `rust-drm 0016` has a commit body whose first sentence restates the subject.
- [ ] Wrap the handful of commit-message lines over 75 columns.
- [ ] Send `sched-fair` separately to Peter Zijlstra / Ingo Molnar once v4 is out.
- [ ] Keep `Assisted-by: Claude:claude-opus-5` consistent; never strip one from a
  commit Claude actually wrote.

### 6.1 `[?]` The push guards from the WORKLOG are not in place -- NEW

`WORKLOG.md` records `git remote set-url --push <remote> DISABLED` on every
remote in both trees, as a deliberate guard. As of today `git remote -v` in
`vino/linux` shows **real push URLs** for drm-rust, freedesktop, github, gitlab
and lyude. The `sendemail.smtpServer` guard is still in place and
`send-series.sh` still refuses `--send` without `--smtp-server`.

Presumably removed deliberately for the v3 send. Flagging it because the WORKLOG
still claims otherwise, and `gitlab Vino:main` is protected against force-push.

---

## Suggested order of work

1. §2.1 `aes_ctr()` -- owed reply, deletes code, and §2.2 depends on it.
2. §5.1 / §5.2 decisions, so the code settles before the split.
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
