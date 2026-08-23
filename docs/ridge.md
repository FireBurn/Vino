# Ridge (Dell D6000, DL-6xxx)

What the vendor does on this dock, measured, and where vino differs. The reference is
`captures/ridge-dlm-ref-20260822`: DisplayLinkManager 3.4.26 run by hand, driving a D6000 on
firmware 12.2.25 (`bcdDevice 31.59`) with an MSI MAG 27CQ6F on socket 2, captured whole -- 841 s,
1362 control-plane records decrypted, plus the video endpoint. vino's side is
`captures/ridge-vino-ref-20260822`, same dock, same monitor, same bus, minutes apart.

⛔ An earlier handover said no like-for-like DLM reference existed for this dock and that only a
July capture on older firmware was available. Both halves are wrong. The corpus holds dozens of
D6000 DLM captures, every recorded `pid=6006` is `bcdDevice=3159`, and no `.spkg` for this family
ships in `/opt/displaylink`, so nothing has ever flashed it. The firmware has not moved.

## The dock has one EDID handler, shared between its heads

This is the fact the rest of the document hangs on, and it explains a dark panel at cold boot and a
connector that ping-pongs between sockets at runtime as the same bug.

A `0x15/0x21` fetch does not read the monitor on the head named at offset 22. It reads whichever
head the dock's EDID handler is currently engaged for. Engaging it for one head disengages it for
the other.

Two consequences, both measured on the wire:

* **Before the handler is engaged, a fetch returns a block the dock synthesises for itself.** DLM's
  first fetch on socket 2 returns 256 bytes describing a `NVT` "NOVATEK" panel whose preferred
  timing is 1920x1080. Its second, 2.4 s later, returns 384 bytes describing the `MSI` "MAG 27CQ6F"
  whose preferred timing is 2560x1440. Same socket, same cable, same session.
* **Fetching on an empty head returns the other head's monitor.** vino re-engaged socket 1, which
  has nothing plugged into it, and the dock answered with the MSI EDID -- `socket 1 EDID 384 B,
  vendor MSI product 0x3cd9`.

### The readiness bit says which answer you are getting

The presence reply `id=0x44 sub=0x20` carries a status word at inner offset 22 and a readiness byte
at offset 26:

| socket | inner 22..26 | offset 26 | what the next fetch returns |
|---|---|---|---|
| empty | `05 01 20 00` | `00` | the other head's monitor, or nothing |
| occupied, handler not engaged | `05 11 27 00` | `00` | the dock's own 1920x1080 block |
| occupied, handler engaged | `05 11 27 00` | `80` | the monitor |

Four cases across two independent captures, and the bit agrees with the payload every time.
`cp::probe_reply_status` already decodes both fields -- `status` from 22..26 and `ready` from
offset 26 bit 7. Nothing acted on `ready`.

⇒ **An EDID fetched while `ready` is clear describes the dock, not a monitor. Discard it.**

## The dock announces changes; it is not polled

DLM sends four `0x15/0x20` presence probes in 841 seconds. Three belong to the bring-up sequence.
The fourth follows an unsolicited `IN id=0x03 sub=0x82` push by 0.1 ms -- that push is the dock
saying its downstream state changed, and DLM's entire runtime hotplug policy is to probe when it
arrives.

vino sends 78 probes in 45 s, on both heads, for the life of the session. Every probe is a chance
to read a transient negative on a head that is lit, and that is how each observed failure starts:

```
socket 2 presence reply id=0x0044 status=0x00200105 -> present=false ready=false (was 0x1105)
2-connector dock: socket 2 presence cleared
socket 2 monitor disconnected
socket 1 absent -- retrying the sink re-engage      <- steals the EDID handler
socket 1 monitor connected after sink re-engagement <- the MSI EDID, on the empty socket
socket 1 monitor disconnected
socket 2 monitor connected after sink re-engagement
```

## The bring-up sequence, as the vendor sends it

Offsets are into the decrypted inner plaintext. `off22` is the head selector; on this dock the
per-head setup burst carries a **one-based** head number at `off23` (1 and 2), not a one-hot bit.

