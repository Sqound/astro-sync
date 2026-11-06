#!/usr/bin/env python3
"""The finish of a rendered image: the contrast curve, softer stars, stronger colours.

A *finish* is what the person chooses beside the render itself, in this order
(each step only on the display image the render wrote, never on a linear
master and never on "No processing"):

1. the contrast of presentation (``renders.presentation_contrast``, the S-curve
   on the luminance);
2. **softened stars** (``soften_stars``): the brightest stars are pulled
   towards the same image without them, so a dense star field stops
   competing with the nebula, above all in a small thumbnail;
3. **colour intensity** (``saturate``): the colours of the image are
   multiplied, its luminance untouched.

The default of each step changes nothing: an image that asks for no finish is
not rewritten.  The stacking script and the plan image call ``apply`` for the
twin they write beside each render.
"""

from __future__ import annotations

import argparse
from dataclasses import dataclass

import numpy as np
from scipy.ndimage import gaussian_filter, grey_dilation, grey_erosion, grey_opening

import renders

# The three settings are continuous.  A setting is written as a *token*: ``none``
# (nothing) or a number; the contrast also has its two historical words.
#
# - contrast: the gain of the S-curve, 0 (none) to ``CONTRAST_MAX``; ``presentation``
#   is 2.0 and ``strong`` 3.6, the two the window's detents stand on;
# - colour intensity: the percent the colours are strengthened by, 0 to
#   ``COLOUR_MAX`` (the chroma is multiplied by 1 + percent / 100); 20 and 40 are the
#   window's detents;
# - softened stars: the percent of each bright star taken out, 0 to ``STARS_MAX``;
#   50 and 85 are the window's detents.
#
# A number equal to a word's value is written as that word, so the files and the
# records of the values the window always offered are named as they were.
CONTRAST_MAX = 6.0
COLOUR_MAX = 100.0
STARS_MAX = 100.0
CONTRAST_WORDS = {"presentation": 2.0, "strong": 3.6}
# Words the first version of the two new settings used: still read, as their numbers.
COLOUR_WORDS = {"enhanced": 20.0, "strong": 40.0}
STARS_WORDS = {"light": 50.0, "strong": 85.0}
CONTRAST_CHOICES = ("none", *CONTRAST_WORDS)
COLOUR_CHOICES = ("none", *COLOUR_WORDS)
STARS_CHOICES = ("none", *STARS_WORDS)


def _number(value: object, words: dict[str, float], maximum: float, what: str) -> float:
    """A setting's value from a token, a word or a number; ``ValueError`` outside 0..maximum."""
    if value is None or value == "" or value == "none":
        return 0.0
    if isinstance(value, bool):
        raise ValueError(f"{what} '{value}' is not a number")
    if isinstance(value, str):
        text = value.strip().lower()
        if text in words:
            return words[text]
        try:
            number = float(text)
        except ValueError:
            names = ", ".join(("none", *words))
            raise ValueError(f"{what} '{value}' is not a number between 0 and {maximum:g}, or {names}") from None
    else:
        number = float(value)  # type: ignore[arg-type]
    if not 0 <= number <= maximum or number != number:
        raise ValueError(f"{what} '{value}' is not between 0 and {maximum:g}")
    return number


def _token(number: float, words: dict[str, float]) -> str:
    if number <= 0:
        return "none"
    for word, value in words.items():
        if abs(number - value) < 1e-9:
            return word
    return f"{number:g}"


def contrast_token(value: object) -> str:
    """``none``, ``presentation``, ``strong`` or a gain such as ``2.5``, from any way of writing one."""
    return _token(_number(value, CONTRAST_WORDS, CONTRAST_MAX, "the contrast"), CONTRAST_WORDS)


def colour_token(value: object) -> str:
    """``none`` or the percent the colours are strengthened by, as ``30``."""
    return _token(_number(value, COLOUR_WORDS, COLOUR_MAX, "the colour intensity"), {})


