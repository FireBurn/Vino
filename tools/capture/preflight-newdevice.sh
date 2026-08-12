#!/bin/bash
# Preflight for a first-contact capture. RUN THIS THE NIGHT BEFORE, not on the day.
#
#   sudo tools/capture/preflight-newdevice.sh [--fix]
#
# Every check here corresponds to a way a real hardware run has been lost. The firmware flash we
# are trying to catch happens ONCE, on first contact between DLM and the device, so there is no
# second take: anything that can be established in advance must be established in advance.
#
# --fix writes the persistent config (usbmon autoload, udl/udlfb blacklist) and tells you whether
# a reboot is required. Without it the script only reports.
set -uo pipefail

FIX=0
[ "${1:-}" = "--fix" ] && FIX=1

PASS=0; WARN=0; FAIL=0; REBOOT=0
ok()   { printf '  \033[1;32mPASS\033[0m  %s\n' "$*"; PASS=$((PASS+1)); }
wrn()  { printf '  \033[1;33mWARN\033[0m  %s\n' "$*"; WARN=$((WARN+1)); }
bad()  { printf '  \033[1;31mFAIL\033[0m  %s\n' "$*"; FAIL=$((FAIL+1)); }
head_() { printf '\n\033[1;36m== %s\033[0m\n' "$*"; }

[ "$(id -u)" = 0 ] || { echo "run with sudo (some checks need root)"; exit 2; }

head_ "1. usbmon — the single most common lost capture"
if ! lsmod | grep -q '^usbmon'; then
  modprobe usbmon 2>/dev/null
fi
if [ -e /dev/usbmon0 ]; then
  ok "usbmon loaded, $(ls /dev/usbmon* | wc -l) node(s): $(ls /dev/usbmon* | tr '\n' ' ')"
else
  bad "no /dev/usbmon* even after modprobe — is CONFIG_USB_MON built on this kernel?"
fi
if [ -f /etc/modules-load.d/usbmon.conf ]; then
  ok "usbmon autoloads at boot (/etc/modules-load.d/usbmon.conf)"
elif [ "$FIX" = 1 ]; then
  echo usbmon > /etc/modules-load.d/usbmon.conf
  ok "wrote /etc/modules-load.d/usbmon.conf — usbmon will autoload from now on"
else
  wrn "usbmon does not autoload; a reboot before the run would leave you without it (--fix writes it)"
fi

head_ "2. capture tools"
if command -v dumpcap >/dev/null; then
  ok "dumpcap: $(dumpcap -v 2>/dev/null | head -1)"
  # Prove it can actually open a usbmon interface RIGHT NOW, rather than at the worst moment.
  T=$(mktemp /tmp/pf-XXXX.pcapng)
  if timeout 5 dumpcap -i usbmon0 -s 0 -a duration:2 -w "$T" >/dev/null 2>&1; then
    ok "dumpcap can open usbmon0 and wrote $(stat -c%s "$T") bytes in 2 s"
  else
    bad "dumpcap could NOT capture on usbmon0 — fix this tonight, not tomorrow"
  fi
  rm -f "$T"
  # dumpcap drops privileges (cap_dac_read_search, not cap_dac_override), so "root can write here"
  # does not imply "dumpcap can write here". The capture directory is created BY ROOT on the day,
  # so probe exactly that shape now.
  D=$(mktemp -d /root/../home/"${SUDO_USER:-root}"/pfprobe-XXXX 2>/dev/null || mktemp -d)
  if timeout 6 dumpcap -i usbmon0 -s 0 -a duration:1 -w "$D/p.pcapng" >/dev/null 2>&1 && [ -s "$D/p.pcapng" ]; then
    ok "dumpcap can write into a root-created directory under \$HOME ($D)"
  else
    bad "dumpcap cannot write into the root-created directory $D — pick an output path under \$HOME"
  fi
  rm -rf "$D"
else
  bad "dumpcap missing (install wireshark-cli / wireshark)"
fi
command -v tshark >/dev/null && ok "tshark present (offline analysis)" || wrn "tshark missing — fw-scan.py does not need it, but the doc's one-liners do"

