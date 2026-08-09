## ⚠ This is experimental. It is not guaranteed to work, and it may eat your kittens.

Vino is an unfinished, not-yet-upstream Rust DRM driver for DisplayLink DL3 and DL7000 docks,
meant to replace the EVDI module and the `DisplayLinkManager` binary. These kernels exist so that
people with the hardware can tell us what breaks. **Do not put this on a machine you need.**

Known ways it can ruin your day: a dock can be left in a state that needs a power cycle; a panel
can stay dark; a monitor can come back at the wrong mode. Nothing here is signed, tested at scale,
or supported.

### What is in the packages

A complete kernel configured from the distribution's own kernel configuration, so the rest of your
hardware keeps working, plus `CONFIG_DRM_VINO=m` (binds `17e9:6006` Dell D6000 and `17e9:7000`
DL-7400) and `CONFIG_DRM_EVDI=m`.

Two deliberate deviations from a distribution kernel:

- **`CONFIG_MODVERSIONS` is off.** Rust modules need `gendwarfksyms`, which needs full debug
  information, which makes the build far larger than a CI runner can carry. Modules built for other
  kernels will not load into this one.
- **Debug information is off**, to keep the packages a sane size. Backtraces still resolve through
  kallsyms, which is what a bug report needs.

### Secure Boot

These kernels are unsigned and **will not boot with Secure Boot enabled**. Turn it off, or enrol
your own key. There is no signed build.

### Before you start

Mask the DisplayLink service, or it will race vino for the dock:

```
sudo systemctl mask displaylink-driver.service
```

Vino binds automatically once installed. `dmesg | grep vino` tells you whether it did.

### If something is wrong

Run the collector and attach what it produces to an issue:

```
./tools/capture/collect-report.sh
```

It gathers your dock's descriptors, the connector states, the EDIDs and the vino log into one
tarball. It reads only; it changes nothing. Note that an EDID contains your monitor's serial
number, so look at the tarball before posting it if that bothers you.

Please also say, **in words, what you saw on the panels**. Connector state and USB traffic are not
evidence that a picture reached a monitor, and we have been caught believing otherwise more than
once.

**If vino cannot drive your dock at all**, there is a fuller recipe for capturing a working session
from the vendor driver, which is what lets us add support for hardware we do not own:
[`docs/new-device-capture.md`](../blob/main/docs/new-device-capture.md).
