# vino firmware DOWNGRADE on Navarro (DL-7400) — 2026-08-23

**✅ SUCCESS. vino flashed a DL-7400 from 12.2.26 down to 11.5.29 and the dock came back running it.
This is the first firmware flash vino has ever completed.**

It also settles the Ridge failure: the ~750 KB wall is **Ridge-specific**, not a vino bug.

## What was done

| | |
|---|---|
| Dock | DL-7400 quad dock, `17e9:7000`, `2-1.3`, identity `NavaDock` |
| Before | **12.2.26** — identity `10 40 0c 02 1a 0b 03 22` |
| Image | **11.5.29** (2024-10-17), 1,615,552 B, `sha256 3c48b3caa115…`, `RD`=`NavaDock`, 395 blocks |
| After | **11.5.29** — identity `10 40 0b 05 1d 0b 03 22`, `bcdDevice` 39.22 unchanged |
| Path | `/sys/class/firmware/vino-dock-2-1.3:1.1` (`update_if_newer` refuses downgrades by design) |

```
vino: dock firmware written; it will re-enumerate to run it
usb 2-1.3: USB disconnect, device number 56
usb 2-1.3: new SuperSpeed Plus Gen 2x1 USB device number 60
vino 2-1.3:1.1: Navarro dock running firmware 11.5.29
```

`bcdDevice` does not move across an update, exactly as `firmware.rs` says — the identity descriptor
is the only thing that reports the change. Byte 4 is the patch level: `0x17`=23, `0x1a`=26,
`0x1d`=29, consistent with DLM's own 11.5.23 -> 12.2.26 upgrade of this dock on 2026-08-01.

⚠ One kernel note on re-enumeration, present on the old firmware and not the new:
`Int endpoint with wBytesPerInterval of 6 in config 1 interface 2 altsetting 0 ep 0x83: setting to 7`.
DLM's upgrade capture shows the same field moving the other way (`maxp 0007` -> `0006`), so this is
a real firmware-version difference in the interrupt endpoint descriptor, not a vino artifact.

## Wire

⚠ The capture is **truncated at 376 of 395 blocks and misses the manifest** — the reader was killed
with SIGTERM, which bypassed its flush. A tooling artifact, not a transfer problem: dmesg confirms
the write completed and the dock re-enumerated on the new image. `monread.py` now traps SIGTERM and
flushes per record.

What the captured portion does establish:

- `DETACH=1`, then **376 image blocks, wValue monotonic 0..375, 1,540,096 bytes**, zero USB errors.
- Every `GETSTATUS`: `bStatus=0`, `bwPollTimeout=0`, `bState=5` (dfuDNLOAD_IDLE). **Always-idle, the
  same as Ridge** — and here it completes.
- **Unpaced: 0.142 ms/block**, 54 ms for 1.54 MB. Faster than vino managed on Ridge (0.214 ms) and
  15x faster than DLM's own Navarro flash (2.1 ms/block).

## What this settles

- ⭐ **vino's `firmware::flash()` works.** The transport, the guards, the block sequencing and the
  manifest all do their job end to end on real hardware.
- ⭐ **A downgrade is accepted.** No rollback protection on this platform; `update_if_newer` refusing
  downgrades is a vino policy, not a dock constraint.
- ⛔ **The Ridge ~750 KB wall is Ridge-specific.** Navarro took **twice** that (1.54 MB and counting)
  in one monotonic session, unpaced, at a *higher* block rate than the Ridge attempts that failed.
  Rate, block size, always-idle status and vino's loop are all exonerated.
- ⭐ **Speed is not a constraint here.** vino streams 15x faster than DLM and the dock is fine with it.

## Not tested

**Nothing is plugged into this dock** — all four connectors (`card2-DP-2..DP-5`) correctly report
`disconnected`, so display bring-up on the old firmware was not exercised. What was verified is
bind, identity, DRM registration on minor 2 with four connectors, the upload node re-registering,
and correct presence reporting. A real test needs a monitor on the DL-7400.

