#!/bin/bash
# Describe a vino head's sink with an EDID read somewhere else, when the dock cannot read it here.
#
# A DP->HDMI converter that mangles or drops DDC leaves the dock unable to read the monitor at all:
# the presence probe reports the socket occupied, but no `id=0x194` EDID ever comes back, so the
# head stays disconnected and nothing is ever driven. The monitor is real, and its EDID is readable
# from a working port on another machine.
#
# This uses DRM's own per-connector override (debugfs `edid_override`), which the probe helper
# applies only to a connector that reports CONNECTED and produced NO modes of its own -- which is
# exactly what `vino edid_override=<mask>` makes such a head do.
#
#   sudo tools/hardware/vino-cycle.sh edid_override=1        # head 0; bit N = head N
#   sudo tools/hardware/vino-edid-override.sh 0 tools/hardware/edid/samsung-qe75q60a.edid.bin
#
# ⚠ An override describes the SINK, not the LINK. If the converter cannot carry the mode the blob
# advertises, the screen stays black in exactly the same way -- this substitutes for a broken read,
# it does not negotiate anything. Prefer a blob whose preferred timing the link can actually do.
set -uo pipefail

say() { printf '\033[1;36m==\033[0m %s\n' "$*"; }
die() { printf '\033[1;31mABORT:\033[0m %s\n' "$*" >&2; exit 1; }

[ "$(id -u)" = 0 ] || die "run with sudo"
[ $# -eq 2 ] || die "usage: $0 <head> <edid.bin>"

HEAD="$1"
BLOB="$2"
[ -r "$BLOB" ] || die "cannot read $BLOB"

# Catch the two mistakes that produce a connector with no modes and no explanation: a hex dump or
# an `edid-decode` transcript instead of the raw blob, and a truncated read.
SIZE=$(stat -c %s "$BLOB")
[ $((SIZE % 128)) -eq 0 ] && [ "$SIZE" -ge 128 ] ||
  die "$BLOB is $SIZE bytes -- an EDID is a whole number of 128-byte blocks (is this a hex dump?)"
head -c 8 "$BLOB" | od -An -tx1 | grep -q '00 ff ff ff ff ff ff 00' ||
  die "$BLOB does not start with the EDID header magic -- this is not a raw EDID"

# The card vino owns, and the sysfs connector for this head. vino registers one connector per head
# in head order and types them DisplayPort, so head N is DP-(N+1) on its own card.
CARD=""
for d in /sys/class/drm/card*; do
  [ -e "$d/device/driver" ] || continue
  case "$(readlink -f "$d/device/driver")" in */vino) CARD=$(basename "$d");; esac
done
[ -n "$CARD" ] || die "no DRM card is bound to vino -- is the module loaded and the dock bound?"

CONN="DP-$((HEAD + 1))"
SYSFS="/sys/class/drm/$CARD-$CONN"
[ -d "$SYSFS" ] || die "$SYSFS does not exist (head $HEAD is not a connector on $CARD)"

# debugfs indexes DRM devices by device name, not by card number.
DEVNAME=$(basename "$(readlink -f "/sys/class/drm/$CARD/device")")
DBG="/sys/kernel/debug/dri/$DEVNAME/$CONN"
[ -w "$DBG/edid_override" ] || die "$DBG/edid_override is missing -- is debugfs mounted?"

STATUS=$(cat "$SYSFS/status")
say "$CARD $CONN (head $HEAD) is currently $STATUS"
if [ "$STATUS" != connected ]; then
  # The override is a fallback for a CONNECTED connector: the core never consults it otherwise.
  # vino publishes the head a few seconds after bring-up, on its re-engage retry.
  printf '\033[1;33mnote:\033[0m the core only applies an override to a CONNECTED connector.\n'
  printf '      Load with `edid_override=%d` and give the re-engage retry ~5 s, then rerun.\n' \
    $((1 << HEAD))
fi

say "writing $SIZE bytes to $DBG/edid_override"
cat "$BLOB" > "$DBG/edid_override" || die "override write rejected -- the core parsed it as invalid"

# fill_modes() -- the one path that consults the override. A plain re-read of `modes` would not.
say "forcing a re-probe"
echo detect > "$SYSFS/status"

say "$CONN is now $(cat "$SYSFS/status"), offering:"
nl -ba "$SYSFS/modes" | head -20
COUNT=$(wc -l < "$SYSFS/modes")
if [ "$COUNT" = 0 ]; then
  printf '\033[1;31mno modes:\033[0m the override did not take. Check `dmesg | grep -i edid` and\n'
  printf '          that vino was loaded with edid_override=%d.\n' $((1 << HEAD))
else
  say "$COUNT mode(s). Now set one, e.g.:"
  printf '    kscreen-doctor output.%s.mode.0\n' "$CONN"
fi
