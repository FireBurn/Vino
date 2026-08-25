# Navarro (DL-7000) — how it differs from Ridge

The WAVLINK DL7400, `17e9:7000`, "Universal DP Quad Display Docking 16G", identity tail
`NavaDock`. Everything here is measured from hardware or from
`captures/navarro-dlm-session-20260801-190654` (247 MB of DLM driving this dock with a monitor lit,
8 key candidates, decrypts with `tools/capture/decrypt-dlm-cp.py`).

## Status

vino drives this dock as a dual-head KMS device with a picture on both panels, confirmed by eye at
**2560x1440p120, 30 bpp, PQ** — the monitors' own OSD reads `10 Bit`. Control session, EDID, 36
EDID-derived modes per head, both connectors `connected`, compositor driving them.

What is known about its limits, each measured rather than inferred:

* **30 bpp needs every escape ceiling raised, and each is stated by its own code table.** Raising
  the encoder's ceiling without raising the table is the failure that looks like corruption; see
  [`hdr.md`](hdr.md) §0.2a for the mechanism and the table-to-plane mapping.
* **2560x1440p165 runs at 24 bpp, not 30.** Two connectors at that refresh is exactly the dock's
  pixel budget, and three quarters of the budget is what a 30 bpp connector is priced against, so
  the depth gives way. Raising the budget to the dock's rated four-connector 4K60 load admits the
  pair and **both sinks then power off with nothing logged**, so the rated figure is not a bound
  this budget can borrow.
* **A warm plug can be answered from the dock's own descriptor.** See §4a.

## 1. Descriptors

| | D6000 (Ridge) | DL7400 (Navarro) |
|---|---|---|
| speed | SuperSpeed 5 Gbps | **SuperSpeed+ 10 Gbps** |
| control | bulk OUT `0x02`, bulk IN `0x84` | same |
| video endpoints | `0x08`, `0x0a`, `0x0b`, `0x0c` (**four**) | **`0x08`, `0x0a` (two)** |
| heads driven | 2, on `0x08` and `0x0b` | 2, on `0x08` and `0x0a` |
| DFU | iface 1, `wTransferSize` 16384 | same |

⚠ Four DisplayPort connectors but only two video endpoints, and DLM brings up exactly two outputs
(`DVI-I-1`, `DVI-I-2`). DisplayLink's own strings describe **"Dual NIVO"** with `TiledNivoViewer`
and `TiledDisplayGroupingStategy`, so four outputs are most likely two streams each carrying a
tiled pair. Driving four needs the tiling protocol, which is not reverse-engineered.

## 2. ⭐ Per-connector HDCP authentication

**Corrected 2026-08-03.** Navarro runs a full fresh HDCP exchange for every physical connector.
Decrypting the complete EP02 capture with the control key shows nine records per connector:
`AKE_Init` → `AKE_Transmitter_Info` → `AKE_No_Stored_km` → `LC_Init` → `SKE_Send_Eks` →
`LC_Send_L_prime` → `RepeaterAuth_Stream_Manage` → stream-open control → strm2.

The four SKEs occur at inner counters 20, 29, 38 and 47. Each has a fresh raw SKE and RIV; its
`Edkey` is standard HDCP `compute_eks` from the connector-local `(km, rtx, rrx, rn)`. DLM converts
the raw SKE to the video key with `raw_ske XOR CP_KEY_WHITEN` and uses the delivered RIV unchanged.

Navarro differs from Ridge only in the selector byte: its AKE/SKE records set one-hot
`content[22 + connector] = 0x80`; Ridge uses a one-based head number in `content[23]`.

⛔ **"uses the delivered RIV unchanged" is RETRACTED (2026-08-03).** The AES-CTR nonce is the
delivered RIV with **`byte7 ^= stream_id`**, where `stream_id = (connector << 3) | 7`. Measured by
decrypting the control plane of `captures/navarro-dlm-modeset-20260802-005453` with the Dl3Cmac tag
as the oracle and comparing each connector's wire RIV against the nonce the live sealer then used:

| stream | wire SKE RIV | id | RIV with `byte7 ^= id` | sealer key |
|---|---|---|---|---|
| link | `7d2cb66b2cd1757c` | `0x04` | `7d2cb66b2cd17578` | `709b61dd…` ✔ |
| connector 0 | `c345fe5593613901` | `0x07` | `c345fe5593613906` | `960b47bf…` ✔ |
| connector 1 | `9446c83da5fa39e3` | `0x0f` | `9446c83da5fa39ec` | `e5151879…` ✔ |
| connectors 2, 3 | `cfa01956e0edb145`, `6918593e8dc4be7d` | — | — | no monitor, no sealer built |

⭐ This is Ridge's rule, generalised. Ridge xors byte 7 with `0x08 | head`, which is precisely
Ridge's stream id, so **one rule covers both platforms**: the tweak is the stream's content-stream
id — the wire `sub` of its control records, and the value its `RepeaterAuth_Stream_Manage`
restatement declares at offset 36. vino already generates the RIV, so nothing here needs capturing.

