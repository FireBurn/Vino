# HDR on DisplayLink — what the shipped binaries actually show

Status: **binary survey plus DL7400 wire evidence.** The binary results below are marked ✅ or ⊙ as
before.  The 2026-08-02 Windows DL7400 captures add a measured *negative* result: toggling HDR does
not identify an HDR-specific DisplayLink control, stream tag, or bulk-video profile.  This proves
neither that the host preserves HDR pixels nor how it tone-maps them; it does establish that Vino
must not invent a dock-side HDR transport from the binary strings alone.

Sources on disk:

| binary | version | notes |
|---|---|---|
| `re-binaries/windows/driver/dlidusb3.dll` | **DisplayLink Core Software v12.2.2204.0** | the Windows UMDF **IddCx** driver — current generation, the one that carries DL-7000 HDR |
| `re-binaries/macos/app/DisplayLink Manager.app/…/DisplayLinkUserAgent` | 16.x | universal x86_64 + arm64 |
| `/opt/displaylink/DisplayLinkManager` | package 6.8.1.0 | Linux |
| `captures/navarro-wincap-20260802/out/cap{3,6,7}-*.pcap` | Windows 11 / DL7400 (`17e9:7000`) | per-connector HDR toggle and matched full-payload HDR/SDR traffic |

---

## 1. ✅ There is one pixel-format enum, and all three host stacks carry it

The same twelve entries, in the same order, appear in the Windows driver, the macOS agent and the
Linux DLM:

```
XRGB  XBGR  ARGB  ABGR  UKWN  NM32  NM24  NM16  YU10  24BG  FP16  NM30
```

In the macOS binary they sit in a table of 32-byte records, `{ char fourcc[16]; u64 bytes_per_pixel;
u64 8 }`, which gives the field that matters:

| fourcc | bytes/px | reading |
|---|---|---|
| `XRGB` `XBGR` `ARGB` `ABGR` | 4 | ordinary 8-bit host surfaces |
| `24BG` | 3 | packed 24-bit BGR |
| `NM32` | 4 | DisplayLink native/wire mode, 32 bpp |
| `NM24` | 3 | native, 24 bpp |
| `NM16` | 2 | native, 16 bpp |
| **`NM30`** | **4** | **native, 30 bpp — 10:10:10 packed in four bytes. This is the 10-bit wire format.** |
| **`YU10`** | **2** | **10-bit YUV, 2 bytes/px average ⇒ chroma-subsampled** |
| **`FP16`** | **8** | **four IEEE half floats — RGBA16F, i.e. the Windows HDR swapchain surface as a first-class input** |

⊙ The `NM` family is clearly named by bit count (16/24/30/32), so `NM30` being the deep-colour
member is a strong reading rather than a certainty.

**The most useful consequence:** the 10-bit format is not a Windows invention bolted on above the
protocol — it lives in the shared cross-platform core, and the *Linux* DLM binary contains the
identical enum. So whatever `NM30` means on the wire, the dock side of it is not OS-specific.

⚠ **But do not over-read the enum.** It is a shared core type listing every format the codebase
knows about. The presence of the *name* `NM30` in the Linux binary says the protocol layer has a
10-bit format; it does **not** say the Linux pipeline can ever select one. §1a is the measurement
that separates those.

## 1a. ✅ The platforms differ in the GPU backend, NOT in colour capability

⚠ **This section was wrong until 2026-08-01 and has been rewritten.** The original comparison
searched only the *plaintext* strings of each binary, and concluded Linux had "no BT.2020 path at
all, a real capability gap". That was an artefact of the method: all three binaries keep most of
their strings in an **AES-encrypted store** (see §2a), and Linux's colourimetry strings live there.
Decrypting it shows Linux carries the same names. The retracted claim is kept visible below because
it was published.

Measured across **both** the plaintext strings and the decrypted store:

| token | Windows | macOS | Linux |
|---|---|---|---|
| `ToYuv420Bt2020` | ✅ | ✅ | ✅ (encrypted only) |
| `ToYuv420Bt709` | ✅ | ✅ | ✅ (encrypted only) |
| `ST2084 colorspace used (HDR)` | ✅ | ✅ | ✅ (all encrypted) |
| `10bit profile`, `Adding 30 bit depth to the mode.` | ✅ | ✅ | ✅ (all encrypted) |
| `NM30`, `FP16` | ✅ | ✅ | ✅ |

