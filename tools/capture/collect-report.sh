#!/bin/bash
# Gather everything a vino bug report needs, into one tarball. Read-only.
#
# Reports what the dock is, what vino made of it, and what the kernel logged. For a dock vino
# cannot drive at all, docs/new-device-capture.md is the fuller recipe.
set -u

OUT="vino-report-$(date -u +%Y%m%d-%H%M%S)"
mkdir -p "$OUT"

{
    echo "date:    $(date -u)"
    echo "kernel:  $(uname -r)"
    echo "distro:  $(. /etc/os-release 2>/dev/null && echo "$PRETTY_NAME")"
    echo "module:  $(modinfo -n vino 2>/dev/null || echo 'not installed')"
    echo "loaded:  $(grep -c '^vino ' /proc/modules)"
    echo "params:  $(grep -ho 'vino[^ ]*' /proc/cmdline 2>/dev/null)"
} > "$OUT/system.txt"

lsusb 2>/dev/null > "$OUT/lsusb.txt"

# Every DisplayLink device, in full. Descriptors are what identify an unfamiliar dock.
for dev in $(lsusb 2>/dev/null | grep -i 17e9 | sed 's/Bus \([0-9]*\) Device \([0-9]*\).*/\1:\2/'); do
    bus=${dev%%:*}
    devnum=${dev##*:}
    lsusb -v -s "$bus:$devnum" 2>/dev/null >> "$OUT/dock-descriptors.txt"
done

# Which USB port it is on, and whether it has re-enumerated.
for d in /sys/bus/usb/devices/*/; do
    [ -e "$d/idVendor" ] || continue
    [ "$(cat "$d/idVendor" 2>/dev/null)" = "17e9" ] || continue
    {
        echo "== $(basename "$d")"
        for f in idProduct bcdDevice devnum speed serial product manufacturer authorized; do
            [ -e "$d/$f" ] && echo "  $f: $(cat "$d/$f" 2>/dev/null)"
        done
    } >> "$OUT/dock-topology.txt"
done

# Connector state for every DRM device: which card is vino's, and what it is driving.
for card in /sys/class/drm/card*/; do
    # /sys/class/drm holds the connectors alongside the cards; only the cards have a dev node.
    [[ $(basename "$card") =~ ^card[0-9]+$ ]] || continue
    drv=$(basename "$(readlink -f "$card/device/driver" 2>/dev/null)" 2>/dev/null)
    echo "== $(basename "$card") driver=${drv:-unknown}" >> "$OUT/drm.txt"
    for conn in "$card"card*-*/; do
        [ -d "$conn" ] || continue
        echo "  $(basename "$conn"): status=$(cat "$conn/status" 2>/dev/null) enabled=$(cat "$conn/enabled" 2>/dev/null)" >> "$OUT/drm.txt"
        if [ -s "$conn/edid" ]; then
            cp "$conn/edid" "$OUT/edid-$(basename "$conn").bin" 2>/dev/null
            command -v edid-decode >/dev/null && edid-decode "$conn/edid" > "$OUT/edid-$(basename "$conn").txt" 2>&1
        fi
        [ -e "$conn/modes" ] && sed 's/^/    mode /' "$conn/modes" >> "$OUT/drm.txt"
    done
done

# The whole boot's kernel log. vino's own lines are the diagnosis; the surrounding USB and DRM
# messages are how a dock reset or a re-enumeration is told apart from a driver bug.
if command -v journalctl >/dev/null; then
    journalctl -k -b --no-pager -o short-monotonic > "$OUT/dmesg.txt" 2>/dev/null
else
    dmesg > "$OUT/dmesg.txt" 2>/dev/null
fi
grep -E 'vino|usb .*17e9|drm' "$OUT/dmesg.txt" > "$OUT/dmesg-relevant.txt" 2>/dev/null

command -v kscreen-doctor >/dev/null && kscreen-doctor -o > "$OUT/kscreen.txt" 2>&1

tar czf "$OUT.tar.gz" "$OUT" && rm -rf "$OUT"

cat <<EOF

Wrote $OUT.tar.gz

Attach it to an issue at https://github.com/FireBurn/Vino/issues, and please say in words what
you saw on the panels -- that is the part no log can tell us.

It contains your monitors' EDIDs, which include their serial numbers. Look inside before posting
if that matters to you.

If a panel stayed dark, reloading with the control-protocol trace on gives us far more:

    sudo modprobe -r vino && sudo modprobe vino debug=1

then reproduce the problem and run this script again.
EOF
