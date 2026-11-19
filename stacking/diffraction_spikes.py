"""Diffraction spikes added to a finished image, the last of the settings of « Régler l'image ».

A reflector's spider (its vanes) diffracts the light of a star into thin rays: a straight vane spreads the light
perpendicular to itself, so a cross of 4 vanes gives 4 spikes (the two vanes of a line share a spike).  Along a spike
the intensity falls roughly as 1 / r**2 (the envelope of the sinc**2 of a thin slit), so on a linear image the length
above a fixed noise level grows as the square root of the star's flux.  The pattern scales with the wavelength: the
red part of a spike reaches further than the blue, which tints its far end.  This light is not in the person's data:
it is added, and the application says so.

``find_stars(layer, image)``  the stars, from the star layer (image minus StarNet2's starless image) and the render:
                              centre, flux, radius and colour of each, measured once at full size, saved to
                              ``stars.json`` beside the layer (``write_stars`` / ``read_stars``).
``spike_layer(...)``          the light of the spikes alone, for the stars at or above a brightness threshold, four
                              spikes (horizontal and vertical) for each, its length and intensity following the
                              star's flux.
``with_spikes(img, layer)``   the image with that light added in screen mode: never darker, no hard edge, and a
                              pixel the layer does not touch is not touched at all.

Two settings: ``strength`` (0..100, 0 = no spike at all, the image comes out unchanged to the bit) sets the length and
the intensity together; ``reach`` (0..100) sets which stars get spikes, from the very brightest only (0) to the
medium ones too (100).  The number of spikes and their angle are fixed.  Lengths are a share of the image's long side
and widths follow each star's measured radius, so the same values give the same picture on the 1080 px preview and on
the whole image.  The window draws the same layer with the same formulas (``app/src/lib/diffractionSpikes.ts``).
"""

from __future__ import annotations

import json
import os
import tempfile
from pathlib import Path

import numpy as np
from scipy import ndimage

LR, LG, LB = 0.2126, 0.7152, 0.0722

# The red end of a spike reaches this much further than the green, the blue as much less (650 / 530 / 460 nm).
DISPERSION = (1.12, 1.0, 0.88)


def lum(img: np.ndarray) -> np.ndarray:
    return LR * img[..., 0] + LG * img[..., 1] + LB * img[..., 2]


def _empty() -> dict[str, np.ndarray]:
    return {"x": np.zeros(0), "y": np.zeros(0), "flux": np.zeros(0), "radius": np.zeros(0),
            "colour": np.zeros((0, 3)), "saturated": np.zeros(0, dtype=bool)}


def find_stars(layer: np.ndarray, image: np.ndarray | None = None, floor: float = 0.06) -> dict[str, np.ndarray]:
    """The stars, measured once at full size from what the chain already has.

    ``layer``: the star layer (image minus StarNet2's starless image, float RGB 0..1).  A star is a basin of its luminance
    above ``floor`` (smoothed by 1 px) around one peak; its flux is the sum of the luminance over the basin.

    ``image``: the finished image.  StarNet2 often leaves the saturated core of the brightest stars in the starless
    image, so they are missing or faint in the layer: a compact patch of the image above 0.9 whose surroundings are
    darker (a galaxy core or a bright nebula is not) counts as a saturated star, its flux taken from the area of its
    saturated core (a larger white disc is a brighter star), its colour from the ring just outside the core.

    Fluxes are normalised to an image whose long side is 1000 px, so a star has the same flux on the preview and on
    the whole image.  The colour is a chroma with a luminance of 1 (a white core says nothing about the star's colour).
    """
    L = lum(layer)
    norm = (1000.0 / max(L.shape)) ** 2
    Ls = ndimage.gaussian_filter(L, 1.0)
    # One basin per peak (watershed), so that the stars of a crowded field or a close pair are measured each on
    # its own: a connected patch would add up all the stars it touches.
    from skimage.segmentation import watershed

    mask = Ls > floor
    peaks = mask & (Ls == ndimage.maximum_filter(Ls, size=5))
    markers, count = ndimage.label(peaks)
    labels = watershed(-Ls, markers, mask=mask) if count else markers
    out = _empty()
    if count:
        index = np.arange(1, count + 1)
        peak = ndimage.maximum(Ls, labels, index)
        # The core: the part of the patch above half its peak (the patch itself spreads with the halo).
        core = (labels > 0) & (Ls > 0.5 * np.concatenate([[np.inf], peak])[labels])
        area = ndimage.sum_labels(core.astype(np.float32), labels, index)
        centres = np.array(ndimage.maximum_position(Ls, labels, index), dtype=np.float64)
        unsat = (layer.max(-1) < 0.97) & (L > floor)
        w = np.where(unsat, L, 0.0)
        rgb = np.stack([ndimage.sum_labels(layer[..., k] * w, labels, index) for k in range(3)], -1)
        wsum = ndimage.sum_labels(w, labels, index)[:, None]
        out = {"x": centres[:, 1], "y": centres[:, 0], "flux": ndimage.sum_labels(L, labels, index) * norm,
               "radius": np.sqrt(area / np.pi), "colour": np.where(wsum > 1e-6, rgb / np.maximum(wsum, 1e-6), 1.0),
               "saturated": np.zeros(count, dtype=bool)}
    if image is not None:
        out = _with_saturated(out, image, norm)
    out["colour"] = out["colour"] / np.maximum(lum(out["colour"]), 1e-6)[:, None]
    return out


