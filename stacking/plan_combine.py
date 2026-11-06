"""The masters of a multi-night plan, combined into one linear image.

The masters arrive aligned pixel to pixel on the reference night
(``plan_stack.py`` asks Siril for that); this module only does the
arithmetic, in three steps:

1. **One level per band.**  Siril normalises each night's master its own
   way, and the sky is never the same from one night to the next.  Within a
   band (broadband, or dual-band), each master is brought to the level of the
   longest one, channel by channel: ``reference ≈ a × master + b``, the
   scale ``a`` read on the **light** of the stars and the offset ``b`` on the sky.  The scale is the median
   ratio of the flux of the same isolated, unsaturated stars in an aperture wide enough to hold the light of
 the wider star and its wings (4 FWHM, 10 px at least: ``level_match.py``): it does not depend on how sharp a star is, which the peak of a
   star does (the stars of a device's image are 10 to 13 % wider than ours, and the peak-based scale
   was measured 16 to 32 % low there, and -12 % to +8 % between two of our own masters).  The offset
   is the clipped median of the sky left over (``level_match.clipped_median``), so the scale and the
   sky background are equalised and a nebula or a satellite never decides them.  One rule for every plan,
   whatever the source of the masters; the only exception is a field with too few readable stars (fewer
   than ``level_match.MINIMUM_STARS``), where the aperture has nothing to measure: the scale is then
   fitted on the pixels of the stars (``level_match.robust_fit``, the former measure) and the record says
   so (``apertureUnavailable``).
2. **One master per band.**  The masters of a band are averaged with a
   weight equal to their integration time (``w_i = T_i / ΣT``): two hours
   weigh twice one hour, as if the frames had been stacked together.
3. **The image** is the weighted mean of step 2 over every master.  A plan of a
   colour sensor with a dual-band master never comes here: it is combined on
   its dual-band master by ``dualband_reference.py`` (reproduced to the
   bit).

Masters that do not cover the same field (the nights turned differently: ``plan_stack.py``
keeps the whole frame of the reference night) leave empty borders, which Siril fills with
exact zeros: every step reads only the pixels a master has data on, and a pixel is
averaged over the masters that cover it.

**What each part of the frame holds**: ``coverage_report`` says, from the same ``coverage``
of each master, how much of the frame has no dual-band data (the corners the dual-band
nights do not reach keep their colour from the broadband ones, without the Hα and OIII),
the exposure time of each zone (the sum of the integrations that cover it), and the largest
axis-aligned rectangle every master covers (the image the person may crop to).

Only numpy and scipy are needed (both in the stacking environment).
"""

from __future__ import annotations

from dataclasses import dataclass

import numpy as np

import level_match
import plan_framing

BROADBAND, DUALBAND = "broadband", "dualband"

# Where the master of a night comes from: the integration Astro Sync kept of its poses, or the image the
# device stacked itself (a Seestar's ``Stacked_*.fit``, converted to a master by ``plan_stack.py``).
OWN, DEVICE = "master", "device"

# Lanczos4 mixes the last pixels of a frame with the empty side: they are not data.
EDGE_PIXELS = 2

# A master must cover at least this share of the master it is brought to the level of.
MIN_OVERLAP = 0.05


@dataclass(frozen=True)
class Master:
    """One master of the plan, as the job names it."""

    night_id: str
    evening_date: str
    filter: str | None
    band: str
    seconds: float
    source: str = OWN


def channels(image: np.ndarray) -> list[np.ndarray]:
    """The planes of an image: three for colour, one for a monochrome one."""
    return list(image) if image.ndim == 3 else [image]


def coverage(image: np.ndarray) -> np.ndarray:
    """The pixels an aligned master has data on: Siril fills what its frame does not cover with exact zeros."""
    data = (image != 0).any(axis=0) if image.ndim == 3 else image != 0
    if data.all():
        return data
    from scipy.ndimage import binary_erosion

    return binary_erosion(data, np.ones((3, 3), bool), iterations=EDGE_PIXELS, border_value=1)


