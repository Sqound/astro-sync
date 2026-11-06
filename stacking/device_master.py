#!/usr/bin/env python3
"""The image a device already stacked, as the master of the renders.

A Seestar writes ``Stacked_<n>_<target>_<exposure>_<filter>_<date>.fit`` beside
its sub-frames: the stack of the night it made itself, a linear colour cube
(3 x H x W, 16 bits, the unsigned offset), its field solution in the header.
Astro Sync can take it as the master of a target instead of stacking the poses
again (``astro_sync_stack.py --from-master``): the renders, the contrast and
"No processing" then start from this image as they start from the master of
a stacking of its own.

What this module does is the one thing the renders need and the device's file
does not give: **the scale of Siril's master**.  The stacking's master is 32-bit
floating point between 0 and 1 (``-output_norm``); the device's image is a
16-bit integer cube.  The conversion divides by 65535, keeps the three planes in
the order the file has them, and writes a small FITS in the work folder
(``masters/stack_linear.fit``, the name the stacking's master has) whose header
keeps what the renders and the provenance read: ``STACKCNT``, ``EXPTIME``,
``TOTALEXP``, ``FILTER``, ``GAIN``, ``OBJECT``.

**Orientation**: nothing is flipped.  A Seestar's ``Stacked_*.fit`` and the master
Astro Sync stacks from the same night's ``Light_*.fit`` have the same
handedness: their stars superpose by a rotation and a translation and not by a
mirror (measured on the night of 10/10/2026, M 45: 21 stars of 70 matched
without a mirror, 3 with one, the level of chance).  The rows are therefore kept in
the order ``astropy`` reads them, as ``renders.py`` and ``linear_export.py``
write them.  A device whose image were mirrored would be mirrored here, once,
by ``convert`` (``mirror=True``), not in the renders.

**What is refused**, with the codes of ``star_quality``'s refusals: an image that
is not a 3-plane cube (``not_an_image``), a file that cannot be read
(``unreadable``), and a stretched image (``stretched``: the median of its
luminance above 2 % of its range, the guard of "Measure a master…") -- the
width of a star on an image made to be looked at means nothing, and a render
made from it would stretch twice.

Needs ``numpy``, ``scipy`` and ``astropy`` (the stacking environment).
"""

from __future__ import annotations

from pathlib import Path

import numpy as np

import star_quality

# What the converted master keeps of the device image's header.
KEPT_KEYS = ("STACKCNT", "EXPTIME", "TOTALEXP", "FILTER", "GAIN", "OBJECT", "XBINNING", "YBINNING", "CREATOR")


class DeviceImageError(Exception):
    """An image that cannot be the master of a render; ``code`` is a word of ``star_quality.MasterError``."""

    def __init__(self, code: str, message: str) -> None:
        super().__init__(message)
        self.code = code


def read_cube(path: Path) -> tuple[np.ndarray, "object"]:
    """The image of ``path`` as 3 x H x W floats between 0 and 1, and its header; or a :class:`DeviceImageError`."""
    from astropy.io import fits

    try:
        with fits.open(path, memmap=False) as hdus:
            data = hdus[0].data
            header = hdus[0].header.copy()
    except (OSError, ValueError, TypeError) as error:
        raise DeviceImageError("unreadable", f"the FITS file could not be read: {error}") from error
    if data is None:
        raise DeviceImageError("not_an_image", "the FITS file holds no image")
    data = np.asarray(data)
    if data.ndim != 3 or data.shape[0] != 3:
        raise DeviceImageError("not_an_image", f"expected a 3 x H x W colour cube, got {data.shape}")
    if data.dtype == np.uint8:
        raise DeviceImageError("stretched", "an 8-bit image is not a linear master")
    if np.issubdtype(data.dtype, np.integer):
        data = data.astype(np.float32) / float(np.iinfo(data.dtype).max)
    return np.nan_to_num(data.astype(np.float32)), header


def convert(source: Path, destination: Path, mirror: bool = False) -> dict[str, object]:
    """Writes the master the renders read at ``destination`` and answers what the provenance records.

    ``mirror`` flips the rows: not used for the Seestar (see the module's
    documentation), kept for a device that would need it.  Raises a
    :class:`DeviceImageError`.
    """
    from astropy.io import fits

    cube, header = read_cube(source)
    level = star_quality.luminance(cube)
    if float(level.max()) <= float(level.min()):
        raise DeviceImageError("not_an_image", "the image is flat: it holds no star")
    reason = star_quality.stretch_reason(level)
    if reason is not None:
        raise DeviceImageError("stretched", reason)
    if mirror:
        cube = cube[:, ::-1, :]
    out = fits.PrimaryHDU(np.ascontiguousarray(cube))
    for key in KEPT_KEYS:
        if key in header:
            out.header[key] = header[key]
    out.header["COMMENT"] = "Astro Sync: the image of the device, scaled to 0..1 for the renders"
    destination.parent.mkdir(parents=True, exist_ok=True)
    out.writeto(destination, overwrite=True)
    return {
        "file": source.name,
        "device": str(header.get("CREATOR", "")).replace("ZWO", "").strip() or None,
        "stackedFrames": int(header.get("STACKCNT", 0) or 0),
        "exposureSeconds": _number(header.get("EXPTIME")),
        "totalSeconds": _number(header.get("TOTALEXP")),
        "filter": (str(header["FILTER"]).strip() or None) if "FILTER" in header else None,
        "gain": _number(header.get("GAIN")),
        "width": int(cube.shape[2]),
        "height": int(cube.shape[1]),
        "mirrored": mirror,
    }


def _number(value: object) -> float | None:
    try:
        return float(value) if value is not None else None
    except (TypeError, ValueError):
        return None
