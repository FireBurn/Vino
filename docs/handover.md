# Handover

All six series were posted on 2026-08-26. This file is the review response: what came back, what
has been answered in the tree already, and what is still owed. The previous contents of this file
(the DL7400/Ridge/Ella capture history, the traps, the capture recipe) are at `3d4e7b4` and are
recoverable with `git show 3d4e7b4:docs/handover.md`.

## What was posted

| Series | Subject prefix | Patches | Lore id |
|--------|----------------|---------|---------|
| rust-core | `[PATCH n/9]` -- **no `v3` in the subject** | 9 | `20260826162851.2497-1-mike@fireburn.co.uk` |
| rust-crypto | `[PATCH v3 n/2]` | 2 | `20260826163004.3365-1-mike@fireburn.co.uk` |
| rust-usb | `[PATCH v3 n/5]` | 5 | posted 16:30 to linux-usb |
| rust-drm | `[PATCH v3 n/23]` | 23 | posted 16:31 to dri-devel |
| rust-firmware | `[PATCH 1/1]` -- **no `v3`** | 1 | `20260826163716.6274-2-mike@fireburn.co.uk` |
| drm-vino | `[PATCH v3 n/13]` | 13 | `20260826163913.7052-1-mike@fireburn.co.uk` |

⚠ Two of the six went out without `v3` in the subject, which is why they do not turn up in a
`subject:"PATCH v3"` search. Fix the prefix for the next posting.

Human replies came from Danilo Krummrich (usb), Eric Biggers (crypto), Miguel Ojeda (crypto, core)
and Andreas Hindborg (core). ⛔ **Nobody has reviewed rust-drm (23 patches) or the vino driver
itself.** The only replies on those two are from sashiko-bot.

## ✅ Folded into the series (2026-08-31)

The six review fixes are no longer working-tree changes: they are folded into the commits that
introduced the lines, the series was rebased with `--autosquash`, and every touched commit builds
on its own. Backup of the pre-fold tip: branch `vino-backup-20260831-180901` (`7497b498d01b`).

| Change | Folded into |
|--------|-------------|
| `crypto`: safe `zeroize()`, `Aes128::drop` uses it, `rust_helper_memzero_explicit` moved here, `#[inline]` on the forwarders, two em-dashes | 1/2 `rust: crypto: add AES-128...` |
| `crypto`: `Secret::drop` uses `zeroize()`; `rust_helper_aes_enckey_zero` deleted; no longer rewrites `Aes128::drop` | 2/2 `rust: crypto: add synchronous RSA...` |
| `firmware`: `write()` clamps the reported count; `cleanup` doc corrected | `rust: firmware: add the firmware upload abstraction` |
| `vino`: single-axis reflection no longer treated as identity | `drm/vino: add the dock activation and scanout path` + `... the KMS device and the atomic path` |
| `vino`: `Kconfig` gains `depends on CRYPTO` / `depends on MMU` | `drm/vino: allow the driver to be built` |

⚠ The crypto fix did **not** split cleanly along the original patch boundary, and the reason is
worth keeping: patch 2/2 was introducing `Secret`, `rust_helper_memzero_explicit`, the
`CONFIG_RUST_CRYPTO_LIB_*` cfg gates **and** a rewrite of `Aes128::drop` -- all infrastructure that
1/2 already needed. Moving `zeroize()` and the memzero forwarder into 1/2 means 2/2 no longer
touches `Aes128::drop` at all and adds no second helper. That is a smaller 2/2 to rewrite for
Eric's RSA-only ask, but it is **not** the Kconfig fix: `RUST_CRYPTO_LIB_AES` and
`RUST_CRYPTO_LIB_SHA256` are still defined in 2/2 and consumed by 1/2. That move is still owed.

⛔ Do **not** "fix" the em-dash at `rust/kernel/usb.rs:2100`: that line is Colin Braun's URB patch,
carried unchanged as a prerequisite.

## Replies sent 2026-08-31

