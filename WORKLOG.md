# vino / evdi / chimera v3 refactor - work log

Append-only. Never rewrite an entry. Read this file in full before touching anything.

Conventions: every entry records date, phase, what was done, resulting SHA, what was
tested and on which hardware, what failed, what was deferred and why, and the immediate
next action.

---

## 2026-08-23 - Session 1 - Phase 0 (baseline and fact-finding)

### Guards installed (constraint section 1)

Done as the first action, before any other work.

Push guards, both trees:

- `git remote set-url --push <remote> DISABLED` on every remote.
  - `vino`: freedesktop, github, gitlab
  - `vino/linux`: drm-rust, freedesktop, github, gitlab
- `pre-push` hook installed, exits non-zero, in both git dirs:
  - `vino/.git/hooks/pre-push`
  - `vino/.git/modules/linux/hooks/pre-push` (linux is a submodule; its git dir is
    under the superproject, not `linux/.git`)

Verified rather than assumed: `git push --dry-run github main` in both trees fails with
`fatal: 'DISABLED' does not appear to be a git repository`. The URL layer rejects first,
so the hook is a second line of defence that is never reached in the normal case. Both
layers are present deliberately.

Send guard, both trees:

- `git config sendemail.smtpServer /nonexistent/v3-refactor-send-disabled` (local).

Verified: `git config --get sendemail.smtpServer` resolves to the nonexistent path in
both trees. This matters because there is a *real* global configuration
(`smtp.gmail.com`, port 587, with credentials) that would otherwise deliver. Git's local
config takes precedence over global, so the guard holds.

  - NOTE FOR MIKE, unrelated to this work: `sendemail.smtppass` is stored in plaintext in
    your global git config. Worth moving to a credential helper or an app-password
    rotation at some point. I have not touched it.

Live send path found in the tree and checked: `tools/send-series.sh` calls
`git -C "$kernel_tree" send-email` at line 124, and only adds `--dry-run` when invoked
with `--dry-run`. Default mode is "prepare", and sending requires an explicit `--send`,
so it does not send by accident. `kernel_tree` defaults to `$workspace/linux`, which
carries the local SMTP guard, so even `--send` now fails closed. No change made to the
script.

`git rerere` enabled with `rerere.autoupdate` in both trees, ahead of the rebase.

### Backup branches (constraint section 1)

Created, and to be treated as immutable:

| Tree | Branch | SHA |
|---|---|---|
| `vino` | `backup/main-pre-v3-refactor-20260823` | `1416345f1b9e42596d72758909e933953b086665` |
| `vino/linux` | `backup/vino-pre-v3-refactor-20260823` | `7445bb14b8fc1b8f34ec6da7968a574d84584816` |

Working branches at session start: `vino` tree on `main` (1416345), `linux` submodule on
`vino` (7445bb14). Both clean of uncommitted driver work. The superproject shows `M linux`
(submodule pointer ahead) plus four untracked capture directories from the 2026-08-23
firmware-flash work; neither is part of this refactor and neither has been touched.

### Blocker 1: `-Znext-solver` - RESOLVED, no action needed

Established by inspection, not assumed. `git grep -rn 'next-solver'` over the whole tree
returns nothing. It is absent from:

- the tree as it stands,
- all 178 of our commits (`git log -S'next-solver' integration/base-20260809..vino`),
- all 114 generated patch files under `vino/patches/`.

Every remaining `-Z` flag in the tree belongs to upstream's own Makefiles
(`-Zfunction-sections`, `-Zsanitizer=...`, arch flags and so on), not to our series.
Nothing of ours carries a `-Z`: `git grep '\-Z' -- drivers/gpu/drm/vino/` is empty.

This matches the note in CLAUDE.md that commit `410d1cdbccb9` scoped the flag and it was
later removed once the ordinary solver coped. Toolchain here is rustc 1.97.1;
`scripts/min-tool-version.sh` requires 1.96.0, so we are above the floor.

Remaining work for this blocker: none in the code. Still to do is a clean build to
confirm the ordinary solver genuinely copes on the *rebased* base, since the current
evidence is that it copes on the *current* base. Deferred to the post-rebase checkpoint 0
build, which is where it belongs.

### Blocker 2: variable-time RSA modexp - APPEARS RESOLVED, needs confirming against the thread

The tree does not contain a hand-rolled modexp. `rust/kernel/crypto/akcipher.rs` builds
on the kernel's existing implementation:
`crypto_alloc_akcipher("rsa")`, with our code contributing only DER encoding of the
public key, OAEP-SHA256 padding and MGF1. Searches for a bignum or modexp of our own
(`fn mod_exp|powm|mont|bn_|bignum`) return nothing, in the current tree and across our
history.

Call sites: `hdcp.rs:90` (`oaep_sha256_encrypt`), `session.rs:394` (key construction),
`vino.rs:153` (the stored key). This is the AKE `Ekpub(km)` operation, i.e. a public-key
operation with a public modulus and public exponent.

So the choice the brief asks for - constant-time implementation versus existing kernel
crypto - has already been made in favour of kernel crypto. I am recording it as resolved
*subject to* reading the actual v2 review objection, because I have not yet seen the
thread and should not assume the objection was aimed at the code I just looked at rather
than at something adjacent. That check is part of the list-movement task below.

### Blocker 3: prerequisite bundles - NOT STARTED

Deferred to after the list check, because the right `prerequisite-message-id:` values
come from the current state of those threads, and picking them before checking would mean
pointing at superseded revisions.

### `vino/patches` orphan check (constraint section 1) - COMPLETE, nothing orphaned

The brief asks for two things to be confirmed before deleting: that the directory is
tracked, and that every patch file's content is reachable from a commit.

- Tracked: yes. 124 files tracked, 124 files on disk, and
  `git status --porcelain -uall -- patches/` is empty. So removal is recoverable from
  superproject history.
- Reachable: yes, exactly. All 114 `.patch` files carry a
  `From <40-hex-sha> Mon Sep 17 00:00:00 2001` header, and every one of those 114 SHAs
  resolves in the linux tree (`git cat-file -e <sha>^{commit}`): 114 present, 0 absent.

  Method note: my first attempt matched on `Subject:` lines and produced six false
  orphans, because `git format-patch` wraps long subjects onto a continuation line and my
  extraction truncated them. The SHA header is the exact check and is also far cheaper.
  Do not redo this with subject matching.

Nothing is orphaned, so deletion is safe whenever we want it. **I have not deleted it
yet**, deliberately - see the next section for why.

### The split tooling exists (section 10) - FOUND, do not delete blindly

Section 10 says to look for the existing per-subsystem split tooling before writing
anything new. It survives, and it is better than I expected:

- `tools/regenerate-patches.sh` - regenerates the patch files from the branch
- `tools/check-series.sh` - re-applies the series in a disposable worktree and compares
  the resulting tree object against the source branch
- `tools/send-series.sh GROUP --version 3` - rerolls one group with per-group numbering
  and a cover letter; does not send by default
- `patches/kernel/groups/*.series` - the actual subsystem split: `rust-core`,
  `rust-crypto`, `rust-drm`, `rust-usb`, `vino`
- `patches/kernel/manifest.tsv` - patch -> commit -> author -> subject for all 114
- `patches/kernel/series`, `README.md`, `COVER_LETTER.md`

The v2 grouping recorded in `README.md`: interrupt-prerequisites 18, kms-lyude 37,
drm-crypto-platform 18, usb 7, rust-runtime-drm 22, evdi 1, vino 5.

**Correction to the brief, flagged rather than actioned.** The brief says `vino/patches`
is "stale output and can be deleted; the content lives in git". That is true of the 114
`.patch` files, which I have now proven byte-for-byte recoverable. It is *not* true of
`groups/*.series` and `manifest.tsv`. Those encode a human curation decision - which
commit belongs in which subsystem posting, and which commits are Lyude's, Colin Braun's,
Alice Ryhl's and Onur Ozkan's rather than ours - and that decision is not reconstructible
from the commits alone. Deleting them would throw away exactly the authorship mapping the
attribution rules in section 1 depend on.

Proposed resolution, for Mike to confirm: delete the generated `.patch` files, keep
`groups/`, `manifest.tsv` and the scripts, and regenerate patches on demand. I have done
nothing destructive pending that call.

### List movement check (section 2.2) - BLOCKED ON TOOLING, partially worked around

- `b4` is not installed on this machine. It is the tool the brief assumes for
  `b4 mbox <message-id>` and later for `b4 prep`.
- lore.kernel.org is reachable, but only with a browser User-Agent: a bare request to
  `https://lore.kernel.org/` returns 403, while
  `curl -A 'mozilla/5.0' https://lore.kernel.org/rust-for-linux/` returns 200. So the
  403 is a bot block, not a lack of connectivity, and list checking is possible by curl
  in the meantime.

Question for Mike: may I install `b4` (pip install b4, or the Gentoo package)? Everything
in sections 2.2 and 10 is much cleaner with it, and hand-rolling lore queries over curl
to replace `b4 prep --auto-to-cc` would be a waste of effort.

### Not yet started

- 2.2 rebase onto drm-rust-next (needs the b4/list decision first, and the remote is
  already configured: `drm-rust` -> `https://gitlab.freedesktop.org/drm/rust/kernel.git`)
- 2.3 deletion of abstractions upstream now provides
- everything from section 3 onward

### Immediate next action

Get three decisions from Mike: the `b4` install, the `patches/` partial-deletion
proposal, and confirmation that `drm-rust` remote above is the intended base. Then run
the list-movement check, then rebase, then checkpoint 0.

No code has been modified this session. The only changes on disk are this file and git
configuration (guards, rerere) plus two new backup branches.

---

## 2026-08-23 - Session 1 (continued) - base confirmed, USB abstraction surveyed

### 2.2 Rebase: ALREADY ON THE RIGHT BASE - the rebase is a no-op today

Confirmed against the real remote rather than assumed, as the brief asks.
`git ls-remote --heads https://gitlab.freedesktop.org/drm/rust/kernel.git`:

```
8cdeaa50eae8  refs/heads/drm-rust-fixes
4c9ba407018e  refs/heads/drm-rust-next
4c9ba407018e  refs/heads/drm-rust-next-fixes
7df76a093f6a  refs/heads/drm-rust-templates
4c9ba407018e  refs/heads/for-linux-next
8cdeaa50eae8  refs/heads/for-linux-next-fixes
```

`drm-rust-next` is at `4c9ba407018e8deb06dbc643112bac8f40404f95`, which is *exactly* our
existing `integration/base-20260809`. So:

- we already sit on the current tip of drm-rust-next,
- `git merge-base --is-ancestor integration/base-20260809 vino` passes: the 178 commits
  sit directly on it,
- there are zero merge commits in the range, so the history is already linear and
  format-patch-ready.

drm-rust-next has therefore not moved in the two weeks since 2026-08-09. The expectation
in the brief ("nothing has changed, but verify") holds, and is now verified rather than
assumed. `base-commit: 4c9ba407018e8deb06dbc643112bac8f40404f95` is the trailer to emit.

Checked for the outcome the brief calls easiest to miss - a carried patch that has since
landed upstream, which we should drop rather than carry. Compared the subject of all 178
of our commits against the last 20,000 subjects of the base: **zero matches**. Nothing of
ours has landed.

