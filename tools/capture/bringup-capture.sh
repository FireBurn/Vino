#!/bin/bash
# Capture one Ella dock bring-up end to end, from enumeration to first content frames.
#
# Each failed bring-up hard-wedges this dock, so a power cycle buys exactly one attempt. This
# script therefore captures everything an attempt can yield in one pass: the full wire, the kernel
# log, the connector and CRTC state, and a pass/fail verdict, into one directory per run.
#
#   ./bringup-capture.sh            # run one attempt
#   ./bringup-capture.sh --cleanup  # remove the blacklist, let vino autoload again
#
# Run it as your normal user. It sudos for the privileged steps and needs kscreen-doctor to reach
# your session, which it cannot do from root.

set -u

BASE="$HOME/vinocap"
BLACKLIST=/etc/modprobe.d/zz-vino-manual.conf
VENDOR=17e9
SETTLE=12      # dumpcap probes every device's descriptors at start; a bind racing that reads as a
               # missing identity descriptor and looks exactly like a wedged dock
WINDOW=60      # total capture seconds; a slow bring-up has put the first frame past 38 s

die() { echo "!! $*" >&2; exit 1; }
say() { echo "== $*"; }

MODE=reload
LOAD=no
WALLS=(/usr/share/wallpapers/FallenLeaf/contents/images/2560x1600.jpg
       /usr/share/wallpapers/Opal/contents/images/3840x2160.png)
ARGS=()
for a in "$@"; do
    if [[ $a == --load ]]; then LOAD=yes; else ARGS+=("$a"); fi
done
set -- "${ARGS[@]:-}"
# The failure being chased landed ~45 s after both heads went live, so a loaded run needs a window
# long enough to contain that, not the 60 s an idle bring-up needs.
[[ $LOAD == yes ]] && WINDOW=150
case "${1:-}" in
--cleanup) sudo rm -f "$BLACKLIST"
           say "blacklist removed; vino will autoload on the next plug"; exit 0 ;;
--power)   MODE=power ;;
--reauth)  MODE=reauth ;;
--reload|"") MODE=reload ;;
*) die "usage: $0 [--reload|--reauth|--power|--cleanup]" ;;
esac
say "cycle mode: $MODE"

command -v kscreen-doctor >/dev/null || die "kscreen-doctor not found"
[[ -w /dev/null ]] || die "run this as your normal user, not root"

# dumpcap holds cap_dac_read_search but not cap_dac_override and drops privileges after opening the
# interface, so it can only write where the dropped user can. A user-owned 0755 directory is NOT
# enough -- measured. 0777 is.
mkdir -p "$BASE" && chmod 777 "$BASE"

n=1
while [[ -e "$BASE/run$(printf %02d $n)" ]]; do n=$((n+1)); done
RUN="$BASE/run$(printf %02d $n)"
mkdir -p "$RUN" && chmod 777 "$RUN"
say "run directory $RUN"

# ---------------------------------------------------------------- phase 0: hold vino off
if [[ ! -f $BLACKLIST ]]; then
    echo 'blacklist vino' | sudo tee "$BLACKLIST" >/dev/null
    say "installed $BLACKLIST (removed by --cleanup)"
