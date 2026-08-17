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

## 3. The DL-3900 clamps 1440p to 50 Hz

`2560x1440` is the one mode that does not come out at ~60. DLM sends CVT reduced blanking
(2720x1474) at 200.25 MHz, which is **49.95 Hz**, and writes `50` into `off44`. The monitor offers
the mode at 59.95; DLM drives it at 50 anyway.

This is a dock bandwidth decision, not a sink limit, and it is the DL-3900's analogue of the
DL-6xxx clamping 180 Hz to 120. A driver that offers 2560x1440 on this dock and programs it at 60
is asking for more than the vendor ever asks for.

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

## 5. off48 is not a row count

`off48` moves inversely with resolution: 6241 at 1440p, 8192 at 1080p, 11915 at 1280x1024, 16384 at
800x600, 21845 at 640x480. `8192 = 65536/8` and `21845 = 65536/3` exactly, so this is a fixed-point
reciprocal of something per-line, not a count of rows. Whatever the driver currently calls "a
measured row count" for this field, the name is wrong.

## 6. What this retires

The driver currently infers this record for any mode it has no capture for, logging
`has no decrypted DLM profile; inferring ...` and, for 1920x1080, `has no measured row count;
sending this dock's default 0x6000`. All ten modes above are now measured on this dock, including
1920x1080 -- whose real `off48` is `8192`, not `0x6000` (24576).