⇒ The handover's "distinct recovered first-pipe key/RIV pair `960b47bf…` / `c345fe55…`, not
associated with Vino's per-connector SKE sealers" is **connector 0's own SKE sealer**. There is no
separate first-pipe key.

## 2a. ⭐ There are three keys, and the video keys are per-head

> **Superseded 2026-08-03.** The analysis below was based on an incomplete decrypted control
> stream. The missing nine-message per-connector AKE/SKE sequence is now decoded in §2; the keys
> are delivered by those fresh SKEs, not derived locally from the main-link session.

Measured 2026-08-02 from `captures/navarro-dlm-session-20260801-190654`, using the Dl3Cmac tag as
the oracle. That is the right tool here: it verifies a candidate `(key, nonce)` against the captured
frame with **no assumption about the plaintext**, where a plaintext-shape guess silently fails when
the guess is wrong — which is exactly what happened on the first attempt at this.

| stream | key | nonce |
|---|---|---|
| control (`0x02`/`0x84`) | `44514ae4ad1ea7f1cf272d8dfabea994` | `09d8c99a45b38dc8` |
| video head 0 (`0x08`) | `989f349c3c8edfb07fe9196921d1868f` | `566c8be978e96150` |
| video head 1 (`0x0a`) | `67e0309ac4c19bdf23b147a2e0662298` | `367ef5b61c3d66ac` |

Frida records each twice, the second being the same key with `nonce[0] ^ 0x80` — the Dl3Cmac nonce.

⛔ The video keys are **not** the control key and **not** derived from it by any simple relation:
`ctrl^H0`, `ctrl^H1`, `H0^H1`, `ks^H0`, `ks^H1` show no structure, and neither AES nor CMAC of the
link key or `ks = ctrl ^ B` under small counters, `B`, or any of the three nonces reproduces them.

⛔ They are **not transmitted**. Decrypting all 1100 sealed control frames with the control key and
searching for each video key, each key `^ B`, and each nonce yields nothing; the nonces do not
appear in the clear anywhere in 5546 frames either.

⭐ And they cannot come from extra handshakes, because **vino and DLM run the identical HDCP
message set, exactly once each**: `AKE_Send_Cert`, `AKE_Send_rrx`, `AKE_Send_H_prime`,
`AKE_Send_Pairing_Info`, `LC_Send_L_prime`, `RepeaterAuth_Send_ReceiverID_List`. One session, three
keys.

⇒ **The per-head video keys are derived locally, by both ends, from the single link session.** vino
therefore already holds all the input material; what is missing is only the derivation. Finding it
means reading DLM's key schedule in the binary — a wire capture cannot show a local computation, so
**a cold-plug capture would not answer this**.

## 2b. ⭐ The key schedule: one sealing key per wire sub

Traced 2026-08-02 with `tools/capture/keysched-backtrace.py`, which attaches to a live DLM and
records the callers the first time each distinct key appears. ⚠ The key schedule is **dormant on a
warm dock** — the setter fires zero times until a fresh session, which a
`echo 0 > .../authorized; echo 1` re-enumeration is enough to force.

A cold session creates **exactly five** sealing keys, all through one call site, and the factory
takes a selector:

| selector (`ecx`) | as hex | stream |
|---|---|---|
| 4 | `0x04` | control plane |
| 7 | `0x07` | **connector 0** stream-open |
| 15 | `0x0f` | **connector 1** stream-open |
| 23 | `0x17` | **connector 2** stream-open |
| 31 | `0x1f` | **connector 3** stream-open |

⚠ **Corrected 2026-08-02.** This table previously read 7/15 as "head 0/1 video *frames*" and 23/31 as
"head 0/1 stream-open". That was wrong, and it mattered: it made the dock look like it had two heads
with two keys each, when it has **four connectors with one key each**.

All four are `(connector << 3) | 7`, and the wire proves it. Counting record subs per video endpoint
over a session that lit all four connectors in turn
(`captures/navarro-portmap-lit2-20260802-114059`):

| endpoint | frame records | stream-open |
|---|---|---|
| `0x08` | `sub=0x0000` ×297 (conn 0), `sub=0x0010` ×654 (conn 2) | `0x0007` ×1, `0x0017` ×1 |
| `0x0a` | `sub=0x0008` ×747 (conn 1), `sub=0x0018` ×155 (conn 3) | `0x000f` ×1, `0x001f` ×1 |

A *frame* sub occurs hundreds of times; `0x07`/`0x0f`/`0x17`/`0x1f` occur **exactly once each**, on
the endpoint owning that connector. Once-per-stream is a stream-open, not a frame key. And there is
no frame key to find because Navarro's pixels are plaintext (§4).

⇒ **`sub = connector << 3` for frame records, `(connector << 3) | 7` for the stream-open**, which
also generalises `DockProfile::head_sub_shift = 3` from heads to connectors.

