#!/usr/bin/env python3
"""Generate genuinely-HDR test content for the DisplayLink DL7400 capture session.

The point of this content is that every pixel on screen has a *known absolute
luminance* and a *known gamut*, so a decoded USB video record can be compared
against a number rather than against a vibe.

Two clips are produced, each in an HDR10 and an SDR flavour:

  pattern   14 static segments, 6 s each -- flat fields, primaries, ramps,
            specular highlights.  Static on purpose: a static screen makes the
            dock send one keyframe and then go quiet, so a capture window
            lands wholly inside one known picture.
  motion    a 1000-nit block sliding across a 100-nit field, 30 s at 30 fps.
            Continuous damage, for measuring flow and for decoding a large
            number of strips of known content.

Both carry a binary marker in the top-left corner (40x40 px cells, MSB left,
lit cells at 203 nits) so a decoded strip can be tied to an exact segment or
an exact frame index without trusting wall-clock alignment.

HDR flavour : SMPTE ST 2084 (PQ) transfer, BT.2020 primaries, 10-bit, limited
              range, HDR10 static metadata (MaxCLL 10000, mastering display
              L 0.0001 - 10000 cd/m2).  Encoded LOSSLESS, so what the decoder
              hands the compositor is bit-exact with ref/*.png.
SDR flavour : BT.709 primaries, sRGB-ish transfer, 8-bit, limited range.  The
              same geometry, with 203 nits mapped to SDR white and everything
              above it clipped -- i.e. deliberately the *same picture* an SDR
              pipeline would be able to show.

Usage:  python3 make-hdr-patterns.py --out DIR [--work DIR] [--skip-encode]
"""

from __future__ import annotations

import argparse
import json
import os
import struct
import subprocess
import sys
import zlib

import numpy as np

W, H = 2560, 1440
CELL = 40                      # marker cell size; divides both 2560 and 1440
PATTERN_FPS = 10               # static content: no reason to run it fast
PATTERN_SECONDS = 6            # per segment
MOTION_FPS = 30
MOTION_SECONDS = 30

SDR_WHITE_NITS = 203.0         # BT.2408 HDR reference white == SDR diffuse white

# ---------------------------------------------------------------- colour ----

# SMPTE ST 2084 constants.
_M1 = 2610.0 / 16384.0
_M2 = 2523.0 / 4096.0 * 128.0
_C1 = 3424.0 / 4096.0
_C2 = 2413.0 / 4096.0 * 32.0
_C3 = 2392.0 / 4096.0 * 32.0


def pq_encode(nits: np.ndarray) -> np.ndarray:
    """Absolute luminance in cd/m2 -> PQ signal in 0..1."""
    y = np.clip(np.asarray(nits, dtype=np.float64) / 10000.0, 0.0, 1.0)
    ym = np.power(y, _M1)
    return np.power((_C1 + _C2 * ym) / (1.0 + _C3 * ym), _M2)


def pq_decode(code: np.ndarray) -> np.ndarray:
    """PQ signal in 0..1 -> absolute luminance in cd/m2."""
    e = np.power(np.clip(np.asarray(code, dtype=np.float64), 0.0, 1.0), 1.0 / _M2)
    num = np.maximum(e - _C1, 0.0)
    den = _C2 - _C3 * e
    return 10000.0 * np.power(num / den, 1.0 / _M1)


# Linear BT.709 -> linear BT.2020 and back.
M_709_TO_2020 = np.array([
    [0.6274039, 0.3292830, 0.0433131],
    [0.0690973, 0.9195404, 0.0113623],
    [0.0163914, 0.0880133, 0.8955953],
], dtype=np.float64)

M_2020_TO_709 = np.array([
    [1.6604910, -0.5876411, -0.0728499],
    [-0.1245505, 1.1328999, -0.0083494],
    [-0.0181508, -0.1005789, 1.1187297],
], dtype=np.float64)


def matmul_img(img: np.ndarray, m: np.ndarray) -> np.ndarray:
    """Apply a 3x3 matrix to an (H, W, 3) linear image."""
    return np.einsum('ij,hwj->hwi', m, img)


def srgb_encode(lin: np.ndarray) -> np.ndarray:
    """Linear 0..1 -> sRGB signal 0..1."""
    lin = np.clip(lin, 0.0, 1.0)
    return np.where(lin <= 0.0031308, lin * 12.92, 1.055 * np.power(lin, 1 / 2.4) - 0.055)


