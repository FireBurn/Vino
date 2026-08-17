"use strict";
// Dump a DL3 control record while it is still plaintext, so its opaque tail can be read.
//
// The seal takes ownership of the record by moving a unique_ptr into a vtable call, and at that
// call instruction the bytes have not been touched yet:
//
//   86c9a2  mov -0x48(%rbp),%rcx   ; rcx = &unique_ptr
//   86c9b8  mov (%rcx),%rdx        ; the buffer pointer
//   86c9be  mov %rdx,-0x40(%rbp)   ; stashed in a local
//   86c9c9  movq $0x0,(%rcx)       ; source nulled -- the move
//   86c9ae  lea -0x40(%rbp),%rsi   ; rsi -> that local
//   86c9d6  call *%rax             ; rax = [[r15+8]+0x10], vtable slot +0x10
//
// So at 0x86c9d6, `rsi` is a pointer to a pointer to the record. The other argument registers are
// dumped alongside because one of them carries the length and it is cheaper to identify it from
// live values than from the prologue.

const SEAL_CALL = 0x86c9d6;
const DUMP = 192;

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

let n = 0;

Interceptor.attach(dlm.base.add(SEAL_CALL), {
    onEnter: function () {
        const ctx = this.context;
        n += 1;
        let bytes = null;
        let buf = null;
        try {
            buf = ptr(ctx.rsi).readPointer();
            bytes = Array.from(new Uint8Array(buf.readByteArray(DUMP)));
        } catch (e) {
            bytes = null;
        }
        send({
            kind: "record",
            n: n,
            buf: buf === null ? null : buf.toString(),
            rdx: ctx.rdx.toString(),
            rcx: ctx.rcx.toString(),
            r8: ctx.r8.toString(),
            r9: ctx.r9.toString(),
            bytes: bytes,
        });
    },
});

send({ kind: "ready", base: dlm.base.toString() });
