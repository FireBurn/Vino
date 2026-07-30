#!/usr/bin/env python3
"""
Capture + decrypt the encrypted control-plane (type=4 sub=0x24) OUT frames from a
live DLM, to find the VideoTimingBlock (header 10 01 00 00 00) sent at mode-set.

Hooks (x86_64 DLM):
  - AES block fn @ 0x269dd0  -> (key, counter-input); CP-stream inputs have
    bytes 8-11 == 0, so riv = input[0:8]. Gives us ks + riv.
  - libusb_submit_transfer @ 0x83c20 -> EP 0x02 OUT type=4 sub=0x24 frames of ANY
    size (the stock hook caps at 64 B; the mode-set frames are 80/96/192 B).

Decrypt each frame directly: keystream block j = AES_ECB(ks, riv||00000000||(seq+j)_BE),
seq = frame[12:16] LE (per §6.1). The correct (ks,riv) is the pair that decrypts a
64 B heartbeat to '1600 75 00 ..'. Then grep all plaintext for the block marker.

Run while triggering a mode change (kscreen-doctor) on the DisplayLink output:
  sudo -E python3 decode-modeset-live.py --secs 25

⚠️ The VideoTimingBlock is CONNECT-TIME ONLY (not re-sent on runtime mode changes;
DLM keeps its dock session alive across compositor output toggles). To capture it
you must catch a fresh connect from t=0 via --spawn, which opens the dock with a
standalone DLM. On a machine actively using the dock this WEDGES the dock firmware
(blank screen; needs a USB replug/reboot to clear). Only run --spawn on an idle or
spare host, or during a maintenance window — never on a dock you're using.
"""
import frida, sys, time, struct, argparse, json
try:
    from Crypto.Cipher import AES
except ImportError:
    from Cryptodome.Cipher import AES

JS = r"""
"use strict";
const dlm = Process.findModuleByName("DisplayLinkManager");
const base = dlm.base;
const AES_FUNC = 0x269dd0, PLT_SUBMIT = 0x83c20;
function hx(b){const u=new Uint8Array(b);let s="";for(let i=0;i<u.length;i++)s+=u[i].toString(16).padStart(2,"0");return s;}

const seenKR = new Set();
function rd(p,n){ try{ const b=p.readByteArray(n); return b?hx(b):""; }catch(e){ return ""; } }
Interceptor.attach(base.add(AES_FUNC), {
  onEnter(args){ try{
    // The round key is INLINE at rdi+16 on this build (DLM 3.4.26) -- this is the access the
    // working gdb harness uses (scripts/capture-arm89-keys.gdb), and it is what recovers the
    // textbook 000102..0f self-test key. The older rdi+8-as-pointer form reads null here and
    // silently yields an empty key, which is what made the 20260726-2300 run keyless. Collect
    // both; decrypt-dlm-cp.py trials every (key,riv) pair anyway.
    this.keys = [ rd(this.context.rdi.add(16), 16) ];
    try{ this.keys.push(rd(this.context.rdi.add(8).readPointer(), 16)); }catch(e){}
    this.input = hx(this.context.rdx.readByteArray(16));
  }catch(e){} },
  onLeave(){ try{
    if(!this.input) return;
    if(this.input.slice(16,24) !== "00000000") return;   // CP-stream counter shape
    const riv = this.input.slice(0,16);
    for(const k of this.keys){
      if(!k || k.length !== 32) continue;
      const kr = k+":"+riv;
      if(seenKR.has(kr)) continue; seenKR.add(kr);
      send({s:"kr", ts:Date.now()/1000, key:k, riv:riv});
    }
  }catch(e){} }
});

Interceptor.attach(base.add(PLT_SUBMIT), {
  onEnter(args){ try{
    const t = args[0];
    const ep = t.add(0x9).readU8();
    // control-plane + per-head bulk OUT endpoints (0x02, 0x0a/0x0b/0x0c)
    if(ep !== 0x02 && ep !== 0x0a && ep !== 0x0b && ep !== 0x0c) return;
    const len = t.add(0x14).readU32();
    if(len < 16 || len > 8192) return;
    const buf = t.add(0x30).readPointer();
    const d = new Uint8Array(buf.readByteArray(len));
    const mtype = d[4]|(d[5]<<8)|(d[6]<<16)|(d[7]<<24);
    const sub = d[8]|(d[9]<<8);
    if(mtype !== 4) return;                  // all encrypted type=4 sub-ids
    send({s:"cp", ts:Date.now()/1000, ep:ep, sub:sub, len:len, full:hx(d.buffer)});
  }catch(e){} }
});
send({s:"ready"});
"""

ap = argparse.ArgumentParser(); ap.add_argument("--secs", type=float, default=25.0)
ap.add_argument("--spawn", metavar="PATH", help="spawn this DLM binary under Frida (hook live from t=0, catches connect-time VideoTimingBlock + fresh session keys)")
ap.add_argument("--out", metavar="JSON", help="dump the raw captured (ks,riv) pairs and type=4 frames here BEFORE decrypting, so a decrypt bug cannot discard a hardware run")
args = ap.parse_args()
dev = frida.get_local_device()
spawned_pid = None
if args.spawn:
    import os
    print(f"[*] spawning {args.spawn} under Frida for {args.secs}s (connect capture)")
    spawned_pid = dev.spawn([args.spawn], cwd=os.path.dirname(args.spawn) or "/opt/displaylink")
    session = dev.attach(spawned_pid); script = session.create_script(JS)
