#!/bin/bash
# Record DLM driving a multi-connector DisplayLink dock while monitors are MOVED BETWEEN PORTS.
#
# The DL7400 (Navarro) exposes FOUR DisplayPort connectors but only TWO video endpoints, and DLM
# brings up exactly two outputs. Which physical port feeds which stream -- and what crosses the wire
# when a cable moves -- is the thing this capture exists to answer. A port move is the only
# experiment that separates "port" from "head": everything else in the corpus was recorded with two
# cables that never moved, so port and head were confounded in every previous run.
#
#   sudo tools/capture/capture-portmap.sh <outdir> [seconds]
#   tools/capture/portmark.sh <outdir> <label>      # journal a step, from another shell
#   touch <outdir>/STOP                             # finish early
#
# Unlike capture-newdevice.sh this does NOT prompt: the cable moves happen at human pace and are
# journalled from outside, so the run can be driven from a chat session or a second terminal. A
# state watcher also journals every connector/device change on its own, so the wire stays sliceable
# even if nobody marks a step.
#
# Gotchas encoded here, each of which has cost a hardware run:
#   * usbmon is NOT autoloaded.
#   * dumpcap DROPS PRIVILEGES -- it can open the interface and still fail on the output file, so
#     the output lands under $HOME and is proved writable before anything irreversible happens.
#   * frida under sudo cannot see user-site packages -> PYTHONPATH must be passed explicitly.
#   * the frida USB hook DROPS bulk transfers; usbmon owns the bytes, frida supplies only keys.
#   * the key schedule is DORMANT on a warm dock: with a session already up there is no fresh key
#     at all. So the device is de-authorised first and re-authorised only once both recorders are
#     proven live -- that is a cold connect without touching the cable.
#   * vino auto-binds 17e9:7000 and will steal the dock from DLM.
set -uo pipefail

# --no-reauth: do not force a session by de-authorising the dock. The forced session brings the
# control plane up but does NOT light the panels (measured: DLM reads EDID, publishes real modes,
# and never starts the pixel clock, because a USB re-authorise does not reset the dock). When the
# panels are already lit, that is worth preserving -- record a real power-cycle instead.
REAUTH=1
case "${1:-}" in
  --no-reauth) REAUTH=0; shift ;;
esac
OUT="${1:?usage: sudo tools/capture/capture-portmap.sh [--no-reauth] <outdir> [seconds]}"
SECS="${2:-1800}"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
VID=17e9

say()  { printf '\033[1;36m==\033[0m %s\n' "$*"; }
warn() { printf '\033[1;33m!!\033[0m %s\n' "$*"; }
die()  { printf '\033[1;31mABORT:\033[0m %s\n' "$*" >&2; exit 1; }
[ "$(id -u)" = 0 ] || die "run with sudo"

mkdir -p "$OUT" || die "cannot create $OUT"
OUT="$(cd "$OUT" && pwd)"
chmod 0777 "$OUT"                 # dumpcap runs unprivileged after dropping caps
JOURNAL="$OUT/journal.tsv"
: > "$JOURNAL"; chmod 0666 "$JOURNAL"
echo "$OUT" > "$OUT/.outdir"

mark() { printf '%s\t%s\n' "$(date +%s.%N)" "$1" >> "$JOURNAL"; }

# ---- preconditions --------------------------------------------------------------------
lsmod | grep -q '^vino ' && die "vino is loaded and will steal the dock -- tools/hardware/vino-cycle.sh --unload"
modprobe usbmon 2>/dev/null
[ -e /dev/usbmon0 ] || die "no /dev/usbmon0 after modprobe usbmon"
command -v dumpcap >/dev/null || die "dumpcap not found (install wireshark-cli)"

# Prove dumpcap can write HERE, now, rather than at the worst possible moment.
if ! timeout 6 dumpcap -i usbmon0 -s 0 -a duration:1 -w "$OUT/.probe.pcapng" >/dev/null 2>&1 \
   || [ ! -s "$OUT/.probe.pcapng" ]; then
  die "dumpcap cannot capture into $OUT -- it drops privileges, so pick a path under \$HOME"
fi
rm -f "$OUT/.probe.pcapng"
say "dumpcap proven able to write into $OUT"

FRIDA_PP="${FRIDA_PP:-}"
if [ -z "$FRIDA_PP" ]; then
  for d in /home/*/.local/lib/python3*/site-packages; do
    [ -d "$d/frida" ] && FRIDA_PP="$d" && break
  done
fi
[ -n "$FRIDA_PP" ] || warn "frida not found -- WIRE ONLY, sealed traffic will be unreadable"

