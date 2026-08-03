#!/usr/bin/env python3
"""Statistical USB/URB audit for DisplayLink cold captures.

This reads pcapng usbmon records directly (no tshark text conversion), pairs submits with
completions by URB id, identifies the DisplayLink device, and reports the transport properties that
are easy to lose in a byte-only diff:

* requested/actual length and transfer-flag distributions;
* submit-to-complete latency and inter-submit cadence;
* exact outstanding-URB depth over time;
* video-frame URB grouping, launch/completion spans, errors, and cold-ARM detection;
* nearest head-0/head-1 frame launch skew.

Multiple captures may be supplied. With --json the full per-transfer/per-frame measurements are
emitted for cohort statistics in a follow-up analysis.

Usage:
  scripts/usb-session-stats.py CAPTURE.pcapng [...]
  scripts/usb-session-stats.py --json CAPTURE.pcapng [...] > sessions.json
"""

from __future__ import annotations

import argparse
import json
import mmap
import os
import statistics
import struct
import sys
from collections import Counter, defaultdict
from dataclasses import asdict, dataclass
from pathlib import Path
from typing import Iterator


PCAPNG_EPB = 0x00000006
PCAPNG_SHB = 0x0A0D0D0A
USBMON_HEADER = 64
USB_BULK = 3
SUBMIT = ord("S")
COMPLETE = ord("C")
ERROR = ord("E")
VIDEO_EPS = (0x08, 0x0A, 0x0B)
VFW_MAGIC = b"VFW2"
VFW_REC = "<QBBBBHBBqiiII8s"
VFW_REC_SIZE = struct.calcsize(VFW_REC)
LEGACY_REC = "<BBBBHqiiII"
LEGACY_REC_SIZE = struct.calcsize(LEGACY_REC)


@dataclass
class Event:
    urb_id: int
    kind: str
    xfer_type: int
    ep: int
    dev: int
    bus: int
    ts: float
    status: int
    length: int
    len_cap: int
    interval: int
    start_frame: int
    xfer_flags: int
    data: bytes


@dataclass
class Transfer:
    urb_id: int
    bus: int
    dev: int
    ep: int
    submit_ts: float
    complete_ts: float | None
    requested: int
    actual: int | None
    status: int | None
    latency_us: float | None
    xfer_flags: int
    data: bytes


@dataclass
class Frame:
    ep: int
    index: int
    start_ts: float
    end_submit_ts: float
    first_complete_ts: float | None
    last_complete_ts: float | None
    requested_bytes: int
    captured_bytes: int
    urb_count: int
    complete: bool
    arm_prefix: bool
    submit_span_us: float
    completion_span_us: float | None
    statuses: list[int | None]
    lengths: list[int]
    flags: list[int]
    urb_ids: list[int]
    first32: str


def _u32(buf: mmap.mmap, off: int) -> int:
    return struct.unpack_from("<I", buf, off)[0]


def iter_events(path: str) -> Iterator[Event]:
    """Yield usbmon events from pcapng, VFW2, or the older setup-less recorder format."""
    with open(path, "rb") as probe:
        magic = probe.read(4)
        if magic == VFW_MAGIC:
            yield from iter_vfw_events(probe)
            return
        if magic != struct.pack("<I", PCAPNG_SHB):
            probe.seek(0)
            yield from iter_legacy_events(probe)
            return

    with open(path, "rb") as fh, mmap.mmap(fh.fileno(), 0, access=mmap.ACCESS_READ) as mm:
        pos = 0
        size = len(mm)
        while pos + 12 <= size:
            block_type = _u32(mm, pos)
            block_len = _u32(mm, pos + 4)
            if block_len < 12 or pos + block_len > size:
                raise ValueError(f"invalid pcapng block at {pos}: length={block_len}")
            if block_type == PCAPNG_EPB and block_len >= 28 + USBMON_HEADER + 4:
                cap_len = _u32(mm, pos + 20)
                pkt = pos + 28
                if cap_len >= USBMON_HEADER and pkt + cap_len <= pos + block_len:
                    (urb_id, etype, xfer_type, ep, dev, bus, _flag_setup, flag_data,
                     ts_sec, ts_usec, status, length, len_cap) = struct.unpack_from(
                        "<QBBBBHbbqiiII", mm, pkt
                    )
                    interval, start_frame, xfer_flags = struct.unpack_from("<iiI", mm, pkt + 48)
                    avail = max(0, min(len_cap, cap_len - USBMON_HEADER))
                    data = bytes(mm[pkt + USBMON_HEADER:pkt + USBMON_HEADER + avail]) \
                        if flag_data == 0 and avail else b""
                    yield Event(
                        urb_id=urb_id,
                        kind=chr(etype),
                        xfer_type=xfer_type,
                        ep=ep,
                        dev=dev,
                        bus=bus,
                        ts=ts_sec + ts_usec / 1_000_000.0,
                        status=status,
                        length=length,
                        len_cap=len_cap,
                        interval=interval,
                        start_frame=start_frame,
                        xfer_flags=xfer_flags,
                        data=data,
                    )
            pos += block_len


