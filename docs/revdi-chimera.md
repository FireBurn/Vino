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

The byte-exact proof checks the production protocol builders against DLM
captures, and the workspace tests pass.

✅ **HW-verified 2026-08-04** with vino unloaded: Revdi's `evdi.ko` serves
librevdi, Chimera authenticates a D6000, fetches head 0's EDID, creates the card,
KWin enables it at 2560x1440@120 and the damage path runs. A frame captured off
the wire and rendered back to pixels decodes 3600/3600 strips with no coverage
gap. Two probe bugs the run exposed are fixed and are worth knowing about,
because both had already been solved in the driver: presence is bit `0x1000` of
the probe reply's **status word**, never the handler id, and a reply must be
paired with its request by **echoed counter** — the dock's answer routinely
arrives only after the next message has gone out, so taking the next frame off
EP84 reports the wrong head connected.

## Sending damage, not frames

Chimera sends what the dock still needs, the way DLM does, following the
driver's scanout engine: `chimera/src/scanout.rs` keeps a content hash per strip
and a retransmit debt of `dock_buffers + 1`. A keyframe is presented as many
times as the dock has buffers; a delta once, with its repeats spread over
following frames by the debt; and a still desktop sends nothing at all. Encoded
strip bodies are cached against the hash they were encoded from, so the
retransmissions the debt owes do not re-run the codec.

⚠ The debt is paid per *presented* frame, so a change made just before the
desktop goes still would strand in one dock buffer and ghost. The daemon
re-presents the last surface while `ControlSession::owes_repaint()` holds.

## Reconnecting to a configured card

`MODE_CHANGED` is an edge, and evdi cards outlive the client process — they are
removed through `/sys/devices/evdi/remove_all`, not by exit. A client attaching
to a card the compositor has already configured therefore used to wait forever
for an announcement that had already happened, with its output dark and the CRTC
enabled. CONNECT now replays the current mode, so restarting the service works.

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
