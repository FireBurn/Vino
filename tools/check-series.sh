#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-2.0
#
# Verify the generated series before they are posted.
#
# Three things are checked, because v3 went out with all three wrong:
#
#   1. Every series applies to the declared base, on its own, in send order.
#      A series whose cover letter names third-party prerequisites is applied
#      on top of the branch state those provide rather than the bare base.
#   2. Applying all of them in send order reproduces the source branch's tree
#      object exactly, so what is posted is what was built and tested.
#   3. The result builds. Applying is not building: drm-vino applies to a bare
#      base and would not compile against it.
#
# Nothing here mails anything or touches the source branch.

set -euo pipefail

workspace="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
kernel_tree="${KERNEL_TREE:-$workspace/linux}"
# The tree the patches are applied to: the posting base plus the third-party
# series the cover letters declare (Ryhl's workqueue, Braun's URB, Lyude's KMS).
# integration/base-20260901 is the base a cover letter quotes; the prereqs tag is
# base plus exactly those, which is what a reviewer taking the prerequisites has.
kernel_base="${KERNEL_BASE:-integration/prereqs-20260901}"
posting_base="${POSTING_BASE:-integration/base-20260901}"
kernel_head="${KERNEL_HEAD:-vino}"
patch_root="$workspace/patches"

# Send order, which is also apply order.
series_order=(rust-core rust-crypto rust-usb rust-drm rust-firmware drm-vino)

# Carried but deliberately not posted: build fixes the reference tree needs that
# enable no part of Vino. They are applied only for the tree-identity check, so
# that the branch and the posting can be compared like for like.
carried_order=(sched-fair drm-tyr)

build=0
[ "${1:-}" = "--build" ] && build=1

for ref in "$kernel_base" "$posting_base" "$kernel_head"; do
    git -C "$kernel_tree" rev-parse --verify --quiet "$ref^{commit}" >/dev/null ||
        { echo "error: missing ref '$ref'" >&2; exit 2; }
done

mkdir -p "$workspace/.worktrees"
temporary="$(mktemp -d "$workspace/.worktrees/check.XXXXXX")"
worktree="$temporary/linux"

cleanup() {
    if [ -e "$worktree/.git" ]; then
        git -C "$kernel_tree" worktree remove --force "$worktree" >/dev/null 2>&1 || true
    fi
    rmdir "$temporary" 2>/dev/null || true
}
trap cleanup EXIT INT TERM

plural() { [ "$1" -eq 1 ] && printf 'patch ' || printf 'patches'; }

patches_of() {
    local group="$1" dir="$patch_root/$1"
    [ -d "$dir" ] || dir="$patch_root/not-posted/$1"
    find "$dir" -maxdepth 1 -name '[0-9][0-9][0-9][0-9]-*.patch' \
        ! -name '0000-*' | sort
}

# The prerequisites a series declares in its own cover letter, as lore ids. A
# series that names one cannot be expected to apply to the bare base, and this
# is the mismatch that put "depends on: none" in the README against three
# series that need somebody else's work under them.
declared_prereqs() {
    local cover="$patch_root/$1/0000-cover-letter.patch"
    [ -r "$cover" ] || return 0
    sed -n '/It applies to the base above plus this/,/^base-commit/p' "$cover" |
        grep -oE 'lore\.kernel\.org/r/[^ ]+' || true
}

echo "posting base: $posting_base ($(git -C "$kernel_tree" rev-parse --short "$posting_base"))"
echo "applied over: $kernel_base ($(git -C "$kernel_tree" rev-parse --short "$kernel_base")), which adds the declared prerequisites"
echo

# --- 1. each series on its own, over the base plus whatever came before it ---
#
# Send order is apply order, so series N is checked against base + series 1..N-1.
# That is exactly what a reviewer who has taken the earlier postings will have.
git -C "$kernel_tree" worktree add --detach --quiet "$worktree" "$kernel_base"

failed=0
for group in "${series_order[@]}"; do
    mapfile -t patches < <(patches_of "$group")
    if [ "${#patches[@]}" -eq 0 ]; then
        echo "error: no patches in $patch_root/$group" >&2
        exit 2
    fi

    prereqs="$(declared_prereqs "$group" | tr '\n' ' ')"
    if git -C "$worktree" am -3 --quiet "${patches[@]}" >/dev/null 2>&1; then
        printf '  %-14s %2d %s  applies\n' "$group" "${#patches[@]}" "$(plural "${#patches[@]}")"
    else
        git -C "$worktree" am --abort >/dev/null 2>&1 || true
        printf '  %-14s %2d %s  DOES NOT APPLY\n' "$group" "${#patches[@]}" "$(plural "${#patches[@]}")"
        [ -n "$prereqs" ] &&
            printf '  %-14s   declares prerequisites: %s\n' '' "$prereqs"
        failed=1
        break
    fi
done

if [ "$failed" -ne 0 ]; then
    echo
    echo "error: the series do not apply in send order over $kernel_base" >&2
    echo "       a series needing third-party work must say so in its cover letter," >&2
    echo "       and that work has to be in the base this is checked against" >&2
    exit 1
fi

# --- 2. the applied result is byte-for-byte the branch that was tested ---
#
# The two carried series go on here as well. They are not posted, but they are on
# the branch, so without them the comparison would always differ and say nothing.
for group in "${carried_order[@]}"; do
    mapfile -t patches < <(patches_of "$group")
    [ "${#patches[@]}" -gt 0 ] || continue
    if git -C "$worktree" am -3 --quiet "${patches[@]}" >/dev/null 2>&1; then
        printf '  %-14s %2d %s  applies (carried, not posted)\n' "$group" "${#patches[@]}" "$(plural "${#patches[@]}")"
    else
        git -C "$worktree" am --abort >/dev/null 2>&1 || true
        printf '  %-14s %2d %s  DOES NOT APPLY (carried)\n' "$group" "${#patches[@]}" "$(plural "${#patches[@]}")"
        exit 1
    fi
done

actual="$(git -C "$worktree" rev-parse HEAD^{tree})"
expected="$(git -C "$kernel_tree" rev-parse "$kernel_head^{tree}")"
if [ "$actual" != "$expected" ]; then
    echo
    echo "error: applied tree $actual does not match $kernel_head ($expected)" >&2
    echo "       regenerate the patches from the branch before sending" >&2
    exit 1
fi
echo
echo "series reproduce $kernel_head ($expected)"

# --- 3. and it builds ---
if [ "$build" -eq 0 ]; then
    echo "(pass --build to compile the result; applying is not building)"
    exit 0
fi

echo
echo "building the applied tree..."
cp "$kernel_tree/.config" "$worktree/.config" 2>/dev/null ||
    { echo "error: no .config in $kernel_tree to build with" >&2; exit 2; }
make -C "$worktree" LLVM=1 olddefconfig >/dev/null
if ! make -C "$worktree" LLVM=1 "-j$(nproc)" >/dev/null; then
    echo "error: the applied series does not build" >&2
    exit 1
fi
if ! make -C "$worktree" LLVM=1 "-j$(nproc)" modules >/dev/null; then
    echo "error: the applied series does not build modules" >&2
    exit 1
fi
test -f "$worktree/drivers/gpu/drm/vino/vino.ko" ||
    { echo "error: no vino.ko was produced -- CONFIG_DRM_VINO did not build" >&2; exit 1; }
echo "builds, and vino.ko was produced"