Caveat, deliberately not glossed over: that check is against drm-rust-next only. The
interrupt prerequisites (Boqun Feng's `SpinLockIrq` work) and Lyude Paul's KMS series
could have landed in `rust-next` or mainline without yet reaching drm-rust-next. Testing
that needs a fetch of rust-next, which I have not done yet. Recorded as open, not as
clear.

### 2.3 What upstream now provides: `rust/kernel/usb.rs`

The brief's description is accurate. The base provides a 500-line `rust/kernel/usb.rs`
with `DeviceId` and `Interface` public, and `struct Device` **private** at line 429 - the
deliberate design the brief describes, where URB work happens on `Interface` and the
parent device stays transparent to interface drivers.

Our series takes that file to 3027 lines: **+2651 / -62**, plus a new
`rust/kernel/usb/ch9.rs` (+295).

What we add is mostly real and defensible transport machinery that upstream does not
have: `Endpoint<K: EndpointKind>` with sealed `BulkIn`/`BulkOut`/`InterruptIn` marker
types, `Urb`/`UrbHandle` with an `Idle`/`Active` typestate, `BulkInQueue`/`BulkOutQueue`,
`Pipe`, `Io`/`IoWindow`, `IsoPacketDescriptor`. That is the "reusable bindings" goal of
the project and is the right shape to argue for.

**The finding that matters, and it is a live review risk.** Three of our additions work
against upstream's model rather than with it:

1. `pub struct Device` at line 2676. We re-export publicly the type upstream
   deliberately made private, and we delete upstream's own private `Device` along with
   its `AlwaysRefCounted for Interface`, `AsBusDevice`, `impl_device_context_deref!` and
   `AsRef<Device>` plumbing, replacing them with our own. Those are the 62 deleted lines.
   A reviewer who chose to make that type private will read this as reverting their
   decision.
2. `pub fn find_device(predicate)` at line 2880, which walks the entire USB topology via
   `usb_for_each_dev`.
3. `DeviceRemovalNotifier` / `DeviceRemovalHandler`, a USB-notifier subscription.

I checked whether these were simply dead code left over from development. They are not,
but the user is **not vino** - it is **evdi**:

- `evdi.rs:182` calls `usb::find_device` to match a device by USB topology path
- `evdi.rs:111,126` hold a `DeviceRemovalNotifier`
- `evdi.rs:210` implements `DeviceRemovalHandler`

vino itself only ever reaches a device through `intf.as_ref()`, at `vino.rs:270` and
`vino.rs:1352`.

Why evdi needs it: evdi's sysfs `add` interface takes a USB topology path from userspace
(`2-1.4` style, parsed at `evdi.rs:175`), finds that device, and symlinks the evdi
platform device to it (`sysfs::create_link(..., c"device")`). That is evdi's existing
userspace ABI, not an invention of ours. So the requirement is genuine - but it is a
requirement of a virtual display driver, and satisfying it by exposing a global USB
topology search in `rust/kernel/usb.rs` is a hard argument to win on the USB list.

This is a design decision, not a cleanup, so I am not deciding it alone. Options as I see
them, with my recommendation:

- **(a) Keep `Device` public and `find_device`, and justify them.** Honest, but asks the
  USB maintainers to reverse a deliberate API decision on behalf of a second driver.
  Weakest position of the three.
- **(b) Recommended: split the two series' fates.** Send the `Endpoint`/`Urb`/queue
  machinery - which is the valuable, defensible part - as the USB series, and keep
  `Device`/`find_device`/the notifier out of it, carried privately by evdi (or deferred
  with evdi) until they can be argued on their own merits. This gets the strong material
  reviewed without the weak material sinking it.
- **(c) Restructure evdi not to need a topology search**, e.g. have userspace pass
  something resolvable without walking all USB devices. This is the cleanest upstream
  story but it is an evdi UAPI change, which section 7 says must be explicit and flagged,
  not silent.

Recommendation is (b), because it makes the USB posting defensible without forcing an
evdi ABI decision in the same breath, and it keeps (c) available later.

### Immediate next action

Blocked on decisions listed at the end of the previous entry, plus the USB question
above. Nothing further changed on disk: still no code modified this session.

---

## 2026-08-23 - Session 1 (continued) - list check done, and blocker 2 was mis-stated

### Standing constraint added by Mike this session

**The Rust evdi should be as similar to vino as possible.** This sharpens section 7:
"siblings, do not merge" still holds, but the default leans toward shared terminology,
shared structure and shared code wherever there is a clear owner, rather than toward
letting the two drivers drift into different idioms. Apply this when doing the naming
pass (section 4) and the file layout (section 6): the same concept gets the same name and
the same shape in both drivers.

Early evidence that this is already partly true and worth preserving: the two Kconfigs
have the same structure, and neither driver contains a single raw `bindings::drm_*` call.

### `patches/` deleted

Mike chose full deletion over my partial proposal. Before doing it I checked whether the
grouping would actually be lost, and **my concern was wrong**: the group boundaries and
the per-group subject patterns live in `tools/regenerate-patches.sh` itself
(`group_starts=(rust-core 1, rust-crypto 35, rust-usb 37, rust-drm 44, vino 105)` plus
`group_pattern()`), not in the generated files. `series`, `manifest.tsv` and
`groups/*.series` are all derived from the branch by that script. So `patches/` was pure
regenerable output after all and the deletion loses nothing.

Committed as `dbe04d3` in the vino superproject, 124 files removed. Deliberately scoped
to `patches/` only: the pre-existing `M linux` submodule pointer change and the four
untracked 2026-08-23 capture directories were left alone.

Pre-deletion superproject SHA, for recovery: `1416345f1b9e42596d72758909e933953b086665`.
`tools/` is intact.

### b4

Mike installed it: `/usr/bin/b4`, version 0.16.0. The curl workaround is no longer needed
(though note lore still needs a browser User-Agent for direct curl calls).

### THE V2 REVIEW FEEDBACK, ENUMERATED - and a correction to the brief

`b4 mbox 20260617151249.2937-1-mike@fireburn.co.uk` returns the whole thread: **41
messages**, covering both v1 (2026-06-17, `RFC PATCH 0/7`) and v2 (2026-07-03,
`RFC PATCH v2 00/10`).

The single most important fact for planning: **all v2 replies are from `sashiko-bot`.
Every human review comment landed on v1, and there are exactly four of them.**

| # | Reviewer | Patch | Point | Status |
|---|---|---|---|---|
| 1 | Eric Biggers | 2/7 | Do not hand-roll AES-CMAC on bare AES; use `include/crypto/aes-cbc-macs.h` | **ADDRESSED, verified** |
| 2 | Danilo Krummrich | 0/7 cover | Driver drives C KMS APIs directly; rework onto Lyude's safe KMS infrastructure and work with her | **ADDRESSED, verified** |
| 2b | Danilo Krummrich | 0/7 cover | Also look at the device-resource series | **OPEN, needs assessment** |
| 3 | Miguel Ojeda | 1/7 | Link the related series and clarify in the cover letters that vino is the user for all of them | **OPEN - cover-letter action for v3** |
| 4 | Julian Braha | 1/7 | `select DRM_GEM_SHMEM_HELPER` is redundant, `RUST_DRM_GEM_SHMEM_HELPER` pulls it in | **ADDRESSED, verified** |

**Correction to the brief, and it matters.** Section 2.1 blocker 2 describes the crypto
objection as "variable-time RSA modexp over HDCP AKE key material". That is not what was
said. Eric Biggers' actual words, on the `aes_cmac` function in patch 2/7:

> There are AES-CMAC library functions that should be used. See
> include/crypto/aes-cbc-macs.h. We don't want drivers rolling their own modes on top
> of bare AES unless they have to, for a number of reasons.

The objection was about a hand-rolled CMAC (including its own `dbl()` subkey derivation),
not about modexp. Nobody raised modexp at all. Reality wins, so I am recording the
blocker as mis-stated rather than implementing against the version in the brief.

Both readings are nonetheless satisfied by the current tree:

- The hand-rolled CMAC is gone. `drivers/gpu/drm/vino/crypto.rs:28` is now a thin
  delegation to `kernel::crypto::aes_cmac`, which reaches the in-tree library through
  `rust/helpers/crypto.c:56` (`aes_cmac_preparekey` + `aes_cmac`, with
  `memzero_explicit` of the prepared key). `include/crypto/aes-cbc-macs.h` is present in
  the base. This is exactly what Eric asked for.
- There is no hand-rolled modexp either, as recorded in the previous entry: RSA goes
  through `crypto_alloc_akcipher("rsa")`.

Verification for Danilo's KMS point: `git grep -c 'bindings::drm_'` over both
`drivers/gpu/drm/vino/` and `drivers/gpu/drm/evdi/` returns **nothing at all**. Neither
driver touches a raw C KMS binding any more; both go through `drm::kms`. The v2 subject
line "built on the safe KMS mode-object layer" is accurate rather than aspirational.

So of the four human objections, three are closed and verified in code, one (Miguel's) is
a cover-letter action that must not be lost at series-preparation time.

**Danilo's 2b remains genuinely open.** `rust/kernel/devres.rs` exists in the base, and
neither vino nor evdi uses `Devres`. I have not yet worked out whether it is applicable
to a USB interface driver's resources here, so I am recording it as to-assess rather than
claiming it either way.

### Consequence for section 10's "vino: no changelog" decision

The brief's reasoning holds and is now evidenced rather than assumed: vino got no human
review, so there is no reader whose mental model needs updating, and a reconstructed
per-patch changelog would be noise. But the premise needs one amendment - vino did get
**automated** Sashiko review on all ten v2 patches. Those are leads worth reading before
cutting v3, and section 10 already says to treat Sashiko output as leads and verify each
one. I have not read them yet.

### Immediate next action

Read the ten sashiko-bot reviews of vino v2 and disposition each. Then assess Danilo's
devres point. Then 2.3 proper (the DRM abstraction diff against the base), then
checkpoint 0.

Still no driver code modified. Changes on disk this session: git guards and rerere, two
backup branches, WORKLOG.md, and commit `dbe04d3` (patches/ deletion).

### Correction from Mike, same session - do not mine the old bot reviews

No person reviewed vino. The AI tool (`sashiko-bot`) checked the patches, and those checks
get **rerun locally against the regenerated v3 series** rather than harvested from the v2
thread. So the "read the ten sashiko-bot reviews of vino v2" action I wrote above is
dropped: stale bot output against a series that is about to be rewritten wholesale is not
worth the reading time, and section 10 already provides for running Sashiko on the
finished series.

What stands from the thread survey is the human feedback table above, which came from the
v1 posting and is already three-quarters closed in code.

Revised next action: assess Danilo's devres point, then 2.3 proper (DRM abstraction diff
against the base), then checkpoint 0.

---

## 2026-08-23 - Session 1 (continued) - all five threads read; the USB framing was backwards

### Our postings, complete inventory with message-ids

v1, 2026-06-17:

| Series | Message-id |
|---|---|
| `rust: usb: synchronous bulk/control transfers + helpers` (0/9) | `20260617145946.1894-1-mike@fireburn.co.uk` |
| `rust: crypto: library AES-128 / SHA-256 / HMAC + RSA` (0/2) | `20260617150143.2152-1-mike@fireburn.co.uk` |
| `rust: drm: minimal KMS bindings, EDID read, rotation, HDCP` (0/5) | `20260617150232.2210-1-mike@fireburn.co.uk` |
| `drm/vino: DisplayLink DL3 dock driver` (0/7) | `20260617151249.2937-1-mike@fireburn.co.uk` |

v2, 2026-07-03:

| Series | Message-id | Human replies |
|---|---|---|
| `rust: usb: synchronous + asynchronous bulk/control transfers` (00/11) | `20260703030020.2694-1-mike@fireburn.co.uk` | **Oliver Neukum x3, Danilo Krummrich x3, Alan Stern** |
| `rust: crypto: library AES-128 / SHA-256 / HMAC + RSA` (v2 0/3) | `20260703030056.2763-1-mike@fireburn.co.uk` | Eric Biggers x2, Miguel Ojeda (on v1) |
| `rust: drm: safe KMS mode-object layer + evdi bindings` (v2 00/18) | `20260703030123.2814-1-mike@fireburn.co.uk` | **Lyude Paul x3**, Miguel Ojeda (on v1) |
| `drm/vino: DisplayLink DL3 dock driver` (v2 00/10) | `20260703030217.2886-1-mike@fireburn.co.uk` | none (sashiko-bot only) |
| `drm/evdi: a Rust EVDI virtual display` (0/2) | `20260703030249.2949-1-mike@fireburn.co.uk` | none (3 messages total) |

Thread sizes: usb 30, drm 29, vino+prev 41, crypto 11, evdi 3.

This confirms the shape the brief describes: reviewers engaged with the subsystem-enabling
work (USB especially) and not with the driver.

### CORRECTION - the USB `usb::Device` framing in the brief, and in my own earlier entry, is backwards

Earlier today I recorded exposing `usb::Device` publicly as "a live review risk" that
"reverts a maintainer's deliberate decision", and recommended splitting the series on
that basis. **Having now read the thread, that is wrong, and I am correcting it.**

Our v2 patch 10/11 was literally titled `rust: usb: keep usb::Device private and gate
...`. That patch is the thing reviewers rejected. Oliver Neukum, the USB maintainer:

> I would say that this is just conceptually wrong.
> 1. drivers talk to the common control endpoint of the _device_ not their interface
> 2. drivers ought to be able to set a configuration (That's a device property)
> 3. Drivers need to be able to claim secondary interfaces (we have an API for that)
> 4. Devices and links (and functions) have states, not interfaces.
> These operations operate on the device level. Hiding that fact behind an interface
> ... is just a layering violation. ... This patch is fundamentally flawed because it
> operates on assumptions that are just not true. USB does device level operations.
> Just drop it.

Alan Stern confirmed the lifecycle premise that makes device access from an interface
sound:

> If a USB device isn't bound to a driver then it can't be configured, and a device in
> the unconfigured state doesn't have any interfaces.

Danilo Krummrich's position is narrower and is about ergonomics plus lifecycle gating,
not about hiding the device: no value in `intf.device().bulk_recv()` over
`intf.bulk_recv()`, because the latter can enforce `Interface<Bound>`; he explicitly said
he is "happy with any solution as long as it considers the device driver lifecycle and
gates I/O operations correspondingly", while agreeing `reset_configuration()` and
`set_interface()` were misplaced. He also objected to an `unsafe as_bound()`:

> If you hit the case where you need an unsafe as_bound() method, it means that either
> your design has issues or the corresponding infrastructure ... isn't available yet.

So the direction of travel is the opposite of what the brief states. Device-level
operations are legitimate and should be reachable; what must be right is the *lifecycle
gating*, not the concealment.

**The current tree has already responded to all of this**, which is why it looks the way
it does:

- `pub struct Device` at `usb.rs:2676` - correct per Oliver, not a regression.
- `as_bound` is gone entirely from both `rust/kernel/usb.rs` and the driver.
- `reset_configuration` is gone.
- `set_interface` now exists in two correctly-scoped forms: `Interface<Bound>` takes just
  an altsetting (`usb.rs:629`), `Device` takes interface plus altsetting (`usb.rs:2769`).

Consequence for the decision Mike took earlier ("split the series' fates"): the *action*
is still reasonable, but the *reason* I gave for it was wrong and must not be carried into
the cover letter. Exposing `Device` is not the weak part of the USB series. The genuinely
unreviewed, evdi-only additions are `find_device()` (a global `usb_for_each_dev`
topology walk) and `DeviceRemovalNotifier`, on which **no reviewer has commented at all**.
Those are what to consider holding back or justifying separately - not `Device` itself.

I have not re-run the decision with Mike because the chosen action survives the
correction; but the cover letter must argue it on the real grounds.

### Answer to Mike's question: yes, it is in the docs, and the v3 has still not landed

`docs/upstream.md` has a `## 2026-08-09 re-check` section which records exactly the
situation remembered:

> **Superseded, migration pending:** Danilo Krummrich has taken over the workqueue work
> and posted `[PATCH v2 0/6] workqueue: OwnedQueue, ScopedQueue and ScopedWork`
> (2026-08-07). It supersedes three commits we carry - Alice Ryhl's three creation
> patches, our own `rust: workqueue: make OwnedQueue thread-safe` ... and Onur Ozkan's
> `cancel_sync` support, which v2 drops in favour of `ScopedWork`. ... a v3 is likely.
> Take it when v3 lands, in one commit that swaps the prerequisites and migrates the
> call sites together.

Rechecked on lore today (2026-08-23), two weeks on:

- **Danilo's v3 has NOT been posted.** His series is still at `[PATCH v2 0/6]`
  (`20260807165252.3849875-1-dakr@kernel.org`, 2026-08-07). Nothing newer from him in
  this area. So the documented "take it when v3 lands" plan is still simply waiting.
- **Movement that postdates the 2026-08-09 re-check:** Onur Ozkan has posted
  `[PATCH v4 1/4] rust: workqueue: impl Send and Sync for OwnedQueue`
  (`20260815-tyr-reset-impl-v4-1-578df9a5e576@onurozkan.dev`, **2026-08-15**), preceded
  by a v4 1/3 on 2026-08-13, with Daniel Almeida reviewing v3 on 2026-08-10. This
  overlaps the commit we carry as `rust: workqueue: make OwnedQueue thread-safe`, so the
  overlap now has two competing upstream candidates rather than one.

Also still standing from that doc section and worth re-checking before the series is cut:
Alvin Sun's `[PATCH v9 00/10] Fix missing fops.owner in Rust DRM/misc abstractions`
(drop ours when theirs lands; not in drm-rust-next as of the last check), and the
Joel Fernandes / John Hubbard `wait_for_completion_timeout()` overlap.

### Blocker 3 (prerequisite bundles) is now quantified

Of our 178 commits, **61 are other people's patches**:

| Author | Commits |
|---|---|
| Mike Lothian | 117 |
| Lyude Paul | 43 |
| Boqun Feng | 9 |
| Colin Braun | 3 |
| Alice Ryhl | 3 |
| Onur Ozkan | 1 |
| Joel Fernandes | 1 |
| Heiko Carstens | 1 |

`rust/kernel/drm/kms*` does not exist in the base at all, so Lyude's 43 are wholly
carried. That is the bundle the brief wants replaced by `prerequisite-message-id:`
trailers.

### Immediate next action

Assess Danilo's devres point, then finish 2.3 (the `rust/kernel/drm` diff is +8362/-14,
of which the `kms/` tree is Lyude's and absent upstream), then checkpoint 0.

---

## 2026-08-23 - Session 1 (continued) - checkpoint 0 gates, and the Onur v4 decision

### DECISION: do not take Onur Ozkan's workqueue v4

Mike asked whether we should take it. Checked the series shape before answering rather
than going on the subject line alone: `b4 mbox` shows it is
**`[PATCH v4 0/4] drm/tyr: GPU reset infrastructure`** - a Tyr GPU driver series in which
`rust: workqueue: impl Send and Sync for OwnedQueue` is merely patch 1/4.

Decision: **no**, for three reasons.

1. It is a bad `prerequisite-message-id:` target. Pointing at a patch embedded in an
   unrelated GPU driver's series ties our postings to Tyr's review schedule and merge
   path.
2. It covers only one third of our overlap. It matches our carried
   `rust: workqueue: make OwnedQueue thread-safe`, but does nothing for Alice Ryhl's
   three creation patches, and nothing for `cancel_sync` - which Danilo's v2 removes
   outright in favour of `ScopedWork`.
3. It would be undone. Danilo has taken over this area and his v2 adds `Send + Sync` to
   `OwnedQueue` directly, so adopting Onur's version now means redoing the work when
   Danilo's lands.

The existing plan in `docs/upstream.md` stands unchanged: wait for Danilo's v3, then swap
the prerequisites and migrate the seven `Work::cancel_sync()` call sites in
`drm_sink.rs`/`vino.rs` in a single commit, with hardware validation.

Worth watching: Danilo is reviewing Onur's Tyr series (he replied to 3/4), so he is aware
of the overlap; it is plausible his workqueue work lands first and Onur rebases onto it.
Re-check both at series-preparation time.

### Checkpoint 0 gates - results

**Build: PASS.** `make LLVM=1 -j16 modules` on the current base, exit 0, **zero warnings
or errors**, with `RUSTC [M]` and `LD [M]` lines for both `vino.o`/`vino.ko` and
`evdi.o`/`evdi.ko`. Checked those lines explicitly, because a green `make modules` can
mean nothing was built when syncconfig has dropped `CONFIG_DRM_VINO`; here both modules
genuinely rebuilt.

This also closes blocker 1 empirically rather than by inspection alone: the tree builds
clean under the **ordinary trait solver** with no `-Znext-solver` anywhere, on rustc
1.97.1 against a `min-tool-version.sh` floor of 1.96.0.

**rustfmtcheck: PASS.** `make LLVM=1 rustfmtcheck` exit 0, no output.

**Clippy: 242 warnings.** `make LLVM=1 CLIPPY=1 -j16 modules`. Note the run ends in a
`modpost: undefined!` failure on `evdi.ko`; that is an artifact of the clippy driver, not
a real regression - the plain build of the identical tree links both modules cleanly. Do
not chase it.

The 242 break down usefully. By kind, the largest groups:

| Count | Warning |
|---:|---|
| 35 | manually reimplementing `div_ceil` |
| 30 | unsafe function's docs are missing a `# Safety` section |
| 26 | casts from `i32` to `i64` expressible via `From` |
| 19 | casts from `u8` to `u16` expressible via `From` |
| 10 | unsafe block missing a safety comment |
| 10 | transmute used without annotations |
| 8 | function has too many arguments (8/7) |
| 2 | unsafe impl missing a safety comment |

**42 of these are the section 1 hard constraint** (missing `# Safety` docs, missing
`SAFETY:` comments, missing `unsafe impl` safety comments). Where they live matters, and
the result is encouraging:

| File | Safety violations |
|---|---:|
| `rust/kernel/drm/kms/connector.rs` | 10 |
| `rust/kernel/drm/kms/crtc.rs` | 9 |
| `rust/kernel/drm/kms/plane.rs` | 7 |
| `rust/kernel/usb.rs` | 5 |
| `rust/kernel/drm/kms/vblank.rs` | 4 |
| `rust/kernel/drm/kms/framebuffer.rs` | 3 |
| `modes.rs`, `encoder.rs`, `atomic.rs`, `event.rs` | 1 each |

**Not one is in `drivers/gpu/drm/vino/` or `drivers/gpu/drm/evdi/`.** The driver code
already satisfies the unsafe-documentation rule; the gaps are entirely in the *bindings* -
mostly Lyude's carried KMS layer, plus our own `usb.rs`.

That has a direct consequence for how they get fixed. The brief says Lyude's patches are
preserved byte for byte, and `patches/README.md` recorded the same convention ("Lyude's
commits remain individual and patch-identical to the imported source; current-tree
adaptations are later Mike-authored commits"). So **safety-doc fixes to `kms/` must be
separate Mike-authored commits on top, never folded into her commits.** The five in
`usb.rs` are ours and can be fixed in place.

By contrast the general-quality warnings (`div_ceil`, infallible `From` casts, too many
arguments) are concentrated in our own code - `session.rs` 41, `drm_sink.rs` 30,
`scanout.rs` 19, `color.rs` 18 - and are fair game for the naming/cleanup phases.

Incidental confirmation of a section 7 point: `drivers/gpu/drm/vino/color.rs` and
`drivers/gpu/drm/evdi/color.rs` produce **18 warnings each**, the same 18. That is the
byte-for-byte duplication the brief calls the obvious candidate for sharing, now
independently visible in the warning profile.

### Immediate next action

Danilo's devres assessment, then `checkpatch --strict`, `rusttest`, and the remaining
checkpoint-0 items.

### Checkpoint 0: checkpatch --strict - PASS, with one disposition that must not be "fixed"

Generated the series to a scratch dir (`git format-patch -M -C`, 178 patches) rather than
recreating `patches/`, and ran `scripts/checkpatch.pl --strict --terse`.

Totals: **67 ERROR, 107 WARNING, 8 CHECK**. Split by author: Mike 64 ERROR / 51 WARNING;
carried 3 ERROR / 56 WARNING / 8 CHECK.

The 64 errors on our own patches look alarming and are not:

| Count | ERROR |
|---:|---|
| 65 | Missing Signed-off-by: line(s) |
| 1 | commit description style `commit <12+ chars of sha> ("...")` |
| 1 | open brace `{` following function definition on wrong line |

**65 of 67 errors are the absent `Signed-off-by`, and that is correct, expected, and must
not be "fixed".** Per the attribution rules, an AI agent must never add `Signed-off-by`;
only Mike can certify the DCO, and he adds it at send time. Any future session that sees
this checkpatch output must not be tempted to silence it. Only **two** errors are real
defects.

Warnings are similarly benign: 41 line-length (expected in Rust, where rustfmt leaves long
string literals alone), 30 "does MAINTAINERS need updating?" (we do touch MAINTAINERS),
15 unwrapped commit description, and 3 `From:/Signed-off-by: email address mismatch` on
**Colin Braun's** patches - which are his commits as posted and must be preserved, not
corrected.

### Danilo's devres point (his v1 item 2b): NOT APPLICABLE - dispositioned

`Devres` exists to revoke **device-bound resources** on unbind, and its use in the base is
MMIO (`io/mem.rs`, `ExclusiveIoMem::into_devres`). Neither driver has any such resource:
`git grep -E 'IoMem|ioremap|Revocable'` over `drivers/gpu/drm/vino/` and
`drivers/gpu/drm/evdi/` finds nothing but two `io::Io` trait imports, which are
byte-buffer accessors, not MMIO regions.

The guarantee Danilo was after - no access after unbind - is already provided by two
USB/DRM-appropriate mechanisms: `usb::Interface<Bound>` gating (the very thing he argued
for in the USB thread) and `drm::Registration`, whose `Drop` runs `drm_dev_unplug()`.

Context that closes it: he raised devres on the **v1** cover letter, when the driver still
drove C KMS APIs directly - the same message in which he asked for the safe-KMS rework.
That rework happened, and it subsumes the suggestion. Recording as not applicable with
reasons, rather than as an open item.

### DECISION: adopt Danilo's workqueue v2 now, rather than waiting for v3

Mike asked whether to move to v2 and fix what we find. Sized it first.

The migration surface is **7 call sites**, and it is well shaped: six are consecutive
lines in a single teardown (`drm_sink.rs:1786-1791`, cancelling `cmd_work`, `cp_watchdog`
and four `scanout_work_hN`), plus one in `vino.rs:1548`. Because `ScopedWork` cancels
synchronously on drop, most of that block is deleted rather than rewritten.

It displaces five carried commits:

| Commit | Author |
|---|---|
| `b7199e8424e4 rust: workqueue: restrict delayed work to global wqs` | Alice Ryhl |
| `6279de070daa rust: workqueue: create workqueue subdirectory` | Alice Ryhl |
| `a2628736ea53 rust: workqueue: add creation of workqueues` | Alice Ryhl |
| `6019a50abad8 rust: workqueue: make OwnedQueue thread-safe` | Mike Lothian |
| `cf6ffc4981e5 rust: workqueue: add cancel_sync support` | Onur Ozkan |

Decision: **yes, adopt v2 now.** Reasons: it removes 5 of the 61 foreign commits and
replaces them with a single `prerequisite-message-id:`, serving blocker 3 directly; the
migration is small and is an improvement in its own right; and being a real user of the
API is the most useful thing we can do for the v3 that we are otherwise just waiting on.
Danilo has already self-reported two v2 defects (wrong default flag, `WQ_PERCPU` vs
`WQ_UNBOUND`, in 4/6; a missing `drop_in_place` in 6/6), so those are known and can be
applied on top.

Constraints on how: it must be **one cleanly separable commit** so that rebasing onto v3
is trivial, and it must not touch the `vino` branch until hardware-validated - the
teardown path is precisely the unbind/unload code that has wedged docks before. Work
proceeds on a scratch branch; Mike validates before it is folded in. Anything we find gets
reported to Danilo.

### Immediate next action

Fetch Danilo's v2, apply on a scratch branch, migrate the 7 call sites, build, and hand
it to Mike for hardware validation.

---

## 2026-08-23 - Session 1 (continued) - CORRECTION on patch handling, and a real gap found in Danilo's v2

### CORRECTION from Mike: never reshape another author's patches

I wrote that the workqueue adoption should be "one cleanly separable commit". Mike read
that as squashing Danilo's six patches into one, and objected: keep the external series
exactly as posted, and put any fix in an extra commit of ours. He noted **this is exactly
what happened with Lyude's patches, and she was not happy.**

He is right and the wording was mine to get wrong. What I meant was that *our call-site
migration* should be one separable commit; what I wrote did not say that.

The action taken was nevertheless correct, and I verified it rather than asserting it.
`git am` applied all six patches individually and unmodified:

| Commit | Author |
|---|---|
| `82d45f7d9337` replace deprecated system_wq with system_{percpu,dfl}_wq | Danilo Krummrich |
| `9df2cc10132f` restrict delayed work to global wqs | Alice Ryhl |
| `d11b031c2d65` create workqueue subdirectory | Alice Ryhl |
| `e49ebb63f87d` add creation of workqueues | Alice Ryhl |
| `7e3302d46034` add ScopedQueue for lifetime bound items | Onur Ozkan |
| `dc6342dbac26` add ScopedWork for non-'static work items | Danilo Krummrich |

Every original trailer survived, including Alice's `Signed-off-by`, John Hubbard's, Gary
Guo's and Andreas Hindborg's `Reviewed-by`, the `Fixes:`/`Cc: stable`, and the `Link:`.
No `Assisted-by` was added to any of them, correctly, since we did not author them.

Two follow-ups done so that this cannot recur:

- Saved as a memory (`feedback-never-reshape-other-peoples-patches-20260823`), including
  the Lyude precedent and the instruction to watch the *wording*, not just the act.
- **Corrected `docs/upstream.md`**, which was itself the source of the ambiguity: it said
  to take the series "in one commit that swaps the prerequisites and migrates the call
  sites together". That sentence would have misled any future session into exactly this
  mistake. It now says to `git am` the series as posted and put our migration in a
  separate commit of our own.

### The migration: partly done, then blocked on a real API gap

Scratch branch `wip/danilo-wq-v2` (never `vino`), built as: our first 24 commits, then
Danilo's six unmodified, then our remaining 149 cherry-picked. **All 149 replayed with
zero conflicts.**

**Done - the `system()` rename.** Danilo's 1/6 removes `workqueue::system()`. Four
callers remained. The choice of replacement is not mechanical and matters:

- vino's three `enqueue_delayed` sites in `drm_sink.rs` take **`system_percpu()`**, not
  `system_dfl()`. `system_dfl()` is documented as the *unbound* default queue, whereas
  `system_percpu()` is the per-CPU queue the old `system_wq` actually was. CLAUDE.md
  records that the scanout worker sits on a **different pool** from `system_unbound()`
  precisely so the encode join cannot self-deadlock. A mechanical conversion to
  `system_dfl()` would have quietly eroded that invariant. Flagging it because it is an
  easy trap for anyone else converting this API.
- Lyude's `SpinLockIrq` doctest takes `system_dfl()`, matching how Danilo converted the
  series' own examples and tests. Fixed in **our** commit, not by editing her patch.

**BLOCKED - `DelayedWork` has no synchronous cancel in v2.** Seven call sites fail to
build: five `Work::cancel_sync`, two `DelayedWork::cancel_sync`
(`drm_sink.rs:1786-1791`, `vino.rs:1548`).

This is a genuine gap in the series, verified rather than assumed:

- v2 drops Onur's `cancel_sync` patch, so `Work` and `DelayedWork` have no cancel method.
  `DelayedWork`'s entire impl block is a single `new()`.
- The offered replacement, `ScopedWork<T>`, cancels on drop - but its destructor calls
  **`cancel_work_sync()`**, and there is no delayed variant anywhere in
  `rust/kernel/workqueue/scoped.rs` (no match for `delayed`/`Delayed` in the file, and
  `cancel_delayed_work_sync()` appears nowhere in the module or in
  `rust/helpers/workqueue.c`).

So **v2 leaves no correct way to synchronously cancel a `DelayedWork` at all**. Worse,
wrapping one in `ScopedWork` would be actively wrong: `cancel_work_sync()` does not cancel
the pending timer, so the work can still fire after the data is dropped. Any driver that
embeds a `DelayedWork` in a pinned struct and has a teardown path - vino's unbind is
exactly that - has no sound option.

vino's two affected fields are `cp_watchdog: DelayedWork<VinoDrmDevice, 5>` and
`cmd_work: DelayedWork<VinoDrmDevice>`.

**This is the payoff for adopting v2 early.** It is a real defect that would otherwise
have landed, and reporting it is worth more to the project than the migration itself.

Recommendation: report to Danilo, and hold the migration until there is a delayed-work
answer. Do **not** paper over it by reintroducing `cancel_sync` as a private patch without
telling him. `wip/danilo-wq-v2` is kept as the evidence, tip `88e9fc610f9f`, and is
deliberately left not building at the seven sites.

### Immediate next action

Get Mike's view on reporting the `DelayedWork` gap to Danilo, and on whether to hold or
work around. `vino` branch is untouched and still builds clean.

---

## 2026-08-23 - Session 1 (continued) - workqueue migration COMPLETE, mail drafted

Mike's call: keep `cancel_sync` for now, complete the rest of the migration, draft a mail
to Danilo.

### Branch `wip/danilo-wq-v2`, tip `d0c860b4d390` - builds clean

Final commit order, and the shape matters:

| Commit | Author | Note |
|---|---|---|
| `82d45f7d9337`..`dc6342dbac26` | Danilo, Alice x3, Onur | Danilo's v2, six patches, **unmodified** |
| ... our 149 ... | | replayed, zero conflicts |
| `ad286380302e` rust: workqueue: add cancel_sync support | **Onur Ozkan** | carried, his authorship intact |
| `d0c860b4d390` drm/vino: move to the reworked Rust workqueue API | **Mike Lothian** | our migration, separate |

Verified after the fact rather than assumed: Danilo's six still show their original authors
and subjects, and our migration is a distinct Mike-authored commit carrying the only
`Assisted-by`. Nobody else's patch was reshaped.

Gates: `make LLVM=1 -j16 modules` exit 0 with **zero warnings**, both `vino.ko` and
`evdi.ko` linked; `make LLVM=1 rustfmtcheck` exit 0.

One formatting follow-up worth noting because it is a general consequence of this API
change: `system_percpu()` is longer than `system()`, so two call sites went past 100
columns and needed wrapping. Fixed with `make LLVM=1 rustfmt` and amended into **our**
commit.

### Why `cancel_sync` is kept, stated in the commit message

`ScopedWork` cancels on drop via `cancel_work_sync()`, which does not cancel a pending
timer, and two of vino's work items are delayed. Keeping Onur's patch is the correct
teardown until the series has a delayed answer. This is recorded in the commit message so
a reviewer does not have to reconstruct it.

### Mail to Danilo: drafted, NOT sent

`vino/outgoing/danilo-scopedwork-delayed.eml`. Threads onto 6/6 via
`In-Reply-To: <20260807165252.3849875-7-dakr@kernel.org>`. Written in Mike's own list
voice, taken from his replies in the v1 thread rather than invented: short, plain, no
ceremony. Verified zero em dashes and zero non-ASCII.

It states the gap as "as far as I can tell", which is the honest framing: I verified
`cancel_delayed_work_sync()` appears nowhere in `rust/kernel/workqueue/` or
`rust/helpers/workqueue.c`, but Danilo may have an intended pattern.

**Sending is Mike's.** The guards remain in place and I have not attempted delivery.

### Immediate next action

Phase 3 (profile/topology), already surveyed on branch `v3/phase3-profile`.

---

## 2026-08-23 - Session 1 (continued) - Phase 3 (architecture) largely done

Branch `v3/phase3-profile`, three commits, each building clean with zero warnings and
`rustfmtcheck` passing.

### `092ef4173d6a` - name the behaviour, not the model

`Generation` is gone. It had exactly **one** behavioural match site
(`matches!(self.generation, Generation::Navarro)` behind `is_navarro()`), so section 3.2's
option (b) applied: delete it as a behavioural input. That resolves the Ella lie by
removal rather than by adding a third variant to an enum nothing branched on.

The 15 `is_navarro()` sites became **eight fields**, because several asked the same
question. The consolidation that matters: cold activation, the dual-head commit path and
holding the control plane quiet across the first mode set are all one property, now
`dock_wide_modeset` - on such a dock, reconfiguring one connector while another is lit
resets it. Full mapping is in the commit message and in the field docs.

`perhead_onehot()` was also a model-derived predicate (`self.is_navarro()`) describing a
real wire difference; it is a field now.

The DRM side cached `is_navarro: AtomicBool` and published it at probe. That cache is now
the four specific flags the KMS path actually consults, set through one
`set_mode_behaviour(profile)`.

### `945b7b9a8c32` - shared-pipe submission is stated, not inferred

`video_on_ctrl_pipe()` computed submission policy as `video_eps[0] == EP_CTRL_OUT`. It is
a field now, with the reasoning recorded: a dock that put video on `0x02` while keeping a
pipe of its own would have been serialised for nothing, and one sharing a pipe at another
address would not have been serialised when it must be.

**Correction to the brief.** Section 3 says "Navarro's shared endpoint layout must be
stated as data, not inferred by searching for equal endpoint addresses." No such search
exists. `video_eps: [0x08, 0x0a, 0x08, 0x0a]` already states the routing directly -
connectors 0/2 on `0x08`, 1/3 on `0x0a` - and `grep` for any equality scan over the array
finds nothing. The only genuine instance of inference was the predicate above, now fixed.
A separate `ConnectorRoute` table would add a type without adding information.

### `d1bdf9ce4fbe` - four distinct responsibilities

`DockProfile` split into `Topology` (3 fields), `Capabilities` (5), `Protocol` (34) and
`Quirks` (2), with `name` staying at the top level. 117 access sites rewritten
mechanically plus 11 by hand (the ones reached through `self` inside profile.rs's own
impl and in tests). No value changed.

`Quirks` carries a deliberately high bar in its doc comment - a field earns a place by
being a defect that contradicts the rest of the model - so it does not become the dumping
ground the brief warns about. Only `shared_edid_handler` and `split_full_packet_frame`
qualify.

**Observation for a later session, not acted on.** 34 fields is a lot for one group, and
`Protocol` visibly subdivides into three clusters: session bring-up
(`initial_vendor_state`, `reply_discipline`, `video_commit_point`, `setup_polls`,
`dock_wide_init`, `ep84_queue_depth`, `probe_bracket`, `reports_presence`,
`status_period_ms`), mode programming and sink control (`dock_wide_modeset`,
`clear_mode_before_set`, `blank_bracket`, `sink_down_state`, `post_mode_sink_states`,
`pre_mode_sink_state`, `allocation`), and the video record stream (the rest). I
implemented the four groups the brief names rather than inventing a fifth, but a
`Protocol { session, modeset, video }` nesting would be the honest next step if this still
reads as too coarse.

### Immediate next action

Section 5, the codec terminology correction (WHT -> Haar).

---

## 2026-08-23 - Session 1 (continued) - Sections 5 and 7

### `a685381db280` - Section 5, the codec terminology - VERIFIED FIRST, then renamed

The brief has been wrong twice today, so I checked the mathematical claim before renaming
anything. **This time it is right, and decisively so.** `video.rs` implements
`haar2d_level!` as `haar2d_8`, `haar2d_4`, `haar2d_2` and applies them in a three-level
Mallat decomposition: 8x8 to three 4x4 detail bands, that LL to 2x2, that LL to the DC and
coarse coefficients. The doc on `transform()` already said "8x8 2-D Haar (Mallat)
transform" and described the band layout.

So the implementation had been correct and correctly documented since the codec was worked
out. Only two things still carried the original guess: the module name `wht`, and one
summary line calling it "DisplayLink's 8x8 Walsh-Hadamard codec". The file named the
transform two different things.

177 occurrences renamed. Checked the token forms first
(`grep -ohE '[A-Za-z_]*[Ww][Hh][Tt][A-Za-z_]*'`) to be sure there were no substring false
positives - all were the bare module path or clean `wht_`/`encode_and_send_wht` prefixes.
No `walsh` or `hadamard` text remains in the driver.

Six out-of-tree docs still say WHT (`docs/simd.md`, `docs/architecture.md`,
`docs/protocol/video.md`, `docs/protocol/navarro-decoded.md`, `docs/new-device-day.md`,
`docs/new-device-day-ella.md`). Left for the section 11 documentation pass, which the brief
says comes after the code settles.

### `9fecae53f9e5` - Section 7, the duplicated colour module now has one owner

`color.rs` was **byte-identical** between vino and evdi, as the brief says. What the brief
does not mention is that this was deliberate and managed: the file's own header said the
two drivers "cannot share a crate, so the copies are kept byte-identical", and
`tools/color-selftest.sh` failed the build if they drifted.

That reasoning had one gap. They cannot share a *driver* crate, but the kernel crate is a
crate they both already depend on, and nothing in the file was DisplayLink-specific: its
only dependencies were `kernel::drm::kms::crtc::{ColorCtm, ColorLut}` and
`kernel::xxhash::xxh64`. It is a general facility - apply CTM and gamma in software for a
driver that advertises the properties but has no hardware behind them.

Moved to `rust/kernel/drm/color_pipeline.rs`, made public, both drivers repointed, and the
duplication and its drift guard are gone. The arithmetic selftest was rewired to the new
location and still passes **15 of 15**.

This also directly serves Mike's standing instruction that evdi mirror vino: the two now
share the implementation rather than a promise to keep two copies equal.

Judgement call worth recording: this adds two exported kernel symbols
(`ColorPipeline::build` and `::tag`), so it widens the kernel's Rust ABI for the benefit of
two drivers. I think that is the right trade - it is a real abstraction with a clear owner,
which is exactly what section 2.3 says to do with something genuinely missing rather than
keeping it as private plumbing - but a reviewer may push back and the cover letter should
argue it rather than let it pass unremarked.

### Build-workflow finding worth knowing

Changing the kernel crate invalidates `Module.symvers`, so `make LLVM=1 modules` alone
fails with `modpost: ... undefined!` even though the symbols are present in `kernel.o`,
`vmlinux` and `exports_kernel_generated.h`. A **full `make LLVM=1`** is needed first. This
is not new to this change - we already modify `rust/kernel/usb.rs` and `rust/kernel/drm/` -
but it is the first time it bit, and the error message points at the wrong thing entirely.

Also hit, and unrelated to any change: `fixdep: error opening file:
drivers/gpu/drm/amd/amdgpu/.umc_v6_7.o.d`. That was two `make` runs racing in the same
tree. Removing the stale object and rebuilding cleared it. Do not run concurrent builds
here.

### State

`v3/phase3-profile` now carries five commits (three profile, one codec rename, one colour
pipeline). Every one builds warning-clean with `rustfmtcheck` passing. Superproject has
`d758d0d` for the tools change. `vino` branch remains untouched.

---

## 2026-08-23 - Session 1 (continued) - colour move REVERTED on Mike's call; key hierarchy documented

### REVERTED: the shared colour pipeline

I moved `color.rs` into `rust/kernel/drm/color_pipeline.rs` and flagged, in the entry
above, that it widened the kernel's Rust ABI for two drivers' benefit and that a reviewer
might push back.

Mike's response settles it against the move: **evdi is not expected to be accepted
upstream, and nothing else in tree would use this.** That does not merely qualify the
justification, it removes it. If evdi never lands, upstream is being asked to export two
kernel-crate symbols for a facility with exactly one consumer, which is private plumbing
promoted to kernel API - the thing section 2.3 warns against, committed in the opposite
direction.

Reverted. Both drivers carry the byte-identical copy again, `tools/color-selftest.sh`
guards the drift, and the arithmetic still passes 15 of 15.

**The add-and-revert pair was then dropped from history entirely** (`git rebase --onto`),
in both the kernel branch and the superproject, so the branch reads as though the move was
never made. The series is cut fresh, and a commit followed by its own revert teaches a
reviewer nothing. The reasoning is preserved here, which is where it belongs.

Standing consequence worth carrying forward: **evdi is unlikely to go upstream.** That
changes the calculus of section 7 generally. Sharing between vino and evdi should be
judged on whether it helps *us* maintain them, not on whether it produces a reusable
kernel abstraction, because the second consumer may never be in tree. Mike's earlier
instruction that evdi mirror vino still holds and is well served by the guarded identical
copy.

### `8f2616354da6` - the HDCP key hierarchy, documented once

Section 4 asks for a comment at the crypto boundary defining the key hierarchy once, so a
reviewer who does not know HDCP can follow the code. Added to `hdcp.rs`: what `km`, `kd`,
`ks` and `riv` are, which derives from which, and that all are per-session.

**Deviation from the naming table, deliberately.** The table says to rename `ks`/`kd`/`km`
to `session_key`/`derived_key`/`master_key`. I have not, because the same section says
"HDCP names that match the spec or DRM helpers may stay short, but document each one once
at a sensible boundary", and these are exactly the HDCP 2.2 spec names. Renaming `km` to
`master_key` would make the code *harder* to read beside the specification, which is how
this code is verified. The risk the brief actually identifies is conflation - "Do not give
both the same name" - and they are not conflated here: `kd` is `[u8; 32]`, `ks` is
`[u8; 16]`, distinct names and distinct types. The documentation was the missing piece.

**A real error caught by verifying rather than transcribing.** My first draft of the
hierarchy said the SKE mask was `edkey = ks XOR dkey_2`. Reading `compute_eks` shows it is
`ks XOR (dkey_2 with its low 8 bytes XOR rrx)`. The doc now says that. Worth noting because
the wrong version is the one the HDCP summary literature usually gives, and it would have
been an authoritative-looking comment that was quietly wrong.

### State

`v3/phase3-profile`: five commits, all building warning-clean with `rustfmtcheck` passing.
Superproject back to `dbe04d3`. `vino` branch untouched throughout.

### Immediate next action

Section 4 proper (the `head` / `HEADS` / `sub` renames, which are the bulk of it) and
section 6 (file layout; `drm_sink.rs` is 6,671 lines).

---

## 2026-08-23 - Session 1 (continued) - Section 4, the connector rename

### `f9a39e56366c` - head -> connector, ~1,900 sites

The driver named one concept two ways. Most of it said `head`, 296 sites already said
`connector`, and both meant the index of a physical downstream socket. Two pieces of
evidence settled that they are one index space and not two:

- `head_i` is literally `let head_i = head as usize` at every one of its definitions - a
  width conversion, not a different quantity.
- `build_ella_config_buf` and `build_navarro_prologue_buf` each contained
  `let connector = head as u8`, i.e. the code itself converting between the two names.

So the rename is `connector`, which is what DRM calls the object being indexed and what
`DockProfile` already called it.

**The one real hazard, found before renaming rather than after.** Rust shadows silently,
so a blind rename would have turned those two `let connector = head as u8` lines into
`let connector = connector as u8` - self-shadowing, still compiling, and semantically fine
by luck but unreadable. I scanned for any scope binding both names first (a small parser
over function bodies), found exactly those two, and pre-renamed their locals to
`connector_selector` before touching anything else.

Also renamed, per the section 4 table and for the reasons it gives:

| Before | After | Why |
|---|---|---|
| `HEADS` | `MAX_CONNECTORS` | it bounds the fixed DRM object layout; the number a dock has is `DockProfile::connectors` |
| `head_i` | `connector_index` | |
| `head_sub`, `head_sub_shift` | `connector_selector`, `connector_selector_shift` | names what the field selects, not where it sits |
| `video_eps`, `eps` | `video_endpoints`, `endpoints` | |
| `active_heads`, `repair_heads`, `edid_heads`, `cmd_heads` | `*_connectors` | |

Verification: build exit 0 with **zero warnings**, `rustfmtcheck` clean, `rusttest` exit 0,
and zero residue of `head`/`heads`/`HEADS`/`head_i` in the driver. Word boundaries meant
`header`, `ahead` and `overhead` were untouched; I also grepped the diff for prose that had
gone strange ("connector room", list/ring/queue contexts) and found none - the renamed
comments read correctly ("per-connector blocks", "the first connector's mode set").

### evdi needs nothing here

Checked, because Mike's standing instruction is that evdi mirror vino. evdi already uses
`connector` (31 sites) and has just **two** occurrences of "head", both prose describing a
"display head (CRTC + primary plane + virtual encoder + virtual connector)". That is the
correct English sense - the whole assembly, not an index - and renaming it would make the
comment worse. Left alone deliberately.

### Not done from the section 4 table, with reasons

- **`ks`/`kd`/`km`/`riv`**: kept as the HDCP 2.2 spec names and documented once instead;
  reasoning in the previous entry.
- **`sub`** as a bare name: `head_sub` is handled, but bare `sub` still appears in record
  framing where it means the record subfield. Wants a per-site pass, since the table itself
  says it is "badly overloaded; resolve per site".
- **`dev` (UsbLink) -> `link`**, **`desc` split**, **`dock_id`**, **`slot`/`slots`**,
  **`pt`/`iv`/`ctr`**, **`w_pad`/`h_pad`**, **`geom`**: not yet done.

### State

`v3/phase3-profile`: six commits, each building warning-clean. `vino` untouched.

### DECISION CONFIRMED by Mike: keep the HDCP spec names

Mike confirmed keeping `km`/`kd`/`ks`/`riv` as the HDCP 2.2 spec names rather than
expanding them per the section 4 table, with the hierarchy documented once at the module
boundary instead. **Settled - do not re-litigate this in a later session.**

### `drm/vino: spell out the abbreviations that outlive a line`

`geom` -> `geometry`, `w_pad`/`h_pad` -> `padded_width`/`padded_height`, `pt` ->
`plaintext`, `dock_id` -> `identity_bytes`, `desc` -> `config_descriptor` (it is the USB
configuration descriptor specifically).

One over-reach caught by the build: `DriverInfo` has an upstream field literally named
`desc`, and the blanket rename hit the initialiser at `drm_sink.rs:6636`. Reverted that one
site. Worth remembering that these sweeps can rename *someone else's* field name at a use
site; the compiler catches it, but only because the field is a struct member rather than a
local.

Still deferred from the table, deliberately: `iv` (a three-line local in `hdcp.rs`, and a
universal crypto term), `ctr` (the brief's stated hazard - `ctr` and `counter` live in one
function - does not occur; `counter` appears only in prose), and `slot`/`slots` and bare
`sub`, both of which the table itself says need per-site judgement.

---

## 2026-08-23 - Session 1 (continued) - Section 6, file layout

`impl VinoDrmData` was **164 methods / 4,530 lines** in a 6,817-line file. It is now
**97 methods / 1,573 lines**, and `drm_sink.rs` is **3,967**. Five subjects moved out, each
a pure move in its own commit:

| Module | Methods | Lines | Subject |
|---|---:|---:|---|
| `drm_sink/activation.rs` | 4 | 1,180 | cold training and the three activation paths |
| `drm_sink/bracket.rs` | 12 | 362 | sink down/up and the mode-set brackets |
| `drm_sink/presence.rs` | 15 | 422 | EDID probe, debounce, flap repair |
| `drm_sink/stream.rs` | 16 | 427 | stream opening and record framing |
| `drm_sink/cp_session.rs` | 20 | 401 | sealed sends and session liveness |

### Two real bugs in my own tooling, both of which compiled cleanly

Recording these because both produce **silent** damage that no build or test catches.

**1. Line-range slicing cut through a run of doc comments.** The first activation extraction
took a line range. Because several methods' doc comments sit consecutively, the cut left
three doc blocks stranded in `drm_sink.rs`, where they silently re-attached to
`build_frame_trailer` - so one function carried four descriptions and three functions lost
theirs. It compiled. I threw the attempt away and rebuilt the tooling to select methods **by
name**, parsing the impl into (docs + attributes + body) units so a unit can never be cut
through.

**2. The unit parser did not understand multi-line attributes.** `#[expect(dead_code, reason
= "...")]` spans four lines, and only the first starts with `#[`; the continuation lines fell
into the "stray line" branch, which flushed the pending doc and attribute as anonymous. Net
effect: `repair_flapped_connector`'s doc comment and its `#[expect]` stayed behind while the
function moved, leaving an unfulfilled lint expectation in one file and an undocumented
function in the other. Caught by the `warning: this lint expectation is unfulfilled` rather
than by anything I had thought to check. Fixed by consuming an attribute until its brackets
balance, then re-verified that activation and bracket had not suffered the same.

Also worth knowing: my orphan detector reports a **false positive** on any multi-line
attribute, because it checks only the line after the `#[`. Zero real orphans across all six
files.

### Visibility: pub(super) does not survive the move

In `drm_sink.rs`, `pub(super)` meant "visible to the crate root". In `drm_sink/<mod>.rs` it
means "visible to `drm_sink`", so every method still called from `vino.rs` had to become
`pub(crate)`. The compiler reports these three different ways depending on the case -
`method X is private` (E0624), `associated function X is private`, and `no method named X
found` (E0599, when it is not visible at all) - so the helper that raises visibility has to
match all three or it silently fixes nothing.

### A self-inflicted mistake worth not repeating

I reverted `drm_sink.rs` with `git checkout --` to undo a bad extraction, but the **bracket**
extraction was still uncommitted at that point. The revert restored the bracket methods into
`drm_sink.rs` while `bracket.rs` still held its copies, and the `sed` that adds `mod
bracket;` then found no anchor, so neither module was declared. The result compiled as
"method not found" rather than as duplicate definitions, which sent me looking in the wrong
place.

**Commit each extraction before starting the next.** The later splits were committed
immediately and none of this recurred.

### Section 11, partial: codec terminology in docs

Six out-of-tree docs still said WHT; all now say Haar, and `video::wht` references became
`video::haar`. `Documentation/gpu/vino.rst` never mentioned it.

One stale pointer removed: `docs/new-device-day.md` referred to `WHT-CODEC.md`, which does
not exist in this repository (it is in the old `dl-scripts/docs`). Repointed to
`protocol/video.md`, which is where the codec is actually documented.

**Noted, not done:** a crude sweep for doc references to files that do not exist turned up
a handful of candidates beyond that one. Most are false positives from the way I resolved
paths (`drivers/gpu/drm/vino/profile.rs` lives under `linux/`, the capture scripts under
`tools/capture/`), but a genuine link audit of `docs/` is worth doing once and is not part
of the codec rename. Left for the section 11 pass proper, which the brief places after the
code settles.

---

## 2026-08-23 - Session 1 (continued) - Section 8, chimera

### chimera was already broken, and still is

Recorded plainly because it would be easy to mistake for damage from this refactor.

chimera compiles the driver's `cp.rs`, `video.rs`, `video_arm.rs` and `hdcp.rs` **verbatim**
from `revdi/chimera/vino/`, kept in step by `revdi/scripts/sync-kernel-sources.sh`. The
renames therefore reach it automatically, and its own `src/` has to follow.

Before touching anything I built chimera against the **pre-session** kernel tree
(`backup/vino-pre-v3-refactor-20260823`) to establish a baseline. It failed with **8
errors**. After syncing my changes and updating chimera's `src/`, it fails with the **same
8 errors**, verified by diffing the normalised error sets rather than comparing counts:

```
baseline errors: 8   now: 8
IDENTICAL - my changes are fully absorbed
```

The eight are pre-existing rot, unrelated to anything here: a `profile` module chimera does
not vendor, `kshim::KVec` lacking `new()` and `IntoIterator`, and three signature
mismatches (`video_arm::build`, a `FnMut(usize, usize)` given a `u8`, and two arity
errors). They want their own fix, which is a separate task from this refactor.

What I did change in chimera's own sources: `video::wht` -> `video::haar`,
`perhead_hdcp_push` -> `per_connector_hdcp_push`, and the `drm_sink` shim's `HEADS` ->
`MAX_CONNECTORS` including the compile-time assertion that keeps it equal to the driver's.
Committed as `86ca3e3` in the revdi repository.

Section 8 also asks for dock identification, generation handling, capability exposure and
HDR gating to be brought in line. That cannot be assessed while the thing does not compile,
so it is **deferred behind fixing the eight**, and recorded here rather than silently
skipped.

### A self-inflicted scare worth recording

To get the baseline I checked the kernel tree out at the backup branch, having stashed
first. Popping the stash afterwards landed `wip/danilo-wq-v2`'s uncommitted work onto
`v3/phase3-profile` and left `drm_sink.rs` with conflict markers.

No harm done: I confirmed the stash's content was already committed on the workqueue branch
(`git diff stash@{0} wip/danilo-wq-v2 -- ...` empty), reset the tree to
`v3/phase3-profile`, and rebuilt clean. The stash is left in place.

**Do not use a bare `git stash` to hop between these branches.** The branches carry
overlapping edits to the same files and the stash does not remember which branch it came
from in any way `pop` respects.

---

## 2026-08-23 - Session 1 (continued) - Section 9, checkpoint record

### CHECKPOINT (software gates only - no hardware exercised)

```
Dock / family        NONE EXERCISED. Ella, Ridge and Navarro all untested this session.
Kernel commit        v3/phase3-profile @ 28043e1b1cbf
                     base integration/base-20260809 = drm-rust-next 4c9ba407018e
Tested connectors    none (no hardware access from this session)
Resolution / refresh  n/a
Modeset              n/a
Stable scanout       n/a
Hotplug / reconnect  n/a
Known limitations    every hardware gate in section 9 is outstanding and is Mike's to run
```

Software gates, all passing:

| Gate | Result |
|---|---|
| `make LLVM=1 -j16 modules` | exit 0, **zero warnings**, both `vino.ko` and `evdi.ko` linked |
| `make LLVM=1 rustfmtcheck` | exit 0 |
| `make LLVM=1 rusttest` | exit 0, no failures |
| lines over 100 columns | 40, matching the 40 on the pre-session backup |
| no hardcoded endpoint addresses in routing paths | confirmed: none outside `profile.rs` and named constants |
| HDR gate holds | confirmed, see below |

**HDR gate.** `hdr_capable` is declared per profile and read, never inferred: Ella `false`,
Ridge `false`, Navarro `true`. That matches DisplayLink documenting HDR10 as DL-7000 only,
and the D6000 reporting `HDR supported = False` to Windows. The only two consumers go
through `data.hdr_capable()`, which is populated from `profile.capabilities.hdr_capable` at
probe. Nothing derives it from connector type or product family.

### The comment reflow, and why it is its own commit

The connector rename pushed **281** comment lines past 100 columns, because `connector` is
five characters longer than `head` and rustfmt does not reflow comments. Measured rather
than assumed: 40 over-length lines on the pre-session backup, 321 after the rename. That is
a real regression and it showed up as 260 checkpatch warnings on one patch.

Reflowed back to 40. Only prose is joined and re-broken; tables, fenced blocks, lists and
indented lines are emitted exactly as found, and a run is skipped entirely if reflowing it
would still leave a line over the limit. Verified that **only comment lines changed** in
the diff.

**The brief says to fold such a fix into the commit that introduced the line, and I tried
and backed out.** `git rebase --autosquash` onto the rename commit conflicts irreducibly:
the reflow touches `drm_sink/activation.rs`, `bracket.rs`, `presence.rs`, `stream.rs` and
`cp_session.rs`, none of which exist at that point in history, and the later split commits
move the very lines being rewrapped. I then tried stopping *at* the rename commit and
reflowing there, which is the right shape - it got through the activation split by redoing
the extraction, then conflicted again on the compound-rename commit and produced a
non-building tree.

At that point I aborted and restored. The fold is cosmetic; a working 190-commit branch is
not. Restored from `backup/phase3-pre-autosquash-20260823`, verified building clean with
`rustfmtcheck` passing and 40 over-length lines.

So the reflow stays as its own commit, and the cover letter should say plainly that it is a
mechanical follow-on to the rename rather than pretend otherwise. If a future session wants
it folded, the way to do it is to redo the whole phase with the reflow applied at each
step, not to rebase it backwards.

**Bug found while reflowing.** The `DockProfile` split had **stranded a doc comment**: the
paragraph explaining why the video endpoints cannot be a global constant came loose and
re-attached above `struct Topology`, in front of that struct's own one-line summary, so
`Topology` carried two descriptions and the text read as a non-sequitur. The first reflow
attempt then *merged* them, which is how I noticed. Repaired: the paragraph is about the
endpoint layout, so it stays with `Topology` as one comment. The other three new structs
were checked and are clean.

That makes three separate doc-comment casualties from the structural work this session, all
of which compiled: two from line-range slicing, one from the struct split. Every one was
found by reading output rather than by a build or test.

### checkpatch on the current series

191 patches: **81 ERROR, 478 WARNING, 8 CHECK**. Of the errors, **78 are the absent
`Signed-off-by`**, which is correct and must not be "fixed" by an agent. Two are real (a
commit-description style, a brace placement) and one is the same missing-SoB on a new
commit.

The warning count is dominated by the rename patch's 260 line-length findings, which the
later reflow commit fixes but which checkpatch still sees on the intermediate patch. That
is the direct consequence of not folding, described above.

---

## 2026-08-23 - Session 1 (final) - THE PLAN FOR THE VINO SERIES. Read this first.

Mike's direction, and it corrects how I had been working: **this is a new driver, so the
series must be original correct commits, not development history.** No "fixup" commits, no
renames-after-the-fact, no reverts. It has only ever been an RFC and it was broken, so the
big changes should be made now, cleanly.

That invalidates the shape of the current `v3/phase3-profile` branch as a *posting*. Its
eleven refactor commits are my working history: "call a connector a connector", "give a dock
profile four distinct responsibilities", "move sink activation into its own module", the
comment reflow. **None of those should appear in the series.** The driver should simply be
introduced with connectors called connectors, the profile already split, the transform
already called Haar, and the modules already separate.

The branch stays as the source of the *content*. The series is cut fresh from its end state.

### The precedent, checked rather than assumed: panthor

I was about to argue for one large core commit. Mike asked which drivers had done this
before, and the answer changed the design.

`drivers/gpu/drm/panthor` is 15,907 lines, comparable to vino's ~19k, and landed as
**eleven commits, one per logical block**:

```
drm/panthor: Add GPU register definitions          239 insertions
drm/panthor: Add the device logical block          943
drm/panthor: Add the GPU logical block             534
drm/panthor: Add GEM logical block                 372
drm/panthor: Add the devfreq logical block         304
drm/panthor: Add the MMU/VM logical block        2,870
drm/panthor: Add the FW logical block            1,865
drm/panthor: Add the heap logical block            636
drm/panthor: Add the scheduler logical block     3,552
drm/panthor: Add the driver frontend block       1,473
drm/panthor: Allow driver compilation               40   <- Kconfig/Makefile LAST
```

⭐ **`Allow driver compilation` adds only Kconfig and Makefile, and comes second to last.**
The ten commits before it deliberately do not build as a driver.

⛔ **This contradicts the brief.** Section 1 warns that "splitting a driver into 'add file
A' / 'add file B' commits where nothing compiles until the last one is a cosmetic split".
Panthor is the accepted modern precedent for a large new DRM driver and does exactly that.
Reality wins. Do not use the brief's rule to argue for a single 19k-line commit.

Contrast points: `udl` (1,380 lines) landed as one commit of 2,324 insertions; `tyr` landed
as a 667-line skeleton and grew afterwards. Size decides, and vino is panthor-sized.

### The target series for the vino group

One commit per logical block, in dependency order, Kconfig last. This needs no surgical
extraction of features, because after the section 6 work the blocks **are** real files.

| # | Commit | Files | ~lines |
|---|---|---|---|
| 1 | protocol and framing | `proto.rs` | 71 |
| 2 | USB transport | `usb_link.rs` | 250 |
| 3 | crypto, HDCP and the AKE | `crypto.rs` `rng.rs` `hdcp.rs` `ake.rs` | 300 |
| 4 | the control plane | `cp.rs` (split further, see below) | 1,816 |
| 5 | dock profiles | `profile.rs` | 866 |
| 6 | the codec | `video.rs` `video_arm.rs` `color.rs` | 2,457 |
| 7 | session bring-up | `session.rs` (split further) | 2,329 |
| 8 | the KMS sink | `drm_sink.rs` + its 5 submodules | 7,300 |
| 9 | firmware reporting and DFU update | `firmware.rs` | 624 |
| 10 | the driver frontend | `vino.rs` | 1,622 |
| 11 | allow driver compilation | Kconfig, Makefile, MAINTAINERS | ~40 |
| 12 | documentation | `Documentation/gpu/vino.rst` | - |

`drm/evdi` is a separate driver and gets its own commit or small series.
`rust: firmware: add the firmware upload abstraction` belongs in a subsystem series, not
here.

### Further splitting, before cutting - Mike asked, and the answer is yes

Commit 8 at 7,300 lines is twice panthor's largest block (3,552). Three files still want
splitting, and each has real internal seams rather than arbitrary ones:

- **`session.rs` (2,329)** is a single `impl VinoDriver`, exactly the shape `drm_sink.rs`
  had before section 6. Split it by method group the same way, with the same
  name-based tooling (`/tmp/.../impl_split.py`, `do_split.py`).
- **`cp.rs` (1,816)** already has seams: `Timing`, `ModeProfile`, `PerheadHdcpPush`, and
  `mod restatement`. Suggest `cp/{mode,hdcp,restatement}.rs`.
- **`video.rs` (2,016)** is one `mod haar`. Suggest `video/{haar,colour,records}.rs`.
- **`drm_sink.rs` (3,981)** may still want one more pass; it is the largest single file.
- `vino.rs` (1,622) is fine: panthor's frontend block was 1,473.

### ⛔ tests.rs is not how the kernel does this

Mike asked whether one big self-test file is normal. **It is not, and vino is the only
place in the entire tree that does it**: `find rust/ drivers/gpu/drm/ -name tests.rs`
returns exactly one file, ours. Every other Rust kernel module puts its tests inline in the
file they test, via `#[kunit_tests(...)]`.

`tests.rs` is 3,011 lines and 92 test functions under a single `#[kunit_tests(vino_protocol)]`.
By subject they already cluster: 11 haar, 10 ella, 5 stream, 4 edid, 3 video, 2 timing,
2 navarro, 2 identity, 2 ctm.

**Distribute them into the modules they test**, so each block commit carries its own tests.
That is both the kernel convention and better for review: a reviewer reading `cp.rs` sees
what pins its wire format.

### Decisions already taken, do not re-litigate

- Keep the HDCP spec names `km`/`kd`/`ks`/`riv`; hierarchy documented once in `hdcp.rs`. **Confirmed by Mike.**
- Dock order is always **Ella, Ridge, Navarro** (oldest first). **Confirmed by Mike.**
- evdi mirrors vino in naming and structure; but evdi is **unlikely to go upstream**, so do
  not justify kernel-wide abstractions by evdi's needs (that is why the shared colour
  pipeline was reverted).
- Fold Mike's own binding fixes into the patch that introduces what they modify.
  ⛔ **Never reshape Lyude Paul's, Colin Braun's, Alice Ryhl's or Onur Ozkan's patches** -
  `git am` them unmodified and put any fix in a later commit of ours. Lyude was unhappy when
  this was got wrong before.

### State at handover

- `v3/phase3-profile` @ `e2e95ee84506` (after the group reorder): builds warning-clean,
  `rustfmtcheck` and `rusttest` pass, 40 lines over 100 columns (the pre-session baseline).
- Backups: `backup/vino-pre-v3-refactor-20260823`, `backup/phase3-pre-autosquash-20260823`,
  `backup/phase3-pre-reorder-20260823`.
- `wip/danilo-wq-v2` @ `d0c860b4d390`: workqueue v2 adopted, builds clean, **needs hardware
  validation before folding in**. Mail to Danilo drafted at
  `outgoing/danilo-scopedwork-delayed.eml`, **not sent**.
- The patch export regenerates for the first time in a while: `tools/regenerate-patches.sh`
  had two pre-existing faults (a `Revert "drm/vino: ..."` subject its group pattern rejected,
  and pinned group starts gone stale). Both fixed; `tools/check-series.sh` confirms the
  export reproduces the branch exactly. Groups: rust-core 34, rust-crypto 2, rust-usb 8,
  rust-drm 62, vino 86.
- Prerequisite message-ids are recorded in `docs/upstream.md`, each verified to resolve.
- ⭐ Boqun Feng's counted-interrupt patch is now in **tip locking/core** (`e901c1510e24`),
  so the interrupt prerequisites are a group to **drop**, not to reference, once a base
  carrying them is used.

### Immediate next action for the next session

Split `session.rs`, `cp.rs` and `video.rs` as above and distribute `tests.rs` into the
modules it tests, all on `v3/phase3-profile`. Then cut the vino series fresh in the
panthor shape from that end state.

---

## 2026-08-24 - Session 1 (close) - further splits done; tests.rs is the handover point

Answering Mike's three questions, with the evidence, then the state.

### Can the files be split further? Yes, and three were

| File | Before | After |
|---|---:|---|
| `session.rs` | 2,329 | `session.rs` 625 + `session/setup.rs` 1,390 + `session/replies.rs` 337 |
| `cp.rs` | 1,816 | `cp.rs` 1,135 + `cp/mode.rs` 327 + `cp/edid.rs` 281 + `cp/cursor.rs` 101 |
| `video.rs` | 2,016 | `video.rs` 346 + `haar/{transform 516, strip 425, records 715}` |

`session.rs` was the worst offender in a way the line count hid: **`send_cp_setup` alone
was 1,376 lines**, 60% of the file, so reading the authentication path meant scrolling past
the entire post-authentication burst.

⚠ **A note for whoever cuts the series.** `send_cp_setup` is still a single 1,376-line
function. Splitting the *function* is a behaviour-affecting refactor of the setup burst,
which is hardware-critical and ordering-sensitive, so I moved it rather than divided it. If
it is divided later, that is a change to test on hardware, not a move.

⛔ **A real cost of splitting the codec, recorded so it is not mistaken for sloppiness.**
Rust lets a child module see its parent's private items but **not a sibling's**. Splitting
`mod haar` into three siblings therefore forced a number of items and `ColourBlock`'s fields
to `pub(crate)`. That is weaker encapsulation in exchange for readable modules; it stays
inside the driver, but it is a real trade and the commit message says so.

Still large and worth a further look before cutting: `drm_sink.rs` (3,981) and
`scanout.rs` (1,783). `vino.rs` (1,622) is fine - panthor's frontend block was 1,473.

### Is one big self-test file normal? No, and vino is the only one

`find rust/ drivers/gpu/drm/ -name tests.rs` returns **exactly one file: ours**. Every other
Rust module in the tree puts tests inline in the file under test, via `#[kunit_tests(...)]`.

`tests.rs` is 3,011 lines: a single `#[kunit_tests(vino_protocol)] mod protocol` holding 92
test functions plus shared fixtures. By subject they already cluster - 11 haar, 10 ella,
5 stream, 4 edid, 3 video, 2 timing, 2 navarro, 2 identity, 2 ctm.

**This is the next task and I deliberately did not start it.** Distributing 92 tests across
~12 modules means deciding where each shared fixture lives, giving each module its own
`#[kunit_tests(vino_<subject>)]` block, and the result **cannot be verified here** - KUnit
tests run at module load on the dock. Starting it at the end of a long session, with no way
to check the outcome, is how a green build hides a test suite that registers nothing. The
file's own doc comment already warns of exactly that failure mode: `#[kunit_tests]` rewrites
the module it is applied to, and an `include!` body "compiles green while registering no
tests at all".

### State at close

`v3/phase3-profile` @ **`d05cf17f9e4d`**, working tree clean.

| Gate | Result |
|---|---|
| `make LLVM=1 -j16 modules` | exit 0, **zero warnings** |
| `make LLVM=1 rustfmtcheck` | exit 0 |
| `make LLVM=1 rusttest` | exit 0 |
| lines over 100 columns | 40 (the pre-session baseline) |

⚠ **No hardware exercised at any point this session.** Ella, Ridge and Navarro are all
untested against this branch, and the code most touched - teardown, presence, brackets, the
control-plane session - is exactly what has wedged docks before.

---

## 2026-08-24 - Session 1 (close, cont.) - series generated; Danilo mail SENT by Mike

### The five series are generated, with prerequisite trailers

`vino/outgoing/v3/` (gitignored, regenerable):

| Series | Patches | prerequisite-message-id | Recipients |
|---|---:|---|---:|
| `rust-core` | 34 | Danilo's workqueue v2 | 90 |
| `rust-crypto` | 2 | none | 30 |
| `rust-usb` | 8 | Colin Braun's URB RFC | 19 |
| `rust-drm` | 62 | Lyude Paul's KMS RFC v3 | 30 |
| `vino` | 86 | none (**provisional**, to be re-cut) | 37 |

Every cover carries `base-commit: 4c9ba407018e...`, no `*** SUBJECT HERE ***` or
`*** BLURB HERE ***` placeholders remain, and each subsystem cover answers the specific v2
review it received. **That closes blocker 3** for the four subsystem series; the vino cover
is explicitly marked provisional because the driver is being re-cut.

⚠ `scripts/get_maintainer.pl` does **not** find the people who actually reviewed v1/v2 -
it finds maintainers and lists. `outgoing/v3/REVIEWERS-TO-ADD.md` records who must be Cc'd
by name per series (Oliver Neukum, Alan Stern, Eric Biggers, Lyude Paul, Julian Braha,
Miguel Ojeda) or they will not see the revision that answers them.

⚠ **Miguel Ojeda's v1 request is still outstanding**: link the related series in the cover
letters and say plainly that vino is the user for all of them. It is noted in
`REVIEWERS-TO-ADD.md` and must be in the final covers.

### Mail to Danilo: SENT

Mike sent it manually on 2026-08-24. It reports that workqueue v2 leaves no way to cancel a
`DelayedWork` synchronously: `cancel_sync` is dropped, `ScopedWork` cancels via
`cancel_work_sync()`, and there is no delayed variant, so wrapping a `DelayedWork` in
`ScopedWork` would let the work fire after the data it borrows is dropped.

**Watch for a reply** on `[PATCH v2 6/6] rust: workqueue: add ScopedWork ...`
(`20260807165252.3849875-7-dakr@kernel.org`). The answer decides whether we keep carrying
Onur's `cancel_sync`, restructure to avoid delayed work, or wait for v3.

### Hardware validation is next, and needs a reboot

Written up for Mike at `outgoing/HW-VALIDATION-workqueue-v2.md`. The essential point: the
branch changes `rust/kernel/workqueue/`, so `make modules` alone fails with a `modpost:
undefined!` that points at entirely the wrong thing. A full `make LLVM=1` is required, then
install and reboot.

The migration touches **teardown**, so that is where a regression would appear: boot
selftests `fail:0`, both panels lit by eye, then `remove_all` plus `modprobe -r vino`
returning promptly. Check for a **D state** before blaming the dock.

---

## 2026-08-24 - Session 2 - tests distributed, drm_sink split again, vino series re-cut

Three tasks, all done, all on new commits. **Nothing pushed, nothing sent.**

### 1. drm_sink.rs split by responsibility: 3,981 -> 1,859

`impl VinoDrmData` alone was 1,681 lines of it, so the profile accessors, the mode
admission checks, the work publication and the workers were one scroll. Six new modules:

| Module | Lines | What it holds |
|---|---:|---|
| `drm_sink/timeline.rs` | 571 | the measured cold-wake and dock-wide timelines, `NavarroColdOp` |
| `drm_sink/limits.rs` | 454 | clock/refresh ceilings, `timing_key`, `effective_timing`, the dock budget |
| `drm_sink/settings.rs` | 473 | the runtime knobs a profile installs, and their accessors |
| `drm_sink/dispatch.rs` | 446 | `queue_cmd`, `queue_scanout`, cursor recording, scanout selection |
| `drm_sink/worker.rs` | 571 | the `WorkItem` impls and the KMS reconcile loop |
| `drm_sink/driver.rs` | 208 | `impl drm::Driver` / `KmsDriver`, the GEM and file types |

⛔ **The visibility cost is real and is not sloppiness.** A Rust module sees its parent's
private items but not a sibling's, so items a *sibling* under `drm_sink/` reads had to become
`pub(crate)` -- and anything nested one level deeper (`timeline::cold`, `NavarroColdOp`'s
methods) needs `pub(crate)` even for a sibling, because `pub(super)` there only reaches
`timeline`. Everything else was tightened back to `pub(super)` by measurement, not by guess.

### 2. tests.rs distributed - 88 tests, 18 suites, and vino is no longer the tree's only `tests.rs`

`find rust/ drivers/gpu/drm/ -name tests.rs` now returns **nothing**.

⭐ **The oracle that made this safe to do without hardware**, which the last session was right
to worry about: `#[kunit_tests]` registration is *readable out of the object file*.

```
llvm-objdump -h drivers/gpu/drm/vino/vino.o | grep 'kunit_test_suites '   # size/8 = suite count
llvm-nm --defined-only .../vino.o | awk '$2=="T"' | grep -oP 'kunit_rust_wrapper_\K\w+' | sort -u
```

Before: 1 suite, 82 unique wrapper symbols. After: **18 suites, the same 82 symbols**, and 88
`#[test]` in source both times. A suite that silently registered nothing would show up as a
missing pointer in `.kunit_test_suites`, which is exactly the failure mode `tests.rs`'s own
doc comment warned about.

Distribution (suite name -> file): `vino_cp` cp.rs 12, `vino_cp_mode` 11, `vino_cp_edid` 7,
`vino_cp_cursor` 1, `vino_haar_transform` 8, `vino_haar_records` 5, `vino_haar_strip` 4,
`vino_video` 3, `vino_video_arm` 1, `vino_profile` 7, `vino_crypto` 2, `vino_firmware` 1,
`vino_sink` drm_sink.rs 4, `vino_timeline` 6, `vino_mode_limits` 3, `vino_scanout` 2,
`vino_mode_objects` 1, and `drm_color_pipeline` in color.rs 10.

⚠ **color.rs is shared verbatim with evdi and `tools/color-selftest.sh` enforces that.** So its
tests are gated `#[cfg(any(CONFIG_DRM_VINO_KUNIT_TEST, CONFIG_DRM_EVDI_KUNIT_TEST))]`, named
`drm_color_pipeline` rather than `vino_*`, and name nothing outside the module -- the block is
byte-identical in both copies and the drift check still passes. **`CONFIG_DRM_EVDI_KUNIT_TEST`
is new in evdi's Kconfig.** Consequence worth knowing: both modules build from one `.config`,
so enabling *either* symbol builds the colour tests into *both* drivers.

✅ Fallout the distribution cleaned up on its own: the four `#[cfg(CONFIG_DRM_VINO_KUNIT_TEST)]`
re-exports in drm_sink.rs that existed only so tests.rs could reach `rot_src`,
`changed_strip_rects` and `parallel_rotation_matches_serial` are gone, and so is the
`let _ = EINVAL; // silence unused import` hack in `rgb565_packing`.

### 3. The vino series re-cut in the panthor shape: 88 commits -> 15

`v3/vino-recut` @ **`4be706928ceb`**, branched from the last rust-drm commit
(`55da92cc9d6f`). Pre-recut state is `backup/phase3-pre-recut-20260824` (= `v3/phase3-profile`).

| # | Commit | Files | Lines |
|---:|---|---:|---:|
| 1 | `rust: firmware: add the firmware upload abstraction` | 1 | 237 |
| 2 | `drm/vino: add the DL3 wire framing` | 1 | 71 |
| 3 | `drm/vino: add the USB transport` | 1 | 250 |
| 4 | `drm/vino: add the crypto primitives and the HDCP 2.2 AKE` | 4 | 420 |
| 5 | `drm/vino: add the encrypted control plane` | 4 | 3,059 |
| 6 | `drm/vino: add the dock profiles` | 1 | 1,068 |
| 7 | `drm/vino: add the video codec` | 6 | 3,291 |
| 8 | `drm/vino: add control-session bring-up` | 3 | 2,352 |
| 9 | `drm/vino: add the KMS device and the atomic path` | 7 | 4,934 |
| 10 | `drm/vino: add the dock activation and scanout path` | 8 | 5,352 |
| 11 | `drm/vino: read the dock's firmware version, and update it over DFU` | 1 | 670 |
| 12 | `drm/vino: add the USB driver frontend` | 1 | 1,615 |
| 13 | `drm/vino: allow the driver to be built` | 5 | 50 |
| 14 | `Documentation/gpu: document the Vino driver` | 3 | 201 |
| 15 | `drm/evdi: add the Rust virtual display driver` | 13 | 2,488 |

No rename commit, no fixup, no revert. Kconfig/Makefile/MAINTAINERS is commit 13, second to
last, exactly as panthor's `Allow driver compilation` is -- so commits 2..12 deliberately do
not build as a driver.

⭐ **The sink was split into two commits (9 and 10) rather than left at 10,286 lines.** One
commit would have been 2.4x panthor's largest block (`panthor_sched.c`, 3,552 added), which the
brief itself calls unreviewable. The seam is textual as well as logical: commit 9 adds
`drm_sink.rs` carrying only the `mod` lines for the files it adds, and commit 10 adds the other
seven files *and* their `mod` lines. Neither commit has a `mod x;` without an `x.rs`.

⚠ **`Assisted-by:` only -- no `Signed-off-by:`.** An AI agent must not certify the DCO, and 78
of the 88 commits it replaced had no SoB either, so the tree's own convention is that Mike adds
it at send time (`git format-patch --signoff` / `git rebase --signoff`). **That has to happen
before this is posted**; the previous export was inconsistent about it (3 of 86 patches had one).

✅ Three ordering faults fixed while re-cutting, and they are the *only* tree differences from
the pre-recut branch besides one typo: `drivers/gpu/drm/Kconfig`'s list says "sorted" and had
`evdi`/`vino` wedged between `bridge` and `etnaviv` (now after `etnaviv` and after `vgem`);
`Documentation/gpu/drivers.rst` had `vino` after `vkms`; the Makefile had evdi before vino,
which contradicts the commit order. The typo: "costs a busy one one spinlock per frame".

### Gates

| Gate | Result |
|---|---|
| `make LLVM=1 -j16 modules` (vino + evdi) | exit 0, **zero warnings** |
| `make LLVM=1 rustfmtcheck` | exit 0 |
| `make LLVM=1 rusttest` | 7 passed, 0 failed |
| `tools/color-selftest.sh` | byte-identical + **15 pass, 0 fail** |
| `checkpatch.pl --strict` over all 15 patches | 0 errors; warnings are only the 11 "does MAINTAINERS need updating?" false positives (it is, in commit 13) and 17 long string literals rustfmt leaves alone |
| non-ASCII in `drivers/gpu/drm/vino/*.rs` | none |
| lines over 100 columns | 40 (the pre-session baseline) |
| `tools/regenerate-patches.sh` with `KERNEL_HEAD=v3/vino-recut` | exit 0; groups rust-core 34, rust-crypto 2, rust-usb 8, rust-drm 62, **vino 15** (was 86) |

⚠ **No hardware exercised at any point this session, again.** Ella, Ridge and Navarro are all
untested against this tree. The code moved most -- the sink's activation, presence and worker
paths -- is exactly what has wedged docks before.

### Next session

1. **Hardware validation**, which is now two branches deep: `outgoing/HW-VALIDATION-workqueue-v2.md`
   still applies, and nothing since the 2026-08-23 refactor has met a dock.
2. **Decide which branch is canonical.** `v3/vino-recut` holds the re-cut; `v3/phase3-profile`
   holds the pre-recut history; `vino` is still at `7445bb14b8fc` and behind both. ⭐ Push
   `vino/linux` before the `vino/` superproject, and take a backup of the previous remote tip.
3. **Re-cut the vino cover letter**, which `outgoing/v3/vino/` marks provisional, and add
   Miguel Ojeda's outstanding v1 request: link the related series and say plainly that vino is
   the user for all of them.
4. Watch for Danilo's reply on `ScopedWork` (`20260807165252.3849875-7-dakr@kernel.org`).

---

## 2026-08-24 - Session 2 addendum - signoffs, `vino` repointed, and a DPMS regression found

### Signoffs

`Signed-off-by: Mike Lothian <mike@fireburn.co.uk>` added to **all 60 of Mike's own commits** in
`integration/base-20260809..vino` (43 already had it; 17 did not, plus the 15 re-cut ones).

⛔ **The 61 commits by other people were deliberately left alone** -- Lyude Paul (43), Boqun Feng
(9), Colin Braun (3), Alice Ryhl (3), Onur Ozkan, Joel Fernandes, Heiko Carstens. They are `git am`'d
prerequisites referenced by message-id, not patches being reposted, and reshaping them is what upset
Lyude before. ⚠ If any of them ends up genuinely *carried* in a posted series rather than referenced,
that one needs Mike's SoB added by hand, because the DCO wants it from whoever passes a patch on.

### `vino` now points at the re-cut

`vino` = `c564f4929c8c`. Backups taken first: `backup/vino-20260824-pre-recut` (the old local tip
`7445bb14b8fc`) and `backup/vino-remote-fdo-20260824` (the previous **remote** tip `bebe3029d79e`).
`backup/vino-pre-signoff-20260824` holds the recut before signoffs. **Still nothing pushed.**

### ★★★★★ The Navarro DPMS symptom is a real regression, and the culprit is `35a4880a1919`

Reported: the monitor on the DL-7400 goes black on DPMS but stays lit. ⭐ **It used to work, and
`git log` on the subsystem found it in one step -- no wire work at all.**

`35a4880a1919 drm/vino: name the behaviour a dock wants, not the model it is` replaced
`is_navarro()` with a profile field. The predicate appeared **twice** in `blank_head`, and the two
occurrences were mapped to two *mutually exclusive* enum values:

```
        if self.is_navarro() { 2f=1; 2e=3; hold bracket; return }   ->  == SinkDown
        ... black frames ...
        if self.is_navarro() { return }                             ->  == BlackFrames
        2f=0; 2e=0; power_down_sink(sink_down_state())
```

The second check was **unreachable** before the refactor (the first returns early), so it carried no
behaviour -- but it was translated as though it did, and `PROFILE_NAVARRO` was then given
`BlackFrames`. The result inverts every dock:

| Dock | before `35a4880a1919` | after it | now |
|---|---|---|---|
| Navarro | `2f=1`, `2e=3`, bracket held | black frames, **stays lit** | restored |
| Ridge | black, `2f=0`/`2e=0`, sink down `2e=1` | `2f=1`, `2e=3` held | restored |
| Ella | black, `2f=0`/`2e=0`, sink down `2e=3` | `2f=1`, `2e=3` held | restored |

⛔ It also made the close bracket, the `connector_present` guard and `power_down_sink` **dead code**
inside `blank_connector` -- so `sink_down_state` stopped being read there at all and every dock
blanked with a hardcoded `3`. Ridge's HW-verified `2e(1)` had not been sent since 23 August.

⚠ **The code was already right and only the profile was wrong.** The `SinkDown` branch is Navarro's
disable byte for byte -- its comment, its markers and its debug string all say Navarro. And the
"until a transcript settles the real sequence" comment was stale: `docs/protocol/navarro-decoded.md`
§2d has had the measured sequence ("confirmed byte-identical on a second window") for weeks.

**Fixed** in `c564f4929c8c`: the variants are renamed for what they do (`MarkersHeld`,
`BlackThenClose`), each dock gets the shape measured from its own vendor, the unreachable branch is
deleted so the close path is live again, and the held-marker branch reads `sink_down_state()` instead
of a literal. Verified equivalent to `35a4880a1919^` per dock. A KUnit case
(`blanking_follows_the_vendor_disable_for_each_dock`) pins all three, because nothing on the wire
reports a blank that only paints.

⚠ **HW-UNVALIDATED, and it changes DPMS on all three docks.** Kept as one commit on top rather than
folded into commits 6 and 10, so a bad result is a single revert. **Fold it once validated.**

Test plan, in order of what it would cost to get wrong:
1. **Navarro first** -- DPMS off should take the signal away (monitor reports no input), and DPMS on
   should relight. ⚠ Watch for the ~2 s re-enumeration: if the dock resets, the profile is wrong and
   the whole desktop goes with it.
2. **Ridge** -- confirm `2e(1)` is on the wire again and the panel darkens, per the 2026-07-27 result.
3. **Ella** -- ⛔ use `kscreen-doctor output.X.disable`, not the panel's own power button.

### Gates after the fix

`modules` zero warnings, `rustfmtcheck` clean, `rusttest` 7/0, colour selftest 15/0, **18 KUnit
suites / 83 test wrappers** (was 82; one added). Export regenerated: 122 patches, vino group **16**.

---

## 2026-08-24 - Session 2, HW results - blanking is measured on all three docks

### The blank fix was two-thirds right, and the wrong third was destructive

User-tested on hardware. **Navarro and Ridge: confirmed good.** Ella: broke, and I broke it.

⛔⛔ **Blanking a DL-3x00 with black frames halts its pipe and kills the session.** Its measured
disable is the held-marker pair alone (`2f=1`, `2e=3`) with **no video in the window**. Video shares
the control pipe on that platform, and the 2026-08-10 capture measured a black desktop
**quadrupling** EP02 traffic (14.0 -> 62.7 MB), which is exactly the load its endpoint halts under:

```
vino: scanout connector=1 pipeline submit at off=458752/1519552 failed
vino: shared video/control pipe failed (EPIPE); abandoning the session
vino: resetting the dock to recover the control session
vino 2-2.1:1.0: disconnected
```

⇒ connector gone from KWin, dock still scanning out its last frame, so the panel **kept showing the
desktop after being asked to blank**. The panel then came up **corrupted** -- block noise over ~2/3
of the width, 1px vertical stripes over the rest, i.e. a decoder reading at the wrong offset, left
by the reset tearing the stream mid-frame. ⭐ **A clean vino reload cleared it**; the codec was never
involved. Fixed in `a51162b2813f` (a `fixup!` for `c564f4929c8c`).

### ⭐ The lesson, and it is the one that cost the outage

I justified Ella's value as "restore what it did before the refactor". That code **predated the
capture that isolated this dock's disable**. Age is not evidence -- the newest measurement is.
Two of three docks happened to be right; the one I reasoned about historically was wrong.

⚠ Second-order: the KUnit case I added to stop this recurring asserted the two non-Navarro docks
*as a group*, which is the same exclusion-list shape I had just fixed elsewhere in the same file.
It would not have caught this. Each dock is now pinned individually.

### The settled table, each row from its own vendor capture

| Dock | blank | `sink_down_state` | status |
|---|---|---|---|
| Ella (DL-3x00) | `MarkersHeld`, **never paints** | 3 | ✅ user-confirmed good |
| Ridge (DL-6xxx) | `BlackThenClose` | 1 | ✅ user-confirmed good |
| Navarro (DL-7400) | `MarkersHeld` | 3 | ✅ user-confirmed good |

⚠ **Ella's DPMS itself is still untested** -- what is confirmed is that it renders correctly again.

### Two false alarms, recorded so they are not re-investigated

- ⛔ "The Ella has no monitor" and "the Ridge lost its monitor" were **neither**. Mike has two
  monitors and moves them between docks to test. Every appear/disappear in dmesg is that.
- ⛔ Three power cycles did **not** trigger an auto-flash; firmware stayed 12.2.15 throughout.

### Other fixes this stretch

- `2185c53ecaf9` -- the selftest that had failed at **every module load** since `15a7b070f58e`:
  it asserted "every family except Ella has `frame_period_ms == 5`" while the DL-6xxx had been
  measured at 8. Now pinned per family. ⚠ My earlier claim of "`pass:37 fail:0`" was stale CLAUDE.md
  text; the real figure is **18 suites / 89 tests**, now `fail:0`.
- ⛔ `/sys/devices/vino/remove_all` **does not exist** in this branch, so the documented teardown
  recipe is stale. Teardown is: unbind each interface under `/sys/bus/usb/drivers/vino/`, wait for
  `refcnt` to reach 0, then `modprobe -r`. Script: `scratchpad/reload-vino.sh`, run detached.
- ⭐ A module-only install needed no reboot, and that was **checked, not assumed**: the new .ko's
  undefined-symbol set was identical to the running one's. This config has neither `MODVERSIONS`
  nor module signing, so a real ABI mismatch would have loaded silently.

### Next

1. **Test DPMS on the Ella** -- the one behaviour still unconfirmed. Use
   `kscreen-doctor output.X.disable`; ⛔ `--dpms off` never reaches this dock.
2. Fold `a51162b2813f` into `c564f4929c8c` (`git rebase -i --autosquash`) once that is confirmed.
3. Then fold the whole blank fix into commits 6 and 10 of the re-cut series.

---

## 2026-08-24 - Session 2 close - fixes folded, series respun

### Folded, with the tested tree preserved exactly

The three follow-up commits are gone; their content now sits in the commits that introduced the
lines, which is where a reviewer will look for it:

| Fix | folded into |
|---|---|
| blank bracket per dock + its KUnit case + the Ella correction (`profile.rs`) | `drm/vino: add the dock profiles` |
| `blank_markers_held` field and accessor (`drm_sink.rs`, `settings.rs`) | `drm/vino: add the KMS device and the atomic path` |
| the blank path itself (`bracket.rs`) | `drm/vino: add the dock activation and scanout path` |
| frame period pinned per family (`profile.rs`) | `drm/vino: add the dock profiles` |

⭐ **The invariant that makes this safe to claim:** `git diff backup/vino-pre-fold-20260824 vino`
is **empty**. The folded tree is byte-identical to what ran on the hardware, and the installed
`.ko` hash is unchanged, so no reload was needed and nothing was re-validated by assertion.

⚠ `git rebase -i`'s todo here is `pick <sha> # <subject>`, not `pick <sha> <subject>` -- this repo
sets `rebase.instructionFormat`. A sequence-editor script matching the documented format silently
matches nothing and the rebase completes as a no-op. Dump the todo before trusting a regex.

### Respun

`tools/regenerate-patches.sh` with `KERNEL_HEAD=vino`: **121 patches**, groups rust-core 34,
rust-crypto 2, rust-usb 8, rust-drm 62, **vino 15**. `tools/check-series.sh` confirms the export
reproduces the branch. checkpatch `--strict`: **0 errors**; warnings are only the 11 "does
MAINTAINERS need updating?" false positives and 17 long string literals rustfmt leaves alone.
All 15 vino patches carry Mike's `Signed-off-by`.

Gates: build zero-warning, `rustfmtcheck` clean, `rusttest` 7/0, 18 KUnit suites.

### Two things seen on hardware that are NOT from this work

- ⚠ **The Ella EPIPE'd again at t=47926**, with markers-held blanking in place and no blanking
  involved: `off=524288/1444032`, the 8-deep URB queue boundary. This is the pre-existing
  load-triggered EP02 halt ([[project_ella_epipe_reproduced_under_load_20260817]]), independent of
  the blank path. Removing black frames from the blank removed *one trigger*, not the fault.
- ⚠ **A DRM minor was consumed and not reused**: after that reset the Ella came back on minor 5,
  having been on minor 4, so `card4` is gone and `card5` is live. CLAUDE.md records the minor leak
  as root-caused and fixed, but that was verified across *module reloads*; this was a **dock reset
  and rebind**, which may not take the same path. Unverified either way -- worth a look.

### ★★★★ A cold-plugged Navarro published the dock's own NOVATEK EDID

Reported as "Navarro garbled, and the EDID isn't our usual testing monitor". It was not a codec
fault and not from the blank work.

`card3-DP-5` carried monitor name **`NOVATEK`** -- the dock's own bridge block -- 26 modes topping
out at 1920x1080, on a socket holding an MSI MAG 27CQ6F (2560x1440). The panel showed two narrow
columns of white dashes at the far left and dark elsewhere: a **geometry mismatch**, exactly what
`session/setup.rs` predicts for this case ("drives the panel at a timing it never advertised").

⭐ **Diagnose from the EDID, not the picture**: `cat /sys/class/drm/cardN-DP-M/edid | strings`.
Two identical monitors are told apart by serial -- `CD9M145800302` (Ridge) vs `...341` (Navarro).

⛔ **The guard for this already exists and is Ridge-only.** `gate_on_ready` in `session/setup.rs`
discards a block offered before the dock reports its downstream read complete (presence reply offset
26 bit 7), and it is keyed on `profile.quirks.shared_edid_handler`: **Ella false, Ridge true,
Navarro false**.

⚠ **Cold-only.** A physical replug produced it; a plain vino reload re-probed and both docks read
their real EDIDs (Navarro back to 37 modes / 2560x1440). ⇒ recovery is a re-probe; reproducing it
needs a physical replug.

⛔ **Do not just set `shared_edid_handler: true` on Navarro.** The same flag suppresses the blind
re-engage in `vino.rs`, which is right where one handler serves several connectors and unproven on a
four-connector dock that pushes hotplug. And if offset-26 bit 7 is not meaningful there, gating
discards **every** EDID and the dock never gets a monitor -- a wrong guess costs a power cycle.
**Next step: settle that bit against the Navarro corpus, then consider splitting the flag** into the
readiness gate and the shared-handler behaviour.

### Settling offset-26 bit 7: the corpus cannot, live hardware can

Asked to settle whether the EDID-readiness gate is safe to enable on Navarro.

⛔ **The corpus cannot answer it, and that is a finding rather than a shrug.** Checked exhaustively:
`captures/` holds **5** readiness records in total, all Ella, all `ready=false`; the Windows Navarro
captures skipped session keys **by design** ("Phase 6 -- impossible here, there is no DisplayLink
user-mode process to hook"); `navarro-dlm-control-094605` has a `.mon` and no keys, and the
`keys-raw.json` in `navarro-dlm-today-124144` is the **string-store** key, not a session key.

⭐ **`modprobe vino debug=1` settles it in one load**, printing the bit for every socket of every
dock:

| dock | socket | status | ready |
|---|---|---|---|
| Navarro `[video 08/0a]` | 1, 3, 4 (empty) | `0x00200105` | **false** |
| Navarro | 2 (**monitor**) | `0x00271105` | **true** |
| Ella `[video 02/02]` | 1 | `0x00100104` | false |
| Ella | 2 | `0x00300105` | true |

**Result:** ✅ **Navarro sets the bit, and only on a populated socket** -- so gating there cannot
discard every EDID, and the failure mode that made a blind flip unsafe is ruled out. The presence
values also agree with the four-connector port map (`status & 0x1000` set only on socket 2).

⛔ **Ella must keep the gate off**, and now for a measured reason rather than caution: its EDID fetch
never reaches ready (`readiness poll hit wall-clock cap` -> `ready=false`) and then **succeeds
anyway**. Enabling the gate there would discard the EDID the dock depends on.

⚠ **One thing is still open, and it is the one that decides the fix.** The presence path and the
fetch path read the same bit at different moments. What is proven is that Navarro *sets* it; what is
not is whether it is set *before* the bridge block is offered on a **cold** dock -- the only case
that fails. This load re-used a cached EDID, so it never exercised that path.

**Next: one physical replug of the Navarro with `debug=1` loaded** (it is loaded now). That both
reproduces the original NOVATEK failure under instrumentation and shows the ordering. If ready
arrives first, the fix is to split `shared_edid_handler` into the readiness gate (Navarro + Ridge)
and the shared-handler blind-engage suppression (Ridge only).

⚠ vino is currently loaded with `debug=1` -- verbose. Reload without it when the cold-plug test is
done.

### ⛔⛔ REFUTED: the EDID readiness gate must NOT be enabled on Navarro

The cold plug was done with `debug=1` loaded, and it **came up correctly** -- `EDID read from dock
(384 bytes)`, two extension blocks, `MAG 27CQ6F CD9M145800341`, 37 modes. So the NOVATEK failure is
**intermittent**, i.e. a race, not a deterministic cold-plug property.

⛔⛔ **And the ordering refutes the fix that was about to be written:**

```
50009.221   EDID read from dock (384 bytes)          <- the GOOD EDID
50010.853   presence reply socket 2 ready=true       <- 1.6 s LATER
```

with **zero** readiness-poll records during that fetch. `gate_on_ready` discards whenever
`!edid_ready`, so on this evidence enabling it for Navarro would have thrown the **good** EDID away
and left the dock with no monitor. **It would break the working case rather than fix the broken
one.** The readiness bit is therefore *not* the discriminator for this failure, even though Navarro
does set it in the presence path.

⭐ This is the same trap as the Ella blank: a change that is plausible, has a real mechanism behind
it, and is wrong because it was never checked against the case that already works. Checking cost one
cold plug; not checking would have cost a dark dock and a power cycle to find out.

**Whatever fixes the NOVATEK block must distinguish it from a monitor block by content or
provenance, not by that bit.** Catch the next occurrence with `debug=1` loaded before proposing
anything.

### State

**No new fix to fold** -- everything from today (blank bracket per dock, the Ella correction, the
frame-period assertion) was already folded into commits 6, 9 and 10. Working tree clean.

Respun and verified: **121 patches**, groups rust-core 34, rust-crypto 2, rust-usb 8, rust-drm 62,
**vino 15**; `check-series.sh` reports the export reproduces the branch; 15/15 vino patches carry
Mike's `Signed-off-by`. Nothing pushed, nothing sent.

⚠ vino is still loaded with `debug=1`. Worth leaving on to catch the intermittent NOVATEK race;
reload without it for a quiet dmesg.

### The Navarro wake: what five trials actually settled

⛔⛔ **The hard failure never reproduced.** Five deliberate attempts, including the exact
combination that produced it:

| | blank | refresh change | flaps | outcome |
|---|---|---|---|---|
| original failure | 168 s | yes | **22** | never recovered |
| trial 1 / 2 | 8 s | no | 0 | clean |
| trial 3 | ~0 s | yes | 0 | clean |
| trial 4 | 180 s | no | 0 | **wake visibly slow** |
| repro attempt | **857 s** | **yes** | 0 | instant |

⇒ neither variable, nor both together, is sufficient. Do not treat a quiet DPMS cycle as proof of
anything; **the only reproducible symptom was the slow wake after a long blank.**

⛔ Three hypotheses died on the way, all plausible, all wrong: the 1440p165 mode words (they are
derived from the measured sync-polarity rule, not guessed, and Mike confirms that mode worked);
the refresh change; and "vino fights its own blank" -- the presence oscillation during a blank is
**documented dock behaviour** and `vino.rs` already guards it (`if data.is_self_blanked(h)`), which
I found only after reporting it as the bug.

### ✅ The fix, and what it is judged on

`f7805ebabd82` -- on a held-marker dock, `close_blank_bracket` now runs the existing
`reengage_connector` after the closing markers. That function already **is** the vendor's wake
(probe, fetch, engage, capability query) and clears the self-blanked record and asserts the closed
bracket on entry. ⚠ Corrected en route: the wake was **not** missing its set-mode --
`close_blank_bracket` is called from inside the activation path, so the mode was always re-sent.
Only the probe/fetch/engage was absent.

HW result, user-confirmed: a **738 s** blank with a refresh change came back **quick**, against
trial 4's 180 s pre-B blank which was visibly slow, and the log now shows the re-engage retrieving
the real monitor EDID:

```
socket 2 EDID 384 B, vendor MSI product 0x3cd9
socket 2 blank bracket closed; wake runs as a repair
```

⚠ **One trial, and against a baseline that does not reproduce** -- so this is evidence the
degraded wake is fixed, not that the hard failure is.

### Open, unrelated: HDR is 8-bit

Mike observed the monitor reports HDR but 8-bit depth. Cause found, not fixed:
`attach_max_bpc_property(8, 10)` in `drm_sink/driver.rs` is the **only** mention of max bpc in the
driver -- the property is advertised and **never read**. Depth comes solely from the committed
framebuffer's fourcc (`dispatch.rs` -> `set_connector_depth`), so the dock is told 24 bpp unless
KWin commits `XRGB2101010`, while off42 bit 6 (ST2084) is set. HDR transfer function on, 8-bit
samples. ⚠ Only the Navarro is `10-bit capable true`; Ridge and Ella are false.

---

## 2026-08-24 - Session 2 (afk) - 10 bpc reaches the DL7400

### ⭐ The gap was `max bpc`, and vino never read it

Everything downstream of the trigger was already correct and pinned by tests: the plane offers
`XRGB2101010` on an `hdr_capable` dock, `from_fourcc` maps it to `Depth::Ten`, the set-mode emits
`off23=3` (NM30) and `off68/69=0x0300`, and the codec uses DC ceiling 12 at 10-bit with the AC
ceilings left at their 8-bit values and `esc` saturating so a wrong ceiling clips rather than
desynchronises. **Nothing was missing except the trigger.**

⛔ **`attach_max_bpc_property` was the only mention of max bpc in the driver.** Dumped the
connector properties and found userspace asking for exactly what it was not getting:

```
connector 54 (DP-7): max bpc = 10  Colorspace = 9  HDR_OUTPUT_METADATA = 90
connector 43 (DP-6): max bpc = 10  Colorspace = 0  HDR_OUTPUT_METADATA = 0
```

KWin sets `max bpc = 10` and keeps committing `XRGB8888` -- and that is **correct behaviour**:
`max bpc` describes the *link*, not the buffer, and an eight-bit surface over a ten-bit link is the
ordinary case everywhere. The Windows captures agree: `bpc 8 <-> 10` per head on an HDR toggle.
So the bug was vino deriving link depth from the framebuffer format.

### The fix, and the HW result

Split the two meanings: the fourcc still decides how a pixel is **decoded**, `max bpc` decides what
the dock is **told** and what the codec emits. Where they differ the sample is widened after
decode, replicating the top bits so `255 -> 1023` exactly; a plain shift leaves white three codes
short and tints every highlight. Pinned by `widening_an_eight_bit_sample_keeps_the_endpoints`.

```
KMS CRTC enable -- connector 0 ... 2560x1440@120 10 bpc         <- HDR off
KMS CRTC enable -- connector 1 ... 2560x1440@165 10 bpc PQ      <- HDR on
```

**757 frames, 0 errors, 0 sink disconnects, selftests 90/0.** The dock accepts the ten-bit stream
and keeps running. ⚠ **Panel content is unverified** -- nobody was at the machine; the dock
accepting bytes is not proof a panel is right.

⛔ **Refuted en route:** advertising `IN_FORMATS` / the linear modifier does **not** make KWin pick
a ten-bit buffer (tested: 755 frames, still eight-bit). Kept as its own commit anyway, because the
framebuffer path accepts only linear and saying so is honest.

### Commits

| | |
|---|---|
| `3cda1ad359e9` | `rust: drm: kms: expose a connector's requested link depth` (binding) |
| `60f4f58a11ca` | `drm/vino: publish the plane's format modifier` |
| `aec4730bd967` | `drm/vino: drive the link at the depth userspace asked for` |
| `e62e082ce489` | `drm/vino: say which sample depth a connector is being driven at` |

⚠ Still unmeasured: the **10-bit AC ceilings**. `ac-hdr.webm` exists in
`navarro-wincap-20260805/hdr-content/ac/` but **no capture of it was ever taken**, so vino's
saturating 8-bit AC ceilings remain the safe choice.
⭐ Read the depth with `modprobe vino debug=1` and grep `KMS CRTC enable`.

### ⛔ The sink flapping is NOT from 10 bpc

Measured on one boot, so the only variable is the depth:

| period | duration | flaps | rate |
|---|---|---:|---|
| 8-bit, after the reload | 90 s | 8 | 5.3/min |
| **10-bit active** | 160 s | 12 | **4.5/min** |
| blanked (idle) | 793 s | 0 | 0 |
| earlier 8-bit periods, same boot | 1550 s | **0** | 0 |

The rate is the same either side of the change, and there is a flap **10 s before** the 10-bit mode
set. ⇒ depth is not the variable.

⭐ What is different in the flapping periods: **both monitors are on the Navarro**, so two
connectors are lit on one dock, where every quiet period had one monitor per dock. That points at
[[project_navarro_modeset_is_dockwide_20260806]] and the presence debounce, not at anything from
today. ⚠ Do not re-blame 10 bpc for this.

⭐ Note the flaps stop entirely once the connectors blank -- the `self_blanked` guard works.

---

## 2026-08-25 - 10 bpc: the encoder is proven correct; the dock's decoder is not switching

### ✅ Proven: vino's 10-bit bitstream is correct

Captured vino's own EP08 output and ran it through the same reference decoder that reads the
Windows corpus:

```
navarro-render.py <vino capture> --depth 8   -> noise
navarro-render.py <vino capture> --depth 10  -> clean content, 0 strips failed
```

⇒ the encoder, the DC ceiling of 12 and the AC ceilings at their 8-bit values are all right. This
also settles it independently against the vendor: probing `cap6` (HDR on) shows ceiling 10 decoding
differently while 11/12/13 agree, i.e. the DC ceiling is above 10, as vino has it.

⛔ **My AC `+2` change was wrong and is reverted.** Raising the AC ceilings desynchronises the dock:
above its ceiling every coefficient below the maximum carries a terminator the dock reads as an
offset bit, and the picture breaks into horizontal bands.

### ✅ Fixed: the strip cache served 8-bit bodies into a 10-bit stream

`63f3772bd8fe`. The cache tag named the colour transform but not the depth, and depth is the same
hazard in a sharper form: it re-maps every sample and moves the escape ceiling while leaving the
framebuffer byte for byte identical, so the content hash hit and stale bodies went out.

### ⛔⛔ The blocker, and why the corpus cannot answer it

The dock renders vino's (correct) 10-bit stream as garbage ⇒ **its decoder is still in 8-bit mode**.
What switches it is unidentified. Eliminated with evidence: the set-mode (carries `off23=3` NM30 and
`off68/69=0x0300`, test-pinned), the per-strip parameter map (a size-class map), and the prologue
ordering (Navarro takes `build_navarro_prologue_buf`, armed *after* the set-mode on every
activation).

⛔⛔ **The decoder configuration is SEALED**, so the corpus cannot settle its contents. Searching for
the `18 00 0b 03` mode header returns **zero** hits even in `cap1`, a cold-boot-to-plug capture that
certainly contains a stream open -- that is the positive control I should have run first, and
without it I drew several worthless conclusions from zero-hit scans today. The Windows captures have
no session keys **by design**.

⭐ **Next**, neither of which is offline work: frida against **Linux** DLM driving a Navarro with HDR
on ([[reference_cp_decrypt_via_frida_live_dlm_20260726]]), or the dock's firmware trace.

### State

Both monitors lit, 1440p120, 10 bpc PQ, 91/0 selftests, build and rustfmt clean -- **and corrupt**,
because the dock-side switch is unsolved. `git revert aec4730bd967` returns to 8 bpc and a correct
picture.

## 2026-08-25 — the 10-bit codec model is verified against the vendor; the ceilings are cleared

Retracted my own claim from earlier in the session that vino's 10-bit encoder was "proven correct".
That test decoded vino's own capture with `tools/codec/`, which shares the encoder's model, so it
only ever proved self-consistency. `navarro-render.py`'s docstring says so explicitly.

Redone against vendor bytes. `out/cap9-hdr-ab-usbpcap1.pcap` is the second Windows set (phaselog:
`bpc 10`, `depth:30`, playing `hdr-pattern.webm`/`hdr-motion.webm`); the rest of `out/` is the
SDR-content set. At `--depth 10` it decodes **6924 of 6928 strips** in its largest frame. The
10-bit model is right.

The unmeasured AC escape ceilings are **not** the cause. Rendering that frame at `luma_ac=9` and
`luma_ac=13` gives byte-identical PNGs -- the ceiling is never reached by this content, so the
"fit prefers 13" from a bitstream-slack sweep is an artifact of scoring an unexercised parameter.
That also explains why the earlier `AC_CMAX + 2` experiment made the picture worse. `dc=14`
overruns, so DC <= 13; 10 vs 12 is not separable from this content. Do not re-chase the ceilings.

So the corruption is not in the strip payload. The photo agrees: strip-aligned runs saturating to
black/white with clean detailed strips between them, and a perfect hardware cursor -- a partition
overrun shape, not the per-row drift a wrong stride gives.

Fixed there: `Allocation::words()` documents that a 30 bpp connector is told three quarters of the
rows a 24 bpp one is, but only `Derived` implemented it; `Measured` and `Fixed` dropped `ten_bit`.
Navarro is `Measured`, tabulated at 24 bpp, so 10 bpc went out as NM30 against a 3 B/px row count
(`0x66db` where the partition holds 19748). Corrected in the `Measured` arm and pinned by a
`vino_profile` assertion. Selftests 18 suites, pass:140 fail:0; rustfmt and build clean.

Unconfirmed: whether that is the whole corruption. It needs eyes on the panels.

## 2026-08-25 (later) -- the decoder config states the sample format, and vino hardcoded 24 bpp

The sealed stream mode header is `[len][kind][0x0204][count]` then the surface stated twice as
`[format][width][height][layout word]`. That leading word is the same DMA format the set-mode
carries at offset 23 -- `2` for NM24, `3` for NM30 -- and `mode_header()` hardcoded `2` whatever
the depth, while the set-mode for the same connector went out saying `3`. The timing said 30 bpp
and the decoder configuration said 24 bpp.

That is consistent with what the panels show: the strips themselves are valid (the model is
verified against the vendor's own 10-bit stream), so a dock unpacking a valid stream into a surface
of the wrong depth mis-renders it in strip-aligned runs while the cursor plane, which does not go
through the decoder, stays clean.

`mode_header()` now takes the depth from the connector's stored timing, so the pair cannot
disagree. The eight-bit bytes are unchanged and pinned by an assertion alongside the ten-bit ones,
so the known-good path cannot regress.

The dock accepts the new configuration -- no stall, reject, reset or endpoint error across a reload
with both heads at 2560x1440@120 in PQ. That says the value is legal, not that it is right; the
picture still needs eyes.

⛔ Not the cause, established today and not to be re-chased: the escape ceilings. The DC ceiling of
12 is measured (cap9's pq_ramp is monotonic only at 12, inverting at 10 and 11), and the AC
ceilings are never reached by this content -- rendering the same vendor frame at luma_ac 9 and 13
gives byte-identical PNGs.

### The dock's own firmware trace says nothing is wrong at 30 bpp

`trace_crypto` is a runtime module parameter, so the dock's firmware log needs no rebuild: reload
with `trace_crypto=1`, take `key=`/`riv_out=` from dmesg, flip bit 0 of riv byte 7 for the IN nonce
and feed both to `dock-trace-live.py --bus N`. All three docks answer. Navarro and Ridge log the
compact `|<ticks> <msgid><args>` form; the HP dock logs English.

A 25 s Navarro trace at 2560x1440@120 in PQ against the same at 24 bpp gives the same message
vocabulary and the same distribution. One line differed on the first pass -- msgid `7551c` carrying
`e10 e10` at 30 bpp against `cb12 cb13` at 24 bpp, and 0xe10 is 3600, the frame's strip count -- but
it does not reproduce: a second 30 bpp trace gives `e594 e593`, consecutive like the 24 bpp pair,
and the leading argument moves between runs. It is a counter, not a format word.

So the dock reports no decode error, no reject and no stall at 30 bpp. That is a negative result
worth keeping: the dock cannot tell us the picture is wrong, so the firmware trace is not an oracle
for this bug and should not be spent on it again.

### The encoder is correct at 30 bpp, measured against the compositor

The wire was finally checked against something independent of the codec model. Forcing a mode set
on a lit connector (an HDR disable/enable) makes vino send a black keyframe and then repaint the
desktop as deltas, so accumulating the deltas that follow the last full frame reconstructs what the
dock was handed. 26,000 strips accumulated, **0 failed to decode**, covering 3139 of 3600 strip
origins; the uncovered remainder stays black and is a budget limit, not a defect.

Against a `spectacle` capture of the same connector, cropped from the multi-head shot by the
`kscreen-doctor` geometry, the reconstruction matches at **Pearson r = 0.96** over 3.2 M covered
pixels. The residual is the transfer function: the wire is PQ at 30 bpp and the screenshot is sRGB.
Structure -- every subject, edge and boundary -- lands in the same place.

⇒ vino's 30 bpp encoder output is correct. Anything still wrong on the panel is the dock
interpreting a correct stream, which is what the mode header's format word governs.

⛔ The dock's presence/status word is not an oracle for depth either: `0x00271105` (present, ready)
is the steady value across mode sets at both 24 and 30 bpp, with no accompanying change.

### The format=2 vs format=3 A/B is inconclusive

Ran the isolation directly: a build with the mode header's format word pinned to 24 bpp against the
depth-aware one, each traced across a mode set on a lit 30 bpp connector. The dock's firmware log is
the same either way -- 116 message ids, identical histogram counts.

One id, `6ebb1`, showed up only in the pinned build's first run, twice, once per connector, and the
three-way split looked decisive: absent at 24 bpp with a matching config, absent at 30 bpp with a
matching config, present only in the mismatched combination. ⛔ It does not reproduce -- a second
run of the same build has none. Noise, exactly like `7551c` earlier.

⇒ The dock's firmware trace cannot distinguish the two format words. The change stays a reasoned
candidate, not a measured fix; the only remaining test is the panel.

### Next lead if 30 bpp is still wrong: `layout_word` may be a 24 bpp byte stride

`layout_word` sits in the same mode-header descriptor as the format word and is likewise a
hardcoded per-dock constant taken from 24 bpp captures. Its doc says it is not a pitch, but the
numbers do not support that:

| dock | width | layout_word | / 3 |
|---|---|---|---|
| Ella | 1920 | 0x1800 = 6144 | **2048** |
| Navarro | 2560 | 0x2100 = 8448 | **2816** |

Both divide by three to an exact multiple of 256 -- 2048 is the next 256-multiple at or above 1920.
That is what a 24 bpp byte stride looks like, and if it is one it owes a factor of 4/3 at 30 bpp
(Navarro 0x2100 -> 0x2C00). Ridge does not divide by three but gives 4096 on four, and its
allocation stride is separately 0x4000, so it may state bytes a different way.

⚠ Deliberately NOT changed. Two 30 bpp wire changes are already live and unvalidated; a third guess
could mask or undo whichever of them works, with no way to tell them apart. Settle the current pair
against the panel first.

## 2026-08-25 (later) -- ROOT CAUSE: the decoder code tables state the escape ceiling

`CODE_TABLES` are not opaque captured constants. Each is the series `2^n * (2^(n+1) - 1)` truncated
at a category, with the second half repeating each entry less `2^(n+1) - 1` and a terminator of
`2^(2N+2)`. A generator built from that reproduces the shipped tables exactly:

    code_table(8) == CODE_TABLES[0]
    code_table(9) == CODE_TABLES[1] == CODE_TABLES[2]

So a table *is* the escape codebook for one ceiling, with `naturals = cmax - 1`: table 0 is a
ceiling of nine (`AC_CMAX`), tables 1 and 2 a ceiling of ten (`DC_CMAX`, `CHROMA_AC_CMAX`). Ella's
narrow tables mirror the same shape one generation down -- table 0 one power shorter, 1 and 2
identical -- which confirms the reading.

A 30 bpp connector raises the DC ceiling to twelve when *encoding* and vino shipped these tables
unchanged, so it told the dock the ceiling was ten and then emitted category-eleven and -twelve
escapes. The dock desynchronises for the rest of that record and resynchronises at the next one.
That is the panel exactly: strip-aligned runs of saturated black and white, clean detailed strips
between them, flat strips unaffected, and the cursor plane -- which does not go through the decoder
-- untouched.

⭐ It also explains every dead end. The repository decoder takes `cmax` from `Depth` rather than
from the tables in the stream, so vino's own 30 bpp wire decodes perfectly (r = 0.96 against a
screenshot) while the dock cannot; and the vendor's stream decodes fine because the vendor ships
tables that match. A 47-entry record holds exactly eleven naturals, so the record was sized for a
ceiling of twelve from the start.

⚠ Which of tables 1 and 2 is the DC plane is not recoverable: they are byte-identical at 24 bpp
because both ceilings are ten, the RE notes never recorded a table-to-plane mapping, and the vendor
computes the tables at runtime rather than storing them (searched DisplayLinkManager and the
Windows dlidusb*.dll for the constants -- absent). Settled empirically with `deep_dc_table`.

## 2026-08-25 -- SOLVED: 30 bpp raises every ceiling, and each is stated by its own code table

User-confirmed on hardware: both DL7400 panels render correctly at 2560x1440@120 in PQ at 10 bpc.

The remaining half of the bug was the AC planes. A coefficient is four times the sample, so every
depth-sensitive ceiling gains two categories at 30 bpp, not just DC: luma AC nine to eleven, chroma
AC and DC ten to twelve. The two failures look nothing alike, which is why they were chased
separately:

- **DC mismatch desynchronises.** The dock loses the bitstream for the rest of the record and
  recovers at the next one: strip-aligned runs of saturated black and white, flat strips fine.
- **AC mismatch does not.** `esc` saturates and the category still fixes the length, so the stream
  stays in step and only the high-frequency values are wrong: smooth gradients perfect, every text
  edge and icon border speckled.

Both were reproduced and then cured in that order, each confirmed by eye.

The table-to-plane mapping, settled by bisection on hardware: table 0 luma AC, table 1 chroma AC,
table 2 DC. Tables 1 and 2 are byte-identical at 24 bpp because chroma AC and DC share a ceiling of
ten there, which is precisely why nothing in the corpus could distinguish them.

Measured on the live wire during HDR video playback: 1.35 GB, 40,709 frames, no decode failure and
no malformed section offset in 6,000 sampled strips; DC reaches category twelve once in 192,000,
luma AC reaches ten -- above the old ceiling of nine, which is exactly why edges speckled -- and
nothing saturates now.

Both diagnostic module parameters are gone and the behaviour is unconditional. The stale
`haar_depth_selects_the_ac_codebooks` assertion, which pinned the AC ceilings as depth-independent,
now states the +2 rule. Selftests pass with no failures.

⚠ Both panels cap at 10 bpc (EDID byte 0x14 = 0xb5), so a 12 bpp path cannot be exercised here.
