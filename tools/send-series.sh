#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-2.0
#
# Send one exported series. Prepares by default; --dry-run and --send are the
# only ways anything leaves the machine.
#
# The series are sent in apply order, and each cover letter links the ones
# already sent. So the loop is:
#
#   tools/send-series.sh rust-core --send ...
#   record the Message-Id in tools/v3-message-ids.txt
#   tools/regenerate-patches.sh
#   tools/send-series.sh rust-crypto --send ...
#
# There is deliberately no In-Reply-To to the v2 threads.
# Documentation/process/submitting-patches.rst says not to attach a new revision
# of a multi-patch series to the old thread; the cover letters carry a lore link
# to v2 instead.

set -euo pipefail

usage() {
    cat <<'EOF'
usage: tools/send-series.sh SERIES [options]

Series, in send order:
  rust-core, rust-crypto, rust-usb, rust-drm, rust-firmware, drm-vino

Options:
  --to ADDRESS      primary recipient; repeatable
  --cc ADDRESS      explicit Cc; repeatable
  --dry-run         run git send-email --dry-run
  --send            actually send; deliberately never implied
  --no-maintainers  do not use scripts/get_maintainer.pl as --cc-cmd
EOF
}

workspace="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
kernel_tree="${KERNEL_TREE:-$workspace/linux}"
mode=prepare
use_maintainers=1
series=""
to_args=()
cc_args=()

while [ "$#" -gt 0 ]; do
    case "$1" in
        --to) to_args+=(--to "$2"); shift 2 ;;
        --cc) cc_args+=(--cc "$2"); shift 2 ;;
        --dry-run) mode=dry-run; shift ;;
        --send) mode=send; shift ;;
        --no-maintainers) use_maintainers=0; shift ;;
        -h|--help) usage; exit 0 ;;
        -*) echo "error: unknown option '$1'" >&2; usage >&2; exit 2 ;;
        *)
            [ -z "$series" ] || { echo "error: only one series may be selected" >&2; exit 2; }
            series="$1"; shift ;;
    esac
done

case "$series" in
    rust-core|rust-crypto|rust-usb|rust-drm|rust-firmware|drm-vino) ;;
    "") echo "error: a series is required" >&2; usage >&2; exit 2 ;;
    *) echo "error: unknown series '$series'" >&2; usage >&2; exit 2 ;;
esac

dir="$workspace/patches/$series"
cover="$dir/0000-cover-letter.patch"
[ -s "$cover" ] || {
    echo "error: no cover letter in $dir; run tools/regenerate-patches.sh first" >&2
    exit 2
}
if grep -q '\*\*\* BLURB HERE \*\*\*' "$cover"; then
    echo "error: $cover still has its placeholder" >&2
    exit 1
fi

mapfile -t mail_files < <(
    printf '%s\n' "$cover"
    find "$dir" -maxdepth 1 -type f -name '[0-9][0-9][0-9][0-9]-*.patch' \
        ! -name '0000-cover-letter.patch' | LC_ALL=C sort
)

# Anything sent to a list has to be ASCII-clean and free of the notes-to-self
# shorthand the working docs use.
if grep -qP '[^\x00-\x7F]' "$cover"; then
    echo "error: $cover has non-ASCII in it" >&2
    exit 1
fi

if [ "$mode" = prepare ]; then
    printf 'series %s: %d message(s)\n' "$series" "${#mail_files[@]}"
    printf '  %s\n' "${mail_files[@]}"
    printf '\nread the cover letter, then re-run with --dry-run or --send\n'
    exit 0
fi

if [ "${#to_args[@]}" -eq 0 ]; then
    echo "error: --to is required for --$mode" >&2
    exit 2
fi

send_args=("${to_args[@]}" "${cc_args[@]}")
if [ "$use_maintainers" -eq 1 ]; then
    send_args+=(--cc-cmd "$kernel_tree/scripts/get_maintainer.pl --norolestats")
fi
[ "$mode" = dry-run ] && send_args+=(--dry-run)

# Default threading is what is wanted here: the cover letter is the thread root
# and the patches reply to it. What is deliberately absent is any --in-reply-to
# pointing at v2.
git -C "$kernel_tree" send-email --thread --no-chain-reply-to \
    "${send_args[@]}" "${mail_files[@]}"

if [ "$mode" = send ]; then
    cat <<EOF

Sent. Now put the cover letter's Message-Id into
  $workspace/tools/v3-message-ids.txt
on the "$series" line, re-run tools/regenerate-patches.sh, and the next series'
cover letter will link this one.
EOF
fi
