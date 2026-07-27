# Cover letter draft: Rust DRM/KMS consumers and Vino DL3 driver

This series prepares the Rust DRM/KMS stack for two real display consumers,
adds a Rust EVDI-compatible virtual display, and adds Vino, a native Rust
DRM/KMS driver for DisplayLink DL3 docks.

The first Vino profile supports the Dell D6000 (`17e9:6006`) with two display
heads. Vino owns device initialization, HDCP 2.2 authentication, the encrypted
control channel, EDID and DDC/CI tunneling, mode programming, cursor updates,
WHT video encoding, and USB submission. It does not require EVDI or a userspace
display daemon.

## Changes since v2

- preserve Lyude Paul's KMS commits and attribution instead of squashing or
  reimplementing them;
- base on the current `drm-next` and `drm-rust-next` tips;
- use the current Rust DRM registration and shmem mapping infrastructure;
- replace consumer raw C/FFI and lifetime escape hatches with safe subsystem
  APIs;
- model USB transfer permission as an adapter-owned revocable I/O window;
- fail probe cleanly if DRM, session work, or DDC/CI registration cannot be
  established instead of binding a partial device;
- use kernel HDCP identifiers and in-tree crypto primitives;
- keep control transactions on an ordered device queue;
- decode bounded DDC/CI read replies through the same encrypted transaction
  path used for monitor-control writes;
- add owned validated shmem views and use bounded four-buffer mapping pools in
  both Revdi and Vino;
- snapshot scanout before releasing the compositor buffer and track damage
  against the last successful dock presentation;
- remove development knobs, experimental commits, superseded protocol notes,
  and hardware bring-up commentary;
- add in-tree driver documentation and a reproducible external evidence/test
  repository.

## Review routing

The complete integration branch is useful for build and dependency testing, but
the new generic work should be reviewed through its owning subsystem:

- Rust core and workqueue changes: Rust-for-Linux;
- USB endpoint, queue, and I/O-window changes: USB and Rust-for-Linux;
- crypto primitives: crypto;
- I2C adapter provider: I2C;
- DRM/KMS, HDCP identifiers, modes, events, and drivers: DRM;
- sysfs/platform helpers: driver core/Rust-for-Linux.

This cover letter does not assume those APIs are accepted merely because both
drivers build.

## Validation

```text
make LLVM=1 -j16 rust/kernel.o
make LLVM=1 -j16 drivers/gpu/drm/evdi/evdi.o
make LLVM=1 -j16 drivers/gpu/drm/vino/vino.o
```

The generated patch set is also applied to a disposable worktree and checked
for tree identity. Revdi/Chimera source synchronization, Cargo tests, library
tests, formatting, and optimized Chimera builds pass.

No kernel was installed or booted, no module was loaded, and no hardware test
was performed for this cleanup revision.
