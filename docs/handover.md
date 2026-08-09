# Handover

Single current handover. Last updated **2026-08-09, evening**. Everything below is either still
true, or a trap worth not repeating. Anything an earlier handover said that is not repeated here
was done, superseded, or retracted.

**Read "⛔ Verification traps" first.** Three real defects got through because the build was
reporting success while building nothing.

---

## Immediate physical state

- **The dock is up and both panels are live.** It re-enumerated at 18:17 and came up clean:
  `4/4 head(s) authenticated`, sockets 1 and 3 `edid:yes deferred:no`, session ready. Both
  connectors are `connected` at 2560x1440@165 with HDR enabled.
- ⚠ It went through the DPMS-wake failure below at 19:23 and was repaired at 19:27, which cost a
  dock reset. The session running now is that post-repair one.
- The dock moves between USB buses; resolve it by `idProduct` (`7000`/`6006`), never by a path.
- vino autoloads (the blacklist was removed), DLM stays masked.
- ⚠ **The loaded module predates today's work.** Nothing below marked "landed" is in the running
  `vino.ko` until it is rebuilt and installed.

## Repository state

⛔ **Nothing is pushed.** Publishing means: back up the previous remote tip to a branch, push that,
then `git push --force-with-lease` `vino`, **then** the superproject. Never the superproject first.

Safety tags: `pre-fold-backup`, `pre-warnfix-backup`.

The kernel tree is committed and clean, builds warning-clean and rustfmt-clean at the tip with a
real config. `revdi` builds, `cargo test --workspace` is 23/23, and `chimera-prove` still reports
**192/192 OUT CP frames byte-identical to DLM**.

`patches/kernel/` is re-exported and current (114 patches), and `tools/validate.sh`
passes end to end -- the first clean run, see the last commit for what that took.

---

## ⛔ Verification traps

**A green build can mean nothing was built.** `make modules` re-runs `syncconfig` when the tree's
Kconfig differs from the one `.config` was generated against. With stdin at `/dev/null` it answers
*default* to every new symbol, silently dropping `CONFIG_DRM_VINO` at any commit where the driver's
Kconfig has not landed. `make` then exits 0 having compiled nothing. This hid three real defects.

**Always**: restore a known-good `.config` before each build and *assert* `CONFIG_DRM_VINO=m`
survived. `scratchpad/verify.sh` does this and refuses to build otherwise; `scratchpad/config-known-good`
is the config. ⚠ At a pure `rust/kernel` commit the refusal is *correct* -- the driver's Kconfig
genuinely is not there yet. Build those with `make LLVM=1 rust/kernel.o` instead.

**`make M=drivers/gpu/drm/vino` never rebuilds `rust/kernel`.** It is a fine fast syntax check and
it will happily tell you a new binding does not exist. Full `make LLVM=1 -j16 modules` before
believing any warning count, and before believing a binding is missing.

**Warnings are config-dependent.** `RUST_DRM_GEM_SHMEM_HELPER` is `select`-only, pulled in by
`DRM_VINO`. With vino off, `ops::Deref`, `FORMAT_MOD_LINEAR` and the crypto imports all look
unused. Fixes are verified in *both* configurations, but the correct commit to fold each into is
**not** the one that creates the file. Check with a build at the target commit before folding.

**Vendored copies drift silently.** `make check-sync` in `revdi/` compares `chimera/vino/*.rs` and
`module/*.rs` against the kernel tree, and now also checks that `vino/color.rs` and `evdi/color.rs`
stay identical. Today's sync pulled in two changes chimera had been running without: the per-head
selector bitmask and the 10-bit codec depth. Run it before trusting a chimera result.

---

## Next, in priority order

### 1. ★ Scanout stops silently on a DPMS wake, and one panel then looks fine

**The highest-value open bug, and the most misleading.** Measured end to end today; full capture in
`captures/dpms-wake-onehead-20260809-192651/`.

On a wake the driver issued a mode set for each lit head and nothing failed: no `ETIMEDOUT`, no USB
error, **every URB on EP02 and EP08 status 0**. Both CRTCs were `enable=1 active=1` with real
framebuffers, both connectors `connected` with 37 modes. And EP08 -- which carries *both* lit heads
on this dock -- moved **1792 bytes in 8 seconds**.

Scanout had simply stopped, for both heads. What that looks like from the chair is one dark panel
and one working one, which is why it has been reported that way twice. It is not what is happening:

- the "working" panel is **frozen on the last frame the dock still holds**;
- the dark one never received a first frame after the wake;
- the pointer keeps moving on the frozen panel because the hardware cursor is *control* traffic on
  EP02, not video -- so the only thing still updating is the pointer, and it looks jerky.

