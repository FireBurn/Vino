# Device profiles

Vino supports two DisplayLink DL3 docks. Each is described by a profile the
rest of the driver reads rather than branches on; what genuinely differs in
code — the initialisation sequence, per-head HDCP framing, stream open and mode
description — is named once as a *generation*.

| | Dell Universal Dock D6000 | DL-7400 quad docks |
|---|---|---|
| USB ID | `17e9:6006` | `17e9:7000` |
| Silicon / generation | DL-6xxx, "Ridge" | DL-7000, "Navarro" |
| Identity blob tail | `RidgeDoc` | `NavaDock` |
| Downstream connectors | 2 | 4 |
| Video bulk-OUT endpoints | `0x08`, `0x0b` | `0x08`, `0x0a` (shared) |
| Dock buffers per head | 2 | 3 |
| Strip geometry | 64×16 px, 8 blocks across | 128×8 px, 16 across |
| Interlaced record bands | no | yes |
| Refresh ceiling | 120 Hz (the vendor driver clamps) | link-rate bound only |
| Per-head clock ceiling | 655.35 MHz | 699.5 MHz |
| Dock-wide pixel budget | 884,736,000 px/s | 1,216,512,000 px/s |

Supporting the DL3 display function of these two devices is not a claim that
every device using a related DisplayLink controller has the same endpoint
layout, timing words, or firmware behaviour. New hardware should add a profile
with its own evidence rather than silently widening an existing match.

## Display resources

Each connector has:

- one XRGB8888 primary plane;
- one ARGB8888 cursor plane;
- one CRTC, encoder, and connector;
- one downstream monitor and EDID control path;
- a video bulk-OUT endpoint, which two connectors may share.

The DL-7400's four connectors are addressed by socket number minus one, and are
multiplexed over two endpoints: `0x08` carries connectors 0 and 2, `0x0a`
carries 1 and 3.

The control and authentication session is shared by the device. Mode, monitor,
cursor, damage, and presentation state are tracked per head.

## USB endpoints

| Endpoint | Direction/type | Purpose |
|---|---|---|
| EP0 | control | descriptors, vendor startup, interface setup |
| `0x02` | bulk OUT | plaintext initialization, HDCP, encrypted control |
| `0x84` | bulk IN | HDCP and control replies, unsolicited status |
| `0x08` | bulk OUT | video |
| `0x0a` / `0x0b` | bulk OUT | video, per the profile above |
| `0x83` | interrupt IN | asynchronous dock status on interface 2 |

The endpoint descriptors are discovered and validated at probe. The driver does
not recover arbitrary endpoints from raw pointers in consumer code.

## Accepted display timings

The dock's set-mode request contains two vendor profile words, at offsets 42 and
66. Both are now derived — offset 42 is sync polarity, offset 66 is the CTA VIC
with an aspect flag — and the derivation reproduces every decrypted message in
the corpus byte-exactly, so an unsampled timing is driven rather than refused.
These are the timings a capture backs directly:

| Active mode | Refresh | Timing family |
|---|---:|---|
| 1280×720 | 60 Hz | CTA VIC 4 |
| 1920×1080 | 60 Hz | CTA VIC 16 |
| 1920×1080 | 120 Hz | CTA VIC 63 |
| 2560×1440 | 60 Hz | captured CVT-RB |
| 2560×1440 | 120 Hz | captured CVT-RB |
| 2560×1440 | 165 Hz | captured CVT-RB (DL-7400) |
| 3840×2160 | 60 Hz | captured CVT-RB |

A mode is admitted on the profile's ceilings: the per-head pixel clock, the
refresh cap where the vendor driver is known to clamp, and the dock-wide
active-pixel budget when more than one head is enabled. So a sink's own variant
of a listed resolution is accepted — a TV's CTA 3840×2160p60 (594.00 MHz,
htotal 4400) is driven as readily as the captured CVT-RB one (533.12 MHz), even
though only the latter appears above.

