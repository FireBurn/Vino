#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-2.0
#
# Regenerate the committed kernel and Revdi patch exports from their independent
# working repositories. Existing generated *.patch, series, and manifest.tsv
# files in the two exact output directories are replaced.

set -euo pipefail

workspace="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
kernel_tree="${KERNEL_TREE:-$workspace/kernel}"
kernel_base="${KERNEL_BASE:-integration/base-20260727}"
kernel_head="${KERNEL_HEAD:-series/vino-upstream}"
revdi_tree="${REVDI_TREE:-$workspace/revdi}"
revdi_base="${REVDI_BASE:-origin/main}"
revdi_head="${REVDI_HEAD:-main}"

require_repo() {
    local tree="$1"
    local label="$2"
    if ! git -C "$tree" rev-parse --git-dir >/dev/null 2>&1; then
        echo "error: $label tree is not a Git repository: $tree" >&2
        exit 2
    fi
}

require_ref() {
    local tree="$1"
    local ref="$2"
    if ! git -C "$tree" rev-parse --verify --quiet "$ref^{commit}" >/dev/null; then
        echo "error: missing commit '$ref' in $tree" >&2
        exit 2
    fi
}

export_series() {
    local tree="$1"
    local base="$2"
    local head="$3"
    local output="$4"
    local temporary

    mkdir -p "$output"
    temporary="$(mktemp -d "$workspace/.patch-export.XXXXXX")"
    git -C "$tree" format-patch \
        --no-signature \
        --numbered \
        --base="$base" \
        --output-directory "$temporary" \
        "$base..$head" >/dev/null

    find "$output" -maxdepth 1 -type f -name '*.patch' -delete
    find "$output" -maxdepth 1 -type f \
        \( -name series -o -name manifest.tsv \) -delete
    find "$temporary" -maxdepth 1 -type f -name '*.patch' -exec mv -t "$output" -- {} +
    rmdir "$temporary"

    find "$output" -maxdepth 1 -type f -name '*.patch' -printf '%f\n' \
        | LC_ALL=C sort >"$output/series"

    printf 'patch\tcommit\tauthor\tsubject\n' >"$output/manifest.tsv"
    while IFS= read -r patch; do
        local commit author subject
        commit="$(sed -n '1s/^From \([^ ]*\) .*$/\1/p' "$output/$patch")"
        author="$(git -C "$tree" show -s --format='%an <%ae>' "$commit")"
        subject="$(git -C "$tree" show -s --format='%s' "$commit")"
        printf '%s\t%s\t%s\t%s\n' "$patch" "$commit" "$author" "$subject" \
            >>"$output/manifest.tsv"
    done <"$output/series"

    printf '%s: %s patches (%s..%s)\n' \
        "$(basename "$output")" \
        "$(wc -l <"$output/series")" \
        "$(git -C "$tree" rev-parse "$base")" \
        "$(git -C "$tree" rev-parse "$head")"
}

mkdir -p "$workspace"
require_repo "$kernel_tree" kernel
require_repo "$revdi_tree" Revdi
require_ref "$kernel_tree" "$kernel_base"
require_ref "$kernel_tree" "$kernel_head"
require_ref "$revdi_tree" "$revdi_base"
require_ref "$revdi_tree" "$revdi_head"

export_series "$kernel_tree" "$kernel_base" "$kernel_head" "$workspace/patches/kernel"
export_series "$revdi_tree" "$revdi_base" "$revdi_head" "$workspace/patches/revdi"

