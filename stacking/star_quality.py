#!/usr/bin/env python3
"""The quality of a linear image, read on its stars.

Astro Sync measures the stacked image it sends: from the stars of the
**linear master** (before any gradient removal, stretch or deconvolution,
which would change the width of the stars and make the figure comparable to
nothing), the median full width at half maximum (FWHM, in pixels), the median
eccentricity of the stars (0 for a round star, towards 1 for a trailed one)
and the number of stars the figures rest on.

One function does it from the same stars (:func:`measure`):

1. the stars are the local maxima of the image more than 40 sigma above the
   sky, short of the brightest 60 % (the saturated ones), 10 px away from the
   edge, and **isolated**: no other maximum above 10 sigma within 10 px, so a
   double star never reads as one elongated star;
2. the FWHM of a star is read on its radial profile, around its centre of
   mass (the same code the deconvolution of ``renders.py`` uses:
   :func:`profile_fwhm`);
3. its eccentricity comes from the second moments of its light above 20 % of
   its peak, in a window that grows with its width:
   ``sqrt(1 - minor/major)`` of the two axes of the moment ellipse (a
   Gaussian star of any size keeps its axis ratio under that cut);
4. fewer than twenty stars: no measure (``None``), never a figure on a few.

Precision expected, on synthetic fields of Gaussian stars with noise (see
``test_star_quality.py``): the FWHM within 5 % from 3.5 px to 7 px (the
window of the radial profile is 15 px wide); under 3 px, an undersampled
field, it is read up to 15 % high and a round star keeps an eccentricity floor
of about 0.25 (the pixel grid), 0.1 to 0.15 above 3.5 px; the eccentricity of
a trailed star within 0.02 of the true ratio.  Nothing else than numpy and
scipy is needed, so a ``--linear-only`` stacking measures without the render
libraries.

Command line, to try a master by hand::

    python3 star_quality.py master.fit

prints the figures as JSON (and ``null`` when fewer than twenty stars are
found).  ``python3 star_quality.py --measure master.xisf`` is the entry the
window of Astro Sync uses for a master a person stacked elsewhere (FITS or
XISF, linear only): it prints ``{"status": "measured", "quality": ...}`` or
``{"status": "refused", "code": ..., "message": ...}`` (see
:class:`MasterError`).
"""

from __future__ import annotations

import json
import sys
import time
from pathlib import Path

import numpy as np
from scipy.ndimage import maximum_filter
from scipy.spatial import cKDTree

MINIMUM_STARS = 20
MEASURED_ON = "linear"
RADIUS = 7  # half width of the window of the radial profile, px
MARGIN = 10  # a star closer than this to an edge is left out, px
NEIGHBOUR = 10  # a star with another maximum this close is not isolated, px
LUMINANCE = (0.2126, 0.7152, 0.0722)  # the weights of ``renders.lum``


def luminance(data: np.ndarray) -> np.ndarray:
    """The image a measure is made on: a mono plane as it is, a 3 x H x W colour master in luminance."""
    data = np.asarray(data, dtype=np.float32)
    if data.ndim == 2:
        return data
    if data.ndim == 3 and data.shape[0] == 3:
        return LUMINANCE[0] * data[0] + LUMINANCE[1] * data[1] + LUMINANCE[2] * data[2]
    raise ValueError(f"expected a 2-D or 3 x H x W image, got {data.shape}")