Linux even carries the assertion
`m_outputControl->configuration().supportedFormatConversions.contains(OutputFormatConversion::ToYuv420Bt2020)`,
which names BT.2020 as a member of an output-format-conversion enum the Linux build tests at runtime.

**What does differ, and is now much better evidenced**, is the GPU backend — by source-file name
rather than by string absence:

| | Windows | macOS | Linux |
|---|---|---|---|
| `libGpu` implementation | **35** files under `libGpu/win32` — OpenCL, DirectCompute, Dx12 encoders | Metal (`MTLDevice`, `default.metallib`) | **`libGpu/posix/PixelProcessorCPU.cpp`** + `PixelProcessorFactory.cpp` |

So Linux encodes on the **CPU** while Windows and macOS have GPU pipelines. That is a real and
substantial difference — but it is a difference of *implementation*, not of whether the codebase
knows about BT.2020 or 10-bit.

<details><summary>Retracted original table (plaintext-only comparison)</summary>

Measured by exact-match string presence in each shipped binary:

| | Linux DLM 6.8.1.0 | macOS agent | Windows `dlidusb3.dll` 12.2.2204.0 |
|---|---|---|---|
| format enum incl. `NM30`/`YU10`/`FP16` | ✅ | ✅ | ✅ |
| GPU conversion backend | ❌ none — CPU | **Metal** (`MTLDevice` ×12, `default.metallib` shipped) | **OpenCL** + D3D interop (`clCreateFromD3D11Texture2D`) |
| colourimetry-specific YUV labels | ❌ only `ToYuv420`, `ToYuv420H`, `ToYuv420L` | ✅ `Bt601` / `Bt709` / **`Bt2020`** | ✅ `Bt601` / `Bt709` / **`Bt2020`** |
| `setHdrMetadata` | ❌ | ✅ | ✅ |
| OS-level HDR10 output | ❌ | ❌ | ✅ DL-7000, Win 11 23H2+ |

Two of those rows need care, in opposite directions:

* **`setHdrMetadata`'s absence on Linux is confounded and proves little.** The *entire*
  `DeviceWindowDispatcher` IPC surface is missing from the Linux binary — `setViewLayout`,
  `notifySurface`, `setSurfaceMap`, `startRender`, `stopRender` and `setGammaRamp` are all absent
  too. Linux DLM is a single process with no such dispatcher, so this row reflects **architecture,
  not capability**.
* **`ToYuv420Bt2020`'s absence is NOT confounded, and is the real finding.** Linux has an
  unqualified `ToYuv420` plus `H`/`L` variants and *no* colourimetry-specific ones. BT.2020 is the
  HDR10 primaries set, so the Linux stack has no BT.2020 conversion path at all — a genuine
  capability gap rather than a naming difference.

⊙ Curiosity: Linux is the only one of the three with a plaintext `ColorimetryDataBlock` (the
CTA-861 EDID block that advertises BT.2020 support).

</details>

⊙ With Linux reading `ColorimetryDataBlock` *and* naming BT.2020 in its conversion enum, the
open question is no longer "can the Linux stack represent BT.2020" but "does anything on Linux ever
select it" — which is a runtime question, not a strings question.

⛔ A trap when doing this comparison: `electro` matches in both the Linux and Windows binaries look
like "electro-optical transfer function". They are **IETF licence boilerplate and a stray Wi-Fi
SSID**. There is no EOTF string in any of the three.

## 2. ✅ Colour conversion is GPU-side, and BT.2020 is in it

`dlidusb3.dll` imports `d3d11`, `d3d12`, `dxgi`, `MFPlat` and **`OpenCL`**, and uses
`clCreateFromD3D11Texture2D` — so surfaces are shared into OpenCL and converted/encoded there.
Build options are visible as plaintext (`-D BGR_FMT`, `-D LUT_IMG`, `-D HORIZONTAL_LAYOUT`,
`-D TU_HDR_LEN=16`) next to a `CSMain` entry-point name.

Three colourimetry names appear:

```
ToYuv420Bt601    ToYuv420Bt709    ToYuv420Bt2020
```

⚠ **Correction (2026-08-01): these are NOT kernel names.** They sit in a logging string table
immediately beside `DPCP`, `HDCP` and `Head group id:` — they are the string labels of a
colour-space enum. An earlier revision of this document called them conversion kernels; that was an
inference from the name, not a measurement. What the strings establish is that the pipeline
*distinguishes* BT.601/709/2020 per stream, which is the part the HDR argument rests on. Whether
there are three separate kernels, one parameterised kernel, or a CPU path is **not** established.

