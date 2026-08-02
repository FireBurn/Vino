#!/usr/bin/env python3
"""Passively capture Navarro stream-opening/configuration sealers from DisplayLinkManager.

This is deliberately narrow: it hooks the proven sealer factory and transform
entry points, and reports stream-opening candidates plus the larger one-time
configuration records which immediately precede first video.  It does not
change USB, DRM, or DLM state.

Run as root (or against a process the caller may ptrace), preferably before a
fresh dock session:

  sudo env PYTHONPATH=/home/fireburn/.local/lib/python3.14/site-packages \
    /usr/bin/python3 navarro-live-stream.py --process 1234 --secs 900
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
if (dlm === null) throw new Error("DisplayLinkManager module was not found");
const base = dlm.base;
function hex(pointer, length) {
  try {
    const bytes = new Uint8Array(pointer.readByteArray(length));
    return Array.from(bytes, x => x.toString(16).padStart(2, "0")).join("");
  } catch (error) { return "<unreadable: " + error + ">"; }
}
function be32(pointer) {
  try {
    const b = new Uint8Array(pointer.readByteArray(4));
    return (((b[0] << 24) | (b[1] << 16) | (b[2] << 8) | b[3]) >>> 0);
  } catch (error) { return null; }
}
send({kind: "ready", base: base.toString()});
Interceptor.attach(base.add(__FACTORY__), {
  onEnter(args) {
    send({kind: "factory", selector: args[3].toUInt32(),
          key: hex(args[2].add(0x18), 16), riv: hex(args[2].add(0x30), 16)});
  }
});
Interceptor.attach(base.add(__TRANSFORM__), {
  onEnter(args) {
    const length = args[3].toInt32();
    if (length < 12 || length > __MAX_SEAL__) return;
    const plaintext = hex(args[1], length);
    // Short type-4 records identify the stream-open family.  The DL7400's
    // missing first-pipe configuration is 304/1104 bytes before sealing, so
    // retain larger transforms even when their internal marker is unknown.
    if (length <= 256 && !plaintext.startsWith("0400") && length !== 16) return;
    send({kind: "seal", length: length, selector: be32(args[0].add(0x38)),
          plaintext: plaintext, key: hex(args[0].add(0x28), 16),
          riv: hex(args[0].add(8), 24)});
  }
});
"""


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--process", required=True,
                        help="DisplayLinkManager PID (preferred over its truncated task name)")
    parser.add_argument("--secs", type=float, default=900.0)
    parser.add_argument("--factory", default="0x86cca0")
    parser.add_argument("--transform", default="0x85c990")
    parser.add_argument("--max-seal", type=int, default=2048,
                        help="largest one-time transform to report (default: 2048)")
    parser.add_argument("--out", help="optional JSON capture file")
    args = parser.parse_args()

    if not 12 <= args.max_seal <= 65536:
        parser.error("--max-seal must be between 12 and 65536")
    rows: list[dict] = []
    session = frida.attach(int(args.process))
    script = session.create_script(
        JS.replace("__FACTORY__", args.factory)
        .replace("__TRANSFORM__", args.transform)
        .replace("__MAX_SEAL__", str(args.max_seal))
    )

    def on_message(message, _data) -> None:
        if message.get("type") != "send":
            print(json.dumps({"kind": "frida", "message": message}), flush=True)
            return
        row = message["payload"]
        rows.append(row)
        print(json.dumps(row, sort_keys=True), flush=True)

    script.on("message", on_message)
    script.load()
    try:
        time.sleep(args.secs)
    except KeyboardInterrupt:
        pass
    finally:
        try:
            session.detach()
        except frida.Error:
            pass
    if args.out:
        with open(args.out, "w") as output:
            json.dump(rows, output, indent=2)
    return 0


if __name__ == "__main__":
    sys.exit(main())
