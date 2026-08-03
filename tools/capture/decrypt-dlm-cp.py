#!/usr/bin/env python3
"""Decrypt every DisplayLink CP frame in a usbmon pcapng using Frida AES candidates.

Unlike the older one-session transcript helpers, this tool understands usbmon device-number
changes and tries all captured `(key, riv)` pairs.  It reports plaintext cap/AKE messages as well
as sealed `wsub=0x24` OUT and `wsub=0x45` IN traffic.  DLM appends a 16-byte Dl3Cmac to both sealed
directions; the tag is deliberately excluded from CTR decryption.

Examples:
  scripts/decrypt-dlm-cp.py CAP.pcapng KEYS.candidates.json --device 61
  scripts/decrypt-dlm-cp.py CAP.pcapng KEYS.candidates.json --start 1784659640 --end 1784659660 --full
"""

from __future__ import annotations

import argparse
import importlib.util
import json
import os
import struct
import sys
from pathlib import Path

try:
    from Crypto.Cipher import AES
    from Crypto.Hash import CMAC
except ImportError:
    from Cryptodome.Cipher import AES
    from Cryptodome.Hash import CMAC


HERE = Path(__file__).resolve().parent
USB_STATS = HERE / "usb-session-stats.py"
_spec = importlib.util.spec_from_file_location("usb_session_stats", USB_STATS)
assert _spec and _spec.loader
_usb = importlib.util.module_from_spec(_spec)
sys.modules[_spec.name] = _usb
_spec.loader.exec_module(_usb)

KNOWN_IDS = {
    0x10, 0x14, 0x15, 0x16, 0x19, 0x1A, 0x1B, 0x1C, 0x1F, 0x22, 0x26, 0x2A,
    0x32, 0x44, 0x48, 0x49, 0x4C, 0x5E, 0x82, 0x94, 0x9A, 0x114, 0x194, 0x401C,
}
KNOWN_SUBS = {
    0x00, 0x04, 0x0B, 0x0C, 0x10, 0x20, 0x21, 0x22, 0x23, 0x24, 0x25,
    0x26, 0x2A, 0x2E, 0x2F, 0x30, 0x31, 0x41, 0x42, 0x43, 0x45, 0x4A,
    0x4B, 0x4C, 0x75, 0x76, 0x78, 0x84, 0x86, 0x90, 0x94,
}


