#!/bin/bash
# FIRST CONTACT: capture the firmware flash DLM performs the first time it meets a device.
#
#   sudo tools/capture/capture-firstcontact.sh <outdir> [minutes]
#     --no-frida     wire only (see the risk note below)
#     --no-xhci      skip the xHCI tracepoint stream
#     --no-prescan   do not pre-enumerate the device to learn its bus first
#
# Run tools/capture/preflight-newdevice.sh the night before. This assumes it passed.
#
# ORDER OF OPERATIONS, AND WHY
#
# DLM ships a per-platform firmware image and a compatibility enforcer, and flashes when the
# device's build does not match. That happens ONCE, on first contact. There is no second take at
# the real event, so the whole script is arranged around "nothing touches the device until every
# recorder is proven to be running".
#
#   1. PRESCAN. The device is plugged in once with DLM MASKED, purely to read descriptors, and then
#      unplugged. Plain USB enumeration cannot flash anything, and it buys the two things that are
#      otherwise guesses: the BUS NUMBER (a wrong bus is the most common empty capture there is)
#      and a before-state to diff against. Skip with --no-prescan if you would rather not plug it
#      in twice.
#   2. RECORDERS, all of them, verified writing bytes:
#        * dumpcap on the device's bus            -- primary
#        * dumpcap on usbmon0 (every bus)         -- a DFU re-enumerates, possibly under a
#                                                    different PID and possibly onto another bus
#        * fw-watch.py reading mon_bin directly   -- independent code path, and the live "is the
#                                                    flash happening" meter
#        * xHCI tracepoints                       -- port resets, slot teardown and per-TRB
#                                                    completion codes, which usbmon cannot show
#        * dmesg -w                               -- re-enumeration, in kernel words
#      No snaplen anywhere: a truncated payload makes the image unreconstructable. No device or
#      PID filter, for the re-enumeration reason above.
#   3. DLM STARTS WITH NO DEVICE PRESENT, and frida attaches while it is idle. This ordering is
#      what makes keys safe to take: the documented hazard is that frida can stall DLM into a
#      watchdog restart, and a watchdog restart in the middle of a firmware write is how these get
#      bricked. Attaching before the device exists means the risky moment happens when there is
#      nothing to corrupt, and by the time the dock appears the session is settled. Never --spawn.
#   4. ONLY THEN is the device plugged in, and the flash window is fully inside every recorder.
#
# Sleep, idle and lid are inhibited throughout.
set -uo pipefail

USE_FRIDA=1; USE_XHCI=1; PRESCAN=1
while [ $# -gt 0 ]; do
  case "$1" in
    --no-frida)   USE_FRIDA=0; shift ;;
    --no-xhci)    USE_XHCI=0; shift ;;
    --no-prescan) PRESCAN=0; shift ;;
    *) break ;;
  esac
done
OUT="${1:?usage: sudo tools/capture/capture-firstcontact.sh [--no-frida|--no-xhci|--no-prescan] <outdir> [minutes]}"
MINUTES="${2:-20}"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
VID=17e9
SPKG_DIR=/opt/displaylink
TR=/sys/kernel/tracing
MIN_SECS=$((5 * 60))    # a ~1.7 MB image takes minutes; do not conclude "no flash" before this

say()  { printf '\033[1;36m==\033[0m %s\n' "$*"; }
warn() { printf '\033[1;33m!!\033[0m %s\n' "$*"; }
big()  { printf '\n\033[1;32m>>> %s\033[0m\n\n' "$*"; }
die()  { printf '\033[1;31mABORT:\033[0m %s\n' "$*" >&2; exit 1; }

[ "$(id -u)" = 0 ] || die "run with sudo"
mkdir -p "$OUT" || die "cannot create $OUT"
OUT="$(cd "$OUT" && pwd)"
exec > >(tee -a "$OUT/run.log") 2>&1
say "output directory: $OUT"

