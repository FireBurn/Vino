# Revdi and Chimera

Revdi and Chimera provide the alternative userspace architecture without
changing the native Vino kernel driver's design.

```text
compositor
   │ DRM/KMS
   ▼
Revdi kernel module
   │ EVDI private ABI
   ▼
librevdi safe Rust client
   │ coherent frames and events
   ▼
Chimera
   │ Vino protocol + codec over libusb
   ▼
D6000
```

## Revdi

The in-tree and standalone module identify as `evdi` and preserve the existing
libevdi/DisplayLinkManager ABI. The standalone `module/*.rs` files are generated
from `kernel/drivers/gpu/drm/evdi`; the kernel tree is authoritative.

Revdi uses the current safe Rust DRM/KMS interfaces for object lifetimes,
framebuffer mapping, event delivery, module pinning, USB removal, and sysfs
links. Its normal driver code has no raw KMS or USB escape hatch.

The current scanout is selected from a bounded pool of four owned, validated
shmem mappings. Repeated GRABPIX calls and swapchain flips reuse those
preparations; DPMS-off and teardown release the pool.

`library/src/safe.rs` gives Rust clients one owned device handle. Callback
storage, registered framebuffer memory, mode changes, frame borrows, and
teardown cannot outlive that handle. The C ABI remains available for the
proprietary manager.

## Chimera

Chimera uses the safe Revdi client and the literal Vino protocol/codec modules.
Its service path:

1. opens and initializes the dock;
2. completes HDCP 2.2 and establishes the encrypted session;
3. authenticates both downstream heads;
4. obtains EDIDs and creates Revdi outputs;
5. waits for compositor-selected supported modes;
6. performs the captured mode activation and video arm;
7. converts coherent XRGB8888 updates, encodes them, and submits them to the
   correct endpoint;
8. reconciles DPMS and hardware-cursor events;
9. forwards DDC/CI writes and returns bounded read replies; and
10. probes and debounces monitor topology, recreating Revdi outputs after
    reattachment;
11. rebuilds all owned session state after transport failure; and
12. maintains the status-poll and heartbeat cadences.

The optimized service and offline oracle build, and all current workspace tests
pass. This cleanup did not run the service against hardware, so live
DisplayLinkManager parity remains a hardware-validation claim, not an automated
test result. The DPMS, cursor, and DDC/CI paths are implemented and covered
offline. Replug and recovery are implemented as owned teardown/recreation
paths. All of those live paths should be included in the validation matrix
before replacing a production manager installation.

## Synchronization

From `revdi/`:

```sh
make check-sync KSRC=../kernel
make sync KSRC=../kernel
```

`check-sync` is read-only. `sync` copies only the explicit Rust sources;
standalone Kbuild and Cargo glue remain local.
