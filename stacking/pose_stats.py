#!/usr/bin/env python3
"""What each light frame says by itself: its stars and its sky.

Astro Sync measures a pose on its own pixels -- the width of its stars, their
eccentricity, how many stars it holds, how high its sky is -- so that the
poses that depart from the rest of the night (dawn, cloud, trailing) can be
named.  The ideas are those of the tools that rate every frame of a night; the
code is Astro Sync's own and rests on the measure of the stars of
``star_quality.py`` (the same stars, the same width, the same eccentricity),
read here frame by frame instead of on a master.

What is measured, for one frame (:func:`measure_pose`):

* ``skyBackground``: the median of the raw frame, in the unit of the file
  (ADU), before any other step;
* the stars, on the luminance of the frame (a colour sensor's mosaic is read
  on its green pixels at full resolution, the red and blue pixels being
  filled in from their green neighbours: a Bayer pattern would give every
  star four different widths, and the stars of a small pixel are only one or
  two pixels wide once averaged 2 x 2; the narrow stars of such a sensor are
  read with the edge rule of :data:`MINIMUM_STAR_FWHM`, a mono frame keeps the
  master's), the same candidates as the master's:
  ``starCount`` is the number of isolated stars found, **whatever their
  number** (a cloud is told by its few stars: ``star_quality.measure`` says
  nothing under twenty, this does say how many there were);
* ``fwhmPixels`` and ``eccentricity``: the medians over the stars read, in
  pixels of the frame, **only when twenty stars or more were read** (never a
  figure on a few); ``None`` otherwise;
* ``readableStars``, ``starWidths`` and ``starEccentricities``: how many stars
  were read, and, **only when there were fewer than twenty**, the width and
  the eccentricity of each (at most :data:`POOLED_PER_POSE`), so that the
  caller can pool the stars of a whole family of such frames (a frame of a
  sparse field has too few stars for a figure of its own, a night of them has
  enough: the width is then the family's, never the frame's);
* ``dateObs`` and ``exposure`` as the header says them (``exposure`` in
  seconds: ``EXPTIME``, else ``EXPOSURE`` in the unit its comment or the
  instrument states, see :func:`integration_master.exposure_seconds`).

Command line, one JSON line per frame, flushed as it goes::

    python3 pose_stats.py --frames-from list.txt [--siril PATH]

where ``list.txt`` holds one path per line.  A frame that cannot be read
gives ``{"path": ..., "status": "unreadable", "message": ...}`` and the next
frame follows.

A camera RAW file is converted by Siril, a few poses at a time, in a
scratch folder beside the list (``--siril`` names the program, else
``siril-cli`` on the PATH), then measured like a one-shot-colour frame; the
answer carries the RAW file's own path and no ``dateObs`` (the converted
file's date is the camera clock turned into UTC with the time zone of this
computer: the caller reads the date of the RAW file itself). Without Siril
a RAW pose is ``unreadable`` and says why.
"""

from __future__ import annotations

import json
import shutil
import sys
import tempfile
from pathlib import Path

import numpy as np

import integration_master
import raw_files
import star_quality

# The stars whose width is read, at most, on one frame (the master's limit).
STAR_LIMIT = 400
# The stars of a frame that has too few for a figure of its own are handed
# over one by one, at most this many (a frame has under twenty of them).
POOLED_PER_POSE = star_quality.MINIMUM_STARS - 1
# On a colour mosaic the stars are read with ``replicate_edges`` (see
# ``star_quality.profile_fwhm``): the small pixels of these sensors make stars
# of 2 to 4 px, and the master's rule would leave out those under 3.1 px, most
# of them (two stars in three on a Vaonis Stellina).  A mono frame keeps the
# master's rule, so that its figures stay those of the earlier versions.
#
# A star narrower than this (frame pixels) is then a hot pixel or a cosmic ray,
# not a star: a star of a real optic spreads over its neighbours, and a lone
# pixel read through the green interpolation falls to half within one pixel
# (on the frames of a Vaonis Stellina the two kinds are two clearly separate
# groups, under 1.8 px and over 2.2 px).
MINIMUM_STAR_FWHM = 2.0
# Which pixels of a 2 x 2 cell are green: the parity of ``row + column`` of the
# green pixels, for each of the four patterns (``RGGB`` has them at
# (0, 1) and (1, 0), ``GRBG`` at (0, 0) and (1, 1)).
GREEN_PARITY = {"RGGB": 1, "BGGR": 1, "GRBG": 0, "GBRG": 0}


