#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-2.0
#
# Export the kernel work as independently postable series, one directory each.
#
# Each series is numbered from 0001 and carries its own cover letter, so it can be
# sent on its own to the subsystem that owns it. A single 122-patch posting is not
# reviewable by anyone, and most of those commits belong to other people anyway:
# only commits authored here are exported. Everything else in the branch is a
# dependency to base on, not a patch to post.
#
# Two of our own groups are exported to patches/not-posted/ instead. They are
# build fixes the reference tree needs and nothing else: they enable no part of
# Vino, so they are not sent alongside it.
#
# The cover letters cross-reference each other by lore link, and a link only
# exists once a series has been sent. tools/v3-message-ids.txt carries the ids;
# fill one in after sending and re-run this before preparing the next series.

set -euo pipefail

workspace="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
kernel_tree="${KERNEL_TREE:-$workspace/linux}"
kernel_base="${KERNEL_BASE:-integration/base-20260901}"
kernel_head="${KERNEL_HEAD:-vino}"
author_email="${AUTHOR_EMAIL:-mike@fireburn.co.uk}"
msgid_file="${MSGID_FILE:-$workspace/tools/v4-message-ids.txt}"
output="$workspace/patches"

# The branch people are asked to clone, and where.
tree_url="https://github.com/FireBurn/linux"
tree_branch="vino"

git -C "$kernel_tree" rev-parse --git-dir >/dev/null 2>&1 ||
    { echo "error: not a git tree: $kernel_tree" >&2; exit 2; }
for ref in "$kernel_base" "$kernel_head"; do
    git -C "$kernel_tree" rev-parse --verify --quiet "$ref^{commit}" >/dev/null ||
        { echo "error: missing ref '$ref'" >&2; exit 2; }
done

base_sha="$(git -C "$kernel_tree" rev-parse "$kernel_base")"
base_short="${base_sha:0:12}"

# Series order is apply order, and also send order: a later one may depend on an
# earlier one, never the reverse. Membership is decided from the subject alone so
# that adding a commit needs no edit here.
series_order=(rust-core rust-crypto rust-usb rust-drm rust-firmware drm-vino)
carried_order=(sched-fair drm-tyr)

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
    esac
}

# Reroll count, per series rather than per round. A series that has been posted
# before keeps counting; one going out for the first time is a v1 and carries no
# version tag, whatever round of the wider effort it happens to arrive in. Two of
# these are prerequisites nobody has seen yet, and labelling them v3 would tell a
# reviewer to go looking for two revisions that do not exist.
reroll() {
    case "$1" in
    rust-crypto|rust-usb|rust-drm|drm-vino) printf '4' ;;
    rust-core|rust-firmware)                printf '2' ;;
    *)                                      printf '' ;;
    esac
}

# The v2 posting this series continues, if there was one. Documentation says not
# to In-Reply-To an older revision of a multi-patch series, so this goes in the
# cover text as a link and nowhere else.
previous() {
    case "$1" in
    rust-core)     printf '20260826162851.2497-1-mike@fireburn.co.uk' ;;
    rust-crypto)   printf '20260826163004.3365-1-mike@fireburn.co.uk' ;;
    rust-usb)      printf '20260826163101.4168-1-mike@fireburn.co.uk' ;;
    rust-drm)      printf '20260826163359.4998-1-mike@fireburn.co.uk' ;;
    rust-firmware) printf '20260826163716.6274-1-mike@fireburn.co.uk' ;;
    drm-vino)      printf '20260826163913.7052-1-mike@fireburn.co.uk' ;;
    *)             printf '' ;;
    esac
}

lists() {
    case "$1" in
    rust-core)     printf 'rust-for-linux and linux-kernel' ;;
    rust-crypto)   printf 'linux-crypto and rust-for-linux' ;;
    rust-usb)      printf 'linux-usb and rust-for-linux' ;;
    rust-drm)      printf 'dri-devel and rust-for-linux' ;;
    rust-firmware) printf 'linux-kernel and rust-for-linux' ;;
    drm-vino)      printf 'dri-devel' ;;
    esac
}

# What a series needs under it, verified by applying it to the base plus that and
# nothing else. Two of them need nothing at all, which is worth a maintainer's
# time to know: they can be taken on their own.
depends() {
    case "$1" in
    drm-vino) printf 'rust-core, rust-crypto, rust-usb, rust-drm, rust-firmware' ;;
    *)        printf 'none' ;;
    esac
}

