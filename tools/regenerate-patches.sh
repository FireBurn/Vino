#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-2.0
#
# Export the kernel work as independently postable series, one directory each.
#
# Each series is numbered from 0001 and carries its own cover letter, so it can be
# sent on its own to the subsystem that owns it. A single 126-patch posting is not
# reviewable by anyone, and most of those commits belong to other people anyway:
# only commits authored here are exported. Everything else in the branch is a
# dependency to base on, not a patch to post.

set -euo pipefail

workspace="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
kernel_tree="${KERNEL_TREE:-$workspace/linux}"
kernel_base="${KERNEL_BASE:-integration/base-20260809}"
kernel_head="${KERNEL_HEAD:-vino}"
author_email="${AUTHOR_EMAIL:-mike@fireburn.co.uk}"
output="$workspace/patches"

git -C "$kernel_tree" rev-parse --git-dir >/dev/null 2>&1 ||
    { echo "error: not a git tree: $kernel_tree" >&2; exit 2; }
for ref in "$kernel_base" "$kernel_head"; do
    git -C "$kernel_tree" rev-parse --verify --quiet "$ref^{commit}" >/dev/null ||
        { echo "error: missing ref '$ref'" >&2; exit 2; }
done

# Series order is apply order: a later one may depend on an earlier one, never the
# reverse. Membership is decided from the subject alone so that adding a commit
# needs no edit here.
series_order=(sched-fair rust-core rust-crypto rust-usb rust-drm rust-firmware drm-tyr drm-vino drm-evdi)

classify() {
    case "$1" in
    "sched/fair: "*)                    printf 'sched-fair' ;;
    "rust: crypto: "*)                  printf 'rust-crypto' ;;
    "rust: usb: "*)                     printf 'rust-usb' ;;
    "rust: drm"*)                       printf 'rust-drm' ;;
    "rust: firmware: "*)                printf 'rust-firmware' ;;
    "rust: "*)                          printf 'rust-core' ;;
    "drm/tyr: "*)                       printf 'drm-tyr' ;;
    "drm/vino: "*|"Documentation/gpu: "*) printf 'drm-vino' ;;
    "drm/evdi: "*)                      printf 'drm-evdi' ;;
    *)                                  printf '' ;;
    esac
}

title() {
    case "$1" in
    sched-fair)    printf 'sched/fair: stop reading a guard flag after the guard drops it' ;;
    rust-core)     printf 'rust: core abstractions for a USB display driver' ;;
    rust-crypto)   printf 'rust: crypto: AES, CMAC, SHA-256, HMAC and RSA bindings' ;;
    rust-usb)      printf 'rust: usb: host-side abstractions for a bulk-endpoint driver' ;;
    rust-drm)      printf 'rust: drm: KMS abstractions for a Rust display driver' ;;
    rust-firmware) printf 'rust: firmware: the firmware upload abstraction' ;;
    drm-tyr)       printf 'drm/tyr: mark the Rust DRM driver as non-KMS' ;;
    drm-vino)      printf 'drm/vino: a Rust driver for DisplayLink DL3 docks' ;;
    drm-evdi)      printf 'drm/evdi: a Rust virtual display driver' ;;
    esac
}

depends() {
    case "$1" in
    rust-crypto|rust-usb|rust-drm|rust-firmware) printf 'rust-core' ;;
    drm-vino)  printf 'rust-core, rust-crypto, rust-usb, rust-drm, rust-firmware' ;;
    drm-evdi)  printf 'rust-core, rust-drm' ;;
    *)         printf 'none' ;;
    esac
}

# Cover-letter text. A series with nothing here gets the placeholder, so an
# unwritten blurb is visible rather than silently absent.
blurb() {
    case "$1" in
    sched-fair) cat <<'BLURB'
"locking: Switch to _irq_{disable,enable}() variants in cleanup guards" removed
the flags field from the raw_spinlock_irqsave and spinlock_irqsave CLASS()
guards, since the new primitives do not need explicit flags storage.
sched_cfs_period_timer() is the one place left in the tree that still read
cfsb_guard.flags directly, so it stops building once that lands.

