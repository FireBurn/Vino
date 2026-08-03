#!/bin/bash
# Journal one step of a running capture-portmap.sh, with a full connector snapshot attached.
#
#   tools/capture/portmark.sh <outdir> <label>
#
# The label is what decrypt-dlm-cp.py --start/--end is later sliced by, so name the ACTION
# ("unplug:port1", "plug:port3"), not the intent. Snapshots go alongside because "which connector
# was connected when" is the whole question here, and reconstructing it afterwards from the wire
# alone is exactly the guesswork this file exists to remove.
set -uo pipefail
OUT="${1:?usage: portmark.sh <outdir> <label>}"
LABEL="${2:?usage: portmark.sh <outdir> <label>}"
J="$OUT/journal.tsv"
[ -e "$J" ] || { echo "no journal at $J -- is the capture running?" >&2; exit 1; }

TS=$(date +%s.%N)
{
  printf '%s\tmark\t%s\n' "$TS" "$LABEL"
  for c in /sys/class/drm/card*-*/; do
    [ -e "$c/status" ] || continue
    printf '%s\tsnap\t%s\t%s\t%s\t%s\n' "$TS" "$LABEL" "$(basename "$c")" \
      "$(cat "$c/status" 2>/dev/null)" "$(head -1 "$c/modes" 2>/dev/null)"
  done
} >> "$J"
printf 'marked %s @ %s\n' "$LABEL" "$TS"
