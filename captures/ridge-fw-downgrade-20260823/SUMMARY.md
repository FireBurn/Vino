# vino firmware flash on Ridge (D6000) — 2026-08-23

**Result: vino's DFU write path does not work on a DL-6xxx. It is not a downgrade problem — the
dock stops at ~750 KB of ANY image, including a re-flash of the version it is already running.**

The dock is unharmed and still runs **12.2.25**. Nothing was restored because nothing was ever
written: no attempt reached the manifest phase, and the running image was never touched.

## Setup

| | |
|---|---|
| Dock | Dell D6000, `17e9:6006`, `4-2.1`, identity `RidgeDoc`, firmware **12.2.25** |
| Downgrade image | **11.5.28** (2024-09-30), 870,928 B, `sha256 7cb69ef54854…`, `RD`=`RidgeDoc` |
| Control image | **12.2.25** (2026-03-23), 888,688 B, `sha256 804aecb4721a…` — the running version |
| Path | `/sys/class/firmware/vino-dock-4-2.1:1.1` (`update_if_newer` refuses downgrade and same-version by design) |

`/lib/firmware/vino/*.spkg` were already held back, so vino could not silently re-upgrade the dock
on re-enumeration. That is what made a downgrade observable at all.

## The four runs

| image / mode | blocks | bytes | % of image | span | median gap | non-OK status |
|---|---|---|---|---|---|---|
| 11.5.28 unpaced | 190 | 778,240 | 89.4% | 40.7 ms | 0.214 ms | 0 |
| 11.5.28 unpaced | 187 | 765,952 | 87.9% | 41.5 ms | 0.224 ms | 0 |
| 11.5.28 paced 2 ms | 183 | 749,568 | 86.1% | 495.8 ms | 2.737 ms | 0 |
| **12.2.25 (same version)** | **183** | **749,568** | 84.3% | 491.6 ms | 2.737 ms | 0 |

Every run: `bStatus=0`, `bwPollTimeout=0`, `bState=5` (dfuDNLOAD_IDLE) after every block, zero USB
errors, then the dock stops answering, drops off the bus ~140 ms later, and re-enumerates ~5 s after
that still on 12.2.25. vino times out the pending `GETSTATUS` at its 5 s `XFER_TIMEOUT` and logs
`firmware upload failed (ETIMEDOUT)`.

## What this eliminates

- ⛔ **Not a downgrade rejection.** Re-flashing the *identical running version* fails at exactly the
  same block (183) and byte (749,568) as the older image. The dock is not refusing old firmware.
- ⛔ **Not the image.** Two different images, different sizes, same cutoff.
- ⛔ **Not the transfer rate.** Pacing at 2.7 ms/block — 12.8x slower, and slower than either
  vendor flash — moved the cutoff *down* (183 vs 190), not up.
- ⛔ **Not the busy/poll handshake.** DLM's successful Navarro flash returned
  `bwPollTimeout=0, bState=5` on all 430 status replies, exactly as Ridge does. An always-idle
  status is normal and is compatible with a flash that completes.
- ⛔ **Not a per-session size cap in the protocol.** DLM pushed Navarro's full **1,758,192 B** as one
  monotonic session, blocks 0..429, in 0.911 s, with no block-number restart.

**What is left:** the DL-6xxx accepts roughly 750-780 KB through this path and then resets, with no
error ever reported. That is inside the dock's firmware, and nothing on the host side that has been
varied changes it.

## The two vendor flashes, decoded for comparison

Both were decoded from usbmon here for the first time.

**DLM / Ella** (`captures/ella-firmware-flash-20260810/fw.mon`) — 227 blocks, 928,464 B, 1.52 s.
This dock *does* use the busy handshake:

```
+0.0002s  DNLOAD block 0 len 4096
+0.0008s  STATUS  bStatus=0 bwPollTimeout=100 ms bState=4   <- dfuDNBUSY
+0.1010s  STATUS  bStatus=0 bwPollTimeout=500 ms bState=4
+0.6011s  STATUS  bStatus=0 bwPollTimeout=0   ms bState=5   <- ready
+0.6020s  DNLOAD block 1 len 4096
```

**DLM / Navarro** (`captures/newdevice-firstcontact-20260801-182636/fw.mon`) — 430 blocks,
1,758,192 B, 0.911 s, 2.1 ms/block, `bState=5 bwPollTimeout=0` throughout. Flashed 11.5.23 ->
12.2.26 (identity `0b 05 17` -> `0c 02 1a`).

Neither issued a USB reset after `DFU_DETACH`, so vino's comment that the download proceeds in the
runtime interface is correct for both, and the `bitWillDetach = 0` in all three docks' DFU
functional descriptors is not being acted on by the vendor either.

## What is now established about vino's flash path

- ✅ The **transport is correct**: `DFU_DETACH`, ascending 4096-byte `DFU_DNLOAD`, `GETSTATUS`
  between blocks. 190 blocks with zero USB errors. `BLOCK = 4096` confirmed against a third platform.
- ✅ The **guards work**: `is_package` and the `RD` family check passed for a legitimate Ridge image,
  and the fw_upload node is per-device, so the right dock was addressed.
- ✅ The **failure is safe**: 5 s timeout, clean `disconnected`, no partial image committed, dock
  recovers on its own with both sockets relit. Four failed flashes, four full recoveries.
- ❌ The path has **never completed a flash**. The only recorded DisplayLink flashes are DLM's. This
  is the first end-to-end exercise of `firmware::flash()`, and it does not work on Ridge.
- ⚠ `wait_ready` is correct but was never exercised — Ridge never reported dfuDNBUSY, so vino never
  slept. On Ella it would pace itself correctly.

⚠ **The auto-flash hazard in `update_if_newer` is unchanged and still real** — it is only untested,
not disproven. Keep `/lib/firmware/vino/*.spkg` held back before plugging in an unfamiliar dock
(see the preflight in `tools/capture/`).

## Next, if this is picked up

There is **no vendor Ridge flash capture** — the D6000 arrived already on current firmware. Settling
the ~750 KB limit needs either one (Dell's `Dock_D6000_FW_Updater_A00_JW5XM.exe` is an unexplored
second source for the host-side sequence) or RE of DLM's Ridge DFU path. Do not spend more runs
varying host-side rate or image: those are eliminated above.

## Files

- `ridge-11.5.28.spkg`, `ridge-12.2.25-restore.spkg` — the two images written
- `flash-11.5.28.jsonl`, `flash-11.5.28-try2.jsonl`, `flash-11.5.28-paced.jsonl`,
  `flash-12.2.25-same.jsonl` — decoded usbmon of all four attempts
- `monread.py` — mon_bin reader (dumpcap is blocked in the agent sandbox)

The pacing experiment was a temporary `flash_block_delay_us` module parameter; it was reverted after
being refuted and `vino/linux` is back at HEAD with no local changes.
