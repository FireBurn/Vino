"use strict";
// Find which generator fills the opaque tails of DL3 control messages.
//
// The tails sit at a different offset in every message class and are the one part of the protocol
// no capture can explain, so the question is answered in the process rather than on the wire.
// Four candidates are live in this binary at once and only one of them can be it:
//
//   * the byte filler at RVA 0x28cb00 -- `*p++ = (uint8_t)rand()` per byte. It is only ever passed
//     as a function pointer, at six sites that look like a crypto self-test, so whether it runs at
//     all in a driving session is exactly what this settles.
//   * glibc `rand` and `random`, seeded from `gettimeofday().tv_usec` by a thunk at 0x305fc0.
//   * `std::random_device`, which reads /dev/urandom.
//   * std::mt19937, whose twist is inlined at 0x305f00 and therefore cannot be hooked -- it is the
//     answer by elimination if the other three stay silent.
//
// Counting alone identifies it: a status poll carries a ten-byte tail and goes out every 250 ms,
// so the generator behind it fires in bursts of ten at four hertz. Everything else is noise.
//
// Backtraces are the point of the exercise rather than a nicety. The call arrives through vtables
// and thunks, so the frame list is what chains a generator back to the message builder that wanted
// the bytes -- print it relative to the module base, which is what Ghidra wants.
//
// DLM 3.4.26. RVAs are file offsets; Ghidra addresses are these + 0x100000.

const FILL_RNG = 0x28cb00;
const SEEDER = 0x305fc0;

// DLM carries its own CSPRNG -- an mbedTLS CTR_DRBG -- which is why no libc generator is ever
// called for a token. These are its reseed/generate entry points, pinned by a cold-plug trace that
// decrypted a real msg0 and matched its ten-byte token to the first ten bytes of the AES output
// here. The libuuid `getrandom` calls that a syscall trace shows are this DRBG's entropy source,
// not something unrelated.
//
// The AES-ECB core (0x269dd0) and the CP cipher loop (0x1cf436) sit under these and are far too
// hot to hook -- doing so stalls DLM into a watchdog restart. Hook the DRBG, never the cipher.
const DRBG = { "drbg_seed": 0x26bd57, "drbg_reseed": 0x26bde6, "drbg_gen1": 0x26bf56, "drbg_gen2": 0x26c406 };

// Frida 17 replaced `Process.findModuleByName` and `Module.findExportByName(null, ...)`.
// Enumerating is available in every version and costs one pass at load.
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
    if (typeof Module.findExportByName === "function") {
        return Module.findExportByName(null, name);
    }
    return null;
}

const dlm = findModule("DisplayLinkManager");
if (dlm === null) {
    throw new Error("DisplayLinkManager module not found");
}

// Backtraces for the first few calls of each site only. A tail is ten bytes and the filler is per
// byte, so tracing every call would cost more than the process can absorb -- and the tenth frame
// list is the same as the first.
const TRACE_LIMIT = 4;
const counts = {};
const traced = {};

function rva(addr) {
    return "0x" + addr.sub(dlm.base).toString(16);
}

function backtrace(ctx, tag) {
    traced[tag] = (traced[tag] || 0) + 1;
    if (traced[tag] > TRACE_LIMIT) {
        return;
    }
    const frames = Thread.backtrace(ctx, Backtracer.FUZZY)
        .map(function (f) {
            // Frames inside DLM are the ones worth having; anything else is libc and named.
            if (f.compare(dlm.base) >= 0 && f.compare(dlm.base.add(dlm.size)) < 0) {
                return rva(f);
            }
            return DebugSymbol.fromAddress(f).name || f.toString();
        });
    send({ kind: "trace", tag: tag, frames: frames });
}

function count(tag) {
    counts[tag] = (counts[tag] || 0) + 1;
}

// The byte filler. Its arguments are (ctx, buf, len), and the bytes it produced are only readable
// once it returns, so the buffer is stashed on entry.
try {
    Interceptor.attach(dlm.base.add(FILL_RNG), {
        onEnter: function (args) {
            this.buf = args[1];
            this.len = args[2].toInt32();
            count("fill_rng");
            backtrace(this.context, "fill_rng");
        },
        onLeave: function () {
            if (this.len > 0 && this.len <= 64) {
                send({
                    kind: "fill",
                    len: this.len,
                    bytes: Array.from(new Uint8Array(this.buf.readByteArray(this.len))),
                });
            }
        },
    });
} catch (e) {
    send({ kind: "error", what: "fill_rng", msg: e.message });
}

// The seeder, which says when a glibc stream starts and with what. `gettimeofday().tv_usec` has a
// million values, so a seed recovered here makes the whole stream reproducible offline.
try {
    Interceptor.attach(dlm.base.add(SEEDER), {
        onEnter: function () {
            count("seeder");
            backtrace(this.context, "seeder");
        },
    });
} catch (e) {
    send({ kind: "error", what: "seeder", msg: e.message });
}

// The DRBG that actually produces DL3 tokens.
Object.keys(DRBG).forEach(function (tag) {
    try {
        Interceptor.attach(dlm.base.add(DRBG[tag]), {
            onEnter: function () {
                count(tag);
                backtrace(this.context, tag);
            },
        });
    } catch (e) {
        send({ kind: "error", what: tag, msg: e.message });
    }
});

// The libc generators themselves, hooked at the export so every caller is caught whatever it
// reaches them through.
["rand", "random"].forEach(function (name) {
    const p = findExport(name);
    if (p === null) {
        return;
    }
    Interceptor.attach(p, {
        onEnter: function () {
            count(name);
            backtrace(this.context, name);
        },
    });
});

// std::random_device, the /dev/urandom path.
["_ZNSt13random_device9_M_getvalEv", "getrandom"].forEach(function (name) {
    const p = findExport(name);
    if (p === null) {
        return;
    }
    Interceptor.attach(p, {
        onEnter: function () {
            count(name);
            backtrace(this.context, name);
        },
    });
});

setInterval(function () {
    send({ kind: "counts", counts: counts });
}, 1000);

send({ kind: "ready", base: dlm.base.toString(), size: dlm.size });