⚠ The binding limit above 120 Hz is the **clock**, not the refresh rate. The
DL-7400 accepts 2560×1440@180 and then fails to deliver it; what separates that
mode from the 165 Hz one it does drive is 714.81 MHz against 699.50 MHz of link
rate. 1440p180 is known-bad on the vendor stack too.

Modes past those ceilings are rejected. Offering a link rate the dock accepts
and then fails to deliver turns an ordinary userspace choice into an unreviewed
hardware experiment.

## Sinks the dock cannot read

The dock reads its own EDID over the downstream link's DDC. A DP→HDMI converter
that mangles or drops DDC therefore breaks the head at its very first step: the
presence probe reports the socket **occupied**, but no `id=0x194` EDID ever
comes back, `reengage_head` returns "no monitor", and the connector stays
disconnected. Nothing is driven, and no message on the wire says why. It is not
a malformed EDID — there is no EDID message at all.

Where the monitor is real and its EDID is readable from a working port on
another machine, that blob can stand in. Vino does not carry a second EDID
source for this; it hands the head to DRM's own override, which already accepts
a blob two ways. The `edid_override` parameter is a bitmask of heads, and for
each named head, while no EDID has been read, vino reports the connector
connected and offers no modes of its own — the two conditions under which the
probe helper consults the override.

```sh
sudo dmesg | grep 'presence reply'                   # which head is actually occupied
sudo tools/hardware/vino-cycle.sh edid_override=1    # bit N = head N
sudo tools/hardware/vino-edid-override.sh 0 tools/hardware/edid/<sink>.edid.bin
sudo tools/hardware/drm-setmode.py --card /dev/dri/card2 --connector DP-2 --mode 1920x1080@60
```

⛔ **Read the presence line first; do not assume a socket number.** The occupied
connector index is not stable across dock re-enumerations — it was observed
moving from head 1 to head 0 with nothing physically touched. Driving video at
an empty head **resets the dock**, and at 640x480 or 4K60 alike it does so about
30 ms after the first video write, which reads exactly like a codec or mode
fault and is not one.

The helper writes the connector's debugfs `edid_override`, then forces the
connector on (`DRM_FORCE_ON`) — in that order, because the core applies an
override only to a connector that is connected and produced no modes, and a
modeless connected connector is mode-set by fbdev before anything can intervene.
With `CONFIG_DRM_LOAD_EDID_FIRMWARE=y` the blob can instead be named once and
survive reloads: `drm_kms_helper.edid_firmware=DP-2:edid/<sink>.bin`.

### Measured 2026-08-05: a Samsung QE75Q60A behind an 8K DP→HDMI cable

The dock never returns an EDID for it, and that connector's probe answers
`status=0x00271105 present=true` with the **EDID-handler ready bit false** — with
the TV awake. Under the override it offers the TV's real 54-mode list, takes a
clean `1920x1080@60` mode-set, and **a picture appears on the panel**. So a
host-supplied EDID is enough to bring the downstream sink up; the dock does not
need to have read it itself.

What then fails is the **first content frame**. The dock accepts the black
training frames (120 000 B, 6 presentations) and then simply stops draining the
endpoint — `stopped accepting video: GET_STATUS=0x0000 halt=0`, no halt, first
real frame `ETIMEDOUT`. Afterwards the control plane wedges, so **cycle the
module between tests** or the next result is meaningless.

★ The live lead is the **allocator row count**: `navarro_total_rows` has measured
DL-7400 values for 2560×1440 and 640×480 only, and both modes this TV offers fall
back to Ridge's `0x6000`. Offset 48 is not derivable, so a wrong value plausibly
mis-sizes the dock's buffer. The 640×480 control test — a mode with a measured
value — has not yet been run from a clean cycle.

⚠ **An override describes the sink, not the link.** If the converter cannot carry
the mode the blob advertises, the screen stays black in exactly the same way.

## Firmware-specific behavior

The protocol contains per-session nonce transforms and per-mode constants.
Those behaviours are isolated behind the profile and backed by captured vectors
and tests.
