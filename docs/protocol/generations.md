# The three DisplayLink generations, side by side

What is the same across every dock this driver drives, what differs, and where each difference was
measured. `drivers/gpu/drm/vino/profile.rs` is the machine-readable form of this table; when the two
disagree, the code is right and this file is stale.

| | **Ridge** DL-6xxx | **Ella** DL-3x00 | **Navarro** DL-7000 |
|---|---|---|---|
| reference unit | Dell D6000 `17e9:6006` | HP 3005pr `17e9:430a` | DL-7400 `17e9:7000` |
| firmware seen | 12.2.25 (`bcdDevice 31.59`) | 12.2.15 (`31.57`) | (`39.22`) |
| connectors | 2 | 2 | 4 |
| control OUT / IN | `0x02` / `0x84` | `0x02` / `0x84` | `0x02` / `0x84` |
| video OUT | `0x08`, `0x0b` | **none -- video shares `0x02`** | `0x08`, `0x0a` (two per endpoint) |
| ring slots on the wire | 3 (`0`, `2`, `4`) | 3 | 3 |
| strip layout | 8 blocks across x 2 down (64x16 px) | 8 across (64x16) | 16 across x 1 down (128x8) |
| interlaced y bands | no | yes | yes |
| band parity in record `sub` | yes | no | no |
| record `sub` for head h | `h` | `h` | `h << 3` |
| content stream id | `0x08 \| h` | `0x08 \| h` | `(h << 3) \| 7` |
| stream opening | ARM burst prefixed to frame 0 | plaintext open + sealed report | plaintext open + sealed report |
| stream marker kind | `0x03` | `0x01` | `0x05` |
| `strm2` marker at off24 | `0x06` | `0x10` | `0x0c` |
| dock-wide init records | vendor sends them; vino does not (open) | yes | yes |
| presence reported | yes | **no** | yes |
| hardware cursor | yes (`0x401c/0x41`, 16448 B) | no | yes |
| 10-bit / HDR | no | no | yes |
| framebuffer allocation | fixed `0x4000`/`0x6000` | derived from 48 MiB/head | measured per mode |
| sink-down `0x16/0x2e` state | 3 | 3 | 3 |
| EDID handler | **one, shared between heads** | per head | per head |

## What every generation shares

The control plane is the same protocol on all three. Session init is plaintext, then an HDCP 2.2 AKE
(`AKE_Init` / `AKE_Send_Cert` / `AKE_Send_H_prime` / LC / SKE), and everything after
`SKE_Send_Eks` is sealed.

* **Session key** = `ske_ks XOR B`, `B = 26abee3893d0c4326143a4bf5b45d6ec`.
* **OUT content nonce** = the delivered RIV with `byte7 ^= stream_id`. The control stream is `0x04`;
  video streams use the id in the table above. `cp::stream_content_nonce` is the single place this
  is computed.
* **IN nonce** = the OUT nonce with `byte7 ^= 0x01`.
* **Dl3Cmac nonce** = the OUT nonce with `byte0 ^= 0x80`. Encrypt-then-MAC; the two nonces differ.
* **Framing**: a 16-byte clear wire header, AES-CTR ciphertext, a 16-byte Dl3Cmac.
* **Lockstep**: each `H->D id=X sub=Y` draws a `D->H id=0x14 sub=Y` acknowledgment. The dock also
  pushes unprompted: a per-head capability report, its certificate, firmware-trace lines under
  `sub=0x0c`, and heartbeats.

### A reply's id is `0x14` plus its payload length

This has now caught the driver out four separate times, on two different generations, and it is the
single most useful rule in the protocol.

| reply | id | payload |
|---|---|---|
| generic acknowledgment | `0x14` | 0 B |
| capability report | `0x76`, `0x78` | 98 B, 100 B |
| EDID, two blocks | `0x114` | 256 B |
| EDID, three blocks | `0x194` | 384 B |

