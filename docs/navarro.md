# Navarro (DL-7000) — how it differs from Ridge

The WAVLINK DL7400, `17e9:7000`, "Universal DP Quad Display Docking 16G", identity tail
`NavaDock`. Everything here is measured from hardware or from
`captures/navarro-dlm-session-20260801-190654` (247 MB of DLM driving this dock with a monitor lit,
8 key candidates, decrypts with `tools/capture/decrypt-dlm-cp.py`).

## Status

vino brings this dock up **stably as a dual-head KMS device**: control session, EDID, 36
EDID-derived modes per head, both connectors `connected`, compositor driving them. **No picture
yet** — video is deliberately gated off, see §4.

## 1. Descriptors

| | D6000 (Ridge) | DL7400 (Navarro) |
|---|---|---|
| speed | SuperSpeed 5 Gbps | **SuperSpeed+ 10 Gbps** |
| control | bulk OUT `0x02`, bulk IN `0x84` | same |
| video endpoints | `0x08`, `0x0a`, `0x0b`, `0x0c` (**four**) | **`0x08`, `0x0a` (two)** |
| heads driven | 2, on `0x08` and `0x0b` | 2, on `0x08` and `0x0a` |
| DFU | iface 1, `wTransferSize` 16384 | same |

⚠ Four DisplayPort connectors but only two video endpoints, and DLM brings up exactly two outputs
(`DVI-I-1`, `DVI-I-2`). DisplayLink's own strings describe **"Dual NIVO"** with `TiledNivoViewer`
and `TiledDisplayGroupingStategy`, so four outputs are most likely two streams each carrying a
tiled pair. Driving four needs the tiling protocol, which is not reverse-engineered.

## 2. ⭐ No per-head HDCP authentication

Ridge authenticates **every head**: its own AKE, `rrx`, `Edkey` and `V`, with the head's video key
coming from that head's SKE.

Navarro does not. The capture contains **exactly one `AKE_No_Stored_km`** and **no per-head burst
of any kind**, sealed or plaintext; the sealed traffic is only `0x14`/`0x15`/`0x16` — status, EDID
and stream control. Running the burst anyway leaves every head waiting for an `AKE_Send_Rrx` that
never comes, so no head completes and the driver reports a missing sink for a monitor that is
plainly there.

Carried as `DockProfile::per_head_auth`.

⊙ Open: where the video keys come from on this platform, given there is no per-head SKE. Most
likely the main-link SKE, but unverified.

## 3. The main AKE is plaintext-framed

The dock's `AKE_Send_Rrx` push (`id=0x10 sub=0x84`, HDCP msg-id `0x06`) arrives with wire-sub
**`0x25`** — the plaintext framing, inner payload at offset 16, msg-id at 25, `rrx` at 26..34.
Ridge seals the same message (`wsub=0x45`). `cp::perhead_rrx` now accepts both.

## 4. ⭐ Video: the Ridge arm sequence hard-resets this dock

vino's first EP08 write killed the device within a millisecond, taking the control session, EDID
and connectors with it — which presented as a spontaneous reset loop every few seconds:

```
KMS CRTC enable -- head 0 display ON, mode 2560x1440@120
head=0 persistent video queue opened by prompt training
head 0 startup frame submitted after 0 ms (205696 bytes)
head 1 sink re-engagement failed (ENODEV)
```

Video is therefore gated by `DockProfile::video_supported`, checked at `run_pending_scanout()` —
which every scanout write funnels through — and at the prompt-training submission.

### What DLM actually sends

From the capture, in order:

```
ep 0x08  len     48   hdr 00 00 2c 00 04 00 00 00   id=0x17 sub=0x02    <- head 0 stream open
ep 0x0a  len     48   hdr 00 00 2c 00 04 00 00 00   id=0x1f sub=0x02    <- head 1 stream open
ep 0x08  len  65536   hdr 00 00 1c 00 02 00 00 00   id=0x07 sub=0x00    <- first frame, part 1
ep 0x08  len  54480   (continuation, raw payload)
ep 0x08  len  65536   hdr 00 00 1c 00 04 00 00 00   id=0x00 sub=0x04    <- steady-state frame
ep 0x08  len  53056   (continuation)
ep 0x0a  len  65536   hdr 00 00 1c 00 04 00 00 00   id=0x08 sub=0x04    <- head 1 steady state
```

So:

* the stream opens with a **48-byte sealed frame** per head — `id=0x17` on `0x08`, `id=0x1f` on
  `0x0a` — not with a large ARM+black frame as on Ridge;
* the **first** data frame uses `sub=0x02` and `id=0x07`; every later frame uses `sub=0x04` with
  `id=0x00` (head 0) or `id=0x08` (head 1);
* frames arrive as a 65536-byte URB plus a remainder (~53–54 KB), i.e. ~119 KB per 2560x1440
  frame.

⚠ vino currently submits a **205696-byte** Ridge ARM+black frame as its opening write. That is the
message the dock rejects.

### Implementing it

1. Build the 48-byte per-head stream-open (`id=0x17`/`0x1f`, `sub=0x02`) and send it on the head's
   video endpoint before any pixels.
2. Frame the first payload with `sub=0x02`/`id=0x07`, then steady state with `sub=0x04` and the
   per-head id.
3. Establish where the video key comes from without a per-head SKE (§2).

Only then lift `video_supported` for the profile.