# ---- locate the dock ------------------------------------------------------------------
DEV=""
for d in /sys/bus/usb/devices/*/; do
  [ "$(cat "$d/idVendor" 2>/dev/null)" = "$VID" ] || continue
  DEV="${d%/}"
done
[ -n "$DEV" ] || die "no $VID device attached"
PID=$(cat "$DEV/idProduct"); BUS=$(cat "$DEV/busnum")
say "dock $VID:$PID at $(basename "$DEV") on bus $BUS"
echo "$DEV" > "$OUT/.devpath"

sha256sum /opt/displaylink/DisplayLinkManager > "$OUT/dlm-sha256.txt" 2>/dev/null
lsusb -v -d "$VID:" > "$OUT/before-lsusb.txt" 2>/dev/null
"$HERE/dl-identity.py" > "$OUT/identity.txt" 2>&1 || true

# ---- cold-connect setup: DLM up, dock absent ------------------------------------------
# A warm dock yields no key schedule at all, so the session must start after the recorders do.
if [ "$REAUTH" = 1 ]; then
  say "de-authorising the dock so DLM starts with no device"
  echo 0 > "$DEV/authorized" || die "cannot de-authorise $DEV"
  sleep 2
else
  say "--no-reauth: leaving the dock alone; power-cycle it once recording starts"
fi

# Run DLM BY HAND, with the unit left masked.
#
# /lib/udev/rules.d/99-displaylink.rules runs /opt/displaylink/udev.sh on every add/remove of a
# 17e9 DL3 interface, and that bounces displaylink-driver.service. So under systemd a dock
# power-cycle -- the one event that actually relights the panels -- RESTARTS DLM, and any frida
# hook bound to the old pid then records nothing for the new session while still looking healthy.
# That cost the keys for a whole lit capture. Launched by hand with the unit masked, udev has
# nothing to restart and the pid is stable for the life of the run.
systemctl mask displaylink-driver.service >/dev/null 2>&1
systemctl stop displaylink-driver.service >/dev/null 2>&1
pkill -f '[D]isplayLinkManager' 2>/dev/null; sleep 1

modprobe evdi 2>/dev/null
( cd /opt/displaylink && exec ./DisplayLinkManager ) > "$OUT/dlm.stdout.log" 2>&1 &
DLMPID=$!
for _ in $(seq 1 30); do kill -0 $DLMPID 2>/dev/null && pgrep -f '[D]isplayLinkManager' >/dev/null && break; sleep 0.5; done
pgrep -f '[D]isplayLinkManager' >/dev/null || { cat "$OUT/dlm.stdout.log"; die "DisplayLinkManager did not start"; }
say "DLM running by hand (pid $DLMPID), unit masked so udev cannot restart it"

# ---- recorders ------------------------------------------------------------------------
# usbmon0 = ALL buses: a dock that re-enumerates can land on a different bus, and following one
# device number would lose exactly the moment worth having.
dumpcap -i usbmon0 -s 0 -w "$OUT/wire.pcapng" >"$OUT/dumpcap.log" 2>&1 &
DUMP=$!
sleep 2
kill -0 $DUMP 2>/dev/null || { cat "$OUT/dumpcap.log"; die "dumpcap failed to start"; }
[ -s "$OUT/wire.pcapng" ] || { cat "$OUT/dumpcap.log"; die "dumpcap started but wrote nothing"; }
say "wire capture live on usbmon0 (pid $DUMP, $(stat -c%s "$OUT/wire.pcapng") bytes)"

FRIDA=""
if [ -n "$FRIDA_PP" ]; then
  # --all-keys: Navarro creates FIVE sealing keys per session (one per wire sub), and the
  # control-stream counter-shape filter drops the four video ones.
  env PYTHONPATH="$FRIDA_PP" python3 "$HERE/decode-modeset-live.py" \
      --secs "$SECS" --all-keys --video-eps 0x02,0x08,0x0a,0x0b,0x0c \
      --stop-file "$OUT/STOP" --flush-secs 5 --reattach \
      --out "$OUT/keys-raw.json" > "$OUT/keys.log" 2>&1 &
  FRIDA=$!
  for _ in $(seq 1 40); do grep -qi 'ready\|attach' "$OUT/keys.log" 2>/dev/null && break; sleep 0.25; done
  kill -0 $FRIDA 2>/dev/null || { cat "$OUT/keys.log"; warn "frida died -- continuing WIRE ONLY"; FRIDA=""; }
  [ -n "$FRIDA" ] && say "key extractor attached (pid $FRIDA)"
fi

# ---- state watcher --------------------------------------------------------------------
# Journals every DRM-connector / USB change by itself, at machine resolution, so the wire stays
# sliceable even when a step is marked late or not at all.
snapshot() {
  for c in /sys/class/drm/card*-*/; do
    [ -e "$c/status" ] || continue
    printf '%s=%s/%s ' "$(basename "$c")" "$(cat "$c/status" 2>/dev/null)" \
                       "$(cat "$c/enabled" 2>/dev/null)"
  done
  for d in /sys/bus/usb/devices/*/; do
    [ "$(cat "$d/idVendor" 2>/dev/null)" = "$VID" ] || continue
    printf 'usb:%s=%s@%s ' "$(basename "$d")" "$(cat "$d/idProduct")" "$(cat "$d/devnum")"
  done
}
(
  prev=""
  while [ ! -e "$OUT/STOP" ]; do
    cur="$(snapshot)"
    if [ "$cur" != "$prev" ]; then
      printf '%s\tstate\t%s\n' "$(date +%s.%N)" "$cur" >> "$JOURNAL"
      prev="$cur"
    fi
    sleep 0.4
  done
) & WATCH=$!

journalctl -f --since now > "$OUT/journal-system.log" 2>&1 & DLMLOG=$!
dmesg -w > "$OUT/dmesg.live.txt" 2>&1 & DMESGW=$!

# ---- go -------------------------------------------------------------------------------
mark "begin:cold-connect"
if [ "$REAUTH" = 1 ]; then
  say "re-authorising the dock -- cold session starts NOW"
  echo 1 > "$DEV/authorized"
else
  say "recording. POWER-CYCLE THE DOCK NOW -- that is what actually relights the panels."
fi

cat <<EOF

  \033[1;32mRECORDING\033[0m into $OUT  (up to ${SECS}s)
  journal a step:   tools/capture/portmark.sh $OUT <label>
  finish:           touch $OUT/STOP

EOF

END=$(( $(date +%s) + SECS ))
while [ ! -e "$OUT/STOP" ] && [ "$(date +%s)" -lt "$END" ]; do sleep 1; done
mark "end:capture"

# ---- stop -----------------------------------------------------------------------------
touch "$OUT/STOP"
# Stop the WIRE first. A lit dock streams video at gigabytes per minute, and the key extractor
# spends a long time in its own post-run decrypt sweep before exiting -- so waiting for frida
# before killing dumpcap kept the capture growing long after the run was over (a measured 50 GB).
kill $DUMP $WATCH $DLMLOG $DMESGW 2>/dev/null
wait $DUMP 2>/dev/null
say "wire capture closed at $(du -h "$OUT/wire.pcapng" 2>/dev/null | cut -f1)"

# The extractor stops on the STOP file and writes its keys on the way out; killing it instead is
# how a keyless run happens, so it is waited for, not signalled.
[ -n "$FRIDA" ] && { say "waiting for the key extractor to flush"; wait $FRIDA 2>/dev/null; }
say "capture stopped"

[ -n "${DLMPID:-}" ] && { kill $DLMPID 2>/dev/null; sleep 1; pkill -f '[D]isplayLinkManager' 2>/dev/null; }
say "DLM stopped; unit left masked"

lsusb -v -d "$VID:" > "$OUT/after-lsusb.txt" 2>/dev/null
cp /sys/class/drm/card*-*/status /dev/null 2>/dev/null
{ for c in /sys/class/drm/card*-*/; do
    echo "== $(basename "$c") status=$(cat "$c/status" 2>/dev/null)"
    cat "$c/modes" 2>/dev/null | head -5
  done; } > "$OUT/after-connectors.txt" 2>&1

# decode-modeset-live.py writes {"krs":[...]}; decrypt-dlm-cp.py wants a bare list of {key,riv}.
if [ -s "$OUT/keys-raw.json" ]; then
  python3 - "$OUT/keys-raw.json" "$OUT/keys.candidates.json" <<'PY' || warn "key conversion failed"
import json, sys
d = json.load(open(sys.argv[1]))
rows = d.get("krs", d if isinstance(d, list) else [])
json.dump(rows, open(sys.argv[2], "w"), indent=1)
print(f"   {len(rows)} (key,riv) candidate(s) -> {sys.argv[2]}")
PY
fi

echo
say "results in $OUT"
printf '   wire     : %s\n' "$(du -h "$OUT/wire.pcapng" 2>/dev/null | cut -f1)"
printf '   keys     : %s candidate(s)\n' "$(python3 -c 'import json,sys;print(len(json.load(open(sys.argv[1]))))' "$OUT/keys.candidates.json" 2>/dev/null || echo 0)"
printf '   journal  : %s line(s)\n' "$(wc -l < "$JOURNAL")"
