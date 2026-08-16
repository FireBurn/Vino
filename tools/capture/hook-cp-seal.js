"use strict";
// Read a DL3 control record's plaintext in the process, and find where its opaque tail was copied
// from.
//
// The CTR loop at 0x1cf380 encrypts in place over 16-byte blocks. Its prologue assigns the roles:
//
//   1cf3a9  mov %rsi,%r13     r13 = the cipher object
//   1cf3ac  mov %r8,%r15      r15 = total length
//   1cf3af  mov %rdx,%rbp     rbp = destination
//   1cf3b7  mov %rcx,%r12     r12 = SOURCE, the plaintext
//   1cf3ba  mov %r8,%rbx      rbx = bytes remaining
//   1cf3bd  lea 0x38(%rsp),%r14   r14 = the 16-byte keystream scratch
//
//   1cf431  call 269dd0       AES-ECB -> keystream in r14
//   1cf436  cmp $0xf,%rbx     <- hook here
//   1cf44a  call 1d1a70       XOR(dst=rbp, ks=r14, 16, src=r12)
//   1cf3fa  add $0x10,%r12    source advances per block
//
// So on the FIRST block -- rbx still equal to r15 -- r12 points at the whole untouched record.
// That is the cheapest correct place to read it: one send per message rather than per block.
//
// This is safe on a DL-3x00 specifically. The standing "never hook the CP cipher, it stalls DLM
// into a watchdog restart" warning comes from the docks whose *video* is sealed; on this family
// the pixels are plaintext, so this path carries control records only -- tens per session, not
// thousands per frame.
//
// Having the plaintext in-process is what makes the tail traceable: scanning writable memory for
// those bytes finds every other copy, and a copy that is not the record is the stored value it was
// taken from.

const CTR_FIRST_BLOCK = 0x1cf436;
const MAX_RECORD = 256;

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
let scans = 0;
const SCAN_LIMIT = 6;

// Where the tail sits depends on the message class, exactly as the wire shows. Anything past the
// end of the standard field is the opaque part.
function tailStart(id, sub, len) {
    if (len === 32 && id === 0x14) {
        return 22;
    }
    if (len === 32 && id === 0x16) {
        return 24;
    }
    if (len === 32 && id === 0x15) {
        return 23;
    }
    return Math.max(0, len - 8);
}

Interceptor.attach(dlm.base.add(CTR_FIRST_BLOCK), {
    onEnter: function () {
        const ctx = this.context;
        // Only the first block of each record; r12 has advanced on every later one.
        if (!ctx.rbx.equals(ctx.r15)) {
            return;
        }
        const len = ctx.r15.toInt32();
        if (len <= 0 || len > MAX_RECORD) {
            return;
        }
        let bytes;
        try {
            bytes = new Uint8Array(ptr(ctx.r12).readByteArray(len));
        } catch (e) {
            return;
        }
        n += 1;
        const id = bytes[0] | (bytes[1] << 8);
        const sub = bytes[2] | (bytes[3] << 8);
        const ts = tailStart(id, sub, len);
        const tail = Array.from(bytes.slice(ts));

        const msg = {
            kind: "plain",
            n: n,
            len: len,
            src: ctx.r12.toString(),
            id: id,
            sub: sub,
            bytes: Array.from(bytes),
            tailAt: ts,
        };

        // Find the tail elsewhere in memory. A hit that is not this record is the value it came
        // from, and its address is what the next step watches.
        if (scans < SCAN_LIMIT && tail.length >= 8) {
            scans += 1;
            const pattern = tail
                .slice(0, 8)
                .map(function (b) {
                    return ("0" + b.toString(16)).slice(-2);
                })
                .join(" ");
            const hits = [];
            Process.enumerateRanges("rw-").forEach(function (r) {
                if (hits.length >= 12 || r.size > 0x4000000) {
                    return;
                }
                try {
                    Memory.scanSync(r.base, r.size, pattern).forEach(function (m) {
                        hits.push({
                            addr: m.address.toString(),
                            inRecord: m.address.compare(ptr(ctx.r12)) >= 0
                                && m.address.compare(ptr(ctx.r12).add(len)) < 0,
                            file: r.file ? r.file.path : null,
                        });
                    });
                } catch (e) {
                    /* range vanished under us */
                }
            });
            msg.scan = hits;
            msg.pattern = pattern;
        }
        send(msg);
    },
});

send({ kind: "ready", base: dlm.base.toString() });
