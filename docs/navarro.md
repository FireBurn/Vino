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

⊙ Open: where the video keys come from on this platform, given there is no per-head SKE. **Not**
the main-link key — see §2a.

## 2a. ⭐ There are three keys, and the video keys are per-head

Measured 2026-08-02 from `captures/navarro-dlm-session-20260801-190654`, using the Dl3Cmac tag as
the oracle. That is the right tool here: it verifies a candidate `(key, nonce)` against the captured
frame with **no assumption about the plaintext**, where a plaintext-shape guess silently fails when
the guess is wrong — which is exactly what happened on the first attempt at this.

| stream | key | nonce |
|---|---|---|
| control (`0x02`/`0x84`) | `44514ae4ad1ea7f1cf272d8dfabea994` | `09d8c99a45b38dc8` |
| video head 0 (`0x08`) | `989f349c3c8edfb07fe9196921d1868f` | `566c8be978e96150` |
| video head 1 (`0x0a`) | `67e0309ac4c19bdf23b147a2e0662298` | `367ef5b61c3d66ac` |

Frida records each twice, the second being the same key with `nonce[0] ^ 0x80` — the Dl3Cmac nonce.

⛔ The video keys are **not** the control key and **not** derived from it by any simple relation:
`ctrl^H0`, `ctrl^H1`, `H0^H1`, `ks^H0`, `ks^H1` show no structure, and neither AES nor CMAC of the
link key or `ks = ctrl ^ B` under small counters, `B`, or any of the three nonces reproduces them.

⛔ They are **not transmitted**. Decrypting all 1100 sealed control frames with the control key and
searching for each video key, each key `^ B`, and each nonce yields nothing; the nonces do not
appear in the clear anywhere in 5546 frames either.

⭐ And they cannot come from extra handshakes, because **vino and DLM run the identical HDCP
message set, exactly once each**: `AKE_Send_Cert`, `AKE_Send_rrx`, `AKE_Send_H_prime`,
`AKE_Send_Pairing_Info`, `LC_Send_L_prime`, `RepeaterAuth_Send_ReceiverID_List`. One session, three
keys.

⇒ **The per-head video keys are derived locally, by both ends, from the single link session.** vino
therefore already holds all the input material; what is missing is only the derivation. Finding it
means reading DLM's key schedule in the binary — a wire capture cannot show a local computation, so
**a cold-plug capture would not answer this**.

## 2b. ⭐ The key schedule: one sealing key per wire sub

Traced 2026-08-02 with `tools/capture/keysched-backtrace.py`, which attaches to a live DLM and
records the callers the first time each distinct key appears. ⚠ The key schedule is **dormant on a
warm dock** — the setter fires zero times until a fresh session, which a
`echo 0 > .../authorized; echo 1` re-enumeration is enough to force.

A cold session creates **exactly five** sealing keys, all through one call site, and the factory
takes a selector:

| selector (`ecx`) | as hex | stream |
|---|---|---|
| 4 | `0x04` | control plane |
| 7 | `0x07` | head 0 video frames |
| 15 | `0x0f` | head 1 video frames |
| 23 | `0x17` | head 0 **stream-open** |
| 31 | `0x1f` | head 1 **stream-open** |

⭐ **The selector is the wire `sub`.** Every value matches the subs already measured in §4 and §4b,
including the eight-apart head spacing — so a sealing key is chosen per message type per head, and
the stream-open has its own key distinct from the frame key.

Offsets in DLM 3.4.26 (module-relative), from `objdump`:

| offset | role |
|---|---|
| `0x86cca0` | sealer factory `(out, cfg, keysrc, sub)` — copies key and riv out of `keysrc`, allocates a `0x90`-byte sealer, constructs it with the sub |
| `0x85c830` | key copy-in, key at `keysrc+0x18`, 16 bytes |
| `0x85c850` | riv copy-in, riv at `keysrc+0x30`, 8 bytes |
| `0x85c5f0` | the wrapper that calls the factory; selector arrives in `ecx` |

⇒ **key-source object layout: key at `+0x18` (16 B), riv at `+0x30` (8 B).**

⊙ **Next step:** the factory only *copies* an already-derived key, so the derivation is whatever
fills `keysrc+0x18`. Hook `0x85c5f0`, take the `rdx` pointer, and find what writes that object.

⚠ Backtraces past frame #0 here come from the fuzzy unwinder and are **not reliable** — chasing
frame #6 of one led straight into iostream formatting code. Trust `this.returnAddress` and the
accurate frame #0 only.

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

### 4b. ⭐ The stream-open plaintext, decrypted

With the keys from §2a, both heads' stream-opens decrypt:

```
head 0 (ep 0x08, wire sub 0x17):  04 00 08 04 05 00 06 00 07 01 08 02 07 00 | d9 33
head 1 (ep 0x0a, wire sub 0x1f):  04 00 08 04 05 00 06 00 07 01 08 02 07 00 | c6 6c
```

⭐ The two are **byte-identical except the final `u16`**, so the head is carried entirely by the
wire `sub` (`0x17`/`0x1f`) and not by the content. The trailing word differs per head and per
session (`0x33d9`, `0x6cc6`) with no relation to the head index — most likely a token, in the same
family as the msg0 token that the dock provably cannot validate.

⚠ `cp::navarro_stream_open()` currently builds something else entirely — the wire sub, `0x0002`,
then a counter, then zeros. That is a guess made before this decrypt and it is **wrong**; the
constant 14-byte prefix above is what the dock is actually sent.

### Implementing it

1. Rewrite `cp::navarro_stream_open()` to emit the measured 14-byte prefix (§4b).
2. Derive the per-head video key (§2a) — **the one remaining unknown**, and the only thing standing
   between here and a picture. It is a local key schedule, so it has to come out of the DLM binary.
3. Seal the stream-open with that key and send it on the head's video endpoint before any pixels.
4. Frame the first payload with `sub=0x02`/`id=0x07`, then steady state with `sub=0x04`.
5. ~~Establish where the bulk video key comes from~~ — moot: the payload is plaintext (§4a).

Only then lift `video_supported` for the profile.