⚠ **This corrects the previous handover**, which said every wedge was preceded by
`dual-head activation failed (ETIMEDOUT)`. This instance had no error of any kind. There are two
distinct DPMS-wake failures and only the loud one was known.

⚠ It is also **not** what the silence watchdog in item 2 fixes, and should not be expected to be:
the dock was answering normally throughout, so no silence deadline can trip. The control session
was healthy; the *scanout* was not.

**Where to look.** The wake path re-programmes the mode but nothing re-arms the per-head scanout
worker, so `pending_scanout` is never drained. `run_scanout_worker` returns when
`modeset_requested[head] == 0` or the slot is empty, and a lost enqueue in that window has bitten
this driver before -- see the stranded-`dirty_ttl` and settle-repaint arcs. Start by instrumenting
whether `enqueue_scanout` is called at all after `atomic_enable` on a wake.

**Repair, for now:** `kscreen-doctor output.DP-4.disable` then `.enable`. Measured: EP08 went from
1792 bytes/8 s to 35 MB/6 s, then 226 MB/4 s. ⚠ It costs a dock-wide reset (reproduced again here),
and on the fresh session socket 3 came up `edid:no deferred:yes` and was recovered 13 s later by
the keepalive's re-engage retry.

### 2. The control session wedges, and it takes both panels down

Different failure from item 1: there the dock keeps talking and the pixels stop, here the dock
stops talking entirely. After a DPMS wake a mode set times out and a control write then blocks
**uninterruptibly and unboundedly**:

```
usb_start_wait_urb → kernel::usb::Io::bulk_send → VinoDrmData::probe_head_present
```

`usb_bulk_msg` honours its 1000 ms timeout, but on expiry it calls `usb_kill_urb`, and *that* wait
is unbounded. Observed three times; only `authorized` cycling or a power cycle cleared it.

**Landed today, HW-unvalidated:**

- The silence deadline is no longer polled at transaction entry. It was, which is why a wedge was
  measured reporting **15787 ms against a 5000 ms limit** -- the only thread that would have
  noticed is the keepalive, and the keepalive is the thread that gets stuck. There is now a
  1 Hz watchdog on the *system* workqueue that reads the deadline lock-free and abandons the
  session independently of anyone calling in.
- `last_reply` and liveness moved out of `cp_link` into a spinlock and an atomic, so
  `cp_link_alive()` no longer blocks on the mutex the wedged thread is holding -- which it did,
  meaning the detector deadlocked on the thing it was detecting.
- Best-effort paths (`drain_cp_pushes`) use `try_lock` instead of queueing behind a wedged transfer.
- An unprompted push now counts as the dock answering, and the deadline is held off entirely during
  Navarro's deliberate setup-to-first-mode-set quiet window -- which is 5000 ms, exactly the
  deadline, so without that a healthy cold bring-up would abandon its own session at the boundary.
- **Recovery without a replug**: after abandoning, vino asks the USB core to reset the dock
  (`usb_queue_reset_device`, new binding). That re-enumerates and re-probes -- what cycling
  `authorized` by hand does -- and it is the one recovery a stuck transfer cannot block, because
  the core runs it from its own work item. One attempt per device, so a reset that does not help
  cannot become a loop.

⚠ All of it is hard to provoke deliberately and none of it has been seen to fire on hardware.

### 3. Device support that survives new hardware — ✅ landed

`drm/vino: bind the display function, and place docks by family`, plus a new
`rust: usb: add a vendor-and-interface-info device id constructor` binding.

The chain is now **interface match → identity → family → profile → connectors**:

1. The USB table matches vendor `17e9` + interface class `0xff`/subclass `0`/protocol `0x03`, and
   separately the DFU interface `0xfe`/`1`/`1`. No product IDs. `modinfo vino` shows exactly two
   modaliases. Protocol `0x00` excludes `udl` for free.
2. `firmware::read_identity` runs at probe on either interface.
3. `profile::for_family` maps the family to a profile; unknown families are **declined by name**.
4. `Endpoints::resolve` returns how many of the profile's video endpoints the device exposes, and
   the connector count follows that, bounded by the profile.
5. A device whose identity cannot be *read* falls back to `profile::for_product`, now a quirk table.

`PROFILE_D6000`/`PROFILE_DL7400` are renamed `PROFILE_RIDGE`/`PROFILE_NAVARRO` to match. Written up
in `docs/adding-a-device.md` and `Documentation/gpu/vino.rst`.

⚠ **Untested on the D6000**, which was not plugged in. The behaviour change to watch: interfaces
2-6 are no longer offered to vino at all. Probe already declined them, so this should be inert, but
the D6000's interface layout has never been dumped here -- get `lsusb -v` on one.

