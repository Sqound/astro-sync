"""Several nights of a colour sensor into one linear image, the dual-band master as the reference.

This is the recipe, written step by step so that the same masters always give the same image, to the
bit.  It is used by a plan of a colour sensor that has at least one dual-band master
(``plan_stack.py``); a plan without a dual-band master keeps ``plan_combine.py``.

**The dual-band nights** (:func:`merge_band`) become one master:

1. a robust low-order background model is taken away from every master (``renders.remove_gradient``,
   degree 2, tiles of 160 px, black point at the 0.25th percentile, nothing below zero);
2. the longest master of the band is the reference; every other one is registered on it from its stars
   (``astroalign``: triangles of the brightest 100 stars, a similarity transform, bicubic resampling);
3. each registered master is brought to the reference's scale channel by channel: the ratio of the 99th
   percentiles of the two images over the field both cover;
4. where a master covers the reference, the pixel is the mean weighted by the integration time; elsewhere
   the reference stays as it is.

**The two bands** (:func:`fuse_band`; :func:`fuse` for one master of each) become the image:

1. the background model is taken away from the dual-band master (the merged one is not exempt) and from
   every broadband master;
2. every broadband master is registered directly on the dual-band one (the brightest 80 stars): the frame
   of the image is the dual-band master's, whatever frame the broadband nights were taken on;
3. the broadband masters are brought to the scale of the longest of them (the ratio of the 99th
   percentiles over the field both cover) and averaged, weighted as in a band, wherever at least one of
   them covers: what one broadband night misses (a camera turned between nights) another fills;
4. the broadband band is brought to the dual-band one's scale (the ratio of the 99th percentiles of
   each channel over the field it covers: a normalisation, not a change of palette);
5. ``image = dual-band + 0.18 × broadband``: the dual-band master keeps the structure of the emission
   nebulae, and the broadband one adds a measured share of continuum and colour, never its sky gradient;
   what no broadband master covers is left black.

With a single broadband master, step 3 is empty and :func:`fuse_band` gives :func:`fuse`'s image to the bit.

The 99th percentiles, the order of the operations and the floating-point types are fixed: on the same
masters the result is the same, value for value (``test_dualband_reference.py`` keeps a reference run).

Arrays are ``(3, height, width)`` float32, the layout of a colour FITS; the steps work on
``(height, width, 3)``.
"""

from __future__ import annotations

import math
from dataclasses import dataclass, field

import numpy as np

from renders import remove_gradient

#: The share of the broadband master added to the dual-band one, after the scale is matched.
BROADBAND_WEIGHT = 0.18

#: The percentile both images are compared at to match their scales.
SCALE_PERCENTILE = 99.0

#: The stars a registration reads: the brightest of each image.
FUSION_CONTROL_POINTS = 80
MERGE_CONTROL_POINTS = 100

#: A star stands this many times the background noise above it.
DETECTION_SIGMA = 4


@dataclass
class Registered:
    """A master brought onto the frame of another one."""

    image: np.ndarray  # (height, width, 3), on the reference's frame; float64, as the resampling leaves it
    valid: np.ndarray  # the pixels the master covers on that frame
    stars: int  # the stars matched between the two
    rotation_degrees: float  # the rotation of the transform
    transform: object  # the similarity transform (``skimage``), source -> reference


@dataclass
class BandMerge:
    """What :func:`merge_band` did, for the record of the plan."""

    reference: int  # index, in the list given, of the master the others were registered on
    weights: list[float]
    merged: list[dict[str, object]] = field(default_factory=list)  # one entry per master registered
    left_out: list[int] = field(default_factory=list)  # the masters no transform was found for
    # The pixels of the reference's frame each master covers (the reference: all of them), by index.
    covers: dict[int, np.ndarray] = field(default_factory=dict)


class RegistrationError(ValueError):
    """Two masters have no stars in common that a transform can be found from."""


def to_pixels(image: np.ndarray) -> np.ndarray:
    """``(3, h, w)`` -> ``(h, w, 3)`` float32."""
    return np.moveaxis(np.asarray(image, np.float32), 0, -1)


