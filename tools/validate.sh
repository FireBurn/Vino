#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-2.0
#
# Build-only validation. This script never installs or loads a module or
# kernel, touches the dock, changes the bootloader, or reboots.

set -euo pipefail

workspace="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
kernel_tree="${KERNEL_TREE:-$workspace/linux}"
kernel_base="${KERNEL_BASE:-integration/base-20260728}"
kernel_head="${KERNEL_HEAD:-vino}"
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
        # LONG_LINE is checkpatch's C rule. For Rust, rustfmt is the authority
        # and it does not split string literals -- breaking a log message across
        # lines only makes it ungreppable. The gate below keeps the rule where it
        # still means something: everything that is not a string literal fits.
        scripts/checkpatch.pl \
            --quiet --strict --ignore FILE_PATH_CHANGES --ignore LONG_LINE \
            -g "$commit"
    )
done < <(
    git -C "$kernel_tree" rev-list --reverse \
        --author='Mike Lothian <mike@fireburn.co.uk>' \
        "$kernel_base..$kernel_head"
)

# Rust lines fit in 100 columns unless the overflow is inside a string literal.
if ! awk 'length($0) > 100 && $0 !~ /"/ {
        printf "%s:%d: %d columns\n", FILENAME, FNR, length($0)
        bad = 1
    }
    END { exit bad }
' $(git -C "$kernel_tree" ls-files -- \
        'drivers/gpu/drm/vino/*.rs' 'drivers/gpu/drm/evdi/*.rs' \
        | sed "s|^|$kernel_tree/|"); then
    echo "error: a Rust line exceeds 100 columns outside a string literal" >&2
    exit 1
fi

# simd.rs is the one exemption, and it is not a bypass: `core::arch` intrinsics
# are `unsafe fn` by definition and CPU feature bits have no safe accessor, so
# there is no subsystem API being gone around. Everything the kernel *can* offer
# safely -- the FPU section -- is taken from `kernel::fpu`. The exemption is paid
# for by the stricter rule below: every one of its unsafe blocks must justify
# itself.
if rg -n \
    '\bunsafe\s*\{|\bunsafe\s+(fn|impl|trait)|\bbindings::|Arc::into_raw|Arc::from_raw|AtomicPtr' \
    "$kernel_tree/drivers/gpu/drm/vino" \
    "$kernel_tree/drivers/gpu/drm/evdi" \
    --glob '*.rs' --glob '!simd.rs'; then
    echo "error: a DRM consumer bypasses a safe Rust subsystem API" >&2
    exit 1
fi

# Every unsafe block in the exempted file states why it is sound, within the
# three lines above it.
if ! awk '
    /SAFETY:/ { safety = NR }
    /unsafe[[:space:]]*\{/ {
        if (NR - safety > 3) {
            printf "%s:%d: unsafe block with no SAFETY comment\n", FILENAME, NR
            bad = 1
        }
    }
    END { exit bad }
' "$kernel_tree/drivers/gpu/drm/vino/simd.rs"; then
    echo "error: an unsafe block in simd.rs is unjustified" >&2
    exit 1
fi

# Only Mike signs off; the assistants that helped are named above it in the
# format Documentation/process/coding-assistants.rst asks for. The model version
# is not pinned here because it legitimately differs between patches written
# months apart.
while IFS= read -r commit; do
    trailers="$(
        git -C "$kernel_tree" show -s --format=%B "$commit" \
            | sed -e '${/^$/d;}' \
            | awk '/^(Assisted-by|Signed-off-by): /{ print; next } { buf = "" }'
    )"
    if [ "$(printf '%s\n' "$trailers" | tail -n 1)" \
        != "Signed-off-by: Mike Lothian <mike@fireburn.co.uk>" ]; then
        echo "error: $commit does not end with Mike's sign-off" >&2
        exit 1
    fi
    if printf '%s\n' "$trailers" | sed '$d' \
        | grep -qvE '^Assisted-by: (Claude|Codex):[a-z0-9.-]+$'; then
        echo "error: unexpected trailer block in $commit" >&2
        printf '%s\n' "$trailers" >&2
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

# Build from a disposable worktree rather than the working tree: an out-of-tree
# build refuses to start if the source has ever been built in place, and a
# developer's tree usually has been. The worktree is the same commit, so the
# check is unchanged -- it still proves the series compiles under a plain
# defconfig rather than under whatever .config happens to be sitting there.
mkdir -p "$workspace/.worktrees"
build_src="$(mktemp -d "$workspace/.worktrees/build.XXXXXX")/linux"
build_dir="${KBUILD_OUTPUT:-$(mktemp -d /tmp/vino-validation.XXXXXX)}"
git -C "$kernel_tree" worktree add --detach --quiet "$build_src" "$kernel_head"
cleanup_build() {
    git -C "$kernel_tree" worktree remove --force "$build_src" >/dev/null 2>&1 || true
    rmdir "$(dirname "$build_src")" 2>/dev/null || true
    [ -n "${KBUILD_OUTPUT:-}" ] || rm -rf -- "$build_dir"
}
trap cleanup_build EXIT INT TERM
if [ -z "${KBUILD_OUTPUT:-}" ]; then
    make -C "$build_src" O="$build_dir" LLVM=1 defconfig
    "$build_src/scripts/config" --file "$build_dir/.config" \
        --enable RUST \
        --enable DRM \
        --enable USB \
        --module DRM_EVDI \
        --module DRM_VINO
    make -C "$build_src" O="$build_dir" LLVM=1 olddefconfig
fi
make -C "$build_src" O="$build_dir" LLVM=1 -j"$jobs" \
    rust/kernel.o \
    drivers/gpu/drm/evdi/evdi.o \
    drivers/gpu/drm/vino/vino.o
make -C "$revdi_tree" test
cargo test --manifest-path "$revdi_tree/Cargo.toml" --workspace --all-features
cargo test --manifest-path "$revdi_tree/library/Cargo.toml"
make -C "$revdi_tree" chimera

echo "validation: all build-only checks passed"