def match_to(reference: np.ndarray, image: np.ndarray, valid: np.ndarray | None = None) -> np.ndarray:
    """``image`` brought to the level of ``reference``, channel by channel (step 1).

    ``valid``: the pixels both have data on, the only ones the level is read on.  The scale is the ratio of
    the light of the stars in an aperture (``level_match``), whatever the sharpness of the stars of each
    image; the pixels-of-the-stars fit is used only when too few stars can be read.
    """
    return match_report(reference, image, valid)[0]


def match_report(reference: np.ndarray, image: np.ndarray,
                 valid: np.ndarray | None = None) -> tuple[np.ndarray, dict[str, object]]:
    """:func:`match_to`, and what was measured: the method (``aperture``, or ``peaks`` when too few stars), the scale of each channel."""
    if valid is not None and valid.mean() < MIN_OVERLAP:
        raise ValueError(f"a master covers {valid.mean():.1%} of the one it is brought to the level of, not enough to match them")
    measured = level_match.aperture_scale(reference, image, valid)
    matched, scales = [], []
    for number, (ref_plane, plane) in enumerate(zip(channels(reference), channels(image))):
        if measured is not None:
            scale = measured.scales[number]
            sky = np.nan_to_num(np.asarray(ref_plane, np.float32)) - scale * np.nan_to_num(np.asarray(plane, np.float32))
            offset = level_match.clipped_median(sky if valid is None else sky[valid])
        else:
            fit = level_match.robust_fit(plane, ref_plane, valid)
            scale = fit.slope if np.isfinite(fit.slope) and fit.slope > 0 else 1.0
            offset = fit.intercept
        scales.append(round(float(scale), 5))
        matched.append(plane * scale + offset)
    stacked = np.stack(matched) if image.ndim == 3 else matched[0]
    report: dict[str, object] = {"method": "aperture" if measured is not None else "peaks", "scales": scales}
    if measured is not None:
        report.update(stars=measured.stars, apertureRadius=round(measured.radius, 2))
    else:
        report["apertureUnavailable"] = True
    return stacked.astype(np.float32), report


def weighted_mean(images: list[np.ndarray], seconds: list[float],
                  covers: list[np.ndarray]) -> tuple[np.ndarray, np.ndarray]:
    """``Σ w_i × image_i`` with ``w_i = T_i / ΣT`` (step 2), and the pixels something covers.

    ``covers``: the pixels each image has data on; a pixel is averaged over the
    images that cover it, their weights brought back to one.  Where none does, the
    plain mean (the empty borders of the images, zeros).
    """
    total = float(sum(seconds))
    weights = [s / total for s in seconds] if total > 0 else [1 / len(images)] * len(images)
    mean = np.zeros_like(images[0], dtype=np.float64)
    weight_sum = np.zeros(images[0].shape[-2:], dtype=np.float64)
    for image, weight, cover in zip(images, weights, covers):
        mean += weight * image * cover
        weight_sum += weight * cover
    covered = weight_sum > 0
    mean /= np.where(covered, weight_sum, 1.0)
    if not covered.all():
        plain = np.zeros_like(images[0], dtype=np.float64)
        for image, weight in zip(images, weights):
            plain += weight * image
        mean = np.where(covered, mean, plain)
    return mean.astype(np.float32), covered


def longest(indices: list[int], masters: list[Master]) -> int:
    """The index of the longest master among ``indices``."""
    return max(indices, key=lambda i: (masters[i].seconds, -i))


def band_mean(indices: list[int], masters: list[Master], images: list[np.ndarray],
              covers: list[np.ndarray], levels: list[dict[str, object]] | None = None) -> tuple[np.ndarray, np.ndarray]:
    """Steps 1 and 2 for one band: matched to its longest master, then averaged; the pixels it covers too.

    The scale is read on the light of the stars in an aperture; ``levels`` (when given) receives, for each
    master matched, what was measured.
    """
    best = longest(indices, masters)
    matched = []
    for i in indices:
        if i == best:
            matched.append(images[i])
            continue
        image, report = match_report(images[best], images[i], covers[best] & covers[i])
        if levels is not None:
            levels.append({"nightId": masters[i].night_id, "to": masters[best].night_id, **report})
        matched.append(image)
    return weighted_mean(matched, [masters[i].seconds for i in indices], [covers[i] for i in indices])


