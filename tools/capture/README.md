# Device-capture toolkit

Everything needed to record an unfamiliar DisplayLink device driving under DLM, and to read what was
recorded. The procedure, prerequisites and the accumulated gotchas are in
[`docs/new-device-capture.md`](../../docs/new-device-capture.md); this is just the inventory.

For a device arriving imminently, the phase-by-phase runbook is
[`docs/new-device-day.md`](../../docs/new-device-day.md) — run `preflight-newdevice.sh` the night
before, then work down it.

| Script | Purpose |
|---|---|
| `preflight-newdevice.sh` | **Run the night before.** Verifies (and with `--fix` repairs) every precondition a lost capture has ever come down to: usbmon loaded *and* autoloading, `dumpcap` proven able to open a usbmon interface right now, DLM masked and hash-matched to the build the key hook was derived for, `udl`/`udlfb` blacklisted, `evdi` present, disk headroom, frida importable under root. |
| `capture-firstcontact.sh` | **The one-shot firmware capture.** Prescans the device with DLM masked to learn its bus, starts five independent recorders and proves they are writing, starts DLM and attaches frida while no device is present, and only then has you plug it in. Refuses to stop mid-transfer. |
| `fw-watch.py` | Live flash meter and independent `mon_bin` recorder. Decodes USB DFU class requests as they happen and fingerprints payloads against the shipped `*-release.spkg` images, so you learn whether the flash was caught **while the device is still on the desk**. |
| `fw-scan.py` | Offline: decodes the whole DFU transaction (block numbers, `GETSTATUS` results, manifestation) and reports image coverage. Needs no tshark. |
| `selftest.py` | Synthesises a complete USB DFU flash of a real shipped `.spkg` in `fw-watch.py`'s own record format and asserts `fw-scan.py` recovers it — including a failing flash, an interrupted one, and the S/C URB pairing. Run by `preflight-newdevice.sh`. The capture is one-shot, so the decoder cannot be debugged against real input afterwards. |
| `dl-identity.py` | Places an unfamiliar device in ten seconds with no driver and no DLM: interfaces, endpoints, the DFU functional descriptor, and DisplayLink's identity blob (descriptor type `0x40`) which names the platform and therefore the firmware package that targets it. |
| `capture-modematrix.sh` | Drives DLM through a mode matrix by replugging (never restarting — that would kill the key session). ⚠ Its original purpose — settling the set-mode words — is done: `off42`, `off66` and `off70..73` were decoded from DLM's serializer, and the fields that remain do not vary with the timing, so a mode sweep cannot move them. |
| `dpms-ports-runbook.sh` | **The user-side half of a `capture-portmap.sh` sitting.** Drives one DLM session through DPMS off/on (fast and via the real idle timeout), a single-output disable while its sibling stays lit, a cable move to sockets 3+4, a cold dock power-cycle there, and a hotplug — journalling each step so the wire slices by label. Run it **as the desktop user**; every step is a `kscreen-doctor` call. |
| `capture-newdevice.sh` | One command: usbmon wire capture **and** frida key extraction over the same session, with before/after device state so a firmware flash is provable. **Guided by default** — prompts through a choreography and timestamps each step into `journal.tsv`. |
| `decode-modeset-live.py` | Attaches frida to a live DLM and recovers the `(ks, riv)` session keys. Called by the above; usable standalone. |
| `hook-setupvideo.js` | Frida hook on DLM's **set-mode serializer** — the function that builds `id=0x48 sub=0x22`. Prints the arguments it is handed and the timing block it reads, which is the only way to see the message fields the serializer *receives* rather than computes (the offset-42 base, offset 62, offset 68's low byte). No keys, no decryption, no mode sweep. |
| `run-setupvideo-hook.py` | Spawns DLM under frida with that hook and logs every call. Spawning catches the bring-up mode sets, so nothing on an existing desktop has to be disturbed. |
| `decrypt-dlm-cp.py` | Offline: decrypts the sealed CP frames in a usbmon pcapng using those keys. |
| `usb-session-stats.py` | usbmon pcapng reader, imported by `decrypt-dlm-cp.py`. Also useful alone for endpoint/byte accounting. |

## Why both usbmon and frida

The init sequence and the whole HDCP AKE cross the wire in the clear, but everything after
`SKE_SEND_EKS` is AES-CTR sealed -- and that is where set-mode, the real EDID and the setup burst
live. So the wire alone is unreadable exactly where device support comes from.

⚠ Do **not** use frida for the wire: its USB hook drops bulk transfers (measured 0 against usbmon's
249 for the same traffic). usbmon owns the bytes; frida supplies only keys.

⚠ Keys are per session, so the two captures must overlap. `capture-newdevice.sh` enforces that.

⚠ The key hook attaches at a **build-specific** offset. It is derived for DLM package 6.8.1.0
(`DisplayLinkManager` sha256 `d3584c4369a594e9bcac…`). On another build the offset must be
re-derived -- do not guess, because a wrong hook yields an empty key and a silently keyless capture.

## Slicing a capture by action

The guided run writes `journal.tsv` as `epoch<TAB>begin:label` / `epoch<TAB>end:label`. Those are the
units `decrypt-dlm-cp.py --start/--end` takes, so a step maps straight onto a window of wire:

```sh
awk -F'\t' '/cursor-shape/' out/journal.tsv
./decrypt-dlm-cp.py out/wire.pcapng out/keys.candidates.json --start <t> --end <t>
```

Without it, a long capture is an undifferentiated wall of frames.

## Requirements

```sh
sudo modprobe usbmon            # NOT autoloaded
pip install --user frida pycryptodome
sudo apt install wireshark-cli  # dumpcap, tshark
```