def candidate_stars(level: np.ndarray, limit: int, isolated: bool = False) -> tuple[np.ndarray, np.ndarray, np.ndarray]:
    """``(signal, ys, xs)``: the sky-subtracted image and the pixels of the stars to measure.

    More than ``limit`` stars: ``limit`` of them, drawn at random with a fixed
    seed (the same stars each time).  ``isolated`` leaves out the stars with
    another maximum above 10 sigma within :data:`NEIGHBOUR` px.
    """
    background = np.median(level)
    signal = level - background
    sigma = np.median(np.abs(signal)) * 1.4826
    local_maxima = signal == maximum_filter(signal, size=9)
    peaks = local_maxima & (signal > 40 * sigma)
    peaks &= signal < 0.60 * signal.max()
    ys, xs = np.nonzero(peaks)
    height, width = level.shape
    inside = (ys > MARGIN) & (ys < height - MARGIN) & (xs > MARGIN) & (xs < width - MARGIN)
    ys, xs = ys[inside], xs[inside]
    if isolated and len(ys):
        others = np.argwhere(local_maxima & (signal > 10 * sigma))
        if len(others) > 1:
            tree = cKDTree(others)
            near = tree.query_ball_point(np.stack([ys, xs], axis=1), r=NEIGHBOUR, p=np.inf)
            alone = np.fromiter((len(found) == 1 for found in near), dtype=bool, count=len(ys))
            ys, xs = ys[alone], xs[alone]
    if len(ys) > limit:
        pick = np.random.default_rng(0).choice(len(ys), limit, replace=False)
        ys, xs = ys[pick], xs[pick]
    return signal, ys, xs


def cut_of(signal: np.ndarray, y: int, x: int, radius: int) -> np.ndarray:
    """The window of ``signal`` around ``(y, x)``, its own border level removed."""
    cut = signal[y - radius:y + radius + 1, x - radius:x + radius + 1].astype(np.float64)
    return cut - np.median(np.concatenate([cut[0], cut[-1], cut[:, 0], cut[:, -1]]))


def profile_fwhm(cut: np.ndarray, grid_y: np.ndarray, grid_x: np.ndarray, replicate_edges: bool = False) -> float | None:
    """The FWHM, in pixels, of the star in a ``cut`` (None when its profile does not fall below half).

    The profile is smoothed over five samples.  By default its ends are padded
    with zeros, which pulls the first samples down: a star whose first
    neighbours are under three quarters of its peak (about 3.1 px wide or
    less) reads as not falling below half and gives None.  ``replicate_edges``
    pads with the end values instead, so a narrower star is read (the measure
    of a single pose, whose stars are small; the master's measure keeps its
    original rule, its figures being compared from one night to the next).
    """
    peak, total = cut.max(), cut.sum()
    if peak <= 0 or total <= 0:
        return None
    centre_y = (cut * grid_y).sum() / total
    centre_x = (cut * grid_x).sum() / total
    distance = np.hypot(grid_y - centre_y, grid_x - centre_x).ravel()
    profile = (cut / peak).ravel()
    order = np.argsort(distance)
    distance, profile = distance[order], profile[order]
    if replicate_edges:
        smoothed = np.convolve(np.concatenate([np.full(2, profile[0]), profile, np.full(2, profile[-1])]), np.ones(5) / 5, mode="valid")
    else:
        smoothed = np.convolve(profile, np.ones(5) / 5, mode="same")
    below = np.nonzero(smoothed < 0.5)[0]
    if len(below) == 0 or below[0] == 0:
        return None
    i = below[0]
    d1, d0 = distance[i], distance[i - 1]
    v1, v0 = smoothed[i], smoothed[i - 1]
    if v0 == v1:
        return None
    return float(2 * (d0 + (0.5 - v0) * (d1 - d0) / (v1 - v0)))


def eccentricity_of(signal: np.ndarray, y: int, x: int, fwhm: float) -> float | None:
    """The eccentricity of the star at ``(y, x)``, in a window of about 1.6 FWHM around it (None: not readable)."""
    radius = int(min(MARGIN, max(5, np.ceil(1.6 * fwhm))))
    cut = cut_of(signal, y, x, radius)
    peak = cut.max()
    if peak <= 0:
        return None
    weights = np.clip(cut - 0.2 * peak, 0, None)
    total = weights.sum()
    if total <= 0:
        return None
    grid_y, grid_x = np.mgrid[-radius:radius + 1, -radius:radius + 1]
    mean_y = (weights * grid_y).sum() / total
    mean_x = (weights * grid_x).sum() / total
    var_y = (weights * (grid_y - mean_y) ** 2).sum() / total
    var_x = (weights * (grid_x - mean_x) ** 2).sum() / total
    cov = (weights * (grid_y - mean_y) * (grid_x - mean_x)).sum() / total
    half_trace = (var_y + var_x) / 2
    spread = np.hypot((var_y - var_x) / 2, cov)
    major, minor = half_trace + spread, half_trace - spread
    if major <= 0 or minor < 0:
        return None
    return float(np.sqrt(1 - minor / major))