⭐ **The selector is the wire `sub`.** Every value matches the subs already measured in §4 and §4b,
including the eight-apart head spacing — so a sealing key is chosen per message type per head, and
the stream-open has its own key distinct from the frame key.

Offsets in DLM 3.4.26 (module-relative), from `objdump`:

| offset | role |
|---|---|
| `0x86cca0` | sealer factory `(out, cfg, keysrc, sub)` — copies key and riv out of `keysrc`, allocates a `0x90`-byte sealer, constructs it with the sub |
| `0x85c830` | key copy-in, key at `keysrc+0x18`, 16 bytes |
| `0x85c850` | riv copy-in, riv at `keysrc+0x30`, 8 bytes |
| `0x85c5f0` | the wrapper that calls the factory; selector arrives in `ecx` |

⇒ **key-source object layout: key at `+0x18` (16 B), riv at `+0x30` (8 B).**

The full key-source object, captured for all five subs in one session:

```
+0x00 vtable   +0x08 flag(1)   +0x10 ptr
+0x18 KEY (16 bytes, inline -- not a std::string)
+0x28 ptr      +0x30 RIV (8 bytes, inline)
+0x38 u32: 0x45, 0x25, 0x25, 0x35, 0x45 for subs 04/07/0f/17/1f
```

⛔ **The five keys are mutually independent.** Tested every pairwise XOR and, against the control
key, `AES(k4, sub)`, `AES(k4, riv|sub)`, `AES(k4, own_riv|0)`, `AES(k4, own_riv|sub)`,
`CMAC(k4, sub)`, `CMAC(k4, riv|sub)` and `CMAC(k4, own_riv)`. Nothing matches, so there is no cheap
per-sub tweak of one master key — each is separately derived from session material.

⊙ **Next step:** the factory only *copies* an already-derived key, so the derivation is whatever
fills `keysrc+0x18`. `tools/capture/keysrc-writer.py` arms a hardware watchpoint there and is
mechanically working (frida 17.9.1 has `Thread.setHardwareWatchpoint`), but it still needs the
right site to arm from: ⛔ **`0x85c560` is not the constructor** — it is never called in a live
session, so reading it as one from nearby disassembly was wrong. Find where the object is actually
allocated (e.g. hook `0x86cca0`, keep the `rdx` pointer, and arm on that address before the *next*
session builds into it).

⚠ Backtraces past frame #0 here come from the fuzzy unwinder and are **not reliable** — chasing
frame #6 of one led straight into iostream formatting code. Trust `this.returnAddress` and the
accurate frame #0 only.

## 2c. ⭐⭐ FOUR connectors, not two — the presence probe enumerates 0..3

Measured 2026-08-02 from `captures/navarro-portmap-20260802-111222`, a capture recorded while two
DisplayPort cables were physically moved between the dock's four sockets. That move is the only
experiment that separates *port* from *head*: every earlier capture used two cables that never
moved, so the two were confounded.

**The `id=0x15 sub=0x20` presence probe carries the connector selector in payload byte 22 — the
same field Ridge uses for its head selector — but Navarro drives it 0, 1, 2, 3 where Ridge only
ever emits 0 or 1.** Measured selector counts in one session: `{0: 8, 1: 4, 2: 3, 3: 3}`.

⇒ On Ridge, "byte 22" is a *head*. On Navarro it is a *connector*, and there are twice as many
connectors as there are video endpoints.

### Presence is read out of the reply, not inferred

The `id=0x44 sub=0x20` reply carries the state at payload offset 21. Every distinct reply seen:

| payload[21:34] | b[23] | b[24] | meaning |
|---|---|---|---|
| `00051127008001000700000000` | `0x11` | `0x27` | **monitor present** |
| `00051127000001000700000000` | `0x11` | `0x27` | **monitor present** |
| `00050161000000000700000000` | `0x01` | `0x61` | empty |
| `00050160000000000700000000` | `0x01` | `0x60` | empty |
| `00050121000000000700000000` | `0x01` | `0x21` | empty |
| `00050120000000000700000000` | `0x01` | `0x20` | empty |

⇒ **bit `0x10` of byte 23 is the presence bit.** A present connector also carries two 32-bit words
further along (`2aea5400`, `22551000` little-endian) which the dock's own firmware trace prints back
as arguments to the same event — see below.

⛔ Do **not** reuse Ridge's presence rule here. On Ridge presence was "a recovered EDID", and removal
was "the dock stops answering that head's probe"
([[project_dpms_blank_and_hpd_instruments_20260727]]). Navarro answers `id=0x44` for **all four**
connectors whether or not a monitor is attached, so the `0x44`-vs-`0x14` distinction says nothing
here and the sustained-silence timeout would never fire.

### Hotplug is pushed by the dock

Ridge never pushed a hotplug event; vino had to notice a probe going unanswered. Navarro **pushes**,
on the `sub=0x0c` channel, and DLM is purely reactive:

```
dock -> host   id=0x3d  sub=0x0c        (event)
dock -> host   id=0x106 sub=0x0c        (detail)
dock -> host   id=0x401/0x414/0x68      (burst)
host -> dock   id=0x15 sub=0x20         probe THAT connector
host -> dock   id=0x15 sub=0x21  -> id=0x194   EDID read
host -> dock   id=0x16 sub=0x23         engage
```

Measured end to end at a cable move: `0x3d` at t+0.000, EDID at t+0.10, engage at t+0.11.

### The dock's firmware trace is tagged with the connector index

The `sub=0x0c` channel also carries the dock's ASCII firmware log (`|2<ticks> <msgid> <args>`, see
`scripts/dock-trace-mon.py`). **The low nibble of a per-connector `msgid` is the connector index**,
and it is usually repeated as an argument. The same bring-up burst, recorded three times for three
different connectors:

| connector 0 | connector 2 | connector 3 |
|---|---|---|
| `7d868`**`0`** ` 3` | `7d868`**`2`** ` 2` | `7d868`**`3`** ` 2` |
| `6bf21`**`0`** ` 0 2` | `6bf21`**`2`** ` 2 2` | `6bf21`**`3`** ` 3 2` |
| `63ad7`**`0`** ` 0` | `63ad7`**`2`** ` 2` | `63ad7`**`3`** ` 3` |
| `777a1`**`0`** ` 0 5 27 2aea5400 22551000` | `777a1`**`2`** ` 2 5 27 …` | `777a1`**`3`** ` 3 5 27 …` |

Sixteen message ids in the burst differ *only* in that nibble. Ids that are genuinely global
(`72b0b12`, `72b0b14`, `60fbc0`) do not carry it. This gives a free oracle: the dock will tell you
which connector it is acting on, with no decryption of anything but the `sub=0x0c` channel.

### ⭐ Socket ↔ index: `index = physical port − 1`

Settled 2026-08-02 from `captures/navarro-portmap-lit2-20260802-114059`, a **lit** session (dock
power-cycled with the recorders already running) in which two cables were walked from sockets 1,2 to
sockets 3,4. The dock's own trace timestamps the events far more precisely than any hand-written
mark, and they line up one-for-one:

| t (epoch) | dock trace | physical action |
|---|---|---|
| 1785667361.3 | connectors **0** and **1** come **up** | power cycle, cables in sockets **1** and **2** |
| 1785667371.9 | connector **0** goes **down** | unplug socket **1** |
| 1785667375.8 | connector **2** comes **up** | plug socket **3** |
| 1785667391.5 | connector **1** goes **down** | unplug socket **2** |
| 1785667395.0 | connector **3** comes **up** | plug socket **4** |

⇒ **connector index = physical socket number − 1.**

The two bursts are distinguishable: an **arrival** includes `777a1<p>` (the one carrying the
capability words `2aea5400 22551000`) and `63ad7<p>`; a **removal** is the shorter
`7d868<p>` / `6bf21<p>` / `75acf<p>` / `6ba5a<p>` / `67525<p>` with no `777a1<p>`.

### ⭐ The two video endpoints follow the connectors

Same session, video bytes per 5 s bucket against those events:

| window | ep `0x08` | ep `0x0a` | dock event |
|---|---|---|---|
| rel 105 | **0 MB** | 136.6 MB | connector **0** down at rel 103 |
| rel 110 | 111.5 MB | 265.9 MB | connector **2** up at rel 107 |
| rel 125 | 222.3 MB | **0 MB** | connector **1** down at rel 122 |
| rel 130 | 267.3 MB | 158.3 MB | connector **3** up at rel 126 |

⇒ **`0x08` carried connectors 0 then 2; `0x0a` carried connectors 1 then 3.** An endpoint goes
silent exactly when its connector is unplugged and resumes when the replacement appears.

⭐⭐ **Settled 2026-08-02: it is even/odd pairing.** `captures/navarro-pair-ports13-20260802-120204`
has monitors in sockets **1 and 3** — connectors **0 and 2**, i.e. both members of one pair — both
lit, both driven by the compositor, with windows dragged across both panels so both were generating
damage:

| device | ep `0x08` | ep `0x0a` |
|---|---|---|
| 72 | 57018 URBs, **1813.7 MB** | 10 URBs, **0.0 MB** (112/48/64 B control only) |
| 66 (earlier session, same cables) | 6406 URBs, **203.7 MB** | — |

Two independent sessions. Under slot allocation two lit monitors must take one endpoint each; they
did not — everything went down `0x08` while `0x0a` carried no payload at all.

⇒ **endpoint `0x08` owns connectors {0, 2}; endpoint `0x0a` owns {1, 3}.** This matches
DisplayLink's own `TiledNivoViewer` / `TiledDisplayGroupingStategy` strings and the "Dual NIVO"
description: two streams, each able to carry a tiled *pair*.

### ⭐⭐ Four outputs are MULTIPLEXING, not more channels