Five replies went out: hrtimer 7/9 (keep the context type), hrtimer 2/9 (drop `restart`), usb 1/5
(the I/O window and `release_driver`), crypto 1/2 (`aes_ctr()`), crypto 2/2 (RSA + Kconfig). Text in
`outgoing/v3-replies/`.

⚠ **The first three went out as HTML and lore rejects those**, so they are not in the archives --
Andreas said so on the 2/9 thread. Gmail needs "Plain text mode" turned on in the compose window.
Resending those three in plain text is still owed.

Commitments made on-list, which v4 has to honour:

- **Drop** `ArcHrTimerHandle::restart` (Andreas) and `Interface::release_driver()` (Danilo).
- **Move** the two Kconfig hunks from 2/2 into 1/2 (Eric).
- **Rewrite** 2/2 as an RSA-only API, no generic akcipher (Eric).
- Cut the bare AES from 1/2 in favour of `aes_ctr()`, leaving the HDCP dKey single-block encrypt as
  the one open question put back to Eric.

## ✅ Done in the tree 2026-08-31 (second pass)

All folded into the commit that introduced the line, series rebased, tree clean, full kernel build
green (`LD vmlinux`, `LD [M] vino.ko`), rustfmt clean, checkpatch 0 errors.

| Change | Folded into | Answers |
|--------|-------------|---------|
| `restart()` changelog now names the real caller: `enable_vblank()` re-arms a **stopped** timer from outside the callback, under the vblank locks with interrupts disabled | `rust: hrtimer: add ArcHrTimerHandle::restart` | Andreas |
| USB transport changelog no longer claims the wrapper opens an interrupt endpoint -- `Endpoints` is bulk only; the dock's interrupt-IN endpoint is what the endpoint typing protects against mis-aiming at | `drm/vino: add the USB transport` | sashiko 2/13 |
| `akcipher` module renamed `rsa`; `RUST_CRYPTO_AKCIPHER` -> `RUST_CRYPTO_RSA` (now also selects `CRYPTO_RSA`); `encrypt()` made private; changelog states the binding deliberately does not wrap `crypto_akcipher` | `rust: crypto: add RSA public-key encryption` + 4 vino commits | ⭐ Eric |
| Automatic flash refuses a non-package image and one for another dock family, the same checks the userspace upload path makes; `update_pending()` mirrors them | `drm/vino: read the dock's firmware version...` | ⭐ sashiko (brick risk) |
| `UPDATE_ATTEMPTS` is 4 per-dock slots claimed by compare-exchange, not one global load-then-store slot two docks evict each other from | same | sashiko |
| Navarro parameter map emits as many records as the bands need instead of overrunning the 4080-byte ceiling above 1920 lines; KUnit test pins 1440p at 2 records and 2160p at 3 | `drm/vino: add the video codec` | sashiko |
| Three test-only helpers no longer warn in a normal build (`is_metered`, `FrameTrailer::none` gated; the redundant `ella_rows_1080p` accessor deleted) | `drm/vino: add the dock profiles`, `... the video codec` | build warnings |

⚠ **`4K60 is reachable on Navarro`**, so the parameter-map overrun was a live bug, not theoretical:
594 MHz pixel clock against `max_connector_clock_khz: 699_500`, 497 Mpx/s against a
`pixel_budget` of 1,216,512,000.

⛔ **`ENCODE_WQ` is still leaked and still needs a decision.** The static `SetOnce<OwnedQueue>` is
never dropped, so every module load leaks a workqueue and its threads. ⭐ Measured fact that unblocks
it: `encode_across_cpus` **joins every chunk before returning**, so the queue does not need to
outlive the module and module-owned ownership would be sound. The two candidate fixes differ in
behaviour -- a per-device queue gives two docks two queues each at `max_active = nr_cpus`, and that
fan-out is the load-bearing ~7.4x measurement -- so this wants measuring, not guessing.

## ✅ The base bump -- DONE 2026-08-31