# The third-party series a group needs, as "message-id|description" lines.
prereqs_of() {
    case "$1" in
    rust-core) cat <<'P'
20260312-create-workqueue-v4-0-ea39c351c38f@google.com|Alice Ryhl, Creation of workqueues in Rust, plus Onur Ozkan's cancel_sync
P
        ;;
    rust-usb) cat <<'P'
20260712-urb-abstraction-v1-v1-0-9fa011634ead@gmail.com|Colin Braun, rust: usb: add usb request block abstractions
P
        ;;
    rust-drm) cat <<'P'
20250305230406.567126-1-lyude@redhat.com|Lyude Paul, Rust bindings for KMS + RVKMS
P
        ;;
    drm-vino) cat <<'P'
20250305230406.567126-1-lyude@redhat.com|Lyude Paul, Rust bindings for KMS + RVKMS
20260712-urb-abstraction-v1-v1-0-9fa011634ead@gmail.com|Colin Braun, rust: usb: add usb request block abstractions
20260312-create-workqueue-v4-0-ea39c351c38f@google.com|Alice Ryhl, Creation of workqueues in Rust, plus Onur Ozkan's cancel_sync
P
        ;;
    esac
}

msgid_of() {
    [ -r "$msgid_file" ] || return 0
    awk -v g="$1" '$1 == g && NF >= 2 { print $2; exit }' "$msgid_file"
}

# The rest of the posting, named from this series' point of view. Sent siblings
# get a lore link; unsent ones get named and placed in the order.
plural() { [ "$1" -eq 1 ] && printf 'patch' || printf 'patches'; }

siblings() {
    local self="$1" group id count
    printf 'The rest of the posting, which is one series per subsystem:\n\n'
    for group in "${series_order[@]}"; do
        count="${counts[$group]}"
        if [ "$group" = "$self" ]; then
            printf '  %s, %d %s, this one\n' "$group" "$count" "$(plural "$count")"
            continue
        fi
        id="$(msgid_of "$group")"
        if [ -n "$id" ]; then
            printf '  %s, %d %s, %s\n' "$group" "$count" "$(plural "$count")" "$(lists "$group")"
            printf '  https://lore.kernel.org/r/%s\n' "$id"
        else
            printf '  %s, %d %s, to %s, not sent yet\n' \
                "$group" "$count" "$(plural "$count")" "$(lists "$group")"
        fi
    done
    printf '\n'
    cat <<'SIBTAIL'
Vino is the user for all of them. The abstractions themselves are generic and
carry no knowledge of DisplayLink
SIBTAIL
}

tree_block() {
    cat <<TREE
The whole thing is one branch, base and prerequisites included, which is the
quickest way to read it:

  git clone -b $tree_branch $tree_url
  cd linux
  make LLVM=1 rustavailable
  make LLVM=1 -j\$(nproc)
  make LLVM=1 -j\$(nproc) modules

CONFIG_RUST=y and CONFIG_DRM_VINO=m are the two to set; DRM_VINO selects the
rest of what it needs

It is the exact tree these patches were generated from, at $base_short, the
drm-rust-next tip of 2026-08-06. drm-next has moved on since, and this follows
drm-rust-next deliberately: the KMS layer underneath this work lives only there,
and that tree picks up drm-next on its own schedule

Two commits on the branch are not in any of the series above, because they
enable no part of Vino: a scheduler call site that stops compiling under the
locking-guard series, and the Kms associated type Tyr needs once the KMS
registration trait requires one
TREE
}

prereq_block() {
    local group="$1" id desc had=0
    while IFS='|' read -r id desc; do
        [ -n "$id" ] || continue
        if [ "$had" -eq 0 ]; then
            printf 'It applies to the base above plus this, and nothing else:\n\n'
            had=1
        fi
        printf '  %s\n  https://lore.kernel.org/r/%s\n\n' "$desc" "$id"
    done < <(prereqs_of "$group")
    if [ "$had" -eq 0 ]; then
        cat <<'NONE'
It applies to the base above on its own, with no unmerged work under it, so it
can be taken without waiting for anything else here
NONE
        printf '\n'
    fi
    cat <<'TAIL'
The reference branch also carries Boqun Feng's counted interrupt disabling
series, which SpinLockIrq needs. One patch of it is already in tip locking/core
as e901c1510e24
TAIL
}