def _with_saturated(stars: dict[str, np.ndarray], image: np.ndarray, norm: float) -> dict[str, np.ndarray]:
    L = lum(image)
    labels, count = ndimage.label(L > 0.9)
    if count == 0:
        return stars
    index = np.arange(1, count + 1)
    area = ndimage.sum_labels(np.ones_like(L), labels, index)
    objects = ndimage.find_objects(labels)
    stars = {k: list(v) for k, v in stars.items()}
    for i, sl in zip(index, objects):
        a = area[i - 1]
        if a < 4:
            continue
        h, w = sl[0].stop - sl[0].start, sl[1].stop - sl[1].start
        r = np.sqrt(a / np.pi)
        if a < 0.55 * h * w or max(h, w) > 3 * min(h, w) or r * r * norm * np.pi > 400:
            continue  # not round, or too large to be a star (the core of a galaxy, a burnt nebula)
        cy, cx = ndimage.center_of_mass(np.ones_like(L[sl]), labels[sl], i)
        cy, cx = cy + sl[0].start, cx + sl[1].start
        y0, y1 = int(max(0, cy - 3 * r - 2)), int(min(L.shape[0], cy + 3 * r + 3))
        x0, x1 = int(max(0, cx - 3 * r - 2)), int(min(L.shape[1], cx + 3 * r + 3))
        gy, gx = np.mgrid[y0:y1, x0:x1]
        d = np.hypot(gy - cy, gx - cx)
        ring = (d > 1.3 * r + 1) & (d < 2.2 * r + 2)
        outer = (d > 2.5 * r + 2)
        if not ring.any() or not outer.any() or np.median(L[y0:y1, x0:x1][outer]) > 0.6:
            continue  # bright all around: not a star standing out of its sky
        colour = image[y0:y1, x0:x1][ring].mean(0)
        flux = 15.0 * (a * norm) ** 0.55
        xs, ys = np.asarray(stars["x"]), np.asarray(stars["y"])
        near = np.hypot(xs - cx, ys - cy) < max(10.0, 2.5 * r) if len(xs) else np.zeros(0, dtype=bool)
        if near.any():
            j = int(np.flatnonzero(near)[np.argmax(np.asarray(stars["flux"])[near])])
            stars["x"][j], stars["y"][j] = cx, cy
            stars["flux"][j] = max(stars["flux"][j], flux)
            stars["radius"][j] = max(stars["radius"][j], r)
            stars["colour"][j] = colour
            stars["saturated"][j] = True
            for other in np.flatnonzero(near):
                if other != j:
                    stars["flux"][other] = 0.0  # a second peak on the same white core: not another star
        else:
            for k, v in (("x", cx), ("y", cy), ("flux", flux), ("radius", r), ("colour", colour), ("saturated", True)):
                stars[k].append(v)
    out = {k: np.asarray(v) for k, v in stars.items()}
    keep = out["flux"] > 0
    return {k: v[keep] for k, v in out.items()}


