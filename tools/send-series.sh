#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-2.0
#
# Prepare and, only with an explicit mode, dry-run or send one review group.

set -euo pipefail

usage() {
    cat <<'EOF'
usage: tools/send-series.sh GROUP [options]

Groups: interrupt-prerequisites, kms-lyude, drm-crypto-platform, usb,
        rust-runtime-drm, evdi, vino

Options:
  --version N       reroll number (default: 3)
  --output DIR      prepared mail directory (default: outgoing/GROUP-vN)
  --to ADDRESS      primary recipient; repeatable
  --cc ADDRESS      explicit Cc; repeatable
  --dry-run         run git send-email --dry-run after preparing
  --send            actually send; deliberately never implied
  --no-maintainers  do not use scripts/get_maintainer.pl as --cc-cmd
EOF
}

workspace="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
kernel_tree="${KERNEL_TREE:-$workspace/linux}"
version=3
mode=prepare
use_maintainers=1
output=""
group=""
to_args=()
cc_args=()

while [ "$#" -gt 0 ]; do
    case "$1" in
        --version) version="$2"; shift 2 ;;
        --output) output="$2"; shift 2 ;;
        --to) to_args+=(--to "$2"); shift 2 ;;
        --cc) cc_args+=(--cc "$2"); shift 2 ;;
        --dry-run) mode=dry-run; shift ;;
        --send) mode=send; shift ;;
        --no-maintainers) use_maintainers=0; shift ;;
        -h|--help) usage; exit 0 ;;
        -*) echo "error: unknown option '$1'" >&2; usage >&2; exit 2 ;;
        *)
            if [ -n "$group" ]; then
                echo "error: only one review group may be selected" >&2
                exit 2
            fi
            group="$1"
            shift
            ;;
    esac
done

case "$group" in
    interrupt-prerequisites|kms-lyude|drm-crypto-platform|usb|rust-runtime-drm|evdi|vino) ;;
    "") echo "error: a review group is required" >&2; usage >&2; exit 2 ;;
    *) echo "error: unknown review group '$group'" >&2; exit 2 ;;
esac
case "$version" in
    ''|*[!0-9]*) echo "error: --version must be a positive integer" >&2; exit 2 ;;
    0) echo "error: --version must be a positive integer" >&2; exit 2 ;;
esac

series="$workspace/patches/kernel/groups/$group.series"
if [ ! -s "$series" ]; then
    echo "error: missing $series; run tools/regenerate-patches.sh first" >&2
    exit 2
fi

commits=()
while IFS= read -r patch; do
    commit="$(sed -n '1s/^From \([^ ]*\) .*$/\1/p' "$workspace/patches/kernel/$patch")"
    commits+=("$commit")
done <"$series"
first="${commits[0]}"
last="${commits[${#commits[@]}-1]}"
if [ "$(git -C "$kernel_tree" rev-list --count "$first^..$last")" -ne "${#commits[@]}" ]; then
    echo "error: '$group' is not a contiguous commit range" >&2
    exit 1
fi

output="${output:-$workspace/outgoing/$group-v$version}"
if [ "$mode" = prepare ]; then
    mkdir -p "$output"
    find "$output" -maxdepth 1 -type f -name '*.patch' -delete
    git -C "$kernel_tree" format-patch \
        --no-signature \
        --numbered \
        --cover-letter \
        --reroll-count="$version" \
        --output-directory "$output" \
        "$first^..$last" >/dev/null
    echo "prepared ${#commits[@]} patches and cover letter in $output"
    echo "edit and review $output/v${version}-0000-cover-letter.patch before sending"
    exit 0
fi
if [ "${#to_args[@]}" -eq 0 ]; then
    echo "error: --to is required for --$mode" >&2
    exit 2
fi
cover="$output/v${version}-0000-cover-letter.patch"
if [ ! -f "$cover" ]; then
    echo "error: no prepared series in $output; run the command without --$mode first" >&2
    exit 2
fi
if rg -q '\*\*\* (SUBJECT|BLURB) HERE \*\*\*' "$cover"; then
    echo "error: edit the generated cover-letter placeholders before --$mode" >&2
    exit 2
fi

send_args=("${to_args[@]}" "${cc_args[@]}")
if [ "$use_maintainers" -eq 1 ]; then
    send_args+=(--cc-cmd "$kernel_tree/scripts/get_maintainer.pl --norolestats")
fi
if [ "$mode" = dry-run ]; then
    send_args+=(--dry-run)
fi

mapfile -t mail_files < <(find "$output" -maxdepth 1 -type f -name '*.patch' | LC_ALL=C sort)
git -C "$kernel_tree" send-email "${send_args[@]}" "${mail_files[@]}"
