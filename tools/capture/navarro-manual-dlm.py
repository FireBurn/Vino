#!/usr/bin/env python3
"""Run a manually owned, Frida-observed DisplayLinkManager for Navarro work.

This intentionally masks ``displaylink-driver.service`` before stopping it, then
spawns DLM suspended and installs the factory/stream-open observer before resuming
the process.  It leaves both the service masked and the manually spawned manager
running when this program exits.  Restore normal ownership explicitly with:

  systemctl unmask displaylink-driver.service
  systemctl reset-failed displaylink-driver.service
  systemctl start displaylink-driver.service
"""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
import time
from pathlib import Path

import frida


SERVICE = "displaylink-driver.service"
DLM = "/opt/displaylink/DisplayLinkManager"
DLM_CWD = "/opt/displaylink"
HOOK = r'''
"use strict";
const dlm = Process.findModuleByName("DisplayLinkManager");
if (dlm === null) throw new Error("DisplayLinkManager module missing");
const base = dlm.base;
function hex(p, n) {
  try { const b = new Uint8Array(p.readByteArray(n));
        return Array.from(b, x => x.toString(16).padStart(2, "0")).join(""); }
  catch (e) { return "<unreadable>"; }
}
function be32(p) {
  try { const b = new Uint8Array(p.readByteArray(4));
        return (((b[0]<<24)|(b[1]<<16)|(b[2]<<8)|b[3]) >>> 0); }
  catch (e) { return null; }
}
function frames(context) {
  let trace=Thread.backtrace(context, Backtracer.ACCURATE);
  if (trace.length < 3) trace=Thread.backtrace(context, Backtracer.FUZZY);
  return trace.slice(0,16).map(address => {
    const module=Process.findModuleByAddress(address);
    return module !== null && module.name === "DisplayLinkManager"
      ? "dlm+0x" + address.sub(base).toString(16) : address.toString();
  });
}
let writerRanges = [];
let writerMonitorArmed = false;
function armWriterMonitor() {
  if (writerMonitorArmed || writerRanges.length !== 4) return;
  try {
    MemoryAccessMonitor.enable(writerRanges, { onAccess(details) {
      send({kind:"writer-access", operation:details.operation,
            address:details.address.toString(), from:"0x" + details.from.sub(base).toString(16)});
    }});
    writerMonitorArmed = true;
    send({kind:"writer-monitor", ranges:writerRanges.map(r => r.base.toString())});
  } catch (error) { send({kind:"writer-monitor-error", error:String(error)}); }
}
send({kind:"ready", base:base.toString()});
Interceptor.attach(base.add(0x86cca0), { onEnter(args) {
  const selector=args[3].toUInt32();
  send({kind:"factory", selector:selector, source:args[2].toString(),
        key:hex(args[2].add(0x18),16), riv:hex(args[2].add(0x30),16)});
  if (__WATCH_WRITERS__ && (selector & 7) === 7 && writerRanges.length < 4) {
    const source=args[2];
    const address=source.add(0x18);
    try {
      writerRanges.push({base:address, size:8});
      send({kind:"writer-watch", slot:writerRanges.length - 1, selector:selector,
            source:source.toString(), address:address.toString()});
      armWriterMonitor();
    } catch (error) { send({kind:"writer-watch-error", error:String(error)}); }
  }
}});
// This is the per-selector wrapper builder immediately upstream of the final
// sealer factory.  Recording its returned source lets a fresh manager startup
// prove the exact producer/consumer association without reusing its keys.
Interceptor.attach(base.add(0x85f5a0), {
  onEnter(args) {
    this.output=args[0]; this.input=args[1];
    send({kind:"source-builder-enter", input:this.input.toString(),
          key:hex(this.input.add(0x1f0),16), riv:hex(this.input.add(0x208),8),
          selector_hint:be32(this.input.add(0x214)),
          return_address:"dlm+0x" + this.returnAddress.sub(base).toString(16),
          bt:frames(this.context)});
  },
  onLeave() {
    try {
      const result=this.output.readPointer();
      send({kind:"source-builder-result", input:this.input.toString(),
            output:this.output.toString(), result:result.toString(),
            key:hex(result.add(0x18),16), riv:hex(result.add(0x30),8)});
    } catch (error) { send({kind:"source-builder-result-error", error:String(error)}); }
  }
});
// The builder calls this ordinary XOR helper at 0x85f621.  Its return address
// makes the relevant invocation unambiguous, without patching an instruction
// in the middle of the builder itself.
Interceptor.attach(base.add(0x1d0ee0), { onEnter(args) {
  if (!this.returnAddress.equals(base.add(0x85f626))) return;
  send({kind:"source-builder-mask", seed:hex(args[0],16), mask:hex(args[1],16),
        length:args[2].toUInt32()});
}});
let videoKeyInstallTarget = null;
// This handler creates each source's random seed/RIV and hands the derived
// 25-byte key-install record to its transport.  Resolve that virtual call once
// and observe the message at a real function boundary.
Interceptor.attach(base.add(0x8644b0), { onEnter(args) {
  const source=args[1];
  send({kind:"video-key-source", source:source.toString(),
        seed:hex(source.add(0x1f0),16), riv:hex(source.add(0x208),8),
        material_160:hex(source.add(0x160),8), material_170:hex(source.add(0x170),16),
        material_1e0:hex(source.add(0x1e0),8)});
  if (videoKeyInstallTarget !== null) return;
  try {
    const transport=source.add(0x80).readPointer();
    videoKeyInstallTarget=transport.readPointer().add(0x18).readPointer();
    Interceptor.attach(videoKeyInstallTarget, { onEnter(callArgs) {
      const length=callArgs[2].toUInt32();
      if (length === 25 && hex(callArgs[1],1) === "0b")
        send({kind:"video-key-install", target:videoKeyInstallTarget.toString(),
              payload:hex(callArgs[1],length)});
    }});
  } catch (error) { send({kind:"video-key-install-hook-error", error:String(error)}); }
}});
// CP setup normally uses libusb's asynchronous submit path.  Keep only small
// EP02 records: this relates the internal 0x0b SKE message to its exact outer
// framing without collecting any video payload.
Interceptor.attach(base.add(0x83c20), { onEnter(args) {
  try {
    const transfer=args[0];
    const endpoint=transfer.add(0x9).readU8();
    const length=transfer.add(0x14).readU32();
    if (endpoint === 0x02 && length > 0 && length <= 512)
      send({kind:"usb-ep02", length:length, payload:hex(transfer.add(0x30).readPointer(), length)});
  } catch (error) { send({kind:"usb-ep02-hook-error", error:String(error)}); }
}});
// The fresh manager uses usbfs directly for most writes.  Record the small
// EP02 submissions and a strictly bounded prefix of first video URBs.  The
// latter correlates a one-time sealed configuration with its actual EP08/EP0A
// wire record without retaining ordinary frame traffic.
try {
  const ioctl=Module.getGlobalExportByName("ioctl");
  const videoUrbs={8:0, 10:0};
  Interceptor.attach(ioctl, { onEnter(args) {
    try {
      if (args[1].toUInt32() !== 0x8038550a) return; // USBDEVFS_SUBMITURB
      const urb=args[2];
      if (urb.readU8() !== 3) return; // bulk OUT
      const endpoint=urb.add(1).readU8();
      const length=urb.add(24).readS32();
      const buffer=urb.add(16).readPointer();
      if (endpoint === 0x02 && length > 0 && length <= 512)
        send({kind:"usbfs-ep02", length:length, payload:hex(buffer,length)});
      if ((endpoint === 0x08 || endpoint === 0x0a) && length > 0 &&
          videoUrbs[endpoint] < __VIDEO_URBS__) {
        const ordinal=videoUrbs[endpoint]++;
        send({kind:"usbfs-video-prefix", endpoint:endpoint, ordinal:ordinal,
              length:length, payload:hex(buffer,Math.min(length,__VIDEO_PREFIX__))});
      }
    } catch (error) { send({kind:"usbfs-ep02-hook-error", error:String(error)}); }
  }});
} catch (error) { send({kind:"usbfs-hook-error", error:String(error)}); }
Interceptor.attach(base.add(0x85c990), { onEnter(args) {
  const length=args[3].toInt32();
  if (length < 12 || length > 2048) return;
  const plaintext=hex(args[1],length);
  if (length === 64)
    send({kind:"seal-64", selector:be32(args[0].add(0x38)), plaintext:plaintext});
  // Preserve the 304/1104-byte first-pipe configuration transforms too.  The
  // short stream-open marker is not sufficient to recover the DL7400 startup
  // sequence, and pixel traffic does not pass through this sealer.
  if (plaintext.startsWith("0400") || length === 16 || length > 256)
    send({kind:"seal", length:length, selector:be32(args[0].add(0x38)), plaintext:plaintext});
}});
'''