danilo_block() {
    cat <<'DANILO'
Danilo Krummrich's OwnedQueue, ScopedQueue and ScopedWork series supersedes part
of the workqueue work carried here, and is the better answer: Vino calls
Work::cancel_sync() in seven places to make teardown wait for its own work
items, and ScopedWork cancels on drop, which is that idiom done properly

  https://lore.kernel.org/r/20260807165252.3849875-1-dakr@kernel.org

It was still moving when this was cut, so this uses what is available today.
When it lands the swap goes in as one commit moving the prerequisites and the
call sites together, since either half alone leaves the tree not building
DANILO
}

disclosure_block() {
    cat <<'DISCLOSURE'
These patches were written with the assistance of Claude (Anthropic), used
through Claude Code as an interactive coding assistant, across the design, the
implementation and the tests. Every patch it contributed to carries an
Assisted-by trailer. The Signed-off-by is mine: I have reviewed and tested what
is here and I stand behind it
DISCLOSURE
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

Not posted with the Vino series: it enables no part of it. It belongs to the
scheduler, and it is a build fix for a series already in flight.
BLURB
        ;;
    drm-tyr) cat <<'BLURB'
The KMS registration trait gains a required Kms associated type. Tyr is a
render-only driver, so it selects the non-KMS PhantomData implementation just as
Nova does.

Not posted with the Vino series: it enables no part of it. It is a build fix for
the KMS series, and belongs with whichever tree takes that first.
BLURB
        ;;
    rust-core) cat <<'BLURB'
Core Rust abstractions a USB display driver needs. They are separated from the
driver because none of them are display specific: each one covers a kernel
facility that has C callers today and no Rust binding

  sync: single-shot and timed completions
  hrtimer: restart an ArcHrTimerHandle, and read the interrupt state inside a
    hard callback
  random, xxhash, time: safe wrappers over get_random_bytes(), xxh64() and
    ktime_get_real_seconds()
  workqueue: make OwnedQueue thread safe
  io: offset copy helpers that check the bounds they are given
  error: expose EPROTO

None of these nine has been posted before, so this goes out unversioned even
though the drivers it feeds are on their third round. A runtime platform-device
creator and a root-device attribute group went out inside the rust: drm v2
series, where they did not belong, and they are not here either, because the
consumer that needed them is not part of this posting

It is small on purpose. Every patch has a caller in the driver at the end of the
chain, and nothing is here on the argument that it might be useful to somebody
later
BLURB
        ;;
    rust-crypto) cat <<'BLURB'
Synchronous crypto bindings for a driver that has to authenticate a device
before it is allowed to drive it

The first patch covers AES-128, AES-CMAC, SHA-256 and HMAC over the existing
synchronous crypto API. The second adds RSA through akcipher, which HDCP 2.2
needs to verify a device certificate and wrap a session key

Changes since v2:

  The hand-rolled AES-CMAC is gone, along with its own dbl() subkey
    derivation. It delegates to the in-tree aes_cmac library through
    include/crypto/aes-cbc-macs.h, which is what Eric Biggers asked for
  There is no private RSA primitive either. Modexp goes through
    crypto_alloc_akcipher("rsa"), and OAEP padding and the HDCP key material
    are held in a memory-wiping secret type
  v2's separate CMAC fix is folded into the commit that introduces the CMAC,
    so this is two patches rather than three

Nothing here knows what HDCP is. The consumer is the DisplayLink driver at the
end of the chain, whose control plane is sealed with AES-CTR and keyed by an
HDCP 2.2 exchange
BLURB
        ;;
    rust-usb) cat <<'BLURB'
Host-side USB abstractions for a driver whose device is a bulk-endpoint pipe
rather than a class device

  Revocable typed interface I/O, so an interface cannot be used after the core
    has taken it back
  Reusable URBs and persistent bulk queues, which is what keeps a video stream
    in flight without allocating per transfer
  The device descriptor fields a driver needs to identify hardware before it
    decides to drive it, and a queue-readiness check
  A device id constructor matching on vendor together with interface class,
    subclass and protocol, for a composite device whose function is not
    identified by the product id alone
  Letting a driver keep its interface usable while unbinding, so teardown can
    still talk to the device it is releasing