head_ "3. DisplayLinkManager — it must be the build the key hook was derived for"
DLM=/opt/displaylink/DisplayLinkManager
KNOWN_681=d3584c4369a594e9bcac20b71150086559d171c40d4949c67ee6affb3f96bfdb
if [ -x "$DLM" ]; then
  H=$(sha256sum "$DLM" | cut -d' ' -f1)
  if [ "$H" = "$KNOWN_681" ]; then
    ok "DLM 6.8.1.0 (sha256 ${H:0:16}…) — AES core 0x269dd0, round key inline at rdi+16"
  else
    wrn "DLM sha256 ${H:0:16}… is NOT the 6.8.1.0 the key hook was derived for."
    wrn "the wire capture is unaffected, but frida keys will come back EMPTY until the offset is re-derived"
  fi
else
  bad "no $DLM"
fi
# `systemctl is-enabled` exits non-zero for a masked unit, and `set -o pipefail` would turn that
# into a failed test even though grep matched. Capture the word first.
DL_STATE="$(systemctl is-enabled displaylink-driver.service 2>/dev/null || true)"
if [ "$DL_STATE" = masked ]; then
  ok "displaylink-driver.service is MASKED — required: DLM must not see the device before capture is running"
else
  if [ "$FIX" = 1 ]; then
    systemctl mask displaylink-driver.service >/dev/null 2>&1 && ok "masked displaylink-driver.service"
  else
    bad "displaylink-driver.service is NOT masked. Plugging the dock in would hand it to DLM"
    bad "  with nothing recording — the flash would be gone. sudo systemctl mask displaylink-driver.service"
  fi
fi

