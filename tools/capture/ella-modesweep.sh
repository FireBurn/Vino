#!/bin/bash
# Drive one DLM output through a resolution sweep, journalling each step.
#
# Only a real RESOLUTION change re-issues the set-mode record (id=0x48 sub=0x22); a refresh-only
# change yields nothing but heartbeats. So every step here changes the pixel count, and the modes
# are chosen to cover the ones the driver currently has no measured profile for and has to infer.
#
#   ./ella-modesweep.sh <output-name> <steps-file>
#
# Run as the DESKTOP USER, not root: every step is a kscreen-doctor call.
#
# The journal it prints maps a wall-clock instant onto a mode, which is what slices the capture:
# feed the timestamps to decrypt-dlm-cp.py --start/--end to read one mode's records alone.
set -u

OUT_NAME="${1:-DP-2}"
JOURNAL="${2:-/dev/stdout}"

# mode-index:label. Indices are kscreen-doctor's, read from `kscreen-doctor -o` immediately before
# the run -- they are per-output and not stable across replugs, so never hardcode them elsewhere.
STEPS=(
    "2:1920x1080@60"
    "1:2560x1440@59.95"
    "6:1680x1050@59.88"
    "8:1280x1024@60.02"
    "9:1440x900@59.90"
    "10:1280x960@60.00"
    "11:1280x720@60.00"
    "16:1024x768@60.00"
    "19:800x600@60.32"
    "30:640x480@60.00"
    "2:1920x1080@60"
)

# Long enough for the dock to complete the mode set and emit its first frames, short enough that
# the whole sweep fits one capture window.
SETTLE="${SETTLE:-14}"

echo -e "epoch\tiso\tstep\tmode" | tee -a "$JOURNAL"
for s in "${STEPS[@]}"; do
    idx="${s%%:*}"
    label="${s#*:}"
    printf '%s\t%s\t%s\t%s\n' "$(date +%s.%N)" "$(date -Is)" "begin" "$label" | tee -a "$JOURNAL"
    if ! kscreen-doctor "output.${OUT_NAME}.mode.${idx}" >/dev/null 2>&1; then
        printf '%s\t%s\t%s\t%s\n' "$(date +%s.%N)" "$(date -Is)" "FAILED" "$label" | tee -a "$JOURNAL"
        continue
    fi
    sleep "$SETTLE"
    printf '%s\t%s\t%s\t%s\n' "$(date +%s.%N)" "$(date -Is)" "settled" "$label" | tee -a "$JOURNAL"
done
echo "sweep complete"