def ctr(key: bytes, riv: bytes, seq: int, data: bytes) -> bytes:
    cipher = AES.new(key, AES.MODE_ECB)
    out = bytearray()
    for off in range(0, len(data), 16):
        iv = riv + bytes(4) + ((seq + off // 16) & 0xFFFFFFFF).to_bytes(4, "big")
        out += bytes(a ^ b for a, b in zip(data[off:off + 16], cipher.encrypt(iv)))
    return bytes(out)


def load_candidates(path: str) -> list[tuple[int | None, bytes, bytes]]:
    rows = json.load(open(path))
    out, seen = [], set()
    for row in rows:
        item = (row.get("pid"), bytes.fromhex(row["key"]), bytes.fromhex(row["riv"]))
        if item[1:] not in seen:
            seen.add(item[1:])
            out.append(item)
    return out


def sane_inner(pt: bytes) -> bool:
    if len(pt) < 8 or pt[6:8] != b"\0\0":
        return False
    # Device-log/status replies use session-varying IDs (0x49/0x5e/0x82 are merely observed
    # examples), so constraining the ID loses valid frames.  Known sub + zero header pad already
    # gives a false-positive probability well below 1e-7 per candidate.
    _iid, sub = struct.unpack_from("<HH", pt)
    return sub in KNOWN_SUBS


def dl3cmac(key: bytes, riv: bytes, seq: int, ciphertext: bytes) -> bytes:
    nonce = bytes([riv[0] ^ 0x80]) + riv[1:]
    cmac = CMAC.new(key, ciphermod=AES)
    cmac.update(nonce + seq.to_bytes(8, "big") + ciphertext)
    return cmac.digest()


def open_frame(candidates: list[tuple[int | None, bytes, bytes]], seq: int,
               wire_body: bytes) -> tuple[int | None, bytes, bytes, bytes] | None:
    if len(wire_body) < 16:
        return None
    ciphertext = wire_body[:-16]
    tag = wire_body[-16:]
    for pid, key, riv in candidates:
        if dl3cmac(key, riv, seq, ciphertext) != tag:
            continue
        pt = ctr(key, riv, seq, ciphertext)
        # A verified tag is authoritative. Large capability pushes legitimately use nonzero
        # inner flag words (for example 0x0080 at offsets 6..7), so the old header heuristic
        # discarded authentic messages and made a complete transcript impossible.
        if len(pt) >= 8:
            return pid, key, riv, pt
    return None


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("pcap")
    ap.add_argument("keys", nargs="?", help="Frida candidate JSON (optional with --key/--riv)")
    ap.add_argument("--key", help="explicit 16-byte session key, hex")
    ap.add_argument("--riv", help="explicit 8-byte OUT RIV, hex; all reply variants are added")
    ap.add_argument("--bus", type=int)
    ap.add_argument("--device", type=int)
    ap.add_argument("--start", type=float, help="absolute pcap epoch timestamp")
    ap.add_argument("--end", type=float, help="absolute pcap epoch timestamp")
    ap.add_argument("--full", action="store_true", help="print the complete inner plaintext")
    ap.add_argument("--json", action="store_true")
    args = ap.parse_args()

    candidates = load_candidates(args.keys) if args.keys else []
    if bool(args.key) != bool(args.riv):
        ap.error("--key and --riv must be supplied together")
    if args.key:
        key = bytes.fromhex(args.key)
        out_riv = bytearray.fromhex(args.riv)
        if len(key) != 16 or len(out_riv) != 8:
            ap.error("--key must be 16 bytes and --riv 8 bytes")
        for direction in (0, 1):
            for head in (0, 1):
                riv = bytearray(out_riv)
                riv[7] ^= direction
                riv[0] ^= head << 7
                candidates.append((None, key, bytes(riv)))
    if not candidates:
        ap.error("provide a candidate JSON or --key with --riv")
    rows = []
    for event in _usb.iter_events(args.pcap):
        if event.xfer_type != _usb.USB_BULK or not event.data:
            continue
        if event.ep == 0x02 and event.kind == "S":
            direction = "OUT"
        elif event.ep == 0x84 and event.kind == "C":
            direction = "IN"
        else:
            continue
        if args.bus is not None and event.bus != args.bus:
            continue
        if args.device is not None and event.dev != args.device:
            continue
        if args.start is not None and event.ts < args.start:
            continue
        if args.end is not None and event.ts > args.end:
            continue

        wire = event.data
        if len(wire) < 16:
            continue
        wtype, wsub, aux, seq = struct.unpack_from("<IHHI", wire, 4)
        opened = None
        tag = "clear"
        if wtype == 4 and wsub in (0x24, 0x45) and len(wire) >= 40:
            opened = open_frame(candidates, seq, wire[16:])
            tag = "sealed" if opened else "UNDECRYPTED"
            inner = opened[3] if opened else b""
        else:
            inner = wire[16:]

        iid = sub = counter = None
        if len(inner) >= 8:
            iid, sub, counter = struct.unpack_from("<HHH", inner)
        row = {
            "ts": event.ts, "bus": event.bus, "device": event.dev,
            "direction": direction, "endpoint": event.ep, "length": len(wire),
            "wire_type": wtype, "wire_sub": wsub, "aux": aux, "seq": seq,
            "crypto": tag, "inner_id": iid, "inner_sub": sub, "inner_counter": counter,
            "plaintext": inner.hex(),
            "pid": opened[0] if opened else None,
            "key": opened[1].hex() if opened else None,
            "riv": opened[2].hex() if opened else None,
        }
        rows.append(row)

    if args.json:
        json.dump(rows, fp=sys.stdout, indent=2)
        print()
        return 0

    print(f"# candidates={len(candidates)} frames={len(rows)}")
    print("# epoch               dev dr  len wire(type/sub/seq)       inner(id/sub/ctr)       crypto key/riv")
    for row in rows:
        inner = (f"{row['inner_id']:#06x}/{row['inner_sub']:#06x}/{row['inner_counter']:5d}"
                 if row["inner_id"] is not None else "       -/-/    -")
        kr = (f"{row['key'][:12]}.. {row['riv']}" if row["key"] else "-")
        print(f"{row['ts']:.6f} {row['bus']}:{row['device']:<3d} {row['direction']:<3s} "
              f"{row['length']:4d} {row['wire_type']}/{row['wire_sub']:#06x}/{row['seq']:5d} "
              f"{inner:>23s} {row['crypto']:<11s} {kr}")
        if args.full and row["plaintext"]:
            print(f"    PT {row['plaintext']}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
