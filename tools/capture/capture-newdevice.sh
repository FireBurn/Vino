#!/bin/bash
# Capture DLM driving an unfamiliar DisplayLink device: the wire AND the session keys, together.
#
# The init sequence and the HDCP AKE cross the wire in the clear, but everything after
# SKE_SEND_EKS is AES-CTR sealed -- and that is where set-mode (id=0x48 sub=0x22), the real EDID
# (id=0x194) and the setup burst live. A wire-only capture of those is unreadable, so this runs
# usbmon and the frida key extractor over the SAME session.
#
#   sudo tools/capture/capture-newdevice.sh <outdir> [seconds]
#
# Then plug the device in (or let DLM restart pick it up) while it runs.
#
# Gotchas this script exists to encode, every one of which has cost a real hardware run:
#   * usbmon is NOT autoloaded.
#   * frida under sudo cannot see user-site packages -> PYTHONPATH must be passed explicitly.
#   * the frida USB hook DROPS bulk transfers (0 vs usbmon's 249 for the same traffic), so usbmon
#     is the source of truth for bytes and frida supplies keys only.
#   * CP crypto is dormant on a warm dock: with an established session there is no AKE and no fresh
#     key, so the capture must span a real connect.
#   * a DFU re-enumerates, possibly under a different product ID, so the capture must follow the
#     whole BUS, never a single device.
#   * raw frames are persisted BEFORE any decrypt attempt, so a decrypt bug cannot discard the run.
set -uo pipefail

GUIDED=1
case "${1:-}" in
  --unguided) GUIDED=0; shift ;;
  --guided)   GUIDED=1; shift ;;
esac
OUT="${1:?usage: sudo tools/capture/capture-newdevice.sh [--unguided] <outdir> [seconds]}"
SECS="${2:-180}"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
VID=17e9

say()  { printf '\033[1;36m==\033[0m %s\n' "$*"; }
warn() { printf '\033[1;33m!!\033[0m %s\n' "$*"; }
die()  { printf '\033[1;31mABORT:\033[0m %s\n' "$*" >&2; exit 1; }
[ "$(id -u)" = 0 ] || die "run with sudo"
mkdir -p "$OUT" || die "cannot create $OUT"

# ---- frida, and the PYTHONPATH gotcha -------------------------------------------------
# Root's python does not see the user site-packages where frida normally lives.
FRIDA_PP="${FRIDA_PP:-}"
if [ -z "$FRIDA_PP" ]; then
  for d in /home/*/.local/lib/python3*/site-packages; do
    [ -d "$d/frida" ] && FRIDA_PP="$d" && break
  done
fi
if [ -z "$FRIDA_PP" ] || [ ! -d "$FRIDA_PP/frida" ]; then
  warn "frida not found in any user site-packages."
  warn "install it as your normal user:  pip install --user frida pycryptodome"
  warn "or set FRIDA_PP=/path/to/site-packages and re-run."
  warn "continuing with the WIRE capture only -- sealed traffic will not be decryptable."
  FRIDA_PP=""
else
  say "frida site-packages: $FRIDA_PP"
fi

# ---- usbmon, and the not-autoloaded gotcha --------------------------------------------
modprobe usbmon 2>/dev/null
[ -e /dev/usbmon0 ] || die "no /dev/usbmon* after modprobe usbmon -- is CONFIG_USB_MON built?"

command -v dumpcap >/dev/null || die "dumpcap not found (install wireshark-cli / wireshark)"