A tree-wide grep for CLASS(raw_spinlock_irqsave, ...) and
CLASS(spinlock_irqsave, ...) turns up no other consumer of the removed field.
This desugars that one call site back to raw_spin_lock_irqsave() and
raw_spin_unlock_irqrestore(). No behavioural change: the lock is held over the
same span through both return paths, and do_sched_cfs_period_timer() still gets
the same flags value.

Sent separately because it belongs to the scheduler rather than to the driver
work that found it, and because it is a build fix for a series already in
flight.
BLURB
        ;;
    rust-core) cat <<'BLURB'
Core Rust abstractions needed by a USB display driver written in safe Rust.
They are separated from the driver because none of them are display-specific:
each covers a kernel facility that has C callers today and no Rust binding.

  - platform: create a platform device at runtime, for a driver that publishes
    virtual devices rather than binding to firmware-described ones.
  - sysfs: attribute groups on a root device.
  - sync: single-shot and timed completions.
  - hrtimer: restart an ArcHrTimerHandle, and see the interrupt state inside a
    hard callback.
  - random, xxhash, time: safe wrappers over get_random_bytes(), xxh64() and
    ktime_get_real_seconds().
  - workqueue: make OwnedQueue thread-safe.
  - io: offset copy helpers that check the bounds they are given.
  - error: expose EPROTO.
  - fpu: an RAII guard for a kernel FPU section.

Each is small and independently reviewable, and each has a user in the driver
series that follows.
BLURB
        ;;
    rust-crypto) cat <<'BLURB'
Synchronous crypto bindings for a driver that has to authenticate a device
before it can drive it.

The first patch covers AES-128, AES-CMAC, SHA-256 and HMAC over the existing
synchronous crypto API. The second adds RSA through akcipher, which HDCP 2.2
needs to verify a device certificate and wrap a session key.

The consumer is the DisplayLink driver in a later series, whose control plane
is sealed with AES-CTR and keyed by an HDCP 2.2 exchange. The bindings
themselves are generic and carry no knowledge of that protocol.
BLURB
        ;;
    rust-usb) cat <<'BLURB'
Host-side USB abstractions for a driver whose device is a bulk-endpoint pipe
rather than a class device.

  - Revocable typed interface I/O, so an interface cannot be used after the
    core has taken it back.
  - Reusable URBs and persistent bulk queues, which is what keeps a video
    stream in flight without allocating per transfer.
  - Topology lookup and removal notification.
  - A device id constructor matching on vendor together with interface class,
    subclass and protocol, for a composite device whose function is not
    identified by the product id alone.
  - Letting a driver keep its interface usable while unbinding, so teardown can
    still talk to the device it is releasing.

These build on the USB abstractions already merged; the consumer is the
DisplayLink driver in a later series.
BLURB
        ;;
    rust-drm) cat <<'BLURB'
KMS abstractions for a Rust display driver, continuing Lyude Paul's KMS series
rather than replacing it. The first patch adapts that work to the DRM APIs as
they stand today; the rest add what a real driver turned out to need.

Broadly they fall into four groups:

  - Lifetime and ownership: mode-object references tied to their owners, owned
    CRTC and vblank references, a safe constructor for owned registration data,
    pinning the owner while DRM files remain open, and rejecting cross-device
    GEM handle creation.
  - Properties a driver must read or publish: typed colour and rotation, plane
    blend mode, FB_DAMAGE_CLIPS, connector colorimetry and HDR metadata, and a
    connector's requested link depth.
  - Callbacks and state: connector detect() and mode_valid(), common state and
    connector helpers, checked plane geometry, walking the CRTCs an atomic
    commit carries, and CRTC mode changes.
  - Modes and framebuffers: an owned display mode constructor, mode flags and
    CTA VIC matching, synthesized CVT connector modes, and validated shmem
    scanout views.

Also here: typed RAII event channels, private ioctl compat translations, and
the HDCP 2.2 message identifiers, which are DRM UAPI rather than driver
constants.

The consumers are the two drivers in the series that follow, both of which
contain no unsafe block and no direct bindings:: call.
BLURB
        ;;
    rust-firmware) cat <<'BLURB'