```
plaintext init      0x05/0x08, 0x04/0x00 -> 0x15/0x90, 0x14/0x76,
                    0x22/0x10, 0x1f/0x10 -> 0x213/0x84 (cert), 0x9a/0x10, ...
seal                0x04/0x06
dock descriptor     0x14/0x00 -> 0x4c/0x00
dock capability     0x14/0x30 -> 0x78/0x30                (160 B, dock-wide)
                    0x15/0x0b -> 0x14/0x0b                   <- vino does not send this
                    0x16/0x2a off22=0 off23=1                 <- nor these
                    0x16/0x2a off22=1 off23=1
per head 0 (off23=1)  0x22/0x10, 0x1f/0x10, 0x9a/0x10, 0x22/0x10, 0x32/0x10,
                      0x2a/0x10, 0x26/0x10, 0x14/0x30 -> 0x78/0x30, 0x19/0x31
per head 1 (off23=2)  the same burst
per connector         0x16/0x4c, 0x15/0x4a -> 0x26/0x4a, 0x16/0x4c   (x2 per head)
presence              0x15/0x20 off22=0 -> 05 01 60 ...   (empty)
                      0x15/0x20 off22=1 -> 05 01 61 ...   (not yet engaged)
                      0x16/0x4b off22=1 off23=1           (readiness kick)
                      0x15/0x20 off22=1 -> 05 11 27 00 00 ...
EDID (placeholder)    0x15/0x21 off22=1 -> 0x114/0x21, 256 B, NOVATEK 1920x1080
engage                0x16/0x23 off22=1 off23=1
post-EDID query       0x15/0x53 off22=2 -> 0x1c/0x53
first set-mode        0x48/0x22   1920x1080@60, off42=0x0600 off66=0x0800
   ~2.4 s later, the dock pushes 0x03/0x82
presence              0x15/0x20 off22=1 -> 05 11 27 00 80 ...    <- ready
EDID (real)           0x15/0x21 off22=1 -> 0x194/0x21, 384 B, MSI MAG 27CQ6F 2560x1440
second set-mode       0x48/0x22   1920x1080@60, off42=0x0400 off66=0x2810
third set-mode        0x48/0x22   2560x1440@120
```

`id = 0x14 + payload length` holds for every reply here: `0x114` is 256 bytes of EDID, `0x194` is
384, `0x78` is a 100-byte capability block. Match the `sub`, never a literal id.

### Set-mode

All three target `off22=1`, `off23=2`. Decoded, with `off70` in units of 10 kHz:

| | off26 h | off28 hbl | off30 hfr | off32 hsy | off34 v | off36 vbl | off42 | off44 | off46 | off48 | off66 | off68 | off70 |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| #1 | 1920 | 280 | 88 | 44 | 1080 | 45 | `0x0600` | 60 | `0x4000` | `0x6000` | `0x0800` | `0x0200` | 14850 |
| #2 | 1920 | 280 | 88 | 44 | 1080 | 45 | `0x0400` | 60 | `0x4000` | `0x6000` | `0x2810` | `0x0200` | 14850 |
| #3 | 2560 | 160 | 48 | 32 | 1440 | 85 | `0x0600` | 120 | `0x4000` | `0x6000` | `0x0800` | `0x0200` | 49775 |

`off46`/`off48` are `0x4000`/`0x6000` at both resolutions, which is why `Allocation::Fixed` is right
for this family and `MeasuredPair` was refuted.

⚠ #1 and #2 carry identical timings and differ in `off42` and `off66` alone, so on this dock those
two words do not follow the timing. They follow whether the mode has a CTA VIC, which DLM only
knows once it holds the real EDID.

### The mode-set bracket

```
0x16/0x2f off22=1 off23=1
0x16/0x2e off22=1 off23=3      <- sink down
0x48/0x22                      <- the timing
0x16/0x2f off22=1 off23=1
0x16/0x2e off22=1 off23=3      <- down again
0x16/0x2f off22=1 off23=1
0x16/0x2e off22=1 off23=0      <- up
```

