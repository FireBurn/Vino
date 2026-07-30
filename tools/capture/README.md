# Device-capture toolkit

Everything needed to record an unfamiliar DisplayLink device driving under DLM, and to read what was
recorded. The procedure, prerequisites and the accumulated gotchas are in
[`docs/new-device-capture.md`](../../docs/new-device-capture.md); this is just the inventory.

| Script | Purpose |
|---|---|
| `capture-newdevice.sh` | One command: usbmon wire capture **and** frida key extraction over the same session, with before/after device state so a firmware flash is provable. Start here. |
| `decode-modeset-live.py` | Attaches frida to a live DLM and recovers the `(ks, riv)` session keys. Called by the above; usable standalone. |
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

## Requirements

```sh
sudo modprobe usbmon            # NOT autoloaded
pip install --user frida pycryptodome
sudo apt install wireshark-cli  # dumpcap, tshark
```
