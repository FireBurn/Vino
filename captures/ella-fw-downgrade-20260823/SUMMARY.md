# vino firmware downgrade + upgrade on Ella (HP 3005pr) — 2026-08-23

**✅ Both directions worked, first try each. This is the run that finally exercises `wait_ready()`.**

| | |
|---|---|
| Dock | HP 3005pr, `17e9:430a`, `2-2.1`, identity `EllaDock`, `bcdDevice` 31.57 |
| Downgrade | 12.2.15 -> **11.4.47** (2024-05-21), 924,928 B, 226 blocks, `sha256 4a5442c5b55e…` |
| Upgrade | 11.4.47 -> **12.2.15** via `update_if_newer`, 928,464 B, 227 blocks, `sha256 b44ba56e5e11…` |
| Restored | 12.2.15, identity `10400c020f08060e456c6c61446f636b` — byte-identical to before |

## ⭐ `wait_ready()` is HW-verified for the first time

Ella is the only dock of the three that uses the DFU busy handshake, so this is the only run in
which vino's poll-timeout handling was ever exercised. It is byte-for-byte DLM's:

```
+0.0000s DETACH
+0.0001s DNLOAD block 0
+0.0006s STATUS -> bStatus=0 poll=100 bState=4   (dfuDNBUSY)
+0.1064s STATUS -> bStatus=0 poll=500 bState=4   (vino slept the 100 ms it was asked for)
+0.6104s STATUS -> bStatus=0 poll=0   bState=5   (vino slept the 500 ms; ready)
+0.6106s DNLOAD block 1
```

Both transfers: **100% image coverage**, `wValue` monotonic, manifest sent, zero non-OK status.

| | blocks | bytes | coverage | span | median gap | bState | bwPollTimeout |
|---|---|---|---|---|---|---|---|
| downgrade 11.4.47 | 226 | 924,928 / 924,928 | 100% | 1.408 s | 0.316 ms | 4:9 5:226 7:4 | 0:230 100:8 500:1 |
| upgrade 12.2.15 | 227 | 928,464 / 928,464 | 100% | 1.408 s | 0.308 ms | 4:9 5:227 7:3 | 0:230 100:8 500:1 |

The dock asks for a wait on 9 blocks (8x100 ms + 1x500 ms, the 500 being the erase on block 0) and
runs free on the rest. vino honours every one. Both transfers land at 1.408 s, essentially DLM's
1.52 s.

## ⭐ The auto-upgrade took ONE flash — unlike Navarro

```
111983.982  Ella dock running firmware 11.4.47
111983.982  updating dock firmware 11.4.47 -> 12.2.15
111983.982  flashing 928464 bytes of dock firmware in 227 block(s)
111985.403  dock firmware written; it will re-enumerate to run it
111989.215  Ella dock running firmware 12.2.15        <- took effect first time
```

## The double-flash question, across all four transfers

| transfer | context | pacing | flashes needed |
|---|---|---|---|
| Navarro downgrade | fw_upload sysfs | unpaced, 0.142 ms/blk, 54 ms | **1** |
| Navarro upgrade | `update_if_newer` in `probe()` | unpaced, 69 ms | **2** |
| Ella downgrade | fw_upload sysfs | dock-paced, 1.408 s | **1** |
| Ella upgrade | `update_if_newer` in `probe()` | dock-paced, 1.408 s | **1** |

⚠ **Only one combination misbehaves: Navarro from `probe()`.** Two variables differ from every
working case at once -- the dock family and the fact that the dock imposes no back-pressure, so a
probe-time flash finishes in 69 ms while the dock is still starting up. **Neither "flashing from
probe races start-up" nor "unpaced writes do not stick" survives on its own**: Ella's upgrade also
runs from `probe()` and sticks, and Navarro's downgrade is also unpaced and sticks. Isolating it
needs another Navarro downgrade + auto-upgrade cycle. Do not assume a cause.

⛔ **The real defect is unchanged and does not depend on the cause:** `update_if_newer` has **no
attempt limit**. It re-flashes on every probe while the reported version is older than the package.
It converged after two here; a dock that never updated its reported version would be re-flashed on
every enumeration forever.

## Not tested

Nothing is plugged into this dock -- `card3-DP-6`/`DP-7` both `disconnected` -- so display bring-up
on 11.4.47 was not exercised. Bind, identity, DRM registration and the upload node all behaved.

## Final state

All three docks on their original firmware, identities byte-identical, `/lib/firmware/vino/*.spkg`
held back again.
