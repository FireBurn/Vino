#!/usr/bin/env python3
"""Generate 4K HDR content that drives the codec's AC coefficients to their ceiling.

## Why this exists

`docs/hdr.md` §0.2 settled the DL7400's 10-bit **DC** escape ceiling at 12. It could not settle
the **AC** ceilings, and the 2026-08-05 capture shows why twice over:

1. Windows tone-maps to the sink's declared peak, and that sink declared 301.8 cd/m2. Every
   contrast in the content was compressed to under a third of the 10-bit range before the codec
   saw it.
2. ⭐ The one segment written to stress the codec, `seg_detail`, used a **16 px** checkerboard.
   The codec transforms **8x8** blocks, so every block sat entirely inside one cell and was
   perfectly flat. It produced 3181 strips and **zero** AC coefficients. A pattern only makes AC
   if it varies *within* a block.

⛔ **And point 1 turns out not to matter.** Running these patterns through a forward model of the
codec (`quantize` clamps every AC coefficient to +/-2047, so category 11 is the most any content
can demand) says a **2 px grating** reaches category 11 on *every* HDR sink, including the 302
cd/m2 panel that was blamed:

    peak  302 cd/m2 -> code  637 -> max luma AC 1274   category 11
    peak 1000 cd/m2 -> code  769 -> max luma AC 1538   category 11
    the 8-bit SDR twin  -> code  255 -> max luma AC  510   category  9

So the sink's brightness was never the blocker -- the 16 px cells were. A brighter or larger sink
adds headroom and a new timing, not the measurement.

⭐ The last line is what makes this decisive: the SDR twin of the same picture lands at **exactly**
the 8-bit ceiling, so the pair is matched. If the 10-bit half codes 1274 while the 8-bit half
saturates at 511, the AC ceiling scales with depth. If both saturate, it does not.

The patterns are achromatic on purpose: 4:2:0 chroma subsampling would smear a 1 px colour pattern,
while luma survives a lossless encode exactly -- and luma AC is the coefficient in question.

`check8` is the deliberate control: an 8 px checkerboard aligned to the block grid, which should
produce no AC at all. If it does produce some, the picture is not landing 1:1 on device pixels and
the whole run is void -- which is a far better check than trusting a scaling setting.

## Reading the result

    tools/codec/depth-probe.py <capture> --device N --ep 8 --row 720 --since ... --until ...

then the AC census in `docs/hdr.md` §0.5. What is wanted is a luma AC coefficient of magnitude
>= 512, i.e. category 10 -- above the 8-bit ceiling of 9. If one appears and the strip still
decodes coherently, the ceiling scales with depth; if the largest magnitude piles up at exactly
511, the encoder is saturating at the 8-bit ceiling and it does not.

Needs Linux, ffmpeg and numpy.  ⚠ Run it with `scratchpad/venv-np/bin/python`: numpy 1.26.4 on
Python 3.14 corrupts array locals of 256 KiB and up, and a 4K frame is 24 MB.
"""

import argparse
import importlib.util
import json
import os
import sys

import numpy as np

HERE = os.path.dirname(os.path.abspath(__file__))

# The sibling generator owns the colour maths, the PNG writers and the encoder invocations, all of
# which are identical here. It is a script with a hyphen in its name, so it cannot be imported by
# name; loading it by path is honest and beats a second copy that can drift.
_spec = importlib.util.spec_from_file_location(
    'hdrgen', os.path.join(HERE, 'make-hdr-patterns.py'))
hdrgen = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(hdrgen)

# ---------------------------------------------------------------- geometry ----

# Default is the 4K TV's native grid. `--size 1440p` gives the same patterns for the MSI panels,
# which the model says work just as well -- a 4K clip shown on a 1440p panel would be RESAMPLED,
# and a resampled 2 px grating is not a 2 px grating.
SIZES = {'4k': (3840, 2160), '1440p': (2560, 1440)}
W, H = SIZES['4k']
CELL = 40                      # marker cell; divides 3840/2160 and 2560/1440
FPS = 10                       # every decoded frame is a full repaint, which is the point
SECONDS = 4                    # per segment

BLOCK = 8                      # the codec's transform block, in pixels
SDR_WHITE = hdrgen.SDR_WHITE_NITS

# The bright level of the high-contrast patterns. 1000 cd/m2 is the standard mastering peak, so a
# tone-mapper passes it through with the least roll-off of any value above the sink's own peak;
# 10000 is included once to see what the roll-off does rather than to guess.
FULL = 1000.0
OVER = 10000.0


# -------------------------------------------------------------- patterns ----

