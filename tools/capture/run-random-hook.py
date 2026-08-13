#!/usr/bin/env python3
"""Spawn DLM under Frida with hook-random.js and report which generator fills the CP tails.

    sudo -E python3 tools/capture/run-random-hook.py [--secs 45] [--out LOG]

Run with `displaylink-driver.service` MASKED and DLM started by nothing else: udev bounces the
unit on every dock re-enumeration, and a hook bound to the old pid then records nothing while
still looking healthy.

Counting settles it on its own. A status poll carries a ten-byte tail and goes out every 250 ms,
so whichever generator is behind it fires in bursts of ten at four hertz; the others stay flat.
The per-second counts are printed for that reason rather than as progress.

The backtraces are the part that chains it back. Frames inside DLM print as module-relative RVAs,
which is what a Ghidra address wants (+0x100000), and the calls arrive through vtables and thunks
so the frame list is the only thing that names the builder that asked for the bytes.
"""
import argparse
import collections
import json
import os
import sys
import time

import frida

HERE = os.path.dirname(os.path.abspath(__file__))
DLM = "/opt/displaylink/DisplayLinkManager"

ap = argparse.ArgumentParser()
ap.add_argument("--secs", type=float, default=45.0)
ap.add_argument("--attach", action="store_true", help="hook a running DLM instead of spawning")
ap.add_argument("--out", metavar="LOG", help="write the raw message stream here as JSON lines")
args = ap.parse_args()

js = open(os.path.join(HERE, "hook-random.js")).read()
dev = frida.get_local_device()

if args.attach:
    procs = [p for p in dev.enumerate_processes() if "DisplayLinkManager" in p.name]
    if not procs:
        sys.exit("DLM not running; drop --attach to spawn it")
    pid = procs[0].pid
    print(f"[*] attaching to DLM pid={pid} for {args.secs}s")
    session = dev.attach(pid)
    spawned = None
else:
    print(f"[*] spawning {DLM} under Frida for {args.secs}s")
    spawned = dev.spawn([DLM], cwd=os.path.dirname(DLM))
    session = dev.attach(spawned)

log = open(args.out, "w") if args.out else None
fills = []
traces = collections.OrderedDict()
last_counts = {}


def on_msg(m, _data):
    global last_counts
    if m.get("type") == "error":
        print("  [js error]", m.get("description"))
        return
    p = m.get("payload") or {}
    if log:
        log.write(json.dumps(p) + "\n")
    kind = p.get("kind")
    if kind == "ready":
        print(f"[*] hooked, module base {p['base']}")
    elif kind == "counts":
        cur = p["counts"]
        delta = {k: cur[k] - last_counts.get(k, 0) for k in cur}
        delta = {k: v for k, v in delta.items() if v}
        last_counts = dict(cur)
        if delta:
            print("    per second:", delta)
    elif kind == "fill":
        fills.append(bytes(p["bytes"]))
    elif kind == "trace":
        traces.setdefault(p["tag"], []).append(p["frames"])
    elif kind == "error":
        print(f"  [hook {p['what']}] {p['msg']}")


script = session.create_script(js)
script.on("message", on_msg)
script.load()
if spawned is not None:
    dev.resume(spawned)

try:
    time.sleep(args.secs)
except KeyboardInterrupt:
    pass

print()
print("=== totals ===")
for k, v in sorted(last_counts.items(), key=lambda kv: -kv[1]):
    print(f"  {k:<34} {v}")

if fills:
    print(f"\n=== byte filler produced {len(fills)} buffers ===")
    for b in fills[:8]:
        print(f"  {len(b):3d} B  {b.hex(' ')}")

for tag, frames in traces.items():
    print(f"\n=== backtrace: {tag} ===")
    for f in frames[0]:
        print(f"  {f}")

if log:
    log.close()
    print(f"\n[*] raw stream in {args.out}")

try:
    session.detach()
    if spawned is not None:
        dev.kill(spawned)
except Exception:
    pass
