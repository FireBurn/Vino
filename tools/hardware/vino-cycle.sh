#!/bin/bash
# Reload vino without physically unplugging the dock, and without unloading under an open fd.
#
# This build has no `/sys/devices/vino/remove_all`, so the old `vino-reload.sh` path is gone. USB
# unbind reaches the same place: it runs the driver's `disconnect()`, which calls
# `drm_dev_unplug()`, so DRM clients get -ENODEV and close the device of their own accord. Only
# once nothing holds the card is `modprobe -r` safe -- unloading under a live fd frees the fops
# underneath the compositor and hangs the machine.
#
#   tools/hardware/vino-cycle.sh            # unbind, unload, load, rebind by re-probing
#   tools/hardware/vino-cycle.sh --unload   # leave it unloaded (for DLM/evdi work)
set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

DRV=/sys/bus/usb/drivers/vino
UNLOAD_ONLY=0
[ "${1:-}" = "--unload" ] && UNLOAD_ONLY=1
MODULE_ARGS=("$@")

say() { printf '\033[1;36m==\033[0m %s\n' "$*"; }
die() { printf '\033[1;31mABORT:\033[0m %s\n' "$*" >&2; exit 1; }
[ "$(id -u)" = 0 ] || die "run with sudo"

vino_nodes() {
  for d in /sys/class/drm/card*; do
    [ -e "$d/device/driver" ] || continue
    case "$(readlink -f "$d/device/driver")" in */vino) basename "$d";; esac
  done
}

if lsmod | grep -q '^vino'; then
  NODES=$(vino_nodes)
  say "vino DRM nodes: ${NODES:-none}"

  # Unbind every interface vino claimed. This is the software equivalent of a dock unplug.
  for i in $(ls "$DRV" 2>/dev/null | grep -E '^[0-9]+-'); do
    say "unbinding $i"
    echo "$i" > "$DRV/unbind" 2>/dev/null || true
  done

  # Wait for the compositor to drop the card. drm_dev_unplug() makes this happen on its own; it is
  # not instant, so poll rather than race it.
  HOLDERS=""
  for _ in $(seq 1 40); do
    HOLDERS=$("$HERE/drm-fd-holders.py" $NODES 2>/dev/null || true)
    [ -z "$HOLDERS" ] && break
    sleep 0.5
  done
  if [ -n "$HOLDERS" ]; then
    printf '\033[1;31mREFUSING to unload -- these still hold vino open:\033[0m\n%s\n' "$HOLDERS" >&2
    die "unloading now would free the fops under them and hang the machine"
  fi

  # The fd holders going away is necessary but not sufficient: the module refcount trails it by a
  # moment (the DRM device is released asynchronously), so `modprobe -r` straight after loses a
  # race and reports "Module vino is in use".
  for _ in $(seq 1 40); do
    [ "$(cat /sys/module/vino/refcnt 2>/dev/null || echo 0)" = 0 ] && break
    sleep 0.5
  done
  RC=$(cat /sys/module/vino/refcnt 2>/dev/null || echo 0)
  [ "$RC" = 0 ] || die "module refcount stuck at $RC"

  say "unloading module (refcount 0)"
  modprobe -r vino || die "modprobe -r vino failed"
fi

[ "$UNLOAD_ONLY" = 1 ] && { say "left unloaded"; exit 0; }

# The DL7400's authenticated RTC message carries local civil time, including the live DST offset.
# The kernel clock is UTC and intentionally has no timezone database, so derive the current offset
# at each userspace-assisted load.  An explicit parameter still wins for protocol experiments.
HAVE_RTC_OFFSET=0
for arg in "${MODULE_ARGS[@]}"; do
  case "$arg" in rtc_utc_offset_minutes=*) HAVE_RTC_OFFSET=1;; esac
done
if [ "$HAVE_RTC_OFFSET" = 0 ]; then
  TZ_NUM=$(date +%z)
  case "$TZ_NUM" in
    +[0-9][0-9][0-9][0-9]|-[0-9][0-9][0-9][0-9]) ;;
    *) die "date returned invalid UTC offset '$TZ_NUM'";;
  esac
  TZ_SIGN=1
  [ "${TZ_NUM:0:1}" = "-" ] && TZ_SIGN=-1
  TZ_HOURS=$((10#${TZ_NUM:1:2}))
  TZ_MINS=$((10#${TZ_NUM:3:2}))
  MODULE_ARGS+=("rtc_utc_offset_minutes=$((TZ_SIGN * (TZ_HOURS * 60 + TZ_MINS)))")
fi

say "loading module${MODULE_ARGS[*]:+ (${MODULE_ARGS[*]})}"
# Any remaining arguments are module parameters, so an experiment behind a param can be cycled
# without editing this script.
modprobe vino "${MODULE_ARGS[@]}" || die "modprobe vino failed"

# The interfaces were unbound, so nothing re-probes them by itself. Re-attach by asking the driver
# core to reconsider the device.
sleep 1
for i in $(ls /sys/bus/usb/devices/ | grep -E '^[0-9]+-[0-9.]+:'); do
  v=$(cat "/sys/bus/usb/devices/$i/../idVendor" 2>/dev/null || true)
  p=$(cat "/sys/bus/usb/devices/$i/../idProduct" 2>/dev/null || true)
  [ "$v" = "17e9" ] || continue
  # Vino supports both the Ridge D6000 and the Navarro DL7400.  Keep the rebind list in
  # lockstep with the driver's USB ID table; otherwise a safe cycle silently leaves the DL7400
  # unbound after unloading the old module.
  case "$p" in
    6006|7000) ;;
    *) continue ;;
  esac
  [ -e "/sys/bus/usb/devices/$i/driver" ] && continue
  echo "$i" > "$DRV/bind" 2>/dev/null && say "bound $i"
done

sleep 10
say "loaded: $(sha256sum "$(modinfo -n vino)" | cut -c1-16)"
say "DRM nodes: $(vino_nodes | tr '\n' ' ')"
