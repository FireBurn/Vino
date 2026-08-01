# Documentation

The documentation is split by responsibility so protocol evidence does not
become mixed with kernel API design or build instructions.

| Document | Purpose |
|---|---|
| [`device.md`](device.md) | Supported hardware, heads, endpoints, modes, and known limits |
| [`architecture.md`](architecture.md) | Kernel ownership, control, KMS, scanout, and teardown invariants |
| [`protocol/usb.md`](protocol/usb.md) | USB endpoint map, transport header, framing, and I/O lifecycle |
| [`protocol/control.md`](protocol/control.md) | Plain initialization and encrypted control-plane structure |
| [`protocol/hdcp.md`](protocol/hdcp.md) | HDCP 2.2 message flow and verified key derivations |
| [`protocol/video.md`](protocol/video.md) | Video arm, codec, records, damage, and submission |
| [`new-device-capture.md`](new-device-capture.md) | Onboarding DisplayLink hardware vino cannot yet drive: descriptor triage, the gen-1 `init_4` divergence, and a capture procedure built to record a firmware update |
| [`new-device-day.md`](new-device-day.md) | **★ Runbook for the WAVLINK DL7400 (Navarro / DL-7000)**: the DFU shape predicted from descriptors, phase-by-phase commands, and what each capture closes |
| [`hdr.md`](hdr.md) | HDR/deep colour: the shared `NM30`/`YU10`/`FP16` format enum found in all three host binaries, the BT.2020 conversion path, and what vino would need |
| [`reverse-engineering.md`](reverse-engineering.md) | Evidence policy, capture method, independent oracles, and adding findings |
| [`navarro.md`](navarro.md) | **★ DL-7000 / DL7400 vs Ridge**: two video endpoints, no per-head HDCP auth, plaintext AKE framing, and the video stream-open sequence that Ridge's arm replaces |
| [`color-management.md`](color-management.md) | Software `CTM`/`GAMMA_LUT` on the CRTC, shared by vino and evdi: pipeline order, the fused fast path, and the sign-magnitude trap |
| [`revdi-chimera.md`](revdi-chimera.md) | Revdi ABI compatibility and Chimera data flow |
| [`upstream.md`](upstream.md) | Current bases, v1/v2 feedback disposition, authorship, and subsystem ownership |
| [`testing.md`](testing.md) | Build-only validation, patch reproduction, examples, and hardware test boundary |

The kernel-facing user documentation is also present in
`linux/Documentation/gpu/vino.rst`. It intentionally documents the supported
production behavior, not the history of discovering it.