def stars_token(value: object) -> str:
    """``none`` or the percent of each bright star taken out, as ``70``."""
    return _token(_number(value, STARS_WORDS, STARS_MAX, "the softened stars"), {})


def contrast_gain(token: str) -> float:
    """The gain of the S-curve of a contrast token; 0 for ``none``."""
    return _number(token, CONTRAST_WORDS, CONTRAST_MAX, "the contrast")


def colour_percent(token: str) -> float:
    return _number(token, COLOUR_WORDS, COLOUR_MAX, "the colour intensity")


def stars_percent(token: str) -> float:
    return _number(token, STARS_WORDS, STARS_MAX, "the softened stars")


def _argument(convert):
    """An ``argparse`` type from one of the token functions: the token, or a message naming the bound."""
    def parse(text: str) -> str:
        try:
            return convert(text)
        except ValueError as error:
            raise argparse.ArgumentTypeError(str(error)) from None
    return parse


contrast_arg = _argument(contrast_token)
colour_arg = _argument(colour_token)
stars_arg = _argument(stars_token)

# The sizes below are in pixels of an image 3000 px on its short side (a
# 2.5 to 4 megapixel plan image); they follow the image's size.
REFERENCE_SIDE = 3000
OPENING_RADIUS = 7  # what is narrower than this disk is a star, not nebula
TOP_HAT_THRESHOLD = 0.08  # the part of a star above its surroundings that counts, 0..1
GROWTH_RADIUS = 4  # the mask grows over the star's halo
EROSION_RADIUS = 4  # the image "without stars" is the eroded one
SMOOTHING = 2.0  # the Gaussian blur of the mask and of the eroded image


def _scaled(size: float, shape: tuple[int, ...]) -> float:
    return size * min(shape[:2]) / REFERENCE_SIDE


def _disk(radius: float) -> np.ndarray:
    """A round footprint of the given radius (at least one pixel)."""
    r = max(1, int(round(radius)))
    y, x = np.ogrid[-r : r + 1, -r : r + 1]
    return (x * x + y * y) <= r * r + r * 0.5


def star_mask(rgb: np.ndarray) -> np.ndarray:
    """The softened mask of the stars of a display image, 0..1 (before any amount).

    The top-hat of the luminance (the luminance minus its opening by a disk)
    keeps what is smaller than the disk and brighter than its surroundings:
    stars, not the nebula, which a plain threshold of the luminance would
    soften too.  It is thresholded, grown over the halos and blurred so the
    mask has no edge.
    """
    level = renders.lum(rgb).astype(np.float32)
    opened = grey_opening(level, footprint=_disk(_scaled(OPENING_RADIUS, rgb.shape)))
    hat = (level - opened) > TOP_HAT_THRESHOLD
    grown = grey_dilation(hat.astype(np.float32), footprint=_disk(_scaled(GROWTH_RADIUS, rgb.shape)))
    return np.clip(gaussian_filter(grown, _scaled(SMOOTHING, rgb.shape)), 0, 1)


def soften_stars(rgb: np.ndarray, amount: float) -> np.ndarray:
    """The image with its stars pulled ``amount`` (0..1) of the way to the same image without them.

    The image "without stars" is the eroded one (a StarNet separation is not
    needed, and none is available for a pipeline of the person's own).  Every
    channel is eroded the same way, so a star becomes the colour around it,
    never a dark hole, and where the mask is nil the image is not touched.
    """
    x = np.clip(np.nan_to_num(np.asarray(rgb, np.float32)), 0, 1)
    if amount <= 0:
        return x
    footprint = _disk(_scaled(EROSION_RADIUS, x.shape))
    sigma = _scaled(SMOOTHING, x.shape)
    bare = np.stack(
        [gaussian_filter(grey_erosion(x[..., c], footprint=footprint), sigma) for c in range(x.shape[-1])],
        axis=-1,
    )
    # An eroded pixel is never brighter than the original, so the blend only removes light.
    weight = (star_mask(x) * amount)[..., None]
    return np.clip(x * (1 - weight) + np.minimum(bare, x) * weight, 0, 1).astype(np.float32)