def to_planes(image: np.ndarray) -> np.ndarray:
    """``(h, w, 3)`` -> ``(3, h, w)`` float32."""
    return np.moveaxis(image, -1, 0).astype(np.float32)


def luminance(rgb: np.ndarray) -> np.ndarray:
    return 0.2126 * rgb[..., 0] + 0.7152 * rgb[..., 1] + 0.0722 * rgb[..., 2]


def alignment_view(data: np.ndarray) -> np.ndarray:
    """What the stars are found on: the luminance, stretched between its 20th and 99.8th percentiles."""
    signal = luminance(data)
    low, high = np.percentile(signal, (20, 99.8))
    return np.sqrt(np.clip((signal - low) / max(high - low, 1e-12), 0, 1))


def register(source: np.ndarray, reference: np.ndarray, control_points: int,
             source_cover: np.ndarray | None = None) -> Registered:
    """``source`` resampled onto ``reference``'s frame (both ``(h, w, 3)``, background already removed).

    ``source_cover``: the pixels of the source that hold data (``data_of``); a source that covers its whole
    frame needs none, and then the valid pixels are exactly its footprint.
    """
    import astroalign

    try:
        transform, (matched, _) = astroalign.find_transform(
            alignment_view(source), alignment_view(reference),
            max_control_points=control_points, detection_sigma=DETECTION_SIGMA,
        )
    except (astroalign.MaxIterError, ValueError) as error:
        raise RegistrationError(str(error)) from error
    image, footprint = astroalign.apply_transform(transform, source, reference, propagate_mask=True)
    valid = ~footprint
    if source_cover is not None and not source_cover.all():
        # A master of a turning field keeps empty (zero) borders inside its frame: they are no data either.
        valid &= carried(source_cover, transform, valid.shape)
    return Registered(image=image, valid=valid, stars=len(matched),
                      rotation_degrees=math.degrees(float(transform.rotation)), transform=transform)


def data_of(image: np.ndarray) -> np.ndarray:
    """The pixels of a ``(h, w, 3)`` master that hold data.

    Siril leaves exact zeros where none of a night's frames reached (a field that turned): those areas
    touch the edge of the frame.  A zero pixel inside the data (rare, but possible) is data.
    """
    empty = ~(image != 0).any(axis=-1)
    if not empty.any():
        return ~empty
    from scipy.ndimage import label

    labels, _ = label(empty)
    edge = np.unique(np.concatenate([labels[0], labels[-1], labels[:, 0], labels[:, -1]]))
    return ~np.isin(labels, edge[edge > 0])


def carried(mask: np.ndarray, transform: object, shape: tuple[int, int]) -> np.ndarray:
    """A mask of the source's frame carried onto the reference's by ``transform`` (as the footprint is)."""
    from skimage.transform import warp

    moved = warp(mask.astype(np.float32), inverse_map=transform.inverse, output_shape=shape, cval=0.0)
    return moved > 0.6


def scales_of(reference: np.ndarray, registered: np.ndarray, valid: np.ndarray) -> list[float]:
    """Per channel, the ratio of the 99th percentiles of ``reference`` and ``registered`` over ``valid``."""
    scales = []
    for channel in range(3):
        ref = np.percentile(reference[..., channel][valid], SCALE_PERCENTILE)
        src = np.percentile(registered[..., channel][valid], SCALE_PERCENTILE)
        scales.append(ref / max(src, 1e-12))
    return scales


def weights_of(seconds: list[float], frames: list[int] | None) -> list[float]:
    """The weight of each master in the mean of a band: its frame count when every master has poses of the
    same length, its integration time otherwise (the same ratio).
    """
    if frames and len(frames) == len(seconds) and all(n > 0 for n in frames):
        exposures = {round(s / n, 6) for s, n in zip(seconds, frames)}
        if len(exposures) == 1:
            return [int(n) for n in frames]
    return [float(s) for s in seconds]


