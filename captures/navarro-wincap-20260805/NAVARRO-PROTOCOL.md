# What Linux already knows about this dock

Reference for reading a Windows capture. Everything here is measured on Linux against DisplayLink's
own binary driver, mostly on 2026-08-02. `navarro-full.md` in this directory is the complete
write-up; this is the part you need to interpret bytes.

## The device

WAVLINK DL7400, `17e9:7000`, "DisplayLink Universal DP Quad Display Docking 16G", platform
`NavaDock`, `bcdDevice = 3922`, SuperSpeed+ 10 Gbps. Four DisplayPort sockets.

Interface 0 endpoints — **this is the whole list**:

| endpoint | role |
|---|---|
| `0x02` bulk OUT | control plane |
| `0x84` bulk IN | control plane |
| `0x08` bulk OUT | video |
| `0x0a` bulk OUT | video |

Also present: interface 1 is USB DFU (firmware flash, `wTransferSize` 16384), interfaces 2-4 are
USB audio. Anything else carrying real traffic is a finding.

⚠ The older Dell D6000 (Ridge, `17e9:6006`) has **four** video endpoints (`0x08 0x0a 0x0b 0x0c`) and
uses `0x08`/`0x0b`. Do not carry D6000 assumptions across.

## Four connectors, two endpoints

**Connector index = physical socket number − 1.** Settled by walking cables between sockets while
recording, and matching against the dock's own timestamped firmware trace.

Four outputs come from **multiplexing, not more channels**:

```
endpoint 0x08  carries connectors 0 and 2   (sockets 1 and 3)
endpoint 0x0a  carries connectors 1 and 3   (sockets 2 and 4)
```

Confirmed by lighting connectors 0 and 2 at the same time: all 1.8 GB of video went down `0x08` and
`0x0a` carried no payload. Two connectors interleave on one endpoint as separately tagged record
streams — not one tiled surface.

## Reading a video record without any key

The record header is **plaintext**, at the start of each bulk-OUT transfer:

```
bytes 0..4    (framing)
bytes 4..8    type   u32 little-endian   -- 4 for a video record
bytes 8..10   sub    u16 little-endian   -- the connector tag
```

```
sub = connector << 3          frame record    0x00  0x08  0x10  0x18
sub = (connector << 3) | 7    stream-open     0x07  0x0f  0x17  0x1f
```

A frame sub appears hundreds of times per session; a stream-open sub appears **exactly once** per
stream, on the endpoint owning that connector.

⭐ **Navarro's pixel payload is plaintext too** (unlike the D6000, where it is sealed). Only the
short stream-open message is encrypted. So a full-payload capture is directly decodable image data —
which is exactly what the Linux driver cannot yet produce and the main reason this capture is worth
taking.

## The control plane (sealed — you will not read this without keys)

`0x02`/`0x84` carry AES-CTR sealed frames. Known structure, for orientation only:

- presence probe `id=0x15 sub=0x20`, connector selector at **payload byte 22**, values 0..3;
- the reply `id=0x44 sub=0x20` reports presence in **bit `0x10` of inner byte 23** — `05 11 27 00`
  occupied, `05 01 <20|21|60|61> 00` empty. ⚠ On this dock the reply *id* is `0x44` for all four
  connectors whether or not a monitor is attached, so the id says nothing about presence;
- hotplug is **pushed by the dock** on the `sub=0x0c` channel (`id=0x3d` → `0x106` → a burst), and
  the driver reacts by probing that connector, reading EDID (`0x15/0x21` → `0x194`) and engaging
  (`0x16/0x23`) — about 110 ms end to end;
- the `sub=0x0c` channel also carries the dock's **ASCII firmware trace**, `|2<ticks> <msgid> <args>`.
  The low nibble of a per-connector msgid is the connector index.

## What would be genuinely new from Windows

In rough order of value:

1. **Video that works.** Linux `vino` gates video off for this dock — the D6000's arm/training
   sequence makes Navarro hard-reset on the first `0x08` write, and Navarro's own sequence is not
   worked out. A Windows capture of a lit dock is a working reference for exactly that gap.
2. **Three or four connectors driven at once.** Linux has only ever seen two, because only two
   monitors were available. If Windows drives more, the record interleaving on a shared endpoint is
   the thing to look at.
3. **Whether Windows sends an ARM burst.** The D6000 needs a "cold ARM" prefix on the first frame
   after a mode set; Linux's DisplayLink driver sends nothing of the sort to Navarro, opening a
   stream with a short message instead. Confirming that from a second implementation matters.
4. **A firmware flash.** `bcdDevice` is `3922` today. If the Windows driver changes it, that is a
   DFU transaction on interface 1 and worth having recorded.