Changes since v2:

  v2 10/11, "keep usb::Device private and gate ...", is dropped entirely.
    Oliver Neukum was right that it was conceptually wrong: USB does device
    level operations, and hiding that behind an interface is a layering
    violation. Device stays public
  as_bound() is gone from both the binding and the driver. Danilo Krummrich's
    point stands: needing an unsafe as_bound() means the design or the
    infrastructure is wrong, not that the escape hatch is needed
  reset_configuration() is gone, and set_interface() now exists in two
    correctly scoped forms, one on Interface<Bound> taking an altsetting and
    one on Device taking interface plus altsetting
  What replaces the concealment is lifecycle gating: an adapter-owned,
    revocable I/O window that is valid across probe, suspend, reset, resume and
    disconnect and invalid outside them. That is the interval in which I/O is
    legal, which is narrower than "the interface is bound"
  There is no private URB implementation. Colin Braun's URB RFC is carried
    unchanged as the foundation and this builds on it
  A topology walk and a device-removal notifier were written after v2 and are
    not here. They existed for a second consumer that is not part of this
    posting, so nothing in what is sent would call them

Alan Stern's lifecycle point is what makes device access from an interface
sound, and is worth restating because the whole shape depends on it: an
unconfigured device has no interfaces, so an interface that exists implies a
configured device
BLURB
        ;;
    rust-drm) cat <<'BLURB'
KMS abstractions for a Rust display driver, continuing Lyude Paul's KMS series
rather than replacing it. The first patch adapts that work to the DRM APIs as
they stand today, and the rest add what a real driver turned out to need

Broadly they fall into four groups:

  Lifetime and ownership: mode-object references tied to their owners, owned
    CRTC and vblank references, a safe constructor for owned registration data,
    pinning the owner while DRM files remain open, and rejecting cross-device
    GEM handle creation
  Properties a driver must read or publish: typed colour and rotation, plane
    blend mode, FB_DAMAGE_CLIPS, connector colorimetry and HDR metadata, and a
    connector's requested link depth
  Callbacks and state: connector detect() and mode_valid(), common state and
    connector helpers, checked plane geometry, walking the CRTCs an atomic
    commit carries, and CRTC mode changes
  Modes and framebuffers: an owned display mode constructor, mode flags and CTA
    VIC matching, synthesized CVT connector modes, and validated shmem scanout
    views

Also here are the HDCP 2.2 message identifiers, which are DRM UAPI rather than
driver constants

Changes since v2:

  Lyude's 43 commits are carried in order and patch-identical to the imported
    source, with her messages and tags untouched and no trailer of mine on any
    of them. v2 mixed her work with adaptations of mine and obscured the
    attribution, which was the fair complaint
  Everything of mine on top is a separate commit, and it is either an
    adaptation to a DRM API that has moved or a safety extension a driver
    needed
  The i2c adapter-provider patch is dropped. Igor Korotin's work is active and
    its provider-lifetime question is not one to answer privately in a driver,
    so the kernel driver registers no downstream I2C adapter this round
  The typed event channels and the private ioctl compat translations are
    dropped. Both existed for a second consumer that is not part of this
    posting, so neither has a user in what is sent
  The hardware-cursor support that was a separate v2 patch is folded into the
    plane work it belongs to

Two overlaps worth naming. Alvin Sun's
"Fix missing fops.owner in Rust DRM/misc abstractions" fixes the same bug as
"rust: drm: pin the owner while DRM files remain open", from the other end,
through ModuleMetadata rather than by threading an owning module through
UnregisteredDevice::new(). Theirs is the better shape and mine should be dropped
the moment it lands; it is still load-bearing today because upstream
UnregisteredDevice::new() takes no module. And where an early patch here fixes a
commit of Lyude's that is itself unmerged, that fix would be better folded into
her next revision than carried separately, and I am happy to do it that way
BLURB
        ;;
    rust-firmware) cat <<'BLURB'
request_firmware() covers "pull an image from /lib/firmware".
firmware_upload_register() covers the other half: userspace hands the driver an
image to write. It publishes /sys/class/firmware/<name>/ with the loading and
data handshake plus status, error, remaining_size and cancel, and is what a
driver uses when an image has to be written on demand rather than only when a
newer one happens to be on disk

