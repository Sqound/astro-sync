"""Overscan trimming for the stacking of a night.

A scientific CCD writes a few columns (or rows) of overscan beside the image and says where the
useful part is: ``TRIMSEC = '[1:4096, :]'`` (1-based, inclusive, x first), ``BIASSEC`` for the
overscan itself.  Left in the frames, the overscan is divided by a master flat that is ~0 there:
the calibrated frames reach −1, and the final ``-output_norm`` rescales [−1, 1] to [0, 1], which
lifts the sky of the master to 0.5.

This module cuts every frame of the night to the ``TRIMSEC`` when the whole night agrees on it.  It
works on the *working copies* only (the links under ``work/input/<kind>/``, replaced by trimmed
files): the originals are never opened for writing.  A frame without ``TRIMSEC`` leaves the night
exactly as it was, bit for bit, because nothing is read beyond the first light and nothing is
written.
"""

from __future__ import annotations

import re
from dataclasses import dataclass
from pathlib import Path

import raw_files

# The keys that describe a region of the sensor: they are removed from a trimmed frame because they
# no longer describe it (TRIMSEC would be applied twice by another tool).
REGION_KEYS = ("TRIMSEC", "BIASSEC", "DATASEC", "CCDSEC")

_AXIS = re.compile(r"^\s*(?:\*|:|(\d+)\s*:\s*(\d+))?\s*$")


@dataclass(frozen=True)
class Section:
    """A region of a frame, 1-based and inclusive, ``None`` for a whole axis."""

    x: tuple[int, int] | None
    y: tuple[int, int] | None

    def slices(self, width: int, height: int) -> tuple[int, int, int, int] | None:
        """``(x0, x1, y0, y1)``, 0-based, end excluded, or None when it does not fit the frame."""
        x0, x1 = (0, width) if self.x is None else (self.x[0] - 1, self.x[1])
        y0, y1 = (0, height) if self.y is None else (self.y[0] - 1, self.y[1])
        if not (0 <= x0 < x1 <= width and 0 <= y0 < y1 <= height):
            return None
        return x0, x1, y0, y1


def parse_section(text: object) -> Section | None:
    """``'[1:4096, :]'`` → ``Section((1, 4096), None)``; None for anything else (a step, a reversed range, text)."""
    if not isinstance(text, str):
        return None
    inner = text.strip()
    if not (inner.startswith("[") and inner.endswith("]")):
        return None
    parts = inner[1:-1].split(",")
    if len(parts) != 2:
        return None
    axes: list[tuple[int, int] | None] = []
    for part in parts:
        found = _AXIS.match(part)
        if found is None:
            return None
        if found.group(1) is None:
            axes.append(None)
            continue
        low, high = int(found.group(1)), int(found.group(2))
        if low < 1 or high < low:
            return None
        axes.append((low, high))
    return Section(axes[0], axes[1])


def _shape(header: object) -> tuple[int, int]:
    return int(header.get("NAXIS1", 0) or 0), int(header.get("NAXIS2", 0) or 0)  # type: ignore[attr-defined]


