#!/bin/bash
# Does the D6000 push pixels? Ridge bound ALONE; verdict on bytes, not on log lines.
export PATH=/usr/lib/llvm/22/bin:$PATH XDG_RUNTIME_DIR=/run/user/1000 WAYLAND_DISPLAY=wayland-0
D=/home/fireburn/vino-artifact-20260803
sudo /home/fireburn/Downloads/dl-scripts/vino/tools/hardware/vino-cycle.sh --unload >/dev/null 2>&1
sudo dmesg -c >/dev/null 2>&1
sudo modprobe vino rtc_utc_offset_minutes=60 >/dev/null 2>&1
sleep 4
for i in 2-1.3:1.0 2-1.3:1.1; do printf '%s' "$i" | sudo tee /sys/bus/usb/drivers/vino/unbind >/dev/null 2>&1; done
sleep 26
sudo rm -f $D/bis.pcapng
sudo timeout 20 dumpcap -i usbmon2 -s 0 -a duration:12 -w $D/bis.pcapng >/dev/null 2>&1 &
sleep 2
plasma-apply-wallpaperimage $D/blank.png   >/dev/null 2>&1; sleep 3
plasma-apply-wallpaperimage $D/pattern.png >/dev/null 2>&1
sleep 9
sudo chown fireburn:users $D/bis.pcapng 2>/dev/null
R=$(sudo dmesg | grep -c "usb 2-2.1: USB disconnect")
B=$(/home/fireburn/Downloads/dl-scripts/scratchpad/venv-np/bin/python \
    /home/fireburn/Downloads/dl-scripts/vino/tools/codec/usbmon_read.py $D/bis.pcapng 2>/dev/null \
    | awk '/ep 0x08|ep 0x0b/ {gsub(",","",$(NF-1)); s+=$(NF-1)} END {print s+0}')
S=$(sudo dmesg | grep -c "control session ready")
echo "resets=$R sessions=$S video_bytes=$B"
if [ "$R" -eq 0 ] && [ "${B:-0}" -gt 2000000 ]; then echo GOOD; else echo BAD; fi
