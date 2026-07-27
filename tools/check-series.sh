#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-2.0
#
# Apply the generated patch sets in disposable worktrees and verify that each
# result has the same tree object as its source branch.

set -euo pipefail

workspace="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
kernel_tree="${KERNEL_TREE:-$workspace/kernel}"
kernel_base="${KERNEL_BASE:-integration/base-20260727}"
kernel_head="${KERNEL_HEAD:-series/vino-upstream}"
revdi_tree="${REVDI_TREE:-$workspace/revdi}"
revdi_base="${REVDI_BASE:-origin/main}"
revdi_head="${REVDI_HEAD:-main}"

mkdir -p "$workspace/.worktrees"
temporary="$(mktemp -d "$workspace/.worktrees/check.XXXXXX")"
kernel_worktree="$temporary/kernel"
revdi_worktree="$temporary/revdi"

cleanup() {
    if [ -e "$kernel_worktree/.git" ]; then
        git -C "$kernel_tree" worktree remove --force "$kernel_worktree" >/dev/null
    fi
    if [ -e "$revdi_worktree/.git" ]; then
        git -C "$revdi_tree" worktree remove --force "$revdi_worktree" >/dev/null
    fi
    rmdir "$temporary" 2>/dev/null || true
}
trap cleanup EXIT INT TERM

read_series() {
    local directory="$1"
    local -n destination="$2"
    destination=()
    while IFS= read -r patch; do
        destination+=("$directory/$patch")
    done <"$directory/series"
    if [ "${#destination[@]}" -eq 0 ]; then
        echo "error: empty patch series in $directory" >&2
        exit 2
    fi
}

compare_tree() {
    local worktree="$1"
    local source_tree="$2"
    local source_head="$3"
    local actual expected
    actual="$(git -C "$worktree" rev-parse HEAD^{tree})"
    expected="$(git -C "$source_tree" rev-parse "$source_head^{tree}")"
    if [ "$actual" != "$expected" ]; then
        echo "error: applied tree $actual does not match $source_head ($expected)" >&2
        exit 1
    fi
}

declare -a kernel_patches revdi_patches
read_series "$workspace/patches/kernel" kernel_patches
read_series "$workspace/patches/revdi" revdi_patches

git -C "$kernel_tree" worktree add --detach "$kernel_worktree" "$kernel_base" >/dev/null
git -C "$kernel_worktree" am --quiet "${kernel_patches[@]}"
compare_tree "$kernel_worktree" "$kernel_tree" "$kernel_head"
git -C "$kernel_tree" worktree remove --force "$kernel_worktree" >/dev/null
echo "kernel: generated series reproduces $kernel_head"

git -C "$revdi_tree" worktree add --detach "$revdi_worktree" "$revdi_base" >/dev/null
git -C "$revdi_worktree" am --quiet "${revdi_patches[@]}"
compare_tree "$revdi_worktree" "$revdi_tree" "$revdi_head"
git -C "$revdi_tree" worktree remove --force "$revdi_worktree" >/dev/null
echo "revdi: generated series reproduces $revdi_head"