count_dev() { local n=0 d
  for d in /sys/bus/usb/devices/*/; do
    [ "$(cat "$d/idVendor" 2>/dev/null)" = "$VID" ] && n=$((n+1)); done; echo "$n"; }
snap_ids() {
  for d in /sys/bus/usb/devices/*/; do
    [ "$(cat "$d/idVendor" 2>/dev/null)" = "$VID" ] || continue
    echo "$(basename "$d") pid=$(cat "$d/idProduct" 2>/dev/null) bcdDevice=$(cat "$d/bcdDevice" 2>/dev/null) $(cat "$d/product" 2>/dev/null)"
  done
}

# ============================================================ 0. preconditions
modprobe usbmon 2>/dev/null
[ -e /dev/usbmon0 ] || die "no /dev/usbmon* -- sudo modprobe usbmon (CONFIG_USB_MON)"
command -v dumpcap >/dev/null || die "dumpcap not found"

if systemctl is-active --quiet displaylink-driver.service; then
  warn "displaylink-driver.service is RUNNING."
  warn "If the new device has EVER been plugged in with it running, the flash has already happened."
  read -rp "   stop and mask it, and continue? [y/N] " a
  [ "$a" = y ] || die "stopped at your request"
  systemctl stop displaylink-driver.service
fi
systemctl mask displaylink-driver.service >/dev/null 2>&1
say "displaylink-driver.service masked -- DLM cannot see the device until we say so"

# vino binds DisplayLink hardware by INTERFACE now, with the product id wildcarded, so it claims a
# dock nobody has driven -- and on the DFU interface its probe writes the packaged firmware to any
# dock reporting an older version. Either outcome spends the first contact: a race with DLM for
# interface 0, or a flash that never reaches this capture at all.
if lsmod | grep -q '^vino '; then
  warn "vino is LOADED and binds every 17e9 DL3 interface (product id wildcarded)."
  warn "It will race DLM for the new dock, and its DFU probe may flash it before DLM ever sees it."
  if modprobe -r vino 2>/dev/null; then
    say "unloaded vino"
  else
    warn "could not unload vino -- it is still bound to something. Unplug the known dock first."
    read -rp "   continue anyway? [y/N] " a
    [ "$a" = y ] || die "stopped: unplug the other dock, then 'sudo modprobe -r vino' and re-run"
  fi