The dock is a "Quad Display Docking" with two video endpoints, and the way those reconcile is now
measured rather than guessed. In the ports-1-and-3 capture, connectors **0 and 2 were lit at the
same time**, both being damaged, and `0x08` carried records under **two different tags**:

```
ep 0x08   type=4  sub=0x0000  x186     <- connector 0
ep 0x08   type=4  sub=0x0010  x82      <- connector 2
```

⇒ one endpoint interleaves two independent connectors, discriminated by `sub = connector << 3`.
Not a tiled single surface — two separately-tagged record streams.

⛔ There is no third or fourth channel to look for. Across both lit captures the only dock endpoints
carrying anything are `0x02`/`0x84` (control, sub-MB), `0x08`/`0x0a` (all the video), and
`0x80`/`0x83` (ep0 and the audio interrupt, a few KB). Interface 0 exposes exactly two video
bulk-OUTs, so **4 outputs = 2 endpoints × 2 multiplexed connectors**.

All four connector tags have been observed in use (`0x00`, `0x08`, `0x10`, `0x18`), and both
endpoints have been observed busy simultaneously (791 MB and 780 MB in one run) — but never four
monitors at once, because only two were ever available.

⊙ Still open: bandwidth admission when both members of a pair run at high resolution. Two 1440p
streams share one endpoint's budget, so the dock's `pixel_per_second_limit` must be enforced per
*endpoint pair*, not per connector.

### 4a. ⛔ The dock answers an early EDID fetch from itself

A fetch this dock cannot yet answer from the monitor is answered from its own bridge: a 256-byte
block, vendor `NVT`, product `0x079c`, describing a 1920x1080 panel at 148.5 MHz. It carries valid
base-block magic and a valid checksum, so nothing structural rejects it, and publishing it drives
the sink at a timing it never advertised — a garbled picture at 1080p60. The real monitor block is
384 bytes with two extension blocks.

Two things guard it, and the first is not enough on its own:

* Offset 26 bit 7 of the presence reply says the downstream DDC read has completed. Gating on it is
  necessary but **not sufficient**: the dock can report readiness and still answer from itself.
  ⚠ The gate is per dock (`edid_ready_reported`) and is *not* the same property as
  `shared_edid_handler` — Ridge happens to have both, which is why one flag covered it there and
  Navarro went ungated. A dock that never reports readiness must not be gated on it, or discovery
  discards every block it is given.
* The block is refused on identity, by vendor and product id. That is what makes "we never see the
  default EDID" hold.

### ⚠ Two gotchas that cost measurements here

1. **A USB re-authorise is not a dock reset.** `echo 0 > authorized; echo 1` reliably forces a fresh
   *session* (the key schedule is dormant otherwise), and DLM will run the whole control plane, read
   real EDIDs and publish a correct mode list — **and the panels stay dark**. Only a physical power
   cycle relights them. Do not read "control plane came up" as "the dock is driving".
2. **udev restarts DLM on every dock re-enumeration.** `/lib/udev/rules.d/99-displaylink.rules` runs
   `/opt/displaylink/udev.sh` on any add/remove of a 17e9 DL3 interface, which bounces
   `displaylink-driver.service`. A frida hook attached to the old pid then captures **nothing** for
   the new session while still looking healthy — that is how a lit power-cycle capture ended up
   undecryptable. Run DLM **by hand with the unit masked**;
   `tools/capture/capture-portmap.sh` now does exactly that, and
   `decode-modeset-live.py --reattach` is the belt-and-braces fallback.

### What this means for vino

✅ **Done.** vino probes selectors `0..connectors-1` — four on Navarro, two on Ridge — reads
presence from bit `0x10` of reply byte 23 (bit `0x1000` of the status word), and acts on the dock's
pushed `sub=0x0c` events rather than on probe silence. The connector count is profile data; the
presence rule turned out to be right for **both** docks, so it is shared rather than Navarro-only.

## 3. The main AKE is plaintext-framed

The dock's `AKE_Send_Rrx` push (`id=0x10 sub=0x84`, HDCP msg-id `0x06`) arrives with wire-sub
**`0x25`** — the plaintext framing, inner payload at offset 16, msg-id at 25, `rrx` at 26..34.
Ridge seals the same message (`wsub=0x45`). `cp::perhead_rrx` now accepts both.

## 4. ⭐ Video: the dock accepts the bytes, then resets on a watchdog

⚠ **Corrected 2026-08-02.** This section previously said vino's first EP08 write "killed the device
within a millisecond". That is not what happens, and the distinction matters: the dock **accepts
every video byte without error** and resets several seconds later. See §4a for the measurement.

The original symptom — a spontaneous reset loop every few seconds:

```
KMS CRTC enable -- head 0 display ON, mode 2560x1440@120
head=0 persistent video queue opened by prompt training
head 0 startup frame submitted after 0 ms (205696 bytes)
head 1 sink re-engagement failed (ENODEV)
```

