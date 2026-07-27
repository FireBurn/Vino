#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-2.0
#
# Sync revdi's vendored copies of the kernel sources from a kernel tree.
#
# revdi is a standalone project: it builds and tests without a kernel tree
# checked out next to it. It does that by keeping its own copies of two sets of
# kernel files:
#
#   module/*.rs        the EVDI module itself, built out-of-tree as evdi.ko
#   chimera/vino/*.rs  the vino driver's control-plane/codec sources, which
#                      chimera compiles VERBATIM in userspace (see chimera/README.md)
#
# The kernel tree is upstream of both: it is where those files are edited and
# where they get built into vmlinux/vino.ko. This script copies changes from
# there into this project, so a standalone checkout can be refreshed with one
# command instead of a hand-diff.
#
# Only the .rs sources are copied. Each side keeps its own build glue -- the
# kernel tree has Kconfig and an in-tree Makefile, revdi has module/Kbuild and
# an out-of-tree Makefile -- and those must not overwrite each other.
#
#   ./scripts/sync-kernel-sources.sh [--check] [KDIR]
#
#     --check   report drift and exit non-zero if any, copying nothing.
#               Suitable for CI or a pre-release gate.
#     KDIR      kernel tree to sync from. Defaults to $KDIR, then to ../kernel
#               (the layout inside the vino working repository).
#
# Exit status: 0 in sync / synced, 1 drift found (--check), 2 usage or bad tree.

set -u

check_only=0
kdir=""
for arg in "$@"; do
    case "$arg" in
        --check) check_only=1 ;;
        -h|--help) sed -n '3,30p' "$0"; exit 0 ;;
        -*) echo "unknown option: $arg" >&2; exit 2 ;;
        *) kdir="$arg" ;;
    esac
done

here="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
kdir="${kdir:-${KDIR:-$here/../kernel}}"

if [ ! -d "$kdir/drivers/gpu/drm" ]; then
    echo "error: '$kdir' is not a kernel tree (no drivers/gpu/drm)." >&2
    echo "Pass one explicitly: $0 [--check] /path/to/drm-next" >&2
    exit 2
fi
kdir="$(cd "$kdir" && pwd)"

# The vino sources chimera compiles verbatim, via chimera/src/kvino.rs's #[path].
# This list is deliberately explicit rather than a wildcard: the rest of the vino
# driver (vino.rs, drm_sink.rs, ...) is kernel-only code that cannot compile in
# userspace, and copying it here would only be noise. If kvino.rs grows a new
# module, the build breaks loudly and one line gets added here.
vino_srcs=(proto.rs cp.rs video.rs video_arm.rs ake.rs hdcp.rs)

drift=0
copied=0

# sync SRC DST -- report, and unless --check, copy.
sync_one() {
    local src="$1" dst="$2" label="$3"
    if [ ! -f "$src" ]; then
        echo "  MISSING in kernel tree: $label" >&2
        drift=1
        return
    fi
    if [ -f "$dst" ] && cmp -s "$src" "$dst"; then
        return
    fi
    drift=1
    if [ ! -f "$dst" ]; then
        echo "  NEW      $label"
    else
        echo "  DIFF     $label ($(diff "$dst" "$src" | grep -c '^[<>]') lines)"
    fi
    if [ "$check_only" -eq 0 ]; then
        install -m 644 "$src" "$dst"
        copied=$((copied + 1))
    fi
}

echo "kernel tree: $kdir"
echo

echo "module/ (evdi.ko sources) <- drivers/gpu/drm/evdi/"
mkdir -p "$here/module"
for f in "$kdir"/drivers/gpu/drm/evdi/*.rs; do
    sync_one "$f" "$here/module/$(basename "$f")" "module/$(basename "$f")"
done

echo "chimera/vino/ (compiled verbatim by chimera) <- drivers/gpu/drm/vino/"
mkdir -p "$here/chimera/vino"
for f in "${vino_srcs[@]}"; do
    sync_one "$kdir/drivers/gpu/drm/vino/$f" "$here/chimera/vino/$f" "chimera/vino/$f"
done

echo
if [ "$drift" -eq 0 ]; then
    echo "in sync with $kdir -- nothing to do."
    exit 0
fi
if [ "$check_only" -eq 1 ]; then
    echo "DRIFT: the vendored copies differ from $kdir."
    echo "Run without --check to update them."
    exit 1
fi
echo "synced $copied file(s) from $kdir."
echo "Review with 'git diff', then build: make && cargo test"
exit 0
