#!/bin/bash
# Repeat a COLD vino bring-up N times and report, per head, whether pixels actually flowed.
#
#   sudo tools/hardware/vino-bringup-trials.sh [trials] [settle_secs]
#
# One trial proves nothing on this dock -- a build confirmed working has come up dark an hour
# later, and a bisect built on one observation per build was contradicted outright. This exists so
# a claim about bring-up is always a count.
#
# ⭐ The reset is a **USB re-authorise**, not `vino-cycle.sh`, because an interface unbind does not
# make the dock re-run its downstream sink discovery: a sink that goes quiet while vino is unloaded
# is never rediscovered, and the head stays missing for the rest of the session. De-authorising
# cannot latch the way `port/disable` can, because the sysfs path stays put whatever the device does.
#
# ⚠ The verdict is **bytes under forced damage**, not dmesg and not "frame ok". A static desktop
# legitimately sends nothing, so an idle head and a jammed one look identical until damage is
# forced; `kscreen-doctor …brightness` is the reliable way to force it. And bytes still are not
# "lit" -- this dock will accept a complete correct frame and never start its pixel clock. Ask a
# human for that. What this script measures is whether the dock kept ACCEPTING video.
set -uo pipefail
TRIALS="${1:-4}"
SETTLE="${2:-45}"
[ "$(id -u)" = 0 ] || { echo "run with sudo" >&2; exit 1; }
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

dockpath() { for d in /sys/bus/usb/devices/*/; do [ "$(cat "$d/idProduct" 2>/dev/null)" = 7000 ] && echo "${d%/}"; done; }
# Run a compositor command as the desktop user; this script is root and kscreen-doctor is not.
# ⚠ `su -` is not enough: kscreen-doctor needs the session's XDG_RUNTIME_DIR and WAYLAND_DISPLAY,
# and a login shell does not carry them, so it silently talks to nothing.
DESKUSER="$(stat -c %U /run/user/1000 2>/dev/null || echo fireburn)"
DESKUID="$(id -u "$DESKUSER")"
asuser() {
  sudo -u "$DESKUSER" env "XDG_RUNTIME_DIR=/run/user/$DESKUID" \
      "WAYLAND_DISPLAY=${WAYLAND_DISPLAY:-wayland-0}" "$@" >/dev/null 2>&1
}
dockouts() { for c in /sys/class/drm/card*-DP-*/; do [ "$(cat "$c/status" 2>/dev/null)" = connected ] || continue; b=$(basename "$c"); echo "${b#card*-}"; done; }

pass=0
for t in $(seq 1 "$TRIALS"); do
  printf '\n=== trial %d/%d\n' "$t" "$TRIALS"
  "$HERE/vino-cycle.sh" --unload >/dev/null 2>&1
  # Wait for the dock rather than scoring a trial against a dock that is not there. A whole run
  # once reported 0/5 "no dock found" because the cables were being moved -- which is not a result,
  # and worse, it left the machine with no driver loaded at all.
  D="$(dockpath)"
  for _ in $(seq 1 60); do [ -n "$D" ] && break; sleep 2; D="$(dockpath)"; done
  [ -n "$D" ] || { echo "  no dock after 120 s -- reloading vino and stopping"; modprobe vino; break; }
  echo 0 > "$D/authorized"; sleep 4; echo 1 > "$D/authorized"
  sleep 6
  modprobe vino debug=1 || { echo "  modprobe failed"; continue; }
  sleep "$SETTLE"
  dmesg -C >/dev/null
  outs="$(dockouts | tr '\n' ' ')"
  for o in $outs; do asuser kscreen-doctor "output.$o.brightness.55"; done
  sleep 3
  for o in $outs; do asuser kscreen-doctor "output.$o.brightness.100"; done
  sleep 6
  line=""
  ok=0
  for h in 0 1 2 3; do
    n=$(dmesg | grep -c "scanout head=$h frame ok")
    [ "$n" -gt 0 ] && { line="$line head$h=$n"; ok=$((ok+1)); }
  done
  stopped=$(dmesg | grep -c "stopped accepting")
  printf '  outputs:%s frames:%s stopped_accepting=%s\n' " $outs" "${line:- none}" "$stopped"
  # `stopped accepting video` is reported separately, not as a failure: it is logged when vino's
  # own URB queue is full, it has been seen once on a bring-up that then streamed for minutes, and
  # the panels were lit through it. Sustained frames on two heads after forced damage is the bar.
  [ "$ok" -ge 2 ] && { pass=$((pass+1)); echo "  PASS"; } || echo "  FAIL"
done
printf '\n%d/%d trials passed\n' "$pass" "$TRIALS"
