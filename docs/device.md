# Device profile

Vino currently supports the Dell Universal Dock D6000, USB ID `17e9:6006`.
The implementation targets the DisplayLink DL3 display function and does not
claim that every device using a related DisplayLink controller has the same
endpoint layout, timing words, or firmware behavior.

## Display resources

The D6000 profile exposes two independent DRM heads. Each head has:

- one XRGB8888 primary plane;
- one ARGB8888 cursor plane;
- one CRTC, encoder, and connector;
- one downstream EDID and DDC/CI path;
- one video bulk-OUT endpoint.

The control and authentication session is shared by the device. Mode,
monitor, cursor, damage, and presentation state are tracked per head.

## USB endpoints

| Endpoint | Direction/type | Purpose |
|---|---|---|
| EP0 | control | descriptors, vendor startup, interface setup |
| `0x02` | bulk OUT | plaintext initialization, HDCP, encrypted control |
| `0x84` | bulk IN | HDCP and control replies, unsolicited status |
| `0x08` | bulk OUT | head 0 video |
| `0x0b` | bulk OUT | head 1 video |
| `0x83` | interrupt IN | asynchronous dock status on interface 2 |

The endpoint descriptors are discovered and validated at probe. The driver
does not recover arbitrary endpoints from raw pointers in consumer code.

## Accepted display timings

The dock's set-mode request contains vendor profile words that cannot safely be
derived from resolution alone. Vino therefore accepts only fully matched timing
profiles whose wire values are known:

| Active mode | Refresh | Timing family |
|---|---:|---|
| 1280×720 | 60 Hz | CTA VIC 4 |
| 1920×1080 | 60 Hz | CTA VIC 16 |
| 1920×1080 | 120 Hz | CTA VIC 63 |
| 2560×1440 | 60 Hz | captured CVT-RB |
| 2560×1440 | 120 Hz | captured CVT-RB |
| 3840×2160 | 60 Hz | captured CVT-RB |

Validation compares the detailed timing, not only width, height, and nominal
refresh. The per-head pixel-clock limit is 750 MHz. Atomic validation also
checks the dock-wide active-pixel budget when both heads are enabled.

Unsupported modes are rejected. Approximating an unknown profile in a
modesetting driver would turn an ordinary userspace choice into an unreviewed
hardware experiment.

## Firmware-specific behavior

The protocol contains per-session nonce transforms and per-mode constants.
Those behaviors are isolated in the D6000 implementation and backed by captured
vectors and tests. New hardware should add a device profile with its own
evidence rather than silently widening the D6000 match.