def star_widths(level: np.ndarray, limit: int = 250, isolated: bool = False) -> list[tuple[float, float | None]]:
    """``(fwhm, eccentricity)`` of each star measured on a luminance ``level`` (the eccentricity can be None)."""
    signal, ys, xs = candidate_stars(level, limit, isolated)
    grid_y, grid_x = np.mgrid[-RADIUS:RADIUS + 1, -RADIUS:RADIUS + 1]
    found = []
    for y, x in zip(ys, xs):
        fwhm = profile_fwhm(cut_of(signal, y, x, RADIUS), grid_y, grid_x)
        if fwhm is None:
            continue
        found.append((fwhm, eccentricity_of(signal, y, x, fwhm)))
    return found


def median_fwhm(level: np.ndarray, limit: int = 250) -> float:
    """The median FWHM in pixels of the stars of ``level`` (NaN when none can be read): ``renders.star_fwhm``."""
    widths = [fwhm for fwhm, _ in star_widths(level, limit)]
    return float(np.median(widths)) if widths else float("nan")


def measure(level: np.ndarray, limit: int = 400) -> dict[str, object] | None:
    """The quality of a linear luminance image, or None under :data:`MINIMUM_STARS` stars.

    ``{"fwhmPixels", "eccentricity", "starCount", "measuredOn": "linear"}``:
    the medians over the stars whose FWHM **and** eccentricity could be read.
    """
    stars = [(fwhm, ecc) for fwhm, ecc in star_widths(level, limit, isolated=True)
             if ecc is not None and np.isfinite(fwhm) and fwhm > 0]
    if len(stars) < MINIMUM_STARS:
        return None
    widths = np.array([fwhm for fwhm, _ in stars])
    eccentricities = np.array([ecc for _, ecc in stars])
    return {
        "fwhmPixels": round(float(np.median(widths)), 3),
        "eccentricity": round(float(np.median(eccentricities)), 3),
        "starCount": len(stars),
        "measuredOn": MEASURED_ON,
    }


def measure_master(path: Path) -> dict[str, object] | None:
    """:func:`measure` of the linear master FITS at ``path`` (mono, or 3 x H x W colour)."""
    from astropy.io import fits

    data = np.nan_to_num(np.asarray(fits.getdata(path, memmap=False), dtype=np.float32))
    return measure(luminance(data))


# --- A master given by hand ----------------------------------------------------
#
# A person who stacks in PixInsight or Siril hands Astro Sync the master of
# their own stacking.  It is measured by the very same code (:func:`measure`),
# but only a **linear** master has a width of star that means something: a
# stretched image (an autostretch, a histogram transformation, a JPEG) has
# stars that are wider, and rounder, than the data they came from.  So the
# image is refused when its histogram says it was stretched.

FITS_SUFFIXES = (".fit", ".fits", ".fts")
XISF_SUFFIXES = (".xisf",)
# The position of the median in the range of the image (lowest to highest
# value): a linear master keeps its sky at a few thousandths of the range
# (0.03 to 0.06 % on the master of a real night, 0.3 to 0.8 % on a raw frame;
# a bright sky raises the lowest value with the median, not the position).  An
# autostretch brings it to a quarter of the range, a histogram transformation
# higher, and a gentle stretch of 3 % already widens the stars by a tenth.
# Above 2 % -- twenty to forty times a real master -- the image is said
# stretched (at 18 % an image stretched to 5 % was accepted with its stars
# wider, or refused for the wrong reason).
STRETCHED_MEDIAN = 0.02


class MasterError(Exception):
    """A master that cannot be measured; ``code`` is one of the words below, ``str(error)`` says it in English.

    ``unsupported_format`` (not FITS or XISF), ``unreadable`` (damaged, or an
    XISF the reader does not cover), ``not_an_image`` (no usable image in
    it), ``stretched`` (not linear), ``too_few_stars``.
    """

    def __init__(self, code: str, message: str) -> None:
        super().__init__(message)
        self.code = code


