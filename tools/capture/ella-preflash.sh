#!/bin/bash
# Phase 0 for an Ella-family dock: everything that can only be learned BEFORE DLM flashes it.
#
#   sudo tools/capture/ella-preflash.sh <outdir> [wait-seconds]
#
# Waits for a 17e9 device to appear, records what it is, then runs the archived 2014 DL3
# implementation against it with the wire recorded. Nothing here can trigger a firmware update:
# DLM must be masked and vino unloaded, and both are checked rather than assumed.
set -uo pipefail

OUT="${1:?usage: sudo tools/capture/ella-preflash.sh <outdir> [wait-seconds]}"
WAIT="${2:-900}"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DL3DIR="$HERE/../../../dl3dev"
VID=17e9

say()  { printf '\033[1;36m==\033[0m %s\n' "$*"; }
big()  { printf '\n\033[1;35m>>> %s\033[0m\n\n' "$*"; }
warn() { printf '\033[1;33m!!\033[0m %s\n' "$*"; }
die()  { printf '\033[1;31mABORT:\033[0m %s\n' "$*" >&2; exit 1; }

[ "$(id -u)" = 0 ] || die "run with sudo"
mkdir -p "$OUT" || die "cannot create $OUT"
OUT="$(cd "$OUT" && pwd)"

# ---- the interlocks, measured ---------------------------------------------------------
say "checking nothing here can flash the dock"
[ "$(systemctl is-enabled displaylink-driver.service 2>&1)" = masked ] \
  || die "displaylink-driver.service is not masked"
pgrep -f '[D]isplayLinkManager' >/dev/null && die "DisplayLinkManager is running -- kill it first"
lsmod | grep -q '^vino ' && die "vino is loaded; sudo modprobe -r vino"
ls /lib/firmware/vino/*-release.spkg >/dev/null 2>&1 \
  && die "packaged firmware still installed -- vino would flash on probe if it loaded"
[ -e /dev/usbmon0 ] || die "no /dev/usbmon* -- sudo modprobe usbmon"
say "DLM masked, vino unloaded, no packaged images: a flash is impossible in this phase"

# ---- wait for the dock ----------------------------------------------------------------
find_dev() {
  for d in /sys/bus/usb/devices/*/; do
    [ "$(cat "$d/idVendor" 2>/dev/null)" = "$VID" ] || continue
    echo "$d"; return 0
  done
  return 1
}
big "PLUG THE HP 3005pr IN NOW (no monitors needed yet)"
say "waiting up to ${WAIT}s for a $VID device"
DEV=""
for _ in $(seq 1 "$WAIT"); do
  DEV="$(find_dev)" && break
  sleep 1
done
[ -n "$DEV" ] || die "no $VID device appeared within ${WAIT}s"
sleep 3          # let every interface finish enumerating before reading descriptors

PID=$(cat "$DEV/idProduct"); BUS=$(cat "$DEV/busnum"); BCD=$(cat "$DEV/bcdDevice")
say "found $VID:$PID  bcdDevice=$BCD  on bus $BUS  ($(cat "$DEV/product" 2>/dev/null))"
echo "$VID:$PID bus=$BUS bcdDevice=$BCD" > "$OUT/device.txt"

# ---- what it is -----------------------------------------------------------------------
big "1/3  identity, endpoints and the DFU descriptor"
python3 "$HERE/dl-identity.py" --json "$OUT/before-identity.json" 2>&1 | tee "$OUT/before-identity.txt"
lsusb -v -d "$VID:" > "$OUT/before-lsusb.txt" 2>/dev/null
lsusb -t            > "$OUT/before-lsusb-tree.txt" 2>/dev/null
for d in /sys/bus/usb/devices/*/; do
  [ "$(cat "$d/idVendor" 2>/dev/null)" = "$VID" ] || continue
  echo "$(basename "$d") pid=$(cat "$d/idProduct") bcdDevice=$(cat "$d/bcdDevice")"
done > "$OUT/before-ids.txt"

# ---- the archived implementation, with the wire recorded ------------------------------
big "2/3  the 2014 DL3 implementation, against pre-flash firmware"
if [ ! -x "$DL3DIR/dl3" ]; then
  warn "no dl3 binary at $DL3DIR/dl3 -- skipping the oracle"
else
  dumpcap -i "usbmon$BUS" -s 0 -w "$OUT/dl3-preflash.pcapng" >"$OUT/dumpcap.log" 2>&1 &
  DC=$!
  sleep 2
  kill -0 $DC 2>/dev/null || { cat "$OUT/dumpcap.log"; warn "dumpcap did not start"; DC=""; }
  [ -n "$DC" ] && say "recording usbmon$BUS -> dl3-preflash.pcapng"

  # dl3 loops on bulk reads and has no exit condition of its own; bound it.
  ( cd "$DL3DIR" && timeout 180 ./dl3 "$PID" 2>"$OUT/dl3-libusb.log" ) | tee "$OUT/dl3.txt"
  RC=${PIPESTATUS[0]}
  say "dl3 exited rc=$RC$([ "$RC" = 124 ] && echo ' (timeout -- it does not self-terminate)')"

  [ -n "$DC" ] && { sleep 2; kill -TERM $DC 2>/dev/null; wait $DC 2>/dev/null; }
fi

# ---- verdict --------------------------------------------------------------------------
big "3/3  what the pre-flash session says"
{
  echo "=== device ==="; cat "$OUT/device.txt"
  echo
  echo "=== identity blob and platform ==="
  grep -iE 'identity|platform|Ella|Ridge|Nava|family' "$OUT/before-identity.txt" 2>/dev/null | head -20
  echo
  echo "=== interface protocol (03 = DL3 and vino's problem, 00 = udl's) ==="
  grep -E 'bInterfaceClass|bInterfaceSubClass|bInterfaceProtocol' "$OUT/before-lsusb.txt" 2>/dev/null \
    | paste - - - | head -8
  echo
  echo "=== endpoints ==="
  grep -E 'bEndpointAddress|Transfer Type' "$OUT/before-lsusb.txt" 2>/dev/null | paste - - | head -12
  echo
  echo "=== DFU functional descriptor ==="
  grep -A6 -i 'Device Firmware Upgrade' "$OUT/before-lsusb.txt" 2>/dev/null | head -12
  echo
  echo "=== dl3: did the AKE verify? ==="
  grep -iE 'H value|L value|matched|AKE|cert|Claiming|Failed' "$OUT/dl3.txt" 2>/dev/null | head -30
  echo
  echo "=== dl3: bytes moved ==="
  grep -cE '^tx:' "$OUT/dl3.txt" 2>/dev/null | sed 's/^/tx frames: /'
  grep -cE '^rx:' "$OUT/dl3.txt" 2>/dev/null | sed 's/^/rx frames: /'
} | tee "$OUT/SUMMARY.txt"

echo
say "pre-flash session saved: $OUT"; du -sh "$OUT"
say "This can never be retaken once DLM has seen the dock."
say "NEXT:  sudo tools/capture/capture-firstcontact.sh ~/dlcap-ella-firstcontact 25"
