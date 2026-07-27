# USB transport

## Frame header

Bulk protocol messages begin with a 16-byte little-endian header:

| Offset | Size | Field |
|---:|---:|---|
| 0 | 2 | reserved/direction |
| 2 | 2 | total frame size minus four |
| 4 | 4 | message type |
| 8 | 2 | subtype |
| 10 | 2 | auxiliary value |
| 12 | 4 | sequence or AES-CTR block counter |
| 16 | variable | body |

The common implementation is `drivers/gpu/drm/vino/proto.rs`. Parsers validate
declared lengths before returning a body view.

The auxiliary field is protocol-specific. In particular, video uses it for
record padding or fragmentation state; it must not be treated globally as
`body_length / 4`.

## Message classes

| Type | Use |
|---:|---|
| `1` | control/opening marker |
| `2` | plaintext device initialization |
| `4` | HDCP, encrypted control, and video data |

Control traffic is sent on EP02 and received on EP84. Video is plaintext after
the dock-specific compression and framing stage and uses the per-head bulk-OUT
endpoint.

## Queueing

EP84 remains persistently posted while the I/O window is open so unsolicited
or tightly timed replies cannot arrive between synchronous reads. Video uses a
bounded persistent request ring. Queue ownership, completion, cancellation,
and drain are implemented by the Rust USB layer rather than by raw URB pointers
inside Vino.

## I/O lifecycle

A device being bound is not sufficient proof that USB I/O is currently legal:
suspend, reset, failed resume, and disconnect narrow that interval. The Rust USB
adapter creates an I/O window after successful probe/resume and revokes it only
after asking the driver to quiesce producers. Transfers require that capability.

This separation follows the USB review feedback from the v2 series: interface
ownership and device-wide control are not interchangeable, and safe transfer
APIs must encode when I/O is permitted.

