"use strict";
// Capture what DLM's CTR_DRBG actually produces, so it can be matched against the wire.
//
// Knowing the tokens come from the DRBG says nothing about what happens to the bytes between the
// generator and the wire. If a token appears on the wire byte for byte, it is raw output and there
// is nothing to derive. If it does not, something transforms it, and the frames recorded here name
// the function that did.
//
// mbedtls_ctr_drbg_random(ctx, output, len): the bytes exist only once it returns, so the buffer
// and length are stashed on entry and read on the way out.
//
// The plaintext AKE on the wire is the oracle that needs no keys: `rtx` sits unencrypted at
// offset 28 of the `sub=0x04` AKE_Init, so a search for those eight bytes in this log answers the
// question outright.
//
// DLM 3.4.26. RVAs are file offsets; Ghidra addresses are these + 0x100000.
// The AES-ECB core (0x269dd0) and the CP cipher loop (0x1cf436) beneath this are far too hot to
// hook -- both stall DLM into a watchdog restart.

// These are addresses *inside* the DRBG's generate loop, immediately after its AES-ECB call --
// not function entries, so Frida's `args` are meaningless here and reading them yields nulls.
// Read the registers instead. Disassembling the loop gives their roles:
//
//   26c3f3  mov %r12,%rcx     r12 -> the 16-byte AES output block, the fresh random
//   26c401  call 269dd0       AES-ECB(ctx, 1, V, out)
//   26c406  test %eax,%eax    <- hook here; r12 is now filled
//   26c42b  mov %dl,(%r15)    r15 -> the caller's output buffer, advancing
//   26c439  sub %rax,%r14     r14 -> bytes still owed to the caller
//
// r15 is the interesting one: it says where the bytes are going, which is what turns "the DRBG
// made these" into "and this is who wanted them".
const GEN = { "gen_26c406": 0x26c406 };
const BLOCK = 16;
const TRACE_LIMIT = 3;

function findModule(name) {
    const mods = Process.enumerateModules();
    for (let i = 0; i < mods.length; i++) {
        if (mods[i].name.indexOf(name) !== -1) {
            return mods[i];
        }
    }
    return null;
}

const dlm = findModule("DisplayLinkManager");
if (dlm === null) {
    throw new Error("DisplayLinkManager module not found");
}

const traced = {};

function frames(ctx) {
    return Thread.backtrace(ctx, Backtracer.FUZZY).map(function (f) {
        if (f.compare(dlm.base) >= 0 && f.compare(dlm.base.add(dlm.size)) < 0) {
            return "0x" + f.sub(dlm.base).toString(16);
        }
        return DebugSymbol.fromAddress(f).name || f.toString();
    });
}

Object.keys(GEN).forEach(function (tag) {
    Interceptor.attach(dlm.base.add(GEN[tag]), {
        onEnter: function (args) {
            traced[tag] = (traced[tag] || 0) + 1;
            this.n = traced[tag];
            // Report the call itself, with its raw arguments, before anything is filtered on
            // them. A hook that only reports on the way out cannot be told apart from a hook
            // that never fired, and three runs were lost to exactly that ambiguity.
            const ctx = this.context;
            let block = null;
            try {
                block = Array.from(new Uint8Array(ptr(ctx.r12).readByteArray(BLOCK)));
            } catch (e) {
                block = null;
            }
            send({
                kind: "drbg",
                site: tag,
                n: this.n,
                block: block,
                dest: ctx.r15.toString(),
                remaining: ctx.r14.toString(),
                frames: this.n <= TRACE_LIMIT ? frames(ctx) : null,
            });
        },
    });
});

send({ kind: "ready", base: dlm.base.toString() });