def longest(seconds: list[float]) -> int:
    """The index of the longest integration; of two equal ones, the first."""
    return max(range(len(seconds)), key=lambda i: (seconds[i], -i))


def merge_band(images: list[np.ndarray], seconds: list[float],
               frames: list[int] | None = None) -> tuple[np.ndarray, BandMerge]:
    """The masters of one band (``(3, h, w)`` each, on their own frames) as one ``(h, w, 3)`` master.

    Returns the master (on the frame of the longest one) and what was done.  One master alone is
    returned as it is: the recipe merges nothing then, and :func:`fuse` takes its background away.  A
    master no transform can be found for is left out of the mean and named (``left_out``).
    """
    if not images or len(images) != len(seconds):
        raise ValueError("one integration time per master is needed")
    weights = weights_of(seconds, frames)
    best = longest(seconds)
    record = BandMerge(reference=best, weights=weights)
    if len(images) == 1:
        image = to_pixels(images[0])
        record.covers[0] = np.ones(image.shape[:2], bool)
        return image, record
    reference = remove_gradient(to_pixels(images[best]))
    record.covers[best] = np.ones(reference.shape[:2], bool)
    registered: list[tuple[int, Registered]] = []
    for i in range(len(images)):
        if i == best:
            continue
        try:
            source = to_pixels(images[i])
            moved = register(remove_gradient(source), reference, MERGE_CONTROL_POINTS, data_of(source))
        except RegistrationError:
            record.left_out.append(i)
            continue
        scales = scales_of(reference, moved.image, moved.valid)
        moved.image *= np.asarray(scales, dtype=np.float32)[None, None, :]
        registered.append((i, moved))
        record.covers[i] = moved.valid
        record.merged.append({"index": i, "to": best, "stars": moved.stars,
                              "scales": [round(float(s), 6) for s in scales],
                              "coverage": round(float(moved.valid.mean()), 4),
                              "rotationDegrees": round(moved.rotation_degrees, 3)})
    combined = reference.copy()
    if len(registered) == 1:
        # Two masters: the recipe's own expression, so that its result is met to the bit.
        i, moved = registered[0]
        valid = moved.valid
        combined[valid] = (weights[best] * reference[valid] + weights[i] * moved.image[valid]) / (weights[best] + weights[i])
    elif registered:
        # More: each pixel is the weighted mean of the masters that cover it, the reference always among them.
        numerator = weights[best] * reference.astype(np.float64)
        denominator = np.full(reference.shape[:2], float(weights[best]))
        for i, moved in registered:
            numerator[moved.valid] += weights[i] * moved.image[moved.valid]
            denominator[moved.valid] += weights[i]
        covered = denominator > weights[best]
        combined[covered] = numerator[covered] / denominator[covered][:, None]
    return combined, record


@dataclass
class Fusion:
    """What :func:`fuse` measured, for the record of the plan."""

    weight: float
    scales: list[float]
    stars: int
    coverage: float
    rotation_degrees: float
    transform: object  # broadband -> dual-band


def fuse(dual: np.ndarray, broad: np.ndarray, weight: float = BROADBAND_WEIGHT) -> tuple[np.ndarray, np.ndarray, Fusion]:
    """The image of a dual-band master and a broadband one (both ``(h, w, 3)``, on their own frames).

    Returns the image (``(h, w, 3)`` float64, on the dual-band master's frame), the pixels the broadband
    master covers (the others are black) and what was measured.  :class:`RegistrationError` when the
    two masters have no stars in common.
    """
    cover = data_of(broad)
    dual = remove_gradient(dual)
    broad = remove_gradient(broad)
    moved = register(broad, dual, FUSION_CONTROL_POINTS, cover)
    valid = moved.valid
    scales = scales_of(dual, moved.image, valid)
    scaled_broad = moved.image * np.asarray(scales, dtype=np.float32)[None, None, :]
    hybrid = dual + weight * scaled_broad
    hybrid[~valid] = 0
    return hybrid, valid, Fusion(weight=weight, scales=[round(float(s), 6) for s in scales], stars=moved.stars,
                                 coverage=round(float(valid.mean()), 4),
                                 rotation_degrees=round(moved.rotation_degrees, 3), transform=moved.transform)