⚠ **`TU_HDR_LEN` is a trap.** It is the *transfer-unit header length* of the codec framing, not
anything to do with high dynamic range. It is the single most tempting false positive in this
binary; do not build an argument on it.

`ToYuv420Bt2020` is still real evidence: BT.2020 is the HDR10 colour primaries set, and its
presence alongside the 601/709 labels means the pipeline selects a colourimetry per stream.

⊙ Combined with `YU10` at 2 bytes/px, the likely HDR path is **FP16 in → BT.2020 4:2:0 10-bit →
codec → wire**, rather than 10-bit RGB end to end. **Confirmed in §2b** by the
`OutputFormatConversion::ToYuv420Bt2020` enum member.

## 2a. ✅ The obfuscated string store is decrypted

Most of what these binaries "say" is not in their plaintext strings. All three keep log messages,
assertion texts and build-server source paths as `@@<base64>@@` blobs:

```
16 bytes IV (zero in every blob seen) || AES-128-CBC ciphertext, PKCS#7 padded
key = 7c01a5ce4fb3f107f1906e7380d76174
```

⭐ **Nothing new had to be reverse-engineered.** That key is selector 6 of the macOS key store
recovered in 2026-06 and filed as "FirmwareModifier/branding AES"
(`macos-decomp/SPKG-KEY-FINDINGS.md`). It is actually the general string-obfuscation key; the
earlier work had it all along without knowing what else it opened.

⛔ The obvious candidate is a decoy: selector 8 decrypts to printable ASCII `"Zr4u7x!A%D*G-KaP"`
and was annotated "obfuscation/default key". It decrypts none of the store.

`re-binaries/decode-string-store.py` reproduces this against any of the three binaries:

| binary | blobs | strings recovered |
|---|---|---|
| `dlidusb3.dll` 12.2.2204.0 | 2803 | **2797** (~105 KB) → `windows-decomp/string-store.txt` |
| macOS agent | 4167 | **2085** → `macos-decomp/string-store.txt` |
| Linux DLM 6.8.1.0 | 2026 | **2024** → `linux-string-store.txt` |

### What it gives us

**271 build-server source paths**, which lay out DisplayLink's internal tree. The codec is
`nivo/core/nivo/dl3/dl3codec/` (38 files), and the names are directly informative:

```
HaarTransformAndSkip.cpp     EntropyEncoder.cpp        Dl3Encoder.cpp
InPairsTileEncoder.cpp       BasicBlockWriter.cpp      AcDecoderInitialisationBlock.cpp
CfbBppLimitGuard.cpp         RidgeDualNivoTuPacker.cpp TileGroupCache.cpp
EllaVideoStreamStrategy.cpp  FireflyVideoStreamStrategy.cpp
ConfigurableRidgeVideoStreamStrategy.cpp
```

— a **Haar** transform stage feeding an **entropy** encoder, per-tile-group encoding with a cache, a
transfer-unit packer, and one video-stream strategy class **per platform** (Ella / Firefly / Ridge).

Runtime strings worth having:

* `, ST2084 colorspace used (HDR)` — PQ is named explicitly, on all three platforms
* `Adding 30 bit depth to the mode.` / `Forcing 10 bit color depth.` — confirms `NM30` = 30 bpp
* `Forcing usage of uncompacted shader encoder, because 10 bit profile is used.` — the 10-bit path
  selects a different encoder
* `Initializing 10bit gamma ramp buffer.` / `Updating 10bit gamma ramp with:`
* `Using hardware gamma ramps.` vs `Using software gamma ramps.`, chosen by output type
  (`Found HDMI/DVI-D output type. Using software gamma ramps.`) — DisplayLink applies gamma in the
  shader exactly as `color-management.md` does in the driver, when hardware cannot
* `Failed to compile kernel from file` — see below

### …and it answers the shader question

**The shader bytecode is embedded directly inside `dlidusb3.dll` in `.rdata` at file offset `0x5ee000`.**

It is encrypted using AES-128 (ECB / CBC zero-IV) with the general obfuscation key `7c01a5ce4fb3f107f1906e7380d76174`. Decrypting the ~1 MB payload starting at offset `0x5ee000` (RVA `0x5ef800`) yields:

1. **Raw SM4/SM5 Instruction Header (`0x00000`–`0x015af`, ~5.5 KB):** Dispatch routines and opcode token streams.
2. **DXBC Container Array (`0x015b0` onwards):** Exactly **19 compiled DXBC Shader Model 5.0 Compute Shader containers (`cs_5_0`)**.

The complete set of extracted `.dxbc` binaries and disassembled `.asm` assembly listings is available in [`gpu-shaders/`](../gpu-shaders/).

### Shader Suite Breakdown

| Shader | Offset | Size | Thread Group | Primary Function |
| :--- | :---: | :---: | :---: | :--- |
| **`shader_00`–`02`** | `0x5ef5b0`–`0x5f3ae0` | 8.8–9.2 KB | `16 × 4 × 1` | Texture input to 1040-word structured records; direct conversion/transform family |
| **`shader_03`–`08`** | `0x5f5ee0`–`0x5ffa90` | 7.7–8.6 KB | `16 × 4 × 1` | Sampled texture input to 528-word structured records; scaling/subsample family |
| **`shader_09`–`14`** | `0x601c30`–`0x644560` | 53.8–55.3 KB | `16 × 1 × 1` | Structured-input bitstream/record packing (`ubfe`, `bfi`, many structured stores) |
| **`shader_15`–`18`** | `0x6517f0`–`0x6cd440` | 140.6–146.4 KB | `8 × 2 × 3` | Fused texture-to-packed-output encoder family; 48 threads, bit reversal and atomic packing |

These labels deliberately stop at what the declarations and instructions establish. In
particular shaders 09–14 contain **no `bfrev` instruction**, while shaders 15–18 contain 128–131;
and nothing in the bytecode alone establishes that the latter perform motion estimation. Run
`scripts/codec-re/dxbc-shader-inventory.py gpu-shaders` from the repository root for reproducible
declaration/opcode counts.

## 2b. ✅ How DLM actually does HDR

Recovered from the decrypted store, so this is measured text rather than inference.

**HDR is a device *profile*, not a per-frame mode.** The device carries an `8bit profile` or a
`10bit profile`; switching is a profile-change event that **recreates the device**:

```
Device profile:                       10bit profile / 8bit profile
Forcing 10 bit color depth.
Checking for profile change
[Profile change] Profile change for …
[Profile change] Recreating device
```

**The mode gains a 30-bit depth**, which pins the format enum from §1 to the wire:

```
Adding 30 bit depth to the mode.        ⇒ NM30 = 30 bpp = 10:10:10
```

**The colour path is YUV 4:2:0 at BT.2020**, and this is now explicit rather than inferred — the
enum members appear by name in all three binaries:

```
OutputFormatConversion::ToYuv420Bt2020
OutputFormatConversion::ToYuv420Bt709
```

⇒ the earlier guess in §2 ("FP16 in → BT.2020 4:2:0 **or 4:2:2** → codec → wire") is confirmed, and
it is **4:2:0**. `YU10` at 2 bytes/px average is exactly 10-bit 4:2:0.

**PQ is named**: `, ST2084 colorspace used (HDR)` — a flag on the logged device/mode state, which
sits in a family that also carries the rest of the output configuration:

```
, DSC On / , DSC Off
, ReducedQuantizationRange On / Off        (limited vs full range)
, ST2084 colorspace used (HDR)
, Output format conversion: …
, Dual NIVO        , Cross-head synchronized       , Just-in-time decode
```

**10-bit costs the fast paths.** Two separate strings say the optimised encoders are disabled:

```
Forcing usage of uncompacted shader encoder, because 10 bit profile is used.
Not using dx12, because software gamma ramp is enabled or 10 bit mode is used.
```

**Gamma is separate in 10-bit**: `Initializing 10bit gamma ramp buffer.`,
`Updating 10bit gamma ramp with: …`

⭐ **DSC is in the stack** — `, DSC On`, `Cannot generate valid RB1 timing for RB2 timing requiring
DSC:`, plus `SlicePixels` and `lastInFrame || lastInSlice` bounds. Display Stream Compression had
not previously been on the map at all, and it matters for the high-bandwidth modes a DL-7000 part
advertises.

### What this still does not tell us