Video is therefore gated by `DockProfile::video_supported`, checked at `run_pending_scanout()` —
which every scanout write funnels through — and at the prompt-training submission.

### 4a. ⭐ Measured: the stream-open is genuinely required

The gate can be lifted at runtime with the `force_video=1` module parameter, which exists to answer
exactly one question: does this platform need its sealed stream-open, or is correct record framing
enough? It is off by default because the answer is that the dock resets.

Two runs, same module (`86059d8c9ed3f34d`), same 80 s capture window
(`captures/navarro-forcevideo-20260802`):

| run | dock instances in 80 s | video writes | video URB errors |
|---|---|---|---|
| `force_video=0` (control) | **1** — 80.3 s continuous | 0 | – |
| `force_video=1` | **9** — ~9.0 s each | 9 per instance, 474368 B | **0** |

The cycle is highly regular, and within each instance:

* every video URB completes with **status 0** — the dock does not reject the framing;
* the control plane keeps working for **~6.2 s** after the last video write, with normal `0x02`/
  `0x84` request/reply traffic;
* only then does the device re-enumerate.

⇒ This is a **watchdog expiring, not a malformed write being refused.** The dock takes the pixels,
has no stream context to put them in because the 48-byte stream-open never arrived, and gives up.
Correct record framing alone is therefore *not* sufficient, and there is no way around building the
stream-open.

⛔ Do not re-run this expecting a different result with framing tweaks: the bytes are already being
accepted, so framing is not what the dock is complaining about.

### What DLM actually sends

From the capture, in order:

```
ep 0x08  len     48   hdr 00 00 2c 00 04 00 00 00   id=0x17 sub=0x02    <- head 0 stream open
ep 0x0a  len     48   hdr 00 00 2c 00 04 00 00 00   id=0x1f sub=0x02    <- head 1 stream open
ep 0x08  len  65536   hdr 00 00 1c 00 02 00 00 00   id=0x07 sub=0x00    <- first frame, part 1
ep 0x08  len  54480   (continuation, raw payload)
ep 0x08  len  65536   hdr 00 00 1c 00 04 00 00 00   id=0x00 sub=0x04    <- steady-state frame
ep 0x08  len  53056   (continuation)
ep 0x0a  len  65536   hdr 00 00 1c 00 04 00 00 00   id=0x08 sub=0x04    <- head 1 steady state
```

So:

* the stream opens with a **48-byte sealed frame** per head — `id=0x17` on `0x08`, `id=0x1f` on
  `0x0a` — not with a large ARM+black frame as on Ridge;
* the **first** data frame uses `sub=0x02` and `id=0x07`; every later frame uses `sub=0x04` with
  `id=0x00` (head 0) or `id=0x08` (head 1);
* frames arrive as a 65536-byte URB plus a remainder (~53–54 KB), i.e. ~119 KB per 2560x1440
  frame.

⚠ vino currently submits a **205696-byte** Ridge ARM+black frame as its opening write. That is the
message the dock rejects.

### ⭐ The pixel payload is plaintext — only the stream-open is sealed

Measured 2026-08-02 over `captures/navarro-dlm-modeset-20260802-005453` (628 DLM video frames).
Shannon entropy of the first 4 KiB of payload, past the 16-byte transport header:

| frame | entropy | reading |
|---|---|---|
| `ep 0x0a`, `id=0x02` record stream | **5.71** bits/byte | structured |
| `ep 0x0a`, continuation | **3.43** bits/byte | structured |

Encrypted data sits at 8.00. The payload is visibly regular in hex as well
(`… 01 fc 00 7e 00 3f 80 1f c0 0f e0 07 f0 03 f8 01 …`), and the inner records carry the same
`00 00 1c 00 02 00 00 00` header shape as the outer frame. **So video pixels are never encrypted on
this platform**, exactly as on Ridge.

⇒ The video key question in §2 collapses to a single message: the **48-byte stream-open** is the
only sealed thing on the video endpoints. Once it can be built, nothing else on `0x08`/`0x0a` needs
a key.

Inner records observed on `ep 0x0a` use sub `0x0f` and `0x1f`; head 0 uses `0x07` and `0x17`. The
per-head offset is **8** throughout, consistent with `DockProfile::head_sub_shift = 3`.

### ⛔ The stream-open is not reproducible in software

The 48-byte opens appear **only in a capture spanning a cold connect**. Attempts that produced
video traffic but no stream-open:

| attempt | result |
|---|---|
| restart `displaylink-driver.service` | frames resume, no open |
| `kscreen-doctor output.…enable`/`disable` | no open |
| resolution change (forces `0x48/0x22`) | no open |
| `echo 0 > …/authorized` then `1` (twice) | full re-enumeration, 628 frames, **no open** |

The last of these was run with vino unbound and blacklisted so DLM certainly owned the device
(`DLM reclaimed after 4s`, 540 distinct keys captured) — the dock still reused its existing video
stream. ⇒ **capturing the stream-open needs a physical replug or dock power-cycle with frida
attached.** Everything else for video is built and gated behind `video_supported`.