def iter_legacy_events(fh) -> Iterator[Event]:
    """Yield the setup-less records written by ``capture-usbmon-session.py``.

    That recorder predates URB ids and the scheduling fields retained by pcapng/VFW2. Give each
    record an id of zero and report those unavailable fields as zero. Payload-oriented consumers
    such as the CP decryptor remain exact; transfer-pairing consumers should use VFW2 or pcapng.
    """
    while True:
        raw_len = fh.read(4)
        if not raw_len:
            return
        if len(raw_len) != 4:
            raise ValueError("truncated legacy record length")
        (record_len,) = struct.unpack("<I", raw_len)
        body = fh.read(record_len)
        if len(body) != record_len or record_len < LEGACY_REC_SIZE:
            raise ValueError("truncated legacy record body")
        (etype, xfer_type, ep, dev, bus, ts_sec, ts_usec, status, length,
         len_cap) = struct.unpack(LEGACY_REC, body[:LEGACY_REC_SIZE])
        data = body[LEGACY_REC_SIZE:LEGACY_REC_SIZE + len_cap]
        yield Event(
            urb_id=0,
            kind=chr(etype),
            xfer_type=xfer_type,
            ep=ep,
            dev=dev,
            bus=bus,
            ts=ts_sec + ts_usec / 1_000_000.0,
            status=status,
            length=length,
            len_cap=len_cap,
            interval=0,
            start_frame=0,
            xfer_flags=0,
            data=data,
        )


def iter_vfw_events(fh) -> Iterator[Event]:
    """Yield events from ``fw-watch.py``'s VFW2 stream.

    VFW2 retains the URB id, status, complete payload and control setup packet. It does not retain
    the three scheduling fields pcapng exposes, so those are reported as zero rather than guessed.
    """
    while True:
        raw_len = fh.read(4)
        if not raw_len:
            return
        if len(raw_len) != 4:
            raise ValueError("truncated VFW2 record length")
        (record_len,) = struct.unpack("<I", raw_len)
        body = fh.read(record_len)
        if len(body) != record_len or record_len < VFW_REC_SIZE:
            raise ValueError("truncated VFW2 record body")
        (urb_id, etype, xfer_type, ep, dev, bus, _flag_setup, _pad, ts_sec, ts_usec,
         status, length, len_cap, _setup) = struct.unpack(VFW_REC, body[:VFW_REC_SIZE])
        data = body[VFW_REC_SIZE:VFW_REC_SIZE + len_cap]
        yield Event(
            urb_id=urb_id,
            kind=chr(etype),
            xfer_type=xfer_type,
            ep=ep,
            dev=dev,
            bus=bus,
            ts=ts_sec + ts_usec / 1_000_000.0,
            status=status,
            length=length,
            len_cap=len_cap,
            interval=0,
            start_frame=0,
            xfer_flags=0,
            data=data,
        )


def looks_like_dl_frame(data: bytes) -> bool:
    if len(data) < 16:
        return False
    wire_len = int.from_bytes(data[2:4], "little")
    wire_type = int.from_bytes(data[4:8], "little")
    return wire_type in (0, 1, 2, 4) and wire_len + 4 <= max(len(data), 16 * 1024 * 1024)


def identify_devices(events: list[Event]) -> list[tuple[int, int]]:
    """Return every D6000 enumeration epoch in first-seen order.

    A physical cold plug changes the usbmon device number.  Selecting only the highest-volume
    bus/device silently attributes the pre-unplug stream to the new cold session, and also hides
    failed enumerations which reached CP but never submitted video.  Keep candidates which either
    carry video or have plausible DisplayLink frames on both control endpoints.
    """
    scores: Counter[tuple[int, int]] = Counter()
    dl_control_eps: dict[tuple[int, int], set[int]] = defaultdict(set)
    video_keys: set[tuple[int, int]] = set()
    first_seen: dict[tuple[int, int], float] = {}
    for e in events:
        if e.xfer_type != USB_BULK:
            continue
        key = (e.bus, e.dev)
        if e.kind == "S" and e.ep in VIDEO_EPS:
            scores[key] += max(e.length, len(e.data)) * 100
            video_keys.add(key)
            first_seen.setdefault(key, e.ts)
        if e.ep in (0x02, 0x84) and e.data and looks_like_dl_frame(e.data):
            scores[key] += 10_000 + len(e.data)
            dl_control_eps[key].add(e.ep)
            first_seen.setdefault(key, e.ts)
    candidates = [
        key for key in scores
        if key in video_keys or dl_control_eps[key] == {0x02, 0x84}
    ]
    if not candidates:
        raise ValueError("could not identify a DisplayLink device in the capture")
    return sorted(candidates, key=lambda key: (first_seen[key], key))


