# Ella / HP 3005pr firmware flash, captured end to end (2026-08-10)

**This is the only recording of a DisplayLink firmware update on the wire. Do not delete it.**
The dock it was taken from is now at the post-flash revision, so it cannot be re-taken without a
downgrade -- which is one of the things this capture exists to make possible.

Only the Ella dock was attached during this capture.

## The device

| | before | after |
|---|---|---|
| USB id | `17e9:430a` | `17e9:430a` |
| `bcdDevice` (firmware revision) | **0157** | **3157** |
| `bcdUSB` | 3.10 | **3.20** |
| identity descriptor (config desc type `0x40`) | `10 40 09 02 26 08 06 0e "EllaDock"` | `10 40 0c 02 0f 08 06 0e "EllaDock"` |

`HP 3005pr USB3.0 Port Replicator`, DisplayLink Ella / DL-3900 family.

Note the `bcdUSB` change: the flash moved the dock from USB 3.10 to 3.20 signalling. A downgrade
has to be expected to move it back.

## The flash protocol, as observed

Standard **USB DFU on interface 1** (`class=fe sub=01 proto=01`). Nothing proprietary in the
transport; the proprietary part is the container.

| step | request | notes |
|---|---|---|
| detach | `bmRequestType 0x21`, `bRequest 0` (DFU_DETACH), iface 1 | switches the dock into its bootloader; **a re-enumeration follows** |
| download | `bmRequestType 0x21`, `bRequest 1` (DFU_DNLOAD) | **4096-byte blocks** (block 15 = 64 KiB, block 31 = 128 KiB) |
| poll | `bmRequestType 0xa1`, `bRequest 3` (DFU_GETSTATUS) | interleaved throughout |
| finish | zero-length DFU_DNLOAD | end of image -> manifestation phase -> **the device resets itself** |

Total wall time for the download phase was about **1.5 s** (`...384.487` to `...384.689`), after a
~1.3 s gap following detach for the bootloader re-enumeration. The capture tool then waited 45 s of
DFU silence before declaring completion.

⚠ The image bytes appear on the wire in the clear and match the `.spkg` contents directly -- the
capture tool matched them against `ella-dock-release.spkg` as they streamed. There is no additional
encryption layer over the DFU payload.

## The `.spkg` container format

⭐ **`ELLA` is the container magic for every DisplayLink firmware package, not a family marker.**
Verified against all four packages shipped in `/opt/displaylink`:

| file | size | magic @0 | LE u32 @4 |
|---|---|---|---|
| `ella-dock-release.spkg` | 928464 | `ELLA` | 928464 |
| `firefly-monitor-release.spkg` | 364048 | `ELLA` | 364048 |
| `navarro-dock-release.spkg` | 1758192 | `ELLA` | 1758192 |
| `ridge-dock-release.spkg` | 888688 | `ELLA` | 888688 |

So the header begins: `magic[4] = "ELLA"`, `total_length[4] = <LE u32, equal to the file size>`.
Do not read the magic as "this package is for Ella" -- `flash-events.txt` phrases it in a way that
invites exactly that mistake.

The remainder of the header is not decoded, and does not need to be: the bytes DFU writes are not a
subset of the container but the whole of it, so a flasher never has to parse the header. See below.

## Contents

| file | what it is |
|---|---|
| `flash-events.txt` | annotated timeline of the whole flash -- the primary artefact |
| `identity-diff.txt` | before/after device identity, the `bcdDevice` and descriptor change |
| `*.spkg` | **byte-identical copies of the four images**, sha256-verified against `spkg-sha256.txt` |
| `spkg-sha256.txt` | hashes as they were in `/opt/displaylink` at capture time |
| `wire-allbus.pcapng`, `wire-bus4.pcapng` | the wire, all buses and bus 4 alone |
| `keys-raw.json`, `keys.candidates.json` | session keys, so the control plane is decryptable |
| `before-*` / `after-*` | lsusb, dmesg, identity, either side of the flash |
| `fw.mon`, `fw-scan.txt`, `xhci-trace.txt` | firmware scanner output and xHCI trace |

The `.spkg` copies are the reason this directory is enough on its own: `/opt/displaylink` will be
overwritten by the next DisplayLink package upgrade, and these are the exact images this dock was
flashed with.

## The DFU payload is the whole package, verbatim -- SETTLED

Extracted every `bmRequestType 0x21` data stage from `wire-bus4.pcapng` and concatenated it:

| | |
|---|---|
| image blocks | **227** (226 x 4096 + one 2768-byte tail) |
| stream length | **928,464 bytes** |
| `ella-dock-release.spkg` | **928,464 bytes** |
| **byte-identical from offset 0** | **yes** |

⭐ **There is no container header to strip.** The entire `.spkg`, including the `ELLA` magic and the
length word, is written to the device exactly as it sits on disk. Block size is 4096 with a short
final block.

⚠ Twelve stray 2-byte `0x21` control writes are interleaved with the image blocks and must be
excluded when reassembling, or the stream comes out 24 bytes long and compares as garbage.

⛔ `flash-events.txt`'s "image offset 24880" is the capture tool's own match accounting, **not** a
container offset -- that offset in the file is a zero run. Do not use it.

## To implement update and downgrade

1. ~~Decode the container header~~ -- done, see above: write the file as-is.
2. A flasher is therefore: `DFU_DETACH` (`0x21`/`0`, iface 1) -> wait for the bootloader to
   re-enumerate -> `DFU_DNLOAD` (`0x21`/`1`) the file in 4096-byte blocks, polling `DFU_GETSTATUS`
   (`0xa1`/`3`) -> zero-length `DFU_DNLOAD` -> the device resets itself.
3. **Downgrade is untested.** Nothing here shows the dock accepting an older revision, and DFU has
   no rollback semantics of its own -- the bootloader may or may not enforce a version check. The
   pre-flash image for this dock is **not** in hand: `dlcap-ella-preflash` recorded the dock at
   `bcdDevice 0157` but did not dump its flash. Recovering `0157` would need it sourced elsewhere.
4. ⛔ vino's DFU probe **already writes firmware to any dock reporting an older version, on
   enumeration**, with no modprobe needed. That is why `/lib/firmware/vino/*.spkg` is currently
   moved to `held-back/` and `blacklist vino` is in `/etc/modprobe.d/zz-dl-capture.conf`. Any
   downgrade work has to reckon with that path re-flashing the dock forward again the moment it is
   plugged in.

## Related captures

- `../ella-preflash-20260810/` -- the dock at `bcdDevice 0157`, before the flash
- `../ella-keyed-20260810/` -- 2.7 GB keyed session
- `../ella-video-evdi-20260810/` -- DLM driving displays; **290 MB of pixels on EP02**, the control
  endpoint, which is what vino needs before it can drive this dock at all