⇒ **Match the `sub`, never a literal id.** `cp::edid_reply_len` documents it;
`is_display_cap_reply` and `probe_reply_status` both had to be rewritten to obey it.

### Message vocabulary

Common to all three, `sub` being what identifies them:

```
0x05/0x08, 0x04/0x00   plaintext session init
0x22/0x10, 0x1f/0x10   AKE init and transmitter capability
0x9a/0x10              AKE no-stored-km
0x32/0x10              LC / SKE
0x2a/0x10, 0x26/0x10   per-head stream manage
0x04/0x06              seal
0x14/0x00 -> 0x4c/0x00 dock descriptor
0x14/0x30 -> 0x78/0x30 display capability (dock-wide on Ridge; per head elsewhere)
0x15/0x20 -> 0x44/0x20 presence probe, connector at off22
0x16/0x4b              downstream DDC readiness kick
0x15/0x21 -> 0x1?4/21  EDID fetch
0x16/0x23              EDID handler engage, connector in off22 AND off23
0x15/0x53 -> 0x1c/0x53 post-EDID capability query
0x48/0x22              set mode
0x16/0x2e, 0x16/0x2f   sink power and mode bracket
0x14/0x0c              status poll, also the keepalive
0x16/0x75              session heartbeat
0x03/0x82              dock -> host: downstream state changed (see below)
0x1b/0x42, 0x401c/0x41, 0x1a/0x43   cursor position, bitmap, visibility
```

### The set-mode record

One layout on all three; the inner offsets are stable.

```
off22 connector (one-based on Ridge)   off23 generation/DMA format
off26 hactive   off28 hblank   off30 hfront  off32 hsync
off34 vactive   off36 vblank   off38 vfront  off40 vsync
off42 sync polarity   off44 refresh Hz
off46 render stride   off48 framebuffer row count
off66 CTA VIC and flags   off68 0x0200   off70 pixel clock, units of 10 kHz
```

The record is self-validating: `off70 / (htotal * vtotal)` must equal `off44`, and htotal is
`off26 + off28`. Every decoded message across three docks satisfies both.

⛔ **Timing follows the sink, never a per-mode table.** The same dock at the same requested mode
gives a different pixel clock on a different monitor. Derive it from the DRM mode and the EDID.

## Where the generations genuinely diverge in code

`profile::Generation` names the only split that is code rather than data, and it has two arms rather
than three: **Ella speaks Ridge's protocol**. What Ella needs beyond that is data --- its video
records travel on the control endpoint, its bands interlace, its stream opens the Navarro way.

The four things that are real code:

1. **The initialisation sequence.** Navarro takes three dock-wide records ahead of the per-head
   blocks; sending them to a dock that does not expect them leaves every later inner counter and AES
   block out of step.
2. **Per-head HDCP framing.** Ridge and Ella put a one-based head number at off23. Navarro puts a
   one-hot selector at `22 + head`, and the high half of that selector lands in inner bytes 6..7 of
   its pushes --- which is why an inbound frame has to be authenticated by its MAC rather than
   sniffed for zero padding.
3. **Stream open.** Ridge prefixes an ARM burst to its first frame. The other two send a short
   plaintext open on the head's video `sub`, then a sealed report on its stream id.
4. **Mode description.** Only in how the framebuffer allocation is stated; the timing block is
   shared.

Everything else --- endpoints, strip geometry, connector count, link limits, pacing, poll periods,
presence semantics --- is a field on `DockProfile`, read rather than branched on. Adding a variant
that differs only in connector count or head count should be a new `static DockProfile` and nothing
else.

## Per-generation detail

* Ridge: `docs/ridge.md`
* Navarro: `docs/navarro.md`, `docs/protocol/navarro-decoded.md`
* Ella: `docs/new-device-day-ella.md`
* Control-plane crypto: `docs/protocol/control.md`, `docs/protocol/hdcp.md`
* Codec: `docs/protocol/video.md`