This adds an Upload trait mirroring struct fw_upload_ops, a Registration that
unregisters on drop, and an Error enum whose values are the fw_upload_err codes
userspace already reads back out of the error attribute

rust: firmware: add request_into_buf() reached drm-rust-next this cycle and is
not a duplicate of this: that is the pull direction, and upstream still has no
binding for the push one

The consumer is the DisplayLink driver at the end of the chain, which writes
dock firmware over DFU. This has not been posted before, so it goes out
unversioned even though that driver is on its third round
BLURB
        ;;
    drm-vino) cat <<'BLURB'
Vino is a DRM/KMS driver for DisplayLink DL3 docks. These devices carry no
standard display protocol: the host encodes each frame with a vendor codec and
ships it over bulk USB, inside a control plane sealed with AES-CTR and keyed by
an HDCP 2.2 authentication exchange. Until now the only way to drive one on
Linux was an out-of-tree kernel module paired with a closed source userspace
daemon

The headline change since v2 is not a refactor. v1 and v2 never lit a panel.
This one does, on three generations of dock, driving a real desktop, from a cold
boot with nothing of the vendor's loaded

Three generations are supported, and they differ in more than identifiers:

  DL-3x00 (Ella), which shares one pipe between control and video, states its
    decoder tables in a narrow form, and must never be blanked by painting
    black, because its shared pipe halts and the session dies with the panel
    still lit
  DL-6xxx (Ridge), including the Dell D6000, which serves both connectors from
    a single EDID handler, so a fetch on an empty connector returns the other
    one's monitor
  DL-7400 (Navarro), four connectors over two video endpoints, 10 Gbps

The differences are data. A dock is placed by family into a DockProfile carrying
its endpoints, codec geometry, allocation rules and quirks, and there is one code
path through the driver for all three. No per-device branches, and no module
parameter selects a profile or a code path

On DL-7400 the driver drives 30 bpp in PQ: 2560x1440p120 on two connectors, with
the sink reporting 10 bit. Depth is not a flag on the wire but a set of
agreements, the DMA format, the colour-depth word, the framebuffer allocation,
and the entropy coder's escape ceilings, each of which is stated to the dock by
its own decoder code table. Getting one of them wrong is not a clean failure: a
DC ceiling the dock was not told about desynchronises the bitstream mid-record,
and an AC one stays in step while reconstructing every sharp edge from a
truncated magnitude

Firmware. A dock carries its running version in a vendor descriptor rather than
in bcdDevice, which does not move across an update, so the driver reads that,
compares it against the packaged image and writes a newer one over DFU. This is
how a dock too old to enumerate its connectors is brought forward. With nothing
installed the dock stays on whatever it shipped with, probe says so and carries
on, so the update path is opt-in by putting the file there

The images are DisplayLink's own, out of their Linux driver bundle, which
installs them to /opt/displaylink. Copy the ones you want into /lib/firmware/vino
under the names they already have:

  ella-dock-release.spkg      DL-3x00
  ridge-dock-release.spkg     DL-6xxx, the D6000
  navarro-dock-release.spkg   DL-7400

From the 6.8.1 bundle those carry 12.2.15, 12.2.25 and 12.2.26, and that is what
the three docks here are running. Each was written by this driver, from 11.4.47,
11.5.28 and 11.5.29 respectively, so the DFU path is exercised rather than only
read. A manual write is also there through /sys/class/firmware/vino-<dock>/,
which is the direction the firmware upload API is designed around

Tested on:

  HP 3005pr port replicator, DL-3900, two connectors at 1920x1080p60
  Dell Universal Dock D6000, 17e9:6006, two connectors at 2560x1440p120
  WAVLINK DL7400 quad dock, 17e9:7000, four connectors, two of them driven at
    2560x1440p120 in 30 bpp PQ

All three bind concurrently on the same host, with monitors attached, driving a
KDE desktop. 97 KUnit tests across 19 suites run at module load under
CONFIG_DRM_VINO_KUNIT_TEST