## Restore

`navarro-12.2.26-restore.spkg` (`sha256 3dd76fadb35c…`) is staged here. Restore with:

```
N=/sys/class/firmware/vino-dock-2-1.3:1.1
sudo sh -c "echo 1 > $N/loading"
sudo dd if=navarro-12.2.26-restore.spkg of=$N/data bs=1758192 count=1 status=none
sudo sh -c "echo 0 > $N/loading"
```

⚠ `/lib/firmware/vino/*.spkg` are held back, so vino will **not** auto-upgrade this dock back on its
own. The downgrade persists across replug and reboot until restored deliberately.

---

# Part 2 — power-cycle persistence, and the automatic upgrade path

## Power cycle: the downgrade persisted

The dock was physically power-cycled on 11.5.29 with two monitors attached. It came back
**still on 11.5.29**, both sockets relit cold, `fb2` created, `DP-2` and `DP-3` connected at
2560x1440. So the image is genuinely committed to flash, and cold bring-up works on the older
firmware -- historically the hard case.

⚠ This is only meaningful because `/lib/firmware/vino/*.spkg` were held back. With them installed,
`update_if_newer` would have re-upgraded the dock on the cold plug and silently destroyed the test.

## `update_if_newer` works -- and flashed TWICE

With **only** `navarro-dock-release.spkg` (12.2.26) installed -- Ella and Ridge were already at
packaged parity, so neither could be touched -- a re-probe of the DFU interface
(`unbind`/`bind` on `2-1.3:1.1`) triggered the automatic path:

```
111654.555  Navarro dock running firmware 11.5.29
111654.556  updating dock firmware 11.5.29 -> 12.2.26
111654.556  flashing 1758192 bytes of dock firmware in 430 block(s) -- do not disconnect
111654.625  dock stopped answering after manifest, as it re-enumerates
111654.625  dock firmware written; it will re-enumerate to run it
111654.693  USB disconnect, device number 70
111656.401  new SuperSpeed Plus Gen 2x1 USB device number 73
111656.412  Int endpoint with wBytesPerInterval of 6 ... setting to 7     <- OLD firmware's descriptor
111656.484  Navarro dock running firmware 11.5.29                        <- STILL OLD
111656.486  updating dock firmware 11.5.29 -> 12.2.26                    <- flashes AGAIN
111656.554  dock firmware written; it will re-enumerate to run it
111658.788  Navarro dock running firmware 12.2.26                        <- took effect
```

✅ The feature works: version comparison, `Firmware::request_nowarn`, the flash, and re-enumeration
all did their job, and the dock ended on 12.2.26 with both screens live.

⚠ **But the first flash did not take.** The dock re-enumerated still reporting 11.5.29, carrying the
old firmware's interrupt-endpoint descriptor (`wBytesPerInterval` 6, absent on 12.2.26 -- the same
field DLM's 2026-08-01 upgrade moved). vino simply flashed it a second time, which worked.

**Why it matters:** `update_if_newer` has **no attempt limit**. It re-flashes whenever the reported
version is older than the package, on every probe. Here it converged after two, but a dock that
never updated its reported version would be re-flashed on every enumeration indefinitely. That is a
robustness gap worth closing regardless of the cause.

⚠ **Cause not established.** The manual downgrade through the fw_upload sysfs path took effect on
the *first* flash; both automatic attempts ran from inside `probe()`. Whether flashing from probe
races the dock's own start-up, or the dock needs two passes for an upgrade specifically, needs
another downgrade + auto-upgrade cycle to separate. Do not assume either.

## Final state

All three docks restored to their original firmware, and the identity blob is byte-identical to
before the experiment:

| dock | firmware | identity |
|---|---|---|
| Navarro `2-1.3` | 12.2.26 | `10400c021a0b03224e617661446f636b` (unchanged) |
| Ella `2-2.1` | 12.2.15 | untouched |
| Ridge `4-2.1` | 12.2.25 | untouched |

`/lib/firmware/vino/*.spkg` are held back again.