request_firmware() covers "pull an image from /lib/firmware".
firmware_upload_register() covers the other half: userspace hands the driver an
image to write. It publishes /sys/class/firmware/<name>/ with the loading and
data handshake plus status, error, remaining_size and cancel, and is what a
driver uses when an image has to be written on demand rather than only when a
newer one appears.

This adds an Upload trait mirroring struct fw_upload_ops, a Registration that
unregisters on drop, and an Error enum whose values are the fw_upload_err codes
userspace already reads back out of the error attribute.

The consumer is the DisplayLink driver, which writes dock firmware over DFU.
BLURB
        ;;
    drm-tyr) cat <<'BLURB'
The KMS registration trait gains a required Kms associated type. Tyr is a
render-only driver, so it selects the non-KMS PhantomData implementation just as
Nova does.

A build fix for the KMS series rather than a change to Tyr's behaviour, sent on
its own so it can go through whichever tree takes it first.
BLURB
        ;;
    drm-evdi) cat <<'BLURB'
A Rust implementation of EVDI that preserves the established libevdi and
DisplayLinkManager display ABI, so existing userspace keeps working unchanged.
The ABI is installed as a normal DRM UAPI header with generated Rust bindings,
and its pointer-bearing ioctls are translated for 32-bit clients.

Cards are created through the established platform sysfs interface and expose
the expected DRM ioctls and events. The driver uses owned DRM, KMS, event,
framebuffer and sysfs abstractions throughout: it contains no unsafe block, no
direct bindings:: call and no reconstructed object lifetime.

Scanout uses a bounded four-entry pool with owned shmem mappings, so compositor
swapchain buffers are mapped before the client is notified and reused for later
flips and GRABPIX calls. The bandwidth limits userspace supplies are enforced as
given, without a driver-invented multiplier.
BLURB
        ;;
    drm-vino) cat <<'BLURB'
Vino is a DRM/KMS driver for DisplayLink DL3 docks. These devices carry no
standard display protocol: the host encodes each frame with a vendor codec and
ships it over bulk USB, inside a control plane sealed with AES-CTR and keyed by
an HDCP 2.2 authentication exchange. Until now the only way to drive one on Linux
was an out-of-tree kernel module paired with a closed-source userspace daemon.
Everything here is reverse-engineered from the wire and from the vendor
binaries; there is no vendor documentation for any of it.

Three generations are supported, and they differ in more than identifiers:

  - DL-3x00 (Ella), which shares one pipe between control and video, states its
    decoder tables in a narrow form, and must never be blanked by painting black.
  - DL-6xxx (Ridge), including the Dell D6000, which serves both connectors from
    a single EDID handler.
  - DL-7400 (Navarro), four connectors over two video endpoints, 10 Gbps.

The differences are data. A dock is placed by family into a DockProfile carrying
its endpoints, codec geometry, allocation rules and quirks, and there is one code
path through the driver for all three -- no per-device branches, no module
parameters selecting behaviour.

The driver reads the dock's running firmware version and can update it over DFU
through the firmware upload API, which is how a dock too old to enumerate its
connectors is brought forward.

On DL-7400 the driver drives 30 bpp in PQ: 2560x1440p120 on two connectors, with
the sink reporting 10 bit. Depth is not a flag on the wire but a set of
agreements -- the DMA format, the colour-depth word, the framebuffer allocation,
and the entropy coder's escape ceilings, each of which is stated to the dock by
its own decoder code table. Getting one of them wrong is not a clean failure: a
DC ceiling the dock was not told about desynchronises the bitstream mid-record,
and an AC one stays in step while reconstructing every sharp edge from a
truncated magnitude.

Tested on all three generations with monitors attached, driving a desktop.

The protocol was reverse engineered from captured wire traffic and from the
vendor binaries; the assistance noted below covers that work as well as the
implementation, and every constant here came from a measurement.
BLURB
        ;;
    *) printf '*** BLURB HERE ***\n' ;;
    esac
}

rm -rf -- "$output"
mkdir -p "$output"

declare -A members
while IFS='|' read -r sha subject; do
    group="$(classify "$subject")"
    if [ -z "$group" ]; then
        echo "error: no series for '$subject'" >&2
        echo "       add a rule to classify() rather than letting it fall out of the export" >&2
        exit 1
    fi
    members[$group]+="$sha "
