"use strict";
// Hook DLM's set-mode serializer -- the function that builds `id=0x48 sub=0x22` -- and print its
// arguments and the timing block it is handed.
//
// This answers the three fields that no capture and no decompile can settle, because the
// serializer does not compute them: it receives them.
//
//   arg2 (dx)  -> offset 42, the sync word. The two polarity flags are OR'd in by the callee, so
//                 whatever arrives here is the BASE -- the 0x0400 whose meaning is still open,
//                 and 0x8000 on a teardown.
//   arg3 (cl)  -> offset 68 low byte. Zero in every capture; is it ever anything else?
//   arg4 (r8d) -> offset 23, the DMA buffer format (0..3, bytes-per-pixel {2,4,3,4}).
//   arg5 (r9d) -> offset 62, a u32 that is zero in every capture and has no other evidence.
//
// The timing block (arg1) is also dumped: its +0x50 word is the colour depth the callee switches
// on (0x10/0x18/0x1e/0x24/0x30 -> 16/24/30/36/48 bpp), so a run at a deep-colour or HDR mode
// shows both that field and whatever moves with it.
//
// Function RVA is for DLM 3.4.26 (md5 1f2e2d68bdec4be9f79d9e76204add4c). Ghidra addresses in the
// project are this + 0x100000.
//
// Usage (unit MASKED, DLM started by hand -- udev will otherwise bounce it and kill the hook):
//   sudo -E python3 tools/capture/run-setupvideo-hook.py --secs 60

const SETUP_VIDEO = 0x5766b0;

const dlm = Process.findModuleByName("DisplayLinkManager");
if (dlm === null) {
    throw new Error("DisplayLinkManager module not found");
}

function u16(p, off) {
    return p.add(off).readU16();
}

// The callee tests these as `char`, so only the low byte is meaningful -- the rest of the u16 is
// uninitialised padding and reads as garbage.
function flag(p, off) {
    return p.add(off).readU8();
}

let calls = 0;

Interceptor.attach(dlm.base.add(SETUP_VIDEO), {
    onEnter(args) {
        const block = args[1];
        const sync = args[2].toInt32() & 0xffff;
        const off68_low = args[3].toInt32() & 0xff;
        const dma_format = args[4].toInt32();
        const off62 = args[5].toInt32();

        const teardown = (sync & 0x8000) !== 0;
        let line =
            `[setupVideo #${calls++}] sync_base=0x${sync.toString(16).padStart(4, "0")}` +
            `${teardown ? " (TEARDOWN)" : ""}` +
            ` off68_low=0x${off68_low.toString(16)}` +
            ` dma_format=${dma_format}` +
            ` off62=0x${off62.toString(16)}`;

        // The callee reads these out of the block; print them so a line stands on its own.
        try {
            const htotal = u16(block, 0x00);
            const hactive = u16(block, 0x02);
            const vtotal = u16(block, 0x0e);
            const vactive = u16(block, 0x10);
            const hsync_inv = flag(block, 0x0c);
            const vsync_inv = flag(block, 0x1a);
            const clock_khz = block.add(0x1c).readU32();
            const vic = block.add(0x20).readU8();
            const depth = block.add(0x50).readU32();
            line +=
                `\n    ${hactive}x${vactive} htotal=${htotal} vtotal=${vtotal}` +
                ` clock=${clock_khz}kHz vic=${vic} depth=${depth}` +
                ` hSyncInv=${hsync_inv} vSyncInv=${vsync_inv}`;
        } catch (e) {
            line += `\n    (timing block unreadable: ${e})`;
        }
        console.log(line);
    },
});

console.log(`[*] hooked setupVideo at ${dlm.base.add(SETUP_VIDEO)} -- change a resolution now`);
