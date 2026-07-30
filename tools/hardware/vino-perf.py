#!/usr/bin/env python3
"""Repeatable performance harness for vino.

Everything measured here is normalised per second, so runs are comparable across builds. The point
is to make a change (`FRAME_PERIOD_MS`, the snapshot path, ...) *provable* rather than lost in
whatever the desktop happened to be doing -- earlier attempts to score the band-major snapshot
rewrite failed because the sample noise was larger than the effect.

What it reports, per run:

  * delivered frames/s and MB/s **per head**, taken from the wire (EP08 = head 0, EP0b = head 1).
    Frames are URB bursts: intra-frame URBs are microseconds apart while frames are milliseconds
    apart, so a gap threshold separates them cleanly.
  * KWin's main-thread CPU, split user/system.
  * Machine-wide busy CPU in core-equivalents, from /proc/stat. This is the only trustworthy
    absolute cost figure -- see `kworker_ticks`.
  * vino's *named* encode-workqueue CPU.

Frames/s is the number that answers "are we actually at 120 Hz"; the CPU columns are what it cost.

Usage:
    sudo tools/hardware/vino-perf.py --secs 30                    # measure whatever is on screen
    sudo tools/hardware/vino-perf.py --secs 30 --load             # drive a deterministic full-motion clip
    sudo tools/hardware/vino-perf.py --secs 30 --load --tag base  # label the run

⚠ Run the *same* `--load` for every build being compared; an idle desktop cannot distinguish a
throughput change from a quiet moment.
"""

from __future__ import annotations

import argparse
import collections
import os
import re
import shutil
import signal
import struct
import subprocess
import sys
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
CAPTURE = HERE / "capture-usbmon-session.py"
CLIP = HERE / "perf-load-2560x1440-60.mp4"

# usbmon record layout written by capture-usbmon-session.py.
REC = "<IBBBBHqiiII"
RECSZ = struct.calcsize(REC)

# EP -> head. Video endpoints only; EP02/EP84 are the control plane.
VIDEO_EPS = {0x08: 0, 0x0B: 1}
# A frame is a burst of URBs; anything idle longer than this starts a new frame. Intra-frame URBs
# land microseconds apart and even a 240 Hz head leaves >4 ms between frames.
FRAME_GAP_S = 0.002


def cpu_ticks(pid: int, tid: int | None = None) -> tuple[int, int]:
    """(utime, stime) in clock ticks for a process or one thread."""
    p = f"/proc/{pid}/task/{tid}/stat" if tid else f"/proc/{pid}/stat"
    try:
        raw = re.sub(r"\(.*\)", "X", Path(p).read_text()).split()
    except OSError:
        return (0, 0)
    return (int(raw[13]), int(raw[14]))


def machine_busy() -> tuple[int, int]:
    """(total, idle) jiffies across all CPUs, from /proc/stat."""
    with open("/proc/stat") as fh:
        for line in fh:
            if line.startswith("cpu "):
                v = [int(x) for x in line.split()[1:9]]
                return sum(v), v[3] + v[4]
    raise RuntimeError("no 'cpu ' line in /proc/stat")


def kworker_ticks() -> dict[int, int]:
    """Ticks per kworker PID running vino's *named* encode workqueue (`vino_encode`).

    Returned per PID, not summed: kworkers are created and reaped constantly, so summing live
    workers at two instants can go *backwards* when one exits mid-run.

    ⚠ This is a LOWER BOUND on vino's CPU, and it used to be a wild over-estimate. Matching
    `events_unbound` swept in every other subsystem sharing the system-wide unbound pool, which
    reported 641% where /proc/stat put vino's real cost at ~267%. vino's scanout worker also runs on
    the shared `system()` queue, whose kworkers are named `-events` and cannot be attributed to a
    driver at all -- so no per-kworker sum can be the whole answer. Compare builds with the
    machine-busy figure and an otherwise-quiet desktop.
    """
    out: dict[int, int] = {}
    for entry in Path("/proc").iterdir():
        if not entry.name.isdigit():
            continue
        try:
            comm = (entry / "comm").read_text().strip()
        except OSError:
            continue
        if "vino" not in comm:
            continue
        u, s = cpu_ticks(int(entry.name))
        out[int(entry.name)] = u + s
    return out


def kworker_delta(before: dict[int, int], after: dict[int, int]) -> int:
    """Ticks consumed between two samples, tolerating workers appearing and exiting.

    A PID present only in `after` is counted in full (it was spawned during the run); one that has
    vanished contributes what we last saw, which is the best available lower bound.
    """
    total = 0
    for pid, end in after.items():
        total += max(0, end - before.get(pid, 0))
    return total


def kwin_pid() -> int | None:
    try:
        return int(subprocess.check_output(["pgrep", "-x", "kwin_wayland"]).split()[0])
    except Exception:
        return None


