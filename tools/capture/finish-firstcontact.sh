#!/bin/bash
# Park a finished first-contact capture in the corpus, owned by the invoking user.
#
#   tools/capture/finish-firstcontact.sh [srcdir] [captures-dir]
#
# Run AFTER capture-firstcontact.sh has exited. It refuses to move a live capture: dumpcap and
# fw-watch hold open handles and the script writes its after-state snapshots to the original path,
# so relocating underneath them loses the tail of the run.
set -uo pipefail
# $HOME is root's under sudo, so default to the invoking user's home, not /root.
HOME_DIR="$(getent passwd "${SUDO_USER:-$(id -un)}" | cut -d: -f6)"
SRC="${1:-${HOME_DIR:-$HOME}/dlcap-firstcontact}"
CAPS="${2:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)/captures}"
USER_NAME="${SUDO_USER:-$(id -un)}"

say()  { printf '\033[1;36m==\033[0m %s\n' "$*"; }
die()  { printf '\033[1;31mABORT:\033[0m %s\n' "$*" >&2; exit 1; }

[ -d "$SRC" ] || die "no such capture directory: $SRC"
[ -d "$CAPS" ] || die "no captures directory: $CAPS"

if pgrep -f 'fw-watch\.py|dumpcap -i usbmon' >/dev/null; then
  pgrep -af 'fw-watch\.py|dumpcap -i usbmon' | grep -v 'bin/bash' | sed 's/^/   /'
  die "recorders are STILL RUNNING -- let capture-firstcontact.sh finish first"
fi

DEST="$CAPS/newdevice-firstcontact-$(date +%Y%m%d-%H%M%S)"
say "moving $SRC -> $DEST"
mv "$SRC" "$DEST" || die "move failed"
chown -R "$USER_NAME" "$DEST" 2>/dev/null || sudo chown -R "$USER_NAME" "$DEST"
say "owner: $(stat -c '%U:%G' "$DEST")   size: $(du -sh "$DEST" | cut -f1)"
say "contents:"
ls -la "$DEST" | tail -n +2 | awk '{printf "   %-34s %10s  %s\n", $9, $5, $1}' | grep -v '^\s*\.' 
echo
say "the proof a flash happened:"
diff "$DEST/before-ids.txt" "$DEST/after-ids.txt" && echo "   bcdDevice UNCHANGED"