def combine(masters: list[Master], images: list[np.ndarray]) -> tuple[np.ndarray, dict[str, object]]:
    """The plan's linear image, and what was done (for the provenance)."""
    if not masters or len(masters) != len(images):
        raise ValueError("one image per master is needed")
    shapes = {image.shape for image in images}
    if len(shapes) != 1:
        raise ValueError(f"the aligned masters differ in size: {sorted(shapes)}")
    total = sum(m.seconds for m in masters)
    weights = [{"nightId": m.night_id, "filter": m.filter, "band": m.band,
                "seconds": m.seconds, "weight": round(m.seconds / total, 4) if total else 0,
                **({"source": m.source} if m.source != OWN else {})}
               for m in masters]
    levels: list[dict[str, object]] = []
    broad = [i for i, m in enumerate(masters) if m.band != DUALBAND]
    dual = [i for i, m in enumerate(masters) if m.band == DUALBAND]
    colour = images[0].ndim == 3 and images[0].shape[0] == 3
    covers = [coverage(image) for image in images]
    if broad and dual and colour:
        raise ValueError("a colour plan with a dual-band master is combined by dualband_reference.py")
    every = list(range(len(masters)))
    mean, covered = band_mean(every, masters, images, covers, levels)
    return mean, {"method": "weighted_mean", "masters": weights, "noData": no_data(covered),
                  "levelMatch": levels, "coverage": coverage_report(masters, covers, colour)}


def no_data(covered: np.ndarray) -> float:
    """The share of the image no master covers (0 unless the nights turned differently and a border stays empty)."""
    return round(float(1.0 - covered.mean()), 4)


# The largest rectangle is searched exactly on a mask of at most this many pixels on its long side;
# a bigger frame is searched on blocks, then the rectangle is grown to the pixel.
RECTANGLE_SEARCH_SIDE = 512


def exact_rectangle(mask: np.ndarray) -> tuple[int, int, int, int]:
    """The largest axis-aligned rectangle of ``True`` in ``mask``, as ``(top, left, height, width)``.

    The classic histogram search, row by row: ``(0, 0, 0, 0)`` when nothing is ``True``.  Of two rectangles of
    the same area the first found (top row first) is kept.
    """
    rows, columns = mask.shape
    heights = [0] * columns
    best, found = 0, (0, 0, 0, 0)
    for row in range(rows):
        line = mask[row]
        for column in range(columns):
            heights[column] = heights[column] + 1 if line[column] else 0
        stack: list[tuple[int, int]] = []  # (start column, height), the heights growing
        for column in range(columns + 1):
            height = heights[column] if column < columns else 0
            start = column
            while stack and stack[-1][1] >= height:
                start, top_height = stack.pop()
                area = top_height * (column - start)
                if area > best:
                    best, found = area, (row - top_height + 1, start, top_height, column - start)
            stack.append((start, height))
    return found


def grow_rectangle(mask: np.ndarray, rectangle: tuple[int, int, int, int]) -> tuple[int, int, int, int]:
    """``rectangle`` grown, one line at a time, on every side whose next line is all ``True``."""
    top, left, height, width = rectangle
    bottom, right = top + height, left + width
    rows, columns = mask.shape
    grown = True
    while grown:
        grown = False
        if top > 0 and mask[top - 1, left:right].all():
            top, grown = top - 1, True
        if bottom < rows and mask[bottom, left:right].all():
            bottom, grown = bottom + 1, True
        if left > 0 and mask[top:bottom, left - 1].all():
            left, grown = left - 1, True
        if right < columns and mask[top:bottom, right].all():
            right, grown = right + 1, True
    return top, left, bottom - top, right - left