head_ "4. firmware images DLM would push"
if ls /opt/displaylink/*-release.spkg >/dev/null 2>&1; then
  for f in /opt/displaylink/*-release.spkg; do
    id=$(strings -n 6 "$f" | grep -E '^[0-9a-f]{8}$' | head -1)
    dt=$(strings -n 6 "$f" | grep -E '^20[0-9]{2}-[0-9]{2}-[0-9]{2}$' | head -1)
    cn=$(strings -n 8 "$f" | grep -E '^[A-Z][a-zA-Z]{3,10}Doc.*OW$|OW$' | head -1)
    printf '        %-34s %8s B  %-11s %s %s\n' "$(basename "$f")" "$(stat -c%s "$f")" "${cn:-?}" "${id:-?}" "${dt:-?}"
  done
  ok "$(ls /opt/displaylink/*-release.spkg | wc -l) platform images present; the dock's identity blob tail names which one targets it"
  ok "  (RidgeDoc = D6000, NavaDock = DL-7400, EllaDock = DL-3x00, FflyMoni = monitor)"
  ok "the enforcer flashes on a version MISMATCH, so any dock older than the date above is a candidate"
else
  bad "no *-release.spkg under /opt/displaylink — nothing to compare wire bytes against"
fi

head_ "5. drivers that would race DLM for the interface"
BL=/etc/modprobe.d/zz-dl-capture.conf
# A module that is not built cannot autoload, so blacklisting it changes nothing and is not a
# reason to reboot. Distinguish the three states rather than reporting "not loaded" for all of
# them -- "not loaded" reads as reassurance when the real answer might be "loaded on next plug".
BLACKLIST_MATTERS=0
for m in udl udlfb; do
  if lsmod | grep -q "^$m "; then
    bad "$m is LOADED — it will race DLM for the device"
    BLACKLIST_MATTERS=1
  elif modinfo "$m" >/dev/null 2>&1; then
    ok "$m not loaded, but IS available — it could autoload when the device appears"
    BLACKLIST_MATTERS=1
  else
    ok "$m is not built on this kernel — it cannot autoload, so it cannot race DLM"
  fi
done
if [ -f "$BL" ]; then
  ok "blacklist present: $BL"
elif [ "$FIX" = 1 ]; then
  printf 'blacklist udl\nblacklist udlfb\n' > "$BL"
  ok "wrote $BL (udl, udlfb)"
  # Only a blacklist that actually suppresses something needs a reboot to take effect.
  [ "$BLACKLIST_MATTERS" = 1 ] && REBOOT=1 \
    || ok "neither module exists here, so the blacklist is belt-and-braces: NO REBOOT NEEDED"
else
  wrn "no udl/udlfb blacklist. --fix writes $BL"
fi
# vino used to match two exact product ids and so could not touch an unfamiliar dock. It now binds
# the FUNCTION -- interface class ff/00/03, plus the DFU interface -- with the product id wildcarded,
# so it claims every DisplayLink dock including one nobody has driven. Read the alias rather than
# trusting either statement: this check has been wrong before precisely because it was a claim about
# the driver instead of a measurement of it.
VINO_GENERIC=0
if modinfo -F alias vino 2>/dev/null | grep -q '^usb:v17E9p\*'; then
  VINO_GENERIC=1
fi
# A blacklist stops the modalias autoload an unfamiliar dock would otherwise trigger, while leaving
# an explicit `modprobe vino` working -- which is what the "let vino try" step needs. Prove it by
# resolving the alias rather than trusting the file's contents.
if [ "$VINO_GENERIC" = 1 ]; then
  VINO_ALIAS='usb:v17E9p4306d0257dc00dsc00dp00icFFisc00ip03in00'
  if modprobe -n -v "$VINO_ALIAS" 2>/dev/null | grep -q 'vino\.ko'; then
    if [ "$FIX" = 1 ]; then
      grep -q '^blacklist vino$' "$BL" 2>/dev/null || printf 'blacklist vino\n' >> "$BL"
      depmod -a 2>/dev/null
      ok "blacklisted vino in $BL — it can no longer autoload when the dock is plugged in"
    else
      bad "vino AUTOLOADS on any 17e9 DL3 interface. Plugging the dock in binds it with no warning."
      bad "  Fix: echo 'blacklist vino' | sudo tee -a $BL   (or --fix). An explicit modprobe still works."
    fi
  else
    ok "vino does not autoload for a DL3 display interface (blacklisted)"
  fi
fi
# The DFU bind is the dangerous half: probe reads the identity descriptor and, if a packaged image
# for that family is present and newer, writes it. The dock's DFU interface does not support upload,
# so there is no way back and the pre-flash firmware is gone. A vintage dock is ALWAYS older than the
# packaged image, so this is not a hypothetical.
VINO_FW=$(ls /lib/firmware/vino/*-release.spkg 2>/dev/null | wc -l)
if [ "$VINO_GENERIC" = 1 ] && [ "$VINO_FW" -gt 0 ]; then
  if [ "$FIX" = 1 ]; then
    mkdir -p /lib/firmware/vino/held-back
    mv /lib/firmware/vino/*-release.spkg /lib/firmware/vino/held-back/ 2>/dev/null
    ok "moved $VINO_FW packaged image(s) to /lib/firmware/vino/held-back/ — vino can no longer"
    ok "  auto-flash on probe. Restore with: sudo mv /lib/firmware/vino/held-back/*.spkg /lib/firmware/vino/"
  else
    bad "vino binds 17e9 by INTERFACE (product id wildcarded) and $VINO_FW packaged image(s) are in"
    bad "  /lib/firmware/vino/. Plugging the dock in makes VINO flash it on probe, before DLM ever"
    bad "  sees it: the first contact is spent, the pre-flash firmware is unrecoverable (DFU has no"
    bad "  upload), and the flash runs through a path never exercised on this dock family."
    bad "  Fix: sudo mv /lib/firmware/vino/*-release.spkg /lib/firmware/vino/held-back/   (or --fix)"
  fi
elif [ "$VINO_GENERIC" = 1 ]; then
  ok "vino binds by interface, but no packaged image is installed — probe cannot flash"
else
  ok "vino matches exact product ids only — it cannot claim an unfamiliar dock"
fi
# Even with flashing disarmed, a bound vino owns the display function and races DLM for it, which
# is the same fault the udl blacklist exists to prevent.
if lsmod | grep -q '^vino '; then
  if [ "$VINO_GENERIC" = 1 ]; then
    wrn "vino is LOADED (refcnt $(cat /sys/module/vino/refcnt 2>/dev/null)) and will bind the new dock's"
    wrn "  display function, racing DLM. Unplug the known dock and 'sudo modprobe -r vino' before"
    wrn "  first contact; capture-firstcontact.sh will not do it for you."
  else
    ok "vino loaded (refcnt $(cat /sys/module/vino/refcnt 2>/dev/null)) — exact-id table, no race"
  fi
else
  ok "vino not loaded"
fi

head_ "6. evdi — DLM does literally nothing without it"
if modinfo evdi >/dev/null 2>&1; then
  ok "evdi module available: $(modinfo -F filename evdi)"
  lsmod | grep -q '^evdi ' && ok "evdi loaded" || wrn "evdi not loaded — 'sudo modprobe evdi' before the run (the capture script does this too)"
else
  bad "no evdi module — DLM will not start"
fi

head_ "7. disk"
for d in /home /var; do
  avail=$(df -BG --output=avail "$d" 2>/dev/null | tail -1 | tr -dc '0-9')
  if [ -n "$avail" ] && [ "$avail" -ge 20 ]; then ok "$d has ${avail}G free"
  else bad "$d has only ${avail:-?}G free — a whole-bus capture needs headroom"; fi
done
echo "        (run the firmware capture with NO MONITORS attached: no video traffic, so the pcap"
echo "         stays in the tens of MB and the flash is unambiguous)"

head_ "8. frida — attached to an IDLE DLM before the device exists, never mid-flash"
PP=""
for p in /home/*/.local/lib/python3*/site-packages; do [ -d "$p/frida" ] && PP="$p"; done
if [ -n "$PP" ]; then
  ok "frida in $PP"
  env PYTHONPATH="$PP" python3 -c 'import frida' 2>/dev/null && ok "importable under root with PYTHONPATH" \
    || bad "frida present but not importable under root — check the PYTHONPATH form"
