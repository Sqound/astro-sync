#!/usr/bin/env python3
"""Render functions of the stacking pipelines, from a linear RGB master.

Input must be a calibrated, registered, linear RGB FITS master.  The script
keeps it intact and writes all display products in a dedicated output folder.

Each function takes the nebula layer and the star layer (both float RGB in
0..1) and returns the display image.  The pipeline definitions
(``pipelines/<id>/pipeline.json``, see ``pipelines/SCHEMA.md``) name them in
their ``entry`` field as ``renders:<function>``; only the functions listed in
``RENDERS`` can be named:

- ``natural_deep`` (pipeline ``raw``): the camera's native RGB ratios, never
  remapped (only the cores the sensor clipped, which have none, are made
  white: ``white_star_cores``); a soft shadow toe, a gentle lift of the faint
  signal, restrained stars (what StarNet2 leaves under them
  cleaned: ``natural_split``, ``starless_around_stars``; the light of the clipped
  ones given back: ``with_clipped_star_light``).  The pipeline's current natural reference.
- ``dualband_deep`` (pipeline ``dualband-correction``): exactly the same
  luminance treatment, then one global, continuous colour transform (green
  towards cyan, red towards red-violet, a slight blue-violet balance) that
  brings the colours of a dual-band capture back towards natural tones.
- ``polished_deep`` (pipeline ``polished``): a dark, neutral sky and a
  stronger nebula, from its own linear preparation (``prepare_polished``,
  then ``polished_layers``): the background and denoising of GraXpert or
  Siril, the colour by the capture's band, a midtones transfer that puts the
  sky at 0.03, white star cores without a ring, smoothing, a lift of the
  midtones, a local contrast, the stars nearly at full strength.  Without
  GraXpert: Siril; without either, the polynomial and no denoising.
- ``hoo_deep`` (pipeline ``hoo``): for a dual-band capture, Hα to
  red and OIII to blue-green on the natural brightness, stars in their
  natural colours.

The older ``natural_v23`` and ``dualband_v7`` renders are kept so that
earlier treatments can be reproduced from the command line of this module;
no pipeline definition uses them.

Notes:

- ``remove_gradient`` lives here, so the module does not import astroalign;
- the StarNet2 executable is a parameter (``--starnet``); without it, or for a
  field smaller than 512 px, the star separation is skipped;
- a single-channel (mono) master is repeated into three equal channels;
- ``save`` can leave the JPEG out (``jpeg=False``), write the TIFF or the PNG
  alone (``formats``), and returns the paths;
- StarNet2's output can go to a log file (``log``);
- the second render is named by its effect (``dualband_*``);
- ``star_fwhm`` is read by ``star_quality.py`` (same stars, same figure), which
  also gives the quality sent with a stacked image;
- ``presentation_contrast`` is an S-curve around the sky level, an option
  applied after a render: it bends the luminance and keeps the RGB ratios.  No
  render function uses it.
"""

from __future__ import annotations

import argparse
import contextlib
import os
import shutil
import subprocess
import sys
import tempfile
from collections.abc import Iterator, Sequence
from dataclasses import dataclass
from pathlib import Path
from typing import IO

import numpy as np
import tifffile
from astropy.io import fits
from PIL import Image
from scipy.ndimage import distance_transform_edt, gaussian_filter, maximum_filter, median_filter
from scipy.signal import fftconvolve
from skimage.color import hsv2rgb, rgb2hsv

import siril_config
import star_quality
from software_tag import software


def lum(rgb: np.ndarray) -> np.ndarray:
    return 0.2126 * rgb[..., 0] + 0.7152 * rgb[..., 1] + 0.0722 * rgb[..., 2]


def save(
    rgb: np.ndarray,
    output: Path,
    stem: str,
    jpeg: bool = True,
    formats: tuple[str, ...] = ("tif", "png"),
) -> list[Path]:
    """Write ``stem.tif`` (16-bit) and/or ``stem.png`` (8-bit), plus a JPEG if asked."""
    rgb = np.clip(rgb, 0, 1)
    written: list[Path] = []
    if "tif" in formats:
        written.append(output / f"{stem}.tif")
        tifffile.imwrite(written[-1], np.round(rgb * 65535).astype(np.uint16),
                         photometric="rgb", compression="deflate", software=software())
    eight = np.round(rgb * 255).astype(np.uint8)
    if "png" in formats:
        written.append(output / f"{stem}.png")
        Image.fromarray(eight, "RGB").save(written[-1], optimize=True)
    if jpeg:
        written.append(output / f"{stem}.jpg")
        Image.fromarray(eight, "RGB").save(written[-1], quality=96, subsampling=0)
    return written


def terms(x: np.ndarray, y: np.ndarray, degree: int = 2) -> np.ndarray:
    columns = (np.ones_like(x), x, y)
    if degree == 2:
        columns += (x * x, x * y, y * y)
    return np.column_stack(columns)


# The black point: the ``black_percentile`` of the frame, but never more than ``BLACK_FLOOR_SIGMAS`` noise widths
# under the sky.  A short stack registered on a rotating field has thinly covered, noisier edges; its darkest
# pixels lie there, so the percentile alone falls several noise widths under the sky and the sky shows grey (the
# 36 frames of 10 s of a real night: 4.3 widths, a sky at 0.32 on the polished render; 0.25 with the floor).  A
# stack whose percentile already sits within the floor keeps it to the last digit: twelve of the fourteen real
# masters measured (the validated renders) do not move.
BLACK_FLOOR_SIGMAS = 2.8  # the sky stays at most this many noise widths above the black point


def _robust_sigma(values: np.ndarray) -> float:
    """1.4826 times the median absolute deviation."""
    return float(1.4826 * np.median(np.abs(values - np.median(values))))


def black_point(image: np.ndarray, black_percentile: float) -> np.ndarray:
    """The black point of an image, per channel: its ``black_percentile``, raised to the floor when it lies lower.

    The floor is the median of the channel minus ``BLACK_FLOOR_SIGMAS`` times its robust noise width (read on one
    pixel in sixteen, the empty pixels of the frame left out).
    """
    today = np.percentile(image, black_percentile, axis=(0, 1))
    floor = []
    for c in range(image.shape[2]):
        channel = image[..., c][::4, ::4].ravel()
        channel = channel[channel > 0] if np.any(channel > 0) else channel
        floor.append(float(np.median(channel)) - BLACK_FLOOR_SIGMAS * _robust_sigma(channel))
    return np.maximum(today, np.asarray(floor, dtype=today.dtype))


def remove_gradient(
    data: np.ndarray,
    tile: int = 160,
    degree: int = 2,
    black_percentile: float = 0.25,
    black: Sequence[float] | None = None,
    measured: dict[str, object] | None = None,
    display: bool = False,
) -> np.ndarray:
    """Subtract a robust low-order background model without flattening stars.

    The black point is the ``black_percentile`` of the flattened image, per channel, unless ``black`` imposes
    it (the value a render of the same plan measured); for the ``display`` of a render it is raised to the floor
    of :func:`black_point` (the data steps -- the alignment and the combination of a plan -- keep the percentile
    alone); ``measured`` (when given) receives the one used, as
    ``measured["black"]``: three floats, the level subtracted from the flattened image -- absolute, in the
    units of the data, so that it means the same level for the stack of one night and the stack of three.
    """
    if degree not in (1, 2):
        raise ValueError("gradient degree must be 1 or 2")
    if not 0 <= black_percentile <= 5:
        raise ValueError("black percentile must be between 0 and 5")
    h, w, _ = data.shape
    xs, ys, samples = [], [], []
    for y0 in range(0, h - tile + 1, tile):
        for x0 in range(0, w - tile + 1, tile):
            patch = data[y0 : y0 + tile, x0 : x0 + tile]
            xs.append((x0 + tile / 2) / w * 2 - 1)
            ys.append((y0 + tile / 2) / h * 2 - 1)
            samples.append(np.percentile(patch, 16, axis=(0, 1)))
    x, y, z = np.asarray(xs), np.asarray(ys), np.asarray(samples)
    design = terms(x, y, degree)
    yy, xx = np.mgrid[:h, :w]
    full = terms(xx.ravel() / w * 2 - 1, yy.ravel() / h * 2 - 1, degree)
    corrected = np.empty_like(data)
    for channel in range(3):
        keep = np.ones(len(z), dtype=bool)
        for _ in range(6):
            coef, *_ = np.linalg.lstsq(design[keep], z[keep, channel], rcond=None)
            residual = z[:, channel] - design @ coef
            med = np.median(residual[keep])
            mad = 1.4826 * np.median(np.abs(residual[keep] - med)) + 1e-12
            keep = np.abs(residual - med) < 2.2 * mad
        model = (full @ coef).reshape(h, w)
        model -= np.percentile(model, 2)
        # The image flattened around the mean level of the model: its sky keeps the level of the data, whatever
        # offset the model had (the tiles' 16th percentile sits lower the noisier the image is).
        corrected[..., channel] = data[..., channel] - model + model.mean()
    point = ((black_point(corrected, black_percentile) if display
              else np.percentile(corrected, black_percentile, axis=(0, 1))) if black is None
             else np.asarray(black, dtype=corrected.dtype))
    if measured is not None:
        measured["black"] = [float(v) for v in point]
    return np.clip(corrected - point, 0, None)


def measure_psf(level: np.ndarray, size: int = 15, limit: int = 400) -> np.ndarray:
    """Empirical PSF: median of isolated, unsaturated stars.

    More faithful than a fitted Gaussian, because the real PSF of an optical
    train is often slightly elliptical with a stable orientation across the field.
    """
    background = np.median(level)
    signal = level - background
    sigma = np.median(np.abs(signal)) * 1.4826
    peaks = (signal == maximum_filter(signal, size=9)) & (signal > 60 * sigma)
    peaks &= signal < 0.30 * signal.max()
    ys, xs = np.nonzero(peaks)
    height, width = level.shape
    inside = (ys > size) & (ys < height - size) & (xs > size) & (xs < width - size)
    ys, xs = ys[inside], xs[inside]
    points = np.stack([ys, xs], axis=1)
    isolated = [i for i in range(len(points))
                if (np.abs(points - points[i]).max(axis=1) < 2 * size).sum() == 1]
    ys, xs = ys[isolated][:limit], xs[isolated][:limit]
    if len(ys) < 20:
        raise RuntimeError(f"only {len(ys)} isolated stars found, PSF not measurable")
    half = size // 2
    stack = []
    for y, x in zip(ys, xs):
        cut = level[y - half:y + half + 1, x - half:x + half + 1].astype(np.float64)
        cut = cut - np.median(np.concatenate([cut[0], cut[-1], cut[:, 0], cut[:, -1]]))
        if cut.max() > 0:
            stack.append(cut / cut.max())
    psf = np.clip(np.median(np.asarray(stack), axis=0), 0, None)
    return psf / psf.sum()


def star_fwhm(level: np.ndarray, limit: int = 250) -> float:
    """Median FWHM in pixels, from radial profiles of isolated stars.

    The measure itself lives in ``star_quality.py`` (numpy and scipy only), so
    that the figure sent with a stacked image and the one the deconvolution
    aims at are read by the same code, on the same stars.
    """
    return star_quality.median_fwhm(level, limit)


def richardson_lucy(image: np.ndarray, psf: np.ndarray, iterations: int) -> np.ndarray:
    observed = np.maximum(image, 1e-9)
    mirrored = psf[::-1, ::-1]
    estimate = observed.copy()
    for _ in range(iterations):
        blurred = fftconvolve(estimate, psf, mode="same")
        ratio = observed / np.maximum(blurred, 1e-9)
        estimate = np.maximum(estimate * fftconvolve(ratio, mirrored, mode="same"), 0)
    return estimate


