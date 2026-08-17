#!/usr/bin/env python3
"""Pair every captured set-mode record with the resolution that provoked it.

DLM re-issues id=0x48 sub=0x22 only on a real resolution change, so each record in the sweep
belongs to the step whose `begin` most recently preceded it.
"""
import re
import sys

dec_path, journal_path = sys.argv[1], sys.argv[2]

# journal: epoch \t iso \t step \t mode
steps = []
for line in open(journal_path):
    f = line.rstrip("\n").split("\t")
    if len(f) < 4 or f[0] == "epoch":
        continue
    if f[2] == "begin":
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


records = []
lines = open(dec_path).read().splitlines()
for i, line in enumerate(lines):
    if "0x0048/0x0022" not in line:
        continue
    ts = float(line.split()[0])
    # payload is on the following "    PT <hex>" line
    for j in range(i + 1, min(i + 3, len(lines))):
        m = re.match(r"\s+PT ([0-9a-f]+)", lines[j])
        if m:
            records.append((ts, mode_for(ts), bytes.fromhex(m.group(1))))
            break

print(f"{len(records)} set-mode record(s); payload {len(records[0][2])} B\n")

hdr = (
    f"{'mode':<18} {'off22':>6} {'off23':>6} {'off42':>7} {'off44':>6} "
    f"{'off48':>7} {'off66':>7} {'off68':>7} {'off69':>6} {'off70':>7} {'off72':>7}"
)
print(hdr)
print("-" * len(hdr))


def u16(b, o):
    return int.from_bytes(b[o : o + 2], "little") if o + 2 <= len(b) else -1


def u8(b, o):
    return b[o] if o < len(b) else -1


for ts, mode, pt in records:
    print(
        f"{mode:<18} {u8(pt,22):>6} {u8(pt,23):>6} {u16(pt,42):#07x} {u16(pt,44):>6} "
        f"{u16(pt,48):>7} {u16(pt,66):#07x} {u16(pt,68):#07x} {u8(pt,69):>6} "
        f"{u16(pt,70):#07x} {u16(pt,72):>7}"
    )

print("\nraw payloads:")
for ts, mode, pt in records:
    print(f"  {mode:<18} {pt.hex()}")
