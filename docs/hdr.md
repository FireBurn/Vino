# HDR on DisplayLink — what the shipped binaries actually show

Status: **binary survey, no wire evidence yet.** Everything below is either measured out of a
shipped binary (marked ✅) or inferred (marked ⊙). Nothing here has been seen on a wire, because no
DL-7000 hardware has been available until now.

Sources on disk:

| binary | version | notes |
|---|---|---|
| `re-binaries/windows/driver/dlidusb3.dll` | **DisplayLink Core Software v12.2.2204.0** | the Windows UMDF **IddCx** driver — current generation, the one that carries DL-7000 HDR |
| `re-binaries/macos/app/DisplayLink Manager.app/…/DisplayLinkUserAgent` | 16.x | universal x86_64 + arm64 |
| `/opt/displaylink/DisplayLinkManager` | package 6.8.1.0 | Linux |

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

## 1a. ✅ Three tiers, not two — the platforms are NOT equivalent below the enum

Measured by exact-match string presence in each shipped binary:

| | Linux DLM 6.8.1.0 | macOS agent | Windows `dlidusb3.dll` 12.2.2204.0 |
|---|---|---|---|
| format enum incl. `NM30`/`YU10`/`FP16` | ✅ | ✅ | ✅ |
| GPU conversion backend | ❌ none — CPU | **Metal** (`MTLDevice` ×12, `default.metallib` shipped) | **OpenCL** + D3D interop (`clCreateFromD3D11Texture2D`) |
| colourimetry-specific YUV kernels | ❌ only `ToYuv420`, `ToYuv420H`, `ToYuv420L` | ✅ `Bt601` / `Bt709` / **`Bt2020`** | ✅ `Bt601` / `Bt709` / **`Bt2020`** |
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

⊙ Curiosity, unexplained: Linux is the **only** one of the three with `ColorimetryDataBlock` (the
CTA-861 EDID block that advertises BT.2020 support). It reads colorimetry capability from the
monitor while having no conversion path to use it — most likely just a different EDID-parser
lineage rather than anything meaningful.

⛔ A trap when doing this comparison: `electro` matches in both the Linux and Windows binaries look
like "electro-optical transfer function". They are **IETF licence boilerplate and a stray Wi-Fi
SSID**. There is no EOTF string in any of the three.

## 2. ✅ Colour conversion is GPU-side, and BT.2020 is in it

`dlidusb3.dll` imports `d3d11`, `d3d12`, `dxgi`, `MFPlat` and **`OpenCL`**, and uses
`clCreateFromD3D11Texture2D` — so surfaces are shared into OpenCL and converted/encoded there.
Three conversion kernels are named:

```
ToYuv420Bt601    ToYuv420Bt709    ToYuv420Bt2020
```

with OpenCL build options visible as `-D BGR_FMT`, `-D LUT_IMG`, `-D HORIZONTAL_LAYOUT`,
`-D TU_HDR_LEN=16`.

⚠ **`TU_HDR_LEN` is a trap.** It is the *transfer-unit header length* of the codec framing, not
anything to do with high dynamic range. It is the single most tempting false positive in this
binary; do not build an argument on it.

`ToYuv420Bt2020` is real evidence: BT.2020 is the HDR10 colour primaries set, and its presence
alongside the 601/709 kernels means the pipeline picks a colourimetry matrix per stream.

⊙ Combined with `YU10` at 2 bytes/px, the likely HDR path is **FP16 in → BT.2020 4:2:0 or 4:2:2
10-bit → codec → wire**, rather than 10-bit RGB end to end. Not proven.

The OpenCL kernel **source is not in plaintext** in the DLL (`clCreateProgramWithSource` is
imported, but no `__kernel`/`get_global_id` strings survive), so it is compressed, packed, or in a
resource. Extracting it would be the single highest-value next step on the codec side.

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

## 5. ⊙ What this means for vino

The dock-facing work and the DRM-facing work are separable.

**DRM side** — what a KMS driver must expose for a compositor to drive HDR at all:

* `HDR_OUTPUT_METADATA` connector property (carries the ST.2086 mastering primaries, MaxCLL,
  MaxFALL) — the direct analogue of `setHdrMetadata`;
* `Colorspace` connector property (`BT2020_RGB` / `BT2020_YCC`);
* `max bpc` connector property, and modes advertised at 10 bpc;
* framebuffer formats beyond `XRGB8888`: at minimum `XRGB2101010`/`ARGB2101010`, ideally
  `ABGR16161616F` to match what the Windows path consumes.

None of that exists in vino today, and `kscreen-doctor` currently reports `HDR: incapable` and
`Color resolution: unknown` for vino's outputs — which is exactly what a driver with none of these
properties looks like.

**Codec/transport side — this is the hard part.** vino's WHT codec is 8-bit end to end: `u8` RGB in,
8-bit colour blocks, an 8-bit quantiser and 8-bit codebooks. `NM30` cannot be reached by adding a
DRM property; it needs either a 10-bit codec mode or a different transport format. That is a
substantial piece of work and should not be started before the wire evidence in §6 exists.

⛔ **It cannot be prototyped on the D6000.** DisplayLink states HDR10 needs DL-7000; Ridge would at
best accept a 10-bit framebuffer and output SDR. Do not spend time making the D6000 "nearly" do it.

## 6. What would settle it — in priority order

1. **Capture a DL-7000 `id=0x48 sub=0x22` set-mode and diff it against the D6000's.** Free: the
   mode matrix run on the new dock produces this anyway. Any word that is new, or that differs
   structurally rather than numerically, is a bpc/format/colourimetry candidate.
2. **Read the DL-7000's `DISPLAY-CAP` push (`id=0x78 sub=0x30`).** It is the dock's per-head
   capability descriptor, and a deep-colour or HDR capability bit would most naturally live there.
   Also free — it arrives unprompted in every connect window.
3. **Find the format enum's cross-reference in the Linux DLM.** The binary contains `NM30`, so
   something selects it. The existing Ghidra projects make this a bounded job, and it would say
   whether the Linux stack can ever emit 10-bit or merely carries the enum.
4. **The Windows differential experiment** — the decisive one, and it needs a Windows host with the
   dock and an HDR monitor:
   * fix resolution and refresh, display a static gradient, capture with HDR **off**;
   * enable HDR **without changing the mode**, capture again;
   * repeat with black, mid-grey, saturated BT.2020, and values differing only in the bottom two
     bits of a 10-bit ramp.

   The small **control-plane** delta gives link bit depth, colourimetry and the metadata message.
   The large **video-endpoint** delta says whether DL-7000 uses new codec framing or carries extra
   precision alongside the existing data. The bottom-two-bits case is the one that proves whether
   10-bit precision actually survives to the wire, and it is the case a casual capture would miss.
5. **Extract the OpenCL kernels** from `dlidusb3.dll` (packed, not plaintext). That would give the
   conversion and encode path in readable OpenCL C rather than by inference.

## Provenance

Windows/macOS binaries and the earlier RE of them live in `re-binaries/` — see its `README.md`,
`windows-decomp/FINDINGS.md` and `macos-decomp/FINDINGS.md`. Those established that all three host
stacks share one CP engine and that the `.spkg` payload key is dock-side. This document is the
colour/HDR layer of the same survey.