def trim_overscan(inputs: dict[str, list[Path]], work_input: Path, cfa: bool) -> dict[str, object] | None:
    """Trim the working copies under ``work_input/<kind>/`` to the ``TRIMSEC`` of the night.

    ``inputs`` maps a kind (``lights``, ``flats``, …) to the original files, in the order
    ``link_inputs`` numbered them.  The rectangle is read on the first light and checked on every
    frame of every kind: the same ``TRIMSEC`` and the same size, otherwise nothing is touched and the
    answer says why.  ``cfa`` is true for a one-shot-colour (Bayer) night: a trim that moves the
    pattern (an odd offset) is refused for the same reason.

    Returns None when the lights carry no ``TRIMSEC`` (nothing to say, nothing done), else a dict:
    ``trimmed`` (bool), ``section`` (the first light's text), and either ``width``, ``height``,
    ``fromWidth``, ``fromHeight``, ``frames``, ``message`` (it was done) or ``reason`` and ``message`` (it was not).
    """
    from astropy.io import fits

    lights = inputs.get("lights") or []
    if not lights or raw_files.is_raw(lights[0]):
        return None  # a camera RAW file declares no overscan
    first = fits.getheader(lights[0])
    text = first.get("TRIMSEC")
    if text is None:
        return None
    width, height = _shape(first)

    def refused(reason: str, message: str) -> dict[str, object]:
        return {"trimmed": False, "section": str(text), "reason": reason, "message": message}

    section = parse_section(text)
    if section is None:
        return refused("unreadable", f"TRIMSEC {text!r} is not a plain rectangle; the frames are left as they are")
    rectangle = section.slices(width, height)
    if rectangle is None:
        return refused("outside", f"TRIMSEC {text!r} does not fit a {width} x {height} frame; the frames are left as they are")
    x0, x1, y0, y1 = rectangle
    if (x0, x1, y0, y1) == (0, width, 0, height):
        return None  # the whole frame is useful: nothing to cut
    for kind, files in inputs.items():
        for path in files:
            header = fits.getheader(path)
            other = parse_section(header.get("TRIMSEC"))
            if other is None or other.slices(*_shape(header)) != rectangle or _shape(header) != (width, height):
                return refused(
                    "disagree",
                    f"the {kind} frame {path.name} does not carry the same TRIMSEC and size as the first light "
                    f"({text}, {width} x {height}); the frames are left as they are",
                )
    if cfa and (x0 % 2 or y0 % 2):
        return refused("bayer", f"TRIMSEC {text!r} would shift the Bayer pattern; the frames are left as they are")

    count = 0
    for kind, files in inputs.items():
        folder = work_input / kind
        for index, source in enumerate(files, 1):
            copy = folder / f"frame_{index:05d}{source.suffix.lower()}"
            _trim_one(copy, x0, x1, y0, y1)
            count += 1
    return {"trimmed": True, "section": str(text), "width": x1 - x0, "height": y1 - y0,
            "fromWidth": width, "fromHeight": height, "frames": count,
            "message": f"overscan cut from {count} working copies: {width} x {height} → {x1 - x0} x {y1 - y0} px (TRIMSEC {text})"}


def _trim_one(copy: Path, x0: int, x1: int, y0: int, y1: int) -> None:
    """Replace the working copy (a link to the original, or a copy) by its trimmed image."""
    import numpy as np
    from astropy.io import fits

    with fits.open(copy, memmap=False) as hdul:
        data = hdul[0].data
        header = hdul[0].header.copy()
    cut = np.ascontiguousarray(data[..., y0:y1, x0:x1])
    for key in REGION_KEYS:
        if key in header:
            del header[key]
    # The sky coordinates, when the frame has them, keep pointing at the same pixels.
    if x0 and "CRPIX1" in header:
        header["CRPIX1"] = float(header["CRPIX1"]) - x0
    if y0 and "CRPIX2" in header:
        header["CRPIX2"] = float(header["CRPIX2"]) - y0
    header["HISTORY"] = f"Overscan trimmed from the working copy: columns {x0 + 1}:{x1}, rows {y0 + 1}:{y1} kept"
    copy.unlink()  # the link first: the file it points to is never written
    fits.PrimaryHDU(data=cut, header=header).writeto(copy)


def lowest_pixel(frames: list[Path]) -> float | None:
    """The lowest finite pixel of the calibrated (floating-point) frames, None when there is none to read."""
    import numpy as np
    from astropy.io import fits

    lowest: float | None = None
    for path in frames:
        with fits.open(path, memmap=False) as hdul:
            if int(hdul[0].header.get("BITPIX", -32)) > 0:
                continue  # whole ADU counts (frames not calibrated): they cannot go below zero, and are not in [0, 1] units
            data = hdul[0].data
            if data is None:
                continue
            value = float(np.nanmin(data)) if data.size else None
        if value is not None and np.isfinite(value) and (lowest is None or value < lowest):
            lowest = value
    return lowest