done < <(git -C "$kernel_tree" log --reverse --format='%H|%s' \
             --author="$author_email" "$kernel_base..$kernel_head")

exported=0
for group in "${series_order[@]}"; do
    shas="${members[$group]:-}"
    [ -n "$shas" ] || { echo "error: series '$group' is empty" >&2; exit 1; }
    dir="$output/$group"
    mkdir -p "$dir"
    n=0
    for sha in $shas; do
        n=$((n + 1))
        git -C "$kernel_tree" format-patch --no-signature --quiet \
            --start-number "$n" --output-directory "$dir" -1 "$sha" >/dev/null
    done
    count=$n
    exported=$((exported + count))

    # A cover letter git cannot generate itself: the commits are not contiguous in
    # the branch, so there is no range to hand format-patch.
    {
        printf 'From: Mike Lothian <%s>\n' "$author_email"
        printf 'Subject: [PATCH 0/%d] %s\n\n' "$count" "$(title "$group")"
        printf 'Depends on: %s\n' "$(depends "$group")"
        printf 'Base:       %s\n\n' "$kernel_base"
        blurb "$group"
        printf '\n'
        # Documentation/process/generated-content.rst asks for the disclosure in the
        # cover letter, not only in the per-patch trailer.
        if grep -lq '^Assisted-by:' "$dir"/[0-9][0-9][0-9][0-9]-*.patch 2>/dev/null; then
            cat <<'DISCLOSURE'
These patches were written with the assistance of Claude (Anthropic), used
through Claude Code as an interactive coding assistant, across the design, the
implementation and the tests. Every patch it contributed to carries an
Assisted-by trailer. The Signed-off-by is mine: I have reviewed and tested what
is here and I stand behind it.

DISCLOSURE
        fi
        printf 'Mike Lothian (%d):\n' "$count"
        for sha in $shas; do
            git -C "$kernel_tree" show -s --format='  %s' "$sha"
        done
        printf '\n'
        # Aggregate the diffstat over the set: the commits are not contiguous, so a
        # range diff would sweep in everything between them.
        for sha in $shas; do
            git -C "$kernel_tree" show --numstat --format= "$sha"
        done | awk '
            $3 != "" { add[$3] += $1; del[$3] += $2 }
            END {
                n = 0; a = 0; d = 0
                for (f in add) { n++; a += add[f]; d += del[f] }
                printf " %d file(s) changed, %d insertion(s)(+), %d deletion(s)(-)\n", n, a, d
            }'
    } >"$dir/0000-cover-letter.patch"

    ls "$dir" | grep -E '^[0-9]{4}-' | LC_ALL=C sort >"$dir/series"
    printf '%-14s %2d patch(es)   depends: %s\n' "$group" "$count" "$(depends "$group")"
done

# Every commit of ours must land in exactly one series.
mine="$(git -C "$kernel_tree" rev-list --count --author="$author_email" "$kernel_base..$kernel_head")"
if [ "$exported" -ne "$mine" ]; then
    echo "error: exported $exported patches but $mine commits are ours" >&2
    exit 1
fi

{
    printf '# Kernel patch export\n\n'
    printf 'Generated by `tools/regenerate-patches.sh`. Each directory is an independent\n'
    printf 'series, numbered from 0001 with its own cover letter, and is posted on its own\n'
    printf 'to the subsystem that owns it.\n\n'
    printf 'Only commits authored by %s are exported. The branch also carries other\n' "$author_email"
    printf "people's in-flight work -- Lyude Paul's KMS series, Boqun Feng's SpinLockIrq\n"
    printf 'series and others -- which are dependencies to base on, never patches to post.\n\n'
    printf '| series | patches | depends on |\n|---|---|---|\n'
    for group in "${series_order[@]}"; do
        c="$(ls "$output/$group" | grep -cE '^[0-9]{4}-.*\.patch$')"
        c=$((c - 1))
        printf '| `%s` | %d | %s |\n' "$group" "$c" "$(depends "$group")"
    done
    printf '\nApply order is the table order. Base: `%s`.\n' "$kernel_base"
} >"$output/README.md"

printf '\n%d patches across %d series\n' "$exported" "${#series_order[@]}"
