#!/usr/bin/env python3
"""Spawn DLM under hook-drbg-trace.js and match its DRBG output against the wire.

    sudo -E python3 tools/capture/run-drbg-trace.py --wire OUT.pcapng [--secs 45]

Answers the only question that matters about the opaque fields: do the generator's bytes reach the
wire unchanged, or is something applied to them on the way? Knowing which generator produces them
does not answer that.

The oracle needs no keys. `rtx` sits unencrypted at offset 28 of the plaintext `sub=0x04` AKE_Init,
and `rn` at the same offset of the plaintext LC_Init, so searching the captured generator output
for those eight bytes settles it. A hit means raw output and nothing to derive; a miss means a
transformation, and the recorded frames name the function to look at next.

Run with `displaylink-driver.service` MASKED and start the wire capture separately -- dumpcap drops
privileges, so its output has to live under $HOME.
"""
import argparse
import importlib.util
import json
import os
import sys
import time

import frida

HERE = os.path.dirname(os.path.abspath(__file__))
DLM = "/opt/displaylink/DisplayLinkManager"

ap = argparse.ArgumentParser()
ap.add_argument("--secs", type=float, default=45.0)
ap.add_argument("--wire", help="pcapng captured over the same window, to match against")
ap.add_argument("--out", metavar="LOG", help="write the raw generator stream here")
ap.add_argument("--attach", action="store_true",
                help="hook a running DLM. Spawning under Frida does not reliably reach a "
                     "session on this dock; attach to a DLM that is already driving it and "
                     "force a fresh AKE with a USB re-authorise while hooked.")
args = ap.parse_args()

js = open(os.path.join(HERE, "hook-drbg-trace.js")).read()
dev = frida.get_local_device()
if args.attach:
    procs = [p for p in dev.enumerate_processes() if "DisplayLinkManager" in p.name]
    if not procs:
        sys.exit("DLM not running; start it by hand first")
    pid = procs[0].pid
    print(f"[*] attaching to DLM pid={pid} for {args.secs}s")
    session = dev.attach(pid)
    spawned = None
else:
    print(f"[*] spawning {DLM} under Frida for {args.secs}s")
    spawned = dev.spawn([DLM], cwd=os.path.dirname(DLM))
    session = dev.attach(spawned)

outputs = []
enters = []
first_frames = {}


def on_msg(m, _data):
    if m.get("type") == "error":
        print("  [js error]", m.get("description"))
        return
    p = m.get("payload") or {}
    if p.get("kind") == "ready":
        print(f"[*] hooked, base {p['base']}")
    elif p.get("kind") == "drbg":
        blk = bytes(p["block"]) if p.get("block") else b""
        outputs.append((p["site"], p["n"], blk))
        enters.append((p["site"], p.get("dest"), p.get("remaining"), blk.hex()))
        if p.get("frames") and p["site"] not in first_frames:
            first_frames[p["site"]] = p["frames"]


script = session.create_script(js)
script.on("message", on_msg)
script.load()
if spawned is not None:
    dev.resume(spawned)
try:
    time.sleep(args.secs)
except KeyboardInterrupt:
    pass

print(f"\n=== {len(enters)} DRBG generate ENTRIES, {len(outputs)} with readable output ===")
for e in enters[:10]:
    print(f"   {e[0]}  dest={e[1]} remaining={e[2]}  block={e[3]}")
by_site = {}
for site, _n, b in outputs:
    by_site.setdefault(site, []).append(b)
for site, bufs in by_site.items():
    lens = sorted({len(b) for b in bufs})
    print(f"  {site}: {len(bufs)} calls, output lengths {lens}")
for site, fr in first_frames.items():
    print(f"\n=== first-call frames: {site} ===")
    for f in fr:
        print(f"  {f}")

if args.out:
    with open(args.out, "w") as fh:
        for site, n, b in outputs:
            fh.write(json.dumps({"site": site, "n": n, "hex": b.hex()}) + "\n")
    print(f"\n[*] generator stream in {args.out}")

if args.wire:
    spec = importlib.util.spec_from_file_location("rs", os.path.join(HERE, "record-stream.py"))
    rs = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(rs)
    pool = b"".join(b for _s, _n, b in outputs)
    print(f"\n=== matching against {args.wire} ({len(pool)} generator bytes pooled) ===")
    found = 0
    for rec in rs.records(args.wire, 2):
        _t, sub, _aux, _seq, body = rs.fields(rec)
        if sub != 0x04 or len(body) < 48:
            continue
        isub = int.from_bytes(body[2:4], "little")
        if isub != 0x10 or body[27] not in (0x02, 0x09):
            continue
        name = "AKE_Init rtx" if body[27] == 0x02 else "LC_Init rn"
        field = bytes(body[28:36])
        where = pool.find(field)
        print(f"  {name}: {field.hex(' ')} -> "
              + (f"FOUND at generator byte {where}" if where >= 0 else "NOT in generator output"))
        found += 1 if where >= 0 else 0
    print(f"\n{found} of the plaintext nonces appear verbatim in the generator's output")

try:
    session.detach()
    if spawned is not None:
        dev.kill(spawned)
except Exception:
    pass
