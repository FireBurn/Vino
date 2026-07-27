# Architecture

Vino keeps the USB control plane, KMS state, and video transport separate while
giving them one per-device owner.

```text
atomic KMS state ──► desired per-head state ──► ordered control queue
       │                                           │
       └─ coherent snapshot ─► per-head encoder ───┴─► persistent USB queues

EP84 replies ──► authenticated session owner ──► monitor/control transitions
```

## Ownership

The bound USB device owns:

- the DRM registration and device;
- the authenticated control session and its counters;
- the revocable USB I/O window;
- ordered control and per-head scanout workers;
- I2C adapters, vblank timers, coherent snapshots, and USB queues.

Normal lifecycle code in `drivers/gpu/drm/vino` and
`drivers/gpu/drm/evdi` contains no `unsafe` blocks, direct `bindings::` calls,
manual `Arc` raw conversions, raw lifetime reconstruction, or global raw device
registries. Where the existing Rust kernel surface was insufficient, the
series adds a safe binding in the owning subsystem before converting the
consumer.

## Control serialization

HDCP keys, nonces, inner counters, wire counters, EP02 writes, and EP84 reply
classification belong to one session. KMS callbacks record desired state and
wake an ordered queue; they do not issue blocking USB transactions.

A mode activation is transactional. The desired generation is retained after
a transient failure and retried, while a newer atomic state supersedes stale
work. Monitor re-engagement follows the same rule.

## Frame ownership

The dock cannot scan out a GEM object. Vino waits for the producer, copies
changed strips into driver-owned coherent storage, and only then lets atomic
completion release the source framebuffer back to userspace. Encoding and USB
submission continue asynchronously from the immutable snapshot.

The Rust DRM framebuffer adapter exposes both borrowed and owned validated
shmem mappings. Vino keeps a round-robin cache of four source bindings per
head, so a compositor swapchain is mapped once and later commits only select a
prepared binding. Revdi uses the same facility for a four-buffer GRABPIX pool.
DPMS teardown drops each pool, and the fixed capacity prevents a reallocating
client from growing pinned memory without bound.

Damage is relative to the last frame successfully presented to the dock. It is
not relative merely to the previous atomic commit, because commits may be
coalesced and transfers may fail. Retransmission debt records strips still owed
to the dock's internal buffers.

## Work isolation

The control path runs on an ordered, device-owned queue. CPU-intensive strip
encoding runs on separate per-head queues. Long device waits do not occupy a
shared system workqueue, and teardown can cancel and drain every producer
before closing USB I/O.

## Teardown order

Disconnect and failed resume/reset follow one ownership boundary:

1. publish the stopping state and reject new work;
2. quiesce producers;
3. close the USB I/O window;
4. cancel and drain work, timers, and request queues;
5. unplug the DRM device;
6. unregister I2C and auxiliary objects;
7. drop snapshots, registrations, and session state.

The USB adapter owns the I/O window and invokes the driver's quiesce callback
before revocation. A consumer cannot reopen that capability independently.

## Source layout

| File | Responsibility |
|---|---|
| `vino.rs` | USB binding, session state, AKE/control orchestration, lifecycle |
| `drm_sink.rs` | DRM/KMS objects, desired state, scanout, damage, cursor, DDC/CI |
| `ake.rs` | HDCP wire message encoding and parsing |
| `hdcp.rs` | HDCP 2.2 derivations and verification |
| `cp.rs` | typed encrypted-control message builders and sealing |
| `proto.rs` | common USB framing and plaintext initialization |
| `video.rs` | WHT codec, strip records, and frame construction |
| `video_arm.rs` | generated decoder configuration for cold stream activation |