# ------------------------------------------------------------------ PNG ------

def write_png16(path: str, rgb16: np.ndarray) -> None:
    """Write an (H, W, 3) uint16 array as a 16-bit RGB PNG."""
    h, w, _ = rgb16.shape
    raw = rgb16.astype('>u2').tobytes()
    stride = w * 6
    lines = bytearray()
    for y in range(h):
        lines.append(0)                                   # filter type 0 (None)
        lines += raw[y * stride:(y + 1) * stride]
    _png(path, w, h, 16, bytes(lines))


def write_png8(path: str, rgb8: np.ndarray) -> None:
    h, w, _ = rgb8.shape
    raw = rgb8.astype(np.uint8).tobytes()
    stride = w * 3
    lines = bytearray()
    for y in range(h):
        lines.append(0)
        lines += raw[y * stride:(y + 1) * stride]
    _png(path, w, h, 8, bytes(lines))


def _png(path: str, w: int, h: int, depth: int, scanlines: bytes) -> None:
    def chunk(tag: bytes, data: bytes) -> bytes:
        return (struct.pack('>I', len(data)) + tag + data
                + struct.pack('>I', zlib.crc32(tag + data) & 0xFFFFFFFF))

    ihdr = struct.pack('>IIBBBBB', w, h, depth, 2, 0, 0, 0)   # colour type 2 = RGB
    with open(path, 'wb') as fh:
        fh.write(b'\x89PNG\r\n\x1a\n')
        fh.write(chunk(b'IHDR', ihdr))
        fh.write(chunk(b'IDAT', zlib.compress(scanlines, 6)))
        fh.write(chunk(b'IEND', b''))


# --------------------------------------------------------------- markers ----

def draw_marker(nits: np.ndarray, value: int, bits: int) -> None:
    """Binary marker in the top-left, MSB in the leftmost cell.

    Cells are CELL x CELL.  A set bit is neutral 203 nits, a clear bit is 0.
    One extra always-lit sentinel cell follows, so the marker's extent is
    unambiguous even in a decoded strip with no absolute coordinates.
    """
    for i in range(bits):
        bit = (value >> (bits - 1 - i)) & 1
        x0 = i * CELL
        nits[0:CELL, x0:x0 + CELL, :] = SDR_WHITE_NITS if bit else 0.0
    x0 = bits * CELL
    nits[0:CELL, x0:x0 + CELL, :] = SDR_WHITE_NITS


# -------------------------------------------------------------- segments ----

def neutral(n: float) -> np.ndarray:
    img = np.zeros((H, W, 3), dtype=np.float64)
    img[:, :, :] = n
    return img


def vbars(values: list[tuple[float, float, float]]) -> np.ndarray:
    """Equal-width vertical bars; each value is a per-channel nit triple."""
    img = np.zeros((H, W, 3), dtype=np.float64)
    n = len(values)
    bw = W // n
    for i, v in enumerate(values):
        img[:, i * bw:(i + 1) * bw, :] = np.array(v, dtype=np.float64)
    return img


def prim_bars(n: float) -> list[tuple[float, float, float]]:
    """R G B C M Y W black, each lit channel at n nits (BT.2020 primaries)."""
    return [
        (n, 0, 0), (0, n, 0), (0, 0, n), (0, n, n),
        (n, 0, n), (n, n, 0), (n, n, n), (0, 0, 0),
    ]


