#!/bin/bash
# Keep vino off ONE dock so the other can be measured on its own.
#
# Two docks bound at once interfere during bring-up (the D6000's control session times out while
# the DL7400 authenticates), and a dock that hard-resets re-enumerates every few seconds, so a
# plain one-shot unbind does not hold. This re-unbinds whenever the driver core re-probes.
#
#   sudo tools/hardware/vino-hold-off.sh 7000    # measure the D6000 alone
#   sudo tools/hardware/vino-hold-off.sh 6006    # measure the DL7400 alone
#
# Ctrl-C to release. Nothing is left behind: re-bind by reloading with tools/hardware/vino-cycle.sh.
#
# ⚠ Resolve the dock by idProduct on every pass. Bus paths change on every re-enumeration -- the
# D6000 has appeared as 2-2.1 and 1-2.4 within one session -- so a hardcoded path silently stops
# matching exactly when the dock is misbehaving enough to be worth measuring.
set -uo pipefail

PID="${1:-}"
case "$PID" in
  6006|7000) ;;
  *) echo "usage: $0 <6006|7000>   (idProduct of the dock to hold vino off)" >&2; exit 1 ;;
esac
[ "$(id -u)" = 0 ] || { echo "run with sudo" >&2; exit 1; }

echo "holding vino off idProduct=$PID (Ctrl-C to release)"
while true; do
  for d in /sys/bus/usb/devices/*/; do
    [ "$(cat "$d/idProduct" 2>/dev/null)" = "$PID" ] || continue
    b=$(basename "$d")
    for i in "$b:1.0" "$b:1.1"; do
      if [ -e "/sys/bus/usb/drivers/vino/$i" ]; then
        echo "$i" > /sys/bus/usb/drivers/vino/unbind 2>/dev/null && echo "unbound $i"
      fi
    done
  done
  sleep 0.2
done