# The flux (normalised, see ``find_stars``) of the faintest star that gets spikes: reach 0 -> only the very brightest,
# reach 100 -> medium stars too.  Calibrated on four real nights.
FLUX_AT_REACH_0 = 40.0
FLUX_AT_REACH_100 = 3.0
# At strength 100, a star of FLUX_AT_REACH_0 has spikes of this share of the image's long side (each arm).
LENGTH_AT_FULL = 0.10
INTENSITY_AT_FULL = 1.0
# The longest arm, whatever the star, as a share of the long side.
LONGEST = 0.22
# The spike's Gaussian width (sigma) as a share of the long side, for a star of FLUX_AT_REACH_0.
WIDTH = 0.00035
# How much of the star's colour a spike takes (the rest is white).
WHITE_CORE = 0.6
# The four spikes: horizontal and vertical, as unit vectors on the image's axes (x to the right, y down).
ARMS = ((1.0, 0.0), (0.0, 1.0), (-1.0, 0.0), (0.0, -1.0))
# The stars file: kept below the faintest flux that can get a spike.
FILE_VERSION = 1


# Below reach 0 the reach counts stars (see ``flux_threshold``), down to this reach: one star, the brightest.
REACH_MIN = -30.0


def flux_threshold(reach: float, fluxes: np.ndarray | None = None) -> float:
    """The flux of the faintest star that gets spikes at ``reach`` (-30..100).

    0..100: a fixed brightness (a setting saved with such a reach draws what it always drew).  Below 0: a number of
    stars, falling in equal ratios from the count at reach 0 to one star at the lowest reach, so the threshold is the
    flux of that many-th brightest star of ``fluxes``.
    """
    if reach >= 0.0 or fluxes is None or len(fluxes) == 0:
        t = min(max(reach, 0.0), 100.0) / 100.0
        return float(FLUX_AT_REACH_0 * (FLUX_AT_REACH_100 / FLUX_AT_REACH_0) ** t)
    ordered = np.sort(np.asarray(fluxes, dtype=np.float64))[::-1]
    start = max(1, int(np.count_nonzero(ordered >= FLUX_AT_REACH_0)))
    share = (max(REACH_MIN, reach) - REACH_MIN) / -REACH_MIN
    wanted = int(np.floor(start ** share + 0.5 + 1e-9))
    return float(ordered[min(len(ordered), max(1, wanted)) - 1])


def star_count(stars: dict[str, np.ndarray], reach: float) -> int:
    """How many stars the reach lets through (the number the window shows)."""
    return int(np.count_nonzero(stars["flux"] >= flux_threshold(reach, stars["flux"])))


def write_stars(stars: dict[str, np.ndarray], path: Path) -> None:
    """The stars that can ever get a spike, as JSON, written beside then renamed (a reader never sees half a file)."""
    keep = stars["flux"] >= FLUX_AT_REACH_100
    rows = [{"x": float(stars["x"][i]), "y": float(stars["y"][i]), "flux": float(stars["flux"][i]),
             "radius": float(stars["radius"][i]), "colour": [float(c) for c in stars["colour"][i]],
             "saturated": bool(stars["saturated"][i])} for i in np.flatnonzero(keep)]
    path.parent.mkdir(parents=True, exist_ok=True)
    fd, temp = tempfile.mkstemp(prefix=".stars-", suffix=".json", dir=path.parent)
    os.close(fd)
    try:
        Path(temp).write_text(json.dumps({"version": FILE_VERSION, "stars": rows}), encoding="utf-8")
        os.replace(temp, path)
    except BaseException:
        Path(temp).unlink(missing_ok=True)
        raise


def read_stars(path: Path) -> dict[str, np.ndarray]:
    rows = json.loads(path.read_text(encoding="utf-8"))["stars"]
    if not rows:
        return _empty()
    return {"x": np.array([r["x"] for r in rows], dtype=np.float64),
            "y": np.array([r["y"] for r in rows], dtype=np.float64),
            "flux": np.array([r["flux"] for r in rows], dtype=np.float64),
            "radius": np.array([r["radius"] for r in rows], dtype=np.float64),
            "colour": np.array([r["colour"] for r in rows], dtype=np.float64).reshape(-1, 3),
            "saturated": np.array([r["saturated"] for r in rows], dtype=bool)}


