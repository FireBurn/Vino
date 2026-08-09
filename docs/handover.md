# Handover

Single current handover. Last updated **2026-08-09**. Everything below is either still true, or a
trap worth not repeating. Anything an earlier handover said that is not repeated here was done,
superseded, or retracted.

**Read "⛔ Verification traps" first.** Three real defects got through this session because the
build was reporting success while building nothing.

---

## Immediate physical state

- **Both dock panels are dark.** The monitors' sinks are asleep; vino publishes without an EDID
  (`socket 1/3 never answered its EDID fetch`). **A dock power cycle is what recovers this** —
  `authorized` cycling has repeatedly failed to.
- The dock is at `2-1.3`, devnum 27, and has re-enumerated many times today. It moves between USB
  buses; resolve it by `idProduct` (`7000`/`6006`), never by a hard-coded path.
- vino autoloads now (the blacklist was removed), DLM stays masked.

## Repository state

⛔ **Nothing is pushed.** `linux:vino` is a rewrite relative to `github/vino` (`ahead 10, behind
16`). Publishing means: back up the previous remote tip to a branch, push that, then
`git push --force-with-lease` `vino`, **then** the superproject. Never the superproject first.

Two safety tags exist locally: `pre-fold-backup` (before the fold) and `pre-warnfix-backup`.

The series is **10 commits, no `fixup!`s**, 112 patches exported to `patches/kernel/`. It builds
warning-clean and rustfmt-clean at the tip with a real config.

### What landed this session, folded into the commits that own the code

| change | folded into |
|---|---|
| KMS `set_mode_changed` / `old_connector_state_for_crtc`, unsafe cleanup | `rust: drm: kms: read a connector's colorimetry…` |
| HDR mode set, protocol traces behind `debug`, CP silence timer | `drm/vino: add the DRM/KMS scanout engine` |
| EDID recovery interleave, hotplug coalescing, keepalive drop, identity log | `drm/vino: add the DisplayLink DL3 USB display driver` |
| `Version`/`Family`/`Identity` formatting | the two firmware commits |

`Family` was moved earlier in the series so the platform→package mapping is written once, and the
upload commit's `firmware.rs` diff is now pure additions.

⚠ **Uncommitted in the working tree**: the KMS-queue `highpri()` change and the moved CP silence
guards. Commit them before anything else.

---

## ⛔ Verification traps

**A green build can mean nothing was built.** `make modules` re-runs `syncconfig` when the tree's
Kconfig differs from the one `.config` was generated against. With stdin at `/dev/null` it answers
*default* to every new symbol — which silently drops `CONFIG_DRM_VINO` at any commit where the
driver's Kconfig has not landed yet. `make` then exits 0 having compiled nothing.

This hid three real defects: duplicated `impl` methods from a bad conflict resolution, a dropped
`identity_family = id.family();` that would have killed the firmware-upload interface, and a
commit that did not compile at all.

**Always**: restore a known-good `.config` before each build, and *assert* `CONFIG_DRM_VINO=m`
survived. `scratchpad/verify2.sh` does this; it prints `vino=0` when the driver is not in the
build, which is the tell.

**`make M=drivers/gpu/drm/vino` never rebuilds `rust/kernel`.** Every "warning-clean" claimed with
it covers the driver only. Touch a file and do a full `make LLVM=1 -j16 modules` before believing
a warning count.

**Warnings are config-dependent.** `RUST_DRM_GEM_SHMEM_HELPER` is `select`-only, pulled in by
`DRM_VINO`. With vino off, `ops::Deref`, `FORMAT_MOD_LINEAR` and the crypto imports all look
unused. Fixes exist and are verified in *both* configurations, but the correct commit to fold each
into is **not** the one that creates the file — `crypto.rs`'s gating only becomes correct at
`rust: crypto: add synchronous RSA akcipher support`, which introduced the `#[cfg]` blocks. Two
attempts were mis-attributed; check with a build at the target commit before folding.

---

## Next, in priority order

### 1. The control session wedges, and it takes both panels down

The highest-value open bug. After a DPMS wake, a mode set times out, and a control write then
blocks **uninterruptibly and unboundedly**:

```
usb_start_wait_urb → kernel::usb::Io::bulk_send → VinoDrmData::probe_head_present
```

`usb_bulk_msg` honours its 1000 ms timeout, but on expiry it calls `usb_kill_urb`, and *that* wait
is unbounded. `ctrl_send` runs holding `cp_link`, so one wedged URB freezes every other path.
Observed three times; only `authorized` cycling or a power cycle clears it.

**Done**: a silence timer abandons the session after 5 s with no reply, and the keepalive then
drops the connectors and exits. ⚠ **The guards were initially placed after the send, where they
are unreachable** — the failure is a send that never returns. They are now before the transfer.
**Unvalidated**, and hard to provoke deliberately.

⚠ Measure silence, never unanswered messages: a healthy lit session logs **zero** unanswered
messages, but a session with one dark head logs ~75 per 100 s while its sibling drives a lit
panel. Counting messages tears down working sessions.

**Still open**: a session abandoned this way stays down until a replug. Re-establishing one against
a dock that has gone quiet needs its own design.