The series now sits on `drm-rust/drm-rust-next` = `c63829528980` (`Merge tag 'v7.3-rc1'`), 17,766
commits on from the old base. **98 commits on top**: 54 Mike, 37 Lyude, 3 Colin Braun, 3 Alice
Ryhl, 1 Onur Ozkan. Full build green (`LD vmlinux`, `LD [M] vino.ko`), rustfmt clean, zero warnings
of ours. Pre-bump tip is on `vino-pre-v73-0831-2003`.

**19 carried prerequisites dropped** because the new base already has them -- the whole
`SpinLockIrq` / interrupt-module / preempt stack from Lyude, Boqun, Joel and Heiko. Two of ours went
too: `rust: error: expose EPROTO` (declared upstream) and `x86/boot: Disable jump tables in the
decompressor`, which git dropped as an empty patch because v7.3-rc1 carries `-fno-jump-tables` at
`arch/x86/boot/compressed/Makefile:30` already. ⭐ The clang-23 decompressor fix is therefore still
in effect; nothing was lost.

⛔⛔ **Two things the earlier scouting got WRONG, corrected here:**
- Colin Braun's two USB patches are **NOT** upstream -- `Urb` and `TransferFlags` exist nowhere in
  v7.3-rc1. They were auto-skipped by a loop that skipped any non-Mike commit that merely
  conflicted. They are carried, and must stay carried.
- A drop list built by **subject match alone is both too strict and too loose**. Three commits
  (`__preempt_count_{sub,add}_return`, counted interrupt disable, `spinlock.rs` `super::*`) had no
  subject match yet were present by content. Verify by content -- grep the base for what the commit
  provides -- never by subject, and never by "it conflicted so skip it".

**API drift v7.3-rc1 forced, folded into the commits that own each file:**

| Change | Why |
|--------|-----|
| `usb_device_table!` loses its `MODULE_USB_TABLE` argument | upstream macro now takes three args |
| `THIS_MODULE` -> `<LocalModule as kernel::ModuleMetadata>::THIS_MODULE` | it is an associated const now |
| `Driver::probe` takes `Option<&'bound Self::IdInfo>` | upstream derives id info via `info_unchecked_opt` |
| `module_parameters::X.value()` no longer dereferenced | `value()` returns `T` by value, `value_ref()` returns the reference |
| `#include "drm/drm.c"` keeps upstream's `CONFIG_DMA_SHARED_BUFFER` guard | Lyude's helpers split vs upstream's new ifdef |

⭐ **A latent defect the newer kernel exposed**: `VinoDrmData` carried three orphaned doc-comment +
`#[pin]` pairs whose fields had been deleted at some point. The old `#[pin_data]` macro tolerated
stacked `#[pin]` attributes; v7.3-rc1's rejects them with "attribute specified more than once".
Removed -- nothing referenced the missing fields.

## Third-party series we carry -- checked 2026-09-01

Answering "has anything we carry been updated or superseded". Checked against the new base by
content, and against the authors' own trees and the list.

| Series | Ours | Newest found | Verdict |
|--------|------|--------------|---------|
| Lyude Paul, Rust KMS (37) | authored 2025-06-19 .. 2025-10-29 | `lyudess/linux` `rvkms-slim`, 2025-11-13 | **not superseded** |
| Colin Braun, USB (3) | 2026-07-12 (v1) | v1 is still the only posting | **latest**, but has open review |
| Alice Ryhl, workqueue (3) | 2026-03-12 | not upstream | still needed |
| Onur Ozkan, `cancel_sync` (1) | 2026-06-17 | not upstream | still needed |
| Andreas Hindborg, `expires-v2` | not carried | not in the base | still in flight |