def read_frame(path: Path) -> tuple[np.ndarray, dict[str, object]]:
    """The pixels of the first image of the FITS file at ``path`` and what its header says."""
    from astropy.io import fits

    with fits.open(path, memmap=False) as hdus:
        for hdu in hdus:
            if hdu.data is not None and getattr(hdu.data, "ndim", 0) in (2, 3):
                header = hdu.header
                data = np.asarray(hdu.data)
                break
        else:
            raise ValueError("the FITS file holds no image")
        info = {
            "dateObs": str(header["DATE-OBS"]) if "DATE-OBS" in header else None,
            "exposure": integration_master.exposure_seconds(header),
            "bayer": str(header.get("BAYERPAT", "")).strip() or None,
            # Where the first pixel sits in the pattern, when a writer shifts it.
            "bayerOffset": (_whole(header.get("XBAYROFF")) + _whole(header.get("YBAYROFF"))) % 2,
        }
    return data, info


def _whole(value: object) -> int:
    """``value`` as a whole number (0 when the header does not say one)."""
    return int(value) if isinstance(value, (int, float)) and value == value else 0


def green_plane(data: np.ndarray, bayer: str, offset: int = 0) -> np.ndarray | None:
    """The green pixels of a Bayer mosaic at full resolution, or None for a pattern with no green pixel known.

    Every red or blue pixel of the mosaic is replaced by the mean of its four
    green neighbours (up, down, left, right: in a Bayer pattern all four are
    green), so the stars keep their frame pixels and only one colour, the
    most sensitive one, is read.  ``offset`` (0 or 1) is the shift of the
    pattern that ``XBAYROFF + YBAYROFF`` declare.
    """
    parity = GREEN_PARITY.get(bayer.upper())
    if parity is None or data.ndim != 2 or min(data.shape) < 3:
        return None
    parity = (parity + offset) % 2
    height, width = data.shape
    rows, columns = np.ogrid[:height, :width]
    is_green = ((rows + columns) % 2) == parity
    padded = np.pad(data, 1, mode="reflect")
    around = (padded[:-2, 1:-1] + padded[2:, 1:-1] + padded[1:-1, :-2] + padded[1:-1, 2:]) * np.float32(0.25)
    return np.where(is_green, data, around).astype(np.float32)