def largest_rectangle(mask: np.ndarray) -> tuple[int, int, int, int]:
    """The largest axis-aligned rectangle of ``True`` in ``mask``: ``(top, left, height, width)``.

    Exact up to ``RECTANGLE_SEARCH_SIDE`` pixels on the long side.  A bigger frame is searched on blocks
    (a block counts when all its pixels do), then the rectangle is grown to the pixel on each side: it is a
    rectangle nothing can enlarge, within a block of the largest one.
    """
    rows, columns = mask.shape
    if mask.all():
        return 0, 0, rows, columns
    block = -(-max(rows, columns) // RECTANGLE_SEARCH_SIDE)
    if block == 1:
        return exact_rectangle(mask)
    coarse = mask[:rows // block * block, :columns // block * block]
    coarse = coarse.reshape(rows // block, block, columns // block, block).all(axis=(1, 3))
    top, left, height, width = exact_rectangle(coarse)
    if height == 0:
        return 0, 0, 0, 0
    return grow_rectangle(mask, (top * block, left * block, height * block, width * block))


def coverage_report(masters: list[Master], covers: list[np.ndarray], colour: bool) -> dict[str, object]:
    """What each part of the frame holds, from the pixels each master has data on.

    * ``noDualBandShare``: the share of the frame (0 to 1) with broadband data and no dual-band one: with a
      colour sensor and both bands, the corners the dual-band masters do not reach; 0 otherwise;
    * ``zones``: the exposure time of each zone of the frame, the one the integrations covering it add up to,
      and the part of it that is dual-band (« centre: 7 h, of which 3 h in Hα »), the zone with the most time
      first; a zone no master covers is not listed (that is ``noData``);
    * ``fullCoverage``: the largest axis-aligned rectangle every master covers (the image cropped to what
      every night reaches: its size, its place, its share of the frame), and the size of the frame;
 * ``degradedShare``, ``fullSeconds``, ``fullDualBandSeconds``, ``framings`` (``plan_framing``):
      the share of the frame below the full exposure and the notches of the slider that goes from the cropped
      image to the whole one (nothing in ``framings`` when there is nothing to choose).
    """
    rows, columns = covers[0].shape
    size = rows * columns
    seconds = np.zeros((rows, columns), np.int64)
    dual_seconds = np.zeros((rows, columns), np.int64)
    every = np.ones((rows, columns), bool)
    for master, cover in zip(masters, covers):
        whole = int(round(master.seconds))
        seconds += whole * cover
        if master.band == DUALBAND:
            dual_seconds += whole * cover
        every &= cover
    keys, counts = np.unique(seconds * (1 << 31) + dual_seconds, return_counts=True)
    zones = [{"share": round(int(count) / size, 4), "seconds": int(key >> 31), "dualBandSeconds": int(key & ((1 << 31) - 1))}
             for key, count in zip(keys.tolist(), counts.tolist()) if key >> 31 > 0]
    zones.sort(key=lambda zone: (-zone["seconds"], -zone["dualBandSeconds"]))
    broad = [c for m, c in zip(masters, covers) if m.band != DUALBAND]
    dual = [c for m, c in zip(masters, covers) if m.band == DUALBAND]
    share = 0.0
    if colour and broad and dual:
        has_broad = np.logical_or.reduce(broad)
        has_dual = np.logical_or.reduce(dual)
        share = round(float((has_broad & ~has_dual).sum()) / size, 4)
    top, left, height, width = largest_rectangle(every)
    return {
        "noDualBandShare": share,
        "zones": zones,
        "fullCoverage": {"top": top, "left": left, "height": height, "width": width,
                         "shareOfFrame": round(height * width / size, 4),
                         "frameHeight": rows, "frameWidth": columns},
        # The frames the person may keep: the notches of the slider, from ``plan_framing``.
        "hasDualBand": bool(colour and any(m.band == DUALBAND for m in masters)),
        **plan_framing.framing_report(seconds, dual_seconds, (top, left, height, width), every),
    }
