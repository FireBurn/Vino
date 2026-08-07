#!/bin/bash
# Drive one DLM recording sitting through every question vino still has to ask this dock:
# how DLM idles an output (DPMS), how it wakes it, how it disables ONE output while another is
# lit, and what its bring-up looks like when the monitors are in sockets 3 and 4.
#
#   terminal 1 (root):  sudo tools/capture/capture-portmap.sh --no-reauth --snap 4096 ~/vino-dpms-ports 3600
#   terminal 2 (you):        tools/capture/dpms-ports-runbook.sh ~/vino-dpms-ports
#
# ⚠ Run THIS ONE AS THE DESKTOP USER, not root: every step is a `kscreen-doctor` call and that
# needs the Wayland session, not a root shell.
#
# Why one sitting: the sealing keys are per USB session and frida only holds them while the DLM
# process it attached to lives. Every step below therefore runs against ONE DLM, one dumpcap and
# one key extractor -- cable moves and dock power cycles are fine (DLM survives them, the unit is
# masked so udev cannot restart it), but a DLM restart would end the run.
#
# Each step brackets itself in journal.tsv as begin:<label> / end:<label>, which is exactly what
# decrypt-dlm-cp.py --start/--end slices by.
set -uo pipefail

OUT="${1:?usage: tools/capture/dpms-ports-runbook.sh <outdir>   (the one capture-portmap.sh is writing)}"
OUT="$(cd "$OUT" && pwd)" || exit 1
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
[ -e "$OUT/journal.tsv" ] || { echo "no journal at $OUT/journal.tsv -- start capture-portmap.sh first" >&2; exit 1; }
[ -e "$OUT/STOP" ] && { echo "$OUT/STOP exists -- that capture has already finished" >&2; exit 1; }
command -v kscreen-doctor >/dev/null || { echo "kscreen-doctor not found" >&2; exit 1; }
[ "$(id -u)" = 0 ] && echo "!! running as root -- kscreen-doctor will not see the session" >&2

say()  { printf '\n\033[1;36m== %s\033[0m\n' "$*"; }
note() { printf '   %s\n' "$*"; }
act()  { printf '\033[1;33m>> %s\033[0m\n' "$*"; }
mark() { "$HERE/portmark.sh" "$OUT" "$1" >/dev/null; printf '   \033[0;90m[%s]\033[0m\n' "$1"; }
ask()  { printf '\033[1;32m?? %s\033[0m ' "$1"; read -r _; }

# Dock outputs are whatever the compositor has that is not the laptop panel. Under DLM these are
# evdi outputs and their names bear no relation to vino's, so they are resolved every time rather
# than remembered.
# ⚠ kscreen-doctor colours its output even into a pipe, so every field is wrapped in ANSI escapes.
# Strip them before matching or the name comes back empty and the step silently does nothing.
dock_outputs() {
  kscreen-doctor -o 2>/dev/null | sed 's/\x1b\[[0-9;]*m//g' \
    | sed -n 's/^Output: [0-9]* \([A-Za-z0-9-]*\).*/\1/p' \
    | grep -v '^eDP' | grep -v '^LVDS'
}

snapshot() { kscreen-doctor -o > "$OUT/kscreen-$1.txt" 2>&1; }

dpms_state() { kscreen-doctor --dpms show 2>/dev/null | sed 's/\x1b\[[0-9;]*m//g'; }
all_on()     { ! dpms_state | grep -q ': *off'; }

