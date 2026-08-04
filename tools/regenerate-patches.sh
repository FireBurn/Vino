#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-2.0
#
# Regenerate the committed kernel patch export and its review-group manifests.

set -euo pipefail

workspace="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
kernel_tree="${KERNEL_TREE:-$workspace/linux}"
kernel_base="${KERNEL_BASE:-integration/base-20260728}"
kernel_head="${KERNEL_HEAD:-vino}"
output="$workspace/patches/kernel"

require_ref() {
    local ref="$1"
    if ! git -C "$kernel_tree" rev-parse --verify --quiet "$ref^{commit}" >/dev/null; then
        echo "error: missing kernel commit '$ref' in $kernel_tree" >&2
        exit 2
    fi
}

if ! git -C "$kernel_tree" rev-parse --git-dir >/dev/null 2>&1; then
    echo "error: kernel tree is not a Git repository: $kernel_tree" >&2
    exit 2
fi
require_ref "$kernel_base"
require_ref "$kernel_head"

mkdir -p "$output/groups"
temporary="$(mktemp -d "$workspace/.patch-export.XXXXXX")"
trap 'rm -rf -- "$temporary"' EXIT INT TERM

git -C "$kernel_tree" format-patch \
    --no-signature \
    --numbered \
    --base="$kernel_base" \
    --output-directory "$temporary" \
    "$kernel_base..$kernel_head" >/dev/null

find "$output" -maxdepth 1 -type f -name '*.patch' -delete
find "$output" -maxdepth 1 -type f \( -name series -o -name manifest.tsv \) -delete
find "$output/groups" -maxdepth 1 -type f -name '*.series' -delete
find "$temporary" -maxdepth 1 -type f -name '*.patch' -exec mv -t "$output" -- {} +

find "$output" -maxdepth 1 -type f -name '*.patch' -printf '%f\n' \
    | LC_ALL=C sort >"$output/series"

printf 'patch\tcommit\tauthor\tsubject\n' >"$output/manifest.tsv"
while IFS= read -r patch; do
    commit="$(sed -n '1s/^From \([^ ]*\) .*$/\1/p' "$output/$patch")"
    author="$(git -C "$kernel_tree" show -s --format='%an <%ae>' "$commit")"
    subject="$(git -C "$kernel_tree" show -s --format='%s' "$commit")"
    printf '%s\t%s\t%s\t%s\n' "$patch" "$commit" "$author" "$subject" \
        >>"$output/manifest.tsv"
done <"$output/series"

write_group() {
    local name="$1"
    local first="$2"
    local last="$3"
    sed -n "${first},${last}p" "$output/series" >"$output/groups/$name.series"
}

# These are independently reviewable, contiguous dependency groups, given as the
# first patch of each. The final group runs to the end of the series, so appending
# a commit to the tip needs no edit here; anything else does.
#
# Only the starts are pinned because a hardcoded total goes stale silently -- the
# previous "expected 106" guard was three commits behind the branch, so the export
# had been failing rather than being regenerated. The tiling check below turns any
# mismatch into an error that names the group instead.
group_starts=(
    "interrupt-prerequisites 1"
    "kms-lyude 19"
    "drm-crypto-platform 56"
    "usb 74"
    "rust-runtime-drm 81"
    "evdi 103"
    "vino 104"
)
total="$(wc -l <"$output/series")"
prev_end=0
for i in "${!group_starts[@]}"; do
    set -- ${group_starts[$i]}
    name="$1"
    first="$2"
    if [ "$first" -ne $((prev_end + 1)) ]; then
        echo "error: group '$name' starts at $first, leaving a gap or overlap after $prev_end" >&2
        echo "       the series is now $total patches; update the starts above" >&2
        exit 1
    fi
    if [ $((i + 1)) -lt ${#group_starts[@]} ]; then
        set -- ${group_starts[$((i + 1))]}
        last=$(( $2 - 1 ))
    else
        last="$total"
    fi
    if [ "$last" -lt "$first" ]; then
        echo "error: group '$name' is empty ($first..$last)" >&2
        exit 1
    fi
    write_group "$name" "$first" "$last"
    prev_end="$last"
done

# Keep the export's own README honest about what it was generated from.
readme="$output/README.md"
if [ -f "$readme" ]; then
    base_sha="$(git -C "$kernel_tree" rev-parse "$kernel_base")"
    head_sha="$(git -C "$kernel_tree" rev-parse "$kernel_head")"
    sed -i \
        -e "s|^base: .*|base: $base_sha|" \
        -e "s|^head: .*|head: $head_sha|" \
        -e "s|^range: .*|range: $kernel_base..$kernel_head|" \
        "$readme"
fi

printf 'kernel: %s patches (%s..%s)\n' \
    "$(wc -l <"$output/series")" \
    "$(git -C "$kernel_tree" rev-parse "$kernel_base")" \
    "$(git -C "$kernel_tree" rev-parse "$kernel_head")"
