#!/bin/bash
# Drive DLM through a mode matrix and record a set-mode for each, keyed so it decrypts.
#
#   sudo tools/capture/capture-modematrix.sh <outdir> [output-name]
#
# WHAT THIS IS FOR
#
# `id=0x48 sub=0x22` is DLM's set-mode. Three of its words are still unresolved on the D6000, and
# they are unresolved for a structural reason rather than a difficult one: the D6000 corpus cannot
# separate the variables.
#
#   off42  resolution-keyed (1440p => 0x0600, 1080p => 0x0400) at every measured refresh -- but
#          docs/protocol reads it as a DP link tier (1024 = HBR, 1536 = HBR2). Those two readings
#          agree on every D6000 mode and disagree on 1080p165, which is why 1080p165 is blocked.
#   off66  moves with refresh at fixed resolution (0x2810 at 1080p60 vs 0x083f at 1080p120), but is
#          measured at exactly ONE refresh for 1440p, so the mapping is a guess above 1080p.
#   off72  ZERO in every capture ever taken. It is believed to be a pixel-clock overflow field, and
#          no clock above 655.35 MHz has ever been on the wire, so DLM cannot settle it.
#
# A quad-4K DL-7400 part changes all three. It should not clamp to 120 Hz the way DLM clamps the
# D6000 (442,368,000 px/s), so modes past 655.35 MHz of pixel clock become reachable and off72 gets
# its first non-zero value. A 1440p panel at 165/180 Hz is enough: 2560x1440@180 is ~663 M active
# px/s, about 750 MHz of clock with blanking. 4K60 is NOT enough (~561 MHz).
#
# HOW IT DRIVES DLM
#
# DLM only reprograms the dock's timing at CONNECT; a runtime resolution change makes it scale and
# emits no set-mode. So each data point needs a fresh connect. This REPLUGS THE DOCK rather than
# restarting DLM, because a restart kills the frida session and the keys with it -- the whole point
# is that these messages are sealed and must decrypt.
set -uo pipefail

OUT="${1:?usage: sudo tools/capture/capture-modematrix.sh <outdir> [output-name]}"
WANT_OUTPUT="${2:-}"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
VID=17e9

say()  { printf '\033[1;36m==\033[0m %s\n' "$*"; }
warn() { printf '\033[1;33m!!\033[0m %s\n' "$*"; }
big()  { printf '\n\033[1;32m>>> %s\033[0m\n' "$*"; }
die()  { printf '\033[1;31mABORT:\033[0m %s\n' "$*" >&2; exit 1; }

[ "$(id -u)" = 0 ] || die "run with sudo"
mkdir -p "$OUT" || die "cannot create $OUT"
OUT="$(cd "$OUT" && pwd)"

# kscreen-doctor talks to the user's compositor, not root's.
USER_NAME="${SUDO_USER:-$(logname 2>/dev/null)}"
[ -n "$USER_NAME" ] || die "cannot determine the desktop user (set SUDO_USER)"
USER_UID="$(id -u "$USER_NAME")"
ks() { sudo -u "$USER_NAME" env XDG_RUNTIME_DIR="/run/user/$USER_UID" kscreen-doctor "$@" 2>&1; }
strip_ansi() { sed -r 's/\x1b\[[0-9;]*m//g'; }

modprobe usbmon 2>/dev/null
command -v dumpcap >/dev/null || die "dumpcap not found"
systemctl is-active --quiet displaylink-driver.service || die "displaylink-driver.service is not running"

# ---- find the dock's output and its modes ---------------------------------------------
ks -o | strip_ansi > "$OUT/outputs-before.txt"
if [ -z "$WANT_OUTPUT" ]; then
  say "outputs seen by the compositor:"
  grep -E '^Output:' "$OUT/outputs-before.txt" | sed 's/^/   /'
  read -rp "   which output is on the NEW dock? (e.g. DP-4): " WANT_OUTPUT
