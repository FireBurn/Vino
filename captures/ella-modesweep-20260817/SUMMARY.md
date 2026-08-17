# DLM's set-mode record, measured across ten resolutions on the Ella dock (HP 3005pr, DL-3900)

Source: `wire.pcapng` (88 MB), one DLM session driving a single connector through eleven
`kscreen-doctor` mode changes. Step boundaries: `../ella-modesweep-20260817-journal.tsv`.

**Decrypt with `../ella-socket1-20260817/keys.candidates.json`.** This capture has no AKE of its
own -- it rides the session established in that run, so it is not independently decryptable. Keep
the two directories together.

Ten resolution changes produced ten `id=0x48 sub=0x22` records, an 80-byte payload each. Only a
real resolution change re-issues the record; a refresh-only change emits nothing but heartbeats,
which is why the sweep changes pixel count at every step.

## 1. Why these numbers can be trusted

The offsets below were read out of the payload independently, then checked against VESA. Every one
of the ten pixel clocks matches the standard for its mode **exactly**, and the refresh recomputed
from the decoded blanking (`clk / (htotal * vtotal)`) matches the independently decoded `off44` in
all ten cases. Three previously documented constants also reappear unchanged across all ten
records: `off68 = 0x0200`, `off72 = 0`, `off69 = 2`.

## 2. The timing block

| mode | hact | hblank | hfp | hsync | vact | vblank | vfp | vsync | htotal | vtotal | clk (MHz) | Hz |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 2560x1440 | 2560 | 160 | 48 | 32 | 1440 | 34 | 3 | 5 | 2720 | 1474 | 200.25 | **49.95** |
| 1920x1080 | 1920 | 280 | 88 | 44 | 1080 | 45 | 4 | 5 | 2200 | 1125 | 148.50 | 60.00 |
| 1680x1050 | 1680 | 560 | 104 | 176 | 1050 | 39 | 3 | 6 | 2240 | 1089 | 146.25 | 59.95 |
| 1440x900 | 1440 | 464 | 80 | 152 | 900 | 34 | 3 | 6 | 1904 | 934 | 106.50 | 59.89 |
| 1280x1024 | 1280 | 408 | 48 | 112 | 1024 | 42 | 1 | 3 | 1688 | 1066 | 108.00 | 60.02 |
| 1280x960 | 1280 | 520 | 96 | 112 | 960 | 40 | 1 | 3 | 1800 | 1000 | 108.00 | 60.00 |
| 1280x720 | 1280 | 370 | 110 | 40 | 720 | 30 | 5 | 5 | 1650 | 750 | 74.25 | 60.00 |
| 1024x768 | 1024 | 320 | 24 | 136 | 768 | 38 | 3 | 6 | 1344 | 806 | 65.00 | 60.00 |
| 800x600 | 800 | 256 | 40 | 128 | 600 | 28 | 1 | 4 | 1056 | 628 | 40.00 | 60.32 |
| 640x480 | 640 | 160 | 16 | 96 | 480 | 45 | 10 | 2 | 800 | 525 | 25.20 | 60.00 |

Field offsets, from the start of the decrypted inner record:

```
off 26  hactive : u16      off 34  vactive : u16
off 28  hblank  : u16      off 36  vblank  : u16
off 30  hfront  : u16      off 38  vfront  : u16
off 32  hsync   : u16      off 40  vsync   : u16
off 70  pixel clock : u16, units of 10 kHz     <- htotal = hactive + hblank
```

## 3. This sink gets 1440p at 50 Hz -- and that is the SINK's limit, not the dock's

On this sink `2560x1440` is the one mode that does not come out at ~60: DLM sends 2720x1474 at
200.25 MHz, which is **49.95 Hz**, and writes `50` into `off44`.

RETRACTED, and worth stating plainly because the first version of this file said it: this is **not**
a dock-wide clamp and is **not** an analogue of the DL-6xxx clamping 180 Hz to 120. The companion
capture `../ella-modesweep-dp-20260817` runs the same sweep on the same dock with a different sink
(MSI MAG 27CQ6F over DisplayPort) and gets `2560x1440` at **2720x1481, 241.50 MHz, 59.95 Hz**.

Same dock, same driver, same mode; different timing. So the timing in this record follows the
**sink**, and a driver must not carry a static per-mode timing table. Derive the timing from the
mode and the EDID, as any DRM driver does. Only the encoding fields below are constant.

## 4. off42 is sync polarity, and the encoding is now readable

Grouping the ten by sync sense makes the high byte fall out:

| off42 | modes | H | V |
|---|---|---|---|
| `0x0400` | 1920x1080, 1280x1024, 1280x960, 1280x720, 800x600 | + | + |
| `0x0500` | 1680x1050, 1440x900 | - | + |
| `0x0700` | 1024x768, 640x480 | - | - |
| `0x0680` | 2560x1440 | + | - |

So bit 0 of the high byte is "hsync negative" and bit 1 is "vsync negative", confirming the
existing reading of this field. The low byte is zero in nine of ten and `0x80` on the one mode that
uses reduced blanking, so `0x0080` is most likely the CVT-RB flag -- one sample, stated as a
hypothesis, not a fact.

## 5. off48 is not a row count -- it is a reciprocal of the padded line width

`off48` moves inversely with resolution and depends on **hactive alone**. It is unchanged by
vactive (1280x1024, 1280x960 and 1280x720 all give 11915), by refresh (1280x1024 at 60.02 and at
75.02 both give 11915), and by sink (every value below is identical in the DisplayPort capture):

```
off48 = 16777216 // (round_up_128(hactive) + 128)
```

| hactive | padded | off48 |
|---|---|---|
| 2560 | 2560 | 6241 |
| 1920 | 1920 | 8192 |
| 1280 | 1280 | 11915 |
| 1024 | 1024 | 14563 |
| 800 | **896** | 16384 |
| 640 | 640 | 21845 |

Exact for all six. The padding is what gives it away: 800 is the only width that is not a multiple
of 128, and it behaves as 896. So this is `2^24` over a padded line width, a fixed-point step per
pixel -- not a count of rows, and the driver's name for the field is wrong.

## 6. What this retires

The driver currently infers this record for any mode it has no capture for, logging
`has no decrypted DLM profile; inferring ...` and, for 1920x1080, `has no measured row count;
sending this dock's default 0x6000`. All ten modes above are now measured on this dock, including
1920x1080 -- whose real `off48` is `8192`, not `0x6000` (24576).