fi
if lsmod | grep -q '^vino'; then
    # The compositor holds the DRM device open, so the module refcount is never zero while the card
    # exists and `modprobe -r` fails outright. Unbinding the interfaces unplugs the card first,
    # which is also the path that has to tell the dock to power its sinks down.
    for i in /sys/bus/usb/drivers/vino/*:*; do
        [[ -e $i ]] || continue
        say "unbinding $(basename "$i")"
        basename "$i" | sudo tee /sys/bus/usb/drivers/vino/unbind >/dev/null 2>&1
    done
    sleep 3
    say "unloading vino"
    sudo modprobe -r vino || die "could not unload vino -- check /proc/<pid>/stack for D-state"
fi

dock_path() {
    local d
    for d in /sys/bus/usb/devices/*; do
        [[ -f $d/idVendor ]] || continue
        [[ $(<"$d/idVendor") == "$VENDOR" ]] && { echo "$d"; return 0; }
    done
    return 1
}

# ---------------------------------------------------------------- phase 1: get a fresh dock
# Three ways to reach an unbound dock, in increasing order of how much they disturb it. `reload`
# never touches USB, so it does not re-run enumeration and cannot clear a dock-side wedge;
# `reauth` re-enumerates without anyone touching the hardware; `power` is the only one that
# recovers the deep wedge, and it needs hands.
case "$MODE" in
power)
    echo
    echo "  >>> POWER-CYCLE THE DOCK NOW (pull its power, count to five, plug it back in) <<<"
    echo
    if dock_path >/dev/null; then
        say "waiting for the dock to disappear..."
        for _ in $(seq 120); do dock_path >/dev/null || break; sleep 1; done
        dock_path >/dev/null && die "dock never went away -- did the power actually drop?"
        say "dock gone"
    fi
    say "waiting for the dock to enumerate..."
    for _ in $(seq 180); do DEV=$(dock_path) && break; sleep 1; done
    ;;
reauth)
    DEV=$(dock_path) || die "dock not present"
    say "de-authorizing $(basename "$DEV") to force re-enumeration"
    echo 0 | sudo tee "$DEV/authorized" >/dev/null
    sleep 5
    echo 1 | sudo tee "$DEV/authorized" >/dev/null
    say "waiting for the dock to re-enumerate..."
    DEV=""
    for _ in $(seq 60); do DEV=$(dock_path) && break; sleep 1; done
    sleep 3
    ;;
reload)
    say "no USB cycle: vino is simply unloaded and reloaded"
    DEV=$(dock_path) || die "dock not present"
    ;;
*) die "unknown mode '$MODE' (power|reauth|reload)" ;;
esac
[[ -n ${DEV:-} ]] || die "dock never came back"
BUS=$(<"$DEV/busnum"); DEVNUM=$(<"$DEV/devnum")
say "dock on bus $BUS device $DEVNUM ($(basename "$DEV"))"
[[ -e /dev/usbmon$BUS ]] || { sudo modprobe usbmon; [[ -e /dev/usbmon$BUS ]] || die "no /dev/usbmon$BUS"; }

# ---------------------------------------------------------------- phase 2: capture
{
    echo "run:        $RUN"
    echo "date:       $(date -Is)"
    echo "bus/dev:    $BUS/$DEVNUM"
    echo "module:     $(sha256sum "$(modinfo -n vino)" | cut -c1-16)"
    echo "kernel:     $(uname -r)"
} > "$RUN/META.txt"

say "starting dumpcap on usbmon$BUS"
sudo dumpcap -i "usbmon$BUS" -s 0 -B 256 -a "duration:$((SETTLE + WINDOW))" \
     -w "$RUN/wire.pcapng" -q >"$RUN/dumpcap.log" 2>&1 &
DUMP=$!
# -W, not -w: follow only what arrives from here on. Replaying the whole ring buffer would let an
# error from an earlier session be scored against this run.
sudo dmesg -W > "$RUN/dmesg.txt" 2>/dev/null &
KLOG=$!
trap 'sudo kill $DUMP $KLOG 2>/dev/null' EXIT

say "letting dumpcap settle for ${SETTLE}s (it probes every device's descriptors first)"
sleep "$SETTLE"

say "loading vino"
sudo modprobe vino debug=1 trace_crypto=1 || die "modprobe failed"

# The compositor may restore a stale setup with the dock outputs disabled, which produces no
# modeset and no pixels and would make this run say nothing about the dock. Force them on.
say "waiting for connectors, then enabling both dock outputs"
for _ in $(seq 30); do
    CARD=$(for c in /sys/class/drm/card*/; do
               [[ $(readlink -f "$c/device/driver" 2>/dev/null) == */vino ]] && basename "$c" && break
           done)
    [[ -n ${CARD:-} ]] && break
    sleep 1