⭐ Also measured: DLM drives this dock at **2560x1440@164.96** on both heads. It does **not** clamp
to 120 Hz the way it does on Ridge ([[project_dlm_clamps_to_120_cp_decrypted_20260726]]), so this is
the platform that can finally answer the `off72` mode word.

### 4b. ⭐ The stream-open plaintext, observed at the live sealer

The direct DLM AES-CTR transform hook sees the complete plaintext passed to the sealer for all four
connectors:

```
04 00 08 04 05 00 06 00 07 01 08 02 07 00
```

⛔ **"exactly 14 bytes" is RETRACTED (2026-08-03).** Recovered whole from the wire instead of from
the sealer: both 48-byte opens in `captures/navarro-dlm-session-20260801-190654` authenticate
byte-for-byte under their connector's key and nonce, which fixes the sealed content at **16**
bytes — the 14-byte marker plus a trailing pair the two opens disagree on (`d9 33`, `c6 6c`). A
14-byte content makes a 46-byte frame where the dock is sent 48.

The full frame is `size=0x002c type=4 sub=<stream_id | 0x10> aux=0x0002 seq=0`. ⚠ Note the sub: the
open uses the stream's **upper** marker sub (`0x17` for connector 0, `0x1f` for connector 1), not
the bare stream id, which names the sealed configuration records instead.

⭐ **HW-verified 2026-08-03:** built this way by vino, the dock **accepts the open (URB status 0)**.
It is also its own bulk transfer — DLM sends it alone, and the dock stalls the endpoint if pixel
records follow it in the same transfer.

### 4c. ⭐⭐ The video prologue, decoded and authenticated

Every connector's first frame is preceded by six records, in one transfer with the pixels.
Recovered from `captures/navarro-dlm-modeset-20260802-005453`, connector 1 (`ep 0x0a`); connector 0
on `ep 0x08` is identical bar its ids and addresses:

```
type=2 sub=0x0f              32 B   body: 0f 00 06 00 …          <- stream marker
type=2 sub=0x1f              32 B   body: 1f 00 06 00 …          <- stream marker | 0x10
type=4 sub=0x0f aux=0x0e     336 B  sealed, seq=0                <- pipe descriptor (304 B)
type=2 sub=0x08              32 B   body: 08 00 00 00 …          <- frame marker
type=4 sub=0x08 aux=0x04     32 B   unsealed: 0a 00 04 …         <- first ring slot
type=4 sub=0x0f aux=0x0e     1136 B sealed, seq=19               <- decoder config (1104 B)
… frame records: type=4 sub=<connector << 3> aux=0 …
```

The two sealed records share the video channel's block counter, so the configuration starts at
`304 / 16 = 19`.

⭐ **The 1104-byte decoder configuration is the message Ridge already sends.**
`video_arm::build_with_layout_word(2560, 1440, 0x2100, tail)` reproduces the captured plaintext
exactly; only the repeated layout word differs from Ridge's `0x4000`, and its derivation from the
mode is not established.

⭐ **The 304-byte pipe descriptor is generative.** It is the 14-byte stream marker, six
`[len=0x002c][kind=0x000e][slot]` records of 40 configuration bytes (records advance by `len + 2`,
not `len + 4`), and a 14-byte host-random tail. Each configuration is a fixed 12-byte header, three
dock-side addresses, and a fixed 10-byte trailer. Fixed by twelve records across two independently
keyed connectors:

| field | at | rule |
|---|---|---|
| slot id | record | `connector * 8 + index`, `index` 0..5 |
| ring | `cfg[12]` | `0x6fcc - slot * 0x21c` |
| plane 0 | `cfg[18]` | `0x71fb9000 - (connector * 6 + index) * 0x5000` |
| plane 1 | `cfg[26]` | `0x72166000 - (connector * 6 + index) * 0x8000` |

⚠ The ring counts in **slot ids**, so it skips the two ids each connector leaves unused; the two
plane pools count in **allocated slots** and do not. Only 2560x1440 has been observed, and the
fixed header carries mode-derived bytes (`0x00b4` = height/8, `0x0014` = width/128).

### 4d. ⭐ Frames cycle three ring slots

A frame closes with **two** 32-byte records, not Ridge's three:

```
type=4 sub=<frame sub> aux=0x0006   08 00 05 <slot>      00 00 <ring(slot)>   00 <frame no> …
type=4 sub=<frame sub> aux=0x0004   0a 00 04 <next slot> 00 00 <ring(next)>   00 00 <ring(slot)> …
```

The slot advances `index 0 -> 2 -> 4 -> 0`, so a head cycles **three** of its six buffers, and the
prologue's unsealed record opens the cycle at index 0 with index 4 as its predecessor. Frame
numbers are a wrapping one-based `u8`. ⇒ this is the same three-slot phase Ridge tracks; only the
record layout differs, so `frame_trailer` needed a second form rather than new bookkeeping.