fi
if ls /lib/firmware/vino/*-release.spkg >/dev/null 2>&1; then
  warn "packaged firmware is installed at /lib/firmware/vino/. If vino is loaded again during this"
  warn "run it will flash the dock on probe. Hold the images back first:"
  warn "   sudo mkdir -p /lib/firmware/vino/held-back"
  warn "   sudo mv /lib/firmware/vino/*-release.spkg /lib/firmware/vino/held-back/"
  read -rp "   continue anyway? [y/N] " a
  [ "$a" = y ] || die "stopped at your request"
fi

AVAIL=$(df -BG --output=avail "$OUT" | tail -1 | tr -dc '0-9')
[ "${AVAIL:-0}" -ge 10 ] || die "only ${AVAIL}G free at $OUT"
say "${AVAIL}G free"

# dumpcap DROPS PRIVILEGES -- it carries cap_dac_read_search (search, not write) rather than running
# as root -- so "sudo can write here" does not imply "dumpcap can write here". Under a restrictive
# parent directory it opens the interface happily and then fails on the output file. Prove it can
# write into THIS directory now, while the device is not even plugged in, rather than discovering it
# in the one window that cannot be repeated.
if ! timeout 6 dumpcap -i usbmon0 -s 0 -a duration:1 -w "$OUT/.writeprobe.pcapng" >"$OUT/.writeprobe.log" 2>&1 \
   || [ ! -s "$OUT/.writeprobe.pcapng" ]; then
  sed 's/^/     /' "$OUT/.writeprobe.log" 2>/dev/null
  die "dumpcap cannot write into $OUT (it drops privileges; a 0700 parent defeats it).
     Choose an output directory under your home, e.g. ~/dlcap-firstcontact, and re-run."
fi
rm -f "$OUT/.writeprobe.pcapng" "$OUT/.writeprobe.log"
say "dumpcap can write into $OUT (probed, not assumed)"

if [ "$(count_dev)" -gt 0 ]; then
  warn "a $VID device is ALREADY attached:"
  snap_ids | sed 's/^/     /'
  warn "for the cleanest capture, unplug everything DisplayLink except the new dock."
  warn "in particular unplug the D6000: it is already up to date, and its traffic is noise here."
  # Without a terminal there is nobody to answer, and defaulting to "no" turns an advisory into an
  # abort. Wait for the device to be removed instead, which is what the answer would have asked for.
  if [ -t 0 ]; then
    read -rp "   continue anyway? [y/N] " a
    [ "$a" = y ] || die "unplug and re-run"
  else
    warn "no terminal to ask: waiting up to 300s for the device to be unplugged"
    for _ in $(seq 1 300); do [ "$(count_dev)" -eq 0 ] && break; sleep 1; done
    [ "$(count_dev)" -eq 0 ] && say "device removed; continuing" \
      || die "device still attached after 300s -- unplug it and re-run"
  fi
fi


sha256sum "$SPKG_DIR"/DisplayLinkManager > "$OUT/dlm-sha256.txt" 2>/dev/null
sha256sum "$SPKG_DIR"/*.spkg            > "$OUT/spkg-sha256.txt" 2>/dev/null
cp "$SPKG_DIR"/*-release.spkg "$OUT/" 2>/dev/null
say "the four shipped images, copied next to the capture so wire bytes can be attributed:"
for f in "$SPKG_DIR"/*-release.spkg; do
  printf '   %-34s %9s B  %s %s\n' "$(basename "$f")" "$(stat -c%s "$f")" \
    "$(strings -n 6 "$f" | grep -E '^[0-9a-f]{8}$' | head -1)" \
    "$(strings -n 6 "$f" | grep -E '^20[0-9]{2}-' | head -1)"
done

# ============================================================ 1. prescan (DLM still masked)
BUSES=""
if [ "$PRESCAN" = 1 ]; then
  big "PRESCAN -- PLUG THE DOCK IN NOW, WITH NO MONITORS ATTACHED."
  cat <<'EOF'
  DLM is masked, so this cannot flash anything. It only reads descriptors, and it is how we learn
  which bus to record and what the firmware revision was BEFORE DLM ever saw the device.

  No monitors, for the whole run: no video traffic means the firmware image is the only large
  transfer on the wire, the capture stays in the tens of MB, and nothing distracts DLM from the
  enforcer path. Ethernet and USB peripherals can stay unplugged too. Power/PD is fine.
EOF
  for _ in $(seq 1 600); do [ "$(count_dev)" -gt 0 ] && break; sleep 1; done
  [ "$(count_dev)" -gt 0 ] || die "no $VID:* device appeared. Is it plugged in and powered?"
  sleep 2
  snap_ids > "$OUT/before-ids.txt"
  say "device present:"; sed 's/^/   /' "$OUT/before-ids.txt"
  lsusb -v -d "$VID:" > "$OUT/before-lsusb.txt" 2>/dev/null
  lsusb -t            > "$OUT/before-lsusb-tree.txt" 2>/dev/null
  lsusb               > "$OUT/before-lsusb-all.txt" 2>/dev/null
  if [ -f "$HERE/dl-identity.py" ]; then
    python3 "$HERE/dl-identity.py" --json "$OUT/before-identity.json" | tee "$OUT/before-identity.txt"
  fi
  dmesg > "$OUT/before-dmesg.txt" 2>&1
  BUSES=$(for d in /sys/bus/usb/devices/*/; do
            [ "$(cat "$d/idVendor" 2>/dev/null)" = "$VID" ] || continue
            cat "$d/busnum" 2>/dev/null; done | sort -un | tr '\n' ' ')
  say "device bus(es): $BUSES"

  big "NOW UNPLUG THE DOCK AGAIN."
  echo "  DLM has to meet it for the first time INSIDE the capture. Unplug it and wait."
  for _ in $(seq 1 300); do [ "$(count_dev)" -eq 0 ] && break; sleep 1; done
  [ "$(count_dev)" -eq 0 ] && say "dock removed" || warn "still present; continuing anyway"
