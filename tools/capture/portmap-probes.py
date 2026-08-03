#!/usr/bin/env python3
"""Read the per-connector presence probes out of a decrypted DLM control-plane dump.

On Ridge (D6000) the `id=0x15 sub=0x20` presence probe carries the HEAD selector in payload
byte 22, and there are two heads, so that byte is only ever 0 or 1. Navarro (DL7400) has FOUR
DisplayPort connectors but only two video endpoints, so the question this tool exists to answer is
whether that same byte enumerates PORTS rather than heads -- and, if it does, what the dock replies
for a port with a monitor on it versus an empty one.

Input is the `--full` output of decrypt-dlm-cp.py; it pairs each probe with the reply that carries
the same inner counter, then groups by the selector byte.

  tools/capture/decrypt-dlm-cp.py wire.pcapng keys.json --device N --full > full.txt
  tools/capture/portmap-probes.py full.txt [journal.tsv]

With a journal the probes are bucketed by capture step, which is what turns "these are the replies"
into "this is what changed when the cable moved".
"""
import re, sys, collections

SEL = 22          # payload byte that selects the connector on Ridge; the hypothesis under test

# The wire field is printed as `4/0x0024/  178` -- padded, so it contains spaces and cannot be
# matched with \S+.
hdr = re.compile(r'^(\d+\.\d+)\s+(\S+)\s+(OUT|IN)\s+(\d+)\s+'
                 r'\d+/0x[0-9a-f]{4}/\s*\d+\s+'
                 r'0x([0-9a-f]{4})/0x([0-9a-f]{4})/\s*(\d+)')

def load(path):
    """Yield (ts, dir, inner_id, inner_sub, ctr, payload_bytes) for every decrypted frame."""
    out, cur = [], None
    for line in open(path, errors="replace"):
        m = hdr.match(line)
        if m:
            ts, _dev, dr, _ln, iid, isub, ctr = m.groups()
            cur = (float(ts), dr, int(iid, 16), int(isub, 16), int(ctr))
            continue
        if cur and line.startswith("    PT "):
            out.append(cur + (bytes.fromhex(line[7:].strip()),))
            cur = None
    return out

def steps(journal):
    """[(t_start, label)] from a portmark journal, so probes can be attributed to a cable move."""
    marks = []
    for line in open(journal, errors="replace"):
        f = line.rstrip("\n").split("\t")
        if len(f) >= 3 and f[1] == "mark":
            marks.append((float(f[0]), f[2]))
        elif len(f) >= 2 and f[1].startswith(("begin:", "end:")):
            marks.append((float(f[0]), f[1]))
    return sorted(marks)

def step_of(marks, ts):
    lab = "(before first mark)"
    for t, l in marks:
        if t <= ts:
            lab = l
        else:
            break
    return lab

def main():
    frames = load(sys.argv[1])
    marks = steps(sys.argv[2]) if len(sys.argv) > 2 else []

    # A reply is matched to its probe by the inner counter, which the dock echoes.
    replies = {(f[4]): f for f in frames if f[1] == "IN"}

    rows = []
    for ts, dr, iid, isub, ctr, pt in frames:
        if dr != "OUT" or iid != 0x15 or isub != 0x20 or len(pt) <= SEL:
            continue
        r = replies.get(ctr)
        rows.append((ts, pt[SEL], r[5] if r else None, step_of(marks, ts) if marks else ""))

    print(f"# {len(rows)} presence probe(s) (id=0x15 sub=0x20), selector = payload byte {SEL}")
    sels = collections.Counter(s for _, s, _, _ in rows)
    print(f"# selector values seen: {dict(sorted(sels.items()))}")
    if not sels:
        print("# none -- the selector hypothesis is wrong for this capture, or nothing decrypted")
        return

    # The interesting field is the reply's status word. On Ridge the rich (id=0x44) reply carries
    # the downstream state; a bare one means the dock did not route the probe to an EDID handler.
    print(f"\n{'step':<26} {'sel':>3}  {'n':>4}  reply status words (byte 21..26)")
    by = collections.defaultdict(lambda: collections.Counter())
    for ts, sel, rpt, step in rows:
        word = rpt[21:27].hex() if rpt and len(rpt) >= 27 else "(no reply decrypted)"
        by[(step, sel)][word] += 1
    for (step, sel), words in sorted(by.items()):
        tot = sum(words.values())
        w = "  ".join(f"{k}x{v}" for k, v in words.most_common(4))
        print(f"{step:<26} {sel:>3}  {tot:>4}  {w}")

if __name__ == "__main__":
    main()