### Where this stands

Implemented and HW-tested 2026-08-03 (`drm/vino: open and drive the DL7400's per-connector video
streams`). Sending each piece as its own bulk transfer localises the dock's objection exactly:

| transfer | bytes | result |
|---|---|---|
| stream-open | 48 | ✅ **URB status 0** |
| prologue (markers + descriptor + ring + decoder config) | 1600 | ✅ **URB status 0** |
| first image records | 203 KB | ❌ **`-EPIPE`, endpoint halted** |

⇒ **The keying, the stream-open and the whole pipe configuration are accepted by the dock.** The
remaining fault is in the image records, and it is not a late record: every URB of that transfer
fails, including the first, so the dock rejects the opening image record itself.

⛔ Do not re-chase: the stream-open (content, length, sub, key, nonce, standalone transfer), the
pipe descriptor, the decoder configuration or the ring trailer. The dock's status-0 acceptance of
all of them settles it.

⛔ Also eliminated: **image-record framing**. vino's records now match DLM's byte-for-byte in the
header — `size=0x0fcc` (4048), `type=4`, `sub = connector << 3`, `aux=0`, `seq=0`, body opening
`36 00 01 28` with the same coordinates and `36 00 36 00` — and the dock still halts on them.

Two framing facts came out of getting there, both now profile fields:

* ⭐ **`aux` on a DL7400 image record is a record *type*, not Ridge's zero-padding count.** Every
  DLM image record carries `aux=0`, while `aux` of 2, 4, 6, 8, 0x0a, 0x0c and 0x0e each name a
  different per-frame control record. Writing a pad count there makes the dock read an image
  record as some other kind entirely.
* ⭐ **A record ends at a y-band boundary only on Ridge**, whose `sub` carries the band parity. The
  DL7400's does not, so its records fill to the stride cap across bands — which is exactly how
  DLM's 4048-byte records arise.

⭐ **A DL7400 frame carries far more than image records.** Over ~19 frames of
`captures/navarro-dlm-modeset-20260802-005453` on connector 1, per frame DLM sends about four
`aux=0` image records **and one record of each of `aux` = 2, 4, 6, 8, 0x0a, 0x0c, 0x0e**, then a
small sealed record back on the *stream* sub (`0x000f`, 128 or 144 bytes, `aux=0x000c` or
`0x0002`). vino sends only the image records. None of these are decoded yet.

### ⭐ The live lead: image records describe a different tiling

vino's first image record is **byte-identical** to DLM's — same 16-byte outer header, same
`36 00 01 28`, same `00 00 00 00` coordinates, same `36 00 36 00` strip-length prefixes. Records
after the first are not, and the difference is in the coordinate pair at inner offset 4:

| | first four records' `(x, y)` |
|---|---|
| DLM | `(0, 0)`, `(1536, 48)`, `(512, 104)`, `(2048, 144)` |
| vino | `(0, 0)`, `(1472, 912)`, `(128, ...)`, `(256, ...)` |

⭐ **DLM's `x` only ever takes five values — 0, 512, 1024, 1536, 2048 — and they repeat in the
scattered order `0, 1536, 512, 2048, 1024`.** Five columns of 512 px across a 2560-wide frame.
vino emits `x` on a 64-px grid in raster order, which is Ridge's tiling.

⇒ Navarro's frame is tiled and ordered differently from Ridge's, and the dock halts partway through
the first transfer of image records — after accepting the byte-identical opening one. That is
consistent with the dock following the record stream until a coordinate it cannot map.

⚠ The strip payloads themselves still look like Ridge's: `36 00` (54-byte) length prefixes and the
same `fc 00 7e 00 3f 80 …` bit patterns in both. So this is a tiling and ordering difference, not a
different codec.

### Where the transfer fails

Every URB of the image transfer is in flight before any completes, so `-EPIPE` on all of them only
bounds the fault to that transfer, not to a record. Splitting the *prologue* out was what localised
it that far; splitting the image transfer the same way would localise it to a 64 KiB window, and
sending a frame with no image records at all would confirm the pipe is otherwise live.

⛔ Eliminated by test, do not re-chase: prologue inline with the pixels versus in its own transfer
(both halt identically once the framing above is correct).

⊙ **Next**, in order:

1. **Recover Navarro's strip tiling and record order** from the coordinate fields above. This is
   the live lead: 5 columns x 512 px, emitted in a fixed scattered order, over ~19 frames in
   `captures/navarro-dlm-modeset-20260802-005453` and four connectors' worth in the Windows cold
   captures.
2. **Decode the per-frame control records** — one each of `aux` = 2, 4, 6, 8, 0x0a, 0x0c, 0x0e on
   the frame sub, plus a 128/144-byte sealed record on the stream sub. vino sends none of them.
3. The descriptor's dock-side addresses were captured at 164.96 Hz while vino drives 120 Hz. They
   are accepted, but a buffer sized for another mode would only be rejected once pixels arrive.
