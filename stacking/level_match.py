"""The scale between two aligned masters, read as the light of the same stars.

``plan_combine.match_to`` brings each master of a plan to the level of the longest one of its band, for every
plan (one rule, whatever the source of the masters). The first scale it used (:func:`robust_fit`, kept below) is
the median ratio of the **sharp** light of the star pixels: it follows the peak of a star. Two masters never
see the same star with the same width: the device's own stack (a Seestar's ``Stacked_*.fit``) has stars 10 to
13 % wider than a stack of the poses by Astro Sync, so the peaks of the sharper image are higher for the same
light. Measured on the night of 10/11 October 2026: against the true ratio of the light of the stars
(aperture photometry) the peak-based scale is 16 to 32 % low between one of our masters and a device image
(it is already -12 % to +8 % between two of ours); the plan's stars then weigh 0.72 of what they should.

This module reads the scale where it does not depend on the width of a star: **the flux in an aperture** wide
enough to hold the wider star and its wings (4 FWHM, 10 to 20 px), minus the sky of the ring around it, for the same isolated, unsaturated stars in
both images (they are on the same pixels once aligned).  The scale of a channel is the median of the flux
ratios; the offset (the sky) is left to the caller, as it was.

With fewer than ``MINIMUM_STARS`` usable stars (a nearly empty field) there is no aperture scale: the caller
fits the pixels of the stars instead (:func:`robust_fit`) and says so in the record (``apertureUnavailable``).
:func:`clipped_median` gives the caller its offset, the sky left over.

Only numpy and scipy are needed (the stacking environment).
"""

from __future__ import annotations

from dataclasses import dataclass

import numpy as np
from scipy.ndimage import distance_transform_edt, maximum_filter, minimum_filter
from scipy.spatial import cKDTree

import star_quality

LUMINANCE = star_quality.LUMINANCE

# The aperture: at least this radius in pixels, 4 times the widest FWHM of the two images, at most the maximum.
# Measured on real nights, a former radius of 1.6 FWHM (at least 6 px) was too small: the star light grows with the
# radius up to about 10 to 12 px and a smaller aperture leaves the scale short of the light of the stars (Askar 50P
# SH2-190: 14 % short, Seestar M 45 halves: 2 to 3 %, a device image: 1 to 2 %); from 4 FWHM (10 px at least) the scale
# is within 2 % of the one read on a 16 px aperture on all three.
MINIMUM_RADIUS = 10.0
RADIUS_PER_FWHM = 4.0
MAXIMUM_RADIUS = 20.0
# The ring that gives the local sky starts this far beyond the aperture and is this wide.
RING_GAP, RING_WIDTH = 4.0, 5.0
# A star counts when its peak stands this many robust sigmas above the sky, and no other such peak is this close.
PEAK_SIGMAS = 12.0
ISOLATION = 14
# A star whose peak is over this share of the brightest pixels of its image may be saturated: left out.
SATURATION_SHARE = 0.55
# A pixel over this share is taken as saturated when a star's aperture or ring is checked for one near it.
NEAR_SATURATION_SHARE = 0.9
# The stars read, the least that makes a scale, and the margin from an edge of the data (pixels).
STAR_COUNT = 400
MINIMUM_STARS = 20
MARGIN = 40


@dataclass(frozen=True)
class ApertureScale:
    """The scale of each channel, and what it rests on."""

    scales: tuple[float, ...]
    stars: int
    radius: float


def _luminance(image: np.ndarray) -> np.ndarray:
    return image if image.ndim == 2 else (LUMINANCE[0] * image[0] + LUMINANCE[1] * image[1] + LUMINANCE[2] * image[2])