def pair_transfers(events: list[Event], bus: int, dev: int) -> list[Transfer]:
    pending: dict[int, Event] = {}
    out: list[Transfer] = []
    for e in events:
        if e.bus != bus or e.dev != dev or e.xfer_type != USB_BULK:
            continue
        if e.kind == "S":
            pending[e.urb_id] = e
        elif e.kind in ("C", "E"):
            s = pending.pop(e.urb_id, None)
            if s is None:
                continue
            # OUT data is captured at submit; IN data at completion.
            data = s.data or e.data
            out.append(Transfer(
                urb_id=e.urb_id,
                bus=bus,
                dev=dev,
                ep=s.ep,
                submit_ts=s.ts,
                complete_ts=e.ts,
                requested=s.length,
                actual=e.length,
                status=e.status,
                latency_us=(e.ts - s.ts) * 1_000_000.0,
                xfer_flags=s.xfer_flags,
                data=data,
            ))
    # Preserve submits whose completion fell beyond capture end.
    for s in pending.values():
        out.append(Transfer(
            urb_id=s.urb_id, bus=bus, dev=dev, ep=s.ep, submit_ts=s.ts, complete_ts=None,
            requested=s.length, actual=None, status=None, latency_us=None,
            xfer_flags=s.xfer_flags, data=s.data,
        ))
    out.sort(key=lambda x: (x.submit_ts, x.urb_id))
    return out


def arm_at_start(data: bytes) -> bool:
    """DLM frame 0 is 16 zero bytes followed by the first type=2 cold-arm record."""
    off = 16 if data.startswith(bytes(16)) else 0
    return len(data) >= off + 12 and int.from_bytes(data[off + 4:off + 8], "little") == 2


def assemble_frames(transfers: list[Transfer], ep: int) -> list[Frame]:
    xs = sorted((x for x in transfers if x.ep == ep and x.data), key=lambda x: x.submit_ts)
    frames: list[Frame] = []
    current: list[Transfer] = []
    for x in xs:
        current.append(x)
        # A transfer shorter than 65536 terminates the logical frame. A zero-length ZLP would too,
        # but has no captured data and is therefore handled only as an incomplete tail here.
        if x.requested < 65536:
            frames.append(make_frame(ep, len(frames), current, True))
            current = []
    if current:
        frames.append(make_frame(ep, len(frames), current, False))
    return frames


def make_frame(ep: int, index: int, xs: list[Transfer], complete: bool) -> Frame:
    cts = [x.complete_ts for x in xs if x.complete_ts is not None]
    first_data = xs[0].data
    return Frame(
        ep=ep,
        index=index,
        start_ts=xs[0].submit_ts,
        end_submit_ts=xs[-1].submit_ts,
        first_complete_ts=min(cts) if cts else None,
        last_complete_ts=max(cts) if cts else None,
        requested_bytes=sum(x.requested for x in xs),
        captured_bytes=sum(len(x.data) for x in xs),
        urb_count=len(xs),
        complete=complete,
        arm_prefix=arm_at_start(first_data),
        submit_span_us=(xs[-1].submit_ts - xs[0].submit_ts) * 1_000_000.0,
        completion_span_us=((max(cts) - min(cts)) * 1_000_000.0) if len(cts) > 1 else 0.0 if cts else None,
        statuses=[x.status for x in xs],
        lengths=[x.requested for x in xs],
        flags=[x.xfer_flags for x in xs],
        urb_ids=[x.urb_id for x in xs],
        first32=first_data[:32].hex(),
    )


def quantiles(values: list[float]) -> dict[str, float] | None:
    v = sorted(values)
    if not v:
        return None
    def q(frac: float) -> float:
        if len(v) == 1:
            return v[0]
        p = frac * (len(v) - 1)
        lo = int(p)
        hi = min(lo + 1, len(v) - 1)
        return v[lo] + (v[hi] - v[lo]) * (p - lo)
    return {
        "min": v[0], "p05": q(0.05), "p50": q(0.50), "p95": q(0.95),
        "max": v[-1], "mean": statistics.fmean(v),
    }