def command(argv: list[str]) -> dict[str, object]:
    result = subprocess.run(argv, text=True, capture_output=True, check=False)
    return {"kind": "command", "argv": argv, "returncode": result.returncode,
            "stdout": result.stdout, "stderr": result.stderr}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--seconds", type=float, default=3600.0)
    parser.add_argument("--out", type=Path,
                        default=Path("/tmp/vino-navarro-manual-dlm.jsonl"))
    parser.add_argument("--pid-file", type=Path,
                        default=Path("/tmp/vino-manual-dlm.pid"))
    parser.add_argument("--video-urbs", type=int, default=8,
                        help="first bulk video URBs retained per endpoint (default: 8)")
    parser.add_argument("--video-prefix", type=int, default=4096,
                        help="bytes retained from each selected video URB (default: 4096)")
    args = parser.parse_args()
    if not 0 <= args.video_urbs <= 64:
        parser.error("--video-urbs must be between 0 and 64")
    if not 16 <= args.video_prefix <= 65536:
        parser.error("--video-prefix must be between 16 and 65536")
    events: list[dict[str, object]] = []

    def emit(row: dict[str, object]) -> None:
        events.append(row)
        print(json.dumps(row, sort_keys=True), flush=True)
        with args.out.open("a") as stream:
            stream.write(json.dumps(row, sort_keys=True) + "\n")

    args.out.unlink(missing_ok=True)
    for argv in (["systemctl", "mask", SERVICE], ["systemctl", "stop", SERVICE]):
        emit(command(argv))
    subprocess.run(["pkill", "-9", "-f", "^/opt/displaylink/DisplayLinkManager$"], check=False)
    time.sleep(1)

    device = frida.get_local_device()
    pid = device.spawn([DLM], cwd=DLM_CWD)
    args.pid_file.write_text(f"{pid}\n")
    emit({"kind": "spawn", "pid": pid})
    session = device.attach(pid)
    script = session.create_script(
        HOOK.replace("__WATCH_WRITERS__", "true")
        .replace("__VIDEO_URBS__", str(args.video_urbs))
        .replace("__VIDEO_PREFIX__", str(args.video_prefix))
    )

    def on_message(message, _data) -> None:
        if message.get("type") == "send":
            emit(message["payload"])
        else:
            emit({"kind": "frida", "message": message})

    script.on("message", on_message)
    script.load()
    time.sleep(0.2)
    device.resume(pid)
    emit({"kind": "resumed", "pid": pid})
    try:
        time.sleep(args.seconds)
    except KeyboardInterrupt:
        pass
    finally:
        # DLM deliberately remains alive and service stays masked.  Detaching Frida does not
        # terminate it, and makes the next experiment's ownership explicit.
        try:
            session.detach()
        except frida.Error:
            pass
    return 0


if __name__ == "__main__":
    sys.exit(main())