def _sky(plane: np.ndarray, valid: np.ndarray) -> tuple[float, float]:
    """The sky level and its noise, robustly, on the pixels of ``valid``."""
    step = max(1, int(valid.sum()) // 400_000)
    sample = plane[valid][::step]
    level = float(np.median(sample))
    spread = 1.4826 * float(np.median(np.abs(sample - level)))
    kept = sample[np.abs(sample - level) < 6 * spread] if spread > 0 else sample
    level = float(np.median(kept))
    return level, 1.4826 * float(np.median(np.abs(kept - level)))


def _ceiling(work: np.ndarray, valid: np.ndarray, share: float = SATURATION_SHARE) -> float:
    """The level over which a star may be saturated: a share of the brightest pixels of the image (sky removed)."""
    step = max(1, int(valid.sum()) // 2_000_000)
    return share * float(np.percentile(work[valid][::step], 99.999))


def find_stars(level: np.ndarray, valid: np.ndarray, count: int = STAR_COUNT) -> tuple[np.ndarray, float]:
    """The brightest isolated, unsaturated stars of a luminance ``level``, as ``(y, x)`` centres, and its sky.

    ``valid``: the pixels of data; a star nearer than ``MARGIN`` pixels to a pixel without data is left out.
    """
    sky, sigma = _sky(level, valid)
    work = level - sky
    inner = minimum_filter(valid.astype(np.uint8), size=2 * MARGIN + 1, mode="constant", cval=0).astype(bool)
    peaks = (work == maximum_filter(work, size=9)) & (work > PEAK_SIGMAS * sigma) & inner
    peaks &= work < _ceiling(work, valid)
    ys, xs = np.nonzero(peaks)
    if len(ys) == 0:
        return np.empty((0, 2)), sky
    amplitude = work[ys, xs]
    tree = cKDTree(np.column_stack([ys, xs]))
    alone = np.fromiter((len(near) == 1 for near in tree.query_ball_point(np.column_stack([ys, xs]), ISOLATION)),
                        dtype=bool, count=len(ys))
    ys, xs, amplitude = ys[alone], xs[alone], amplitude[alone]
    order = np.argsort(-amplitude)[:count]
    centres = []
    grid_y, grid_x = np.mgrid[-4:5, -4:5]
    for y, x in zip(ys[order], xs[order]):
        cut = np.clip(work[y - 4:y + 5, x - 4:x + 5], 0, None)
        total = float(cut.sum())
        if total <= 0:
            continue
        centres.append((y + float((cut * grid_y).sum()) / total, x + float((cut * grid_x).sum()) / total))
    return np.array(centres).reshape(-1, 2), sky


def aperture_flux(plane: np.ndarray, y: float, x: float, radius: float) -> float:
    """The flux of the star at ``(y, x)`` in a circle of ``radius`` pixels, minus the median of the ring around it."""
    outer = radius + RING_GAP + RING_WIDTH
    y0, x0 = int(round(y)), int(round(x))
    reach = int(np.ceil(outer)) + 1
    if y0 - reach < 0 or x0 - reach < 0 or y0 + reach + 1 > plane.shape[0] or x0 + reach + 1 > plane.shape[1]:
        return float("nan")
    cut = plane[y0 - reach:y0 + reach + 1, x0 - reach:x0 + reach + 1]
    grid_y, grid_x = np.mgrid[-reach:reach + 1, -reach:reach + 1]
    distance = np.hypot(grid_y + (y0 - y), grid_x + (x0 - x))
    sky = float(np.median(cut[(distance >= radius + RING_GAP) & (distance <= outer)]))
    return float((cut[distance <= radius] - sky).sum())


def fwhm_of(level: np.ndarray) -> float:
    """The median width of the stars of ``level`` in pixels, NaN when none can be read."""
    try:
        return star_quality.median_fwhm(level, 150)
    except (ValueError, FloatingPointError):
        return float("nan")


def aperture_radius(*levels: np.ndarray) -> float:
    """The aperture radius that holds the light of the wider of the stars of ``levels`` (and its wings)."""
    widths = [w for w in (fwhm_of(level) for level in levels) if np.isfinite(w) and w > 0]
    return min(MAXIMUM_RADIUS, max(MINIMUM_RADIUS, RADIUS_PER_FWHM * max(widths))) if widths else MINIMUM_RADIUS


def aperture_scale(reference: np.ndarray, image: np.ndarray, valid: np.ndarray | None = None) -> ApertureScale | None:
    """The scale per channel such that ``reference ≈ scale × image`` on the light of the stars, or ``None``.

    ``reference`` and ``image``: aligned masters, ``(3, h, w)`` or ``(h, w)``; ``valid``: the pixels both have
    data on.  ``None`` when fewer than ``MINIMUM_STARS`` stars can be read in both.
    """
    reference = np.nan_to_num(np.asarray(reference, np.float32))
    image = np.nan_to_num(np.asarray(image, np.float32))
    if valid is None:
        valid = np.ones(reference.shape[-2:], bool)
    if not valid.any():
        return None
    level_ref, level_img = _luminance(reference), _luminance(image)
    radius = aperture_radius(level_ref, level_img)
    # The candidates are taken three times over: those with a saturated pixel (in either image) in their aperture
    # or ring are left out below, and the brightest STAR_COUNT of the rest are kept.
    stars, sky_ref = find_stars(level_ref, valid, 3 * STAR_COUNT)
    if len(stars):
        sky_img, sigma_img = _sky(level_img, valid)
        ceiling_img = _ceiling(level_img - sky_img, valid)
        near_img = _ceiling(level_img - sky_img, valid, NEAR_SATURATION_SHARE)
        near_ref = _ceiling(level_ref - sky_ref, valid, NEAR_SATURATION_SHARE)
        # A star saturated in the other image would read short there: it goes too.
        keep = [level_img[int(round(y)), int(round(x))] - sky_img < ceiling_img
                and level_img[int(round(y)), int(round(x))] - sky_img > PEAK_SIGMAS * sigma_img / 4 for y, x in stars]
        stars = stars[np.array(keep, bool)].reshape(-1, 2)
        # A bright star near a saturated one has the saturated one's light in its aperture or in its ring, which
        # is not the same in both images: the distance of each star to the nearest saturated pixel is checked.
        saturated = valid & (((level_ref - sky_ref) >= near_ref) | ((level_img - sky_img) >= near_img))
        if len(stars) and saturated.any():
            distance = distance_transform_edt(~saturated)
            clear = np.array([distance[int(round(y)), int(round(x))] > radius + RING_GAP + RING_WIDTH for y, x in stars], bool)
            stars = stars[clear].reshape(-1, 2)
        stars = stars[:STAR_COUNT]
    planes_ref = [reference] if reference.ndim == 2 else list(reference)
    planes_img = [image] if image.ndim == 2 else list(image)
    ratios: list[list[float]] = [[] for _ in planes_ref]
    for y, x in stars:
        row = []
        for ref_plane, img_plane in zip(planes_ref, planes_img):
            a, b = aperture_flux(ref_plane, y, x, radius), aperture_flux(img_plane, y, x, radius)
            if not (np.isfinite(a) and np.isfinite(b) and a > 0 and b > 0):
                break
            row.append(a / b)
        else:
            for channel, ratio in enumerate(row):
                ratios[channel].append(ratio)
    stars_used = len(ratios[0])
    if stars_used < MINIMUM_STARS:
        return None
    return ApertureScale(tuple(float(np.median(r)) for r in ratios), stars_used, radius)


# --- The fit on the pixels of the stars, and the sky left over --------------------------------------------------

#: The pixels a residual sets aside, in robust standard deviations, and how many times the median is taken again.
CLIP_SIGMAS = 3.0
CLIP_ROUNDS = 5
#: A star's pixel stands this many robust standard deviations above its neighbourhood.
STAR_SIGMAS = 10.0
#: The side, in pixels, of the neighbourhood whose mean is taken away to keep only the stars.
STAR_BOX = 7
#: Below this many star pixels the slope is a plain least-squares fit.
MIN_STAR_PIXELS = 50
#: At most this many pixels feed a median (an even sample).
SAMPLE_PIXELS = 400_000


@dataclass(frozen=True)
class LineFit:
    """``target ≈ slope × source + intercept``, and the star pixels it read."""

    slope: float
    intercept: float
    kept: int


def _robust_sigma(values: np.ndarray, centre: float) -> float:
    return 1.4826 * float(np.median(np.abs(values - centre)))


def _sharp(image: np.ndarray) -> np.ndarray:
    """``image`` without its smooth parts: each pixel minus its neighbourhood's mean."""
    from scipy.ndimage import uniform_filter

    return image - uniform_filter(image, size=STAR_BOX, mode="nearest")


def clipped_median(values: np.ndarray) -> float:
    """The median of ``values``, its outliers set aside (``CLIP_SIGMAS``, ``CLIP_ROUNDS``)."""
    step = max(1, values.size // SAMPLE_PIXELS)
    sample = values.reshape(-1)[::step]
    sample = sample[np.isfinite(sample)]
    if sample.size == 0:
        return 0.0
    centre = float(np.median(sample))
    for _ in range(CLIP_ROUNDS):
        sigma = _robust_sigma(sample, centre)
        if sigma == 0:
            break
        kept = sample[np.abs(sample - centre) <= CLIP_SIGMAS * sigma]
        if kept.size == 0 or kept.size == sample.size:
            break
        sample = kept
        centre = float(np.median(sample))
    return centre


def robust_fit(source: np.ndarray, target: np.ndarray, valid: np.ndarray | None = None) -> LineFit:
    """``target ≈ slope × source + intercept``, read on the pixels of the stars.

    ``slope``: the median ratio of the stars' sharp light in ``target`` and in ``source``; ``intercept``: the
    sky left by ``target − slope × source``.  With too few stars, a least-squares fit.  ``valid``: the pixels
    both images have data on; what is outside is flat-filled, and neither the stars, the sigma nor the sky read it.
    """
    x = np.nan_to_num(np.asarray(source, np.float32))
    y = np.nan_to_num(np.asarray(target, np.float32))
    if valid is None:
        valid = np.ones(x.shape, bool)
    elif not valid.any():
        raise ValueError("the two images have no pixel in common")
    elif not valid.all():
        x = np.where(valid, x, np.median(x[valid]))
        y = np.where(valid, y, np.median(y[valid]))
    sharp_x, sharp_y = _sharp(x), _sharp(y)
    inner = valid
    if not valid.all():
        from scipy.ndimage import binary_erosion

        inner = binary_erosion(valid, np.ones((STAR_BOX, STAR_BOX), bool), border_value=1)
    step = max(1, int(valid.sum()) // SAMPLE_PIXELS)
    sigma = _robust_sigma(sharp_x[inner][::step], 0.0)
    stars = (sharp_x > STAR_SIGMAS * sigma) & inner if sigma > 0 else np.zeros(x.shape, bool)
    if int(stars.sum()) >= MIN_STAR_PIXELS:
        slope = float(np.median(sharp_y[stars] / sharp_x[stars]))
    else:
        xs, ys = x[valid][::step], y[valid][::step]
        slope = float(np.polyfit(xs, ys, 1)[0]) if xs.size >= 3 and np.ptp(xs) > 0 else 0.0
    intercept = clipped_median((y - slope * x)[valid])
    return LineFit(slope=slope, intercept=intercept, kept=int(stars.sum()))