else
  # A before-state taken by an earlier phase is still a before-state: only create the file when
  # there is not one already, so --no-prescan can be used after a separate descriptor pass rather
  # than costing the run its firmware diff.
  if [ -s "$OUT/before-ids.txt" ]; then
    say "no prescan, but a before-state is already present:"
    sed 's/^/   /' "$OUT/before-ids.txt"
  else
    warn "no prescan: there will be no before-state to diff, so a flash cannot be proven from ids"
    : > "$OUT/before-ids.txt"
  fi
  # The device is not attached now, so its bus cannot be read from sysfs. FC_BUSES lets a caller
  # that already knows it add the per-bus recorder next to the all-bus one.
  BUSES="${FC_BUSES:-}"
  [ -n "$BUSES" ] && say "bus(es) from FC_BUSES: $BUSES"
  dmesg > "$OUT/before-dmesg.txt" 2>&1
fi

# ============================================================ 2. recorders
PIDS=()
for b in $BUSES; do
  [ -e "/dev/usbmon$b" ] || { warn "/dev/usbmon$b missing, skipping"; continue; }
  setsid dumpcap -i "usbmon$b" -s 0 -w "$OUT/wire-bus$b.pcapng" >"$OUT/dumpcap-bus$b.log" 2>&1 &
  PIDS+=($!); say "dumpcap usbmon$b -> wire-bus$b.pcapng (pid ${PIDS[-1]})"
done
setsid dumpcap -i usbmon0 -s 0 -w "$OUT/wire-allbus.pcapng" >"$OUT/dumpcap-allbus.log" 2>&1 &
PIDS+=($!); say "dumpcap usbmon0 (all buses; survives a re-enumeration onto another bus)"

sleep 3
for p in "${PIDS[@]}"; do
  kill -0 "$p" 2>/dev/null || { cat "$OUT"/dumpcap-*.log; die "a dumpcap died on startup -- nothing has been flashed yet, fix it and re-run"; }
done
say "all dumpcaps alive"

XPID=""
if [ "$USE_XHCI" = 1 ] && [ -d "$TR/events/xhci-hcd" ]; then
  # usbmon taps URB submit/giveback; it never sees a port reset, a slot teardown, or a per-TRB
  # completion code. A DFU re-enumerates, so those are exactly the events that describe the shape
  # of the flash. Streamed through trace_pipe rather than left in the ring, because the ring here
  # is 16 KB per CPU and would silently drop the interesting part.
  echo 0 > "$TR/tracing_on"
  echo > "$TR/trace"
  echo nop > "$TR/current_tracer" 2>/dev/null
  echo 65536 > "$TR/buffer_size_kb" 2>/dev/null || warn "could not grow the trace buffer"
  for e in xhci-hcd/xhci_handle_port_status xhci-hcd/xhci_setup_device xhci-hcd/xhci_alloc_dev \
           xhci-hcd/xhci_free_dev xhci-hcd/xhci_discover_or_reset_device \
           xhci-hcd/xhci_handle_cmd_reset_dev xhci-hcd/xhci_handle_cmd_disable_slot \
           xhci-hcd/xhci_stop_device xhci-hcd/xhci_handle_transfer xhci-hcd/xhci_urb_enqueue \
           xhci-hcd/xhci_urb_giveback xhci-hcd/xhci_ring_expansion; do
    [ -e "$TR/events/$e/enable" ] && echo 1 > "$TR/events/$e/enable"
  done
  echo 1 > "$TR/tracing_on"
  setsid sh -c "cat $TR/trace_pipe > '$OUT/xhci-trace.txt'" >/dev/null 2>&1 </dev/null &
  XPID=$!
  say "xHCI tracepoints streaming -> xhci-trace.txt (pid $XPID)"
elif [ "$USE_XHCI" = 1 ]; then
  warn "no $TR/events/xhci-hcd -- xHCI tracing unavailable on this kernel, continuing without it"
fi

setsid sh -c "dmesg -w > '$OUT/dmesg-live.txt'" >/dev/null 2>&1 </dev/null & DPID=$!
say "dmesg -w streaming -> dmesg-live.txt"

systemd-inhibit --what=sleep:idle:handle-lid-switch --who=vino \
  --why="DisplayLink firmware capture -- interrupting a flash bricks the device" \
  --mode=block sleep infinity >/dev/null 2>&1 </dev/null &
INHIBIT=$!
say "sleep/idle/lid inhibited"

