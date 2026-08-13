"use strict";
// Walk back from the wire to whatever writes a CP message's opaque tail.
//
// The tail is not fresh generator output: a window carrying 65 tailed messages calls the CTR_DRBG
// zero times, and none of rand/random/random_device/getrandom either. It is also session-specific
// -- the same message in two sessions carries different bytes -- so it is computed from session
// state, and the chain that computes it is what this is for.
//
// Anchor on the USB submission rather than guessing at the builder. It is cheap (tens of calls,
// not thousands), it is unambiguous -- the buffer it is handed is exactly what the capture shows --
// and its backtrace names the frames above it. Those frames are the entry to the chain.
//
// The buffer at that point is sealed, so it cannot be read for the tail. What it gives is the
// caller chain and the record's length, which is enough to pick the builder out and hook that
// next.

function findModule(name) {
    const mods = Process.enumerateModules();
    for (let i = 0; i < mods.length; i++) {
        if (mods[i].name.indexOf(name) !== -1) {
            return mods[i];
        }
    }
    return null;
}

function findExport(name) {
    if (typeof Module.findGlobalExportByName === "function") {
        return Module.findGlobalExportByName(name);
    }
    return Module.findExportByName(null, name);
}

const dlm = findModule("DisplayLinkManager");
if (dlm === null) {
    throw new Error("DisplayLinkManager module not found");
}

function frames(ctx) {
    return Thread.backtrace(ctx, Backtracer.FUZZY).map(function (f) {
        if (f.compare(dlm.base) >= 0 && f.compare(dlm.base.add(dlm.size)) < 0) {
            return "0x" + f.sub(dlm.base).toString(16);
        }
        return DebugSymbol.fromAddress(f).name || f.toString();
    });
}

// The vendor ships its own libusb and does not export its symbols usefully, so hooking
// libusb_* catches nothing. Every submission still becomes a usbfs ioctl, which is one anchor
// that cannot be bypassed by however the library is linked.
//
//   USBDEVFS_SUBMITURB = _IOWR('U', 10, struct usbdevfs_urb) = 0x8038550a
//   struct usbdevfs_urb: endpoint at +1, buffer at +16, buffer_length at +24
const SUBMITURB = 0x8038550a;
const ioctl = findExport("ioctl");
if (ioctl !== null) {
    Interceptor.attach(ioctl, {
        onEnter: function (args) {
            if (args[1].toInt32() >>> 0 !== SUBMITURB) {
                return;
            }
            const urb = args[2];
            const len = urb.add(24).readInt();
            // Control records are small; frames are megabytes. Skipping the latter keeps this
            // off the hot path entirely.
            if (len <= 0 || len > 256) {
                return;
            }
            const buf = urb.add(16).readPointer();
            send({
                kind: "urb",
                ep: urb.add(1).readU8(),
                len: len,
                head: Array.from(new Uint8Array(buf.readByteArray(Math.min(len, 48)))),
                frames: frames(this.context),
            });
        },
    });
}

// Kept for builds that do export them.
const SYNC = findExport("libusb_bulk_transfer");
if (SYNC !== null) {
    Interceptor.attach(SYNC, {
        onEnter: function (args) {
            const len = args[3].toInt32();
            // Control records are small; frames are megabytes. Only the former are interesting,
            // and skipping the latter keeps this off the hot path entirely.
            if (len <= 0 || len > 256) {
                return;
            }
            send({
                kind: "bulk",
                ep: args[1].toInt32(),
                len: len,
                head: Array.from(new Uint8Array(args[2].readByteArray(Math.min(len, 32)))),
                frames: frames(this.context),
            });
        },
    });
}

const ASYNC = findExport("libusb_submit_transfer");
if (ASYNC !== null) {
    Interceptor.attach(ASYNC, {
        onEnter: function (args) {
            const t = args[0];
            // struct libusb_transfer: endpoint at 0x08, length at 0x10, buffer at 0x28 on x86-64.
            const len = t.add(0x10).readInt();
            if (len <= 0 || len > 256) {
                return;
            }
            const buf = t.add(0x28).readPointer();
            send({
                kind: "submit",
                ep: t.add(0x08).readU8(),
                len: len,
                head: Array.from(new Uint8Array(buf.readByteArray(Math.min(len, 32)))),
                frames: frames(this.context),
            });
        },
    });
}

send({ kind: "ready", base: dlm.base.toString(), sync: SYNC !== null, async: ASYNC !== null });
