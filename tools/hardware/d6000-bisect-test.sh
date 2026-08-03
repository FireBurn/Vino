#!/bin/bash
# Verdict for `git bisect` on the D6000 regression.
#
# Builds ONLY drivers/gpu/drm/vino from the current checkout, installs it, binds the D6000
# ALONE (the DL7400 is unbound, or shared module state contaminates the verdict), forces damage
# and measures whether the dock streams video without resetting.
#
# ⚠ Verdict is "does it push pixels", not "does a log line appear": `encrypted control session
# ready` and a `connected` DRM connector have both been observed on a dock showing nothing.
#
# Exit 0 = good, 1 = bad, 125 = untestable (build failed -- bisect skips).
set -u
K=/home/fireburn/Downloads/dl-scripts/vino/linux
RIDGE=(2-2.1:1.0 2-2.1:1.1); NAVARRO=(2-1.3:1.0 2-1.3:1.1)
export PATH=/usr/lib/llvm/22/bin:$PATH XDG_RUNTIME_DIR=/run/user/1000 WAYLAND_DISPLAY=wayland-0
D=/home/fireburn/vino-artifact-20260803

cd "$K" || exit 125
make LLVM=1 -j16 M=drivers/gpu/drm/vino modules >/tmp/bisect-build.log 2>&1 || { echo "BUILD FAILED -> skip"; exit 125; }
# A no-op rebuild prints no LD line, so check the artefact exists rather than the log text.
[ -f drivers/gpu/drm/vino/vino.ko ] || { echo "no vino.ko produced -> skip"; exit 125; }
sudo cp drivers/gpu/drm/vino/vino.ko /lib/modules/$(uname -r)/kernel/drivers/gpu/drm/vino/ && sudo depmod -a

sudo /home/fireburn/Downloads/dl-scripts/vino/tools/hardware/vino-cycle.sh --unload >/dev/null 2>&1
sudo dmesg -c >/dev/null 2>&1
sudo modprobe vino rtc_utc_offset_minutes=60 >/dev/null 2>&1
sleep 4
for i in "${NAVARRO[@]}"; do printf '%s' "$i" | sudo tee /sys/bus/usb/drivers/vino/unbind >/dev/null 2>&1; done
sleep 30

OUT=$D/bisect.pcapng; sudo rm -f "$OUT"
sudo dumpcap -i usbmon2 -s 0 -a duration:12 -w "$OUT" >/dev/null 2>&1 &
sleep 2
plasma-apply-wallpaperimage $D/blank.png   >/dev/null 2>&1; sleep 3
plasma-apply-wallpaperimage $D/pattern.png >/dev/null 2>&1
wait
sudo chown fireburn:users "$OUT" 2>/dev/null

RESETS=$(sudo dmesg | grep -c "usb 2-2.1: USB disconnect")
BYTES=$(/home/fireburn/Downloads/dl-scripts/scratchpad/venv-np/bin/python \
        /home/fireburn/Downloads/dl-scripts/vino/tools/codec/usbmon_read.py "$OUT" 2>/dev/null \
        | awk '/ep 0x08|ep 0x0b/ {gsub(",","",$(NF-1)); s+=$(NF-1)} END {print s+0}')
echo "resets=$RESETS video_bytes=$BYTES"
# A working Ridge answers a full-screen wallpaper change with megabytes and does not re-enumerate.
[ "$RESETS" -eq 0 ] && [ "${BYTES:-0}" -gt 2000000 ] && { echo GOOD; exit 0; }
echo BAD; exit 1
