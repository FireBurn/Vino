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

⭐ **Chimera drives both DL7400 connectors at 2560x1440p120 in 30 bpp** (2026-08-26). What stood
between a correct wire and a lit pair was orchestration, not the codec -- every strip of a captured
frame decodes byte-correct at the depth the stream declares, 3600 of 3600. Four things were wrong,
and each is the kind that the dock answers by staying silent rather than by reporting anything:

* **The first picture after a mode set reused the carrier's ring slot.** The trailer's phase,
  `seq % dock_buffers`, is how the dock steps between its buffers, so the picture landed in the
  buffer the dock was presenting and the transfer never completed. The frame that opens a stream
  takes sequence zero without being counted, which is where the drift came from. ⚠ The sequence is
  advanced *behind* each submission now: one spent on a frame that never reached the dock leaves
  the ring a slot ahead of what was sent.
* **The decoder configuration was built at eight bits unconditionally**, while the feeder and codec
  took the link's depth. A ten-bit connector then coded coefficients two categories larger than the
  dock had been told to expect: flat areas survive, every edge comes back in primaries. ⭐ The dock
  also **couples depth and transfer function** -- driven at ten bits with an SDR curve it accepts
  the mode and never brings the sink up, so the two are stated together.
* **A dock that wants its video engine committed after finalisation never got it at all**, so its
  pipes were never started. Found by reading the profile, not on hardware.
* **The plane path programmed a mode the dock had declined.** The event path honoured a decline
  beside a lit connector; the plane path recorded the mode anyway, so frames went out against a
  timing the dock never received.

⭐ **Diff chimera against vino, not against the wire.** Both compile the same `cp.rs`, `video.rs`
and `profile.rs`, so where chimera behaves differently the difference is in the orchestration around
them, and that is a much smaller thing to read than a capture.

⚠ **Presence needs the EDID handler engaged first.** A probe taken before engagement answers about
the dock, not about what is plugged into it.

The connector selector is now checked against the vendor's own bytes on all three families -- the
DL-3x00's downstream burst is sent in the clear, so its plaintext is kept as a fixture and every
per-connector record in it names its connector the way this driver does, in the order the vendor
sent them. ⚠ The `strm2` marker has no instance in either fixture and remains unverified.

⚠ **A whole surface is framed in the generic interlaced order**, not the vendor's measured producer
order: that permutation describes the order the driver's own full-surface path produces strips in,
and the strips reaching the framing function do not arrive in it. Applying it names bands that were
never assembled and the dock lights nothing. Reproducing it needs the producing side to match first.


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
and a retransmit debt. All three counts come from the dock's own
`FrameDelivery` -- how many presentations a keyframe makes, how many a delta
makes, and how many logical frames a changed strip stays selected for -- because
the same ring depth does not imply the same delivery choreography, and deriving
them from it had one dock sending every ordinary update three times across four
debt frames. A still desktop sends nothing at all. Encoded strip bodies are
cached against the hash they were encoded from, so the retransmissions the debt
owes do not re-run the codec.

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

⛔ **Open (2026-08-26): chimera receives no cursor event at all.** Not a shape, not a movement, so
there is no hardware pointer on the dock -- and because the pointer was taken out of band, nothing
draws a software one into the frame either. Publishing the plane's format modifier was tried and is
not the cause. The next thing to look at is the module's advertised `max_cursor: (64, 64)`, which a
compositor with a larger pointer cannot satisfy and would answer by compositing into the primary
plane instead; that is untested.

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

It also checks one thing that is not a copy: `drivers/gpu/drm/{vino,evdi}/color.rs` are the same
software colour pipeline and are byte-identical by intent. Neither driver may include the other's
source, so nothing but this check enforces it, and a fix to one has to be ported to the other.

⚠ Vendored copies drifting is not theoretical. The 2026-08-09 sync pulled in two changes chimera
had been running without for days: the **per-head selector bitmask** (`1 << head`, not `head + 1`,
which is the same byte for heads 0 and 1 and wrong for every socket past them) and the **10-bit
codec depth**. Run `make check-sync` before trusting a chimera result.

## What chimera and vino share, and what they do not

Both are driven from the same sources wherever the sources can be shared, so this list is about
what is *structurally* different rather than what someone forgot to copy.

| | vino | chimera |
|---|---|---|
| CP seal, HDCP, codec, record framing | `drivers/gpu/drm/vino/*.rs` | the same files, vendored and compiled verbatim |
| What a dock is | `profile.rs` | the same file, vendored and compiled verbatim |
| Device identification | interface match → identity → family → profile | the same; `vino-driver` parses the identity descriptor and the profile table places it |
| Connector count | from the profile, checked against the endpoints the device exposes | the same |
| Cursor, damage/retransmit | yes | yes |
| Software gamma/CTM | `vino/color.rs` | `evdi/color.rs`, byte-identical |
| Firmware read and DFU update | yes | no, and deliberately: one thing should be able to flash a dock |
| HDR / 10-bit scanout | yes | yes -- the feeder, the codec and the decoder configuration all take the link's depth |
| Stream opening per family | yes | yes: ARM burst, DL7400 prologue, or the shared-pipe ring + configuration |
| Dock-wide mode transaction | yes | **no** -- it declines rather than reset the dock |

The remaining gap is the last row. A dock that reconfigures itself whole needs every lit connector
gathered and committed together; that transaction lives in `drm_sink/activation.rs`, which is
DRM-specific and cannot be vendored into userspace the way `cp.rs`, `video.rs` and `profile.rs`
are. Chimera programs one connector at a time, and where the profile says the dock reconfigures
whole it refuses a second connector rather than doing the thing that resets the dock and takes the
desktop with it.

## Chimera reads the driver's profile, it does not keep its own

Every per-dock decision comes out of `profile.rs`: the strip the codec tiles, the selector a record
carries, the ring depth and how a change is spread over it, what opens a stream, which sink states
bracket a mode set and in which order, whether the pipe is cleared first, how much flat carrier
precedes the first picture, how a connector blanks, the status and frame cadence, whether the dock
composites a cursor of its own, whether its presence probe means anything, and whether an EDID
offered before the downstream read completed may be published.

That consolidation removed a second dock table that had drifted. `vino-driver` now parses the
identity descriptor and nothing else: `Dock::open` takes a `Placement` -- a name, the video
endpoints, the connector count -- from the caller, which gets it from the profile table. A dock
added to the driver is a dock chimera drives, with no second edit and no second opinion.
