# Binary reverse-engineering helpers

Ghidra scripts for reading the DisplayLink binaries. They are driven headless — the GhidraMCP
bridge has never worked, and the GUI is not scriptable from a runbook.

| Script | Purpose |
|---|---|
| `BSimQueryAt.java` | Ranks the whole DLM corpus by function similarity to the function containing an address, without the GUI. Ghidra's own `QueryFunction.java` prompts interactively and cannot be used from a script. |

## Running one

```sh
ghidra-headless <project-dir> <ProjectName> -process <binary> -noanalysis \
    -scriptPath tools/re -postScript BSimQueryAt.java \
    file:/home/fireburn/dlm-bsim/dlm 0x<ghidra_addr>
```

Ghidra addresses are the ELF offset plus the `0x100000` image base.

## What BSim can and cannot do here

It maps one function across architecture, version and OS — genuinely useful for following a
routine from x86-64 back to a cleaner aarch64 build, or across DLM versions.

⛔ **It will not name anything in the device layer.** The only corpus members that kept symbols are
the macOS *agent* (`DLUserAgent`, `DLXpcService`) and the Windows *dlidusb* driver, and neither
contains the `libdl3device` code. Every match for a DLM device-layer function is another stripped
DLM. Names for that layer come from the obfuscated string store instead: its blobs sit in source
order at a fixed stride, and because they are inline in the binary their addresses serve as xref
anchors. That is how the set-mode serializer was found and named.