⚠ Only `NavaDock` and `RidgeDoc` map to a profile. `EllaDock`/`FflyMoni` are recognised names that
take the decline path.

### 4. PR #1 — reply drafted, not sent

`https://github.com/FireBurn/Vino/pull/1`, "vino-driver: support the ThinkPad USB 3.0 Pro Dock"
(`17e9:433f`). Still `CONFLICTING`.

**Three fixes from it are already applied**, because they were wrong for every dock, not just
theirs:

- **short writes were silently successful** on both the sync and async paths -- `write_bulk`
  returns `Ok(n)` with `n < len` and nobody checked, so a truncated video record just went out.
  There is now a `ShortWrite { wrote, wanted }` error.
- **`write_video` did not take `out_lock`**, unlike every other host→dock write.
- **the 500 ms cancellation deadlines were a use-after-free**: `shared` and `completions` are
  locals the libusb callback writes through, and giving up and calling `libusb_free_transfer`
  anyway means the callback lands on freed stack. Both waits are unbounded now, with the reasoning
  in a comment.

**Not taken**: `head_count(product_id)` and `video_endpoints(product_id)` with `unreachable!()`
arms. That is the pattern item 3 removes, and their dock now gets one head because it exposes one
video endpoint.

**What the reply asks for**: the `collect-report.sh` output, so we learn what `433f` reports as its
family -- if it is `RidgeDoc` it already works, and if not it is one line in `profile::for_family`,
but it needs the real string. Plus evidence for the shared-EP02 claim and where the 64 KiB /
depth-3 numbers were measured. Draft is ready to send; it has not been posted.

### 5. Package naming and size — ✅ landed, unpushed

CI works (run `31306653153`, release `7.2.0-rc2-vino-20260809`). The rpm-size and deb-version
defects were fixed earlier. Today: `fetch-depth: 0` → `1`, and release provenance now comes from
the submodule SHA (`git rev-parse --short=12`) rather than `git describe`, which was the only thing
needing twenty years of Linux history on every run.

### 6. Bring-up dead air — ✅ landed, HW-unvalidated

`NAVARRO_REAL_MODE_H0_MS` was 2978, a copied capture offset, while DLM's own prelude finishes at
2016 ms. It is now derived: `NAVARRO_COLD_PRELUDE_END_MS + 20`, so the two cannot drift apart.
Expect roughly **940 ms off plug-to-pixels** (it was 53% of a 5.59 s bring-up). Pinned by a KUnit
test. Needs one cold plug to confirm the dock does not want that second.

### 7. revdi / chimera parity — ✅ largely landed

- `vino-driver::profile` mirrors the kernel's identification exactly: same identity descriptor,
  same families, same profiles, same quirk fallback. `Dock::open` matches the display function and
  places the dock by family.
- `HEADS = 2` is gone from `daemon.rs` and `session.rs`; arrays are `MAX_HEADS` (4) and every loop
  is bounded by `dock.connectors()`.
- The video API is head-indexed; `write_video2*` and the hard-coded `0x0b` are gone.
- Vendored sources resynced, which is where the per-head bitmask and 10-bit depth came from.
- `make check-sync` now also guards `vino/color.rs` vs `evdi/color.rs`.

**The real remaining gap is Navarro cold activation.** Its prelude, dock-wide mode transaction and
first-video choreography live in `drm_sink.rs`, which is DRM-specific and cannot be vendored the
way `cp.rs` and `video.rs` are. Chimera speaks the Navarro protocol but cannot bring a DL-7400 up
from cold. Closing it means extracting the choreography into a vendorable module (better) or
reimplementing it in `chimera/src/session.rs`. Not small. Table of what else differs is in
`docs/revdi-chimera.md`.

### 8. Smaller, open

- **Cursor stutter under load** — `kms_queue` is `.highpri()`. Still untested under load, and note
  that the jerkiness reported today was item 1, not this.
- **The three `rust/kernel` warnings** — real only when vino is disabled; placement is the hard part.
- **`2560x1440@165 has no decrypted DLM profile`** fires twice on every mode set and the inferred
  `sync_flags=0x0600 vic_word=0x0800` has never been checked against a capture. It works, so the
  inference is at least not fatal, but it is guesswork on the mode this dock actually runs at.

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

⚠ **Sockets 1 and 3 share video endpoint 0x08** (connectors 0 and 2; `0x0a` carries 1 and 3). Zero
bytes on `0x0a` with two monitors attached is normal, not a fault. It also means EP08 byte counts
cannot separate the two lit heads -- decode the record `sub` field for that.

⚠ **Bytes on the wire are not a lit panel, and neither is a picture.** A panel showing a stale
frame is indistinguishable from a working one until something moves. Ask, and ask about *motion*.
