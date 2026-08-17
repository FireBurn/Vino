# The same sweep, a different sink: what in the set-mode record follows the monitor

Source: `wire.pcapng` (189 MB), the same Ella dock and the same eleven `kscreen-doctor` steps as
`../ella-modesweep-20260817`, but driving an **MSI MAG 27CQ6F over DisplayPort** (EDID 384 B, 2
extension blocks) instead of the HDMI sink (EDID 256 B, 1 extension block). Step boundaries:
`journal.tsv`. This run has its own AKE and its own
`keys.candidates.json` -- unlike the first sweep, it decrypts standalone.

Ten `id=0x48 sub=0x22` records again, each landing ~0.29 s after its step began.

## 1. Read the resolution out of the record, never off the request

Three steps did not produce the mode that was asked for. The sink advertises a mode list that DLM
synthesises -- it is byte-identical to the HDMI sink's list, including capping 1440p at 59.95 on a
panel that does 165 Hz -- so a requested index is not evidence of what was programmed. The timing
block is:

| requested | actually sent | htotal | vtotal | MHz | Hz |
|---|---|---|---|---|---|
| 2560x1440 | 2560x1440 | 2720 | 1481 | 241.50 | 59.95 |
| 1680x1050 | **1920x1080** | 2200 | 1125 | 148.50 | 60.00 |
| 1280x1024 | 1280x1024 | 1688 | 1066 | 135.00 | 75.02 |
| 1440x900 | **1920x1080** | 2200 | 1125 | 148.50 | 60.00 |
| 1280x960 | **1280x1024** | 1688 | 1066 | 135.00 | 75.02 |
| 1280x720 | 1280x720 | 1650 | 750 | 74.25 | 60.00 |
| 1024x768 | 1024x768 | 1344 | 806 | 65.00 | 60.00 |
| 800x600 | 800x600 | 1056 | 628 | 40.00 | 60.32 |
| 640x480 | 640x480 | 800 | 525 | 25.17 | 59.93 |
| 1920x1080 | 1920x1080 | 2200 | 1125 | 148.50 | 60.00 |

The substitutions are useful rather than lost steps: they delivered `1280x1024` at 75.02 Hz, and
the HDMI run has the same mode at 60.02 Hz, which is what separates the refresh-dependent fields
from the rest.

## 2. The timing follows the SINK. There is no per-mode timing table.

`2560x1440` is the decisive case, because both sinks were asked for it on the same dock:

| | htotal | vtotal | clock | refresh | off44 |
|---|---|---|---|---|---|
| HDMI sink | 2720 | 1474 | 200.25 MHz | 49.95 | 50 |
| DisplayPort sink | 2720 | **1481** | **241.50 MHz** | **59.95** | 60 |

This retracts the claim in the first sweep's summary that the DL-3900 clamps 1440p to 50 Hz. It
does not. The 50 Hz was the HDMI sink's own ceiling, and DLM sent what that sink could take.

Consequence for a driver: build the timing from the mode and the sink's EDID, the ordinary DRM way.
A static per-mode table would have programmed 200.25 MHz into a monitor that wanted 241.50.

## 3. What does NOT follow the sink

Constant in all twenty records across both sinks: `off23 = 2`, `off68 = 0x0200`, `off69 = 2`,
`off72 = 0`, `off22 = 0`.

`off42` is sync polarity and follows the mode, not the sink: high-byte bit 0 is hsync-negative and
bit 1 is vsync-negative, and every mode present in both captures agrees.

`off48` is identical in both captures for every shared hactive, and is now closed-form -- see
section 5 of the HDMI sweep's summary.

## 4. One monitor at a time

Both monitors were physically attached to the dock for this run, on separate sockets. DLM created
**one** evdi device and drove **one** output throughout, and after the power cycle it reported the
DisplayPort sink rather than the HDMI one it had been driving before.

This is recorded as an observation, not a conclusion: it was not the experiment being run, the
HDMI cable's state at that moment is not independently confirmed, and no second-connector bring-up
was captured. It matters because the driver was recently suspected of a presence bug on exactly
this dock's second socket, and the vendor stack may be doing the same thing.