fi
MODES=$(awk -v o="$WANT_OUTPUT" '
  $1=="Output:" { cur=$3 }
  cur==o && /Modes:/ { sub(/.*Modes: */,""); print }
' "$OUT/outputs-before.txt" | tr ' ' '\n' | sed -E 's/^[0-9]+://; s/[!*]//g' | grep -E '^[0-9]+x[0-9]+@' | sort -u)
[ -n "$MODES" ] || die "no modes found for output '$WANT_OUTPUT' -- check the name"

# ---- choose the matrix -----------------------------------------------------------------
# Rank by active pixels/second; the top one is the off72 candidate. Then take, for the two highest
# resolutions, the lowest and highest refresh, which is what separates a resolution-keyed field
# from a refresh-keyed or link-tier-keyed one.
PICK=$(python3 - "$OUT" <<PY
import re, sys
modes = """$MODES""".split()
rows = []
for m in modes:
    g = re.match(r"(\d+)x(\d+)@([\d.]+)$", m)
    if not g:
        continue
    w, h, r = int(g[1]), int(g[2]), float(g[3])
    rows.append((w * h * r, w, h, r, m))
rows.sort(reverse=True)
chosen, seen = [], {}
if rows:
    chosen.append(rows[0][4])                      # highest clock: the off72 candidate
for res in sorted({(w, h) for _, w, h, _, _ in rows}, reverse=True)[:2]:
    same = [x for x in rows if (x[1], x[2]) == res]
    for cand in (same[0][4], same[-1][4]):         # highest and lowest refresh at that resolution
        if cand not in chosen:
            chosen.append(cand)
for cand in [x[4] for x in rows if (x[1], x[2]) == (1920, 1080)][:1]:
    if cand not in chosen:
        chosen.append(cand)                        # a 1080p anchor: the one mode already decoded
print(" ".join(chosen))
for px, w, h, r, m in rows[:1]:
    clk = px * 1.13 / 1e6
    sys.stderr.write(f"top mode {m}: {px/1e6:.0f} M active px/s, ~{clk:.0f} MHz with blanking; "
                     f"off72 needs >655.35 MHz -> {'REACHES IT' if clk > 655.35 else 'NOT ENOUGH'}\n")
PY
)
say "mode matrix for $WANT_OUTPUT: $PICK"

# ---- recorders --------------------------------------------------------------------------
BUS=$(for d in /sys/bus/usb/devices/*/; do
        [ "$(cat "$d/idVendor" 2>/dev/null)" = "$VID" ] || continue
        cat "$d/busnum" 2>/dev/null; done | sort -un | head -1)
[ -n "$BUS" ] || die "no $VID device attached"
say "capturing usbmon$BUS and usbmon0"
setsid dumpcap -i "usbmon$BUS" -s 0 -w "$OUT/wire.pcapng"        >"$OUT/dumpcap.log" 2>&1 & D1=$!
setsid dumpcap -i usbmon0      -s 0 -w "$OUT/wire-allbus.pcapng" >"$OUT/dumpcap0.log" 2>&1 & D2=$!
sleep 2
kill -0 $D1 2>/dev/null || { cat "$OUT/dumpcap.log"; die "dumpcap failed"; }

FRIDA_PP="${FRIDA_PP:-}"
if [ -z "$FRIDA_PP" ]; then
  for d in /home/*/.local/lib/python3*/site-packages; do [ -d "$d/frida" ] && FRIDA_PP="$d" && break; done
fi
FR=""
if [ -n "$FRIDA_PP" ] && pgrep -f '[D]isplayLinkManager' >/dev/null; then
  # One frida session for the WHOLE matrix. Replugging keeps DLM alive so the session survives;
  # restarting DLM would not, and every mode after the first would come back sealed.
  env PYTHONPATH="$FRIDA_PP" python3 "$HERE/decode-modeset-live.py" \
      --secs 1800 --out "$OUT/keys-raw.json" > "$OUT/keys.log" 2>&1 &
  FR=$!
  say "key extractor attached (pid $FR) -- ONE session for the whole matrix"
  sleep 3
else
  warn "no frida / no DLM process: set-mode messages will be captured SEALED and unreadable"
fi

JOURNAL="$OUT/journal.tsv"; : > "$JOURNAL"
mark() { printf '%s\t%s\n' "$(date +%s.%N)" "$1" >> "$JOURNAL"; }

wait_for_dock() {  # returns when a 17e9 device is present (or absent, with --gone)
  local want_present=1; [ "${1:-}" = "--gone" ] && want_present=0
  for _ in $(seq 1 120); do
    local n; n=$(ls -d /sys/bus/usb/devices/*/ 2>/dev/null | while read -r d; do
      [ "$(cat "$d/idVendor" 2>/dev/null)" = "$VID" ] && echo x; done | wc -l)
    if [ "$want_present" = 1 ] && [ "$n" -gt 0 ]; then return 0; fi
    if [ "$want_present" = 0 ] && [ "$n" -eq 0 ]; then return 0; fi
    sleep 1
  done
  return 1
}

# ---- the matrix --------------------------------------------------------------------------
mark "begin:matrix"
for M in $PICK; do
  big "MODE $M"
  mark "begin:mode:$M"
  say "applying $M (by WxH@rate -- mode INDICES are renumbered between calls and silently set the wrong mode)"
  ks output."$WANT_OUTPUT".mode."$M" | sed 's/^/   /'
  sleep 4
  CUR=$(ks -o | strip_ansi | awk -v o="$WANT_OUTPUT" '$1=="Output:"{cur=$3} cur==o && /Modes:/{print}' \
        | grep -oE '[0-9]+x[0-9]+@[0-9.]+\*' | head -1 | tr -d '*')
  say "compositor now reports: ${CUR:-unknown}"
  echo "$M -> ${CUR:-unknown}" >> "$OUT/mode-applied.txt"

  big "UNPLUG THE DOCK, wait 3 seconds, PLUG IT BACK IN."
  echo "  (DLM programs the dock's timing only at CONNECT. A runtime change makes it scale and"
  echo "   emits no set-mode, so every data point needs a real reconnect. Replug -- do NOT restart"
  echo "   DLM, that would kill the key session and the rest of the matrix would not decrypt.)"
  mark "unplug-prompt:$M"
  if wait_for_dock --gone; then
    say "dock gone"; mark "dock-gone:$M"
  else
    warn "no unplug detected; continuing anyway"
  fi
  if wait_for_dock; then
    say "dock back"; mark "dock-present:$M"
  else
    warn "dock did not come back within 120 s"
  fi
  say "letting it settle and re-apply (30 s)"
  sleep 20
  # KDE restores its saved per-output config on hotplug, but verify rather than assume.
  CUR2=$(ks -o | strip_ansi | awk -v o="$WANT_OUTPUT" '$1=="Output:"{cur=$3} cur==o && /Modes:/{print}' \
         | grep -oE '[0-9]+x[0-9]+@[0-9.]+\*' | head -1 | tr -d '*')
  say "after reconnect the output is at: ${CUR2:-unknown}  (wanted $M)"
  echo "$M -> reconnect ${CUR2:-unknown}" >> "$OUT/mode-applied.txt"
  sleep 10
  mark "end:mode:$M"
done
mark "end:matrix"

# ---- a few things only this device can answer, while it is still connected ---------------
big "HEAD ENUMERATION -- plug a monitor into a DIFFERENT dock port"
echo "  A quad-head part is the first chance to see how head ids are encoded beyond 0/1. On the"
echo "  D6000 the head selector is a single byte (probe byte22, EDID engage off23, cursor off22)"
echo "  and with two heads a 0/1 index and a 1<<head bitmask are indistinguishable. Three or four"
echo "  heads tell them apart. Move the monitor to the LAST DP port if you only have one."
mark "begin:head-move"
read -rp "  press enter when the monitor is on another port and has settled: " _ || true
mark "end:head-move"

big "DPMS"
echo "  blank the screen and wake it. The D6000 corpus cannot settle the sink power-down because a"
echo "  DLM output toggle emits the same 0x2e/0x2f sequence as a mode-set bracket; an isolated"
echo "  action is what disentangles it."
mark "begin:dpms"; sleep 25; mark "end:dpms"

big "MONITOR HOTPLUG"
echo "  unplug the monitor from the dock, wait 10 s, plug it back in."
mark "begin:monitor-unplug"; sleep 40; mark "end:monitor-unplug"

# ---- stop ---------------------------------------------------------------------------------
say "stopping"
[ -n "$FR" ] && { kill -INT "$FR" 2>/dev/null; wait "$FR" 2>/dev/null; }
kill -TERM $D1 $D2 2>/dev/null; sleep 2; kill -KILL $D1 $D2 2>/dev/null
ks -o | strip_ansi > "$OUT/outputs-after.txt"

if [ -s "$OUT/keys-raw.json" ]; then
  python3 - "$OUT/keys-raw.json" "$OUT/keys.candidates.json" <<'PY'
import json, sys
d = json.load(open(sys.argv[1]))
rows = d.get("krs", d if isinstance(d, list) else [])
json.dump(rows, open(sys.argv[2], "w"), indent=1)
print(f"   {len(rows)} (key,riv) candidate(s) -> {sys.argv[2]}")
PY
fi

echo
say "done: $OUT"
say "slice each mode out of the wire and decode its set-mode:"
echo "   awk -F'\\t' '/mode:/' $OUT/journal.tsv"
echo "   tools/capture/decrypt-dlm-cp.py $OUT/wire.pcapng $OUT/keys.candidates.json --start <t> --end <t>"
echo
say "what to read out of the decodes (id=0x48 sub=0x22):"
echo "   off42  same at two refreshes of one resolution => resolution-keyed; different => link tier"
echo "   off44  should equal the refresh rate (already established; a third platform confirms it)"
echo "   off66  differs between the two refreshes at the SAME resolution => the refresh mapping"
echo "   off72  NON-ZERO on the >655 MHz mode is the first observation of this field, ever"