else
  wrn "frida not found; install as your NORMAL user: pip install --user frida pycryptodome"
fi
python3 -c 'import Crypto' 2>/dev/null && ok "pycryptodome present (offline decrypt)" || wrn "pycryptodome missing"

head_ "9. self-test the capture scripts (syntax only, no hardware)"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
for s in "$HERE"/capture-firstcontact.sh "$HERE"/capture-modematrix.sh "$HERE"/capture-newdevice.sh; do
  [ -f "$s" ] || { wrn "missing $s"; continue; }
  bash -n "$s" && ok "$(basename "$s") parses" || bad "$(basename "$s") has a syntax error"
done
for s in "$HERE"/fw-watch.py "$HERE"/fw-scan.py "$HERE"/dl-identity.py; do
  [ -f "$s" ] || { wrn "missing $s"; continue; }
  python3 -m py_compile "$s" 2>/dev/null && ok "$(basename "$s") compiles" || bad "$(basename "$s") has a syntax error"
done

head_ "10. prove the flash DECODER works, on a synthesised DFU download"
# Syntax is not the risk. The risk is that the one capture we cannot retake decodes into nothing,
# and by then there is no way to debug it. selftest.py builds a complete DFU flash of a real
# shipped .spkg in fw-watch.py's own record format and asserts fw-scan.py recovers it.
if [ -f "$HERE/selftest.py" ]; then
  if OUT_ST="$(python3 "$HERE/selftest.py" 2>&1)"; then
    ok "$(echo "$OUT_ST" | grep -oE '[0-9]+ pass, [0-9]+ fail' | tail -1) — the decoder recovers a synthetic flash end to end"
  else
    bad "the firmware decoder self-test FAILED:"
    echo "$OUT_ST" | grep -E 'FAIL' | sed 's/^/        /'
  fi
else
  wrn "selftest.py missing — the decode path is unproven"
fi

echo
printf '\033[1m== %d pass, %d warn, %d fail\033[0m\n' "$PASS" "$WARN" "$FAIL"
if [ "$REBOOT" = 1 ]; then
  printf '\033[1;33m== a blacklist was written: REBOOT before the run so nothing is still holding a device.\033[0m\n'
fi
if [ "$FAIL" -gt 0 ]; then
  echo "== fix the FAILs tonight. On the day there is no second attempt at the flash."
  exit 1
fi
echo "== ready. Tomorrow:  sudo tools/capture/capture-firstcontact.sh ~/dlcap-firstcontact"