Changes since v2:

  It works, which v2 did not. The gate was one byte: the EDID engage message
    carries its connector selector in two places and the second was being filled
    with the message's random tail, so the dock acked it and then never enabled
    the downstream sink
  No raw C KMS anywhere. The driver is built on the safe KMS mode-object layer,
    which is what Danilo Krummrich asked for on v1, and git grep bindings::drm_
    over the driver returns nothing
  No unsafe block and no direct bindings:: call in the driver at all
  Three generations rather than one, and the second and third arrived without
    adding a branch, which is the test of whether the profile split was real
  The development history is folded away. This was 33 commits carrying a revert
    pair, a module parameter added and later deleted, and fixes to patches
    earlier in the same series. It is now 13 that introduce the driver in the
    order it is understood, and a fix to a commit this series adds is folded
    into that commit
  select DRM_GEM_SHMEM_HELPER is gone, since RUST_DRM_GEM_SHMEM_HELPER pulls it
    in, which Julian Braha pointed out
  The related series are linked and Vino is named as the user for all of them,
    which Miguel Ojeda asked for

The trace_crypto module parameter, default off, deliberately logs one session's
keys so that a USB capture of that session can be decrypted. Every constant in
this driver came from such a capture, and it is the only way somebody holding a
DisplayLink dock nobody here owns can produce one that says anything. It is
flagged here rather than left to be found, because a kernel option that
discloses key material is a fair thing to argue about

The protocol was reverse engineered from captured wire traffic and from the
vendor binaries. There is no vendor documentation for any of it, every constant
here came from a measurement, and the assistance noted below covers that work as
well as the implementation
BLURB
        ;;
    *) printf '*** BLURB HERE ***\n' ;;
    esac
}

rm -rf -- "$output"
mkdir -p "$output" "$output/not-posted"

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

# Counts first: the cover letters name each other's sizes.
declare -A counts
for group in "${series_order[@]}" "${carried_order[@]}"; do
    shas="${members[$group]:-}"
    [ -n "$shas" ] || { echo "error: series '$group' is empty" >&2; exit 1; }
    counts[$group]="$(printf '%s\n' $shas | wc -l)"
done

# format-patch is handed one commit at a time, because the commits of a series are
# not contiguous in the branch, so it cannot number them itself. Stamp the prefix
# afterwards: without it every patch goes out as a bare [PATCH] under a numbered
# cover letter, which tells a reviewer nothing about where in the series it sits.
emit_patches() {
    local dir="$1" shas="$2" prefix="$3" total="$4" sha n=0 f
    mkdir -p "$dir"
    for sha in $shas; do
        n=$((n + 1))
        git -C "$kernel_tree" format-patch --no-signature --quiet \
            --start-number "$n" --output-directory "$dir" -1 "$sha" >/dev/null
        f="$(find "$dir" -maxdepth 1 -name "$(printf '%04d' "$n")-*.patch")"
        [ -n "$f" ] || { echo "error: no patch emitted for $sha" >&2; exit 1; }
        sed -i "0,/^Subject: \\[PATCH\\] /s//Subject: [$prefix $n\\/$total] /" "$f"
        grep -q "^Subject: \\[$prefix $n/$total\\] " "$f" ||
            { echo "error: could not stamp the subject of $f" >&2; exit 1; }
    done
    ls "$dir" | grep -E '^[0-9]{4}-' | LC_ALL=C sort >"$dir/series"
}

diffstat_of() {
    local sha
    for sha in $1; do
        git -C "$kernel_tree" show --numstat --format= "$sha"
    done | awk '
        $3 != "" { add[$3] += $1; del[$3] += $2 }
        END {
            n = 0; a = 0; d = 0
            for (f in add) { n++; a += add[f]; d += del[f] }
            printf " %d file%s changed, %d insertion%s(+), %d deletion%s(-)\n",
                   n, n == 1 ? "" : "s", a, a == 1 ? "" : "s", d, d == 1 ? "" : "s"
        }'
}