* **The wire format of a 10-bit frame.** The codec bitstream for `NM30`/`YU10` is unrecovered; the
  kernels are compiled from a file we do not have (§2a).
* **How HDR metadata reaches the dock.** `setHdrMetadata` is a host-side IPC method; no control-plane
  message for ST.2086 primaries, MaxCLL or MaxFALL has been identified.
* **Whether any of it engages on DL-6xxx.** The profile machinery is generic across the codebase;
  DisplayLink's own statement is that HDR10 needs DL-7000.

⇒ the differential capture in §6 is still the decisive experiment, but it is now much better aimed:
watch for a **profile change and device recreation**, a mode carrying **30-bit depth**, the output
format conversion switching to **ToYuv420Bt2020**, and the DSC and quantisation-range flags moving.

## 3. ✅ `setHdrMetadata` is per-view state, sibling to `setGammaRamp`

The Windows driver's `DeviceWindowDispatcher` exposes this method list, in table order:

```
terminate   setViewLayout   notifySurface   removeSurface   setSurfaceMap
startRender setUnusedAreaBlankingConfig     stopRender      setGammaRamp    setHdrMetadata
```

So HDR metadata is **device/view-level state pushed the same way as a gamma ramp**, not something
threaded through every frame at this layer. That maps neatly onto DRM's `HDR_OUTPUT_METADATA`
connector property, which is also set-once-per-change rather than per-flip.

## 4. ✅ macOS is not the HDR target — checked, not assumed

The macOS agent has the same format enum (including `FP16` and `NM30`) and the same
`setHdrMetadata` symbol, but **no HDR framework usage**: no `ST2084`, no `ITU_R_2020`, no
`maximumExtendedDynamicRangeColorComponentValue`, no `wantsExtendedDynamicRangeContent`. The only
colour APIs present are ordinary colour management — `CGDisplayCopyColorSpace`,
`kCVImageBufferCGColorSpaceKey`, `kCIImageColorSpace`.

(An initial token scan appeared to find `EDR`; that was base64 noise inside the obfuscated key-store
blobs. It is not real.)

This corroborates DisplayLink's own support statement that HDR10 requires **DL-7000 on Windows 11
23H2 or later**. **The Windows driver is the target.**

## 5. Implementation boundary

The Rust EVDI path now has the safe DRM plumbing needed to expose packed 10-bit scanout:
`XRGB2101010`/related formats, `max bpc` (8–10), `HDR_OUTPUT_METADATA`, and the DP Colorspace
property. That lets a compositor negotiate the standard KMS state without an unsafe binding or a
silently ignored property failure.

This is deliberately **not** HDR transport in Vino. The DL7400 evidence is now stronger than an
absence of decoding: `cap3` toggles HDR one connector at a time on a live link, while `cap6` and
`cap7` repeat the same shared-endpoint animation with HDR on and off. They retain the normal
`connector << 3` record tags and differ by only 0.4% in bytes per frame record (40,278 versus
40,133). No HDR-specific control exchange, metadata payload, or bulk-video framing occurs in those
captures. The most supportable conclusion is that any HDR conversion/composition is upstream of the
captured DisplayLink transport, not a Vino-to-dock feature.

Consequently Vino must keep its video transport SDR-like and must not accept 10-bit primary-plane
formats as a claim of end-to-end HDR support. The earlier suggestions of `set_color_profile(0x02)`,
a CTA InfoFrame payload, a BT.2020 matrix, altered WHT limits, or kernel AVX2/NEON were design
guesses, not reverse-engineered facts. The EVDI KMS properties remain useful capability plumbing,
but they do not authorize HDR advertisement by Vino.

## 6. Verification roadmap

1. Verify all four SDR connectors and recover the sealed Navarro stream-open/key schedule.
2. Preserve the completed Windows HDR/SDR A/B as negative transport evidence; repeat it only if a
   new dock firmware or Windows driver changes the observed control or bulk traffic.
3. Do not add an HDR Vino wire profile or advertise HDR until a future differential capture shows a
   distinct, decodable transport change and an end-to-end HDR monitor test corroborates it.

## Provenance

Windows/macOS binaries and the earlier RE of them live in `re-binaries/` — see its `README.md`,
`windows-decomp/FINDINGS.md` and `macos-decomp/FINDINGS.md`. Those established that all three host
stacks share one CP engine and that the `.spkg` payload key is dock-side. This document is the
colour/HDR layer of the same survey.