def saturate(rgb: np.ndarray, gain: float) -> np.ndarray:
    """The colours multiplied by ``gain``, the luminance kept.

    Each channel moves away from the pixel's luminance by ``gain``; where that
    would leave 0..1 the pixel's chroma is reduced just enough to fit (the
    hue and the luminance stay), so a bright star is never clipped into white
    or a colour into another.
    """
    x = np.clip(np.nan_to_num(np.asarray(rgb, np.float32)), 0, 1)
    if x.ndim == 2 or gain == 1:
        return x
    level = renders.lum(x)[..., None]
    chroma = (x - level) * gain
    with np.errstate(divide="ignore", invalid="ignore"):
        up = np.where(chroma > 1e-9, (1 - level) / chroma, np.inf)
        down = np.where(chroma < -1e-9, -level / chroma, np.inf)
    fit = np.minimum(1.0, np.minimum(up, down).min(axis=-1, keepdims=True))
    return np.clip(level + chroma * fit, 0, 1).astype(np.float32)


@dataclass(frozen=True)
class Finish:
    """What is applied to a render to make its twin: contrast, softened stars, colour intensity (tokens)."""

    contrast: str = "none"
    colour: str = "none"
    stars: str = "none"

    def is_plain(self) -> bool:
        return self.contrast == "none" and self.colour == "none" and self.stars == "none"

    def suffix(self) -> str:
        """``strong``, ``contrast-2.5_colour-30_stars-70``...; empty for a plain finish.

        The two words of the contrast keep the names they always had, so the files of a run that
        asks for nothing else are named as before; a number is named by its setting.
        """
        parts = []
        if self.contrast != "none":
            parts.append(self.contrast if self.contrast in CONTRAST_WORDS else f"contrast-{self.contrast}")
        if self.colour != "none":
            parts.append(f"colour-{self.colour}")
        if self.stars != "none":
            parts.append(f"stars-{self.stars}")
        return "_".join(parts)

    def apply(self, image: np.ndarray) -> np.ndarray:
        """The twin of a display image (float, 0..1, colours last)."""
        out = image
        if self.contrast != "none":
            out = renders.presentation_contrast(out, contrast_gain(self.contrast))
        if self.stars != "none":
            out = soften_stars(out, stars_percent(self.stars) / 100)
        if self.colour != "none":
            out = saturate(out, 1 + colour_percent(self.colour) / 100)
        return out


def checked(contrast: object, colour: object, stars: object) -> Finish:
    """A finish from three settings (tokens, words or numbers); ``ValueError`` for one out of its bounds."""
    return Finish(contrast_token(contrast), colour_token(colour), stars_token(stars))


def pipeline_finish(text: str) -> tuple[str, Finish]:
    """``ID=CONTRAST,COLOUR,STARS`` (``hoo=strong,20,50``): the finish of one pipeline, its id first.

    Each of the three values is written as for ``--contrast``, ``--colour`` and ``--soft-stars``;
    ``none`` for all three asks for the render alone.  ``ValueError`` for a malformed text.
    """
    ident, sep, values = text.partition("=")
    parts = values.split(",")
    if not sep or not ident.strip() or len(parts) != 3:
        raise ValueError(f"'{text}' is not ID=CONTRAST,COLOUR,STARS (hoo=strong,20,50)")
    return ident.strip(), checked(*(part.strip() for part in parts))


pipeline_finish_arg = _argument(pipeline_finish)


def fields(finish: Finish) -> dict[str, str]:
    """The finish as the provenance and the events write it: ``contrast``, ``colour``, ``softStars`` (tokens)."""
    return {"contrast": finish.contrast, "colour": finish.colour, "softStars": finish.stars}