def seg_gamut_ab() -> np.ndarray:
    """Left half BT.709 primaries, right half BT.2020 primaries, same nits.

    Pure gamut A/B at fixed luminance: if the dock's path is BT.709-limited the
    two halves decode to the same thing; if it is wide-gamut they do not.
    """
    img = np.zeros((H, W, 3), dtype=np.float64)
    n = SDR_WHITE_NITS
    band = H // 3
    prims_2020 = [(n, 0, 0), (0, n, 0), (0, 0, n)]
    for i, p in enumerate(prims_2020):
        y0 = i * band
        y1 = H if i == 2 else (i + 1) * band
        # Left: the same primary specified in BT.709, carried into BT.2020.
        lin709 = np.array(p, dtype=np.float64)
        lin2020 = M_709_TO_2020 @ lin709
        img[y0:y1, 0:W // 2, :] = lin2020
        # Right: the BT.2020 primary itself.
        img[y0:y1, W // 2:, :] = np.array(p, dtype=np.float64)
    return img


def seg_pq_ramp() -> np.ndarray:
    """Staircase defined in PQ *code* space, 64 steps of 40 px.

    This one is authored as code values rather than nits, because it is a
    bit-depth probe: step k is PQ code k * 1023/63, so an 8-bit path collapses
    adjacent steps and a 10-bit path does not.
    """
    img = np.zeros((H, W, 3), dtype=np.float64)
    steps = 64
    sw = W // steps
    for k in range(steps):
        code = (k * 1023.0 / (steps - 1)) / 1023.0
        img[:, k * sw:(k + 1) * sw, :] = pq_decode(np.array(code))
    return img


def seg_near_black() -> np.ndarray:
    """PQ codes 0,4,8..60 -- the region an 8-bit pipeline cannot represent."""
    img = np.zeros((H, W, 3), dtype=np.float64)
    steps = 16
    sw = W // steps
    for k in range(steps):
        code = (k * 4.0) / 1023.0
        img[:, k * sw:(k + 1) * sw, :] = pq_decode(np.array(code))
    return img


def seg_specular() -> np.ndarray:
    """Flat 100-nit field, 640 px 1000-nit square, 160 px 4000-nit core."""
    img = neutral(100.0)
    y0, x0 = (H - 640) // 2, (W - 640) // 2
    img[y0:y0 + 640, x0:x0 + 640, :] = 1000.0
    y1, x1 = (H - 160) // 2, (W - 160) // 2
    img[y1:y1 + 160, x1:x1 + 160, :] = 4000.0
    return img


def seg_detail() -> np.ndarray:
    """16 px checkerboard 100 vs 1000 nits on the left, flat 203 on the right.

    Codec stress at HDR range: the worst case for a block transform, with a
    flat control on the same rows.
    """
    img = np.zeros((H, W, 3), dtype=np.float64)
    img[:, :, :] = SDR_WHITE_NITS
    cell = 16
    half = W // 2
    ys = (np.arange(H) // cell)[:, None]
    xs = (np.arange(half) // cell)[None, :]
    check = ((ys + xs) % 2) == 0
    left = np.where(check, 1000.0, 100.0)
    img[:, 0:half, :] = left[:, :, None]
    return img


SEGMENTS: list[tuple[str, str]] = [
    ('black', 'full black -- the baseline; nothing on screen'),
    ('grey100', 'flat neutral 100 cd/m2 (traditional SDR white level)'),
    ('grey203', 'flat neutral 203 cd/m2 (BT.2408 HDR reference white)'),
    ('grey1000', 'flat neutral 1000 cd/m2 -- unambiguously beyond SDR'),
    ('grey4000', 'flat neutral 4000 cd/m2 -- extreme, above most panel peaks'),
    ('steps8', '8 vertical bars, neutral 0/10/50/100/203/400/1000/4000 cd/m2'),
    ('prim2020_203', 'BT.2020 R G B C M Y W black, lit channels at 203 cd/m2'),
    ('prim2020_1000', 'BT.2020 R G B C M Y W black, lit channels at 1000 cd/m2'),
    ('gamut_ab', 'left half BT.709 primaries, right half BT.2020, both 203 cd/m2'),
    ('pq_ramp', '64-step staircase in PQ code space, 0..1023'),
    ('near_black', 'PQ codes 0,4,8..60 -- sub-8-bit near-black steps'),
    ('specular', '100 cd/m2 field, 640 px 1000 cd/m2 square, 160 px 4000 cd/m2 core'),
    ('detail', '16 px 100-vs-1000 cd/m2 checkerboard left, flat 203 cd/m2 right'),
    ('black_end', 'full black -- end marker'),
]


def build_segment(idx: int) -> np.ndarray:
    """Linear BT.2020 RGB in absolute cd/m2 for segment idx."""
    name = SEGMENTS[idx][0]
    if name in ('black', 'black_end'):
        img = neutral(0.0)
    elif name == 'grey100':
        img = neutral(100.0)
    elif name == 'grey203':
        img = neutral(SDR_WHITE_NITS)
    elif name == 'grey1000':
        img = neutral(1000.0)
    elif name == 'grey4000':
        img = neutral(4000.0)
    elif name == 'steps8':
        img = vbars([(v, v, v) for v in (0, 10, 50, 100, 203, 400, 1000, 4000)])
    elif name == 'prim2020_203':
        img = vbars(prim_bars(SDR_WHITE_NITS))
    elif name == 'prim2020_1000':
        img = vbars(prim_bars(1000.0))
    elif name == 'gamut_ab':
        img = seg_gamut_ab()
    elif name == 'pq_ramp':
        img = seg_pq_ramp()
    elif name == 'near_black':
        img = seg_near_black()
    elif name == 'specular':
        img = seg_specular()
    elif name == 'detail':
        img = seg_detail()
    else:
        raise AssertionError(name)
    draw_marker(img, idx, 8)
    return img


MOTION_REFS = [0.0, 100.0, SDR_WHITE_NITS, 1000.0, 4000.0]


def build_motion_frame(i: int, total: int) -> np.ndarray:
    """100-nit field, 1000-nit block sweeping left to right, static ref column."""
    img = neutral(100.0)

    # Static reference column down the left edge: known nits, never moves, so
    # it is present in every keyframe and every partial update of that column.
    for i_ref, n in enumerate(MOTION_REFS):
        img[i_ref * 240:(i_ref + 1) * 240, 0:160, :] = n

    bw, bh = 400, 300
    span = W - 160 - bw
    phase = (i * 2.0 / total) % 2.0
    t = phase if phase <= 1.0 else 2.0 - phase
    x = 160 + int(t * span)
    y = (H - bh) // 2
    img[y:y + bh, x:x + bw, :] = 1000.0

    # A saturated BT.2020 red trailer block, so motion also exercises gamut.
    img[y - 340:y - 40, x:x + bw, :] = np.array([1000.0, 0.0, 0.0])

    draw_marker(img, i, 16)
    return img


class MotionPalette:
    """Fast path for the motion clip.

    Only six distinct colours ever appear, so the (slow) transfer-function and
    matrix maths runs once on a six-entry palette and every frame is then just
    a handful of array fills.  Generating a frame this way is milliseconds, not
    a second, which is what makes it practical to regenerate the whole clip per
    codec and pipe it straight into ffmpeg instead of parking 20 GB of raw
    frames on disk.
    """

    KEYS = ['field', 'block', 'red', 'lit', 'unlit'] + \
           ['ref%d' % i for i in range(len(MOTION_REFS))]

    def __init__(self, hdr: bool):
        self.hdr = hdr
        colours = {
            'field': (100.0, 100.0, 100.0),
            'block': (1000.0, 1000.0, 1000.0),
            'red': (1000.0, 0.0, 0.0),
            'lit': (SDR_WHITE_NITS,) * 3,
            'unlit': (0.0, 0.0, 0.0),
        }
        for i, n in enumerate(MOTION_REFS):
            colours['ref%d' % i] = (n, n, n)
        stack = np.array([colours[k] for k in self.KEYS],
                         dtype=np.float64).reshape(1, -1, 3)
        if hdr:
            enc = to_hdr_png16(stack)[0]
        else:
            enc = to_sdr_png8(stack)[0]
        self.p = {k: enc[i] for i, k in enumerate(self.KEYS)}
        self.dtype = np.uint16 if hdr else np.uint8

    def frame(self, i: int, total: int) -> bytes:
        img = np.empty((H, W, 3), dtype=self.dtype)
        img[:, :, :] = self.p['field']
        for i_ref in range(len(MOTION_REFS)):
            img[i_ref * 240:(i_ref + 1) * 240, 0:160, :] = self.p['ref%d' % i_ref]

        bw, bh = 400, 300
        span = W - 160 - bw
        phase = (i * 2.0 / total) % 2.0
        t = phase if phase <= 1.0 else 2.0 - phase
        x = 160 + int(t * span)
        y = (H - bh) // 2
        img[y:y + bh, x:x + bw, :] = self.p['block']
        img[y - 340:y - 40, x:x + bw, :] = self.p['red']

        for b in range(16):
            on = (i >> (15 - b)) & 1
            img[0:CELL, b * CELL:(b + 1) * CELL, :] = self.p['lit' if on else 'unlit']
        img[0:CELL, 16 * CELL:17 * CELL, :] = self.p['lit']

        if self.hdr:
            return img.astype('<u2').tobytes()
        return img.tobytes()


# ------------------------------------------------------------- flavours -----

def to_hdr_png16(nits: np.ndarray) -> np.ndarray:
    """Linear BT.2020 cd/m2 -> PQ-coded 16-bit RGB (full range 0..65535)."""
    return np.round(pq_encode(nits) * 65535.0).astype(np.uint16)


def to_sdr_png8(nits: np.ndarray) -> np.ndarray:
    """Linear BT.2020 cd/m2 -> clipped BT.709 sRGB 8-bit."""
    lin709 = matmul_img(nits, M_2020_TO_709) / SDR_WHITE_NITS
    return np.round(srgb_encode(lin709) * 255.0).astype(np.uint8)


# ---------------------------------------------------------------- encode ----

MASTER_DISPLAY = 'G(8500,39850)B(6550,2300)R(35400,14600)WP(15635,16450)L(10000000,1)'
MASTER_DISPLAY_P3 = 'G(13250,34500)B(7500,3000)R(34000,16000)WP(15635,16450)L(10000000,1)'

# Metadata probes: identical picture, deliberately different HDR10 static
# metadata, one axis at a time.  This is the field-map lever -- the same trick
# that cracked the Ridge set-mode words.  Values are chosen to be findable as
# little-endian u16 in a byte stream (605 = 0x025d, 4000 = 0x0fa0, 50 = 0x0032)
# and to be implausible as anything else.
#
# CTA-861 units, for reading a capture: mastering display max luminance is
# whole cd/m2, min luminance is 0.0001 cd/m2, MaxCLL/MaxFALL are whole cd/m2,
# primaries and white point are 0.00002 chromaticity units.
METADATA_PROBES = [
    ('A-baseline', 'bt2020', 'smpte2084', MASTER_DISPLAY, '1000,400',
     'BT.2020 / PQ, mastering peak 1000, MaxCLL 1000 MaxFALL 400'),
    ('B-peak4000', 'bt2020', 'smpte2084',
     'G(8500,39850)B(6550,2300)R(35400,14600)WP(15635,16450)L(40000000,50)',
     '4000,1234',
     'same but mastering peak 4000 min 0.005, MaxCLL 4000 MaxFALL 1234'),
    ('C-peak605', 'bt2020', 'smpte2084',
     'G(8500,39850)B(6550,2300)R(35400,14600)WP(15635,16450)L(6000000,500)',
     '605,123',
     'mastering peak 600 min 0.05, MaxCLL 605 MaxFALL 123'),
    ('D-nometa', 'bt2020', 'smpte2084', None, None,
     'PQ + BT.2020 tagged, but NO mastering display and NO MaxCLL at all'),
    ('E-p3prim', 'bt2020', 'smpte2084', MASTER_DISPLAY_P3, '1000,400',
     'baseline with DCI-P3 mastering primaries -- isolates the primaries fields'),
    ('F-hlg', 'bt2020', 'arib-std-b67', None, None,
     'HLG transfer instead of PQ -- a different EOTF entirely'),
    ('G-bt709tag', 'bt709', 'bt709', None, None,
     'SDR tags on byte-identical samples -- isolates tag from pixel data'),
]


def encode_probe(png: str, out_base: str, colorprim: str, transfer: str,
                 master: str | None, cll: str | None) -> None:
    """One metadata probe: fixed picture, one metadata axis moved.

    The YUV matrix is bt2020nc for every probe including the BT.709-tagged one,
    so the coded samples are byte-identical across the whole set and the only
    difference on the wire can be metadata.
    """
    x265 = ['lossless=1', 'repeat-headers=1', 'range=limited',
            'colormatrix=bt2020nc', 'colorprim=' + colorprim,
            'transfer=' + transfer]
    if master:
        x265 += ['hdr10=1', 'hdr10-opt=1', 'master-display=' + master]
    if cll:
        x265 += ['max-cll=' + cll]
    run(['ffmpeg', '-y', '-hide_banner', '-loglevel', 'error',
         '-loop', '1', '-t', str(PATTERN_SECONDS), '-i', png,
         '-vf', HDR_VF, '-r', str(PATTERN_FPS),
         '-c:v', 'libx265', '-pix_fmt', 'yuv420p10le',
         '-x265-params', ':'.join(x265),
         '-color_primaries', colorprim, '-color_trc', transfer,
         '-colorspace', 'bt2020nc', '-color_range', 'tv',
         '-tag:v', 'hvc1', out_base + '.mp4'])

HDR_TAGS = ['-color_primaries', 'bt2020', '-color_trc', 'smpte2084',
            '-colorspace', 'bt2020nc', '-color_range', 'tv']
SDR_TAGS = ['-color_primaries', 'bt709', '-color_trc', 'bt709',
            '-colorspace', 'bt709', '-color_range', 'tv']

HDR_VF = ('scale=in_range=full:out_color_matrix=bt2020nc:out_range=tv,'
          'format=yuv420p10le')
SDR_VF = ('scale=in_range=full:out_color_matrix=bt709:out_range=tv,'
          'format=yuv420p')


def run(cmd: list[str], feeder=None) -> None:
    print('  $', ' '.join(cmd[:6]), '...', flush=True)
    if feeder is None:
        subprocess.run(cmd, check=True,
                       stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
        return
    proc = subprocess.Popen(cmd, stdin=subprocess.PIPE,
                            stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
    try:
        feeder(proc.stdin)
    finally:
        proc.stdin.close()
    err = proc.stderr.read()
    if proc.wait() != 0:
        raise subprocess.CalledProcessError(proc.returncode, cmd, stderr=err)


def encode(src: list[str], out_base: str, fps: int, hdr: bool, feeder=None) -> None:
    """Encode one input spec to both an HEVC .mp4 and a VP9 .webm, losslessly."""
    tags = HDR_TAGS if hdr else SDR_TAGS
    vf = HDR_VF if hdr else SDR_VF
    pixfmt = 'yuv420p10le' if hdr else 'yuv420p'

    x265 = ['lossless=1', 'repeat-headers=1', 'range=limited']
    if hdr:
        x265 += ['colorprim=bt2020', 'transfer=smpte2084', 'colormatrix=bt2020nc',
                 'hdr10=1', 'hdr10-opt=1',
                 'master-display=' + MASTER_DISPLAY, 'max-cll=10000,4000']
    else:
        x265 += ['colorprim=bt709', 'transfer=bt709', 'colormatrix=bt709']

    run(['ffmpeg', '-y', '-hide_banner', '-loglevel', 'error'] + src
        + ['-vf', vf, '-r', str(fps),
           '-c:v', 'libx265', '-pix_fmt', pixfmt,
           '-x265-params', ':'.join(x265)] + tags
        + ['-tag:v', 'hvc1', out_base + '.mp4'], feeder)

    # libvpx-vp9 folds colour signalling into VP9's own coarse colour-space
    # field, and ffmpeg then writes a WebM Colour element carrying only
    # MatrixCoefficients and Range -- TransferCharacteristics and Primaries are
    # silently dropped, so Chromium sees an untagged BT.2020 stream and treats
    # it as SDR.  Verified by reading the EBML back.  Remuxing with -c copy and
    # the tags restated does write them (0x55ba = 16 PQ, 0x55bb = 9 BT.2020).
    tmp = out_base + '.tmp.webm'
    run(['ffmpeg', '-y', '-hide_banner', '-loglevel', 'error'] + src
        + ['-vf', vf, '-r', str(fps),
           '-c:v', 'libvpx-vp9', '-pix_fmt', pixfmt,
           '-profile:v', '2' if hdr else '0',
           '-lossless', '1', '-row-mt', '1', '-cpu-used', '4',
           '-g', str(fps)] + tags + ['-f', 'webm', tmp], feeder)
    run(['ffmpeg', '-y', '-hide_banner', '-loglevel', 'error', '-i', tmp,
         '-c:v', 'copy'] + tags + ['-f', 'webm', out_base + '.webm'])
    os.remove(tmp)


# ------------------------------------------------------------------ main ----

def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument('--out', required=True, help='destination directory')
    ap.add_argument('--work', default=None, help='scratch dir for frames')
    ap.add_argument('--skip-encode', action='store_true')
    ap.add_argument('--only', choices=['pattern', 'motion', 'probes', 'decoded'],
                    default=None)
    args = ap.parse_args()

    out = os.path.abspath(args.out)
    work = os.path.abspath(args.work or os.path.join(out, '_work'))
    ref = os.path.join(out, 'ref')
    os.makedirs(ref, exist_ok=True)
    os.makedirs(work, exist_ok=True)

    manifest: dict = {
        'width': W, 'height': H,
        'marker': {'cell_px': CELL, 'origin': [0, 0], 'msb': 'left',
                   'lit_nits': SDR_WHITE_NITS,
                   'pattern_bits': 8, 'motion_bits': 16,
                   'sentinel': 'one always-lit cell immediately after the last bit'},
        'hdr': {'transfer': 'smpte2084', 'primaries': 'bt2020',
                'matrix': 'bt2020nc', 'range': 'limited', 'depth': 10},
        'sdr': {'transfer': 'bt709/srgb', 'primaries': 'bt709',
                'matrix': 'bt709', 'range': 'limited', 'depth': 8,
                'sdr_white_nits': SDR_WHITE_NITS},
    }

    # ------------------------------------------------ static pattern clip ---
    if args.only in (None, 'pattern'):
        print('building %d pattern segments' % len(SEGMENTS), flush=True)
        segs = []
        for idx, (name, desc) in enumerate(SEGMENTS):
            nits = build_segment(idx)
            hdr_png = os.path.join(ref, 'seg%02d-%s.hdr.png' % (idx, name))
            sdr_png = os.path.join(ref, 'seg%02d-%s.sdr.png' % (idx, name))
            write_png16(hdr_png, to_hdr_png16(nits))
            write_png8(sdr_png, to_sdr_png8(nits))
            segs.append({
                'index': idx, 'name': name, 'description': desc,
                'start_s': idx * PATTERN_SECONDS,
                'end_s': (idx + 1) * PATTERN_SECONDS,
                'ref_hdr': os.path.relpath(hdr_png, out),
                'ref_sdr': os.path.relpath(sdr_png, out),
                'peak_nits': float(np.max(nits)),
            })
            print('  seg%02d %-14s peak %8.1f cd/m2' % (idx, name, np.max(nits)),
                  flush=True)
        manifest['pattern'] = {
            'fps': PATTERN_FPS, 'segment_seconds': PATTERN_SECONDS,
            'total_seconds': PATTERN_SECONDS * len(SEGMENTS),
            'segments': segs,
        }

        if not args.skip_encode:
            for flavour, ext in (('hdr', 'hdr.png'), ('sdr', 'sdr.png')):
                lst = os.path.join(work, 'concat-%s.txt' % flavour)
                with open(lst, 'w') as fh:
                    for idx, (name, _) in enumerate(SEGMENTS):
                        p = os.path.join(ref, 'seg%02d-%s.%s' % (idx, name, ext))
                        fh.write("file '%s'\nduration %d\n" % (p, PATTERN_SECONDS))
                    # concat demuxer ignores the last duration unless the file
                    # is repeated.
                    idx = len(SEGMENTS) - 1
                    fh.write("file '%s'\n" % os.path.join(
                        ref, 'seg%02d-%s.%s' % (idx, SEGMENTS[idx][0], ext)))
                print('encoding %s pattern' % flavour, flush=True)
                encode(['-f', 'concat', '-safe', '0', '-i', lst],
                       os.path.join(out, '%s-pattern' % flavour),
                       PATTERN_FPS, flavour == 'hdr')

    # ------------------------------------------------------- motion clip ----
    if args.only in (None, 'motion'):
        total = MOTION_FPS * MOTION_SECONDS
        manifest['motion'] = {
            'fps': MOTION_FPS, 'seconds': MOTION_SECONDS, 'frames': total,
            'note': ('frame index is in the 16-bit top-left marker; '
                     'static reference column x=0..160 is 0/100/203/1000/4000 '
                     'cd/m2 in 240 px bands from the top'),
        }
        if not args.skip_encode:
            for flavour in ('hdr', 'sdr'):
                print('encoding %s motion (%d frames)' % (flavour, total),
                      flush=True)
                pal = MotionPalette(flavour == 'hdr')

                def feeder(stream, pal=pal):
                    for i in range(total):
                        stream.write(pal.frame(i, total))

                src = ['-f', 'rawvideo',
                       '-pix_fmt', 'rgb48le' if flavour == 'hdr' else 'rgb24',
                       '-s', '%dx%d' % (W, H), '-framerate', str(MOTION_FPS),
                       '-i', '-']
                encode(src, os.path.join(out, '%s-motion' % flavour),
                       MOTION_FPS, flavour == 'hdr', feeder)
        # A couple of reference stills for the motion clip.
        for i in (0, MOTION_FPS * MOTION_SECONDS // 4):
            nits = build_motion_frame(i, total)
            write_png16(os.path.join(ref, 'motion-f%04d.hdr.png' % i),
                        to_hdr_png16(nits))
            write_png8(os.path.join(ref, 'motion-f%04d.sdr.png' % i),
                       to_sdr_png8(nits))

    # ------------------------------------------------- decoder-exact refs ---
    # ref/*.png is the ideal picture that went *into* the encoder. What the
    # compositor actually receives is that picture after a 4:2:0 round trip,
    # which is bit-identical in flat areas but blends about 4 px either side of
    # every hard saturated edge. Analysis should compare against this, so that
    # an edge mismatch is never mistaken for a dock or codec defect.
    if args.only in (None, 'decoded'):
        dec_dir = os.path.join(ref, 'decoded')
        os.makedirs(dec_dir, exist_ok=True)
        for flavour, pix, ext in (('hdr', 'rgb48le', 'hdr'), ('sdr', 'rgb24', 'sdr')):
            clip = os.path.join(out, '%s-pattern.mp4' % flavour)
            if not os.path.exists(clip):
                continue
            print('extracting decoder-exact refs from', os.path.basename(clip),
                  flush=True)
            for idx, (name, _) in enumerate(SEGMENTS):
                t = idx * PATTERN_SECONDS + PATTERN_SECONDS / 2.0
                raw = subprocess.run(
                    ['ffmpeg', '-v', 'error', '-ss', str(t), '-i', clip,
                     '-frames:v', '1',
                     '-vf', 'scale=in_range=tv:out_range=full,format=%s' % pix,
                     '-f', 'rawvideo', '-'],
                    check=True, capture_output=True).stdout
                dst = os.path.join(dec_dir, 'seg%02d-%s.%s.png' % (idx, name, ext))
                if flavour == 'hdr':
                    arr = np.frombuffer(raw, dtype='<u2').reshape(H, W, 3)
                    write_png16(dst, arr)
                else:
                    arr = np.frombuffer(raw, dtype=np.uint8).reshape(H, W, 3)
                    write_png8(dst, arr)
        manifest['decoded_refs'] = {
            'dir': 'ref/decoded',
            'note': ('the pattern segments as a decoder actually produces them; '
                     'flat interiors match ref/ exactly, edges carry the 4:2:0 '
                     'chroma blend -- compare wire output against these'),
        }

    # --------------------------------------------------- metadata probes ----
    if args.only in (None, 'probes'):
        probe_dir = os.path.join(out, 'probes')
        os.makedirs(probe_dir, exist_ok=True)
        png = os.path.join(ref, 'seg11-specular.hdr.png')
        if not os.path.exists(png):
            write_png16(png, to_hdr_png16(build_segment(11)))
        rows = []
        for name, prim, trc, master, cll, desc in METADATA_PROBES:
            base = os.path.join(probe_dir, 'probe-%s' % name)
            if not args.skip_encode:
                print('encoding probe %s' % name, flush=True)
                encode_probe(png, base, prim, trc, master, cll)
            rows.append({'name': name, 'file': os.path.relpath(base + '.mp4', out),
                         'primaries': prim, 'transfer': trc,
                         'master_display': master, 'max_cll_fall': cll,
                         'description': desc})
        manifest['metadata_probes'] = {
            'picture': os.path.relpath(png, out),
            'seconds_each': PATTERN_SECONDS,
            'note': ('identical coded samples across the whole set; only the '
                     'HDR10 metadata differs, one axis at a time'),
            'probes': rows,
        }

    # Merge rather than overwrite: a --only run fills in one section, and
    # clobbering the rest leaves a manifest that describes a fraction of what is
    # actually in the directory. (It did exactly that once.)
    path = os.path.join(out, 'manifest.json')
    if os.path.exists(path):
        with open(path) as fh:
            try:
                merged = json.load(fh)
            except ValueError:
                merged = {}
        merged.update(manifest)
        manifest = merged
    with open(path, 'w') as fh:
        json.dump(manifest, fh, indent=2)
    print('manifest written to', path, '- sections:', ', '.join(sorted(manifest)))
    return 0


if __name__ == '__main__':
    sys.exit(main())
