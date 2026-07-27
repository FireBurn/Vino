# Reverse-engineering method

The driver is clean-room code built from observed USB behavior, public display
and HDCP specifications, and independently implemented validation tools. The
production source records invariants and wire layouts; it does not carry the
chronology of failed experiments.

## Evidence order

Use the strongest available evidence:

1. public specifications or an existing kernel subsystem definition;
2. multiple decoded captures from the proprietary stack;
3. independent userspace reproduction against the hardware;
4. decompiler or debugger observations tied to a binary hash;
5. a single capture or hypothesis.

A value moves into production only when its scope is explicit. A captured
D6000 mode word is a D6000 profile value, not a generic DL3 rule.

## Avoiding circular tests

An encoder/decoder round trip can pass when both implementations share the same
mistake. Protocol and codec validation therefore uses:

- immutable captured messages and strip records;
- the kernel implementation;
- a userspace oracle with different structure;
- focused adversarial vectors at category, escape, padding, and length
  boundaries.

Chimera compiles the literal kernel `proto.rs`, `cp.rs`, `ake.rs`, `hdcp.rs`,
`video.rs`, and `video_arm.rs` for integration tests. That detects source drift
and transport composition errors. Separate fixtures and reference helpers are
still required for independence.

## Recording a finding

A new finding should include:

- device USB ID and firmware/product string;
- capture or binary SHA-256;
- endpoint, direction, transaction index, and surrounding messages;
- the decoded field and byte order;
- whether the result repeated across sessions, heads, modes, or monitors;
- a fixture and a failing test added before the implementation change;
- a confidence label: specified, multi-capture, single-capture, or hypothesis.

Do not place dates, “try this”, disabled experiments, or superseded theories in
production comments. Keep those in a research note or Git history, then update
the concise protocol document once verified.

## Historical archive

The original research ledger remains in `../docs` relative to the parent
`dl-scripts` checkout. It contains captures, refuted hypotheses, decompiler
addresses, and dated handovers. It is intentionally static. When it conflicts
with the current kernel source, the source plus its tests and the curated docs
in this repository are authoritative.