def luminance_of(data: np.ndarray, bayer: str | None, offset: int = 0) -> tuple[np.ndarray, int]:
    """``(level, scale)``: the image the stars are read on, and how many frame pixels one of its pixels covers."""
    data = np.nan_to_num(np.asarray(data, dtype=np.float32))
    if data.ndim == 3:
        if data.shape[0] == 1:
            data = data[0]
        else:
            return star_quality.luminance(data), 1
    if bayer and data.ndim == 2:
        green = green_plane(data, bayer, offset)
        if green is not None:
            return green, 1
        # A pattern of which the green pixels are not known: 2 x 2 super-pixels.
        height, width = (data.shape[0] // 2) * 2, (data.shape[1] // 2) * 2
        cells = data[:height, :width].reshape(height // 2, 2, width // 2, 2)
        return cells.mean(axis=(1, 3)), 2
    return data, 1


# A candidate whose four nearest neighbours hold less than this share of its
# peak is a lone pixel: a star of 2 px or more keeps half of its peak one
# pixel away, a hot pixel read through the green interpolation a quarter.
LONE_PIXEL_SHARE = 0.35


def spread_out(signal: np.ndarray, ys: np.ndarray, xs: np.ndarray) -> tuple[np.ndarray, np.ndarray]:
    """The candidates that are not lone pixels (hot pixels, cosmic rays).

    On a frame that has not been calibrated, hot pixels are most of the
    candidates; the budget of :data:`STAR_LIMIT` draws is spent on the others.
    """
    if not len(ys):
        return ys, xs
    around = (signal[ys - 1, xs] + signal[ys + 1, xs] + signal[ys, xs - 1] + signal[ys, xs + 1]) * 0.25
    kept = around >= LONE_PIXEL_SHARE * signal[ys, xs]
    return ys[kept], xs[kept]


def measure_stars(level: np.ndarray, scale: int, narrow_stars: bool = False) -> dict[str, object]:
    """The stars of ``level``: how many, and (twenty or more read) their median width and eccentricity.

    Under twenty stars read the frame gives no figure of its own, but hands
    over the width and the eccentricity of each star it read
    (``starWidths``, ``starEccentricities``, in frame pixels) for a figure of
    its family.  ``narrow_stars``: the frame is a colour mosaic whose stars
    are 2 to 4 px wide (see :data:`MINIMUM_STAR_FWHM`).
    """
    if min(level.shape) < 4 * star_quality.MARGIN or float(level.max()) <= float(level.min()):
        return {"starCount": 0, "readableStars": 0, "fwhmPixels": None, "eccentricity": None}
    # Every isolated star counts, however many (the master's measure draws at most 400).
    signal, ys, xs = star_quality.candidate_stars(level, limit=10 ** 9, isolated=True)
    count = int(len(ys))
    ys, xs = spread_out(signal, ys, xs)
    if len(ys) > STAR_LIMIT:
        pick = np.random.default_rng(0).choice(len(ys), STAR_LIMIT, replace=False)
        ys, xs = ys[pick], xs[pick]
    grid_y, grid_x = np.mgrid[-star_quality.RADIUS:star_quality.RADIUS + 1, -star_quality.RADIUS:star_quality.RADIUS + 1]
    widths: list[float] = []
    eccentricities: list[float] = []
    for y, x in zip(ys, xs):
        fwhm = star_quality.profile_fwhm(
            star_quality.cut_of(signal, y, x, star_quality.RADIUS), grid_y, grid_x, replicate_edges=narrow_stars)
        if fwhm is None or not np.isfinite(fwhm) or fwhm <= 0:
            continue
        if narrow_stars and fwhm < MINIMUM_STAR_FWHM:
            continue
        eccentricity = star_quality.eccentricity_of(signal, y, x, fwhm)
        if eccentricity is None:
            continue
        widths.append(fwhm)
        eccentricities.append(eccentricity)
    if len(widths) < star_quality.MINIMUM_STARS:
        return {
            "starCount": count,
            "readableStars": len(widths),
            "fwhmPixels": None,
            "eccentricity": None,
            "starWidths": [round(width * scale, 3) for width in widths[:POOLED_PER_POSE]],
            "starEccentricities": [round(value, 3) for value in eccentricities[:POOLED_PER_POSE]],
        }
    return {
        "starCount": count,
        "readableStars": len(widths),
        "fwhmPixels": round(float(np.median(widths)) * scale, 3),
        "eccentricity": round(float(np.median(eccentricities)), 3),
    }


def measure_pose(path: Path) -> dict[str, object]:
    """What one frame says (see the module documentation)."""
    data, info = read_frame(path)
    level, scale = luminance_of(data, info["bayer"], info["bayerOffset"])
    raw = np.nan_to_num(np.asarray(data, dtype=np.float32))
    answer: dict[str, object] = {
        "path": str(path),
        "status": "measured",
        "dateObs": info["dateObs"],
        "exposure": info["exposure"],
        "skyBackground": round(float(np.median(raw)), 2),
    }
    plane = data[0] if data.ndim == 3 and data.shape[0] == 1 else data
    mosaic = plane.ndim == 2 and str(info["bayer"] or "").upper() in GREEN_PARITY
    answer.update(measure_stars(level, scale, narrow_stars=mosaic))
    return answer


def measure_raw(path: Path, converter: "raw_files.Converter", upcoming: list[Path]) -> dict[str, object]:
    """The measure of a RAW pose: Siril converts it, the converted copy is measured and removed.

    The answer is under the RAW file's path and has no date: the date of a converted copy is wrong (see the
    module documentation).
    """
    converted = converter.fits_of(path, upcoming)
    try:
        answer = measure_pose(converted)
    finally:
        converter.release(path)
    answer["path"] = str(path)
    answer["dateObs"] = None
    return answer


def main(argv: list[str] | None = None) -> int:
    arguments = sys.argv[1:] if argv is None else argv
    siril: Path | None = None
    if len(arguments) == 4 and arguments[2] == "--siril":
        siril = Path(arguments[3])
        arguments = arguments[:2]
    if len(arguments) != 2 or arguments[0] != "--frames-from":
        print("usage: pose_stats.py --frames-from LIST.txt [--siril PATH]", file=sys.stderr)
        return 2
    list_file = Path(arguments[1])
    paths = [line.strip("\r\n") for line in list_file.read_text(encoding="utf-8").splitlines() if line.strip()]
    converter: raw_files.Converter | None = None
    scratch: Path | None = None
    try:
        for index, path in enumerate(paths):
            try:
                if raw_files.is_raw(Path(path)):
                    if converter is None:
                        scratch = Path(tempfile.mkdtemp(prefix="raw-poses-", dir=list_file.parent))
                        converter = raw_files.Converter(raw_files.find_siril(siril), scratch)
                    upcoming = [Path(other) for other in paths[index + 1:] if raw_files.is_raw(Path(other))]
                    answer = measure_raw(Path(path), converter, upcoming)
                else:
                    answer = measure_pose(Path(path))
            except Exception as error:  # a frame that cannot be read never stops the others
                answer = {"path": path, "status": "unreadable", "message": str(error)}
            print(json.dumps(answer), flush=True)
    finally:
        if converter is not None:
            converter.close()
        if scratch is not None:
            shutil.rmtree(scratch, ignore_errors=True)
    return 0


if __name__ == "__main__":
    sys.exit(main())