def flat(n: float) -> np.ndarray:
    img = np.empty((H, W, 3), dtype=np.float64)
    img[:] = n
    return img


def checker(cell: int, lo: float, hi: float) -> np.ndarray:
    """Square checkerboard of `cell`-pixel squares."""
    ys = (np.arange(H) // cell)[:, None]
    xs = (np.arange(W) // cell)[None, :]
    v = np.where(((ys + xs) % 2) == 0, hi, lo)
    return np.repeat(v[:, :, None], 3, axis=2)


def vgrating(cell: int, lo: float, hi: float) -> np.ndarray:
    """Vertical stripes: pure horizontal frequency, no vertical component."""
    xs = (np.arange(W) // cell)[None, :]
    v = np.where((xs % 2) == 0, hi, lo)
    v = np.repeat(v, H, axis=0)
    return np.repeat(v[:, :, None], 3, axis=2)


def hgrating(cell: int, lo: float, hi: float) -> np.ndarray:
    """Horizontal stripes: pure vertical frequency. A DL7400 strip is 8 rows, so a 1 px
    grating puts the whole of its energy inside one strip's vertical extent."""
    ys = (np.arange(H) // cell)[:, None]
    v = np.where((ys % 2) == 0, hi, lo)
    v = np.repeat(v, W, axis=1)
    return np.repeat(v[:, :, None], 3, axis=2)


def noise() -> np.ndarray:
    """Fixed-seed per-pixel noise over the full range.

    Unlike a checkerboard this puts energy in *every* sub-band at once, so one segment covers the
    whole coefficient space rather than one frequency of it. The seed is fixed so a decoded strip
    can be checked against the source rather than merely looked at.
    """
    rng = np.random.default_rng(0x5EED)
    v = rng.integers(0, 2, size=(H, W)).astype(np.float64) * FULL
    return np.repeat(v[:, :, None], 3, axis=2)


def step_edges() -> np.ndarray:
    """Vertical hard edges every 8 px, stepping the contrast across the PQ range.

    The checkerboards give one AC magnitude, repeated. This gives a *spread*: each 128 px strip
    holds a different step height, so a single decoded strip shows which magnitudes the coder
    reaches and how it codes each -- the difference between "the ceiling is 11" and "the ceiling
    is 9 and everything above it saturates" is visible as a pile-up at one value.
    """
    img = np.zeros((H, W, 3), dtype=np.float64)
    steps = W // (BLOCK * 2)
    for i in range(steps):
        # PQ code 0..1 across the screen, converted back to the luminance that codes to it.
        code = (i + 1) / steps
        hi = float(hdrgen.pq_decode(np.array([code]))[0])
        x0 = i * BLOCK * 2
        img[:, x0:x0 + BLOCK, :] = 0.0
        img[:, x0 + BLOCK:x0 + BLOCK * 2, :] = hi
    return img


# Ordered so the decisive segments come first: if the run has to be cut short, segments 01-03 are
# the experiment and everything after is corroboration. `expect` is the highest luma AC category
# the forward model says the picture demands of the coder at 10 bits, against an 8-bit ceiling of
# 9 -- so anything marked 10 or 11 is asking for something the SDR codebook cannot express.
SEGMENTS: list[tuple[str, str, int]] = [
    ('black', 'flat 0 -- DC reference and a clean segment boundary', 0),
    ('vlines2_full', '2 px vertical grating 0 <-> 1000 cd/m2 -- THE test, level-2 band', 11),
    ('hlines2_full', '2 px horizontal grating 0 <-> 1000 cd/m2 -- same, vertical frequency', 11),
    ('vlines2_sdr', '2 px vertical grating 0 <-> 203 cd/m2 -- the matched in-SDR-range control', 9),
    ('vlines4_full', '4 px vertical grating -- the coarsest AC band (level 3)', 11),
    ('vlines1_full', '1 px vertical grating -- the finest horizontal band (level 1 HL)', 10),
    ('check1_full', '1 px checkerboard -- the finest band of all (level 1 HH)', 10),
    ('check2_full', '2 px checkerboard -- level 2, both directions at once', 10),
    ('check8_control', '8 px checkerboard ON the block grid -- MUST produce NO AC', 0),
    ('noise', 'fixed-seed per-pixel noise at 1000 cd/m2 -- every sub-band at once', 10),
    ('step_edges', '8 px edges stepping the whole PQ range -- a spread of magnitudes', 11),
    ('black_end', 'flat 0 -- marks the end of the sequence', 0),
]


BUILDERS = {
    'black': lambda: flat(0.0),
    'black_end': lambda: flat(0.0),
    'vlines2_full': lambda: vgrating(2, 0.0, FULL),
    'hlines2_full': lambda: hgrating(2, 0.0, FULL),
    'vlines2_sdr': lambda: vgrating(2, 0.0, SDR_WHITE),
    'vlines4_full': lambda: vgrating(4, 0.0, FULL),
    'vlines1_full': lambda: vgrating(1, 0.0, FULL),
    'check1_full': lambda: checker(1, 0.0, FULL),
    'check2_full': lambda: checker(2, 0.0, FULL),
    'check8_control': lambda: checker(BLOCK, 0.0, FULL),
    'noise': noise,
    'step_edges': step_edges,
}


def build(idx: int) -> np.ndarray:
    name = SEGMENTS[idx][0]
    img = BUILDERS[name]()
    # The marker is the only thing that identifies a decoded strip's segment without trusting the
    # wall clock. It overwrites the top row of cells, which is 40 of 2160 rows.
    hdrgen.CELL = CELL
    hdrgen.SDR_WHITE_NITS = SDR_WHITE
    hdrgen.draw_marker(img, idx, 8)
    return img


# ----------------------------------------------------------------- output ----

def frames(out_dir: str, hdr: bool):
    """Feed raw frames to ffmpeg's stdin.

    Written rather than parked on disk: 480 4K frames is 24 GB as 16-bit PNG and the encoder
    reads them exactly once.
    """
    def feeder(stdin):
        for idx in range(len(SEGMENTS)):
            nits = build(idx)
            if hdr:
                buf = hdrgen.to_hdr_png16(nits).tobytes()
            else:
                buf = hdrgen.to_sdr_png8(nits).tobytes()
            for _ in range(FPS * SECONDS):
                stdin.write(buf)
            print(f'    segment {idx:02d} {SEGMENTS[idx][0]}', flush=True)
    return feeder


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument('--out', required=True)
    ap.add_argument('--size', choices=sorted(SIZES), default='4k',
                    help='4k for the TV (default), 1440p for the MSI panels')
    ap.add_argument('--skip-encode', action='store_true')
    ap.add_argument('--refs-only', action='store_true',
                    help='write the reference PNGs and stop')
    args = ap.parse_args()

    global W, H
    W, H = SIZES[args.size]

    out = args.out
    ref = os.path.join(out, 'ref')
    os.makedirs(ref, exist_ok=True)

    print('reference pictures')
    for idx, (name, _desc, _e) in enumerate(SEGMENTS):
        nits = build(idx)
        # Only a crop: a full 4K 16-bit reference is 50 MB each and nothing reads more than a
        # corner of it. 512x256 covers four strips across and 32 bands down.
        hdrgen.write_png16(os.path.join(ref, f'ac{idx:02d}-{name}.hdr.png'),
                           hdrgen.to_hdr_png16(nits)[0:256, 0:512])
        hdrgen.write_png8(os.path.join(ref, f'ac{idx:02d}-{name}.sdr.png'),
                          hdrgen.to_sdr_png8(nits)[0:256, 0:512])
        print(f'  {idx:02d} {name}')

    manifest = {
        'size': args.size, 'width': W, 'height': H, 'fps': FPS, 'seconds_per_segment': SECONDS,
        'block_px': BLOCK, 'full_nits': FULL, 'over_nits': OVER,
        'segments': [{'index': i, 'name': n, 'description': d,
                      'expect_max_luma_ac_category_10bit': e}
                     for i, (n, d, e) in enumerate(SEGMENTS)],
        'note': ('Achromatic by design: 4:2:0 subsampling would smear a 1 px colour pattern while '
                 'luma survives a lossless encode exactly. check8_control must decode with NO AC '
                 'coefficients; if it has any, the picture is not 1:1 on device pixels.'),
    }
    with open(os.path.join(out, 'manifest.json'), 'w') as fh:
        json.dump(manifest, fh, indent=2)

    if args.refs_only or args.skip_encode:
        return 0

    for hdr, base, pixfmt, depth in ((True, 'ac-hdr', 'rgb48le', 16),
                                     (False, 'ac-sdr', 'rgb24', 8)):
        print(f'encoding {base} ({"HDR PQ/BT.2020" if hdr else "SDR BT.709"})')
        src = ['-f', 'rawvideo', '-pix_fmt', pixfmt, '-s', f'{W}x{H}',
               '-r', str(FPS), '-i', '-']
        hdrgen.encode(src, os.path.join(out, base), FPS, hdr, feeder=frames(out, hdr))

    print('done')
    return 0


if __name__ == '__main__':
    sys.exit(main())
