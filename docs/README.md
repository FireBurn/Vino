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
| [`reverse-engineering.md`](reverse-engineering.md) | Evidence policy, capture method, independent oracles, and adding findings |
| [`revdi-chimera.md`](revdi-chimera.md) | Revdi ABI compatibility and Chimera data flow |
| [`upstream.md`](upstream.md) | Current bases, v1/v2 feedback disposition, authorship, and subsystem ownership |
| [`testing.md`](testing.md) | Build-only validation, patch reproduction, examples, and hardware test boundary |

The kernel-facing user documentation is also present in
`kernel/Documentation/gpu/vino.rst`. It intentionally documents the supported
production behavior, not the history of discovering it.