def _background_noise(level: np.ndarray, tile: int = 64) -> float:
    """Noise from the darkest tiles only, so nebulosity does not inflate it."""
    height, width = level.shape
    samples = []
    for y in range(0, height - tile, tile * 2):
        for x in range(0, width - tile, tile * 2):
            patch = level[y:y + tile, x:x + tile]
            middle = np.median(patch)
            samples.append((middle, np.median(np.abs(patch - middle)) * 1.4826))
    if not samples:
        middle = np.median(level)
        return float(np.median(np.abs(level - middle)) * 1.4826)
    samples.sort(key=lambda item: item[0])
    keep = max(1, len(samples) // 10)
    return float(np.mean([spread for _, spread in samples[:keep]]))


# The stars whose core the sensor clipped (the brightest ones of the frame) have no PSF to be
# deconvolved by: ``measure_psf`` leaves them out of the model, and Richardson-Lucy, asked to
# recover a flat top it cannot see, rings -- a dark halo, or a dark crescent, 6 to 14 px from the
# star, that grows with every iteration.  They are held out of the deconvolution: ``core`` is the
# share of the frame's peak above which a pixel belongs to such a star, ``hold`` the distance
# (px) around it that stays untouched, ``fade`` the distance over which the deconvolution comes back.
DECONV_CORE = 0.30  # the share of the peak ``measure_psf`` already leaves out of the model
DECONV_CORE_HOLD = 10.0
DECONV_CORE_FADE = 6.0
# Every step of 5 iterations must bring the stars at least this much narrower (px), or it is not kept:
# the width the iterations read is not monotonic, and a step that gains nothing only adds artefacts.
DECONV_MIN_GAIN = 0.03


def clipped_core_weight(
    level: np.ndarray,
    core: float = DECONV_CORE,
    hold: float = DECONV_CORE_HOLD,
    fade: float = DECONV_CORE_FADE,
) -> np.ndarray:
    """1 on and around the cores of the brightest stars, fading to 0 beyond; the deconvolution mask is multiplied by ``1 - weight``.

    The cores are the pixels above ``core`` times the peak of the frame (above its median).
    """
    background = float(np.median(level))
    bright = (level - background) > core * float(level.max() - background)
    if not bright.any():
        return np.zeros(level.shape, np.float32)
    distance = distance_transform_edt(~bright)
    return gaussian_filter(np.clip(1.0 - (distance - hold) / max(fade, 1e-6), 0.0, 1.0), 1.5).astype(np.float32)


def deconvolve_linear(
    linear: np.ndarray,
    target_fwhm: float,
    max_iterations: int = 60,
    step: int = 5,
    mask_low: float = 6.0,
    mask_high: float = 25.0,
) -> np.ndarray:
    """Masked Richardson-Lucy that preserves the native RGB ratios.

    Deconvolution is applied to the luminance only; the channels are then
    rescaled by the same factor, so the colour of the natural reference is
    untouched. The result is blended back through a local-SNR mask, so the faint
    background keeps its original pixels and its noise is not amplified.  The
    stars whose core is clipped, and the ring of sky around them, are kept out of
    the mask (``clipped_core_weight``): there is no PSF to recover, and the
    iterations would dig a dark ring around them.

    Iterations stop at the first multiple of ``step`` that reaches
    ``target_fwhm``, without going past it when a coarser result already
    qualifies, and as soon as a step brings the stars less than ``DECONV_MIN_GAIN`` px
    narrower.  Do not aim below about 2.2 px on an undersampled field (around
    4 arcsec per pixel): the stars would square off.
    """
    level = lum(linear)
    start = star_fwhm(level)
    if np.isfinite(start) and start <= target_fwhm:
        print(f"deconvolution skipped: FWHM {start:.2f} px already meets the {target_fwhm:.2f} px target")
        return linear

    psf = measure_psf(level)
    floor = np.percentile(level, 5)
    noise = _background_noise(level)
    snr_map = (gaussian_filter(level, 1.0) - floor) / max(noise, 1e-12)
    mask = np.clip((snr_map - mask_low) / (mask_high - mask_low), 0, 1)
    mask = gaussian_filter(mask * mask * (3 - 2 * mask), 2.0)
    mask *= 1.0 - clipped_core_weight(level)

    best, used, reached = None, 0, float("nan")
    for total in range(step, max_iterations + 1, step):
        candidate = level * (1 - mask) + richardson_lucy(level, psf, total) * mask
        width = star_fwhm(candidate)
        if not np.isfinite(width):
            break
        if width < target_fwhm and best is not None:
            break  # this step would overshoot; keep the previous, gentler result
        if best is not None and width > reached - DECONV_MIN_GAIN:
            break  # this step gains nothing: keep the previous one, with fewer artefacts
        best, used, reached = candidate, total, width
        if width <= target_fwhm:
            break

    if best is None:
        print(f"deconvolution skipped: no usable result at FWHM {start:.2f} px")
        return linear
    warning = ""
    if reached < target_fwhm - 0.05:
        warning = (f" -- WARNING: {step} iterations already overshoot the target; "
                   f"the PSF is narrow, consider a higher --deconv-fwhm")
    print(f"deconvolution: {used} iterations, FWHM {start:.2f} -> {reached:.2f} px, "
          f"mask over {100 * mask.mean():.1f}% of the frame{warning}")
    gain = best / np.maximum(level, 1e-8)
    return linear * gain[..., None]


def read_linear_rgb(source: Path) -> np.ndarray:
    """Read a linear master as height x width x 3 (a mono master is repeated)."""
    data = np.asarray(fits.getdata(source), np.float32)
    if data.ndim == 2:
        return np.repeat(data[..., None], 3, axis=2)
    if data.ndim == 3 and data.shape[0] == 3:
        return np.moveaxis(data, 0, -1)
    raise ValueError(f"{source.name}: expected a 2-D or 3 x H x W FITS image, got {data.shape}")


def calibrated_linear(
    source: Path,
    gradient_degree: int,
    black_percentile: float,
    deconv_fwhm: float | None,
    black: Sequence[float] | None = None,
    measured: dict[str, object] | None = None,
) -> np.ndarray:
    """Prepare a linear image before the common display stretch.

    ``black`` imposes the black point instead of measuring it, ``measured`` receives the one used
    (``remove_gradient``).

    Each integration keeps its own gradient correction and neutral background,
    but the display stretch can be shared with another integration.  This is
    essential for progression views: a separate percentile stretch for every
    cumulative stack makes their signal-to-noise improvement look much smaller
    than it really is.
    """
    raw = read_linear_rgb(source)
    linear = remove_gradient(
        raw,
        degree=gradient_degree,
        black_percentile=black_percentile,
        black=black,
        measured=measured,
        display=True,
    )
    if deconv_fwhm is not None:
        linear = deconvolve_linear(linear, deconv_fwhm)
    sky = np.percentile(linear, 22, axis=(0, 1))
    neutral = np.exp(np.mean(np.log(np.maximum(sky, 1e-12))))
    gain = np.clip(neutral / np.maximum(sky, 1e-12), 0.86, 1.16)
    return linear * gain[None, None, :]


def stretch_scale(linear: np.ndarray) -> float:
    """Return the luminance scale used by the natural display stretch."""
    return float(np.percentile(lum(linear), 99.82) + 1e-12)


# A master that a device (a smart telescope) already stretched is not linear: its sky stands at a tenth of the
# range or more, where a linear stack has it at a few hundredths of the bright end (0.0145 on the synthetic
# stack of the tests, 0.03 to 0.19 on the same stack stretched to a sky of 0.05 to 0.40).  The arcsinh display stretch, applied to it a
# second time, lifts that sky to the middle of the range (0.55 to 0.79 on the display image): every render
# came out with a grey background, whatever its tone.  Such a master is shown with a straight scale instead:
# its own stretch is the display's, and the render's tone and S-curve work on a sky that is where the device
# put it, with the gradient and the black point already removed.
# The bound is deliberately high: a linear master that a nebula fills must never take this road (its renders
# are the validated ones).  A device that stretches to the usual sky of 0.2 or more is recognised; one that
# stretches less is not, and keeps today's renders.  Measured on synthetic stacks only: the real masters of a
# smart telescope are to confirm it (the figure is in the render log, see ``sky_fraction``).
STRETCHED_SKY_FRACTION = 0.10  # median / ``stretch_scale`` of the prepared luminance, from which it is a stretched master


def sky_fraction(linear: np.ndarray) -> float:
    """The median luminance of a prepared image over its ``stretch_scale``: what ``already_stretched`` reads."""
    return float(np.median(lum(linear)) / stretch_scale(linear))


def already_stretched(linear: np.ndarray) -> bool:
    """Whether a prepared image (black point and gradient removed) was stretched before it reached us."""
    return sky_fraction(linear) >= STRETCHED_SKY_FRACTION


def note_stretched(linear: np.ndarray) -> None:
    """Say in the render log which display a master gets, with the figure the choice reads."""
    fraction = sky_fraction(linear)
    if fraction >= STRETCHED_SKY_FRACTION:
        print(f"already-stretched master (sky at {fraction:.3f} of the scale): straight display scale")
    else:
        print(f"linear master (sky at {fraction:.4f} of the scale): arcsinh display stretch")


def display_curve(level: np.ndarray, scale: float, stretched: bool) -> np.ndarray:
    """The display stretch of a luminance (0..1 out): arcsinh for a linear master, the straight scale for a stretched one."""
    if stretched:
        return np.clip(level / scale, 0, 1)
    return np.clip(np.arcsinh(18.0 * level / scale) / np.arcsinh(18.0), 0, 1)


def prepare_native(
    source: Path,
    gradient_degree: int = 2,
    black_percentile: float = 0.25,
    deconv_fwhm: float | None = None,
    display_scale: float | None = None,
    black: Sequence[float] | None = None,
    measured: dict[str, object] | None = None,
    light: list[np.ndarray | None] | None = None,
) -> np.ndarray:
    """The natural display image of a master.

    The display is two numbers: the black point (per channel) and the stretch scale.  They are measured on
    the image unless ``black`` and ``display_scale`` impose them -- the plan's milestones are shown at the
    level of the image of the plan --; ``measured`` (when given) receives the pair used, as
    ``measured["black"]`` and ``measured["scale"]``.  ``light`` (a list, when given) receives the clipped stars'
    light (``clipped_star_light``) the render gives back.
    """
    linear = calibrated_linear(source, gradient_degree, black_percentile, deconv_fwhm, black, measured)
    scale = display_scale if display_scale is not None else stretch_scale(linear)
    if measured is not None:
        measured["scale"] = float(scale)
    note_stretched(linear)
    return native_from_linear(linear, scale, light)


# The cores of the stars the sensor clipped, in the natural display.  A clipped core has no colour of its own: its
# plateau takes the ratio of the channels' clip levels, pushed again by the neutral-sky gains (violet, green or
# cyan, never the star's), and the edge of the plateau, where each channel leaves its clip level at its own radius
# (the optics spread red and blue wider than green), is strongly coloured.  The luminance stretch lifts that edge
# to the top of the range with its colour ratios, a channel passes 1, and the roll by the brightest channel darkens
# the pixel under the core it surrounds: a dark, violet ring around a coloured centre, which StarNet2, the
# composition of the render and every later contrast or colour step only make stronger.  Around such a core, and
# only there (``near_stars``), the natural display makes the core neutral at its luminance, rolls the luminance
# instead of the brightest channel, brings a channel past 1 towards white at the same luminance (``into_gamut``)
# and lets the highlights fade to white, as the polished render does.  A master without a clipped core is shown
# as before, to the bit.
NATURAL_ROLL_KNEE = 0.72  # display level above which the natural stretch rolls its highlights
# A clipped channel holds its clip level over a plateau; an unclipped star peaks on a single pixel of the 3 x 3
# median (a Gaussian stays within 0.2 % of its peak over a tenth of its width only).
CLIP_PLATEAU_PIXELS = 5  # pixels of the 3 x 3 median at the clip level, from which a channel counts as clipped
CLIP_PLATEAU_TOLERANCE = 0.002  # share of the clip level within which a pixel stands on the plateau
# The clip level is looked for among the brightest pixels of a channel only (the full 3 x 3 median of a 24 Mpx
# frame takes seconds): those at least ``CLIP_SEARCH_SHARE`` of its ``CLIP_SEARCH_RANK``-th brightest pixel, so a
# few hot pixels left by the stacking cannot hide the plateau.
CLIP_SEARCH_RANK = 50
CLIP_SEARCH_SHARE = 0.5
# The work around the clipped cores is done in boxes: ``STAR_BOX_MARGIN`` px around each core covers the reach of
# ``near_stars`` and of the smoothings (1 + 4 + 6 + 8 px), so a box gives the same pixels as the whole frame would.
STAR_BOX_MARGIN = 32
STAR_BOX_BLOCK = 16  # the grid (px) on which nearby cores are grouped into one box


def natural_roll(x: np.ndarray) -> np.ndarray:
    """The highlight roll of the natural display, on any display value (the curve ``native_from_linear`` rolls the
    brightest channel with)."""
    over = np.maximum(x - NATURAL_ROLL_KNEE, 0)
    return np.where(x > NATURAL_ROLL_KNEE, NATURAL_ROLL_KNEE + over / (1 + 1.9 * over), x)


def mask_boxes(
    mask: np.ndarray, margin: int, block: int = STAR_BOX_BLOCK, disjoint: bool = True,
) -> list[tuple[slice, slice]]:
    """Boxes that hold every pixel of ``mask`` with ``margin`` px around it (the image's edges cut them); ``disjoint``:
    no two of them overlap."""
    from scipy.ndimage import binary_dilation, find_objects, label

    height, width = mask.shape
    rows, cols = -(-height // block), -(-width // block)
    padded = np.zeros((rows * block, cols * block), bool)
    padded[:height, :width] = mask
    grid = padded.reshape(rows, block, cols, block).any(axis=(1, 3))
    grown = binary_dilation(grid, iterations=-(-margin // block), structure=np.ones((3, 3), bool))
    boxes = [(slice(s[0].start * block, min(s[0].stop * block, height)),
              slice(s[1].start * block, min(s[1].stop * block, width)))
             for s in find_objects(label(grown, structure=np.ones((3, 3), bool))[0]) if s is not None]
    # Two bounding boxes of separate groups can still overlap: join them until none does.
    joined = disjoint
    while joined:
        joined = False
        for i in range(len(boxes)):
            for j in range(i + 1, len(boxes)):
                a, b = boxes[i], boxes[j]
                if all(a[k].start < b[k].stop and b[k].start < a[k].stop for k in (0, 1)):
                    boxes[i] = (slice(min(a[0].start, b[0].start), max(a[0].stop, b[0].stop)),
                                slice(min(a[1].start, b[1].start), max(a[1].stop, b[1].stop)))
                    del boxes[j]
                    joined = True
                    break
            if joined:
                break
    return boxes


def clip_levels(linear: np.ndarray) -> list[float | None]:
    """Each channel's clip level (the highest value of its 3 x 3 median, as in ``clipped_cores``), or None when no
    plateau of ``CLIP_PLATEAU_PIXELS`` stands at it: that channel was not clipped."""
    levels: list[float | None] = []
    for channel in range(linear.shape[2]):
        plane = linear[..., channel]
        flat = plane.ravel()
        rank = flat.size - CLIP_SEARCH_RANK
        top = float(np.partition(flat, rank)[rank]) if rank > 0 else 0.0
        bright = plane >= CLIP_SEARCH_SHARE * top
        medians = [median_filter(plane[box], size=3) for box in mask_boxes(bright, 2, disjoint=False)]
        level = max((float(m.max()) for m in medians), default=0.0)
        plateau = max((int(np.count_nonzero(m >= level * (1 - CLIP_PLATEAU_TOLERANCE))) for m in medians), default=0)
        levels.append(level if level > 0 and plateau >= CLIP_PLATEAU_PIXELS else None)
    return levels


def near_clipped_cores(linear: np.ndarray, levels: list[float | None]) -> np.ndarray:
    """The pixels of ``linear`` at or above ``core_from`` of the clip level of a clipped channel."""
    near_core = np.zeros(linear.shape[:2], bool)
    for channel, value in enumerate(levels):
        if value is not None:
            near_core |= linear[..., channel] >= POLISHED.core_from * value
    return near_core


def white_star_cores(linear: np.ndarray, rgb: np.ndarray, level: np.ndarray, stretched: np.ndarray,
                     levels: list[float | None] | None = None) -> np.ndarray:
    """``rgb``, the natural display before its saturation, with the cores the sensor clipped made white and their
    halo never darkened under them (in place); unchanged where no clipped core is near.

    ``level`` is the luminance of ``linear``, ``stretched`` its display luminance before the roll; ``levels`` the
    clip levels (``clip_levels``), read on ``linear`` when not given.
    """
    levels = clip_levels(linear) if levels is None else levels
    if all(value is None for value in levels):
        return rgb
    span = max(POLISHED.core_to - POLISHED.core_from, 1e-6)
    near_core = near_clipped_cores(linear, levels)
    for box in mask_boxes(near_core, STAR_BOX_MARGIN):
        local = linear[box]
        # The weight of ``clipped_cores``, on the clipped channels only.
        share = np.zeros(local.shape[:2], np.float32)
        for channel, value in enumerate(levels):
            if value is not None:
                share = np.maximum(share, np.clip((local[..., channel] / value - POLISHED.core_from) / span, 0, 1))
        cores = np.clip(gaussian_filter(maximum_filter(share, size=3), 1.0), 0, 1).astype(np.float32)
        near = near_stars(cores)[..., None]
        shown = natural_roll(stretched[box])
        star = into_gamut(neutral_cores(local, cores) * (shown / np.maximum(level[box], 1e-8))[..., None], shown)
        fade = np.clip((shown - POLISHED.white_from) / max(POLISHED.white_to - POLISHED.white_from, 1e-6), 0, 1)
        keep = (1 - fade * fade * (3 - 2 * fade))[..., None]
        star = shown[..., None] + keep * (star - shown[..., None])
        rgb[box] += near * (star - rgb[box])
    return rgb


# The light of the stars the sensor clipped, for the renders of the natural preparation.  The renders hold every
# star back (``controlled_stars``: about half its displayed level); the brightest ones used to escape it by accident,
# StarNet2 leaving their ringed cores in the starless layer.  With a white core StarNet2 takes them out like any
# other star, and they came out as flat grey discs, dimmer than the medium stars.  Their light is given back after
# the render (``with_clipped_star_light``): white at the clipped core, then falling with the linear luminance, on the
# logarithm, down to the level the display stretch saturates at (``display_scale``), under which nothing changes.
# The natural display shows that whole span at its top, flat: the star's own profile is read on the linear image.
STAR_LIGHT_REACH = 3.0  # radii of the clipped core within which a pixel may take the star's light ...
STAR_LIGHT_MARGIN = 6.0  # ... plus this many px, the last half of the reach fading out: a bright nebula never does
# The light has the star's own colour, white at the core.  The colour of a clipped core is the clip levels', never the
# star's: it is read on the star's wings that the sensor did not clip, the first half of the reach, less the sky or
# nebula around it (the median of a ring ``STAR_COLOUR_RING`` px wide just past the reach, so the blue reflection nebula
# around a star is not taken for the star's colour).  Only the blue-orange part of that colour is kept: the optics
# spread red and blue wider than green, and the green-magenta part of a halo is that chromatism.  The share of each
# pixel's linear light that is the star's (above that sky) carries the colour, ``STAR_COLOUR`` of it, at most
# ``STAR_COLOUR_MAX``: a blue star's halo is bluish, an orange one's warm, a white one's white.
STAR_COLOUR = 0.75
STAR_COLOUR_MAX = 0.3
STAR_COLOUR_RING = 10
# Adding ``t`` along this axis keeps the luminance and ``(R + B) / 2 - G``, and moves ``B - R`` by ``t``.
BLUE_ORANGE_AXIS = np.array([-0.4298, 0.0702, 0.5702], np.float32)


def clipped_star_reaches(linear: np.ndarray, levels: list[float | None],
                         margin: float = STAR_LIGHT_MARGIN, radii: float = STAR_LIGHT_REACH) -> Iterator[tuple]:
    """Each core the sensor clipped (``near_clipped_cores``) and the reach around it: ``(box, cores, core, distance,
    reach, fade)`` -- the box (the reach and a ``STAR_COLOUR_RING`` around it), the labelled cores in it, this core's
    pixels, their distance to it, the reach (``radii`` radii of the core + ``margin`` px) and the fade (1 up to half
    the reach, then down to 0 at it)."""
    from scipy.ndimage import find_objects, label

    labelled, _ = label(near_clipped_cores(linear, levels), structure=np.ones((3, 3), bool))
    height, width = labelled.shape
    for index, found in enumerate(find_objects(labelled), start=1):
        if found is None:
            continue
        own = labelled[found] == index
        radius = float(np.sqrt(own.sum() / np.pi))
        reach = radii * radius + margin
        grow = int(np.ceil(reach)) + STAR_COLOUR_RING + 1
        box = (slice(max(found[0].start - grow, 0), min(found[0].stop + grow, height)),
               slice(max(found[1].start - grow, 0), min(found[1].stop + grow, width)))
        core = labelled[box] == index
        distance = distance_transform_edt(~core)
        x = np.clip((reach - distance) / (0.5 * reach), 0, 1)
        yield box, labelled[box], core, distance, reach, x * x * (3 - 2 * x)


def clipped_star_light(linear: np.ndarray, level: np.ndarray, scale: float,
                       levels: list[float | None]) -> np.ndarray | None:
    """The light of the clipped stars, height x width x 2: ``[..., 0]`` (0..1) how much of a clipped star's light each
    pixel takes back, ``[..., 1]`` the colour it takes along the blue-orange axis (``BLUE_ORANGE_AXIS``); None when
    no channel is clipped.  ``with_clipped_star_light`` gives it back.

    The light is 1 on the clipped core, ``log(level / scale) / log(core / scale)`` around it (``core``: the brightest
    luminance of the core), 0 where the luminance is under ``scale`` or farther than the reach from the core.  The
    colour is the star's (``STAR_COLOUR``), times the share of the pixel's linear light that is the star's.
    """
    if all(value is None for value in levels):
        return None
    light = np.zeros((*level.shape, 2), np.float32)
    for box, cores, core, distance, reach, fade in clipped_star_reaches(linear, levels):
        local = level[box]
        top = float(local[core].max())
        if top > scale * 1.01:
            share = np.log(np.maximum(local, 1e-12) / scale) / np.log(top / scale)
        else:
            share = (local >= scale).astype(np.float32)
        weight = np.clip(share, 0, 1) * fade
        light[box][..., 0] = np.maximum(light[box][..., 0], weight.astype(np.float32))
        measured = clipped_star_colour(linear[box], cores, core, distance, reach)
        if measured is None:
            continue
        colour, sky = measured
        sky_level = float(lum(sky[None])[0])
        own_light = np.clip((local - sky_level) / np.maximum(local, 1e-12), 0, 1)
        tint = (colour * own_light * fade).astype(np.float32)
        light[box][..., 1] = np.where(np.abs(tint) > np.abs(light[box][..., 1]), tint, light[box][..., 1])
    return light


def clipped_star_colour(local: np.ndarray, cores: np.ndarray, core: np.ndarray, distance: np.ndarray,
                        reach: float, share: float = STAR_COLOUR,
                        most: float = STAR_COLOUR_MAX) -> tuple[float, np.ndarray] | None:
    """A clipped star's colour along the blue-orange axis (``share`` of it, at most ``most``) and the
    sky around it, ``(colour, sky RGB)``, read on ``local`` (a linear box of ``clipped_star_reaches``): its unclipped
    wings, the first half of the reach, less the median of a ``STAR_COLOUR_RING`` just past it.  None when there is
    not enough of either to read."""
    others = (cores > 0) & ~core
    ring = (distance >= reach) & (distance < reach + STAR_COLOUR_RING) & ~others
    wings = (distance >= 1) & (distance < 0.5 * reach) & ~others
    if ring.sum() < 20 or not wings.any():
        return None
    sky = np.median(local[ring], axis=0)
    flux = (local[wings] - sky).sum(axis=0)
    total = float(lum(flux[None])[0])
    if total <= 0:
        return None
    return float(np.clip(share * (flux[2] - flux[0]) / total, -most, most)), sky


def with_clipped_star_light(image: np.ndarray, light: np.ndarray | None) -> np.ndarray:
    """A render (0..1) with the clipped stars' light given back (``clipped_star_light``): screened with white by
    its weight, then coloured along the blue-orange axis by the star's colour, the less the whiter the core.  Nothing
    changes where both are 0, nor when it is None."""
    if light is None:
        return image
    out = np.clip(image, 0, 1).astype(np.float32)
    lit = (light[..., 0] > 0) | (light[..., 1] != 0)
    weight = np.clip(light[lit, 0], 0, 1)[:, None]
    shown = 1 - (1 - out[lit]) * (1 - weight)
    level = lum(shown)
    shown = shown + (level * light[lit, 1] * (1 - weight[:, 0]))[:, None] * BLUE_ORANGE_AXIS
    out[lit] = into_gamut(np.clip(shown, 0, None), level)
    return out


def native_from_linear(linear: np.ndarray, display_scale: float | None = None,
                       light: list[np.ndarray | None] | None = None) -> np.ndarray:
    """The natural display stretch of a prepared linear image (what ``prepare_native`` ends with).

    Around the cores the sensor clipped, the core is white and the halo is never darkened under it
    (``white_star_cores``).  ``light`` (a list, when given) receives the clipped stars' light
    (``clipped_star_light``), for the render."""
    level = lum(linear)
    scale = display_scale if display_scale is not None else stretch_scale(linear)
    stretched = display_curve(level, scale, already_stretched(linear))
    rgb = linear * (stretched / np.maximum(level, 1e-8))[..., None]
    maximum = np.max(rgb, axis=2)
    rolled = np.where(maximum > 0.72, 0.72 + (maximum - 0.72) / (1 + 1.9 * (maximum - 0.72)), maximum)
    rgb *= rolled[..., None] / np.maximum(maximum[..., None], 1e-8)
    levels = clip_levels(linear)
    rgb = white_star_cores(linear, rgb, level, stretched, levels)
    if light is not None:
        light.append(clipped_star_light(linear, level, scale, levels))
    hsv = rgb2hsv(np.clip(rgb, 0, 1))
    hsv[..., 1] = np.clip(hsv[..., 1] * (1.0 + 0.10 * (1 - hsv[..., 1])), 0, 0.88)
    return hsv2rgb(hsv)


STARNET_MODEL_PATTERN = "StarNet2_weights_*.mlmodelc"


def starnet_temp_dirs() -> list[Path]:
    """The folders where StarNet2 leaves the model it compiles at every run (macOS: Core ML).

    The temporary folder of this process, and the system's own for the user (``getconf DARWIN_USER_TEMP_DIR``),
    which StarNet2 uses whatever ``TMPDIR`` says.
    """
    folders = [Path(tempfile.gettempdir())]
    if sys.platform == "darwin":
        with contextlib.suppress(AttributeError, OSError, ValueError):
            folders.append(Path(os.confstr("CS_DARWIN_USER_TEMP_DIR")))
    unique: list[Path] = []
    for folder in folders:
        if folder.is_dir() and folder not in unique:
            unique.append(folder)
    return unique


def starnet_models() -> set[Path]:
    return {model for folder in starnet_temp_dirs() for model in folder.glob(STARNET_MODEL_PATTERN)}


@contextlib.contextmanager
def starnet_cleanup() -> Iterator[None]:
    """Removes the compiled model (~120 MB) that StarNet2 left while the block ran, whatever its outcome.

    Only the models that were not there before are removed: a model of another run, or the person's own, stays.
    """
    before = starnet_models()
    try:
        yield
    finally:
        for model in starnet_models() - before:
            shutil.rmtree(model, ignore_errors=True)


def star_layer(
    native: np.ndarray,
    output: Path,
    stem: str,
    starnet: Path | None = None,
    log: IO[bytes] | None = None,
) -> tuple[np.ndarray, np.ndarray]:
    """Split stars from nebula with StarNet2; without it, keep a single layer.

    ``log``, when given, receives StarNet2's own output instead of the terminal.
    """
    if starnet is None:
        print("StarNet2 not given: star separation skipped, stars stay in the nebula layer")
        return np.clip(native, 0, 1), np.zeros_like(native)
    # StarNet2 needs at least two 256-pixel tiles.  A sequence registered with
    # a very conservative common overlap can be smaller than that; keep its
    # genuine pixels for the progression rather than failing or upscaling it.
    if min(native.shape[:2]) < 512:
        print("StarNet2 skipped: registered common field is smaller than 512 px")
        return np.clip(native, 0, 1), np.zeros_like(native)
    source = output / f"{stem}_pre_starnet.tif"
    starless_path = output / f"{stem}_starless.tif"
    mask_path = output / f"{stem}_starmask.tif"
    tifffile.imwrite(source, np.round(np.clip(native, 0, 1) * 65535).astype(np.uint16),
                     photometric="rgb", compression="deflate", software=software())
    with starnet_cleanup():
        subprocess.run([
            str(starnet), "-i", str(source), "-o", str(starless_path),
            "-m", str(mask_path), "-s", "256",
        ], check=True, stdout=log, stderr=subprocess.STDOUT if log is not None else None)
    try:
        array = tifffile.imread(starless_path)
    except ValueError as error:
        # StarNet2 writes LZW-compressed TIFFs. Pillow can read them when
        # imagecodecs is not installed in the environment.
        if "imagecodecs" not in str(error):
            raise
        array = np.asarray(Image.open(starless_path))
    starless = array.astype(np.float32) / np.iinfo(array.dtype).max
    # StarNet2 reads a three-channel image whose channels are all equal (a mono
    # master) as grayscale and writes a two-dimensional TIFF; repeat it into the
    # equal channels of ``native`` so that the subtraction lines up.
    if starless.ndim == 2 and native.ndim == 3:
        starless = np.repeat(starless[..., None], native.shape[2], axis=2)
    stars = np.maximum(native - starless, 0)
    return np.clip(starless, 0, 1), np.clip(stars, 0, 1)


# What StarNet2 leaves under a star.  Where it takes a star out, its starless image keeps a little of the pixel in a
# colour of its own: a centre without green (0.08 / 0.01 / 0.08 under a small yellow star of a real master), or a
# greenish patch where it took out the wide glow of a bright one.  The renders lift the faint signal of the starless
# layer (``deep_nebula_tone``) while they hold the stars back, so that colour shows: a violet centre on hundreds of
# unclipped stars, a green band beside the brightest.  Under the stars only, the starless layer keeps its luminance and
# takes the colour of the nebula or sky around them: what lies behind a star, not the star's own colour (the star
# layer gives that).  Where the star layer is empty, nothing moves.
UNDER_STARS_FROM = 0.1  # share of a pixel's display luminance in the star layer from which its starless colour is replaced ...
UNDER_STARS_TO = 0.5  # ... wholly from this share
UNDER_STARS_LEVEL = 0.05  # display luminance of the star layer (smoothed) at which a pixel is wholly under a star
UNDER_STARS_REACH = 6.0  # px: the width of the average the colour around the stars is read on
# The rim of a star.  The optics spread red and blue a little wider than green: on a real master the linear halo of a
# bright star, 6 to 10 px out, has red and blue 1.3 to 2.1 times its green, a lavender ring around a whiter centre, that
# every colour step makes stronger.  On the rim only (the star layer under ``RIM_TO`` of its brightest pixel within
# ``RIM_REACH`` px), its magenta (red and blue above green) is taken back by ``RIM_AMOUNT``; the colour between blue
# and red-orange, the star's own, is kept, and so is the core: a blue star stays blue, an orange one orange.
RIM_FROM = 0.5
RIM_TO = 0.95
RIM_REACH = 10
RIM_AMOUNT = 0.85
# The halo of a clipped star in the polished render (``clipped_star_halos``).  The optics colour it in rings: on a real
# master, red and blue stand above green 10 to 20 px from the centre of a bright star (a violet ring), green above
# them 25 to 60 px out (+0.10 to +0.14 of the luminance, three times the nebula's), and blue above red where the
# channels leave their clip levels.  The polished curve shows the whole halo bright, and its saturation and the strong
# contrast strengthen each ring: a white disc, a blue ring, then a green glow.  Over the halo -- ``HALO_RADII`` radii of
# the core + ``HALO_MARGIN`` px, fading out over its second half -- the star's light takes the star's own colour
# (``clipped_star_colour`` read whole, at most ``HALO_COLOUR_MAX``: white, bluish or warm), its luminance kept; the sky
# or nebula under the halo keeps its own.  The colour then goes continuously from the white core to the nebula.
HALO_RADII = 10.0
HALO_MARGIN = 40.0
HALO_COLOUR_MAX = 0.6
# Removing ``m`` of ``(R + B) / 2 - G`` along this axis keeps the luminance and ``B - R``.
MAGENTA_AXIS = np.array([0.7152, -0.2848, 0.7152], np.float32)


def under_stars(starless: np.ndarray, stars: np.ndarray) -> np.ndarray | None:
    """0..1: how much each pixel of the starless layer lies under a star (``UNDER_STARS_*``); None under none."""
    starless_level, star_level = lum(starless), lum(stars)
    share = gaussian_filter(star_level / np.maximum(star_level + starless_level, 1e-4), 1.0)
    x = np.clip((share - UNDER_STARS_FROM) / (UNDER_STARS_TO - UNDER_STARS_FROM), 0, 1)
    # The grain StarNet2 leaves in the star layer is no star: under a quarter of ``UNDER_STARS_LEVEL`` nothing moves.
    lit = np.clip(gaussian_filter(star_level, 1.0) / UNDER_STARS_LEVEL - 0.25, 0, 1)
    under = maximum_filter(x * x * (3 - 2 * x) * lit * lit * (3 - 2 * lit), size=3).astype(np.float32)
    return under if under.any() else None


def starless_around_stars(starless: np.ndarray, stars: np.ndarray) -> np.ndarray:
    """The starless layer with the colour StarNet2 left under the stars replaced by the colour of the nebula or sky
    around them (an average weighted away from the stars, ``UNDER_STARS_REACH`` px), at its own luminance."""
    under = under_stars(starless, stars)
    if under is None:
        return starless
    colour = starless - lum(starless)[..., None]
    away = 1 - under
    around = (gaussian_filter(colour * away[..., None], (UNDER_STARS_REACH, UNDER_STARS_REACH, 0))
              / (gaussian_filter(away, UNDER_STARS_REACH)[..., None] + 0.02))
    return np.clip(starless + under[..., None] * (around - colour), 0, 1).astype(np.float32)


def star_rim(stars: np.ndarray) -> np.ndarray:
    """0..``RIM_AMOUNT``: how much of its magenta each pixel of a star's rim loses (``without_magenta``)."""
    level = lum(stars)
    peak = maximum_filter(gaussian_filter(level, 1.0), size=2 * RIM_REACH + 1)
    x = np.clip((level / np.maximum(peak, 1e-6) - RIM_FROM) / (RIM_TO - RIM_FROM), 0, 1)
    return (RIM_AMOUNT * (1 - x * x * (3 - 2 * x)) * (level > 0)).astype(np.float32)


def without_magenta(rgb: np.ndarray, amount: np.ndarray) -> np.ndarray:
    """``rgb`` with ``amount`` (per pixel) of its magenta (red and blue above green) taken back, its luminance and
    ``B - R`` kept.  A greener pixel is left as it is: taking its green back would turn a yellow star orange."""
    magenta = np.maximum(0.5 * (rgb[..., 0] + rgb[..., 2]) - rgb[..., 1], 0)
    return np.clip(rgb - (amount * magenta)[..., None] * MAGENTA_AXIS, 0, 1).astype(np.float32)


def clipped_star_halos(image: np.ndarray, linear: np.ndarray,
                       levels: list[float | None]) -> tuple[np.ndarray, np.ndarray] | None:
    """The clipped stars' halos (``HALO_RADII``): ``(added, weight)``, what each pixel of ``image`` gains to take its
    star's colour (RGB, its luminance 0) and how much it lies in a halo (0..1, the fade of the nearest); None when no
    channel is clipped.  ``image`` (linear, the two layers summed) is what the colours
    are read on, ``linear`` the master the clipped cores are found on.

    A halo's light at a distance from its core is the mean of that circle less the sky around the halo (the median of a
    ``STAR_COLOUR_RING`` past it), a field star on the circle left out: the field star is not the halo's, and keeps its
    colour.  That light is given the star's colour at the same luminance."""
    from scipy.ndimage import mean, median

    if all(value is None for value in levels):
        return None
    added = np.zeros(image.shape, np.float32)
    weight = np.zeros(image.shape[:2], np.float32)
    claimed = np.zeros(image.shape[:2], np.float32)
    largest = np.zeros(image.shape[:2], np.float32)
    floor = np.full(image.shape[:2], np.inf, np.float32)
    for (box, cores, core, distance, reach, _), (halo, labelled, own, far, extent, fade) in zip(
            clipped_star_reaches(linear, levels), clipped_star_reaches(linear, levels, HALO_MARGIN, HALO_RADII)):
        local = image[halo]
        others = (labelled > 0) & ~own
        ring = (far >= extent) & (far < extent + STAR_COLOUR_RING) & ~others
        if ring.sum() < 20:
            continue
        sky = np.median(local[ring], axis=0)
        circles = np.rint(far).astype(np.int32)
        inside = (far < extent) & ~others
        index = np.arange(int(np.ceil(extent)) + 1)
        # The mean of each circle, without what stands above 1.5 times its median luminance (a field star).
        local_level = lum(local)
        typical = np.nan_to_num(np.asarray(median(local_level, np.where(inside, circles, -1), index)))
        kept = np.where(inside & (local_level <= 1.5 * typical[np.minimum(circles, index[-1])]), circles, -1)
        light = np.clip(np.stack([np.nan_to_num(np.asarray(mean(local[..., c], kept, index)))
                                  for c in range(3)], axis=-1) - sky, 0, None)
        measured = clipped_star_colour(image[box], cores, core, distance, reach, 1.0, HALO_COLOUR_MAX)
        star = 1 + (0.0 if measured is None else measured[0]) * BLUE_ORANGE_AXIS
        change = (lum(light)[:, None] * star[None] - light).astype(np.float32)
        added[halo] += np.where(inside[..., None], change[np.minimum(circles, index[-1])], 0) * fade[..., None]
        weight[halo] = np.maximum(weight[halo], fade)
        claim = np.where(inside, lum(light)[np.minimum(circles, index[-1])], 0).astype(np.float32) * fade
        claimed[halo] += claim
        largest[halo] = np.maximum(largest[halo], claim)
        floor[halo] = np.where(claim > 0, np.minimum(floor[halo], float(lum(sky[None])[0])), floor[halo])
    # Where the halos of a group of bright stars overlap, each one reads the light of the others on its circles and
    # recolours it too: summed, the changes took the violet of the group out several times over and left a green
    # glow between the stars.  There the light the halos claim together is held to what the pixel has above the
    # sky around them.  A halo alone is never scaled: the hold comes in with the share of the claims that is not
    # the largest one's (wholly from a quarter).
    own = np.clip(lum(image) - np.where(np.isfinite(floor), floor, 0), 0, None)
    overlap = np.clip((1 - largest / np.maximum(claimed, 1e-12)) / 0.25, 0, 1)
    held = (largest + overlap * (own - largest)) / np.maximum(claimed, 1e-12)
    added *= np.clip(held, 0, 1)[..., None]
    return (added, weight) if weight.any() else None


def with_halo_colours(layer: np.ndarray, whole: np.ndarray, added: np.ndarray) -> np.ndarray:
    """A linear layer with its share (of the luminance of ``whole``, the two layers summed) of the halos' change of
    colour (``clipped_star_halos``)."""
    share = lum(layer) / np.maximum(lum(whole), 1e-12)
    return np.clip(layer + share[..., None] * added, 0, None).astype(np.float32)


def star_rims(stars: np.ndarray) -> np.ndarray:
    """The star layer with the magenta of the stars' rims taken back (``RIM_AMOUNT``)."""
    return without_magenta(stars, star_rim(stars))


def natural_split(starless: np.ndarray, stars: np.ndarray) -> tuple[np.ndarray, np.ndarray]:
    """StarNet2's split for the renders of the natural preparation: the star layer's rims cleaned (``star_rims``); the
    colour under the stars is cleaned by each render (``starless_around_stars``).  A split
    whose star layer is empty is returned as it is."""
    if not np.any(stars > 0):
        return starless, stars
    return starless, star_rims(stars)


def controlled_stars(stars: np.ndarray, value: float) -> np.ndarray:
    star_lum = lum(stars)
    chroma = gaussian_filter(stars - star_lum[..., None], sigma=(1.05, 1.05, 0))
    result = np.clip(star_lum[..., None] + 0.58 * chroma, 0, 1)
    hsv = rgb2hsv(result)
    hsv[..., 1] = np.clip(hsv[..., 1] * 0.72, 0, 0.58)
    hsv[..., 2] = np.clip(hsv[..., 2] * value, 0, 0.88)
    return hsv2rgb(hsv)


def natural_v23(nebula: np.ndarray, stars: np.ndarray) -> np.ndarray:
    nebula = nebula.copy()
    level = lum(nebula)
    black = np.percentile(level, 1.0)
    x = np.clip((level - black) / (1 - black + 1e-12), 0, 1)
    target = np.clip(1.08 * np.power(x, 0.75), 0, 1)
    nebula *= target[..., None] / np.maximum(level[..., None], 1e-6)
    return np.clip(1 - (1 - np.clip(nebula, 0, 1)) * (1 - controlled_stars(stars, 0.58)), 0, 1)


def deep_nebula_tone(nebula: np.ndarray) -> np.ndarray:
    """Reveal faint integrated signal while protecting highlights."""
    nebula = nebula.copy()
    level = lum(nebula)
    shadow = 0.35 * np.percentile(level, 0.1)
    x = np.clip((level - shadow) / (1 - shadow + 1e-12), 0, 1)
    target = np.clip(1.08 * np.power(x, 0.71), 0, 1)
    local = target - gaussian_filter(target, sigma=24)
    protection = np.clip(1 - target / 0.72, 0, 1)
    target = np.clip(target + 0.08 * local * protection, 0, 1)
    nebula *= target[..., None] / np.maximum(level[..., None], 1e-6)
    return np.clip(nebula, 0, 1)


def natural_deep(nebula: np.ndarray, stars: np.ndarray) -> np.ndarray:
    """Natural deep reference with a soft toe and restrained stars."""
    nebula = deep_nebula_tone(starless_around_stars(nebula, stars))
    return np.clip(
        1 - (1 - nebula) * (1 - controlled_stars(stars, 0.52)),
        0,
        1,
    )


def dualband_palette(nebula: np.ndarray) -> np.ndarray:
    """Apply the continuous dual-band colour correction."""
    nebula = np.clip(nebula, 0, 1)
    hsv = rgb2hsv(nebula)
    hue, saturation = hsv[..., 0], hsv[..., 1]
    circular_distance = lambda h, c: np.minimum(np.abs(h - c), 1 - np.abs(h - c))
    red = np.exp(-0.5 * (circular_distance(hue, 0.0) / 0.15) ** 2)
    green = np.exp(-0.5 * (circular_distance(hue, 0.34) / 0.16) ** 2)
    cyan = np.exp(-0.5 * (circular_distance(hue, 0.50) / 0.16) ** 2)
    delta = (-0.075 * red + 0.205 * green + 0.085 * cyan) / (red + green + cyan + 1e-6)
    hsv[..., 0] = (hue + delta * (0.38 + 0.42 * np.sqrt(np.clip(saturation, 0, 1)))) % 1.0
    hsv[..., 1] = np.clip(saturation * (0.94 + 0.12 * (1 - saturation)), 0, 0.72)
    nebula = hsv2rgb(hsv)
    before = lum(nebula)
    nebula *= np.array([1.045, 0.915, 1.16], np.float32)[None, None, :]
    nebula *= before[..., None] / np.maximum(lum(nebula)[..., None], 1e-6)
    level = lum(nebula)
    return np.clip(
        level[..., None]
        + gaussian_filter(nebula - level[..., None], sigma=(0.75, 0.75, 0)),
        0,
        1,
    )


def dualband_v7(nebula: np.ndarray, stars: np.ndarray) -> np.ndarray:
    nebula = nebula.copy()
    level = lum(nebula)
    black = np.percentile(level, 1.0)
    x = np.clip((level - black) / (1 - black + 1e-12), 0, 1)
    target = np.clip(1.10 * np.power(x, 0.73), 0, 1)
    nebula *= target[..., None] / np.maximum(level[..., None], 1e-6)
    nebula = dualband_palette(nebula)
    return np.clip(1 - (1 - nebula) * (1 - controlled_stars(stars, 0.64)), 0, 1)


def dualband_deep(nebula: np.ndarray, stars: np.ndarray) -> np.ndarray:
    """Dual-band correction on top of the validated deep luminance treatment."""
    nebula = dualband_palette(deep_nebula_tone(starless_around_stars(nebula, stars)))
    return np.clip(1 - (1 - nebula) * (1 - controlled_stars(stars, 0.58)), 0, 1)


# --------------------------------------------------------------------------
# "Polished render" (pipeline ``polished``).  It needs its own linear
# preparation (``prepare_polished``), named for its render in ``PREPARATIONS``:
#
# (a) background by GraXpert's background extraction (AI model, pinned), or
#     Siril's ``subsky -rbf``; (b) the denoising of the same engine, on the
#     linear image (Siril's at full dose); (c) the natural pipeline's neutral
#     sky; (d) the masked deconvolution of the natural pipeline (measured PSF);
#     (e) a display image, for StarNet2 alone (``polished_stretch``).
#
# Once StarNet2 has split it, ``polished_layers`` carries the split back to
# linear light (a dark ring the deconvolution dug around a star is filled with
# StarNet2's nebula) and makes the render's own two layers (RENDER-R5-01):
#
# (f) colour, by what is known of the capture's band (``CAPTURE_BANDS``):
#     the median field star made white (``star_white_gains``: one star, one
#     voice, so a few very bright stars of one colour are not the reference)
#     and the sky neutral; on a broadband
#     capture the green of the nebula is taken back (SCNR, full); a dual-band
#     capture keeps its colours (its green is OIII) and so does a capture whose
#     band is not known for certain;
# (g) over the halo of each core the sensor clipped (``clipped_star_halos``),
#     the halo's light takes the star's own colour, the luminance kept: the
#     optics colour it in rings (violet near the core, green farther out) that
#     the split carries into the nebula layer and every later step
#     strengthens; then the clipped cores are made neutral (``clipped_cores``):
#     their colour is the clip levels', never the star's.  A master without a
#     clipped core is rendered as before; under and just around the small
#     stars the nebula layer takes the colour of what surrounds them
#     (``nebula_around_stars``), the star layer holds the star's own;
# (h) a midtones transfer whose black point stands ``black_sigmas`` noise
#     widths under the sky and that puts the sky at ``sky`` (no flat black: the
#     grain stays above 0), the same curve for the nebula and for the stars;
#     a channel the curve would push past 1 is brought towards white at the
#     same luminance (``into_gamut``): a bright, coloured halo is never
#     darkened under the white core it surrounds; around a clipped star the
#     highlights fade to white, and the core itself is white;
# (i) the midtones lifted (a power on the signal above the sky), more when the
#     bright part of the signal is a faint object (a galaxy) than a bright
#     nebula, then a non-local-means smoothing of the nebula, lighter where the
#     signal is strong (its texture is kept), and the sky put back at its level;
# (j) a local contrast, then the bright core of a nebula (M42) bent under the
#     white at large scale, its fine structure and its colour kept;
# (k) the colour of the signal strengthened, the sky left neutral; the halos
#     of the clipped stars brought towards white just enough that the strong
#     contrast lifts them whole (``halos_within_contrast``);
# (l) in ``polished_deep``: the stars put back nearly at full strength, their
#     colour held back (pale: blue-white to orange, never a saturated dot).
#
# Each step reads its figure from ``PolishedSettings`` (``POLISHED``): the
# render's settings are a value, not constants spread through the code.
#
# Without GraXpert the pipeline still runs: Siril, else the polynomial
# background and no denoising, and it says so (``prepare_polished`` returns
# what it did).

# The GraXpert models are pinned: the same model gives the same image, and a
# pinned model already downloaded needs no network (« latest » asks the
# server for the list each time).
GRAXPERT_BACKGROUND_MODEL = "1.0.1"
GRAXPERT_DENOISE_MODEL = "3.0.2"
GRAXPERT_SMOOTHING = 0.5  # 0..1: a moderate smoothing of the background model
GRAXPERT_DENOISE_STRENGTH = 0.3  # 0..1 (GraXpert's default is 0.5): the light step
SHOULDER_KNEE = 0.70  # display level above which the highlights are compressed (StarNet2's display)
# (d) The natural pipeline deconvolves only when ``--deconv-fwhm`` asks for it;
# the polished one aims at the measured FWHM times this ratio by itself, and
# leaves an undersampled field alone: below the floor ``deconvolve_linear``
# warns about, the stars would square off.
POLISHED_DECONV_RATIO = 0.85
POLISHED_DECONV_FLOOR = 2.0  # 2.2 left real masters (2.4 to 2.5 px) undeconvolved

# The engine of the background and of the denoising (``recipe.render.polished.engine``):
# GraXpert's AI models (the default), or Siril's own commands, with no model and no
# dependency beyond Siril -- ``subsky -rbf`` for the background, ``denoise`` (NL-Bayes)
# for the noise.  Siril is also the fallback when GraXpert is missing or fails.
POLISHED_ENGINES = ("graxpert-ai", "siril")
DEFAULT_POLISHED_ENGINE = "graxpert-ai"
# Siril's own defaults (20 samples a row, tolerance 1.0, smoothing 0.5) take the faint,
# extended Hα halo of a large nebula for sky and subtract it, colour included; 10 samples,
# tolerance 0.5 and the strongest smoothing follow the gradient like the natural plane
# does and leave the halo (measured on real masters).
SIRIL_RBF_SAMPLES = 10
SIRIL_RBF_TOLERANCE = 0.5
SIRIL_RBF_SMOOTHING = 1.0
# 0..1 (Siril's default is 1).  Siril's ``-mod`` is a linear blend of the image and its full denoising; the
# full dose costs nothing more and takes 16 to 27 % more noise away (RENDER-STUDY-01).
SIRIL_DENOISE_MODULATION = 1.0

# What is known of the band of a capture (``capture_band`` of the stacking script): only a capture known to be
# broadband loses the green of its nebula; a dual-band one keeps it (OIII), and so does one whose band is not
# known for certain.
CAPTURE_BANDS = ("broadband", "dual-band", "unknown")


@dataclass(frozen=True)
class PolishedSettings:
    """The figures of the polished render, step by step (RENDER-STUDY-01's R5, corrected in RENDER-R5-01)."""

    sky: float = 0.03  # (h) the displayed sky level
    black_sigmas: float = 2.8  # (h) the black point, in noise widths of the sky under it
    shoulder: float = 0.80  # (h) the display level above which the highlights are compressed
    lift: float = 0.85  # (i) the power on the signal above the sky (1: none, lower: the midtones lifted)
    lift_strongest: float = 0.65  # (i) the strongest power a faint image may get
    lift_target: float = 0.45  # (i) the level the bright part of the signal (99.5th percentile, smoothed) is lifted to, at most
    lift_area: float = 0.15  # (i) ... only when its upper half covers less of the frame than this (an object, not a glow)
    nebula_knee: float = 0.6  # (j) the large-scale level of the nebula above which its highlights are compressed
    nebula_ceiling: float = 0.78  # (j) ... towards this level
    white_from: float = 0.75  # (h) displayed luminance from which the colours of a clipped star fade towards white
    white_to: float = 0.97  # (h) ... and are gone
    smoothing: float = 0.8  # (j) the non-local means' strength, in noise widths of the sky (0: none)
    smoothing_kept: float = 0.5  # (j) the share of the texture given back where the signal is strong
    local_contrast: float = 0.35  # (j) the local contrast of the nebula (24 px)
    saturation: float = 1.3  # (k) the factor on the chroma of the signal
    green: float = 1.0  # (f) the SCNR on the nebula of a broadband capture
    stars: float = 0.9  # (l) the strength the stars are put back with
    star_colour: float = 0.6  # (l) the factor on the chroma of the stars
    core_from: float = 0.90  # (g) a pixel belongs to a clipped core from this share of its channel's clip level
    core_to: float = 0.97  # (g) ... and wholly at this share


POLISHED = PolishedSettings()


def polished_deconv_target(linear: np.ndarray, measured: float | None = None) -> float | None:
    """The FWHM the polished deconvolution aims at; None when the stars cannot be
    measured or are already too narrow (the target would fall under the floor).

    ``measured``: the FWHM read on the master before any denoising; a denoising
    engine narrows the stars a little (Siril's by 0.05 to 0.1 px) and the target
    must not depend on the engine.  Without it, the stars of ``linear`` are measured."""
    start = float(measured) if measured is not None and np.isfinite(measured) else star_fwhm(lum(linear))
    if not np.isfinite(start):
        return None
    target = POLISHED_DECONV_RATIO * float(start)
    if target < POLISHED_DECONV_FLOOR:
        print(f"deconvolution skipped: stars of {start:.2f} px, the field is undersampled")
        return None
    return target


def run_graxpert(
    graxpert: Path,
    command: str,
    source: Path,
    output: Path,
    options: list[str],
    log: IO[bytes] | None = None,
) -> Path:
    """One GraXpert command-line operation on a linear FITS; the ``<output>.fits`` it wrote.

    ``output`` is a stem without extension (GraXpert adds ``.fits``), made
    absolute: GraXpert joins a relative one to the folder of its input, and
    nothing is ever written beside the master.  ``-cli`` is given although
    GraXpert 3.1 made it optional: 3.0 needs it.
    """
    output = output.resolve()
    run = subprocess.run(
        [str(graxpert), "-cli", "-cmd", command, *options, "-output", str(output), str(source)],
        stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
    )
    if log is not None:
        log.write(run.stdout)
        log.flush()
    if run.returncode != 0:
        raise RuntimeError(graxpert_failure(run.returncode, run.stdout.decode("utf-8", "replace")))
    written = output.with_name(output.name + ".fits")
    if not written.is_file():
        raise RuntimeError(f"GraXpert wrote no {written.name}")
    return written


def graxpert_failure(code: int, said: str) -> str:
    """Why GraXpert failed, in words the person can act on.

    A GraXpert installed with ``pip`` or ``uv`` without ``onnxruntime`` answers its version (the detection finds
    it) and then fails at its first command: that is said as such (a real night fell back on Siril so, with only
    an exit status in its record).  Otherwise the last error line GraXpert wrote, else its exit status.
    """
    if "onnxruntime" in said or "ONNX Runtime" in said:
        return ("GraXpert is installed without its AI library (onnxruntime): install it from its page, or add "
                "onnxruntime to the environment GraXpert was installed in")
    errors = [line.strip() for line in said.splitlines() if "ERROR" in line or "Error" in line]
    return f"GraXpert stopped with status {code}" + (f": {errors[-1][-200:]}" if errors else "")


def graxpert_linear(
    source: Path,
    graxpert: Path,
    work: Path,
    stem: str,
    log: IO[bytes] | None = None,
) -> Path:
    """(a) and (b): GraXpert's background extraction, then its light denoising."""
    work.mkdir(parents=True, exist_ok=True)
    background = run_graxpert(
        graxpert, "background-extraction", source, work / f"{stem}_graxpert_background",
        ["-ai_version", GRAXPERT_BACKGROUND_MODEL, "-correction", "Subtraction",
         "-smoothing", str(GRAXPERT_SMOOTHING)], log,
    )
    return run_graxpert(
        graxpert, "denoising", background, work / f"{stem}_graxpert_denoised",
        ["-ai_version", GRAXPERT_DENOISE_MODEL, "-strength", str(GRAXPERT_DENOISE_STRENGTH)], log,
    )


def siril_linear(
    source: Path,
    siril: Path,
    work: Path,
    stem: str,
    log: IO[bytes] | None = None,
) -> Path:
    """(a) and (b) with Siril alone: ``subsky -rbf`` then ``denoise``; the FITS it wrote in ``work``.

    No model and nothing to download.  Siril runs in ``work`` (its working
    folder) and reads the master by its absolute path; nothing is written
    beside the master.
    """
    work.mkdir(parents=True, exist_ok=True)
    work = work.resolve()
    produced = work / f"{stem}_siril_denoised"
    script = work / f"{stem}_siril_linear.ssf"
    script.write_text(
        "requires 1.2.0\n"
        f'load "{source.resolve()}"\n'
        f"subsky -rbf -samples={SIRIL_RBF_SAMPLES} -tolerance={SIRIL_RBF_TOLERANCE} -smooth={SIRIL_RBF_SMOOTHING}\n"
        f"denoise -mod={SIRIL_DENOISE_MODULATION}\n"
        f'save "{produced}"\n',
        encoding="utf-8",
    )
    subprocess.run(
        siril_config.command(siril, work, script),
        check=True, stdout=log, stderr=subprocess.STDOUT if log is not None else None,
    )
    for suffix in (".fit", ".fits", ".fts"):
        written = produced.with_name(produced.name + suffix)
        if written.is_file():
            return written
    raise RuntimeError(f"Siril wrote no {produced.name}.fit")


def neutral_sky(linear: np.ndarray) -> np.ndarray:
    """The natural pipeline's neutral background (the same gains as ``calibrated_linear``)."""
    sky = np.percentile(linear, 22, axis=(0, 1))
    neutral = np.exp(np.mean(np.log(np.maximum(sky, 1e-12))))
    gain = np.clip(neutral / np.maximum(sky, 1e-12), 0.86, 1.16)
    return linear * gain[None, None, :]


def scnr_green(rgb: np.ndarray, amount: float) -> np.ndarray:
    """SCNR, average neutral protection: green above the mean of red and blue is taken back by ``amount``.

    A pixel whose green does not exceed that mean (grey, red, blue, violet) is
    left as it is; only a green cast moves.
    """
    if not 0 <= amount <= 1:
        raise ValueError("SCNR amount must be between 0 and 1")
    out = np.array(rgb, dtype=np.float32, copy=True)
    neutral = 0.5 * (out[..., 0] + out[..., 2])
    excess = np.maximum(out[..., 1] - neutral, 0)
    out[..., 1] -= amount * excess
    return out


def soft_shoulder(x: np.ndarray, knee: float = SHOULDER_KNEE) -> np.ndarray:
    """Identity below ``knee``; above it, an exponential approach to 1 with the same slope at the knee.

    Increasing and continuous, it never reaches 1: values the natural stretch
    would clip (bright cores) keep their gradation.
    """
    x = np.asarray(x, dtype=np.float32)
    room = 1.0 - knee
    above = knee + room * (1.0 - np.exp(-(x - knee) / room))
    return np.where(x > knee, above, x).astype(np.float32)


def prepare_polished(
    source: Path,
    graxpert: Path | None = None,
    work: Path | None = None,
    stem: str = "polished",
    log: IO[bytes] | None = None,
    gradient_degree: int = 2,
    black_percentile: float = 0.25,
    deconv_fwhm: float | None = None,
    engine: str = DEFAULT_POLISHED_ENGINE,
    siril: Path | None = None,
    imposed: dict[str, object] | None = None,
    measured: dict[str, object] | None = None,
) -> tuple[np.ndarray, np.ndarray, dict[str, object]]:
    """Linear steps (a)-(d) and the display of (e); ``(linear image, display image, what was done)``.

    The black point and the stretch scale of the display are measured on the master unless ``imposed`` gives
    them (``{"black": [r, g, b], "scale": s, ...}``, what a render of the same plan recorded: a milestone of the
    progression is shown at the level of the image of the plan); ``measured`` (when given) receives the ones used.

    ``engine`` picks the background and the denoising (``POLISHED_ENGINES``).
    ``graxpert-ai`` falls back on Siril when GraXpert is not given or fails,
    and any engine falls back on the polynomial (no denoising) when nothing
    else can run.

    ``what was done``: ``{"engine": "graxpert-ai" | "siril" | "polynomial",
    "background": "graxpert" | "siril" | "polynomial", "denoise": "graxpert" |
    "siril" | None, "deconvolutionFwhm": target in px | None, ...}`` -- the
    provenance says it, as it says whether StarNet2 separated the stars.
    ``engineRequested`` and ``fallbackReason`` appear when the engine used is
    not the one asked for.
    """
    if engine not in POLISHED_ENGINES:
        raise ValueError(f"unknown polished engine '{engine}': {', '.join(POLISHED_ENGINES)}")
    if (graxpert is not None or siril is not None) and work is None:
        raise ValueError("GraXpert and Siril need a work folder: nothing is written beside the master")
    raw = read_linear_rgb(source)
    master_fwhm = star_fwhm(lum(raw)) if deconv_fwhm is None else None
    imposed = imposed or {}
    done: dict[str, object] = {}
    cleaned: Path | None = None
    used = "polynomial"
    reasons: list[str] = []
    for candidate in (["siril"] if engine == "siril" else ["graxpert-ai", "siril"]):
        try:
            if candidate == "graxpert-ai":
                if graxpert is None:
                    reasons.append("GraXpert not given")
                    print("GraXpert not given: " + ("Siril background and denoising" if siril is not None
                                                    else "polynomial background, no denoising"))
                    continue
                cleaned = graxpert_linear(source, graxpert, work, stem, log)
            else:
                if siril is None:
                    reasons.append("Siril not given")
                    print("Siril not given: polynomial background, no denoising")
                    continue
                cleaned = siril_linear(source, siril, work, stem, log)
            used = candidate
            break
        except (subprocess.CalledProcessError, RuntimeError, OSError) as error:
            reasons.append(f"{'GraXpert' if candidate == 'graxpert-ai' else 'Siril'} failed: {error}")
            if candidate == "graxpert-ai":
                done["graxpertFailed"] = str(error)
                print(f"GraXpert failed ({error}): " + ("Siril background and denoising" if siril is not None
                                                        else "polynomial background, no denoising"))
            else:
                done["sirilFailed"] = str(error)
                print(f"Siril failed ({error}): polynomial background, no denoising")
    if cleaned is not None:
        cleaned_raw = read_linear_rgb(cleaned)
        black = (black_point(cleaned_raw, black_percentile) if imposed.get("black") is None
                 else np.asarray(imposed["black"], dtype=cleaned_raw.dtype))
        if measured is not None:
            measured["black"] = [float(v) for v in black]
        linear = np.clip(cleaned_raw - black, 0, None)
        tool = "graxpert" if used == "graxpert-ai" else "siril"
        done.update({"background": tool, "denoise": tool})
    else:
        linear = remove_gradient(raw, degree=gradient_degree, black_percentile=black_percentile,
                                 black=imposed.get("black"), measured=measured, display=True)
        done.update({"background": "polynomial", "denoise": None})
    done["engine"] = used
    if master_fwhm is not None and np.isfinite(master_fwhm):
        done["masterFwhm"] = round(float(master_fwhm), 2)
    if used != engine:
        done["engineRequested"] = engine
        done["fallbackReason"] = "; ".join(reasons)
    linear, display = polished_display(linear, deconv_fwhm, done, master_fwhm, imposed.get("scale"), measured)
    return linear, display, done


def polished_display(
    linear: np.ndarray,
    deconv_fwhm: float | None = None,
    done: dict[str, object] | None = None,
    master_fwhm: float | None = None,
    scale: float | None = None,
    measured: dict[str, object] | None = None,
) -> tuple[np.ndarray, np.ndarray]:
    """Steps (c), (d) and the display of (e) on a linear image whose background is already removed.

    ``(linear image, display image)``: the first is what ``polished_layers`` renders, the second what StarNet2
    reads.  ``scale``: the stretch scale to use instead of the image's own (``stretch_scale``); ``measured``
    (when given) receives the one used.  ``master_fwhm``: the stars' FWHM on the master, before any denoising,
    that the deconvolution target is read from.  ``done`` (when given) receives the deconvolution target.
    """
    done = {} if done is None else done
    linear = neutral_sky(linear)
    target = deconv_fwhm if deconv_fwhm is not None else polished_deconv_target(linear, master_fwhm)
    if target is not None:
        try:
            linear = deconvolve_linear(linear, target)
        except RuntimeError as error:  # too few isolated stars for a PSF: the image goes on undeconvolved
            print(f"deconvolution skipped: {error}")
    done["deconvolutionFwhm"] = None if target is None else round(float(target), 2)
    scale = stretch_scale(linear) if scale is None else scale
    if measured is not None:
        measured["scale"] = float(scale)
    return linear.astype(np.float32), polished_stretch(linear, scale)


def polished_stretch_level(level: np.ndarray, scale: float, stretched: bool) -> np.ndarray:
    """The luminance curve of ``polished_stretch``: the natural stretch, then the soft shoulder."""
    return soft_shoulder(display_curve(level, scale, stretched))


def polished_stretch_inverse(shown: np.ndarray, scale: float, stretched: bool) -> np.ndarray:
    """The linear luminance ``polished_stretch_level`` shows at ``shown`` (its inverse, below 1)."""
    shown = np.clip(np.asarray(shown, dtype=np.float64), 0, 0.9999)
    room = 1.0 - SHOULDER_KNEE
    x = np.where(shown > SHOULDER_KNEE, SHOULDER_KNEE - room * np.log(1 - (shown - SHOULDER_KNEE) / room), shown)
    x = np.clip(x, 0, 1)
    if stretched:
        return x * scale
    return np.sinh(x * np.arcsinh(18.0)) * scale / 18.0


def polished_stretch(linear: np.ndarray, scale: float | None = None) -> np.ndarray:
    """The natural arcsinh stretch with a soft shoulder before the natural roll: the display StarNet2 reads in (e)."""
    level = lum(linear)
    scale = stretch_scale(linear) if scale is None else scale
    stretched = polished_stretch_level(level, scale, already_stretched(linear))
    rgb = linear * (stretched / np.maximum(level, 1e-8))[..., None]
    maximum = np.max(rgb, axis=2)
    over = np.maximum(maximum - 0.72, 0)  # the natural roll, without dividing by zero below it
    rolled = np.where(maximum > 0.72, 0.72 + over / (1 + 1.9 * over), maximum)
    rgb *= rolled[..., None] / np.maximum(maximum[..., None], 1e-8)
    hsv = rgb2hsv(np.clip(rgb, 0, 1))
    hsv[..., 1] = np.clip(hsv[..., 1] * (1.0 + 0.10 * (1 - hsv[..., 1])), 0, 0.88)
    return hsv2rgb(hsv)


def linear_split(linear: np.ndarray, display: np.ndarray, starless: np.ndarray,
                 scale: float) -> tuple[np.ndarray, np.ndarray]:
    """StarNet2's split of ``display`` carried back to linear light: ``(nebula, stars)``.

    The share of each pixel's displayed luminance StarNet2 left in the starless image is taken back through the
    inverse of the display curve: the linear nebula is the linear luminance that shows at that share.  Where the
    deconvolution dug a dark ring around a star (``linear`` under what StarNet2 sees as the nebula there), the
    nebula keeps StarNet2's level -- the ring is not carried into the render -- and the star layer is what
    stands above it.
    """
    level = lum(linear)
    stretched = already_stretched(linear)
    shown_starless = lum(starless)
    shown = polished_stretch_level(level, scale, stretched)
    ratio = shown_starless / np.maximum(lum(display), 1e-6)
    # Under the display (a pixel StarNet2 lowered): the share of the pixel's own display.  Above it (a dark ring the
    # display shows, StarNet2 filled): StarNet2's own level, up to 2.5 times the pixel.
    wanted = np.where(ratio <= 1, shown * np.clip(ratio, 0, 1), shown_starless)
    nebula_level = polished_stretch_inverse(wanted, scale, stretched)
    share = np.clip(nebula_level / np.maximum(level, 1e-12), 0, 2.5).astype(np.float32)
    nebula = (linear * share[..., None]).astype(np.float32)
    return nebula, np.clip(linear - nebula, 0, None).astype(np.float32)


# The colour of the linear nebula layer around the stars.  ``linear_split`` gives each pixel's own colour to both
# layers: under a star, and on the ring the deconvolution digs around it (deeper in green and blue than in red), the
# nebula layer holds the star's colour at the nebula's level.  The render then strengthens the colours of the nebula
# and holds the stars' back: a dark orange rim around each small star, a brown ring on a blue nebula.  There
# (``under_stars`` of StarNet2's split, widened by ``SPLIT_UNDER_REACH`` px for the ring) the nebula layer keeps its
# luminance and takes the colour of the nebula or sky around the star, as ``starless_around_stars`` does for the
# natural renders; the star layer, what stands above it, then holds the star's colour.
SPLIT_UNDER_REACH = 4


def nebula_around_stars(nebula: np.ndarray, display: np.ndarray, starless: np.ndarray) -> np.ndarray:
    """The linear nebula layer with, under and just around the stars StarNet2 took out of ``display``, the colour of
    what surrounds them (an average weighted away from the stars, ``UNDER_STARS_REACH`` px), its luminance kept.
    Nothing moves away from the stars, nor in the middle of a star too wide for its surroundings to be read."""
    under = under_stars(starless, np.clip(display - starless, 0, 1))
    if under is None:
        return nebula
    yy, xx = np.mgrid[-SPLIT_UNDER_REACH:SPLIT_UNDER_REACH + 1, -SPLIT_UNDER_REACH:SPLIT_UNDER_REACH + 1]
    disc = (yy * yy + xx * xx) <= SPLIT_UNDER_REACH * SPLIT_UNDER_REACH
    under = np.clip(gaussian_filter(maximum_filter(under, footprint=disc), 1.0), 0, 1)
    level = lum(nebula)
    away = 1 - under
    read = gaussian_filter(away, UNDER_STARS_REACH)
    around = (gaussian_filter(nebula * away[..., None], (UNDER_STARS_REACH, UNDER_STARS_REACH, 0))
              / np.maximum(gaussian_filter(level * away, UNDER_STARS_REACH), 1e-12)[..., None])
    weight = (under * np.clip(read / 0.05, 0, 1))[..., None]
    return np.clip(nebula + weight * (level[..., None] * around - nebula), 0, None).astype(np.float32)


def tiles_sky(level: np.ndarray, tile: int = 64) -> float:
    """The sky level of an image: the tenth percentile of the medians of its ``tile`` px tiles (empty tiles out)."""
    height, width = level.shape
    medians = np.asarray([np.median(level[y:y + tile, x:x + tile])
                          for y in range(0, max(height - tile, 1), tile) for x in range(0, max(width - tile, 1), tile)])
    medians = medians[medians > 0] if np.any(medians > 0) else medians
    return float(np.percentile(medians, 10))


# (f) The white reference.  It is read star by star: the light of each unclipped field star (an aperture of
# ``WHITE_STAR_APERTURE`` px, less the median of a ring around it: the sky or the nebula it stands on), then the median
# over the stars of each channel's share.  One star, one voice: a few very bright stars of one colour -- the blue
# Pleiades, whose wide halos hold most of the light of the star layer -- do not make the reference, the many field
# stars do.  The ``WHITE_STARS_MAX`` brightest are read; under ``WHITE_STARS_MIN`` stars the reference is the
# light of the whole star layer, as it was.
WHITE_STAR_APERTURE = 5
WHITE_STAR_RING = (8, 12)
WHITE_STARS_MIN = 12
WHITE_STARS_MAX = 2000


def field_star_colours(stars: np.ndarray, linear: np.ndarray) -> np.ndarray:
    """The share of each channel in the light of each unclipped field star, stars x 3 (rows sum to 1), brightest first.

    The stars are the peaks of the star layer (luminance smoothed at 1 px, the highest within 11 px, five noise
    widths of the image up); their light is read on ``linear``.  A star whose aperture holds a pixel near the highest
    level of the image (a clipped core: its colour is the clip levels') is left out, and so is one whose light is not
    above its surroundings in every channel.
    """
    level = gaussian_filter(lum(stars), 1.0)
    reach = WHITE_STAR_RING[1]
    peaks = (level == maximum_filter(level, size=11)) & (level > 5 * _background_noise(lum(linear)))
    peaks[:reach] = peaks[-reach:] = False
    peaks[:, :reach] = peaks[:, -reach:] = False
    ys, xs = np.nonzero(peaks)
    order = np.argsort(-level[ys, xs])
    top = np.max(linear, axis=-1)
    ceiling = 0.9 * float(median_filter(top, size=3).max())
    yy, xx = np.mgrid[-reach:reach + 1, -reach:reach + 1]
    distance = np.hypot(yy, xx)
    aperture = distance <= WHITE_STAR_APERTURE
    ring = (distance >= WHITE_STAR_RING[0]) & (distance <= WHITE_STAR_RING[1])
    shares = []
    for y, x in zip(ys[order], xs[order]):
        box = (slice(y - reach, y + reach + 1), slice(x - reach, x + reach + 1))
        if float(top[box][aperture].max()) >= ceiling:
            continue
        local = linear[box].astype(np.float64)
        light = (local[aperture] - np.median(local[ring], axis=0)).sum(axis=0)
        if np.all(light > 0):
            shares.append(light / light.sum())
            if len(shares) == WHITE_STARS_MAX:
                break
    return np.asarray(shares, np.float64).reshape(-1, 3)


def star_white_gains(stars: np.ndarray, linear: np.ndarray) -> np.ndarray | None:
    """Per-channel gains that make the median field star white; None when there are no stars to read.

    The colour of the median star is the median, channel by channel, of ``field_star_colours``.  With fewer than
    ``WHITE_STARS_MIN`` stars to read there is no median to trust: the light of the star layer is read as a whole
    instead (its brightest tenth, the clipped pixels left out).  The gains are bounded: a field of only red giants
    does not turn the sky blue.
    """
    level = lum(stars)
    lit = level > 0
    if lit.sum() < 100:
        return None
    shares = field_star_colours(stars, linear)
    if len(shares) >= WHITE_STARS_MIN:
        typical = np.median(shares, axis=0)
    else:
        top = np.max(linear, axis=-1)
        chosen = (level > np.percentile(level[lit], 90)) & (top < 0.9 * float(top.max()))
        if chosen.sum() < 50:
            return None
        typical = stars[chosen].astype(np.float64).sum(axis=0)
        if np.any(typical <= 0):
            return None
    return np.clip(typical.mean() / typical, 0.7, 1.4).astype(np.float32)


def clipped_cores(linear: np.ndarray, settings: PolishedSettings = POLISHED) -> np.ndarray:
    """0..1: how much each pixel belongs to a core the sensor clipped (``core_from``..``core_to`` of the clip level).

    The clip level of a channel is its highest value once single pixels are taken out (a 3 x 3 median): the
    plateau of a clipped core survives it, a hot pixel does not.  The weight is widened by one pixel and
    smoothed, so the edge of the plateau, where a channel has just left its clip level, is in it too.
    """
    weight = np.zeros(linear.shape[:2], np.float32)
    for channel in range(linear.shape[2]):
        plane = linear[..., channel]
        level = float(median_filter(plane, size=3).max())
        if level <= 0:
            continue
        share = (plane / level - settings.core_from) / max(settings.core_to - settings.core_from, 1e-6)
        weight = np.maximum(weight, np.clip(share, 0, 1).astype(np.float32))
    return np.clip(gaussian_filter(maximum_filter(weight, size=3), 1.0), 0, 1).astype(np.float32)


def neutral_cores(rgb: np.ndarray, weight: np.ndarray) -> np.ndarray:
    """Each pixel brought towards the grey of its own luminance by ``weight``."""
    level = lum(rgb)[..., None]
    return (rgb + weight[..., None] * (level - rgb)).astype(np.float32)


def near_stars(cores: np.ndarray, reach: int = 6) -> np.ndarray:
    """0..1: the clipped cores and ``reach`` px around them, smoothed.  The highlights fade to white there only: a
    clipped core is a star's, and its colour is the clip levels'; the bright core of a nebula (M42) is not one, and
    keeps its colour and its texture."""
    yy, xx = np.mgrid[-reach:reach + 1, -reach:reach + 1]
    disc = (yy * yy + xx * xx) <= reach * reach
    return np.clip(gaussian_filter(maximum_filter(cores, footprint=disc), reach / 3), 0, 1).astype(np.float32)


def into_gamut(rgb: np.ndarray, level: np.ndarray) -> np.ndarray:
    """A pixel whose channels pass 1 brought towards white at the same luminance ``level`` (never darkened).

    A luminance stretch keeps each pixel's colour ratios; a strongly coloured, bright pixel then has a channel
    above 1.  Dividing it by that channel would darken it -- the coloured halo of a bright star would turn into
    a dark ring around its white core --; it is desaturated instead, just enough to fit.
    """
    level = np.clip(np.asarray(level, dtype=np.float32), 0, 1)[..., None]
    top = np.max(rgb, axis=-1, keepdims=True)
    room = np.where(top > 1.0, (1.0 - level) / np.maximum(top - level, 1e-6), 1.0)
    out = level + np.clip(room, 0, 1) * (rgb - level)
    return np.clip(out, 0, 1).astype(np.float32)


def midtones_transfer(x: np.ndarray, midtones: float) -> np.ndarray:
    """The midtones transfer function: 0 and 1 stay, ``midtones`` goes to 0.5."""
    x = np.clip(x, 0, 1)
    return (midtones - 1) * x / ((2 * midtones - 1) * x - midtones)


def polished_curve(nebula: np.ndarray, whole: np.ndarray, settings: PolishedSettings = POLISHED) -> dict[str, float]:
    """(h) The curve of the render, read on the image: ``{"black", "top", "midtones"}``.

    ``black``: the sky of the nebula layer less ``black_sigmas`` noise widths; ``top``: the luminance (above
    ``black``) shown at the top, the 99.99th percentile of the whole image; ``midtones``: the transfer that puts
    the sky at ``settings.sky``.
    """
    level = lum(nebula)
    sky = tiles_sky(level)
    black = sky - settings.black_sigmas * _background_noise(level)
    top = max(float(np.percentile(lum(whole), 99.99)) - black, 1e-9)
    x0 = float(np.clip((sky - black) / top, 1e-6, 0.999))
    target = settings.sky
    midtones = x0 * (target - 1) / (2 * target * x0 - target - x0)
    return {"black": float(black), "top": float(top), "midtones": float(np.clip(midtones, 1e-6, 0.999))}


def curve_display(linear: np.ndarray, curve: dict[str, float], settings: PolishedSettings = POLISHED,
                  near_stars: np.ndarray | None = None) -> np.ndarray:
    """(h) A linear layer through the render's curve: luminance curved, colours kept, brought into gamut towards white.

    ``near_stars`` (0..1, default 1 everywhere): where the highlights fade towards white -- around the stars, never
    on the bright core of a nebula, which keeps its colour and its texture."""
    lifted = np.clip(linear - np.float32(curve["black"]), 0, None)
    level = lum(lifted)
    shown = soft_shoulder(midtones_transfer(level / curve["top"], curve["midtones"]), settings.shoulder)
    rgb = into_gamut(lifted * (shown / np.maximum(level, 1e-12))[..., None], shown)
    # The highlights fade towards white: the core of a bright star is white whatever the clip levels of the
    # channels left around it, and its halo keeps its colour where it is not near white.
    x = np.clip((shown - settings.white_from) / max(settings.white_to - settings.white_from, 1e-6), 0, 1)
    keep = 1 - x * x * (3 - 2 * x) * (1.0 if near_stars is None else near_stars)
    return np.clip(shown[..., None] + keep[..., None] * (rgb - shown[..., None]), 0, 1).astype(np.float32)


def with_level(rgb: np.ndarray, level: np.ndarray, wanted: np.ndarray) -> np.ndarray:
    """``rgb`` (whose luminance is ``level``) scaled to the luminance ``wanted``, brought into gamut."""
    scaled = rgb * (wanted / np.maximum(level, 1e-6))[..., None]
    return into_gamut(scaled, wanted)


def strong_signal(level: np.ndarray, sky: float, smoothing: float, low: float, high: float) -> np.ndarray:
    """0..1: where the luminance smoothed at ``smoothing`` px stands ``low`` (0) to ``high`` (1) noise widths of the
    smoothed sky above the sky, with a smooth step."""
    smooth = gaussian_filter(level, smoothing)
    noise = _background_noise(smooth) + 1e-6
    x = np.clip(((smooth - sky) / noise - low) / max(high - low, 1e-6), 0, 1)
    return (x * x * (3 - 2 * x)).astype(np.float32)


def lift_power(nebula: np.ndarray, sky: float, settings: PolishedSettings = POLISHED) -> float:
    """(i) The power of the lift: ``lift``, or lower (down to ``lift_strongest``) when the bright part of the
    signal would stay under ``lift_target`` -- a faint galaxy on a black sky is lifted, a bright nebula is not.

    A glow that covers the frame (a vignetting the background step left, its upper half over ``lift_area`` of
    the frame) is not an object: it is not lifted.
    """
    smooth = gaussian_filter(lum(nebula), 4.0)
    bright = float(np.percentile(smooth, 99.5))
    above = (bright - sky) / (1 - sky)
    wanted = (settings.lift_target - sky) / (1 - sky)
    if above <= 1e-4 or above >= wanted:
        return settings.lift
    if float(np.mean(smooth > sky + 0.5 * (bright - sky))) > settings.lift_area:
        return settings.lift
    return float(np.clip(np.log(wanted) / np.log(above), settings.lift_strongest, settings.lift))


def lift_midtones(nebula: np.ndarray, sky: float, power: float) -> np.ndarray:
    """(i) A power on the signal above the sky; the sky (at or under ``sky``) and the white do not move."""
    level = lum(nebula)
    above = np.clip((level - sky) / (1 - sky), 0, 1)
    wanted = np.where(level > sky, sky + (1 - sky) * np.power(above, power), level)
    return with_level(nebula, level, wanted)


def smooth_nebula(nebula: np.ndarray, sky: float, settings: PolishedSettings = POLISHED) -> np.ndarray:
    """(j) Non-local means on the nebula, ``smoothing`` noise widths of the sky; texture given back where the signal is strong.

    The strength is read on the sky's grain (the darkest tiles), not on the nebula.  Where the signal stands far
    out of the noise (30 to 120 noise widths, smoothed) ``smoothing_kept`` of the texture comes back: a bright
    nebula keeps its fine structure instead of turning waxy.
    """
    if settings.smoothing <= 0:
        return nebula
    from skimage.restoration import denoise_nl_means

    level = lum(nebula)
    detail = level - gaussian_filter(level, 2.0)
    noise = float(_background_noise(detail))
    if noise <= 0:
        return nebula
    smoothed = denoise_nl_means(nebula, h=settings.smoothing * noise, sigma=noise, fast_mode=True,
                                patch_size=5, patch_distance=6, channel_axis=-1).astype(np.float32)
    kept = settings.smoothing_kept * strong_signal(level, sky, 3.0, 30.0, 120.0)
    return np.clip(smoothed + kept[..., None] * (nebula - smoothed), 0, 1).astype(np.float32)


def compress_highlights(nebula: np.ndarray, knee: float, ceiling: float, spared: np.ndarray | None = None,
                        scale: float = 12.0) -> np.ndarray:
    """(j) The bright core of a nebula brought down without flattening it: its large-scale level (smoothed at
    ``scale`` px) bends from ``knee`` towards ``ceiling`` (same slope at the knee), its fine structure is kept on
    top of it.  Below the knee nothing moves.  ``spared`` (0..1): the clipped stars StarNet2 left in the nebula,
    which keep their level and so stand out, white, above the lowered core around them."""
    level = lum(nebula)
    base = gaussian_filter(level, scale)
    if float(base.max()) <= knee:
        return nebula
    room = max(ceiling - knee, 1e-3)
    bent = np.where(base > knee, knee + room * (1 - np.exp(-(base - knee) / room)), base)
    lowered = np.minimum(bent - base, 0)
    if spared is not None:
        lowered = lowered * (1 - spared)
    wanted = np.clip(level + lowered, 0, 1)
    return with_level(nebula, level, wanted)


def sky_back_to(nebula: np.ndarray, sky: float) -> np.ndarray:
    """The sky (``tiles_sky``) brought back to ``sky`` by a straight scale under it and above it, joined there: no
    level is clipped, so no flat black."""
    level = lum(nebula)
    now = tiles_sky(level)
    if now <= sky or now >= 1:
        return nebula
    wanted = np.where(level <= now, level * (sky / now), sky + (level - now) * (1 - sky) / (1 - now))
    return with_level(nebula, level, wanted)


def local_contrast(nebula: np.ndarray, sky: float, amount: float) -> np.ndarray:
    """(j) The nebula against its surroundings at 24 px, kept out of the sky and of the brightest parts."""
    if amount <= 0:
        return nebula
    level = lum(nebula)
    local = level - gaussian_filter(level, 24)
    signal = np.clip((level - sky) / max(float(np.percentile(level, 75)) - sky, 1e-6), 0, 1)
    wanted = np.clip(level + amount * local * signal * np.clip(1 - level / 0.85, 0, 1), 0, 1)
    return with_level(nebula, level, wanted)


def chroma(rgb: np.ndarray, factor: float, weight: np.ndarray | None = None) -> np.ndarray:
    """The colours multiplied by ``factor`` (by ``weight``, 0..1, per pixel), the luminance kept, in gamut."""
    level = lum(rgb)
    gain = factor if weight is None else 1 + (factor - 1) * weight[..., None]
    out = np.clip(level[..., None] + (rgb - level[..., None]) * gain, 0, None)
    return into_gamut(out, level)


def signal_weight(rgb: np.ndarray) -> np.ndarray:
    """0 in the sky, 1 six noise widths above it (luminance smoothed at 2 px): where (k) strengthens the colours."""
    smooth = gaussian_filter(lum(rgb), 2)
    sky = float(np.percentile(smooth, 10))
    noise = float(_background_noise(lum(rgb))) + 1e-4
    return np.clip((smooth - sky) / (6 * noise), 0, 1).astype(np.float32)


def polished_layers(
    linear: np.ndarray,
    display: np.ndarray,
    starless: np.ndarray,
    scale: float,
    band: str = "unknown",
    settings: PolishedSettings = POLISHED,
    imposed: dict[str, object] | None = None,
    measured: dict[str, object] | None = None,
    done: dict[str, object] | None = None,
) -> tuple[np.ndarray, np.ndarray]:
    """Steps (f) to (k): the nebula and star layers of the polished render (display, 0..1), from the linear image.

    ``display`` and ``starless``: StarNet2's input and its starless output (the same image when the stars were
    not separated); ``scale``: the stretch scale of ``display``.  ``band``: one of ``CAPTURE_BANDS``.
    ``imposed`` (``{"curve": {...}}``, what a render of the same plan recorded) gives the curve and the star
    gains instead of reading them on the image; ``measured`` receives the ones used; ``done`` what was done
    (``band``, ``green``, ``starGains``).
    """
    if band not in CAPTURE_BANDS:
        raise ValueError(f"unknown capture band '{band}': {', '.join(CAPTURE_BANDS)}")
    done = {} if done is None else done
    imposed_curve = (imposed or {}).get("curve") or {}
    nebula, stars = linear_split(linear, display, starless, scale)
    # (f) the colour
    gains = None
    if band != "dual-band":
        offsets = np.array([tiles_sky(nebula[..., c]) for c in range(3)], np.float32)
        mean = float(offsets.mean())
        nebula = np.clip(nebula - offsets + mean, 0, None)
        gains = (np.asarray(imposed_curve["gains"], np.float32) if imposed_curve.get("gains") is not None
                 else star_white_gains(stars, linear))
        if gains is not None:
            # The sky stays neutral: its grain and what the background step left of a gradient are not the stars'
            # light, and gains far from 1 would tint them.  The gains come in with the signal (``signal_weight``).
            signal = signal_weight(nebula)[..., None]
            nebula = np.clip((nebula - mean) * (1 + signal * (gains - 1)) + mean, 0, None)
            stars = stars * gains
    green = settings.green if band == "broadband" else None
    if green:
        nebula = scnr_green(nebula, green)
    done.update({"band": band, "green": green, "starGains": None if gains is None else [round(float(g), 4) for g in gains]})
    # (g) the halo of the clipped stars, from the star's colour to the nebula's, and their cores neutral
    halos = clipped_star_halos(nebula + stars, linear, clip_levels(linear))
    if halos is not None:
        whole = nebula + stars
        nebula, stars = with_halo_colours(nebula, whole, halos[0]), with_halo_colours(stars, whole, halos[0])
    cores = clipped_cores(linear, settings)
    if float(cores.max()) > 0:
        nebula, stars = neutral_cores(nebula, cores), neutral_cores(stars, cores)
    # Under the small stars the nebula layer takes the colour of what surrounds them; the star layer holds the rest.
    whole = nebula + stars
    nebula = nebula_around_stars(nebula, display, starless)
    stars = np.clip(whole - nebula, 0, None).astype(np.float32)
    # (h) the curve
    curve = ({key: float(imposed_curve[key]) for key in ("black", "top", "midtones")} if imposed_curve.get("black") is not None
             else polished_curve(nebula, nebula + stars, settings))
    near = near_stars(cores)
    nebula_shown = curve_display(nebula, curve, settings, near)
    stars_shown = np.clip(curve_display(nebula + stars, curve, settings, near) - nebula_shown, 0, 1)
    # (i) - (k) the nebula
    power = (float(imposed_curve["lift"]) if imposed_curve.get("lift") is not None
             else lift_power(nebula_shown, settings.sky, settings))
    curve["lift"] = power
    done["curve"] = {key: round(value, 6) for key, value in curve.items()}
    if measured is not None:
        measured["curve"] = {**curve, "gains": None if gains is None else [float(g) for g in gains]}
    nebula_shown = lift_midtones(nebula_shown, settings.sky, power)
    nebula_shown = smooth_nebula(nebula_shown, settings.sky, settings)
    # The lift bends the grain above the sky more than the grain under it: once smoothed, the sky stands a little
    # higher; it is brought back to its level.
    nebula_shown = sky_back_to(nebula_shown, settings.sky)
    nebula_shown = local_contrast(nebula_shown, settings.sky, settings.local_contrast)
    nebula_shown = compress_highlights(nebula_shown, settings.nebula_knee, settings.nebula_ceiling, cores)
    if settings.saturation != 1.0:
        nebula_shown = chroma(nebula_shown, settings.saturation, signal_weight(nebula_shown))
    # The clipped cores white: the shoulder shows them at 0.8 to 0.9, a flat grey disc.  The nebula layer is brought
    # towards white over them, softened over 2 px around, so the render is white there whatever the star layer holds,
    # and falls from it smoothly.
    white = np.maximum(cores, gaussian_filter(cores, 2.0))[..., None]
    nebula_shown = (nebula_shown + white * (1 - nebula_shown)).astype(np.float32)
    if halos is not None:
        nebula_shown, stars_shown = halos_within_contrast(nebula_shown, stars_shown, halos[1], settings)
    return nebula_shown, stars_shown.astype(np.float32)


def halos_within_contrast(nebula: np.ndarray, stars: np.ndarray, weight: np.ndarray,
                          settings: PolishedSettings = POLISHED) -> tuple[np.ndarray, np.ndarray]:
    """The halos of the clipped stars (``weight``, ``clipped_star_halos``) brought towards white, just enough that the
    strong contrast (``CONTRAST``) lifts them without a channel passing 1.

    ``presentation_contrast`` scales such a pixel back under 1, which darkens it: a bright blue halo came out darker
    than the paler nebula beyond it and the white core inside it -- a disc with a sharp edge, then a dark ring.  A
    halo that fits the curve keeps the curve's luminance, which falls from the core to the nebula in one piece.
    Nothing moves outside the halos."""
    image = polished_deep(nebula, stars, settings)
    level = lum(image)
    lifted = s_curve(level, float(np.percentile(level, PIVOT_PERCENTILE)), CONTRAST)
    ratio = np.max(image, axis=-1) / np.maximum(level, 1e-6)
    room = np.clip((1 / np.maximum(lifted, 1e-6) - 1) / np.maximum(ratio - 1, 1e-6), 0, 1)
    lost = (weight * (1 - room)).astype(np.float32)
    return chroma(nebula, 0.0, lost), chroma(stars, 0.0, lost)


def polished_deep(nebula: np.ndarray, stars: np.ndarray, settings: PolishedSettings = POLISHED) -> np.ndarray:
    """(l) Polished render: the stars put back on the nebula (screen), their colour reduced."""
    stars = chroma(np.clip(stars, 0, 1), settings.star_colour)
    return np.clip(1 - (1 - np.clip(nebula, 0, 1)) * (1 - settings.stars * stars), 0, 1).astype(np.float32)


# --------------------------------------------------------------------------
# "HOO palette" (pipeline ``hoo``), for a dual-band capture made
# with a colour sensor: Hα (red pixels) goes to red, OIII (green and blue
# pixels) to blue-green -- an assumed palette, not the continuous correction
# of ``dualband_deep``.  The brightness is the natural one (same preparation,
# same tone curve): only the hue and the saturation come from the palette.
# The stars keep their natural colours.

HOO_MAX_SATURATION = 0.78
HOO_SATURATION = 1.25  # the palette's saturation is lifted by this factor (bounded above)
# OIII is fainter than Hα: it is brought towards Hα's scale by a factor read on
# the image (their brightest signal), damped and bounded, so the palette stays
# red where Hα dominates and turns blue-green only where OIII is really there.
HOO_OIII_DAMPING = 0.6
HOO_OIII_BOOST = (1.0, 2.0)
# The colour is read on the image smoothed at this scale (px): the palette is a
# colour of regions, and a single noisy pixel must not pick a hue.
HOO_SMOOTHING = 3.0
# The palette takes over from the natural colours between these signal-to-noise
# ratios of the smoothed image, with a smooth step (no threshold): the sky keeps
# the natural neutral sky, the faint red zones fade into the palette.
HOO_SNR_FROM = 1.0
HOO_SNR_TO = 12.0


def _soft_positive(x: np.ndarray, width: np.ndarray | float) -> np.ndarray:
    """The positive part of ``x`` with its corner rounded over ``width`` (smooth where ``max(x, 0)`` has a kink)."""
    return 0.5 * (x + np.sqrt(x * x + width * width))


def _hoo_bands(nebula: np.ndarray) -> tuple[np.ndarray, np.ndarray]:
    """Hα and OIII of a nebula layer, smoothed at ``HOO_SMOOTHING``."""
    smooth = gaussian_filter(np.asarray(nebula, dtype=np.float32), sigma=(HOO_SMOOTHING, HOO_SMOOTHING, 0))
    return smooth[..., 0], 0.5 * (smooth[..., 1] + smooth[..., 2])


def hoo_levels(nebula: np.ndarray) -> dict[str, float]:
    """The figures the HOO palette reads on the image: the sky (median) and the noise of Hα and OIII, and the boost
    of OIII."""
    ha, oiii = _hoo_bands(nebula)
    levels = {"ha": float(np.median(ha)), "oiii": float(np.median(oiii)),
              "ha_noise": _background_noise(ha) + 1e-6, "oiii_noise": _background_noise(oiii) + 1e-6}
    ha_signal = _soft_positive(ha - levels["ha"], levels["ha_noise"])
    oiii_signal = _soft_positive(oiii - levels["oiii"], levels["oiii_noise"])
    ratio = np.percentile(ha_signal, 99.5) / max(np.percentile(oiii_signal, 99.5), 1e-6)
    levels["boost"] = float(np.clip(HOO_OIII_DAMPING * ratio, *HOO_OIII_BOOST))
    return levels


def hoo_weight(nebula: np.ndarray, levels: dict[str, float] | None = None) -> np.ndarray:
    """0 in the sky, 1 where Hα or OIII stand well above the noise; smooth in between (no threshold).

    The signal-to-noise ratio is read on the image smoothed at ``HOO_SMOOTHING``
    and taken through a smooth step, so the edge of a faint red zone is a
    gradient, not a line.  ``levels``: ``hoo_levels``, read on ``nebula`` when not given.
    """
    levels = hoo_levels(nebula) if levels is None else levels
    ha, oiii = _hoo_bands(nebula)
    snr = np.maximum((ha - levels["ha"]) / levels["ha_noise"], (oiii - levels["oiii"]) / levels["oiii_noise"])
    x = np.clip((snr - HOO_SNR_FROM) / (HOO_SNR_TO - HOO_SNR_FROM), 0, 1)
    return gaussian_filter(x * x * (3 - 2 * x), sigma=HOO_SMOOTHING).astype(np.float32)


def hoo_palette(nebula: np.ndarray, reference: np.ndarray | None = None) -> np.ndarray:
    """The HOO palette on the brightness of the natural tone, fading into the natural colours.

    Where Hα or OIII stand out of the noise the colour is the palette's (Hα to
    red, OIII to blue-green); in the sky it is the natural tone's own neutral
    sky, and the faint zones in between are a smooth mix of both.  The sky, the noise and the boost of OIII are
    read on ``reference`` when given (``hoo_deep``: the layer before its colour under the stars was cleaned, so
    that the cleaning moves nothing away from the stars).
    """
    nebula = np.clip(np.asarray(nebula, dtype=np.float32), 0, 1)
    levels = hoo_levels(nebula if reference is None else np.clip(np.asarray(reference, dtype=np.float32), 0, 1))
    toned = deep_nebula_tone(nebula)
    # The nebula layer keeps the linear ratios of the channels (the stretch
    # scales each pixel), so its red is Hα and the mean of green and blue OIII.
    ha, oiii = _hoo_bands(nebula)
    # Signals above the sky, the corner of the positive part rounded at the noise.
    ha_signal = _soft_positive(ha - levels["ha"], levels["ha_noise"])
    oiii_signal = _soft_positive(oiii - levels["oiii"], levels["oiii_noise"]) * levels["boost"]
    palette = np.stack([ha_signal, oiii_signal, oiii_signal], axis=-1)
    hsv = rgb2hsv(palette / max(float(palette.max()), 1e-6))
    value = np.max(toned, axis=-1)
    saturation = np.clip(hsv[..., 1] * HOO_SATURATION, 0, HOO_MAX_SATURATION)
    coloured = hsv2rgb(np.stack([hsv[..., 0], saturation, value], axis=-1))
    weight = hoo_weight(nebula, levels)[..., None]
    mixed = toned * (1 - weight) + coloured * weight
    # The brightness is the natural one: the brightest channel is the natural tone's.
    return np.clip(mixed * (value / np.maximum(mixed.max(axis=-1), 1e-6))[..., None], 0, 1)


def hoo_deep(nebula: np.ndarray, stars: np.ndarray) -> np.ndarray:
    """HOO palette: the palette on the nebula, the stars in their natural colours."""
    nebula = hoo_palette(starless_around_stars(nebula, stars), nebula)
    return np.clip(1 - (1 - nebula) * (1 - controlled_stars(stars, 0.52)), 0, 1)


# Contrast of presentation.  The curve is a tanh S-curve in a warped domain
# around the sky level, so the slope stays continuous at the pivot (two joined
# power laws made it jump there and turned star halos into hard edges).
PIVOT_PERCENTILE = 60.0  # the pivot is this percentile of the image's luminance
CONTRAST = 3.6  # the gain of the strong contrast (the presentation gain is 2.0)
# The two gains the option offers: 2.0 for presentation, 3.6 for strong.
CONTRAST_GAINS = {"presentation": 2.0, "strong": CONTRAST}
CONTRAST_CHOICES = ("none", *CONTRAST_GAINS)


def s_curve(x: np.ndarray, pivot: float, gain: float = CONTRAST) -> np.ndarray:
    """The S-curve on values in 0..1 around ``pivot``: 0, ``pivot`` and 1 stay in place."""
    x = np.clip(x, 0, 1)
    u = np.where(x < pivot, 0.5 * x / pivot, 0.5 + 0.5 * (x - pivot) / (1 - pivot))
    s = 0.5 + 0.5 * np.tanh(gain * (u - 0.5)) / np.tanh(gain / 2)
    return np.clip(np.where(s < 0.5, 2 * s * pivot, pivot + (2 * s - 1) * (1 - pivot)), 0, 1)


def presentation_contrast(rgb: np.ndarray, gain: float = CONTRAST_GAINS["presentation"]) -> np.ndarray:
    """The S-curve on the luminance of a display image (float, 0..1), colours kept.

    The pivot is the ``PIVOT_PERCENTILE`` of the luminance (the sky level of a
    render): a pixel at the pivot does not move, brighter ones are lifted and
    darker ones lowered.  Each pixel keeps its RGB ratios (it is scaled, never
    recoloured); where scaling would push a channel past 1 the pixel is scaled
    back, hue first.  A single-channel image is bent as it is.
    """
    x = np.clip(np.nan_to_num(np.asarray(rgb, np.float32)), 0, 1)
    if x.ndim == 2:
        return s_curve(x, float(np.percentile(x, PIVOT_PERCENTILE)), gain).astype(np.float32)
    level = lum(x)
    curved = s_curve(level, float(np.percentile(level, PIVOT_PERCENTILE)), gain)
    out = x * (curved / np.maximum(level, 1e-8))[..., None]
    return (out / np.maximum(out.max(axis=-1, keepdims=True), 1.0)).astype(np.float32)


# The render functions a pipeline definition may name in its ``entry``
# (``renders:<key>``).  ``pipeline_defs.py`` reads the keys of this literal
# without importing the module, so keep it a plain dict of name -> function.
RENDERS = {
    "natural_deep": natural_deep,
    "dualband_deep": dualband_deep,
    "polished_deep": polished_deep,
    "hoo_deep": hoo_deep,
}

# The linear preparation a render needs, when it is not the natural one
# (``prepare_native``): the stacking script prepares each kind once and gives
# its layers to every render of that kind.  A pipeline's own script gets the
# natural one.
PREPARATIONS = {
    "polished_deep": "polished",
}


def preparation_of(render_function: str | None) -> str:
    """``native`` or ``polished``: the preparation the render needs."""
    return PREPARATIONS.get(render_function or "", "native")


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("source", type=Path)
    parser.add_argument("output", type=Path)
    parser.add_argument("--stem", required=True)
    parser.add_argument("--dualband", action="store_true", help="Also render the documented dual-band V7 render")
    parser.add_argument("--dualband-deep", action="store_true", help="Also render the dual-band deep render")
    parser.add_argument("--deep", action="store_true", help="Also render a deeper natural variant")
    parser.add_argument(
        "--stretch-reference",
        type=Path,
        metavar="FITS",
        help="Use the display stretch measured from this calibrated linear reference. "
             "Use the final cumulative master for a progression so every step shares one scale.",
    )
    parser.add_argument("--gradient-degree", type=int, choices=(1, 2), default=2)
    parser.add_argument("--black-percentile", type=float, default=0.25)
    parser.add_argument(
        "--deconv-fwhm",
        type=float,
        default=None,
        metavar="PX",
        help="Masked Richardson-Lucy on the linear master until stars reach this FWHM "
             "in pixels. Off by default. Do not go below 2.2 on an undersampled field.",
    )
    parser.add_argument(
        "--starnet",
        type=Path,
        default=None,
        metavar="EXECUTABLE",
        help="StarNet2 command-line executable. Without it, stars are not separated.",
    )
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    display_scale = None
    if args.stretch_reference:
        reference = calibrated_linear(
            args.stretch_reference, args.gradient_degree, args.black_percentile, args.deconv_fwhm,
        )
        display_scale = stretch_scale(reference)
        print(f"shared display scale: {display_scale:.8g} from {args.stretch_reference}")
    native = prepare_native(
        args.source, args.gradient_degree, args.black_percentile, args.deconv_fwhm, display_scale,
    )
    nebula, stars = star_layer(native, args.output, args.stem, args.starnet)
    save(natural_v23(nebula, stars), args.output, f"{args.stem}_natural_v23")
    if args.deep:
        save(natural_deep(nebula, stars), args.output, f"{args.stem}_natural_deep")
    if args.dualband:
        save(dualband_v7(nebula, stars), args.output, f"{args.stem}_dualband_v7")
    if args.dualband_deep:
        save(dualband_deep(nebula, stars), args.output, f"{args.stem}_dualband_deep")


if __name__ == "__main__":
    main()