def fuse_band(dual: np.ndarray, broads: list[np.ndarray], seconds: list[float], frames: list[int] | None = None,
              weight: float = BROADBAND_WEIGHT) -> tuple[np.ndarray, np.ndarray, Fusion, BandMerge]:
    """The image of a dual-band master (``(h, w, 3)``) and the broadband masters (``(3, h, w)`` each, on their
    own frames): each broadband master registered directly on the dual-band frame.

    Merging the broadband masters on the frame of the longest of them first (:func:`merge_band`) would
    crop the image to that night's footprint: a night taken with the camera turned leaves its corners
    black although the other nights cover them.  Here the broadband band covers the union of its masters.

    Returns the image (``(h, w, 3)`` float64, on the dual-band frame), the pixels at least one broadband
    master covers (the others are black), what the fusion measured (the stars, rotation and transform
    are the longest broadband master's; the scales and the coverage, the band's) and what the band did
    (``covers``: on the dual-band frame; ``merged``: each other master levelled on the longest).  A master
    no transform can be found for is left out (``left_out``); :class:`RegistrationError` when none is left.
    With one broadband master the image is :func:`fuse`'s, to the bit.
    """
    if not broads or len(broads) != len(seconds):
        raise ValueError("one integration time per master is needed")
    weights = weights_of(seconds, frames)
    dual = remove_gradient(dual)
    moved: dict[int, Registered] = {}
    left_out: list[int] = []
    for i, planes in enumerate(broads):
        broad = to_pixels(planes)
        cover = data_of(broad)
        try:
            moved[i] = register(remove_gradient(broad), dual, FUSION_CONTROL_POINTS, cover)
        except RegistrationError:
            left_out.append(i)
    if not moved:
        raise RegistrationError("no broadband master could be registered on the dual-band one")
    # The level of the band is the longest master's: the longest of those registered.
    best = max(moved, key=lambda i: (seconds[i], -i))
    record = BandMerge(reference=best, weights=weights, left_out=left_out)
    reference = moved[best]
    record.covers[best] = reference.valid
    if len(moved) == 1:
        band = reference.image
        valid = reference.valid
    else:
        numerator = weights[best] * np.where(reference.valid[..., None], reference.image, 0.0)
        denominator = np.where(reference.valid, float(weights[best]), 0.0)
        for i, other in moved.items():
            if i == best:
                continue
            common = other.valid & reference.valid
            if common.sum() < 1000:
                # Too little field in common to read a level from: the master is left out.
                record.left_out.append(i)
                continue
            scales = scales_of(reference.image, other.image, common)
            image = other.image * np.asarray(scales, dtype=np.float32)[None, None, :]
            numerator[other.valid] += weights[i] * image[other.valid]
            denominator[other.valid] += weights[i]
            record.covers[i] = other.valid
            record.merged.append({"index": i, "to": best, "stars": other.stars,
                                  "scales": [round(float(s), 6) for s in scales],
                                  "coverage": round(float(other.valid.mean()), 4),
                                  "rotationDegrees": round(other.rotation_degrees - reference.rotation_degrees, 3)})
        valid = denominator > 0
        band = np.zeros(numerator.shape, np.float64)
        band[valid] = numerator[valid] / denominator[valid][:, None]
    record.left_out.sort()
    scales = scales_of(dual, band, valid)
    scaled_broad = band * np.asarray(scales, dtype=np.float32)[None, None, :]
    hybrid = dual + weight * scaled_broad
    hybrid[~valid] = 0
    return hybrid, valid, Fusion(weight=weight, scales=[round(float(s), 6) for s in scales], stars=reference.stars,
                                 coverage=round(float(valid.mean()), 4),
                                 rotation_degrees=round(reference.rotation_degrees, 3),
                                 transform=reference.transform), record