# One DPMS cycle: down for $2 seconds, then back up the way $3 says.
#   wake=dpms  -- the compositor's own restore path
#   wake=input -- you move the mouse, which is what a real user does
#
# ⚠ `kscreen-doctor --dpms off` exists in two shapes across releases: one applies the change and
# exits, the other holds the screens down for as long as it runs. Backgrounding it and killing it
# at the end of the window is correct for both -- for the second shape the kill IS the wake, and
# for the first it is a no-op on an already-dead pid.
dpms_cycle() {
  local label="$1" down="$2" wake="$3" pid
  mark "begin:$label"
  act "screens off for ${down}s -- DO NOT TOUCH the mouse or keyboard"
  kscreen-doctor --dpms off >>"$OUT/kscreen-doctor.log" 2>&1 &
  pid=$!
  sleep 3
  all_on && printf '   \033[1;33m!! nothing reports dpms off -- check %s\033[0m\n' "$OUT/kscreen-doctor.log"
  sleep "$down"
  if [ "$wake" = input ]; then
    act "now WAKE THEM: move the mouse or press shift"
    local i=0
    while [ $i -lt 60 ] && ! all_on; do sleep 1; i=$((i+1)); done
  fi
  kill $pid 2>/dev/null
  kscreen-doctor --dpms on >>"$OUT/kscreen-doctor.log" 2>&1
  sleep 12                       # let the dock finish whatever it does after the wake
  mark "end:$label"
  snapshot "$label"
}

cat <<'EOF'

  ┌────────────────────────────────────────────────────────────────────────────┐
  │  DLM recording sitting -- roughly 25 minutes, most of it waiting            │
  │                                                                            │
  │  A  ports 1+2 : DPMS off/on x3, then one output disabled while lit         │
  │  B  ports 3+4 : cable move, cold dock power-cycle, DPMS again              │
  │  C  ports 3+4 : one monitor unplugged and replugged                        │
  │                                                                            │
  │  Nothing here restarts DLM, so the keys hold for the whole run.            │
  └────────────────────────────────────────────────────────────────────────────┘
EOF

say "0. Baseline"
lsmod | grep -q '^vino ' && printf '   \033[1;31m!! vino is loaded and will steal the dock from DLM\033[0m\n'
pgrep -f '[D]isplayLinkManager' >/dev/null \
  || printf '   \033[1;31m!! no DisplayLinkManager running -- capture-portmap.sh starts it\033[0m\n'
# The lock screen is a full-screen repaint on every dock head, so a locked wake looks like a
# content storm on the wire and the password prompt stretches the step. Turn it off for the run.
if command -v kreadconfig6 >/dev/null \
   && [ "$(kreadconfig6 --file kscreenlockerrc --group Daemon --key Autolock 2>/dev/null)" != "false" ]; then
  printf '   \033[1;33m!! the screen locker is on -- it will fire with the blank and repaint every head\033[0m\n'
  printf '      turn it off for the run:  System Settings > Screen Locking > uncheck lock after\n'
fi
note "Dock outputs seen by the compositor: $(dock_outputs | tr '\n' ' ')"
snapshot baseline
ask "Both monitors in PORTS 1 and 2, both showing a desktop? [Enter]"
mark "mark:panels-lit-ports12"

# ---------------------------------------------------------------------------------------
# A1/A2. Two fast DPMS cycles. This is the transcript vino needs most: today `blank_head()`
# does nothing at all on this dock, which is exactly why the panels stay on when the laptop
# screen goes off, and why waking re-runs a whole bring-up.
# ---------------------------------------------------------------------------------------
say "A1. DPMS off/on, woken by the compositor  (~45 s)"
dpms_cycle dpms-fast-1 20 dpms

say "A2. Same again -- one trial proves nothing on this dock  (~45 s)"
dpms_cycle dpms-fast-2 20 dpms

# ---------------------------------------------------------------------------------------
# A3. The real thing. `kscreen-doctor --dpms off` and an idle timeout are supposed to be the
# same KWin path; if the two transcripts differ, everything measured with the fast form is
# suspect, and that is worth 3 minutes to know once.
# ---------------------------------------------------------------------------------------
say "A3. REAL idle blank -- the 2-minute screen timeout, woken by hand  (~3.5 min)"
act "hands OFF the machine until this says otherwise -- any keypress restarts the idle timer"
mark "begin:dpms-idle"
i=0
while [ $i -lt 170 ]; do
  sleep 5; i=$((i+5))
  printf '\r   waiting for the idle timeout... %3ds  (dock outputs: %s)' \
    "$i" "$(dpms_state | grep -c ': *off') off"
