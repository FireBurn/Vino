#!/usr/bin/env python3
"""Spawn DLM under Frida with hook-setupvideo.js and print every set-mode call.

    sudo -E python3 tools/capture/run-setupvideo-hook.py [--secs 60] [--attach]

Spawns by default, so the bring-up mode sets are caught -- those are the only ones that happen
without disturbing an existing desktop. `--attach` hooks an already-running DLM instead, in which
case you must trigger a *resolution* change yourself.

Run with `displaylink-driver.service` MASKED; udev bounces the unit on every dock re-enumeration
and would kill the hook. See feedback_run_dlm_by_hand_for_captures.
"""
import argparse
import os
import sys
import time

import frida

HERE = os.path.dirname(os.path.abspath(__file__))
DLM = "/opt/displaylink/DisplayLinkManager"

ap = argparse.ArgumentParser()
ap.add_argument("--secs", type=float, default=60.0)
ap.add_argument("--attach", action="store_true", help="hook a running DLM instead of spawning")
ap.add_argument("--out", metavar="LOG", help="also write the hook output here")
args = ap.parse_args()

js = open(os.path.join(HERE, "hook-setupvideo.js")).read()
dev = frida.get_local_device()

spawned = None
if args.attach:
    procs = [p for p in dev.enumerate_processes() if "DisplayLinkManager" in p.name]
    if not procs:
        sys.exit("DLM not running; drop --attach to spawn it")
    pid = procs[0].pid
    print(f"[*] attaching to DLM pid={pid} for {args.secs}s -- change a resolution now")
    session = dev.attach(pid)
else:
    print(f"[*] spawning {DLM} under Frida for {args.secs}s")
    spawned = dev.spawn([DLM], cwd=os.path.dirname(DLM))
    session = dev.attach(spawned)

lines = []


def on_msg(m, _data):
    if m.get("type") == "error":
        print("  [js error]", m.get("description"))
        return
    text = m.get("payload")
    if text is None:
        return
    print(text, flush=True)
    lines.append(str(text))


def on_log(_level, text):
    print(text, flush=True)
    lines.append(str(text))


script = session.create_script(js)
script.on("message", on_msg)
# frida >= 16 prints console.log itself unless a log handler is installed, which would keep the
# hook output out of --out entirely.
script.set_log_handler(on_log)
script.load()
if spawned is not None:
    dev.resume(spawned)

try:
    time.sleep(args.secs)
finally:
    try:
        session.detach()
    except Exception:
        pass
    if spawned is not None:
        try:
            dev.kill(spawned)
        except Exception:
            pass

print(f"\n[*] {len([l for l in lines if 'setupVideo #' in l])} setupVideo call(s) captured")
if args.out:
    with open(args.out, "w") as fh:
        fh.write("\n".join(lines) + "\n")
    print(f"[*] wrote {args.out}")