def parse_mon(path: Path) -> dict[int, list[tuple[float, int]]]:
    """(timestamp, urb_length) per video endpoint, submissions only."""
    data = path.read_bytes()
    out: dict[int, list[tuple[float, int]]] = collections.defaultdict(list)
    off = 0
    while off + 4 <= len(data):
        (rl,) = struct.unpack_from("<I", data, off)
        if rl == 0 or off + 4 + rl > len(data):
            break
        _, typ, _xfer, ep, _dev, _bus, ts, tus, _st, lurb, _lcap = struct.unpack_from(
            REC, data, off
        )
        if chr(typ) == "S" and ep in VIDEO_EPS:
            out[ep].append((ts + tus / 1e6, lurb))
        off += 4 + rl
    return out


def frames_from_bursts(samples: list[tuple[float, int]]) -> tuple[int, int, list[float]]:
    """(frame count, total bytes, frame start times) by grouping URBs into bursts."""
    if not samples:
        return (0, 0, [])
    samples.sort()
    starts = [samples[0][0]]
    total = samples[0][1]
    prev = samples[0][0]
    for ts, ln in samples[1:]:
        if ts - prev > FRAME_GAP_S:
            starts.append(ts)
        total += ln
        prev = ts
    return (len(starts), total, starts)


def worst_gaps(starts: list[float], n: int = 4) -> list[tuple[float, float]]:
    """The largest frame intervals as (seconds into the run, interval ms).

    Position matters: outliers clustered at the start are the load settling, while outliers spread
    through the run are real stalls.
    """
    if len(starts) < 3:
        return []
    t0 = starts[0]
    gaps = [
        (starts[i] - t0, (starts[i + 1] - starts[i]) * 1000) for i in range(len(starts) - 1)
    ]
    return sorted(gaps, key=lambda g: -g[1])[:n]