def max_inflight(events: list[Event], bus: int, dev: int, ep: int) -> int:
    depth = peak = 0
    outstanding: set[int] = set()
    for e in events:
        if e.bus != bus or e.dev != dev or e.xfer_type != USB_BULK or e.ep != ep:
            continue
        if e.kind == "S":
            outstanding.add(e.urb_id)
        elif e.kind in ("C", "E"):
            outstanding.discard(e.urb_id)
        depth = len(outstanding)
        peak = max(peak, depth)
    return peak


def endpoint_summary(events: list[Event], transfers: list[Transfer], bus: int, dev: int, ep: int) -> dict:
    xs = sorted((x for x in transfers if x.ep == ep), key=lambda x: x.submit_ts)
    gaps = [(b.submit_ts - a.submit_ts) * 1_000_000.0 for a, b in zip(xs, xs[1:])]
    lats = [x.latency_us for x in xs if x.latency_us is not None]
    return {
        "endpoint": ep,
        "transfers": len(xs),
        "bytes_requested": sum(x.requested for x in xs),
        "lengths": dict(sorted(Counter(x.requested for x in xs).items())),
        "actual_lengths": dict(sorted(Counter(x.actual for x in xs if x.actual is not None).items())),
        "statuses": dict(sorted(Counter(x.status for x in xs).items(), key=lambda kv: str(kv[0]))),
        "flags": {f"0x{k:08x}": v for k, v in sorted(Counter(x.xfer_flags for x in xs).items())},
        "latency_us": quantiles(lats),
        "submit_gap_us": quantiles(gaps),
        "max_inflight": max_inflight(events, bus, dev, ep),
    }


def nearest_head_skew(frames0: list[Frame], frames1: list[Frame]) -> list[float]:
    if not frames0 or not frames1:
        return []
    t1 = [f.start_ts for f in frames1]
    return [min(t1, key=lambda t: abs(t - f.start_ts)) - f.start_ts for f in frames0]


def analyse_device(path: str, events: list[Event], bus: int, dev: int,
                   epoch: int, epoch_count: int) -> dict:
    device_events = [e for e in events if e.bus == bus and e.dev == dev]
    first_ts = min(e.ts for e in device_events)
    last_ts = max(e.ts for e in device_events)
    transfers = pair_transfers(events, bus, dev)
    eps = sorted({x.ep for x in transfers})
    frames = {ep: assemble_frames(transfers, ep) for ep in VIDEO_EPS}
    second_ep = 0x0A if frames[0x0A] else 0x0B
    skew = nearest_head_skew(frames[0x08], frames[second_ep])
    return {
        "path": os.path.abspath(path),
        "epoch": epoch,
        "epoch_count": epoch_count,
        "bus": bus,
        "device": dev,
        "event_count": len(device_events),
        "first_ts": first_ts,
        "last_ts": last_ts,
        "duration_s": last_ts - first_ts,
        "endpoints": [endpoint_summary(events, transfers, bus, dev, ep) for ep in eps],
        "frames": {f"0x{ep:02x}": [asdict(f) for f in fs] for ep, fs in frames.items()},
        "head1_minus_head0_start_us": [x * 1_000_000.0 for x in skew],
        "head_skew_us": quantiles([abs(x) * 1_000_000.0 for x in skew]),
    }


def analyse(path: str) -> list[dict]:
    events = list(iter_events(path))
    devices = identify_devices(events)
    return [
        analyse_device(path, events, bus, dev, epoch, len(devices))
        for epoch, (bus, dev) in enumerate(devices, 1)
    ]


def fmt_q(q: dict[str, float] | None) -> str:
    if not q:
        return "n/a"
    return f"p50={q['p50']:.1f} p95={q['p95']:.1f} mean={q['mean']:.1f} max={q['max']:.1f}"