vino's `pre_mode_sink_state = Some(3)` reproduces the leading half. Its
`post_mode_sink_states = [0, 0]` does not reproduce the trailing half, which is `[3, 0]`.
Every sink-down the vendor sends on this dock is state **3**; `DockProfile::sink_down_state` is
`1`, which no message in this capture supports.

## Throughput is not the problem -- measured, so it is not re-chased

Peak bytes in a sliding window on `ep 0x0b`, the endpoint that carries socket 2:

| window | DLM | vino |
|---|---|---|
| 0.10 s | 130.5 MB/s | 138.9 MB/s |
| 1.00 s | 120.2 MB/s | 132.1 MB/s |
| 5.00 s |  99.1 MB/s |  64.3 MB/s |

DLM holds 120 MB/s for a second and 99 MB/s over five, and sustains 3.34 GB across the session.
vino is within ten percent of that and below it over five seconds. ⛔ A `StreamPacing` envelope for
this family would be treating a symptom that is not present. The `EPROTO` that halts `ep 0x0b` has
another cause.

## The dock announces; do not poll it

DLM sends four `0x15/0x20` presence probes in 841 s. Three are part of the bring-up sequence. The
fourth follows an unsolicited `IN id=0x03 sub=0x82` push by 0.1 ms -- the dock saying its downstream
state changed. That push is the whole of the vendor's runtime hotplug policy.

vino sends 78 probes in 45 s, on both heads, for the life of the session, and every one is a chance
to read a transient negative on a head that is lit. That is how each observed failure began.

⇒ Open work: act on `0x03/0x82` and stop the timed probe on this family.

## What was fixed, and what it looked like

Two defects, one cause -- the shared EDID handler.

**Cold boot took the dock's own EDID.** `session.rs`'s early discovery loop accepted the first block
that arrived, which on a cold dock is the NOVATEK 1920x1080 descriptor. The panel was then driven at
a timing it never advertised. Acceptance is now gated on the readiness bit
(`DockProfile::shared_edid_handler`), and on hardware the gate fires and is followed by the real
EDID:

```
socket 2 discarding an EDID offered before the downstream read completed
socket 2 monitor connected
cached socket 2 EDID (384 bytes)
```

⚠ **This is why the bug was intermittent and why measurements disagreed.** On a *warm* rebind the
handler is still engaged from the previous session, so the first fetch returns the real EDID and
everything looks correct. Only a cold dock reproduces it. A measurement taken after a rebind proves
nothing about a cold boot.

**A monitor that moved between sockets.** The keepalive spent a blind re-engage on socket 1, which is
empty. That engaged the handler for a head with nothing on it, and the fetch that followed returned
socket 2's monitor -- so the MSI EDID was published on socket 1, socket 2 was torn down, and the two
sockets swapped back and forth:

```
socket 2 monitor disconnected
socket 1 absent -- retrying the sink re-engage
socket 1 monitor connected after sink re-engagement   <- the other socket's monitor
socket 1 monitor disconnected
socket 2 monitor connected after sink re-engagement
```

A head the probe reports absent is no longer engaged on a dock with a shared handler. Verified: zero
`monitor connected after sink re-engagement` events across a full session.

## ✅ SOLVED: the endpoint stopped accepting because a frame was split in two

The symptom was the video endpoint refusing further writes under sustained damage while reporting
itself healthy:

```
head=1 endpoint=0x0b stopped accepting video: GET_STATUS=0x0000 halt=0
scanout head=1 pipeline submit at off=0/30048 failed
```

⭐ **The cause is `never end a frame on a full packet`, added for a different dock and applied to
this one.** A frame whose length `N` is a whole number of 1024-byte packets was sent as two
transfers, `N - 16` and `16`. `N - 16` is `1008` modulo 1024, so the *first* transfer ends on a
short packet as well -- and a dock that delimits frames by a short packet therefore sees the frame
end sixteen bytes early and a stray sixteen-byte frame behind it.