exported=0
for group in "${series_order[@]}"; do
    shas="${members[$group]}"
    count="${counts[$group]}"
    dir="$output/$group"
    ver="$(reroll "$group")"
    emit_patches "$dir" "$shas" "PATCH${ver:+ v$ver}" "$count"
    exported=$((exported + count))

    prev="$(previous "$group")"
    subject_tag="PATCH${ver:+ v$ver}"

    # A cover letter git cannot generate itself: the commits are not contiguous in
    # the branch, so there is no range to hand format-patch.
    {
        printf 'From: Mike Lothian <%s>\n' "$author_email"
        printf 'Subject: [%s 0/%d] %s\n\n' "$subject_tag" "$count" "$(title "$group")"
        blurb "$group"
        printf '\n'
        if [ -n "$prev" ]; then
            printf 'v2: https://lore.kernel.org/r/%s\n\n' "$prev"
        fi
        siblings "$group"
        printf '\n'
        tree_block
        printf '\n'
        prereq_block "$group"
        printf '\n'
        case "$group" in
        rust-core|drm-vino) danilo_block; printf '\n' ;;
        esac
        # Documentation/process/generated-content.rst asks for the disclosure in the
        # cover letter, not only in the per-patch trailer.
        if grep -lq '^Assisted-by:' "$dir"/[0-9][0-9][0-9][0-9]-*.patch 2>/dev/null; then
            disclosure_block
            printf '\n'
        fi
        printf 'Mike Lothian (%d):\n' "$count"
        for sha in $shas; do
            git -C "$kernel_tree" show -s --format='  %s' "$sha"
        done
        printf '\n'
        diffstat_of "$shas"
        printf '\nbase-commit: %s\n' "$base_sha"
        while IFS='|' read -r pid _; do
            [ -n "$pid" ] && printf 'prerequisite-message-id: <%s>\n' "$pid"
        done < <(prereqs_of "$group")
    } >"$dir/0000-cover-letter.patch"

    printf '%-14s %2d %-7s %-8s depends: %s\n' \
        "$group" "$count" "$(plural "$count")" "${ver:+v$ver}" "$(depends "$group")"
done

for group in "${carried_order[@]}"; do
    shas="${members[$group]}"
    count="${counts[$group]}"
    dir="$output/not-posted/$group"
    emit_patches "$dir" "$shas" "PATCH" "$count"
    exported=$((exported + count))
    {
        printf 'From: Mike Lothian <%s>\n' "$author_email"
        printf 'Subject: [NOT POSTED 0/%d] %s\n\n' "$count" "$(title "$group")"
        blurb "$group"
    } >"$dir/0000-cover-letter.patch"
    printf '%-14s %2d %-7s %-8s not posted\n' "$group" "$count" "$(plural "$count")" ""
done

# Every commit of ours must land in exactly one series.
mine="$(git -C "$kernel_tree" rev-list --count --author="$author_email" "$kernel_base..$kernel_head")"
if [ "$exported" -ne "$mine" ]; then
    echo "error: exported $exported patches but $mine commits are ours" >&2
    exit 1
fi

{
    printf '# Kernel patch export\n\n'
    printf 'Generated by `tools/regenerate-patches.sh` from `%s`. Each directory is\n' "$kernel_head"
    printf 'an independent series, numbered from 0001 with its own cover letter, and is\n'
    printf 'posted on its own to the subsystem that owns it.\n\n'
    printf 'Only commits authored by %s are exported. The branch also carries other\n' "$author_email"
    printf "people's in-flight work -- Lyude Paul's KMS series, Boqun Feng's SpinLockIrq\n"
    printf 'series and others -- which are dependencies to base on, never patches to post.\n\n'
    printf '| series | patches | version | list | depends on |\n|---|---|---|---|---|\n'
    for group in "${series_order[@]}"; do
        ver="$(reroll "$group")"
        printf '| `%s` | %d | %s | %s | %s |\n' \
            "$group" "${counts[$group]}" "${ver:+v$ver}" "$(lists "$group")" "$(depends "$group")"
    done
    printf '\nApply order is the table order, and it is also send order: each cover letter\n'
    printf 'links the ones already sent, so fill in `tools/v3-message-ids.txt` after each\n'
    printf 'posting and re-run this before preparing the next.\n\n'
    printf '## Not posted\n\n'
    printf 'Under `not-posted/`. Both are build fixes the reference tree needs and neither\n'
    printf 'enables any part of Vino, so neither is sent alongside it.\n\n'
    for group in "${carried_order[@]}"; do
        printf '| `%s` | %d |\n' "$group" "${counts[$group]}"
    done
    printf '\nBase: `%s` (`%s`).\n' "$kernel_base" "$base_short"
} >"$output/README.md"

printf '\n%d patches: %d across %d posted series, %d carried and not posted\n' \
    "$exported" \
    "$(( exported - counts[sched-fair] - counts[drm-tyr] ))" \
    "${#series_order[@]}" \
    "$(( counts[sched-fair] + counts[drm-tyr] ))"