def read_pixels(path: Path) -> np.ndarray:
    """The image of the FITS or XISF file at ``path`` on its format's scale (``H x W`` or ``C x H x W``)."""
    suffix = path.suffix.lower()
    if suffix in XISF_SUFFIXES:
        import xisf_reader

        try:
            return xisf_reader.normalise(xisf_reader.read_xisf(path))
        except xisf_reader.XisfError as error:
            raise MasterError("unreadable", str(error)) from error
        except OSError as error:
            raise MasterError("unreadable", f"the file could not be read: {error}") from error
    if suffix in FITS_SUFFIXES:
        from astropy.io import fits

        try:
            data = fits.getdata(path, memmap=False)
        except (OSError, ValueError, TypeError) as error:
            raise MasterError("unreadable", f"the FITS file could not be read: {error}") from error
        if data is None:
            raise MasterError("not_an_image", "the FITS file holds no image")
        data = np.asarray(data)
        # An 8-bit plane is an image made to be looked at, never a master.
        if data.dtype == np.uint8:
            raise MasterError("stretched", "an 8-bit image is not a linear master")
        if np.issubdtype(data.dtype, np.integer):
            data = data.astype(np.float32) / float(np.iinfo(data.dtype).max)
        return data.astype(np.float32)
    raise MasterError("unsupported_format", "only a FITS or XISF image can be measured")


def stretch_reason(level: np.ndarray) -> str | None:
    """Why ``level`` is not a linear master, or None when it looks linear (the histogram's median, see :data:`STRETCHED_MEDIAN`)."""
    finite = level[np.isfinite(level)]
    low, high = float(finite.min()), float(finite.max())
    position = (float(np.median(finite)) - low) / (high - low)
    if position > STRETCHED_MEDIAN:
        return f"stretched image: the median sits at {position:.1%} of its range, a linear master stays under {STRETCHED_MEDIAN:.0%}"
    return None


def measure_file(path: Path) -> dict[str, object]:
    """The quality of the master at ``path`` (FITS or XISF), as :func:`measure` gives it, or a :class:`MasterError`."""
    data = np.nan_to_num(read_pixels(path))
    if data.ndim == 3 and data.shape[0] == 1:
        data = data[0]
    if data.ndim == 3 and data.shape[0] != 3:
        raise MasterError("not_an_image", f"expected a mono or 3-channel image, got {data.shape}")
    if data.ndim not in (2, 3):
        raise MasterError("not_an_image", f"expected a 2-D image, got {data.shape}")
    level = luminance(data)
    if min(level.shape) < 4 * MARGIN:
        raise MasterError("not_an_image", "the image is too small to hold stars")
    if float(level.max()) <= float(level.min()):
        raise MasterError("not_an_image", "the image is flat: it holds no star")
    reason = stretch_reason(level)
    if reason is not None:
        raise MasterError("stretched", reason)
    quality = measure(level)
    if quality is None:
        raise MasterError("too_few_stars", f"fewer than {MINIMUM_STARS} readable stars")
    quality["measuredAt"] = time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())
    return quality


def main(argv: list[str] | None = None) -> int:
    arguments = sys.argv[1:] if argv is None else argv
    if len(arguments) == 2 and arguments[0] == "--measure":
        # What Astro Sync's window asks for a master a person gave: one JSON
        # line, ``measured`` with the figures or ``refused`` with a code.
        try:
            answer = {"status": "measured", "quality": measure_file(Path(arguments[1]))}
        except MasterError as error:
            answer = {"status": "refused", "code": error.code, "message": str(error)}
        print(json.dumps(answer))
        return 0
    if len(arguments) != 1:
        print("usage: star_quality.py MASTER.fit\n       star_quality.py --measure MASTER.fit|MASTER.xisf", file=sys.stderr)
        return 2
    print(json.dumps(measure_master(Path(arguments[0])), indent=2))
    return 0


if __name__ == "__main__":
    sys.exit(main())