- ⭐ **Lyude**: `rvkms-slim` contains all 37 of our subjects, so nothing was dropped or renamed under
  us. Its extra 52 commits are RVKMS itself, gem-shmem work, unrelated `kernel::fmt` cleanups, and
  the preempt/irq stack that is now upstream anyway. The KMS files do differ, but the divergence
  runs **our** way: ~1250 lines only-in-ours against ~66 only-in-hers, and hers are mostly doc
  rewording. We have extended her bindings substantially (`ColorLut`, `ColorCtm`, `CrtcRef`,
  `enable_gamma`, `enable_color_mgmt`, the LUT accessors) -- 7 commits on `crtc.rs` alone. Adopting
  `rvkms-slim` would mean re-applying all of that for one lifetime-bound change on `Crtc::new`.
  ⛔ Not worth doing, and definitely not before a hardware test.
  ⚠ Two commits we ship are still titled `WIP:` (`WIP: Add very basic bindings for modes`,
  `WIP: drm/modes: Fix arg types in drm_set_preferred_mode`). They are `WIP:` in her tree too, but
  shipping a patch called WIP in a posting invites the obvious objection -- retitle or get her to.
- ⚠ **Colin Braun**: the 2026-07-12 v1 is still the only posting, seven weeks on, and Danilo
  Krummrich, Daniel Almeida and Oliver Neukum all replied to it on 13-14 July. We carry the latest
  revision, but it is a revision with unanswered review feedback, and rust-usb depends on it. If he
  respins, our USB series moves under us.
- **Andreas's `expires-v2`**: not in the base -- `expires_unchecked` is absent from both. Our two
  hrtimer patches are the only divergence in `rust/kernel/time/hrtimer.rs` (29 lines), so nothing
  clashes yet and the question in the correction draft is still the right one to ask.

## v4 patches -- respun 2026-08-31

⚠ `outgoing/` is in `.gitignore`, so `outgoing/v3/` was **never tracked** and its deletion is not
recoverable with git. It is regenerable from the pre-bump branch `vino-pre-v73-0831-2003` with
`git format-patch --subject-prefix="PATCH v3"` over the same ranges. `outgoing/v4/` holds seven
postings generated with
`--subject-prefix="PATCH v4"`; all 97 series patches apply in order onto the base, verified.

| Posting | Patches |
|---------|---------|
| `00-sched` | 1 (unrelated `sched/fair` fix, post separately) |
| `rust-core` | 12 |
| `rust-crypto` | 2 |
| `rust-usb` | 8 |
| `rust-drm` | 61 |
| `rust-firmware` | 1 |
| `drm-vino` | 13 |

⚠ `RECIPIENTS.txt` was carried over for four of them; `drm-vino` and `rust-firmware` never had one.
⚠ Cover letters are the generated stubs -- **the rust-usb cover still needs the corrected paragraph
below**, and every cover needs its blurb written before sending.

## rust-usb v3 -- blocked on a reply, not on code

Danilo replied twice, and Cc'd Greg, Oliver and Alan on the cover. **Answer the thread before
touching the code**: his questions decide the shape.

1. ⛔ **"This seems to reinvent Devres, which we superseded with Rust native lifetimes and
   higher-ranked types. Please use that instead."** He also asks "How is this different or narrower
   than the device's `Bound` type state represents?" There is a real answer -- suspend and
   `pre_reset` close the window while the interface stays bound, so the window *is* narrower than
   `Bound` -- but it has to be made on-list. Either rebuild `IoWindow` on the existing revocable
   infrastructure or defend the mutex+condvar window with that argument.
2. ✅ **DONE -- `Interface::release_driver()` dropped** (no caller anywhere in the tree). It had no
   caller in this posting. Drop it, or, if vino's sysfs `remove_all` needs it, put it behind a
   scope type that cannot leak into `Interface<Core>` (he sketched exactly that; the deref chain
   makes the naive version impossible to constrain).
