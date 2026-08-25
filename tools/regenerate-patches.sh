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