modprobe evdi 2>/dev/null && say "evdi loaded (DLM does nothing without it)" \
  || warn "could not load evdi -- DLM may refuse to start"

# ============================================================ 3. DLM, then frida, with NO device
big "STARTING DLM WITH NO DEVICE ATTACHED"
# Run DLM BY HAND, with the unit left masked.
#
# /lib/udev/rules.d/99-displaylink.rules runs /opt/displaylink/udev.sh on every add/remove of a
# 17e9 DL3 interface, and that bounces displaylink-driver.service. Under systemd the dock plug --
# the one event this whole capture exists for -- therefore RESTARTS DLM a few seconds later, and
# the frida hook bound to the old pid records nothing for the new session while still looking
# healthy. That is exactly what happened on 2026-08-01: the flash was caught, keys.log came back
# EMPTY, and the journal shows Stopping/Started 42 s after the attach.
systemctl mask displaylink-driver.service >/dev/null 2>&1
systemctl stop displaylink-driver.service >/dev/null 2>&1
pkill -f '[D]isplayLinkManager' 2>/dev/null; sleep 1
( cd /opt/displaylink && exec ./DisplayLinkManager ) > "$OUT/dlm.stdout.log" 2>&1 &
DLMPID=$!
date +%s.%N > "$OUT/dlm-start-epoch.txt"
for _ in $(seq 1 30); do
  kill -0 $DLMPID 2>/dev/null && pgrep -f '[D]isplayLinkManager' >/dev/null && break
  sleep 0.5
done
if pgrep -f '[D]isplayLinkManager' >/dev/null; then
  say "DLM running by hand (pid $DLMPID), unit masked so udev cannot restart it"
else
  cat "$OUT/dlm.stdout.log"
  warn "DisplayLinkManager did not start -- see $OUT/dlm.stdout.log"
fi
sleep 5

FRIDA=""
if [ "$USE_FRIDA" = 1 ]; then
  FRIDA_PP="${FRIDA_PP:-}"
  if [ -z "$FRIDA_PP" ]; then
    for d in /home/*/.local/lib/python3*/site-packages; do [ -d "$d/frida" ] && FRIDA_PP="$d" && break; done
  fi
  if [ -n "$FRIDA_PP" ] && pgrep -f '[D]isplayLinkManager' >/dev/null; then
    H=$(cut -d' ' -f1 < "$OUT/dlm-sha256.txt" 2>/dev/null)
    [ "$H" = "d3584c4369a594e9bcac20b71150086559d171c40d4949c67ee6affb3f96bfdb" ] \
      || warn "DLM is not the 6.8.1.0 build the AES offset was derived for -- keys may come back EMPTY"
    env PYTHONPATH="$FRIDA_PP" python3 "$HERE/decode-modeset-live.py" \
        --secs "$((MINUTES * 60 + 120))" --out "$OUT/keys-raw.json" > "$OUT/keys.log" 2>&1 &
    FRIDA=$!
    sleep 5
    if kill -0 "$FRIDA" 2>/dev/null; then
      say "frida attached to the IDLE DLM (pid $FRIDA) -- the risky moment is now behind us"
    else
      warn "frida exited immediately; see $OUT/keys.log. Continuing WIRE-ONLY."
      warn "that is survivable: the .spkg payload key is dock-side, so DLM pushes the container"
      warn "opaquely and the image should be recognisable on the wire without any key."
      FRIDA=""
    fi
  else
    warn "frida unavailable or DLM not running -- WIRE ONLY"
  fi
else
  say "frida disabled by --no-frida (wire only)"
fi

# ============================================================ 4. first contact
big "PLUG THE DOCK IN NOW. NO MONITORS. THEN DO NOT TOUCH ANYTHING."
cat <<'EOF'
  From this point:
    * DO NOT unplug the dock, the power, or anything else on that bus.
    * DO NOT suspend or close the lid.
    * DO NOT Ctrl-C twice.
  Interrupting a firmware write is how these get bricked. If a flash starts, the watcher below
  says so in magenta and keeps reporting how much of the image it has seen. Long quiet stretches
  are normal: the enforcer verifies, flashes, resets the device, and verifies again.

  fw-watch.py is now in the foreground. YOU DO NOT HAVE TO WATCH THE CLOCK: once the image is
  manifested it counts down 45 s of DFU silence and stops by itself, printing "the flash is
  COMPLETE". A second image restarts that wait. Press Ctrl-C ONCE only if you want to stop
  early; it exits cleanly, and this script refuses to finish while the wire is still busy.