done
printf '\n'
mark "mark:dpms-idle-blanked"
act "now WAKE the machine: move the mouse"
i=0
while [ $i -lt 90 ] && ! all_on; do sleep 1; i=$((i+1)); done
sleep 15
mark "end:dpms-idle"
snapshot dpms-idle
ask "Did the panels actually go dark and come back? [Enter]"

# ---------------------------------------------------------------------------------------
# A4. One output off while its sibling stays lit. Vino currently treats this as a dock-wide
# re-activation because doing it per-connector re-enumerates the dock; DLM's own answer to the
# same request is the ground truth for whether that is really necessary.
# ---------------------------------------------------------------------------------------
say "A4. Disable ONE dock output while the other stays lit  (~40 s)"
FIRST="$(dock_outputs | head -1)"
if [ -n "$FIRST" ]; then
  mark "begin:single-output-disable"
  act "disabling $FIRST"
  kscreen-doctor "output.$FIRST.disable" >>"$OUT/kscreen-doctor.log" 2>&1
  sleep 15
  mark "mark:single-output-disabled"
  act "re-enabling $FIRST"
  kscreen-doctor "output.$FIRST.enable" >>"$OUT/kscreen-doctor.log" 2>&1
  sleep 20
  mark "end:single-output-disable"
  snapshot single-output-disable
  ask "Is $FIRST showing a desktop again? [Enter]"
else
  note "no dock output found -- skipping"
fi

# ---------------------------------------------------------------------------------------
# B. Sockets 3 and 4. Connector index = physical socket - 1, so this is connectors 2 and 3:
# one on each video endpoint, same pairing as 1+2 but the far slot of each pair.
# ---------------------------------------------------------------------------------------
say "B1. Move BOTH cables to PORTS 3 and 4"
act "unplug both monitors from ports 1 and 2, plug them into ports 3 and 4"
mark "begin:cable-move-to-34"
ask "Cables moved? [Enter]"
note "waiting 30 s for DLM to bring them up"
sleep 30
mark "end:cable-move-to-34"
snapshot ports34-warm
ask "Are BOTH panels lit on ports 3 and 4? (a 'no' here is itself the answer) [Enter]"

say "B2. Cold bring-up on ports 3+4 -- POWER-CYCLE THE DOCK  (~1 min)"
note "pull the dock's power (or its host USB-C) and put it back after ~5 s"
note "DLM survives this: the unit is masked, so udev cannot restart it and the keys hold"
mark "begin:cold-ports34"
ask "Dock power-cycled? [Enter]"
note "waiting 45 s for the cold bring-up"
sleep 45
mark "end:cold-ports34"
snapshot ports34-cold
ask "Both panels lit after the power cycle? [Enter]"

say "B3. DPMS off/on on ports 3+4  (~45 s)"
dpms_cycle dpms-ports34 20 dpms

# ---------------------------------------------------------------------------------------
# C. A hotplug on a far-slot connector: the dock pushes these on sub=0x0c, and vino has only
# ever seen the push for connectors 0 and 1.
# ---------------------------------------------------------------------------------------
say "C. Unplug and replug ONE monitor  (~1 min)"
mark "begin:hotplug-port3"
act "unplug the monitor in PORT 3"
ask "Unplugged? [Enter]"
sleep 12
mark "mark:hotplug-port3-out"
act "plug it back into PORT 3"
ask "Replugged? [Enter]"
sleep 25
mark "end:hotplug-port3"
snapshot hotplug-port3

say "Done"
mark "mark:runbook-complete"
note "stopping the recorder"
touch "$OUT/STOP"
note "terminal 1 will close the wire capture and wait for the key extractor to flush --"
note "let it finish; killing it is how a keyless capture happens."
note ""
note "Afterwards:  $OUT/{wire.pcapng,keys*.json,journal.tsv}"
