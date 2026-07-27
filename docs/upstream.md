# Upstream status and review disposition

Status was rechecked on 2026-07-27 against the live lore threads and remote
branch tips.

## Current bases

- `drm-next`: `ea97ab2759506d9a818ffed1009bde01062b4091`;
- `drm-rust-next`: `6dcbb4b1320fa91fee349462a52bb69135f2e45e`;
- Lyude Paul's `rvkms-slim`: unchanged at
  `25bc8cc7e97fd292bea4b77354aaac7eba6c5385`.

The working base merges the first two tips. No newer complete `rvkms-slim`
revision was found.

## v1/v2 feedback carried forward

### Rust DRM/KMS

Lyude Paul NAKed the v2 series shape because it lost her attribution and
development history, mixed unrelated changes, and duplicated existing work.
The new history preserves her commits in order, with patch-identical content
and only her original commit messages/tags. Mike and AI attribution never
appears on her commits. Necessary current-tree adaptations and safety fixes are
separate Mike-authored patches.

The driver uses Lyude's KMS object layer and the current shmem mapping
infrastructure. It does not call raw C KMS from consumer code or duplicate the
accepted registration lifetime work.

Thread:
<https://lore.kernel.org/all/20260703030123.2814-1-mike@fireburn.co.uk/>

### USB

The v2 discussion distinguished a bound interface from the narrower period in
which USB I/O is legal and objected to exposing device-wide operations through
an arbitrary interface. The new series uses typed endpoints and an adapter-owned
revocable I/O window across probe, suspend, reset, resume, and disconnect.

Colin Braun's URB RFC remains relevant overlapping work. Vino's additions are
kept in the USB subsystem part of the series and should be coordinated there
rather than presented as private DRM helpers.

Threads:

- <https://lore.kernel.org/all/20260703030020.2694-1-mike@fireburn.co.uk/>
- <https://lore.kernel.org/all/20260712-urb-abstraction-v1-v1-0-9fa011634ead@gmail.com/>

### Crypto

Eric Biggers objected to exposing bare, repeatedly expanded AES and to growing
the `crypto_akcipher` API. The current implementation uses the in-tree AES-CMAC
library, prepares AES keys once, and puts the RSA public operation in
`lib/crypto` rather than exposing `crypto_akcipher` to Rust.

No human reply was added to the crypto v2 posting after it was sent.

Thread:
<https://lore.kernel.org/all/20260703030056.2763-1-mike@fireburn.co.uk/>

### Vino v2 and Revdi

Vino v2 received automated review but no human follow-up. Verified findings were
folded into the production tree; the review transcript is not treated as human
acceptance. The Rust EVDI posting also received no replies.

Threads:

- <https://lore.kernel.org/all/20260703030217.2886-1-mike@fireburn.co.uk/>
- <https://lore.kernel.org/all/20260703030249.2949-1-mike@fireburn.co.uk/>

## Patch authorship

Third-party commits retain their authors, messages, and trailers. In
particular:

- Lyude's KMS patches contain no Mike or AI tags;
- Onur Özkan's `cancel_sync` patch is kept as his work;
- prerequisite commits from Joel Fernandes, Boqun Feng, and Heiko Carstens are
  not re-authored.

Every Mike-authored patch ends with this contiguous trailer block:

```text
Assisted-by: Claude:claude-opus-5-0
Assisted-by: Codex:gpt-5
Signed-off-by: Mike Lothian <mike@fireburn.co.uk>
```

This follows `Documentation/process/coding-assistants.rst` in the kernel base:
the assistant and exact model are identified with `AGENT_NAME:MODEL_VERSION`,
and only Mike supplies the DCO sign-off. It does not invent a human identity or
add trailers to someone else's patch.

## Series shape

The bring-up diary, hardware experiments, temporary knobs, reversions, and
“keep going” commits are absent. Shared Rust APIs are introduced in their
own subsystem patches with real consumers. Revdi lands once on the safe APIs.
Vino is split into:

1. encrypted control and HDCP;
2. video codec and arm configuration;
3. DRM/KMS scanout;
4. USB integration and lifecycle;
5. kernel documentation.

This is a review branch, not evidence that every new subsystem API is accepted.
USB, crypto, I2C, sysfs, platform, workqueue, and DRM changes still need their
respective maintainers and list routing.