3. ⛔ **The cover letter's central claim is wrong -- replacement text below.** v3 said "Alan Stern
   confirmed that a bound interface implies a configured device". Danilo re-quoted Alan: a user can
   write `bConfigurationValue` at any time, so that is not an invariant.

   ⭐ The real guarantee is the one worth stating, and it is stronger: changing the configuration
   **destroys the old interfaces, which unbinds their drivers first**. A driver therefore cannot be
   holding `Interface<Bound>` while the device is unconfigured -- not because the configuration
   cannot change, but because the unbind happens before it does. Use for v4:

   > Since v2, the patch that kept `usb::Device` private is dropped. Oliver Neukum's objection was
   > that USB genuinely does device-level operations and hiding them behind an interface is a
   > layering violation. The v3 cover claimed a bound interface implies a configured device; that
   > was wrong, as Alan Stern pointed out -- `bConfigurationValue` is writable at any time. What
   > the abstraction actually relies on is narrower: changing the configuration destroys the old
   > interfaces and unbinds their drivers, so no driver holds a bound interface across it.
   > Danilo Krummrich's related point about gating I/O on the driver lifecycle is addressed by
   > keeping the I/O helpers on `usb::Interface<Bound>`; the unsafe `as_bound()` he objected to is
   > gone, as is `reset_configuration()`, and `set_interface()` is split so an interface sets its
   > own altsetting and the device sets any other.
4. He thinks dropping v2 10/11 over-corrected: `intf.bulk_send()` is fine, and if the USB topology
   reading matters, add a borrowed `IoDevice<'a>` newtype rather than concealing the device.

## rust-crypto v3 -- Eric wants 2/2 rewritten

1. ✅ **DONE 2026-08-31 -- "If you need RSA, then please just create an API for RSA specifically. The
   `crypto_akcipher` abstraction has never worked well."** Patch 2/2 is a generic akcipher wrapper.
   It needs to become an RSA-specific API.