### 2. Wake-from-DPMS is fragile

Every wedge was preceded by `dual-head activation failed (ETIMEDOUT)`, sometimes twice, sometimes
followed by `ENODEV`. Panels do usually come back, but slowly and sometimes one at a time. This is
the root cause behind both the wedge and "only one screen came back on"; the silence timer only
catches the terminal case.

⚠ **Heads are not independent** and cannot be made so: reconfiguring one connector while another
is lit re-enumerates the dock ~100 ms later, measured five ways. `dock_wide_modeset` already folds
lit siblings into one transaction, which is why a late head works at all — it costs the sibling a
blink. `HOTPLUG_COALESCE` (1500 ms) only avoids *duplicate* re-activations; observed gaps are
718 ms and 3.11 s, so it covers one and not the other. Widening it trades latency for blinks;
don't widen it before deciding that trade deliberately.

### 3. Device support that survives new hardware

Written up in full in **`docs/adding-a-device.md`**. The enabler is already in the tree:
`firmware::read_identity` walks the plain configuration descriptor — no session, no crypto — so
hardware can be identified at probe.

In dependency order:

1. Match on vendor `17e9` + `bInterfaceClass 0xff` + `bInterfaceProtocol 0x03` instead of two
   product IDs. `DeviceId::from_interface_info` already exists. (`0x03` = DL3, `0x00` = udl, so
   udl hardware is excluded for free.)
2. Read the identity at probe; if `Family::from_identity` returns `None`, log
   `unrecognised device <tag>` and decline without registering DRM. This is the safety valve that
   makes step 1 acceptable.
3. Select the profile from the family; demote the product-ID table to quirks.
4. Probe head count instead of tabling it.

⚠ Only `NavaDock` has been read off real hardware; the other three family spellings are unverified
vendor names, so step 2's `None` path will fire more than expected at first.

### 4. PR #1 — do not merge as-is

`https://github.com/FireBurn/Vino/pull/1`, "vino-driver: support the ThinkPad USB 3.0 Pro Dock".

**It is `CONFLICTING` and cannot be merged.** More importantly it adds

```rust
fn head_count(product_id: u16) -> usize {
    match product_id { … _ => unreachable!("unsupported product passed device filter") }
}
```

and the same shape for endpoints — a product-ID capability table that **panics** on unknown
hardware. That is the pattern item 3 exists to remove.

The finding is real and valuable: the fixed `HEADS` array is wrong on a dock with fewer heads, in
chimera *and* in vino, and they found it on hardware we do not own. Take the intent, source the
number from the device, and reply rather than merge.

### 5. Package naming and size

CI **works** — run `31306653153` succeeded, release `7.2.0-rc2-vino-20260809`, no `+` suffix. Two
defects, fixed in the workflow but **unpushed**:

- the rpm was **1.1 GB** — debug info was still on; the deb hid it in a `-dbg` package the
  artifact filter dropped, so only the rpm exposed it. `--set-val CONFIG_DEBUG_INFO_NONE y` did not
  take (choice symbol); now `--enable DEBUG_INFO_NONE` plus a hard assertion that
  `CONFIG_DEBUG_INFO=y` is absent.
- the deb version named a commit — `7.2.0.rc2-01600-g2ad8d256721e-2`, the pre-fold one — and the
  formats disagreed on revision. `KDEB_PKGVERSION` now uses the kernel release.

⚠ `fetch-depth: 0` clones the whole Linux history on every run, for one line of provenance in the
release notes. Worth replacing with a shallow checkout plus the submodule SHA.

### 6. Smaller, open

- **Cursor stutter under load** — cursor movement is control messages on the KMS worker, which was
  at default priority. `Queue::new_ordered().highpri()` applied (the `alloc_workqueue` binding
  already exists — an older note claiming otherwise is wrong). **Untested under load.**
- **The three `rust/kernel` warnings** — real only when vino is disabled. Fixes known and verified
  both ways; placement is the hard part, see the traps section.
- **`NAVARRO_REAL_MODE_H0_MS = 2978`** is 53% of a clean bring-up (5.59 s enumerate-to-pixels).
  DLM's own prelude finishes at 2016 ms, so ~960 ms is dead air. One-line experiment, own commit.
- **Chimera vs vino parity** — chimera has cursor and damage; HDR, gamma and firmware are vino-only.
  A keyword count, not an audit, and revdi/chimera split responsibilities differently.

---

## Reference: what a clean bring-up looks like

From a dock power cycle, user-confirmed both panels lit:

```
t+0.00  new SuperSpeed Plus
t+0.06  bound DisplayLink control interface
t+1.38  4/4 head(s) authenticated
t+2.16  socket 1 edid:yes, socket 3 edid:yes -- both connected, ONE hotplug, session ready
t+5.44  head 0 first video
t+5.59  head 2 first video
```

Both EDIDs answered inside the setup transcript (`deferred:no`), so the topology reached the
compositor complete and at once. That is the condition that avoids a dock-wide reset.

⚠ `head=N endpoint=0x08 stopped accepting video` fires on **healthy** bring-ups too. It is
one-shot-latched back-pressure, not a fault. It means something only when no `video submit took`
line follows it.
