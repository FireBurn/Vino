#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-2.0
#
# Regenerate the committed kernel patch export and its review-group manifests.

set -euo pipefail

workspace="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
kernel_tree="${KERNEL_TREE:-$workspace/linux}"
kernel_base="${KERNEL_BASE:-integration/base-20260728}"
kernel_head="${KERNEL_HEAD:-vino-upstream-rebuild}"
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

# These are independently reviewable, contiguous dependency groups. Keep the
# bounds in step with the documented series shape when commits are reorganised.
if [ "$(wc -l <"$output/series")" -ne 106 ]; then
    echo "error: expected the documented 106-patch series; update review-group bounds" >&2
    exit 1
fi
write_group interrupt-prerequisites 1 18
write_group kms-lyude 19 55
write_group drm-crypto-platform 56 73
write_group usb 74 80
write_group rust-runtime-drm 81 100
write_group evdi 101 101
write_group vino 102 106

printf 'kernel: %s patches (%s..%s)\n' \
    "$(wc -l <"$output/series")" \
    "$(git -C "$kernel_tree" rev-parse "$kernel_base")" \
    "$(git -C "$kernel_tree" rev-parse "$kernel_head")"
