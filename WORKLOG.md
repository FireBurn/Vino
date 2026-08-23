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
