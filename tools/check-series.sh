#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-2.0
#
# Apply the generated kernel series in a disposable worktree and verify that it
# reproduces the source branch's tree object exactly.

set -euo pipefail

workspace="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
kernel_tree="${KERNEL_TREE:-$workspace/linux}"
kernel_base="${KERNEL_BASE:-integration/base-20260809}"
kernel_head="${KERNEL_HEAD:-vino}"
patch_dir="$workspace/patches/kernel"

mkdir -p "$workspace/.worktrees"
temporary="$(mktemp -d "$workspace/.worktrees/check.XXXXXX")"
worktree="$temporary/linux"

cleanup() {
    if [ -e "$worktree/.git" ]; then
        git -C "$kernel_tree" worktree remove --force "$worktree" >/dev/null
    fi
    rmdir "$temporary" 2>/dev/null || true
}
trap cleanup EXIT INT TERM

patches=()
while IFS= read -r patch; do
    patches+=("$patch_dir/$patch")
done <"$patch_dir/series"
if [ "${#patches[@]}" -eq 0 ]; then
    echo "error: empty patch series in $patch_dir" >&2
    exit 2
fi

git -C "$kernel_tree" worktree add --detach "$worktree" "$kernel_base" >/dev/null
git -C "$worktree" am --quiet "${patches[@]}"

actual="$(git -C "$worktree" rev-parse HEAD^{tree})"
expected="$(git -C "$kernel_tree" rev-parse "$kernel_head^{tree}")"
if [ "$actual" != "$expected" ]; then
    echo "error: applied tree $actual does not match $kernel_head ($expected)" >&2
    exit 1
fi

echo "kernel: generated series reproduces $kernel_head ($expected)"
