#!/bin/bash
# One atomic cycle: fresh DLM, attach, force a session, collect. DLM exits after a re-authorise,
# so starting it in a previous command always loses the race.
set -u
# 'DisplayLinkManager' is 18 chars; pkill -x matches on comm, capped at 15, so it
# silently never matches. Match the full command line instead.
for p in $(ps -eo pid,args --no-headers | awk '$2=="./Display""LinkManager"{print $1}'); do kill $p; done
sleep 2
lsmod | grep -q "^evdi" || insmod /home/fireburn/Downloads/dl-scripts/evdi/module/evdi.ko
echo 1 > /sys/devices/evdi/remove_all 2>/dev/null
sleep 2
( cd /opt/displaylink && exec ./DisplayLinkManager ) >/home/fireburn/vinocap/dlm-oneshot.log 2>&1 &
for i in $(seq 1 30); do
  [ "$(cat /sys/devices/evdi/count 2>/dev/null || echo 0)" != "0" ] && break
  sleep 1
done
echo "[*] DLM up, evdi=$(cat /sys/devices/evdi/count 2>/dev/null)"
D=/sys/bus/usb/devices/4-2.1/
( sleep 14; echo 0 > ${D}authorized; sleep 3; echo 1 > ${D}authorized ) >/dev/null 2>&1 &
PYTHONPATH=/home/fireburn/.local/lib/python3.14/site-packages python3 /home/fireburn/vinocap/run-ba.py 60
