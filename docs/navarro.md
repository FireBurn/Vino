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

## 4. ⭐ Video: the dock accepts the bytes, then resets on a watchdog

⚠ **Corrected 2026-08-02.** This section previously said vino's first EP08 write "killed the device
within a millisecond". That is not what happens, and the distinction matters: the dock **accepts
every video byte without error** and resets several seconds later. See §4a for the measurement.

The original symptom — a spontaneous reset loop every few seconds:

```
KMS CRTC enable -- head 0 display ON, mode 2560x1440@120
head=0 persistent video queue opened by prompt training
head 0 startup frame submitted after 0 ms (205696 bytes)
head 1 sink re-engagement failed (ENODEV)
```

Video is therefore gated by `DockProfile::video_supported`, checked at `run_pending_scanout()` —
which every scanout write funnels through — and at the prompt-training submission.

### 4a. ⭐ Measured: the stream-open is genuinely required

The gate can be lifted at runtime with the `force_video=1` module parameter, which exists to answer
exactly one question: does this platform need its sealed stream-open, or is correct record framing
enough? It is off by default because the answer is that the dock resets.

Two runs, same module (`86059d8c9ed3f34d`), same 80 s capture window
(`captures/navarro-forcevideo-20260802`):

| run | dock instances in 80 s | video writes | video URB errors |
|---|---|---|---|
| `force_video=0` (control) | **1** — 80.3 s continuous | 0 | – |
| `force_video=1` | **9** — ~9.0 s each | 9 per instance, 474368 B | **0** |

The cycle is highly regular, and within each instance:

* every video URB completes with **status 0** — the dock does not reject the framing;
* the control plane keeps working for **~6.2 s** after the last video write, with normal `0x02`/
  `0x84` request/reply traffic;
* only then does the device re-enumerate.

⇒ This is a **watchdog expiring, not a malformed write being refused.** The dock takes the pixels,
has no stream context to put them in because the 48-byte stream-open never arrived, and gives up.
Correct record framing alone is therefore *not* sufficient, and there is no way around building the
stream-open.

⛔ Do not re-run this expecting a different result with framing tweaks: the bytes are already being
accepted, so framing is not what the dock is complaining about.

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

### ⭐ The pixel payload is plaintext — only the stream-open is sealed

Measured 2026-08-02 over `captures/navarro-dlm-modeset-20260802-005453` (628 DLM video frames).
Shannon entropy of the first 4 KiB of payload, past the 16-byte transport header:

| frame | entropy | reading |
|---|---|---|
| `ep 0x0a`, `id=0x02` record stream | **5.71** bits/byte | structured |
| `ep 0x0a`, continuation | **3.43** bits/byte | structured |

Encrypted data sits at 8.00. The payload is visibly regular in hex as well
(`… 01 fc 00 7e 00 3f 80 1f c0 0f e0 07 f0 03 f8 01 …`), and the inner records carry the same
`00 00 1c 00 02 00 00 00` header shape as the outer frame. **So video pixels are never encrypted on
this platform**, exactly as on Ridge.

⇒ The video key question in §2 collapses to a single message: the **48-byte stream-open** is the
only sealed thing on the video endpoints. Once it can be built, nothing else on `0x08`/`0x0a` needs
a key.

Inner records observed on `ep 0x0a` use sub `0x0f` and `0x1f`; head 0 uses `0x07` and `0x17`. The
per-head offset is **8** throughout, consistent with `DockProfile::head_sub_shift = 3`.

### ⛔ The stream-open is not reproducible in software

The 48-byte opens appear **only in a capture spanning a cold connect**. Attempts that produced
video traffic but no stream-open:

| attempt | result |
|---|---|
| restart `displaylink-driver.service` | frames resume, no open |
| `kscreen-doctor output.…enable`/`disable` | no open |
| resolution change (forces `0x48/0x22`) | no open |
| `echo 0 > …/authorized` then `1` (twice) | full re-enumeration, 628 frames, **no open** |

The last of these was run with vino unbound and blacklisted so DLM certainly owned the device
(`DLM reclaimed after 4s`, 540 distinct keys captured) — the dock still reused its existing video
stream. ⇒ **capturing the stream-open needs a physical replug or dock power-cycle with frida
attached.** Everything else for video is built and gated behind `video_supported`.

⭐ Also measured: DLM drives this dock at **2560x1440@164.96** on both heads. It does **not** clamp
to 120 Hz the way it does on Ridge ([[project_dlm_clamps_to_120_cp_decrypted_20260726]]), so this is
the platform that can finally answer the `off72` mode word.

### Implementing it

1. Build the 48-byte per-head stream-open (`id=0x17`/`0x1f`, `sub=0x02`) and send it on the head's
   video endpoint before any pixels. **Needs one physical replug capture** — see above.
2. Frame the first payload with `sub=0x02`/`id=0x07`, then steady state with `sub=0x04` and the
   per-head id.
3. ~~Establish where the video key comes from~~ — moot: the payload is plaintext.

Only then lift `video_supported` for the profile.
