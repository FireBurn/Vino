#!/usr/bin/env python3
"""Catch the code that writes a DisplayLink sealing key into its key-source object.

`keysched-backtrace.py` establishes that DLM builds one sealer per wire sub and that the factory
only *copies* an already-derived key out of a source object (key inline at +0x18, riv at +0x30).
The five keys are mutually independent, so the derivation is a real key schedule and the only way
to see it is to watch the moment the key lands.

The object is constructed at 0x85c560, which allocates the key and riv fields and returns before
anything fills them.  Hook that, take the object pointer, and arm a hardware watchpoint on
`obj+0x18`; the trap fires in the writer.

Needs a **cold** session -- the schedule is dormant on a warm dock.  Force one with
`echo 0 > /sys/bus/usb/devices/<dev>/authorized; sleep 6; echo 1 > ...`.
"""

from __future__ import annotations

import argparse
import json
import sys
import time

import frida

JS = r"""
"use strict";
const dlm = Process.findModuleByName("DisplayLinkManager");
const base = dlm.base;
const CTOR = __CTOR__;        // builds the key-source object
const KEY_AT = __KEY_AT__;    // key field, inline

function hx(b){const u=new Uint8Array(b);let s="";for(let i=0;i<u.length;i++)s+=u[i].toString(16).padStart(2,"0");return s;}
function rd(p,n){ try{ const b=p.readByteArray(n); return b?hx(b):""; }catch(e){ return ""; } }

let armed = 0;
const MAX_ARMED = 3;   // four watchpoint registers exist on x86-64; leave one spare

Process.setExceptionHandler(function (details) {
  if (details.type !== "breakpoint" && details.type !== "single-step") return false;
  try {
    const pc = details.context.pc;
    send({ s: "write", pc: "0x" + pc.sub(base).toString(16),
           ctx: { rax: details.context.rax.toString(16),
                  rdi: details.context.rdi.toString(16),
                  rsi: details.context.rsi.toString(16) } });
  } catch (e) {}
  return true;   // resume
});

Interceptor.attach(base.add(CTOR), {
  onEnter(){
    try{
      if (armed >= MAX_ARMED) return;
      // The object arrives in rdi and the ctor tail-calls, so there is no reliable onLeave and no
      // return value to read -- arm on the way in.
      const obj = this.context.rdi;
      if (obj.isNull()) return;
      const target = obj.add(KEY_AT);
      Thread.setHardwareWatchpoint(armed, target, 8, "w");
      send({ s: "armed", slot: armed, addr: target.toString(), pre: rd(target, 16) });
      armed++;
    }catch(e){ send({s:"err", m:String(e)}); }
  }
});
send({s:"ready"});
"""


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--secs", type=float, default=45.0)
    ap.add_argument("--ctor", default="0x85c560")
    ap.add_argument("--key-at", default="0x18")
    ap.add_argument("--out", default="keysrc-writer.json")
    ap.add_argument("--process", default="DisplayLinkManager")
    args = ap.parse_args()

    rows: list[dict] = []

    def on_message(msg, _data):
        if msg.get("type") != "send":
            print("   [frida]", msg.get("description") or msg, flush=True)
            return
        p = msg["payload"]
        rows.append(p)
        if p.get("s") == "ready":
            print("frida attached", flush=True)
        elif p.get("s") == "armed":
            print(f"  watchpoint {p['slot']} on {p['addr']} (was {p['pre']})", flush=True)
        elif p.get("s") == "write":
            print(f"  ★ WRITER at {p['pc']}  rax={p['ctx']['rax']} rdi={p['ctx']['rdi']}", flush=True)
        elif p.get("s") == "err":
            print(f"   [err] {p['m']}", flush=True)

    session = frida.attach(args.process)
    js = JS.replace("__CTOR__", args.ctor).replace("__KEY_AT__", args.key_at)
    script = session.create_script(js)
    script.on("message", on_message)
    script.load()
    time.sleep(args.secs)
    try:
        session.detach()
    except frida.Error:
        pass

    json.dump(rows, open(args.out, "w"), indent=1)
    writers = sorted({r["pc"] for r in rows if r.get("s") == "write"})
    print(f"\ndistinct writer sites: {writers or 'none'}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
