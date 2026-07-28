#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-2.0
#
# Build-only validation. This script never installs or loads a module or
# kernel, touches the dock, changes the bootloader, or reboots.

set -euo pipefail

workspace="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
kernel_tree="${KERNEL_TREE:-$workspace/linux}"
kernel_base="${KERNEL_BASE:-integration/base-20260728}"
kernel_head="${KERNEL_HEAD:-vino-upstream-rebuild}"
revdi_tree="$workspace/revdi"
jobs="${JOBS:-16}"

"$workspace/tools/regenerate-patches.sh"
"$workspace/tools/check-series.sh"

git -C "$kernel_tree" diff --check "$kernel_base..$kernel_head"
git -C "$workspace" diff --check -- . ':(exclude,glob)patches/kernel/*.patch'

# Check every patch we authored; third-party series retain their own review
# state and are verified for patch/message identity below.
while IFS= read -r commit; do
    (
        cd "$kernel_tree"
        scripts/checkpatch.pl \
            --quiet --strict --ignore FILE_PATH_CHANGES \
            -g "$commit"
    )
done < <(
    git -C "$kernel_tree" rev-list --reverse \
        --author='Mike Lothian <mike@fireburn.co.uk>' \
        "$kernel_base..$kernel_head"
)

if rg -n \
    '\bunsafe\s*\{|\bunsafe\s+(fn|impl|trait)|\bbindings::|Arc::into_raw|Arc::from_raw|AtomicPtr' \
    "$kernel_tree/drivers/gpu/drm/vino" \
    "$kernel_tree/drivers/gpu/drm/evdi" \
    --glob '*.rs'; then
    echo "error: a DRM consumer bypasses a safe Rust subsystem API" >&2
    exit 1
fi

expected_trailers=$'Assisted-by: Claude:claude-opus-5-0\nAssisted-by: Codex:gpt-5\nSigned-off-by: Mike Lothian <mike@fireburn.co.uk>'
while IFS= read -r commit; do
    trailers="$(
        git -C "$kernel_tree" show -s --format=%B "$commit" \
            | sed -e '${/^$/d;}' \
            | tail -n 3
    )"
    if [ "$trailers" != "$expected_trailers" ]; then
        echo "error: unexpected Mike trailer block in $commit" >&2
        exit 1
    fi
done < <(
    git -C "$kernel_tree" rev-list \
        --author='Mike Lothian <mike@fireburn.co.uk>' \
        "$kernel_base..$kernel_head"
)

if git -C "$kernel_tree" log \
    --author='Lyude Paul' \
    --format='%H%n%B%n--END--' \
    "$kernel_base..$kernel_head" \
    | rg '^(Assisted-by:|Signed-off-by: Mike Lothian|Co-developed-by: Mike Lothian)'; then
    echo "error: a Lyude-authored patch carries a Mike or assistance trailer" >&2
    exit 1
fi

external_reference=backup/vino-usb-unsplit-20260728
for author in 'Lyude Paul' 'Colin Braun'; do
    diff -u \
        <(
            git -C "$kernel_tree" log -p --author="$author" \
                "$kernel_base..$external_reference" \
                | git patch-id --stable \
                | awk '{print $1}' \
                | LC_ALL=C sort
        ) \
        <(
            git -C "$kernel_tree" log -p --author="$author" \
                "$kernel_base..$kernel_head" \
                | git patch-id --stable \
                | awk '{print $1}' \
                | LC_ALL=C sort
        )
done

compare_external_commit() {
    local original="$1"
    local author subject current
    author="$(git -C "$kernel_tree" show -s --format=%ae "$original")"
    subject="$(git -C "$kernel_tree" show -s --format=%s "$original")"
    current="$(
        git -C "$kernel_tree" log --format=%H --author="$author" \
            --fixed-strings --grep="$subject" "$kernel_base..$kernel_head" \
            | head -n 1
    )"
    if [ -z "$current" ]; then
        echo "error: missing external patch '$subject'" >&2
        exit 1
    fi
    diff -u \
        <(git -C "$kernel_tree" show "$original" | git patch-id --stable | cut -d' ' -f1) \
        <(git -C "$kernel_tree" show "$current" | git patch-id --stable | cut -d' ' -f1)
    diff -u \
        <(git -C "$kernel_tree" show -s --format=%B "$original") \
        <(git -C "$kernel_tree" show -s --format=%B "$current")
}

compare_external_commit a62794b6f18f500e11e24508f359e25098085315
compare_external_commit 9f600d4e9b33039a58e85755de08e57f9f900e08
compare_external_commit 2af162b390e2009919c9368e4925e0bd998d64f6
compare_external_commit 6d9ec0afc7731332c98d946f2400a1d6928f8cda

make -C "$revdi_tree" check-sync KSRC="$kernel_tree"
cargo fmt --manifest-path "$revdi_tree/Cargo.toml" --all -- --check
cargo fmt --manifest-path "$revdi_tree/library/Cargo.toml" -- --check

if [ "${SKIP_BUILD:-0}" = "1" ]; then
    echo "validation: source, history, patch, formatting, and sync checks passed"
    exit 0
fi

build_dir="${KBUILD_OUTPUT:-$(mktemp -d /tmp/vino-validation.XXXXXX)}"
if [ -z "${KBUILD_OUTPUT:-}" ]; then
    trap 'rm -rf -- "$build_dir"' EXIT INT TERM
    make -C "$kernel_tree" O="$build_dir" LLVM=1 defconfig
    "$kernel_tree/scripts/config" --file "$build_dir/.config" \
        --enable RUST \
        --enable DRM \
        --enable USB \
        --module DRM_EVDI \
        --module DRM_VINO
    make -C "$kernel_tree" O="$build_dir" LLVM=1 olddefconfig
fi
make -C "$kernel_tree" O="$build_dir" LLVM=1 -j"$jobs" \
    rust/kernel.o \
    drivers/gpu/drm/evdi/evdi.o \
    drivers/gpu/drm/vino/vino.o
make -C "$revdi_tree" test
cargo test --manifest-path "$revdi_tree/Cargo.toml" --workspace --all-features
cargo test --manifest-path "$revdi_tree/library/Cargo.toml"
make -C "$revdi_tree" chimera

echo "validation: all build-only checks passed"