EOF
sleep 2

INTR=0
trap 'INTR=1' INT
START=$(date +%s)
python3 "$HERE/fw-watch.py" --bus 0 --out "$OUT/fw.mon" --spkg-dir "$SPKG_DIR" \
        --secs "$((MINUTES * 60))" --events "$OUT/flash-events.txt"
ELAPSED=$(( $(date +%s) - START ))
trap - INT

# ============================================================ 5. never stop mid-transfer
busy_check() {
  local f a b
  f="$(ls -S "$OUT"/wire-*.pcapng 2>/dev/null | head -1)"
  [ -n "$f" ] || return 1
  a=$(stat -c%s "$f"); sleep 4; b=$(stat -c%s "$f")
  [ $((b - a)) -gt 200000 ]
}
# Bounded: a dock that has come up with a panel attached moves this much continuously, and an
# unbounded loop would then never let the script finish.
BUSY_WAITS=0
while busy_check; do
  BUSY_WAITS=$((BUSY_WAITS + 1))
  warn "the wire is still MOVING (>200 KB in 4 s). Not stopping -- this could be the flash. [$BUSY_WAITS/45]"
  if [ "$BUSY_WAITS" -ge 45 ]; then
    warn "three minutes of continuous traffic with no DFU activity: that is video, not a flash."
    warn "proceeding to stop. If a flash IS running, Ctrl-C now and let it finish."
    sleep 5
    break
  fi
done
if [ "$ELAPSED" -lt "$MIN_SECS" ] && [ ! -s "$OUT/flash-events.txt" ]; then
  warn "stopped after ${ELAPSED}s with no firmware detected, and a flash can take minutes."
  read -rp "   really stop? [y/N] " a
  if [ "$a" != y ]; then
    trap 'INTR=1' INT
    python3 "$HERE/fw-watch.py" --bus 0 --out "$OUT/fw2.mon" --spkg-dir "$SPKG_DIR" \
            --secs "$((MINUTES * 60))" --events "$OUT/flash-events.txt"
    trap - INT
  fi
fi

# ============================================================ 6. stop and snapshot
say "stopping recorders"
# The key extractor was started with --secs covering the whole window, so a bare `wait` blocks
# until that timer expires even though the capture is finished -- minutes of apparently hung
# script after Ctrl-C. Ask it to stop, give it a few seconds to write keys-raw.json, then insist.
if [ -n "$FRIDA" ]; then
  kill -INT "$FRIDA" 2>/dev/null
  for _ in $(seq 1 20); do kill -0 "$FRIDA" 2>/dev/null || break; sleep 0.5; done
  if kill -0 "$FRIDA" 2>/dev/null; then
    warn "key extractor did not exit on SIGINT after 10 s; terminating it"
    kill -TERM "$FRIDA" 2>/dev/null; sleep 2
    kill -0 "$FRIDA" 2>/dev/null && kill -KILL "$FRIDA" 2>/dev/null
  fi
  wait "$FRIDA" 2>/dev/null
fi
for p in "${PIDS[@]}"; do kill -TERM "$p" 2>/dev/null; done
sleep 2
for p in "${PIDS[@]}"; do kill -0 "$p" 2>/dev/null && kill -KILL "$p" 2>/dev/null; done
[ -n "$XPID" ] && { kill "$XPID" 2>/dev/null; echo 0 > "$TR/tracing_on" 2>/dev/null; }
kill "$DPID" "$INHIBIT" 2>/dev/null

snap_ids > "$OUT/after-ids.txt"
lsusb -v -d "$VID:" > "$OUT/after-lsusb.txt" 2>/dev/null
lsusb -t            > "$OUT/after-lsusb-tree.txt" 2>/dev/null
[ -f "$HERE/dl-identity.py" ] && python3 "$HERE/dl-identity.py" --json "$OUT/after-identity.json" \
  | tee "$OUT/after-identity.txt"