def spike_layer(shape: tuple[int, int], stars: dict[str, np.ndarray], strength: float, reach: float,
                scale: float = 1.0, star_factor: float = 1.0) -> np.ndarray:
    """The light of the spikes alone (float RGB, 0..1, ``shape`` = height, width).

    ``stars`` measured at full size; ``scale`` = this layer's long side / the full image's (the preview: < 1), the
    stars' positions and radii are scaled by it.  ``star_factor``: the « Étoiles » setting as a factor (1 + stars /
    100), so taking the stars away takes their spikes too.
    """
    h, w = shape
    out = np.zeros((h, w, 3), dtype=np.float32)
    if strength <= 0 or star_factor <= 0 or len(stars["flux"]) == 0:
        return out
    s = min(strength, 100.0) / 100.0
    long_side = float(max(h, w))
    keep = stars["flux"] >= flux_threshold(reach, stars["flux"])
    order = np.argsort(stars["flux"][keep], kind="stable")  # faint first (screen mode is commutative anyway)
    # A pixel's centre is at its index: the full image's pixel x lands on (x + 0.5) * scale - 0.5.
    xs = (stars["x"][keep][order] + 0.5) * scale - 0.5
    ys = (stars["y"][keep][order] + 0.5) * scale - 0.5
    fl, rad = stars["flux"][keep][order], stars["radius"][keep][order] * scale
    col = stars["colour"][keep][order]
    for x0, y0, f, r, c in zip(xs, ys, fl, rad, col):
        rel = f / FLUX_AT_REACH_0
        # Length ~ sqrt(flux) (1/r^2 envelope above a fixed level), times the strength.
        length = min(LENGTH_AT_FULL * s ** 0.7 * np.sqrt(rel), LONGEST * s ** 0.7) * long_side
        r0 = max(r, 1.0)  # inside the core the star itself is there: the spike starts soft
        if length < 3 * r0:
            continue
        # Intensity near the core: grows with flux but saturates (a stretched image compresses bright stars).
        i0 = INTENSITY_AT_FULL * s ** 0.35 * star_factor * (1 - np.exp(-1.2 * np.sqrt(rel)))
        # Across the spike: a share of the image (the star cores of one image are about the same size), a little wider
        # for a bright star (its wings), never under half a pixel.
        sigma = max(0.5, WIDTH * long_side * (1 + 0.35 * np.log2(1 + rel)))
        tint = np.clip(WHITE_CORE + (1 - WHITE_CORE) * c, 0, None)
        reach_px = length * DISPERSION[0]
        half = 6.6 * sigma
        for ex, ey in ARMS:
            # The box of the arm: its rectangle, ``reach_px`` long and ``2 * half`` wide.
            cx = (x0, x0 + ex * reach_px)
            cy = (y0, y0 + ey * reach_px)
            x_lo = int(max(0, np.floor(min(cx) - half * abs(ey))))
            x_hi = int(min(w, np.ceil(max(cx) + half * abs(ey)) + 1))
            y_lo = int(max(0, np.floor(min(cy) - half * abs(ex))))
            y_hi = int(min(h, np.ceil(max(cy) + half * abs(ex)) + 1))
            if x_lo >= x_hi or y_lo >= y_hi:
                continue
            gy, gx = np.mgrid[y_lo:y_hi, x_lo:x_hi]
            dx, dy = gx - x0, gy - y0
            along = dx * ex + dy * ey
            across = -dx * ey + dy * ex
            band = (np.abs(across) < half) & (along > 0)
            if not band.any():
                continue
            a = along[band]
            # The base of a spike is wider (it opens out of the star's halo), then the width settles.
            wide = sigma * (1 + 1.2 * np.exp(-a / (2.5 * r0)))
            shape_of = np.exp(-0.5 * (across[band] / wide) ** 2) * (1 - np.exp(-((a / r0) ** 2)))
            for k in range(3):
                lk = length * DISPERSION[k]
                t = np.clip(a / lk, 0, 1)
                # 1/(1 + a/a0) near the star, brought to zero smoothly at the arm's end (no hard end).
                prof = (1.0 / (1.0 + a / (0.07 * lk))) * (1 - t * t) ** 2
                v = i0 * tint[k] * prof * shape_of
                sub = out[y_lo:y_hi, x_lo:x_hi, k]
                sub[band] = 1 - (1 - sub[band]) * (1 - v)
    return np.clip(out, 0, 1)


def with_spikes(img: np.ndarray, spikes: np.ndarray) -> np.ndarray:
    """Screen mode: the spikes' light added, never darker, a white pixel stays white.  A pixel the layer does not
    touch keeps its value exactly (``1 - (1 - x)`` is not always ``x`` in floating point), and no spike at all
    returns ``img`` itself."""
    touched = spikes > 0
    if not touched.any():
        return img
    return np.where(touched, 1 - (1 - img) * (1 - spikes), img).astype(img.dtype, copy=False)
