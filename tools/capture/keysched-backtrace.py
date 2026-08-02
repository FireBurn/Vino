#!/usr/bin/env python3
"""Find where DisplayLinkManager derives its per-stream keys, by backtracing the AES calls.

The DL-7000 platform runs one HDCP handshake and ends up with three different sealing keys: one
for the control plane and one per video head.  They are not transmitted and are not a simple
function of the link key, so both ends must compute them locally -- which means the derivation is
only visible from inside DLM.

This attaches to a live DLM, hooks the AES block function every other tool here uses, and records
a **backtrace the first time each distinct key is seen**.  Backtracing every call would be far too
expensive on a function this hot (and stalls DLM into a watchdog restart), but the first
appearance of each key is a handful of events and points straight at the key schedule.

Feed the printed offsets to Ghidra.

  sudo env PYTHONPATH=... python3 keysched-backtrace.py --secs 60 --out sched.json
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
const AES_FUNC = __HOOK__;
const KEY_AT   = __KEY_AT__;   // offset of the 16-byte key from the hooked register
const KEY_REG  = "__KEY_REG__";

function hx(b){const u=new Uint8Array(b);let s="";for(let i=0;i<u.length;i++)s+=u[i].toString(16).padStart(2,"0");return s;}
function rd(p,n){ try{ const b=p.readByteArray(n); return b?hx(b):""; }catch(e){ return ""; } }

// One backtrace per distinct key, never per call: this function is hot enough that unconditional
// unwinding stalls DLM into a watchdog restart.
const seenKey = new Set();

Interceptor.attach(base.add(AES_FUNC), {
  onEnter(args){
    try{
      // Round key is inline at rdi+16 on 3.4.26; see decode-modeset-live.py.
      const key = rd(this.context[KEY_REG].add(KEY_AT), 16);
      if(!key || key.length !== 32 || seenKey.has(key)) return;
      seenKey.add(key);
      // The whole key-source object: the riv lives at +0x30 and the surrounding words are worth
      // having, since relating the five per-sub keys to each other is cheaper than finding the
      // code that writes them.
      const blob = rd(this.context[KEY_REG], 0x50);
      const input = rd(this.context.rdx, 16);
      // The sealer factory takes the per-stream selector in ecx; capturing it maps each key to
      // the stream it seals.
      let idx = null;
      try{ idx = this.context.rcx.toInt32() & 0xffffffff; }catch(e){}
      let frames = Thread.backtrace(this.context, Backtracer.ACCURATE);
      // The AES primitives are leaves compiled without frame pointers, so the accurate unwinder
      // often yields a single frame. Fuzzy scanning of the stack recovers the callers that matter
      // here; they are printed as candidates, not as gospel.
      if(frames.length < 3){
        const fz = Thread.backtrace(this.context, Backtracer.FUZZY);
        if(fz.length > frames.length) frames = fz;
      }
      const bt = frames.map(a => {
        const off = a.sub(base);
        const sym = DebugSymbol.fromAddress(a);
        return { off: "0x" + off.toString(16),
                 name: (sym && sym.name) ? sym.name : null,
                 mod:  (sym && sym.moduleName) ? sym.moduleName : null };
      });
      // The return address is read straight off the frame and is reliable, unlike the fuzzy
      // frames above -- it is the caller that actually matters here.
      let ret = null;
      try{ ret = "0x" + this.returnAddress.sub(base).toString(16); }catch(e){}
      send({ s: "sched", ts: Date.now()/1000, key: key, input: input, ret: ret, idx: idx, blob: blob, bt: bt });
    }catch(e){}
  }
});
send({s:"ready"});
"""


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--secs", type=float, default=60.0)
    ap.add_argument("--out", default="keysched.json")
    ap.add_argument("--process", default="DisplayLinkManager")
    ap.add_argument("--hook", default="0x269dd0", help="module offset to hook")
    ap.add_argument("--key-reg", default="rdi", help="register holding the key base")
    ap.add_argument("--key-at", default="16", help="byte offset of the key from that register")
    args = ap.parse_args()

    rows: list[dict] = []

    def on_message(msg, _data):
        if msg.get("type") != "send":
            return
        p = msg["payload"]
        if p.get("s") == "ready":
            print("frida attached", flush=True)
        elif p.get("s") == "sched":
            rows.append(p)
            print(f"  key {p['key']}  idx={p.get('idx')}  ret={p.get('ret')}", flush=True)

    js = (JS.replace("__HOOK__", args.hook)
            .replace("__KEY_AT__", args.key_at)
            .replace("__KEY_REG__", args.key_reg))
    session = frida.attach(args.process)
    script = session.create_script(js)
    script.on("message", on_message)
    script.load()
    time.sleep(args.secs)
    try:
        session.detach()
    except frida.Error:
        pass

    json.dump(rows, open(args.out, "w"), indent=1)
    print(f"\n{len(rows)} distinct keys -> {args.out}")
    for r in rows:
        print(f"\n== key {r['key']}")
        for i, f in enumerate(r["bt"][:12]):
            nm = f["name"] or ""
            print(f"   #{i:<2} {f['off']:>12}  {nm}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
