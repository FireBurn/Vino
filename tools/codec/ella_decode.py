#!/usr/bin/env python3
"""The DL-3x00 strip grammar, as a decoder, and a whole-corpus check of it.

A strip encoder can put records of exactly the right size on the wire and still be speaking a
dialect the dock does not read, so length is no evidence at all. This decodes captured strips with
the grammar the driver encodes with and reports how many decode cleanly. Clean means the decode
consumes every bit up to the region boundary with only zero padding left over -- the regions are
byte-padded, and requiring an exact landing scores every hypothesis at about 6%, including the
right one.

Run it against a vendor capture: anything short of 100% is a hole in the grammar.

  ella_decode.py CAP.pcapng [strip count]
"""
import sys
sys.path.insert(0, "/home/fireburn/Downloads/dl-scripts/vino/tools/codec")
from usbmon_read import iter_transfers

COEFFS = 64
AC_CMAX, CHROMA_AC_CMAX, DC_CMAX = 9, 10, 10

class R:
    __slots__ = ("b", "p")
    def __init__(s, b): s.b, s.p = b, 0
    def bit(s):
        if s.p >= len(s.b): raise EOFError
        v = s.b[s.p]; s.p += 1; return v

def bits(bs):
    out = bytearray()
    for b in bs:
        for i in range(8): out.append((b >> i) & 1)
    return out

def esc_after_one(r, cmax):
    """A magnitude escape whose leading unary one has already been consumed.

    The payload is `offset ++ sign` and, like every interleaved payload on this dock, its first
    transmitted bit is the least significant. At the codebook maximum there is no terminator.
    """
    pay = [r.bit()]
    while len(pay) < cmax:
        if not r.bit():
            break
        pay.append(r.bit())
    v = payload(pay)
    off, sign = v >> 1, v & 1
    return ((1 << (len(pay) - 1)) + off) * (1 if sign else -1)

def esc(r, cmax):
    if not r.bit(): return 0
    return esc_after_one(r, cmax)

def payload(pay):
    """The interleaved payload, least significant bit first -- see `esc_after_one`."""
    v = 0
    for i, b in enumerate(pay):
        v |= b << i
    return v

def n_chroma(r):
    """A chroma plane's last significant coefficient. The leading unary one is already consumed."""
    pay = [r.bit()]
    while r.bit():
        pay.append(r.bit())
    return ((1 << len(pay)) - 1) + payload(pay)

def n_luma(r):
    """A luma plane's last significant coefficient; six ones means a flat block."""
    pay = []
    while len(pay) < 6:
        if not r.bit():
            return (64 - (1 << len(pay))) - payload(pay)
        pay.append(r.bit())
    r.bit()
    return 0

def unit(r):
    if r.bit():
        lcr = n_chroma(r)
        if r.bit(): return lcr, n_chroma(r), n_luma(r)
        return lcr, 0, n_luma(r)
    if r.bit(): return 0, n_chroma(r), n_luma(r)
    return 0, 0, n_luma(r)

def decode_main(s):
    r = R(bits(s[16:]))
    lasts = [unit(r) for _ in range(16)]
    for _ in range(16):
        esc(r, DC_CMAX); esc(r, DC_CMAX); esc(r, DC_CMAX)
    return lasts, r

def decode_ac_row(body, blocks):
    """blocks is a list of (lcr, lcb, ly). Returns the reader for slack checking."""
    r = R(bits(body))
    for (lcr, lcb, ly) in blocks:
        for last, cmax in ((lcr, CHROMA_AC_CMAX), (lcb, CHROMA_AC_CMAX), (ly, AC_CMAX)):
            for _ in range(1, last + 1):
                esc(r, cmax)
    return r

def slack_ok(r):
    """Every remaining bit must be zero padding."""
    return all(b == 0 for b in r.b[r.p:])

def strips_from(path, want):
    out = []
    stream = bytearray()
    for _d, _e, p in iter_transfers(path, endpoint=2):
        stream += p
        if len(stream) > 40_000_000: break
    off = 0
    while off + 16 <= len(stream) and len(out) < want:
        size = int.from_bytes(stream[off+2:off+4], "little")
        stride = size + 4
        if stride < 16 or stride % 16 or off + stride > len(stream): break
        body = stream[off+16:off+stride]
        if len(body) >= 4 and body[2:4] == b"\x01\x28":
            p = 0
            while p + 2 <= len(body) and len(out) < want:
                ln = int.from_bytes(body[p:p+2], "little")
                if ln == 0 or p + 2 + ln > len(body): break
                out.append(bytes(body[p+2:p+2+ln])); p += 2 + ln
        off += stride
    return out

if __name__ == "__main__":
    cap = sys.argv[1]; want = int(sys.argv[2]) if len(sys.argv) > 2 else 6000
    S = strips_from(cap, want)
    print(f"{len(S)} strips")
    stats = {"main ok": 0, "main fail": 0, "row0 ok": 0, "row0 fail": 0,
             "row1 ok": 0, "row1 fail": 0, "whole ok": 0, "no ac": 0}
    for s in S:
        if len(s) < 16: continue
        w18 = int.from_bytes(s[10:12], "little")
        w1c = int.from_bytes(s[12:14], "little")
        if not (16 <= w18 <= w1c <= len(s)): continue
        try:
            lasts, rm = decode_main(s[:w18])
        except (EOFError, IndexError):
            stats["main fail"] += 1; continue
        if not slack_ok(rm):
            stats["main fail"] += 1; continue
        stats["main ok"] += 1
        if w18 == w1c == len(s):
            stats["no ac"] += 1; stats["whole ok"] += 1; continue
        ok = True
        for name, body, blocks in (("row0", s[w18:w1c], lasts[:8]),
                                   ("row1", s[w1c:], lasts[8:])):
            try:
                r = decode_ac_row(body, blocks)
                good = slack_ok(r)
            except (EOFError, IndexError):
                good = False
                r = None
            stats[f"{name} {'ok' if good else 'fail'}"] += 1
            if not good:
                ok = False
        if ok: stats["whole ok"] += 1
    for k, v in stats.items(): print(f"  {k:<10} {v}")
    tot = stats["main ok"] or 1
    print(f"\nwhole strips: {100*stats['whole ok']/tot:.1f}% of decodable-main strips")
