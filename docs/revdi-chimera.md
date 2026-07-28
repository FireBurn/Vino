# Revdi and Chimera

Revdi and Chimera provide an alternative userspace architecture without
changing the native Vino driver's design.

```text
compositor -> Revdi DRM device -> safe librevdi client -> Chimera -> D6000
```

## Revdi

The in-tree and standalone module identify as `evdi` and preserve the
established libevdi/DisplayLinkManager ioctl layout. The standalone
`module/*.rs` files are synced from `linux/drivers/gpu/drm/evdi`; the kernel
tree is authoritative.

Revdi uses safe Rust DRM/KMS lifetimes, a conventional UAPI header with
generated bindings, safe 32-bit compat translation, module pinning, driver-core
device management, and bounded owned shmem mappings. Its consumer source has
no raw KMS escape hatch.

The DDC/CI response ioctl remains ABI-compatible but is a no-op until a common
Rust I2C provider API can express the adapter lifetime safely. Revdi does not
invent a private I2C binding to recreate that facility.

`library/src/safe.rs` gives Rust clients one owned device handle. Callback
storage, registered buffers, mode changes, frame borrows, and teardown cannot
outlive that handle. The C ABI remains available for existing managers.

## Chimera

Chimera uses the safe Revdi client and compiles the literal Vino protocol and
codec modules in userspace. The kernel-shim crate supplies userspace
implementations of the same typed secret, HDCP, and RSA-OAEP interfaces. A
userspace-only DDC/CI vendor tunnel remains separate from the vendored kernel
module so it cannot accidentally become an unsupported kernel API.

The service establishes the encrypted dock session, authenticates downstream
heads, fetches EDIDs, creates Revdi outputs, performs validated mode activation,
encodes compositor frames, reconciles DPMS and cursor state, monitors topology,
and rebuilds owned session state after transport failure.

The optimized source and offline oracle build, and the workspace tests pass.
The byte-exact proof checks the production protocol builders against DLM
captures. This cleanup did not exercise the service against hardware, so full
live DLM parity remains a hardware-validation claim.

## Synchronization

From `revdi/`:

```sh
make check-sync KSRC=../linux
make sync KSRC=../linux
```

`check-sync` is read-only. `sync` copies only the explicit Rust sources;
standalone Kbuild, userspace DDC/CI, and Cargo glue remain local.