2. ✅ **`aes_ctr()` -- Eric answered on 2026-08-31, and the answer keeps `Aes128`.** He will not
   expose raw-key AES functions ("computing the AES round keys is fairly slow and most users use
   their AES keys multiple times"), and directs: `aes_prepareenckey()` + `aes_encrypt()` +
   `memzero_explicit()` for a single block, `aes_prepareenckey()` + `aes_ctr()` +
   `memzero_explicit()` for CTR, and ⭐ "if you're using either key multiple times you should call
   `aes_prepareenckey()` just once and cache the result, as that is what it is for."

   That is precisely what `Aes128` does, so the type stays. v4 work: add a CTR method on `Aes128`
   that calls `aes_ctr()` with the already-prepared key, and replace vino's two hand-rolled CTR
   loops with it. `encrypt_block` stays for the HDCP 2.2 dKey derivation.
   ⚠ **Blocked on a base bump**: `aes_ctr()` is not in `4c9ba407018e`. The same bump is the
   prerequisite for dropping the EPROTO patch, so do it once and clear both.
3. ✅ **DONE 2026-08-31 -- both symbols now live in 1/2 with the `#[cfg]` gates for its own code.**
   Was: `RUST_CRYPTO_LIB_AES` and `RUST_CRYPTO_LIB_SHA256` were added in `cbe74d3d59dc` (patch 2/2)
   but consumed by `c8a44968d623` (patch 1/2).** So 1/2 compiles with every `#[cfg]` block switched
   off -- a green build that builds nothing, exactly the trap in the memory note. Move the two
   Kconfig hunks back into 1/2. This is an interactive rebase across ~85 commits, which is why it
   was left rather than done blind.
4. Kconfig symbol in `lib/` with the code in `rust/`: Eric objects, **Miguel already answered him**
   (`CANiq72=uUR4Vo9W55Kq6tQjc+6Q4_+wiWhCo-VLdkvg6PJeoAw@mail.gmail.com`) and says symbols are not
   tied to paths and can move later. Reply pointing at that rather than reworking it.
5. `memzero_explicit`: Miguel confirmed Rust has no standard equivalent, so the helper stays. He is
   asking upstream Rust about it again. ✅ The follow-up he asked for -- one safe Rust function
   instead of a second helper -- is done.
6. ✅ **DONE 2026-08-31 -- 1/2 changelog corrected.** It was factually wrong: it says `Aes128` prepares the key schedule once and
   reuses it "for block encryption and CMAC", but `aes_cmac()` is standalone and re-expands the key
   every call. Fix the commit message.

## rust-core -- two patches to drop, one to rework, and a split

1. ⛔ **8/9 `rust: error: expose EPROTO` is already upstream** as `b93fb6e76ec1` ("rust: error: add
   remaining error codes"). ⚠ That commit is **not** in this tree -- the base is
   `4c9ba407018e`, the drm-rust-next tip of 2026-08-06 -- so dropping the patch breaks the build
   until the base moves. Rebase first, then drop.
2. ⚠ **PATCH RESTORED + changelog fixed; the on-list correction is drafted but NOT SENT.**
   2/9 `ArcHrTimerHandle::restart` -- the reply sent to Andreas is WRONG and needs a
   correction on-list.** It says "nothing anywhere calls the handle's restart()". It does:
   `drm_sink/mode_objects.rs:454`, `h.restart(Delta::from_nanos(interval))`. The earlier grep was
   `\.restart()`, which requires empty parens, and the real call takes a `Delta` -- so the search
   found nothing and the patch was dropped on that basis. Restoring it was forced by the build:
   `error[E0599]: no method named restart found for reference &ArcHrTimerHandle<VblankTimer>`.

   ⭐ The call site is the justification the reply should have carried. `enable_vblank` re-arms a
   timer that has already **stopped** -- `disable_vblank` lets it die by returning `NoRestart` -- so
   it re-arms from **outside** the callback, where `forward()` + `HrTimerRestart::Restart` cannot
   reach. It runs under the vblank locks with interrupts disabled, and the safe alternative (drop
   the handle, call `start()` again) cancels, which blocks until a running callback finishes.
   That is illegal there. The patch stays; Andreas needs that call site.

   **2026-08-31:** Andreas accepted the drop ("You are welcome :)") before the error was found, so
   the correction is owed on a thread where he has already agreed. Draft `r-5622368346489276394` is
   in Gmail, threaded, naming the call site and asking whether `expires-v2` changes the shape.
   ⚠ Not sent -- needs Plain text mode enabled in the Gmail compose window first.

3. **9/9 `ktime_get_real_seconds`** -- Andreas wants it expressed through the new `TimeUnit` concept
   as a seconds-based `Instant`, not a bare wrapper.
4. ⭐ **7/9 hard-callback IRQ state: "This looks good to me"** -- but he is proposing to *remove* the
   context type in the same series and says "we might have to keep it around for this to work".
   **Reply asking him to keep it.** Vino is the user that justifies it; this is the one place where
   being the consumer is leverage.
5. **Miguel on the cover: "these patches touch different subsystems ... some of them may want that
   you split things up accordingly."** hrtimer/time, workqueue, io, sync, random and xxhash all
   have different maintainers. Split rust-core per subsystem for the next posting.

## rust-firmware

✅ The `write()` bounds hole and the `cleanup` doc are fixed above. Still open:

- ⚠ **Pre-existing UAF, not introduced by this patch, but it lands on `Registration::drop`.**
  `firmware_upload_unregister()` skips `flush_work()` when `progress == FW_UPLOAD_PROG_IDLE`. A
  sysfs write racing the unregister can queue `fw_upload_main` just before `device_unregister`
  disables sysfs; `drop` then frees `U::Data` via `from_foreign()` and the queued worker touches it.
  Worth raising on-list as a separate C fix.

## drm-vino -- sashiko only; act on merit

Credible and cheap to check:

- ⭐ **`Bits::finish` pads the final byte with zero bits.** Zero is a valid symbol, so the decoder
  can read unintended coefficients off the tail of an unaligned strip; the vendor pads with 1-bits,
  which reads as a truncated all-ones escape and is ignored. Given the strip-desync history this is
  the first thing to measure against a vendor capture. ⛔ Verify against **vendor bytes**, not a
  round-trip through our own decoder.
- ✅ **DONE 2026-08-31 (with a KUnit test).** Navarro parameter map could exceed `STRIDE_CAP` at 4K. In `navarro_strip_params()` the second
  record takes `bands.div_ceil(PARAM_BANDS_PER_TLV)` TLVs; at 3840x2160 that is 270 bands, 15 TLVs
  in record 0 and 19 left, and 19 * 262 = 4978 > 4080. Pure arithmetic, checkable without hardware.
- ✅ **DONE 2026-08-31.** `update_if_newer()` did not call `package_family()`. The manual `Upload::prepare` path does.
  A wrong file in `/lib/firmware/vino/` would be flashed on enumeration and brick the dock. Left
  undone here only because it changes flashing behaviour on real hardware -- but it should be done.
- ✅ **DONE 2026-08-31 (4 per-dock slots, compare-exchange).** Was one global non-atomic slot. Two docks evict each
  other's counts, so `MAX_UPDATE_ATTEMPTS` never trips and neither breaks the reflash loop. Make it
  per-device, or a real compare-and-swap.
- ⛔ **STILL OPEN, needs a measurement not a guess (see the second-pass section).** `ENCODE_WQ` in a `SetOnce` static is never dropped, so the unbound workqueue leaks its
  threads on module unload. It is the last global in the driver.
- ✅ **DONE 2026-08-31.** 2/13's changelog claimed an interrupt endpoint `Endpoints` does not have.
- Import style (vertical) and `#[inline]` on forwarders were raised on vino 11/13 as well. Mechanical.

Needs judgement, because the bot does not know the threading model:

- ⚠ **The "sleeping in `atomic_update`" cluster** (8/13, 9/13, 11/13, raised many times):
  `GFP_KERNEL` allocations, `vmap`, sleeping `Mutex`, and a memcpy of up to 14.7 MB inside
  `atomic_update` / `atomic_enable` / `atomic_disable`. The snapshot-in-commit design is deliberate
  and hardware-proven, **but a DRM reviewer will raise exactly this**, so have the answer ready:
  which commits are non-blocking, and where the driver actually sleeps.
- ⭐ **`atomic_check` reads `last_timing` under a local lock instead of a `drm_private_obj`**, so two
  concurrent commits each see the other's old bandwidth and both pass. This is the same soft spot as
  the DPMS-wake depth split -- the rule from that hunt was **never record state in a check** -- and
  the bot is pointing at it from the other side. Fix it properly.
- Error paths that propagate without `retire_failed_video_queue()` (`send_stream_open`,
  `send_video_keepalive`, `submit_prompt_training`) would leave the endpoint wedged. Consistent with
  the Ella EPIPE history.
- `video_staging` held across synchronous `queue.send()`; `activate_dual_wake` returning `Ok(false)`
  on `sent.count_ones() < 2` without `unwind_bracket`/`programmed_timing` cleanup; `edid_target` and
  `edid_caught` mutated from two connectors' workers. Plausible, but check against the real model.

## Posting mechanics

- ⚠ **Mail bounced for two recipients on multiple series**: `nick.desaulniers+lkml@gmail.com`
  (Gmail spam-blocked, `550 5.7.1`) and `daniel.almeida@collabora.com` (`554 5.7.7 Email policy`).
  Both are real reviewers, so the fix is probably a different relay rather than dropping them from
  `outgoing/v3/*/RECIPIENTS.txt`. Decide before the next send. The `tor.source.kernel.org` bounces
  are the kernel.org forwarding of the sender address and are separate.
- **The cover letters say rust-drm and drm-vino are "not sent yet"** when they went out six minutes
  later. Cross-reference them properly next time.
- Miguel's v2 ask -- link the related series and say plainly that vino is the user for all of them
  -- was met in the usb cover. Keep it in every cover.