def print_report(s: dict) -> None:
    suffix = f" [epoch {s['epoch']}/{s['epoch_count']}]" if s["epoch_count"] > 1 else ""
    print(f"\n=== {s['path']}{suffix} ===")
    print(f"DisplayLink device: bus {s['bus']} device {s['device']}  "
          f"({s['event_count']} events, {s['duration_s']:.3f}s)")
    for e in s["endpoints"]:
        ep = e["endpoint"]
        print(f"  EP{ep:02x}: n={e['transfers']} bytes={e['bytes_requested']} max-inflight={e['max_inflight']}")
        print(f"        lengths={e['lengths']} status={e['statuses']} flags={e['flags']}")
        print(f"        latency-us {fmt_q(e['latency_us'])}; submit-gap-us {fmt_q(e['submit_gap_us'])}")
    for ep in VIDEO_EPS:
        fs = s["frames"][f"0x{ep:02x}"]
        print(f"  EP{ep:02x} logical frames: {len(fs)}")
        for f in fs[:12]:
            bad = [x for x in f["statuses"] if x not in (0, None)]
            print(f"        #{f['index']:03d} bytes={f['requested_bytes']:7d} urbs={f['urb_count']:2d} "
                  f"submit-span={f['submit_span_us']:8.1f}us complete-span="
                  f"{str(None if f['completion_span_us'] is None else round(f['completion_span_us'], 1)):>8} "
                  f"arm={f['arm_prefix']} complete={f['complete']} bad={bad} lengths={f['lengths']}")
        if len(fs) > 12:
            print(f"        ... {len(fs)-12} more")
    print(f"  |head1-head0 nearest frame start| us: {fmt_q(s['head_skew_us'])}")


def pearson(xs: list[float], ys: list[float]) -> float | None:
    if len(xs) < 3 or len(xs) != len(ys):
        return None
    mx, my = statistics.fmean(xs), statistics.fmean(ys)
    dx, dy = [x - mx for x in xs], [y - my for y in ys]
    den = sum(x * x for x in dx) * sum(y * y for y in dy)
    if den <= 0:
        return None
    return sum(x * y for x, y in zip(dx, dy)) / den ** 0.5


def print_cohort(results: list[dict]) -> None:
    """Cross-session transport distributions, excluding tiny side-band EP08 messages."""
    print("\n================ CROSS-SESSION VIDEO COHORT ================")
    print(f"captures={len(results)}")
    for ep in VIDEO_EPS:
        key = f"0x{ep:02x}"
        frames = [f for r in results for f in r["frames"][key]
                  if f["complete"] and f["requested_bytes"] >= 10_000]
        arm = [f for f in frames if f["arm_prefix"]]
        ordinary = [f for f in frames if not f["arm_prefix"]]
        gaps = []
        for r in results:
            fs = [f for f in r["frames"][key]
                  if f["complete"] and f["requested_bytes"] >= 10_000]
            gaps.extend((b["start_ts"] - a["start_ts"]) * 1_000_000.0
                        for a, b in zip(fs, fs[1:]))
        max_depth = []
        for r in results:
            e = next((e for e in r["endpoints"] if e["endpoint"] == ep), None)
            if e:
                max_depth.append(e["max_inflight"])
        sizes = [float(f["requested_bytes"]) for f in ordinary]
        completion = [float(f["completion_span_us"]) for f in ordinary
                      if f["completion_span_us"] is not None]
        paired_sizes = [float(f["requested_bytes"]) for f in ordinary
                        if f["completion_span_us"] is not None]
        corr = pearson(paired_sizes, completion)
        tails = Counter(f["lengths"][-1] for f in ordinary if f["lengths"])
        urb_counts = Counter(f["urb_count"] for f in ordinary)
        bad = sum(any(s not in (0, None) for s in f["statuses"]) for f in frames)
        print(f"EP{ep:02x}: video-frames={len(frames)} ordinary={len(ordinary)} arm={len(arm)} "
              f"bad-frames={bad} max-inflight/session={max_depth}")
        print(f"  arm sizes={[f['requested_bytes'] for f in arm]} "
              f"urb-shapes={[f['lengths'] for f in arm]}")
        print(f"  ordinary size bytes: {fmt_q(quantiles(sizes))}")
        print(f"  frame submit-span us: "
              f"{fmt_q(quantiles([f['submit_span_us'] for f in ordinary]))}")
        print(f"  frame completion-span us: {fmt_q(quantiles(completion))}")
        print(f"  frame-start cadence us: {fmt_q(quantiles(gaps))}")
        print(f"  URBs/frame={dict(sorted(urb_counts.items()))} final-URB top="
              f"{dict(tails.most_common(12))}")
        print(f"  corr(frame bytes, completion span)="
              f"{'n/a' if corr is None else f'{corr:+.4f}'}")
    skews = [abs(x) for r in results for x in r["head1_minus_head0_start_us"]]
    print(f"nearest cross-head launch skew us: {fmt_q(quantiles(skews))}")


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("captures", nargs="+")
    ap.add_argument("--json", action="store_true")
    args = ap.parse_args()
    results = []
    for path in args.captures:
        try:
            results.extend(analyse(path))
        except Exception as exc:
            print(f"{path}: {exc}", file=sys.stderr)
            return 2
    if args.json:
        json.dump(results, sys.stdout, indent=2)
        print()
    else:
        for result in results:
            print_report(result)
        if len(results) > 1:
            print_cohort(results)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