# ---- locate the device and its BUS ----------------------------------------------------
BUS=""
for d in /sys/bus/usb/devices/*/; do
  [ "$(cat "$d/idVendor" 2>/dev/null)" = "$VID" ] || continue
  BUS="$(cat "$d/busnum" 2>/dev/null)"
  say "found $VID:$(cat "$d/idProduct" 2>/dev/null) at $(basename "$d") on bus $BUS"
done
if [ -z "$BUS" ]; then
  warn "no $VID device attached yet."
  read -rp "   enter the bus number you will plug it into (lsusb -t), or blank for ALL buses: " BUS
fi
IFACE="usbmon${BUS:-0}"
[ -e "/dev/$IFACE" ] || die "/dev/$IFACE does not exist"
say "capturing on $IFACE for ${SECS}s"

# ---- competing drivers ----------------------------------------------------------------
for m in vino udl udlfb; do
  if lsmod | grep -q "^$m "; then
    warn "$m is loaded and will race DLM for the interface."
    warn "blacklist it in /etc/modprobe.d/ and reboot, or this capture may show a half-bound device."
  fi
done
systemctl is-active --quiet displaylink-driver.service \
  || warn "displaylink-driver.service is not active; DLM must be running to capture it."

# ---- "before" state: proves whether a firmware flash happened -------------------------
snap_ids() {
  for d in /sys/bus/usb/devices/*/; do
    [ "$(cat "$d/idVendor" 2>/dev/null)" = "$VID" ] || continue
    echo "$(basename "$d") pid=$(cat "$d/idProduct" 2>/dev/null) bcdDevice=$(cat "$d/bcdDevice" 2>/dev/null)"
  done
}
snap_ids > "$OUT/before-ids.txt"
lsusb -v -d "$VID:" > "$OUT/before-lsusb.txt" 2>/dev/null
sha256sum /opt/displaylink/*.spkg > "$OUT/spkg-sha256.txt" 2>/dev/null
sha256sum /opt/displaylink/DisplayLinkManager > "$OUT/dlm-sha256.txt" 2>/dev/null
# The key hook is build-specific; without this the offsets cannot be re-derived later.
say "recorded DLM build hash -- needed to re-derive the AES offset if the keys come back empty"

# ---- go -------------------------------------------------------------------------------
dumpcap -i "$IFACE" -s 0 -w "$OUT/wire.pcapng" >"$OUT/dumpcap.log" 2>&1 &
DUMP=$!
sleep 1
kill -0 $DUMP 2>/dev/null || { cat "$OUT/dumpcap.log"; die "dumpcap failed to start"; }
say "wire capture running (pid $DUMP)"

FRIDA=""
if [ -n "$FRIDA_PP" ] && pgrep -f '[D]isplayLinkManager' >/dev/null; then
  env PYTHONPATH="$FRIDA_PP" python3 "$HERE/decode-modeset-live.py" \
      --secs "$SECS" --out "$OUT/keys-raw.json" > "$OUT/keys.log" 2>&1 &
  FRIDA=$!
  say "key extractor attached (pid $FRIDA)"
else
  warn "not attaching frida (no DLM running, or frida unavailable) -- WIRE ONLY"
fi

# ---- the choreography ----------------------------------------------------------------
# Each step is journalled with an epoch timestamp so the capture can be sliced by action
# afterwards. decrypt-dlm-cp.py takes --start/--end in exactly these units, so a journal line is
# directly usable: without it, a long capture is an undifferentiated wall of frames and working out
# which bytes belong to which action is guesswork. That guesswork is what made several of these
# behaviours take whole sessions to pin down.
JOURNAL="$OUT/journal.tsv"
: > "$JOURNAL"
mark() { printf '%s\t%s\n' "$(date +%s.%N)" "$1" | tee -a "$JOURNAL" >/dev/null; }

step() { # step <seconds> <label> <instruction...>
  local secs="$1" label="$2"; shift 2
  printf '\n\033[1;32m>>>\033[0m %s\n' "$*"
  mark "begin:$label"
  local i="$secs"
  while [ "$i" -gt 0 ]; do printf '\r    %2ds ' "$i"; sleep 1; i=$((i-1)); done
  printf '\r        \r'
  mark "end:$label"
}

if [ "$GUIDED" = 1 ] && [ -t 0 ]; then
  cat <<'EOF'

  A guided capture follows. Each step is timestamped so the wire can be sliced by action.
  Do each one when prompted; if a step does not apply (no second head, no monitor to
  unplug) just let it run out. Do NOT unplug anything if a firmware flash starts.

EOF
  read -rp "  press enter to begin: " _ || true

  step 15 idle-before      "Leave everything ALONE. Baseline: what the link does with nothing happening."
  step 30 connect          "PLUG THE DOCK IN NOW (or restart DLM). This carries init, the HDCP AKE, EDID and the mode-set."
  step 15 settle           "Leave it alone until the picture settles."
  step 15 cursor-move      "Move the mouse pointer around ON THE DOCK SCREEN, slowly."
  step 15 cursor-shape     "Hover over a TEXT FIELD then a LINK, so the pointer CHANGES SHAPE a few times."
  step 10 cursor-off       "Move the pointer OFF the dock screen entirely and leave it off."
  step 15 window-drag      "DRAG A WINDOW around the dock screen."
  step 20 video            "Play a VIDEO FULLSCREEN on the dock screen."
  step 30 idle-after       "Stop everything. Do not touch the mouse. True idle."
  step 20 mode-change      "Change the dock output's RESOLUTION (address it as WxH@rate, not by index)."
  step 20 dpms             "Blank the screen and wake it (DPMS off, then on)."
  step 20 monitor-unplug   "UNPLUG THE MONITOR from the dock, wait, then PLUG IT BACK IN."
  step 10 dock-unplug      "UNPLUG THE DOCK."
else
  cat <<EOF

  >>> NOW: plug the device in, or restart DLM, so a FRESH session initialises. <<<
      A warm dock has no AKE and yields no keys.
      Capturing for ${SECS}s. Do NOT unplug if a firmware flash starts.

EOF
  sleep "$SECS"
fi

[ -n "$FRIDA" ] && wait $FRIDA 2>/dev/null
kill $DUMP 2>/dev/null; wait $DUMP 2>/dev/null
say "capture stopped"

# ---- "after" state -------------------------------------------------------------------
snap_ids > "$OUT/after-ids.txt"
lsusb -v -d "$VID:" > "$OUT/after-lsusb.txt" 2>/dev/null
journalctl -u displaylink-driver.service --since "-$((SECS/60 + 3)) min" > "$OUT/dlm.log" 2>&1
dmesg > "$OUT/dmesg.txt" 2>&1

# ---- convert the key file into what the decryptor expects ----------------------------
# decode-modeset-live.py writes {"krs": [...], "frames": [...]}; scripts/decrypt-dlm-cp.py wants a
# bare list of {"key","riv"} rows.
if [ -s "$OUT/keys-raw.json" ]; then
  python3 - "$OUT/keys-raw.json" "$OUT/keys.candidates.json" <<'PY' || warn "key conversion failed"
import json, sys
d = json.load(open(sys.argv[1]))
rows = d.get("krs", d if isinstance(d, list) else [])
json.dump(rows, open(sys.argv[2], "w"), indent=1)
print(f"   {len(rows)} (key,riv) candidate(s) -> {sys.argv[2]}")
PY
fi

# ---- report --------------------------------------------------------------------------
echo
say "results in $OUT"
printf '   wire            : %s\n' "$(du -h "$OUT/wire.pcapng" 2>/dev/null | cut -f1)"
printf '   key candidates  : %s\n' "$(python3 -c 'import json,sys;print(len(json.load(open(sys.argv[1]))))' "$OUT/keys.candidates.json" 2>/dev/null || echo 0)"
echo
say "did the firmware move? (a changed bcdDevice proves a flash)"
diff "$OUT/before-ids.txt" "$OUT/after-ids.txt" && echo "   no change in IDs"
echo
say "sanity-check the sealed traffic decrypts:"
echo "   tools/capture/decrypt-dlm-cp.py $OUT/wire.pcapng $OUT/keys.candidates.json | head -40"
echo
if [ -s "$JOURNAL" ]; then
  printf '   journal        : %s step(s) -> %s\n' "$(($(wc -l < "$JOURNAL") / 2))" "$JOURNAL"
  say "slice the capture by action, e.g. the cursor steps:"
  echo "   awk -F'\t' '/cursor/ {print}' $JOURNAL"
  echo "   tools/capture/decrypt-dlm-cp.py $OUT/wire.pcapng $OUT/keys.candidates.json --start <t> --end <t>"
fi
say "then send the whole directory. Even a keyless run is worth sending -- the wire cannot be"
say "recaptured later, but keys can be re-extracted from the recorded DLM build hash."