else:
    procs = [p for p in dev.enumerate_processes() if "DisplayLinkManager" in p.name]
    if not procs: sys.exit("DLM not running")
    pid = procs[0].pid
    print(f"[*] attaching to DLM pid={pid} for {args.secs}s — trigger a mode change now")
    session = dev.attach(pid); script = session.create_script(JS)

krs = []      # (key,riv) candidates
frames = []   # (len, hexstr)
def on_msg(m, data):
    if m.get("type")=="error": print("  [js]", m.get("description")); return
    p = m.get("payload") or {}
    if p.get("s")=="ready": print("  [*] hooks active")
    elif p.get("s")=="kr": krs.append((p["key"], p["riv"]));
    elif p.get("s")=="cp": frames.append((p["ep"], p["sub"], p["len"], p["full"], p.get("ts", 0.0)))
script.on("message", on_msg); script.load()
if spawned_pid is not None:
    dev.resume(spawned_pid)
time.sleep(args.secs)
try: session.detach()
except Exception: pass
if spawned_pid is not None:
    try: dev.kill(spawned_pid)
    except Exception: pass

print(f"\n[*] {len(krs)} (ks,riv) candidate(s); {len(frames)} type=4 OUT frame(s)")

# Persist the raw capture FIRST. A hardware run is expensive (it toggles the user's monitors); a
# bug anywhere in the decrypt path below must never be able to throw it away -- which is exactly
# what happened on the 20260726-2300 run (a 0-byte key from a failed rdi+8 read raised out of
# AES.new and lost 8 keys + 159 frames that were only ever in memory).
if args.out:
    with open(args.out, "w") as fh:
        json.dump({"krs": [{"key": k, "riv": r} for k, r in krs],
                   "frames": [{"ep": e, "sub": s, "len": l, "full": f, "ts": t}
                              for e, s, l, f, t in frames]},
                  fh, indent=1)
    print(f"[*] raw capture saved -> {args.out}")

# The AES hook occasionally yields a short/empty key when the round-key pointer read races; those
# are not usable and must not abort the sweep.
_bad = [ (k,r) for k,r in krs if len(k) != 32 or len(r) != 16 ]
if _bad:
    print(f"[!] dropping {len(_bad)} malformed (ks,riv) candidate(s) (short key/riv)")
krs = [ (k,r) for k,r in krs if len(k) == 32 and len(r) == 16 ]

from collections import Counter
print("    by (ep,sub,len): " + ", ".join(f"ep{e:#x}/sub{s:#x}/{l}B×{c}"
      for (e,s,l),c in Counter((e,s,l) for e,s,l,_,_ in frames).most_common(12)))
def ecb(ks, iv): return AES.new(ks, AES.MODE_ECB).encrypt(iv)
def decrypt(ks, riv, seq, ct):
    out=bytearray()
    for j in range(0, len(ct), 16):
        iv = riv + b"\x00\x00\x00\x00" + struct.pack(">I",(seq + j//16) & 0xffffffff)
        ks_blk = ecb(ks, iv)
        out += bytes(c^k for c,k in zip(ct[j:j+16], ks_blk))
    return bytes(out)

# Identify the correct OUT (ks,riv): the pair that decrypts a 64 B frame to 16 00 75 00
good=None
for (kh,rh) in {(k,r) for k,r in krs}:
    ks=bytes.fromhex(kh); riv=bytes.fromhex(rh)
    for _ep,_sub,ln,fh,_ts in frames:
        if ln!=64: continue
        f=bytes.fromhex(fh); seq=struct.unpack_from("<I",f,12)[0]
        pt=decrypt(ks,riv,seq,f[16:])
        if pt[:4]==bytes([0x16,0x00,0x75,0x00]) or pt[:2]==bytes([0x16,0x00]):
            good=(ks,riv); print(f"[*] OUT key/riv identified via heartbeat: ks={kh[:12]}.. riv={rh}")
            break
    if good: break
if not good and krs:
    kh,rh=krs[0]; good=(bytes.fromhex(kh),bytes.fromhex(rh)); print("[!] heartbeat match not found; using first (ks,riv)")
if not good: sys.exit("no keys captured")

ks,riv=good
def asc(b): return "".join(chr(x) if 0x20<=x<0x7f else "." for x in b)
hits=0
seen=set()
print("\n=== decrypted frames (non-64 B and/or marker 10 01 00 00 00) ===")
for ep,sub,ln,fh,ts in sorted(frames, key=lambda x:-x[2]):
    if fh in seen: continue
    seen.add(fh)
    f=bytes.fromhex(fh); seq=struct.unpack_from("<I",f,12)[0]; pt=decrypt(ks,riv,seq,f[16:])
    marker = pt.hex().find("1001000000")
    if ln!=64 or marker>=0 or sub!=0x24:
        tag = "  <<< VideoTimingBlock!" if marker>=0 else ""
        stamp = time.strftime("%H:%M:%S", time.localtime(ts)) + f"{ts%1:.3f}"[1:] if ts else "--"
        print(f"\n[{stamp} ep={ep:#x} sub={sub:#x} {ln}B seq=0x{seq:08x}]{tag}")
        print(f"  PT: {pt.hex()}")
        print(f"  asc:|{asc(pt)}|")
        if marker>=0: hits+=1
print(f"\n[*] {hits} frame(s) contained the VideoTimingBlock marker.")
