# vino auto-flash on Ridge, first action after a power cycle — 2026-08-23

**✅ It worked. vino flashed the D6000 11.5.28 -> 12.2.25 in ONE flash.**

This is the controlled test for the Ridge failures. It says the earlier failures were **not** a bug in
vino's DFU sequence.

## The test

Armed `update_if_newer` by installing only `ridge-dock-release.spkg` (12.2.25) with the dock
downgraded to 11.5.28, then power-cycled the dock so vino's probe would flash it as its **first
action on a freshly enumerated dock** -- the same position DLM was in when its flash succeeded.

```
113624.693995  Ridge dock running firmware 11.5.28
113624.694538  updating dock firmware 11.5.28 -> 12.2.25
113624.694540  flashing 888688 bytes of dock firmware in 217 block(s)
113628.981897  Ridge dock running firmware 12.2.25          <- one flash, took effect
```

## What this settles

⛔ **RETRACTED: "the Ridge ~750 KB wall is a dock-side limit, not a vino bug."** Also retracted:
"concurrent EP 0x0b video traffic is the cause" (attempt 2 failed with 64 bytes of concurrent
traffic, and the 750 KB stall had none).

The variable that actually tracks the outcome is **how deep into a power session the dock is**:

| attempt | dock state | result |
|---|---|---|
| vino, monitor lit, 12.2.25, cycles deep | engaged, several bring-ups in | stalled at ~750 KB |
| vino, 11.5.28, cycles deep | engaged | 217/217 delivered, did not take |
| vino, 11.5.28, cycles deep (retry) | engaged | 217/217 delivered, did not take |
| usbfs, no driver, fresh after downgrade | idle, nothing bound | **worked** |
| DLM, fresh power cycle | idle, DLM's first contact | **worked** |
| **vino, fresh power cycle, first action** | **idle, first probe** | **worked** |

Consistent with the documented D6000 behaviour of roughly two clean bring-ups per power cycle.
**A firmware flash needs a freshly powered dock.**

## ⚠ A real vino defect this did expose

vino binds **both** interfaces, and the USB core probes them independently, so they race:

```
113624.694540  vino ...:1.1  flashing 888688 bytes ...        <- iface 1 writing firmware
113625.746361  vino ...:1.0  control-session attempt 1/8 failed (ETIMEDOUT); retrying in 250 ms
113627.026316  vino          device identity unavailable (ETIMEDOUT)
113627.034446  vino ...:1.1  dock firmware check skipped (EPROTO)
113627.107568  vino          init_0 failed (ENOENT)
113627.107574  vino ...:1.0  control-session attempt 2/8 failed (ENOENT); retrying in 500 ms
113627.620188  vino          device identity unavailable (EPROTO)
113627.701439  vino          init_0 failed (ENOENT)
113627.701447  vino ...:1.0  control-session attempt 3/8 failed (ENOENT); retrying in 1000 ms
```

Interface 0 tries to bring up the control-plane session against a dock that interface 1 is in the
middle of reflashing. Three session attempts fail, and a second DFU probe reads the identity back as
`EPROTO`. **DLM never does this**: it flashes alone and engages afterwards. The flash survived it
here, but this is the mess that plausibly turns a marginal dock state into a failure. The firmware
update should quiesce, or serialise against, the control-session bring-up.

⛔ **Still open and unchanged:** `update_if_newer` has **no attempt limit** -- it re-flashes on every
probe while the reported version is older than the package
([[project_vino_firmware_flash_works_navarro_20260823]]).

## Also recorded today

`vino/captures/ridge-dlm-flash-20260823/` -- **the first DLM Ridge flash ever captured** (the
reference that was previously missing). DLM's sequence is the same shape as vino's: `DETACH`
wValue=100, 217 monotonic blocks (4096 + final 3952), 888,688 bytes, `GETSTATUS` between every
block, 0.226 ms/block vs vino's 0.214, and it **does** send the zero-length manifest. Within the DFU
window the two are indistinguishable.

⭐ One clean difference: **DLM reads the identity with a vendor request** --
`bmRequestType=0xc1, bRequest=0xFE, wIndex=1, wLength=16` returns the 16-byte blob directly:
`10400b051c0904375269646765446f63`. vino fetches the whole configuration descriptor and walks it for
the `0x40` vendor descriptor. Not shown to be load-bearing, but it is one control transfer instead
of two and it is what the vendor does.

## Final state

All three docks on their original firmware, identities byte-identical, `/lib/firmware/vino/*.spkg`
held back.

| dock | firmware | identity |
|---|---|---|
| Navarro `2-1.3` | 12.2.26 | `10400c021a0b03224e617661446f636b` |
| Ella `2-2.1` | 12.2.15 | `10400c020f08060e456c6c61446f636b` |
| Ridge `4-2.1` | 12.2.25 | `10400c02190904375269646765446f63` |
