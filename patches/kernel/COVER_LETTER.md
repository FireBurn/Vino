# Cover letter draft: Rust Vino DL3 USB display driver

This series adds Vino, a native Rust DRM/KMS driver for DisplayLink DL3 docks.
The initial, deliberately narrow hardware profile supports the Dell D6000
(`17e9:6006`) with two display heads. It owns device initialization, HDCP 2.2
authentication, the encrypted control channel, EDID acquisition, mode
programming, cursor updates, WHT video encoding, USB submission, and recovery.
It does not require EVDI or a userspace display daemon.

The Vino group depends on the separately routed Rust DRM/KMS, crypto, USB,
workqueue, timer, random, and xxHash bindings identified in `groups/`. Those
bindings are not bundled into the Vino posting merely to make the driver
self-contained.

## Changes since v2

- preserve Lyude Paul's KMS commits, messages, and attribution; put
  current-tree adaptations in later commits;
- reuse Colin Braun's current USB RFC and extend its abstractions in USB-owned
  patches rather than maintaining Vino-local URB or lifetime wrappers;
- use kernel HDCP identifiers and kernel AES, CMAC, SHA-256, HMAC, and
  `crypto_akcipher` RSA facilities through safe Rust interfaces;
- remove consumer `unsafe`, direct bindings, raw ownership reconstruction,
  global device registries, and private Rust ioctl declarations;
- use typed, revocable USB interface I/O and reusable persistent bulk queues;
- use bounded owned shmem scanout views and complete framebuffer reads before
  returning atomic completion;
- retain desired state across transient control failures and rebuild the
  session after reset, resume, or transport loss;
- parallelize rotated and reflected strip encoding;
- make normal operation quiet while retaining opt-in `debug=1` diagnostics;
- remove the bring-up diary, experimental switches, superseded protocol
  hypotheses, and unsupported device matches;
- add kernel and companion protocol/reverse-engineering documentation plus
  reproducible validation and submission scripts.

Vino does not register a private I2C adapter while the Rust I2C adapter lifetime
work remains under subsystem review. It does not handle firmware update
interfaces or claim unvalidated DisplayLink products.

## Validation

The focused kernel build covers `rust/kernel.o`, `evdi.o`, and `vino.o`.
Strict checkpatch checks pass for the EVDI and Vino consumer patches. The
generated series reapplies to its pinned base with an identical tree, and the
standalone protocol oracle passes its byte-exact DLM capture tests.

No kernel or module was installed or loaded and no live hardware test was run
for this cleanup revision.
