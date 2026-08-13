#!/usr/bin/env python3
"""Diff two senders' *decrypted* control messages byte for byte, and report every divergence.

`sequence-diff.py` compares record shapes and needs no keys, which makes it the right tool for
"where does the bring-up first go wrong".  It cannot see inside a sealed body, so a message of the
right length carrying the wrong bytes reads as agreement.  This tool opens both streams and
compares the plaintext.

It deliberately reports the WHOLE list rather than stopping at the first difference.  Fixing one
divergence per hardware run costs a dock power cycle each time; the point here is to enumerate
them all from captures already on disk.

  plaintext-diff.py DLM.pcapng DLM.keys.json VINO.pcapng VINO.keys.json

Messages are matched by `(id, sub)` -- the control plane's own identity for a message -- and the
first instance of each is compared.  Offsets known to differ for reasons that are not bugs are
reported separately from the rest:

  * offset 4    the message counter, which is a per-session sequence
  * offset 22   the head/connector selector, where two senders drove different sockets
  * offsets 74+ the six-byte tail, which 74 vendor status polls show carries 74 distinct values
                with no constant byte and which the dock does not validate

Anything outside those is a real difference in what the two drivers told the dock.
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from importlib import import_module

_rs = import_module("record-stream".replace("-", "_")) if False else None

# record-stream.py is not an importable name; load it by path.
import importlib.util

_spec = importlib.util.spec_from_file_location(
    "record_stream", Path(__file__).resolve().parent / "record-stream.py"
)
record_stream = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(record_stream)

VOLATILE = {4, 5, 22}
TAIL_FROM = 74


def load_keys(path: str):
    return [
        (bytes.fromhex(c["key"]), bytes.fromhex(c["riv"]))
        for c in json.load(open(path))
    ]


def plaintexts(capture: str, keys_path: str, endpoint: int):
    """Map `(id, sub) -> (first plaintext, count)` for every CP message in a stream."""
    opener = record_stream.Opener(load_keys(keys_path))
    out: dict[tuple[int, int], tuple[bytes, int]] = {}
    for record in record_stream.records(capture, endpoint):
        typ, sub, _aux, seq, body = record_stream.fields(record)
        if typ != 4:
            continue
        if sub == record_stream.SUB_CP_PLAIN:
            clear = body
        elif sub == record_stream.SUB_CP_SEALED:
            opened = opener.open(sub, seq, body)
            if not opened:
                continue
            clear = opened[1]
        else:
            continue
        if len(clear) < 4:
            continue
        ident = (
            int.from_bytes(clear[0:2], "little"),
            int.from_bytes(clear[2:4], "little"),
        )
        if ident in out:
            out[ident] = (out[ident][0], out[ident][1] + 1)
        else:
            out[ident] = (clear, 1)
    return out


def video_records(capture: str, keys_path: str, endpoint: int):
    """Map a video record's kind to its first body and a count.

    The kind is `(aux, length)` on a plaintext record and `("sealed", length)` on a sealed one,
    which separates the ring descriptor, the frame opener, the decoder configuration and the
    per-frame stream report from each other and from the images.
    """
    opener = record_stream.Opener(load_keys(keys_path))
    out: dict[tuple, tuple[bytes, int]] = {}
    for record in record_stream.records(capture, endpoint):
        typ, sub, aux, seq, body = record_stream.fields(record)
        if sub in (record_stream.SUB_CP_PLAIN, record_stream.SUB_CP_SEALED, 0x25, 0x45):
            continue
        if len(body) >= 4 and body[2:4] == record_stream.CODEC_SYNC:
            kind = ("image", None)
            clear = b""
        elif sub & 0x08:
            opened = opener.open(sub, seq, body)
            if not opened:
                continue
            clear = opened[1]
            kind = ("sealed", len(clear))
        else:
            clear = body
            kind = (aux, len(clear))
        if kind in out:
            out[kind] = (out[kind][0], out[kind][1] + 1)
        else:
            out[kind] = (clear, 1)
    return out


def report_video(ref, cmp_):
    print("\n== video-plane records (ring descriptor, frame opener, decoder config, report)")
    only_ref = sorted(set(ref) - set(cmp_), key=str)
    only_cmp = sorted(set(cmp_) - set(ref), key=str)
    for kind in only_ref:
        print(f"  ONLY REFERENCE sends {kind}  x{ref[kind][1]}")
    for kind in only_cmp:
        print(f"  ONLY COMPARED sends  {kind}  x{cmp_[kind][1]}")
    for kind in sorted(set(ref) & set(cmp_), key=str):
        if kind[0] == "image":
            print(f"  {kind}: ref x{ref[kind][1]}, vino x{cmp_[kind][1]} (bodies not compared)")
            continue
        a, na = ref[kind]
        b, nb = cmp_[kind]
        diffs = [o for o in range(min(len(a), len(b))) if a[o] != b[o]]
        if not diffs and len(a) == len(b):
            print(f"  {kind}: IDENTICAL  (ref x{na}, vino x{nb})")
            continue
        print(f"  {kind}: ref x{na}, vino x{nb}")
        if len(a) != len(b):
            print(f"     LENGTH {len(a)} vs {len(b)}")
        for off in diffs[:24]:
            print(f"       off{off:<3} ref={a[off]:#04x}  vino={b[off]:#04x}")
        if len(diffs) > 24:
            print(f"       ... {len(diffs) - 24} more")


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("reference")
    ap.add_argument("reference_keys")
    ap.add_argument("compared")
    ap.add_argument("compared_keys")
    ap.add_argument("--endpoint", type=int, default=2)
    ap.add_argument("--video", action="store_true", help="compare the video plane instead")
    args = ap.parse_args()

    if args.video:
        report_video(
            video_records(args.reference, args.reference_keys, args.endpoint),
            video_records(args.compared, args.compared_keys, args.endpoint),
        )
        return 0

    ref = plaintexts(args.reference, args.reference_keys, args.endpoint)
    cmp_ = plaintexts(args.compared, args.compared_keys, args.endpoint)

    print(f"reference: {len(ref)} distinct (id,sub)   compared: {len(cmp_)} distinct")

    only_ref = sorted(set(ref) - set(cmp_))
    only_cmp = sorted(set(cmp_) - set(ref))
    if only_ref:
        print("\n== messages the reference sends and the compared sender never does")
        for i, s in only_ref:
            print(f"  id={i:#06x} sub={s:#04x}  x{ref[(i, s)][1]}")
    if only_cmp:
        print("\n== messages the compared sender sends and the reference never does")
        for i, s in only_cmp:
            print(f"  id={i:#06x} sub={s:#04x}  x{cmp_[(i, s)][1]}")

    print("\n== byte differences in messages both send")
    clean = 0
    for ident in sorted(set(ref) & set(cmp_)):
        a, na = ref[ident]
        b, nb = cmp_[ident]
        notes = []
        if len(a) != len(b):
            notes.append(f"LENGTH {len(a)} vs {len(b)}")
        real, excused = [], []
        for off in range(min(len(a), len(b))):
            if a[off] == b[off]:
                continue
            (excused if off in VOLATILE or off >= TAIL_FROM else real).append(off)
        if not real and not notes:
            clean += 1
            continue
        i, s = ident
        print(f"\n  id={i:#06x} sub={s:#04x}  (ref x{na}, cmp x{nb})")
        for n in notes:
            print(f"     {n}")
        if real:
            print(f"     {len(real)} differing byte(s) outside the volatile fields:")
            for off in real:
                print(f"       off{off:<3} ref={a[off]:#04x}  vino={b[off]:#04x}")
        if excused:
            print(f"     ({len(excused)} in counter/head/tail, not reported)")
    print(f"\n  {clean} message type(s) byte-identical outside the volatile fields")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
