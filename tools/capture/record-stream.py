#!/usr/bin/env python3
"""Dump a DisplayLink dock's OUT record stream in order, decrypting what it can.

This is the "reference sequence" tool.  Spot-checking a capture is how three confident
conclusions got retracted in one session; one ordered pass over the whole stream is what
found the records that were actually missing.  Build the sequence first, every time.

It differs from `decrypt-dlm-cp.py` in two ways that matter on a dock which carries video on
its control pipe:

  * records are parsed from the *concatenated* endpoint stream, because a record spans USB
    transfer boundaries and a per-transfer parse silently overruns; and
  * sealed records on a head's content-stream id are decrypted with that head's own video key,
    which is a different key per stream and is discovered from the candidate list by its
    Dl3Cmac tag rather than by guessing at the plaintext.

Usage:
    record-stream.py CAP.pcapng KEYS.candidates.json [--endpoint 2] [--count 200]
    record-stream.py CAP.pcapng KEYS.candidates.json --stats
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "codec"))
from usbmon_read import iter_transfers  # noqa: E402

try:
    from Crypto.Cipher import AES
    from Crypto.Hash import CMAC
except ImportError:
    from Cryptodome.Cipher import AES
    from Cryptodome.Hash import CMAC

# A record's `sub` is the plane discriminator.  These are the control ones; anything else is
# video, and the low bits are the connector.
SUB_CP_PLAIN = 0x04
SUB_CP_SEALED = 0x24

CODEC_SYNC = b"\x01\x28"


def ctr(key: bytes, riv: bytes, seq: int, data: bytes) -> bytes:
    cipher = AES.new(key, AES.MODE_ECB)
    out = bytearray()
    for off in range(0, len(data), 16):
        iv = riv + bytes(4) + ((seq + off // 16) & 0xFFFFFFFF).to_bytes(4, "big")
        out += bytes(a ^ b for a, b in zip(data[off:off + 16], cipher.encrypt(iv)))
    return bytes(out)


def dl3cmac(key: bytes, riv: bytes, seq: int, ciphertext: bytes) -> bytes:
    nonce = bytes([riv[0] ^ 0x80]) + riv[1:]
    mac = CMAC.new(key, ciphermod=AES)
    mac.update(nonce + seq.to_bytes(8, "big") + ciphertext)
    return mac.digest()


class Opener:
    """Opens sealed bodies, remembering which key belongs to which `sub`.

    The Dl3Cmac tag is the oracle: a verified tag is authoritative, and no guess about the
    plaintext's shape is needed or trustworthy.  Once a `sub` has been opened, its key is tried
    first, which turns a 723-candidate sweep into one CMAC per record.
    """

    def __init__(self, candidates):
        self.candidates = candidates
        self.known: dict[int, tuple[bytes, bytes]] = {}

    def open(self, sub: int, seq: int, body: bytes):
        if len(body) < 32:
            return None
        ciphertext, tag = body[:-16], body[-16:]
        known = self.known.get(sub)
        order = [known] + self.candidates if known else self.candidates
        for key, riv in order:
            if dl3cmac(key, riv, seq, ciphertext) == tag:
                self.known[sub] = (key, riv)
                return key, ctr(key, riv, seq, ciphertext)
        return None


def records(path: str, endpoint: int):
    """Yield whole records from the concatenated OUT stream of one endpoint."""
    buf = bytearray()
    for _dev, _ep, payload in iter_transfers(path, endpoint=endpoint, transfer_type=3):
        buf += payload
        while len(buf) >= 16:
            stride = int.from_bytes(buf[2:4], "little") + 4
            if stride < 16 or stride % 16:
                # Resync a byte at a time rather than dropping the rest of the buffer.
                del buf[:1]
                continue
            if len(buf) < stride:
                break
            record = bytes(buf[:stride])
            del buf[:stride]
            yield record


def fields(record: bytes):
    return (
        int.from_bytes(record[4:8], "little"),   # type
        int.from_bytes(record[8:10], "little"),  # sub
        int.from_bytes(record[10:12], "little"),  # aux
        int.from_bytes(record[12:16], "little"),  # seq: the AES-CTR block counter
        record[16:],
    )


def describe(index: int, record: bytes, opener: Opener) -> str:
    typ, sub, aux, seq, body = fields(record)
    head = f"#{index:<6} len={len(record):<6} type={typ} sub={sub:#04x} aux={aux:#06x} seq={seq:<6}"

    if typ == 2 and len(body) >= 4:
        named = int.from_bytes(body[0:2], "little")
        marker = int.from_bytes(body[2:4], "little")
        return f"{head} ANNOUNCE sub={named:#04x} marker={marker}"

    if sub == SUB_CP_PLAIN and len(body) >= 28:
        iid = int.from_bytes(body[0:2], "little")
        isub = int.from_bytes(body[2:4], "little")
        return (f"{head} CP-PLAIN id={iid:#06x} sub={isub:#04x} "
                f"ctr={int.from_bytes(body[4:6], 'little')} hdcp-msg={body[27]:#04x}")

    if sub == SUB_CP_SEALED:
        got = opener.open(sub, seq, body)
        if not got:
            return f"{head} CP-SEALED <no key>"
        _key, pt = got
        iid = int.from_bytes(pt[0:2], "little")
        isub = int.from_bytes(pt[2:4], "little")
        extra = f" off22={pt[22]:#04x} off23={pt[23]:#04x}" if len(pt) > 23 else ""
        return (f"{head} CP id={iid:#06x} sub={isub:#04x} "
                f"ctr={int.from_bytes(pt[4:6], 'little')} clen={len(pt)}{extra}")

    if len(body) >= 4 and body[2:4] == CODEC_SYNC:
        return f"{head} IMAGE first-strip-len={int.from_bytes(body[0:2], 'little')}"

    got = opener.open(sub, seq, body)
    if got:
        _key, pt = got
        return f"{head} STREAM clen={len(pt)} :: {pt.hex(' ')}"
    return f"{head} RAW {body[:32].hex(' ')}"


def stats(path: str, endpoint: int) -> None:
    from collections import Counter

    kinds: Counter = Counter()
    total = 0
    for record in records(path, endpoint):
        typ, sub, aux, _seq, body = fields(record)
        total += len(record)
        image = len(body) >= 4 and body[2:4] == CODEC_SYNC
        kinds[("IMAGE", sub) if image else (typ, sub, aux, len(record))] += 1
    print(f"{sum(kinds.values())} records, {total} bytes")
    for kind, count in kinds.most_common():
        print(f"  {count:>8}  {kind}")


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("capture")
    ap.add_argument("keys", help="frida candidate list, as keys.candidates.json")
    ap.add_argument("--endpoint", type=int, default=2, help="OUT endpoint number (default 2)")
    ap.add_argument("--count", type=int, default=200, help="records to print (0 for all)")
    ap.add_argument("--skip", type=int, default=0, help="records to skip first")
    ap.add_argument("--stats", action="store_true", help="count record kinds instead of listing")
    args = ap.parse_args()

    if args.stats:
        stats(args.capture, args.endpoint)
        return 0

    rows = json.load(open(args.keys))
    seen = set()
    candidates = []
    for row in rows:
        item = (bytes.fromhex(row["key"]), bytes.fromhex(row["riv"]))
        if item not in seen:
            seen.add(item)
            candidates.append(item)
    opener = Opener(candidates)

    for index, record in enumerate(records(args.capture, args.endpoint)):
        if index < args.skip:
            continue
        if args.count and index >= args.skip + args.count:
            break
        print(describe(index, record, opener))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