journalctl -u displaylink-driver.service --since "-$((MINUTES + 10)) min" > "$OUT/dlm.log" 2>&1
dmesg > "$OUT/after-dmesg.txt" 2>&1

if [ -s "$OUT/keys-raw.json" ]; then
  python3 - "$OUT/keys-raw.json" "$OUT/keys.candidates.json" <<'PY' || warn "key conversion failed"
import json, sys
d = json.load(open(sys.argv[1]))
rows = d.get("krs", d if isinstance(d, list) else [])
json.dump(rows, open(sys.argv[2], "w"), indent=1)
print(f"   {len(rows)} (key,riv) candidate(s) -> {sys.argv[2]}")
PY
fi

# ============================================================ 7. verdict, while it is still on the desk
echo
say "=== DID THE FIRMWARE MOVE? (a changed bcdDevice is proof of a flash) ==="
diff "$OUT/before-ids.txt" "$OUT/after-ids.txt" && echo "   bcdDevice unchanged"
if [ -f "$OUT/before-identity.txt" ] && [ -f "$OUT/after-identity.txt" ]; then
  if diff "$OUT/before-identity.txt" "$OUT/after-identity.txt" > "$OUT/identity-diff.txt"; then
    echo "   identity blob unchanged"
  else
    echo "   identity blob CHANGED:"; sed 's/^/     /' "$OUT/identity-diff.txt"
  fi
fi
echo
say "=== re-enumerations the kernel saw ==="
grep -E 'new (high|full|super)-speed USB device|USB disconnect|device descriptor read|reset .*-speed' \
     "$OUT/dmesg-live.txt" 2>/dev/null | head -30
if [ -s "$OUT/xhci-trace.txt" ]; then
  echo
  say "=== xHCI: port status / slot lifecycle (a DFU shows up as a reset + re-address) ==="
  grep -cE 'xhci_handle_port_status' "$OUT/xhci-trace.txt" | sed 's/^/   port-status events: /'
  grep -E 'xhci_alloc_dev|xhci_free_dev|xhci_setup_device|xhci_discover_or_reset_device' \
       "$OUT/xhci-trace.txt" | head -20 | sed 's/^/   /'
fi
if [ -s "$OUT/flash-events.txt" ]; then
  echo; say "=== LIVE FLASH EVENTS ==="; sed 's/^/   /' "$OUT/flash-events.txt"
fi
echo
say "=== offline confirmation ==="
[ -f "$HERE/fw-scan.py" ] && python3 "$HERE/fw-scan.py" "$OUT" | tee "$OUT/fw-scan.txt" \
  || warn "fw-scan.py missing; run it by hand"

echo
say "capture complete: $OUT"; du -sh "$OUT"

# Keys are bound to the pid frida attached to. If DLM was replaced mid-run, everything sealed in
# this capture is unreadable and it is much cheaper to learn that now than during analysis.
echo
NOWPID=$(pgrep -f '[D]isplayLinkManager' | head -1)
if [ -n "${DLMPID:-}" ] && [ -n "$NOWPID" ] && [ "$NOWPID" != "$DLMPID" ]; then
  warn "DLM pid changed $DLMPID -> $NOWPID: it was restarted during the run, so any keys belong"
  warn "  to a session the wire does not contain. The WIRE is still good (the .spkg payload key is"
  warn "  dock-side, so the image is recognisable unkeyed), but sealed CP frames will not decrypt."
elif [ -s "$OUT/keys-raw.json" ]; then
  say "DLM pid stable at ${DLMPID:-?} for the whole run -- keys and wire are from one session"
else
  warn "no keys captured (keys-raw.json empty). Wire-only: fine for the flash, not for CP frames."
fi
echo
say "DLM is LEFT RUNNING (by hand, unit masked) for the protocol captures that follow."
say "back to vino work:  sudo pkill -f DisplayLinkManager"
say "NEXT: attach monitors, then"
say "  sudo tools/capture/capture-newdevice.sh  ~/dlcap-keyed     # protocol + feature choreography"
say "  sudo tools/capture/capture-modematrix.sh ~/dlcap-modes     # off42 / off66 / off72"