done
sleep 6
mapfile -t OUTS < <(kscreen-doctor -o 2>/dev/null | sed -e 's/\x1b\[[0-9;]*m//g' \
                    | awk '/^Output:/{name=$3} /connected/&&name{print name; name=""}')
for o in "${OUTS[@]}"; do
    [[ $o == eDP-* ]] && continue
    say "enabling $o"
    kscreen-doctor "output.$o.enable" >>"$RUN/kscreen.log" 2>&1
    sleep 3
done

# An idle desktop exercises almost nothing: head 0 spent most of run01 reporting that no strip
# content had changed. The failure being chased appeared well after both heads were lit and under
# real repaint traffic, so drive every head continuously for the rest of the window. A wallpaper
# flip repaints all screens at once, which is the only whole-desktop damage reachable without a
# pointer on Wayland.
if [[ $LOAD == yes ]]; then
    say "driving both heads with wallpaper flips for the rest of the window"
    (
        i=0
        while kill -0 $DUMP 2>/dev/null; do
            plasma-apply-wallpaperimage "${WALLS[$((i % ${#WALLS[@]}))]}" >/dev/null 2>&1
            i=$((i+1)); sleep 4
        done
    ) >>"$RUN/load.log" 2>&1 &
    LOADPID=$!
fi

say "capturing for the rest of the window..."
wait $DUMP 2>/dev/null
[[ -n ${LOADPID:-} ]] && kill $LOADPID 2>/dev/null
sudo kill $KLOG 2>/dev/null

# ---------------------------------------------------------------- phase 3: collect + verdict
say "collecting state"
kscreen-doctor -o 2>/dev/null | sed -e 's/\x1b\[[0-9;]*m//g' > "$RUN/kscreen-outputs.txt"
# The glob has to expand inside the privileged shell: /sys/kernel/debug is 0700 root, so an
# unprivileged expansion silently yields nothing and every verdict reads "no active CRTC".
sudo sh -c 'for s in /sys/kernel/debug/dri/*/state; do echo "--- $s"; cat "$s"; done' \
     2>/dev/null > "$RUN/drm-state.txt"
for c in /sys/class/drm/card*-DP-*; do
    [[ -e $c/status ]] || continue
    echo "$(basename "$c") $(<"$c/status") edid=$(sudo wc -c <"$c/edid" 2>/dev/null)"
done > "$RUN/connectors.txt"
sudo chown -R "$USER" "$RUN" 2>/dev/null

# Score only vino's own card: the laptop panel is always active and would mask a dead dock.
MINOR=${CARD#card}
sudo sh -c "cat /sys/kernel/debug/dri/$MINOR/state" 2>/dev/null > "$RUN/vino-state.txt"
LIT=$(grep -c 'active=1' "$RUN/vino-state.txt")
VERDICT="PASS ($LIT head(s) lit)"
grep -qE 'EPIPE|abandoning the session|resetting the dock' "$RUN/dmesg.txt" \
    && VERDICT="FAIL (pipe error)"
[[ $LIT -gt 0 ]] || VERDICT="FAIL (no active CRTC)"
{
    echo "verdict:    $VERDICT"
    echo "wire:       $(du -h "$RUN/wire.pcapng" 2>/dev/null | cut -f1)"
    echo "connectors:"; sed 's/^/  /' "$RUN/connectors.txt"
    echo "vino log:"
    grep -i vino "$RUN/dmesg.txt" | grep -vE ' ok [0-9]+ ' | tail -40 | sed 's/^/  /'
} >> "$RUN/META.txt"

echo
say "VERDICT: $VERDICT   ->  $RUN"
say "wire $(du -h "$RUN/wire.pcapng" 2>/dev/null | cut -f1), summary in $RUN/META.txt"
echo
echo "Run again for another attempt. When you are done, ./bringup-capture.sh --cleanup"
