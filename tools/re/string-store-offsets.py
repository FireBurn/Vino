#!/usr/bin/env python3
"""Decrypt DisplayLink's obfuscated string store *in address order*.

    tools/re/string-store-offsets.py <binary> [-o out.txt] [--grep REGEX] [--context N]

`re-binaries/decode-string-store.py` answers "what strings are in here" and sorts them
alphabetically. This one answers a different and more useful question: **which literals belong to
the same function**.

DisplayLink's compiler emits each `@@<base64>@@` blob inline in rodata, in source order, at a
fixed 0x38 stride. So a function's literal run reads out as a contiguous block once the blobs are
sorted by file offset -- which is how the whole `id=0x48 sub=0x22` set-mode argument list was
recovered (see `project_setmode_serializer_decompiled_20260805`). The blob addresses double as
xref anchors: `objdump -d BIN | grep '# <addr>'` lands in the function that uses one.

Note this ELF/PE has file offset == VA for the rodata segment, so the printed offset is directly
greppable in an objdump listing.

Key and container format are as `decode-string-store.py` documents them: 16 zero IV bytes then
AES-128-CBC under `7c01a5ce4fb3f107f1906e7380d76174`, PKCS#7 padded.
"""
import argparse
import base64
import re
import sys

try:
    from Crypto.Cipher import AES
except ImportError:
    sys.exit("needs pycryptodome:  pip install --user pycryptodome")

KEY = bytes.fromhex("7c01a5ce4fb3f107f1906e7380d76174")


def decode(blob: bytes):
    """One `@@...@@` payload to text, or None if it is not a string record."""
    try:
        raw = base64.b64decode(blob + b"=" * (-len(blob) % 4))
    except Exception:
        return None
    if len(raw) < 32 or len(raw) % 16:
        return None
    plain = AES.new(KEY, AES.MODE_CBC, raw[:16]).decrypt(raw[16:])
    pad = plain[-1]
    if 1 <= pad <= 16 and plain[-pad:] == bytes([pad]) * pad:
        plain = plain[:-pad]
    try:
        text = plain.decode("utf-8")
    except UnicodeDecodeError:
        return None
    if text and all(32 <= ord(c) < 127 or c in "\t\n\r" for c in text):
        return text
    return None


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("binary")
    ap.add_argument("-o", "--out")
    ap.add_argument("--grep", help="only show runs containing a match for this regex")
    ap.add_argument("--context", type=int, default=12,
                    help="literals either side of a --grep hit (default 12)")
    args = ap.parse_args()

    data = open(args.binary, "rb").read()

    rows = []
    for m in re.finditer(rb"@@([A-Za-z0-9+/=]{8,})@@", data):
        text = decode(m.group(1))
        if text is not None:
            rows.append((m.start(), text))
    rows.sort()

    print(f"{len(rows)} literals in address order", file=sys.stderr)

    if args.grep:
        pat = re.compile(args.grep, re.I)
        keep, n = set(), len(rows)
        for i, (_, text) in enumerate(rows):
            if pat.search(text):
                keep.update(range(max(0, i - args.context), min(n, i + args.context + 1)))
        emit = [(i, rows[i]) for i in sorted(keep)]
    else:
        emit = list(enumerate(rows))

    lines, prev = [], None
    for i, (off, text) in emit:
        if prev is not None and i != prev + 1:
            lines.append("")
        # A stride other than 0x38 means a different function's run has started.
        stride = "" if prev is None or i != prev + 1 else f" (+0x{off - rows[prev][0]:x})"
        lines.append(f"0x{off:08x}{stride}\t{text!r}")
        prev = i

    body = "\n".join(lines) + "\n"
    if args.out:
        open(args.out, "w").write(body)
        print(f"wrote {args.out}", file=sys.stderr)
    else:
        sys.stdout.write(body)


if __name__ == "__main__":
    main()
