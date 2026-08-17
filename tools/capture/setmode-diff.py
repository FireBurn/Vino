#!/usr/bin/env python3
"""Compare the set-mode records two sinks produced for the same resolutions.

The question this answers: is id=0x48 sub=0x22 a pure function of the mode, or does it carry
something specific to the sink or the socket? If every shared mode gives a byte-identical payload
across two different sinks, a driver can build the record from the mode alone.

Bytes that legitimately differ every time are excluded by position: the inner counter, and the
trailing bytes that carry per-message opaque content.

  ./setmode-diff.py <decrypted-a.txt> <journal-a.tsv> <decrypted-b.txt> <journal-b.tsv>
"""
import re
import sys

# Inner counter at 4..6 advances with every message; the last 22 bytes are the per-message tail.
SKIP = set(range(4, 6))
TAIL = 22


def load(dec_path, journal_path):
    steps = []
    for line in open(journal_path):
        f = line.rstrip("\n").split("\t")
        if len(f) >= 4 and f[2] == "begin":
            steps.append((float(f[0]), f[3]))
    steps.sort()

    def mode_for(ts):
        best = None
        for t, label in steps:
            if t <= ts:
                best = label
            else:
                break
        return best or "?"

    out = {}
    lines = open(dec_path).read().splitlines()
    for i, line in enumerate(lines):
        if "0x0048/0x0022" not in line:
            continue
        ts = float(line.split()[0])
        for j in range(i + 1, min(i + 3, len(lines))):
            m = re.match(r"\s+PT ([0-9a-f]+)", lines[j])
            if m:
                # Normalise the label: the sweep names carry a refresh that the sink may not honour.
                out[mode_for(ts).split("@")[0]] = bytes.fromhex(m.group(1))
                break
    return out


a = load(sys.argv[1], sys.argv[2])
b = load(sys.argv[3], sys.argv[4])
shared = sorted(set(a) & set(b), key=lambda m: -int(m.split("x")[0]))
print(f"A: {len(a)} record(s)   B: {len(b)} record(s)   shared modes: {len(shared)}\n")

identical = 0
for mode in shared:
    pa, pb = a[mode], b[mode]
    body = max(len(pa), len(pb)) - TAIL
    diffs = [
        i
        for i in range(body)
        if i not in SKIP and (i >= len(pa) or i >= len(pb) or pa[i] != pb[i])
    ]
    if not diffs:
        identical += 1
        print(f"{mode:<12} IDENTICAL")
    else:
        print(f"{mode:<12} differs at {len(diffs)} byte(s): {diffs}")
        for i in diffs:
            print(f"{'':<14}off {i:<3} A={pa[i]:#04x} B={pb[i]:#04x}")

print(f"\n{identical}/{len(shared)} shared modes byte-identical outside the counter and tail")