def smoothness(starts: list[float]) -> dict[str, float]:
    """Frame-interval spread. For an interactive desktop this matters more than peak throughput:
    a steady 60 fps looks better than an average 80 fps that stutters."""
    if len(starts) < 8:
        return {}
    iv = sorted((starts[i + 1] - starts[i]) * 1000 for i in range(len(starts) - 1))
    n = len(iv)
    med = iv[n // 2]
    mean = sum(iv) / n
    var = sum((x - mean) ** 2 for x in iv) / n
    return {
        "median": med,
        "p95": iv[int(0.95 * n)],
        "p99": iv[int(0.99 * n)],
        "worst": iv[-1],
        "jitter": var ** 0.5,
        # fraction of intervals more than 1.5x the median -- a visible hitch
        "hitches": 100.0 * sum(1 for x in iv if x > 1.5 * med) / n,
    }


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--secs", type=float, default=30.0)
    ap.add_argument("--bus", type=int, default=2)
    ap.add_argument("--load", action="store_true", help="play the deterministic clip fullscreen")
    ap.add_argument(
        "--windowed",
        action="store_true",
        help="run the load in a window rather than fullscreen. Fullscreen video is the worst case "
        "(full-frame damage every frame); a window damages only part of the plane, which is much "
        "closer to normal desktop interaction.",
    )
    ap.add_argument("--tag", default="", help="label for the run")
    ap.add_argument(
        "--screens",
        default="2",
        help="comma-separated KWin screen indices to place one load on each (e.g. '1,2' to drive "
        "both dock heads). The real desktop configuration is two heads, which halves the per-head "
        "budget, so a single-head number overstates what is achievable.",
    )
    ap.add_argument(
        "--screen",
        type=int,
        default=2,
        help="KWin screen index to move the load onto (a dock head; 0 disables). Wayland clients "
        "cannot place themselves, so the window is moved with KWin's 'Window to Screen N' "
        "shortcut after launch.",
    )
    args = ap.parse_args()

    if os.geteuid() != 0:
        sys.exit("run with sudo (usbmon needs root)")
    if not CAPTURE.exists():
        sys.exit(f"missing {CAPTURE}")
    # usbmon is not autoloaded and does not survive a reboot; without it the capture silently
    # produces no file at all.
    if not Path("/dev/usbmon0").exists():
        subprocess.run(["modprobe", "usbmon"], check=False)
        if not Path("/dev/usbmon0").exists():
            sys.exit("usbmon unavailable: modprobe usbmon failed")

    kpid = kwin_pid()
    if kpid is None:
        print("[!] kwin_wayland not found -- CPU columns will be blank")

    mon = Path(f"/tmp/vino-perf-{int(time.time())}.mon")
    player = None
    if args.load:
        if not CLIP.exists():
            sys.exit(f"missing load clip {CLIP}; generate it with ffmpeg (see docs)")
        if not shutil.which("mpv"):
            sys.exit("mpv not installed")
        # Run as the desktop user; root has no Wayland connection.
        screens = [int(x) for x in args.screens.split(",") if x.strip()]
        players = []
        for sc in screens:
            players.append(subprocess.Popen(
                ["sudo", "-u", "#1000", "env", "XDG_RUNTIME_DIR=/run/user/1000",
                 "WAYLAND_DISPLAY=wayland-0", "mpv", "--really-quiet", "--loop",
                 "--no-audio", "--profile=low-latency",
                 *(["--autofit=40%x40%"] if args.windowed else ["--fs"]), str(CLIP)],
                stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
            ))
            time.sleep(4)
            # Each new window takes focus, so move it before starting the next one.
            subprocess.run(
                ["sudo", "-u", "#1000", "env", "XDG_RUNTIME_DIR=/run/user/1000",
                 "gdbus", "call", "--session", "--dest", "org.kde.kglobalaccel",
                 "--object-path", "/component/kwin", "--method",
                 "org.kde.kglobalaccel.Component.invokeShortcut",
                 f"Window to Screen {sc}"],
                stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=False,
            )
            time.sleep(2)
        player = players[0] if players else None
        time.sleep(3)  # let both settle and scanout reach steady state

    cap = subprocess.Popen(
        [sys.executable, str(CAPTURE), "--bus", str(args.bus), "--out", str(mon),
         "--snap", "0", "--secs", str(args.secs + 5)],
        stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
    )
    time.sleep(1.0)

    k0 = cpu_ticks(kpid, kpid) if kpid else (0, 0)
    w0 = kworker_ticks()
    m0 = machine_busy()
    t0 = time.time()
    time.sleep(args.secs)
    elapsed = time.time() - t0
    k1 = cpu_ticks(kpid, kpid) if kpid else (0, 0)
    w1 = kworker_ticks()
    m1 = machine_busy()

    cap.send_signal(signal.SIGINT)
    try:
        cap.wait(timeout=20)
    except subprocess.TimeoutExpired:
        cap.kill()
    if player:
        subprocess.run(["pkill", "-f", "perf-load-2560x1440"], check=False)

    hz = os.sysconf("SC_CLK_TCK")
    if not mon.exists():
        sys.exit("capture produced no file -- is usbmon loaded and the bus correct?")
    eps = parse_mon(mon)
    mon.unlink(missing_ok=True)

    tag = f" [{args.tag}]" if args.tag else ""
    print(f"\n=== vino perf{tag} -- {elapsed:.1f}s, load={'yes' if args.load else 'no'} ===\n")
    print(f"{'head':<6}{'frames/s':>10}{'MB/s':>10}{'URBs/s':>10}")
    any_traffic = False
    smooth: dict[int, dict[str, float]] = {}
    starts_by_head: dict[int, list[float]] = {}
    for ep, head in sorted(VIDEO_EPS.items(), key=lambda kv: kv[1]):
        frames, total, starts = frames_from_bursts(eps.get(ep, []))
        urbs = len(eps.get(ep, []))
        if urbs:
            any_traffic = True
        smooth[head] = smoothness(starts)
        starts_by_head[head] = starts
        print(f"{head:<6}{frames/elapsed:>10.1f}{total/elapsed/1e6:>10.1f}{urbs/elapsed:>10.1f}")
    for head, m in smooth.items():
        if not m:
            continue
        print(
            f"\nhead {head} frame interval (ms): median {m['median']:.1f}  p95 {m['p95']:.1f}  "
            f"p99 {m['p99']:.1f}  worst {m['worst']:.1f}"
        )
        print(f"  jitter (sd) {m['jitter']:.1f} ms   hitches >1.5x median: {m['hitches']:.1f}%")
        wg = worst_gaps(starts_by_head.get(head, []))
        if wg:
            print("  worst gaps: " + ", ".join(f"{ms:.0f}ms @ t+{t:.1f}s" for t, ms in wg))
    if not any_traffic:
        print("  (no video traffic seen -- is vino bound and scanning out?)")

    print()
    ku = (k1[0] - k0[0]) / hz / elapsed * 100
    ks = (k1[1] - k0[1]) / hz / elapsed * 100
    kw = kworker_delta(w0, w1) / hz / elapsed * 100
    tot, idle = m1[0] - m0[0], m1[1] - m0[1]
    cores = (tot - idle) / tot * os.cpu_count() if tot else 0.0
    print(f"kwin main thread : {ku:5.1f}% user  {ks:5.1f}% sys   (% of one core)")
    print(f"machine busy     : {cores:5.2f} cores  ({(tot - idle) / tot * 100:4.1f}% of {os.cpu_count()})")
    print(f"vino_encode wq   : {kw:5.1f}% (lower bound; shared-queue work is unattributable)")
    print("\ncompare runs only against the same --load setting, on an otherwise-quiet desktop.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
