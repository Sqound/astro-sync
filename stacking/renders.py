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
  remapped; a soft shadow toe, a gentle lift of the faint signal, restrained
  stars.  The pipeline's current natural reference.
- ``dualband_deep`` (pipeline ``dualband-correction``): exactly the same
  luminance treatment, then one global, continuous colour transform (green
  towards cyan, red towards red-violet, a slight blue-violet balance) that
  brings the colours of a dual-band capture back towards natural tones.
- ``polished_deep`` (pipeline ``polished``): the natural render
  pushed further without changing its colours, from its own linear
  preparation (``prepare_polished``): GraXpert's background and light
  denoising, a moderate SCNR, a soft shoulder on the highlights, a moderate
  local contrast on the nebula layer.  Without GraXpert: the polynomial
  background and no denoising.
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
from collections.abc import Iterator
from pathlib import Path
from typing import IO

import numpy as np
import tifffile
from astropy.io import fits
from PIL import Image
from scipy.ndimage import distance_transform_edt, gaussian_filter, maximum_filter
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


def remove_gradient(
    data: np.ndarray,
    tile: int = 160,
    degree: int = 2,
    black_percentile: float = 0.25,
) -> np.ndarray:
    """Subtract a robust low-order background model without flattening stars."""
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
        corrected[..., channel] = data[..., channel] - model
    black = np.percentile(corrected, black_percentile, axis=(0, 1))
    return np.clip(corrected - black, 0, None)


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
) -> np.ndarray:
    """Prepare a linear image before the common display stretch.

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


def prepare_native(
    source: Path,
    gradient_degree: int = 2,
    black_percentile: float = 0.25,
    deconv_fwhm: float | None = None,
    display_scale: float | None = None,
) -> np.ndarray:
    linear = calibrated_linear(source, gradient_degree, black_percentile, deconv_fwhm)
    return native_from_linear(linear, display_scale)


def native_from_linear(linear: np.ndarray, display_scale: float | None = None) -> np.ndarray:
    """The natural display stretch of a prepared linear image (what ``prepare_native`` ends with)."""
    level = lum(linear)
    scale = display_scale if display_scale is not None else stretch_scale(linear)
    stretched = np.clip(np.arcsinh(18.0 * level / scale) / np.arcsinh(18.0), 0, 1)
    rgb = linear * (stretched / np.maximum(level, 1e-8))[..., None]
    maximum = np.max(rgb, axis=2)
    rolled = np.where(maximum > 0.72, 0.72 + (maximum - 0.72) / (1 + 1.9 * (maximum - 0.72)), maximum)
    rgb *= rolled[..., None] / np.maximum(maximum[..., None], 1e-8)
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
    nebula = deep_nebula_tone(nebula)
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
    nebula = dualband_palette(deep_nebula_tone(nebula))
    return np.clip(1 - (1 - nebula) * (1 - controlled_stars(stars, 0.58)), 0, 1)


# --------------------------------------------------------------------------
# "Polished render" (pipeline ``polished``): the natural render pushed
# further without changing its colours.  It needs its own linear preparation
# (``prepare_polished``), named for its render in ``PREPARATIONS``:
#
# (a) background by GraXpert's background extraction (AI model, pinned) instead
#     of the polynomial; (b) GraXpert's denoising, a light step, on the linear
#     image; (c) the natural pipeline's neutral sky, then a moderate SCNR
#     (green above the mean of red and blue is taken back); (d) the masked
#     deconvolution of the natural pipeline (measured PSF), unchanged; (e) the
#     natural arcsinh stretch with a soft shoulder instead of the clip (bright
#     cores never burn) then, in ``polished_deep``, the natural toe with a
#     moderate local contrast on the nebula layer only; (f) the stars put back,
#     held a little more than in the natural render.
#
# Without GraXpert the pipeline still runs: polynomial background, no
# denoising, and it says so (``prepare_polished`` returns what it did).

# The GraXpert models are pinned: the same model gives the same image, and a
# pinned model already downloaded needs no network (« latest » asks the
# server for the list each time).
GRAXPERT_BACKGROUND_MODEL = "1.0.1"
GRAXPERT_DENOISE_MODEL = "3.0.2"
GRAXPERT_SMOOTHING = 0.5  # 0..1: a moderate smoothing of the background model
GRAXPERT_DENOISE_STRENGTH = 0.3  # 0..1 (GraXpert's default is 0.5): the light step
SCNR_AMOUNT = 0.5  # 0..1: half of the green excess is taken back
SHOULDER_KNEE = 0.70  # display level above which the highlights are compressed
POLISHED_LOCAL_CONTRAST = 0.22  # natural: 0.08
POLISHED_STARS = 0.48  # natural: 0.52
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
SIRIL_DENOISE_MODULATION = 0.5  # 0..1 (Siril's default is 1): the light step


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
    subprocess.run(
        [str(graxpert), "-cli", "-cmd", command, *options, "-output", str(output), str(source)],
        check=True, stdout=log, stderr=subprocess.STDOUT if log is not None else None,
    )
    written = output.with_name(output.name + ".fits")
    if not written.is_file():
        raise RuntimeError(f"GraXpert wrote no {written.name}")
    return written


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


def scnr_green(rgb: np.ndarray, amount: float = SCNR_AMOUNT) -> np.ndarray:
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


def sky_display_level(linear: np.ndarray) -> float:
    """The sky level of a linear image as the display stretch shows it: the median of the stretched luminance.

    The sky is most of a frame, so this median is the level of the sky on the
    screen; the stretch is the natural arcsinh one (``stretch_scale``).
    """
    level = lum(linear)
    scale = stretch_scale(linear)
    return float(np.median(np.arcsinh(18.0 * level / scale) / np.arcsinh(18.0)))


def match_sky_level(linear: np.ndarray, reference: float) -> tuple[np.ndarray, float]:
    """The image with its displayed sky level brought back to ``reference`` (``sky_display_level``); ``(image, shift)``.

    One offset, in linear light, for the three channels, so the neutral sky
    stays neutral.  The background steps (GraXpert's or Siril's subtraction,
    the denoising, the SCNR) each take a little of the sky level with them;
    the natural render keeps it, and the polished one is to look as deep as the
    natural one, not darker.  The stretch is increasing, so the median of the
    displayed luminance is the stretch of the median: the offset is exact.
    """
    scale = stretch_scale(linear)
    wanted = scale * float(np.sinh(reference * np.arcsinh(18.0))) / 18.0
    shift = wanted - float(np.median(lum(linear)))
    return np.clip(linear + np.float32(shift), 0, None), shift


def natural_sky_levels(raw: np.ndarray, gradient_degree: int, black_percentile: float) -> tuple[float, float]:
    """``(displayed, final)``: the sky level the natural render keeps, before and after its tone.

    The median luminance of the natural display image (polynomial background,
    neutral sky, natural stretch) and of the same image once the natural toe
    (``deep_nebula_tone``) has been applied: the second is what a person sees.
    """
    linear = neutral_sky(remove_gradient(raw, degree=gradient_degree, black_percentile=black_percentile))
    display = native_from_linear(linear)
    return sky_display_level(linear), float(np.median(lum(deep_nebula_tone(display))))


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
    dualband: bool = False,
) -> tuple[np.ndarray, dict[str, object]]:
    """Linear steps (a)-(d) and the stretch of (e); ``(display image, what was done)``.

    ``engine`` picks the background and the denoising (``POLISHED_ENGINES``).
    ``graxpert-ai`` falls back on Siril when GraXpert is not given or fails,
    and any engine falls back on the polynomial (no denoising) when nothing
    else can run.  ``dualband``: the capture went through a dual-band filter,
    whose green is OIII, signal: the SCNR is skipped.

    ``what was done``: ``{"engine": "graxpert-ai" | "siril" | "polynomial",
    "background": "graxpert" | "siril" | "polynomial", "denoise": "graxpert" |
    "siril" | None, "scnr": amount | None, "deconvolutionFwhm": target in px |
    None, ...}`` -- the provenance says it, as it says whether StarNet2
    separated the stars.  ``engineRequested`` and ``fallbackReason`` appear
    when the engine used is not the one asked for.
    """
    if engine not in POLISHED_ENGINES:
        raise ValueError(f"unknown polished engine '{engine}': {', '.join(POLISHED_ENGINES)}")
    if (graxpert is not None or siril is not None) and work is None:
        raise ValueError("GraXpert and Siril need a work folder: nothing is written beside the master")
    raw = read_linear_rgb(source)
    master_fwhm = star_fwhm(lum(raw)) if deconv_fwhm is None else None
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
        black = np.percentile(cleaned_raw, black_percentile, axis=(0, 1))
        linear = np.clip(cleaned_raw - black, 0, None)
        # The sky level the natural render would have kept (its polynomial, its neutral sky).
        reference = natural_sky_levels(raw, gradient_degree, black_percentile)
        tool = "graxpert" if used == "graxpert-ai" else "siril"
        done.update({"background": tool, "denoise": tool})
    else:
        linear = remove_gradient(raw, degree=gradient_degree, black_percentile=black_percentile)
        reference = None
        done.update({"background": "polynomial", "denoise": None})
    done["engine"] = used
    if master_fwhm is not None and np.isfinite(master_fwhm):
        done["masterFwhm"] = round(float(master_fwhm), 2)
    if used != engine:
        done["engineRequested"] = engine
        done["fallbackReason"] = "; ".join(reasons)
    return polished_display(linear, deconv_fwhm, done, None if dualband else SCNR_AMOUNT, reference, master_fwhm), done


def polished_display(
    linear: np.ndarray,
    deconv_fwhm: float | None = None,
    done: dict[str, object] | None = None,
    scnr: float | None = SCNR_AMOUNT,
    sky_reference: tuple[float, float] | None = None,
    master_fwhm: float | None = None,
) -> np.ndarray:
    """Steps (c), (d) and the stretch of (e) on a linear image whose background is already removed.

    ``scnr``: the amount of the SCNR, ``None`` to skip it (a dual-band
    capture: the green of the OIII is signal).  ``sky_reference``: the
    ``(displayed, final)`` sky level of the natural render (``natural_sky_levels``)
    that the image is to keep: its sky is brought to the first before the
    stretch, then nudged until the second is met after the tone
    (``polished_nebula_tone``; its toe follows the noise of the darkest pixels,
    which the denoising has lowered).  ``master_fwhm``: the stars' FWHM on the master,
    before any denoising, that the deconvolution target is read from.  ``done`` (when given) receives the SCNR,
    the deconvolution target and the sky shift.
    """
    done = {} if done is None else done
    linear = neutral_sky(linear)
    if scnr is not None:
        linear = scnr_green(linear, scnr)
    done["scnr"] = scnr
    target = deconv_fwhm if deconv_fwhm is not None else polished_deconv_target(linear, master_fwhm)
    if target is not None:
        try:
            linear = deconvolve_linear(linear, target)
        except RuntimeError as error:  # too few isolated stars for a PSF: the image goes on undeconvolved
            print(f"deconvolution skipped: {error}")
    done["deconvolutionFwhm"] = None if target is None else round(float(target), 2)
    if sky_reference is None:
        return polished_stretch(linear)
    displayed, final = sky_reference
    scale = stretch_scale(linear)  # the shift is small against the scale: one scale for the whole search
    image, shift = polished_stretch(linear), 0.0
    for _ in range(SKY_MATCH_ROUNDS):
        matched, shift = match_sky_level(linear, displayed)
        image = polished_stretch(matched, scale)
        reached = float(np.median(lum(polished_nebula_tone(image))))
        done["skyLevelShift"] = round(shift, 6)
        if abs(reached / final - 1.0) < SKY_MATCH_TOLERANCE:
            break
        displayed *= final / max(reached, 1e-6)
    return image


SKY_MATCH_ROUNDS = 4
SKY_MATCH_TOLERANCE = 0.004  # the polished sky is within 0.4 % of the natural one before the render's own gap


def polished_stretch(linear: np.ndarray, scale: float | None = None) -> np.ndarray:
    """The natural arcsinh stretch with a soft shoulder before the natural roll; the display image of (e)."""
    level = lum(linear)
    scale = stretch_scale(linear) if scale is None else scale
    stretched = soft_shoulder(np.arcsinh(18.0 * level / scale) / np.arcsinh(18.0))
    rgb = linear * (stretched / np.maximum(level, 1e-8))[..., None]
    maximum = np.max(rgb, axis=2)
    over = np.maximum(maximum - 0.72, 0)  # the natural roll, without dividing by zero below it
    rolled = np.where(maximum > 0.72, 0.72 + over / (1 + 1.9 * over), maximum)
    rgb *= rolled[..., None] / np.maximum(maximum[..., None], 1e-8)
    hsv = rgb2hsv(np.clip(rgb, 0, 1))
    hsv[..., 1] = np.clip(hsv[..., 1] * (1.0 + 0.10 * (1 - hsv[..., 1])), 0, 0.88)
    return hsv2rgb(hsv)


def polished_nebula_tone(nebula: np.ndarray) -> np.ndarray:
    """The natural toe, a soft shoulder instead of the clip, and a moderate local contrast.

    The local contrast (the nebula against its surroundings at 24 px) is kept
    out of the sky -- its noise is not lifted -- and out of the brightest
    parts, which the shoulder has just compressed.
    """
    nebula = np.clip(np.asarray(nebula, dtype=np.float32), 0, 1)
    level = lum(nebula)
    shadow = 0.35 * np.percentile(level, 0.1)
    x = np.clip((level - shadow) / (1 - shadow + 1e-12), 0, 1)
    target = soft_shoulder(1.08 * np.power(x, 0.71), 0.78)
    local = target - gaussian_filter(target, sigma=24)
    sky = np.percentile(target, 25)
    signal = np.clip((target - sky) / max(np.percentile(target, 75) - sky, 1e-6), 0, 1)
    highlights = np.clip(1 - target / 0.85, 0, 1)
    target = np.clip(target + POLISHED_LOCAL_CONTRAST * local * signal * highlights, 0, 1)
    nebula = nebula * (target / np.maximum(level, 1e-6))[..., None]
    # Never push a channel past 1: the pixel is scaled back, colour kept.
    return np.clip(nebula / np.maximum(nebula.max(axis=-1, keepdims=True), 1.0), 0, 1)


def polished_deep(nebula: np.ndarray, stars: np.ndarray) -> np.ndarray:
    """Polished render: the polished nebula tone, then the stars put back, contained."""
    nebula = polished_nebula_tone(nebula)
    return np.clip(1 - (1 - nebula) * (1 - controlled_stars(stars, POLISHED_STARS)), 0, 1)


def match_final_sky_level(
    nebula: np.ndarray,
    stars: np.ndarray,
    target: float,
    rounds: int = SKY_MATCH_ROUNDS,
    tolerance: float = SKY_MATCH_TOLERANCE,
) -> tuple[np.ndarray, float]:
    """The nebula layer lifted or lowered, in display light, until the polished render's sky level is ``target``; ``(layer, shift)``.

    ``target`` is the median luminance of the natural render of the same
    master (``natural_final_level``), stars included.  It is the level a
    person compares: once StarNet2 has taken the stars out, the polished
    image (denoised, its stars thinner) keeps a little more or less of their
    halos in the sky than the natural one, which the linear match
    (``match_sky_level``) cannot see.  A secant search on the shift, one
    offset for the three channels so the neutral sky stays neutral.
    """
    nebula = np.asarray(nebula, dtype=np.float32)

    def level(shift: float) -> float:
        lifted = np.clip(nebula + np.float32(shift), 0, 1)
        return float(np.median(lum(polished_deep(lifted, stars))))

    shift, reached = 0.0, level(0.0)
    previous: tuple[float, float] | None = None
    for _ in range(rounds):
        if abs(reached / target - 1.0) < tolerance:
            break
        if previous is None:
            step = 0.5 * (target - reached)  # the toe's slope at the sky is about 2: half is a safe first step
        else:
            slope = (reached - previous[1]) / (shift - previous[0])
            step = (target - reached) / slope if slope > 1e-3 else 0.5 * (target - reached)
        previous = (shift, reached)
        shift += step
        reached = level(shift)
    return np.clip(nebula + np.float32(shift), 0, 1), shift


def natural_final_level(nebula: np.ndarray, stars: np.ndarray) -> float:
    """The sky level of the natural render of a master: the median luminance of ``natural_deep``."""
    return float(np.median(lum(natural_deep(nebula, stars))))


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


def hoo_weight(nebula: np.ndarray) -> np.ndarray:
    """0 in the sky, 1 where Hα or OIII stand well above the noise; smooth in between (no threshold).

    The signal-to-noise ratio is read on the image smoothed at ``HOO_SMOOTHING``
    and taken through a smooth step, so the edge of a faint red zone is a
    gradient, not a line.
    """
    smooth = gaussian_filter(np.asarray(nebula, dtype=np.float32), sigma=(HOO_SMOOTHING, HOO_SMOOTHING, 0))
    ha, oiii = smooth[..., 0], 0.5 * (smooth[..., 1] + smooth[..., 2])
    snr = np.maximum(
        (ha - np.median(ha)) / (_background_noise(ha) + 1e-6),
        (oiii - np.median(oiii)) / (_background_noise(oiii) + 1e-6),
    )
    x = np.clip((snr - HOO_SNR_FROM) / (HOO_SNR_TO - HOO_SNR_FROM), 0, 1)
    return gaussian_filter(x * x * (3 - 2 * x), sigma=HOO_SMOOTHING).astype(np.float32)


def hoo_palette(nebula: np.ndarray) -> np.ndarray:
    """The HOO palette on the brightness of the natural tone, fading into the natural colours.

    Where Hα or OIII stand out of the noise the colour is the palette's (Hα to
    red, OIII to blue-green); in the sky it is the natural tone's own neutral
    sky, and the faint zones in between are a smooth mix of both.
    """
    nebula = np.clip(np.asarray(nebula, dtype=np.float32), 0, 1)
    toned = deep_nebula_tone(nebula)
    # The nebula layer keeps the linear ratios of the channels (the stretch
    # scales each pixel), so its red is Hα and the mean of green and blue OIII.
    smooth = gaussian_filter(nebula, sigma=(HOO_SMOOTHING, HOO_SMOOTHING, 0))
    ha, oiii = smooth[..., 0], 0.5 * (smooth[..., 1] + smooth[..., 2])
    ha_noise, oiii_noise = _background_noise(ha) + 1e-6, _background_noise(oiii) + 1e-6
    # Signals above the sky, the corner of the positive part rounded at the noise.
    ha_signal = _soft_positive(ha - np.median(ha), ha_noise)
    oiii_signal = _soft_positive(oiii - np.median(oiii), oiii_noise)
    ratio = np.percentile(ha_signal, 99.5) / max(np.percentile(oiii_signal, 99.5), 1e-6)
    oiii_signal = oiii_signal * float(np.clip(HOO_OIII_DAMPING * ratio, *HOO_OIII_BOOST))
    palette = np.stack([ha_signal, oiii_signal, oiii_signal], axis=-1)
    hsv = rgb2hsv(palette / max(float(palette.max()), 1e-6))
    value = np.max(toned, axis=-1)
    saturation = np.clip(hsv[..., 1] * HOO_SATURATION, 0, HOO_MAX_SATURATION)
    coloured = hsv2rgb(np.stack([hsv[..., 0], saturation, value], axis=-1))
    weight = hoo_weight(nebula)[..., None]
    mixed = toned * (1 - weight) + coloured * weight
    # The brightness is the natural one: the brightest channel is the natural tone's.
    return np.clip(mixed * (value / np.maximum(mixed.max(axis=-1), 1e-6))[..., None], 0, 1)


def hoo_deep(nebula: np.ndarray, stars: np.ndarray) -> np.ndarray:
    """HOO palette: the palette on the nebula, the stars in their natural colours."""
    nebula = hoo_palette(nebula)
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