The split cannot do what it is named for. There is no way to divide a multiple of the packet size
into two transfers where only the last is short: if `A + B` is a multiple and `A` is too, `B` must
be. Ending such a frame short needs a zero-length packet.

**The evidence, three independent captures:**

| capture | 16-byte transfers | first failure after the first one |
|---|---|---|
| `ridge-vino-fixed-20260822` | 2 | **49.0 ms** |
| `ridge-vino-shake-20260822` | 1 | **46.8 ms** |
| `ridge-vino-subbit-20260822` | 3 | **50.7 ms** |

The middle capture contained exactly one such transfer and exactly one failure cascade. DLM emits
none on this endpoint and never fails. vino's own known-good capture from July
(`vino-exact-wire-20260722-031912`) predates the change.

Now `DockProfile::split_full_packet_frame`: `true` for DL-3x00, where it was measured, `false` for
DL-6xxx and DL-7000. ✅ **HW-verified**: the shake that reproduced the fault every time no longer
does, and the wire shows **0 sixteen-byte transfers and 0 failing completions** across 2865 frames.

### ⛔ What this cost, and the lesson

Five hypotheses were built and tested on hardware before this one, each on a real measured
divergence from the vendor, and each wrong:

| tried | measurement that killed it |
|---|---|
| throughput / `StreamPacing` | vino sits inside DLM's envelope, which DLM holds for 3.34 GB |
| ring-slot accounting | both streams show slots `0/2/4` in equal thirds, same opening |
| sustained frame rate | paced to the vendor's floor and then its median; still failed |
| the steady-state record `sub` bit | implemented and tested; still failed -- and vino's own known-good era never set it |
| presentation coverage floor | raising vino to the vendor's 1800-strip floor made it fail *sooner*, at the vendor's byte rate and at twice it |

⭐⭐ **The lesson: the report said "this used to work". That makes it a regression, and a regression
is found by diffing against the last good version of your own driver, not by decoding the vendor
harder.** Every one of the five was a genuine difference from DLM, and none of them was the change
that broke this dock. The July vino capture and `git log` over the video path since the pre-DL-3x00
commit would have named the suspect in minutes.

## Which Navarro-era fixes reach this dock

Asked directly, and answered against the wire rather than the changelog.

| fix | reaches Ridge? |
|---|---|
| ring slot spent only on a frame the dock received (`be8d71890581`) | ✅ shared code, and Ridge's wire shows slots `0/2/4` in equal thirds, matching the vendor exactly |
| never end a frame on a full packet (`07124a5a3ca1`) | ✅ `BULK_MAX_PACKET` is a plain constant, not profile-gated. Checked anyway: none of the failing frame sizes is a multiple of 1024 |
| come back from the reset that recovers a wedged dock (`53b8fa4cd159`) | ✅ generic, and it is why the dock now returns by itself |
| build as many heads as the dock has sockets (`5ba58eae7cdd`) | ✅ Ridge publishes exactly two connectors |
| derive the framebuffer allocation (`88e0ab5de5e6`) | ⛔ deliberately not applied, and correctly so: the vendor sends `off46=0x4000 off48=0x6000` at both 1920x1080 and 2560x1440, so `Allocation::Fixed` is right |
| parameter map among the records (`6112303b251d`) | n/a, DL7400 only |
| **open a stream with the carrier frames the vendor sends** (`6470d5ff7d9d`) | ⛔ **never applied.** Ridge is still `carrier_frames: u32::MAX`, bounded by a wall-clock window -- the exact shape that fix removed from the DL7400 |

⚠ **The carrier divergence is measured.** The vendor spends **27 presentations** before its first
steady-state image record. vino spends about 360: roughly 45 submissions at
`COLD_TRAINING_PRESENTATIONS = 8` each. Every one walks the dock's ring and steps its frame counter,
which is the reasoning that produced the DL7400 fix. Not changed here, because the stall happens
long after training and the bring-up path is currently reliable -- but it is the first thing to try
if the load capture does not settle it.
