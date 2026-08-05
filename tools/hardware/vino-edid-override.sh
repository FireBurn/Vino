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
# in head order and types them DisplayPort -- but the DP type index is NOT 1-based per card: this
# machine's vino card starts at DP-2, because the connector type ids continue from another device.
# So take the card's DP connectors in numeric order and index by head, never by arithmetic.
CARD=""
for d in /sys/class/drm/card*; do
  [ -e "$d/device/driver" ] || continue
  case "$(readlink -f "$d/device/driver")" in */vino) CARD=$(basename "$d");; esac
done
[ -n "$CARD" ] || die "no DRM card is bound to vino -- is the module loaded and the dock bound?"

mapfile -t CONNS < <(
  for c in /sys/class/drm/"$CARD"-DP-*; do [ -d "$c" ] && basename "$c"; done |
    sed "s/^$CARD-//" | sort -t- -k2 -n
)
[ "${#CONNS[@]}" -gt "$HEAD" ] ||
  die "$CARD has ${#CONNS[@]} DP connector(s); head $HEAD is not one of them"
CONN="${CONNS[$HEAD]}"
SYSFS="/sys/class/drm/$CARD-$CONN"
say "head $HEAD is $CARD $CONN (of ${CONNS[*]})"

# debugfs indexes DRM devices by device name, not by card number.
DEVNAME=$(basename "$(readlink -f "/sys/class/drm/$CARD/device")")
DBG="/sys/kernel/debug/dri/$DEVNAME/$CONN"
[ -w "$DBG/edid_override" ] || die "$DBG/edid_override is missing -- is debugfs mounted?"

say "$CARD $CONN (head $HEAD) is currently $(cat "$SYSFS/status")"

# Order matters, and not only for tidiness. The core applies an override to a connector that is
# CONNECTED and produced NO modes -- and if the override is missing it falls back to adding a
# 1024x768 mode instead. A modeless connector that goes connected is mode-set by fbdev emulation
# within milliseconds, and driving the dock at such a default RESETS it, into a re-enumeration
# loop. So: install the description first, and only force the connector on once it is in place.
say "writing $SIZE bytes to $DBG/edid_override"
cat "$BLOB" > "$DBG/edid_override" || die "override write rejected -- the core parsed it as invalid"

# DRM_FORCE_ON: status becomes connected without consulting the driver's detect(), and fill_modes()
# runs -- the one path that consults the override. `detect` would merely ask vino again, which has
# no EDID to report. Restore later with `echo detect > $SYSFS/status`.
say "forcing $CONN on"
echo on > "$SYSFS/status"

say "$CONN is now $(cat "$SYSFS/status"), offering:"
nl -ba "$SYSFS/modes" | head -20
COUNT=$(wc -l < "$SYSFS/modes")
if [ "$COUNT" = 0 ]; then
  printf '\033[1;31mno modes:\033[0m the override did not take. Check `dmesg | grep -i edid` and\n'
  printf '          that vino was loaded with edid_override=%d.\n' $((1 << HEAD))
  # A forced-on connector carrying only the core's 1024x768 consolation mode is exactly the state
  # that resets this dock. Put it back rather than leave it armed.
  echo detect > "$SYSFS/status"
  die "released the force on $CONN rather than leave a modeless connector armed"
elif [ "$COUNT" -le 2 ] && grep -qx '1024x768' "$SYSFS/modes"; then
  printf '\033[1;31m1024x768 only:\033[0m that is the core'\''s fallback for a connected connector\n'
  printf '               with no modes -- the override did NOT take.\n'
  echo detect > "$SYSFS/status"
  die "released the force on $CONN rather than drive the dock at a fallback mode"
else
  say "$COUNT mode(s). Now set one, e.g.:"
  printf '    kscreen-doctor output.%s.mode.0\n' "$CONN"
fi
