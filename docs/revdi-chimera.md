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

## Cursor

The module advertises a real cursor plane, so the compositor keeps the pointer out of the primary
framebuffer and every pointer movement no longer dirties the desktop.

Cursor reporting is opt-in through `DRM_IOCTL_EVDI_ENABLE_CURSOR_EVENTS`; a client that has not asked
for it composites the pointer itself and must not also receive it out of band. Once enabled, a shape
change emits `CURSOR_SET` and a movement emits `CURSOR_MOVE`. Shape changes are filtered on the
framebuffer identity, because the compositor commits the cursor plane on every movement and each
`CURSOR_SET` costs the client a map, a copy and a handle close.

No hotspot is reported. The compositor has already applied it when it placed the plane, so the
destination rect **is** where the bitmap goes; reporting one as well would have the client subtract
it a second time and shift the pointer. vino, which drives the same sinks, positions by the bitmap
corner for the same reason. The position sent is the **unclipped** origin
(`destination - source`): when the cursor straddles an edge the helper clips the destination and
advances the source by the same amount, so the difference recovers where the corner actually is,
including off-screen. `destination` alone pins the pointer to the edge and makes it drift.

`CURSOR_SET` carries a GEM handle minted in the *client's* file, which libevdi maps, copies out and
closes — so a fresh handle is needed per change, and a pre-minted or reused one does not work.
Minting allocates and takes mutexes, so it cannot run under `event_lock` where the channel's only
reference to the connected file lives, and DRM files are not refcounted so the reference cannot be
held across the sleep either. `EventChannel::with_connected_file` resolves that: it confirms the file
is still on `drm_device::filelist` while holding `filelist_mutex`, which `drm_close_helper()` holds
across the `list_del()` and releases only before `drm_file_free()`. The C EVDI driver instead stashes
a `drm_file` pointer at connect time and uses it later, racing fd close.

⚠ The event payload has three bytes of ABI padding between `enabled` and `buffer_handle`. They are
named explicitly and zeroed, so the payload stays provably padding-free rather than leaking stack to
userspace.

### Dock cursor protocol

Chimera drives the dock's own cursor from these events. Offset 22 of the control message carries the
head id **counted from one**, offset 23 is the **visible flag**, and the bitmap starts at **offset
34**. Hiding the cursor clears offset 23; parking it off-screen instead leaves a ghost pointer at the
top-left, because the dock wraps an out-of-range origin rather than clipping. The differential prover
reproduces all twelve cursor messages in the reference DLM capture byte-for-byte, and that capture
independently shows DLM hiding the cursor the same way.

## Synchronization

From `revdi/`:

```sh
make check-sync KSRC=../linux
make sync KSRC=../linux
```

`check-sync` is read-only. `sync` copies only the explicit Rust sources;
standalone Kbuild, userspace DDC/CI, and Cargo glue remain local.
