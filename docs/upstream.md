# Upstream status and review disposition

Status was rechecked on 2026-07-28 against the public patch threads and the
available remote branch tips.

## Current base

- `drm-next` parent: `ea97ab2759506d9a818ffed1009bde01062b4091`;
- `drm-rust-next` parent: `93b9511a3bba7f31d95502e5f912f0a476b0cf4a`;
- existing merge used as the series base:
  `0755a4e3e809610a14befc9ad28d35e2e460da68`;
- Lyude Paul's `rvkms-slim`: no newer complete revision was found after the
  imported `25bc8cc7e97fd292bea4b77354aaac7eba6c5385`.

The full integration branch is not a proposed single-list posting. Its group
manifests separate existing work and independently owned subsystem APIs from
the EVDI and Vino consumers.

## v1/v2 feedback carried forward

### Rust DRM/KMS

The v2 series obscured Lyude Paul's attribution and mixed her work with
unrelated adaptations. The rebuilt history preserves her 37 KMS commits in
order and patch-identical to the imported source. Her messages and original
tags are untouched, and no Mike or assistant trailer appears on them.
Adaptations to the current DRM APIs and all later safety extensions are
separate Mike-authored commits.

Consumer code uses the safe KMS object layer and validated shmem scanout views.
It has no raw C KMS calls, direct bindings, or reconstructed object lifetimes.

### USB

The v2 review distinguished a bound interface from the narrower interval in
which I/O is legal. The rebuilt code models that interval as an adapter-owned,
revocable interface capability across probe, suspend, reset, resume, and
disconnect.

Colin Braun's current URB RFC overlaps this work. Its first three commits are
retained unchanged, followed by USB-owned additions for typed revocable I/O,
reusable persistent queues, topology lookup, and removal notification. Vino
does not carry a private URB implementation.

### Crypto

The current implementation does not add a Vino-specific RSA primitive. It uses
the kernel crypto implementations for AES, CMAC, SHA-256, HMAC-SHA256, and RSA,
with safe Rust wrappers in crypto-owned patches. RSA-OAEP padding and HDCP key
handling use a memory-wiping secret type and the existing `crypto_akcipher`
facility.

### I2C

Igor Korotin's active Rust I2C adapter work was checked. Its provider-lifetime
issue is not solved privately in Vino. The kernel driver therefore does not
register a downstream I2C adapter in this revision. Chimera retains its
userspace-only DDC/CI vendor transaction so protocol research can continue
without creating a competing kernel API.

### Vino and EVDI

Automated findings from the previous Vino posting were rechecked and corrected,
but are not treated as human acceptance. EVDI now uses a conventional C UAPI
header, generated Rust bindings, safe compat translation, normal DRM plane
geometry, and the shared owned shmem view. The Vino match table remains limited
to the hardware profile actually validated: Dell D6000 `17e9:6006`.

## Reused external work

The integration history deliberately retains:

- Lyude Paul's Rust DRM/KMS commits;
- Colin Braun's current USB URB RFC foundation;
- Alice Ryhl's v4 owned-workqueue series;
- Onur Özkan's `cancel_sync` workqueue patch;
- scheduler, locking, architecture, and preemption prerequisites under their
  original authors.

No newer revisions of those selected series were found during the 2026-07-28
check. They must still be posted and reviewed through their own maintainers;
their presence in the integration branch is not a claim of acceptance.

## Patch authorship

Third-party commits retain their authors, messages, and trailers. Every
Mike-authored kernel patch ends with:

```text
Assisted-by: Claude:claude-opus-5-0
Assisted-by: Codex:gpt-5
Signed-off-by: Mike Lothian <mike@fireburn.co.uk>
```

This follows `Documentation/process/coding-assistants.rst`: the assistant and
model are identified, while only Mike supplies the DCO sign-off.

## Series shape

The branch contains 106 commits in seven contiguous review groups:

1. interrupt/preemption prerequisites;
2. Lyude's Rust KMS series;
3. DRM, crypto, and driver-core interfaces;
4. USB interfaces;
5. Rust runtime and further DRM interfaces;
6. EVDI;
7. five Vino patches: control, codec, KMS, USB/lifecycle, documentation.

Bring-up chronology, experimental switches, reversions, temporary workarounds,
and unsupported hardware claims are absent. Generic facilities are introduced
in their owning subsystem rather than hidden in the driver.

## References

- [Rust crypto v2](https://patchew.org/linux/20260703030056.2763-1-mike%40fireburn.co.uk/)
- [Rust KMS v2](https://patchew.org/linux/20260703030123.2814-1-mike%40fireburn.co.uk/)
- [Vino v2](https://patchew.org/linux/20260617151249.2937-1-mike%40fireburn.co.uk/)
- [Colin Braun's USB URB RFC](https://patchew.org/linux/20260712-urb-abstraction-v1-v1-0-9fa011634ead%40gmail.com/)
- [Alice Ryhl's v4 workqueue series](https://patchew.org/linux/20260312-create-workqueue-v4-0-ea39c351c38f%40google.com/)
- [Igor Korotin's Rust I2C adapter RFC](https://patchew.org/linux/20260131-i2c-adapter-v1-0-5a436e34cd1a%40gmail.com/)
