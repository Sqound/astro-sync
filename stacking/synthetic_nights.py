"""Synthetic frames of a sky that turns, for the end-to-end tests of the stacking (Siril on the PATH).

The stars of a disc wide enough to fill the frame at any angle, seen through a frame turned about its
centre: the field rotation of an alt-azimuth mount (finding TD1), at the scale of a test.
"""

from __future__ import annotations

import math
from pathlib import Path

import numpy as np
from astropy.io import fits

SHAPE = (256, 320)


def sky_around_the_pointing(seed: int = 7) -> np.ndarray:
    """The stars around the pointing: a disc wide enough to fill the frame at any angle (rows: v, u, flux)."""
    rng = np.random.default_rng(seed)
    count, radius = 500, 215.0
    distance = radius * np.sqrt(rng.uniform(0, 1, count))
    bearing = rng.uniform(0, 2 * np.pi, count)
    return np.column_stack([distance * np.sin(bearing), distance * np.cos(bearing), rng.uniform(0.3, 1.0, count)])


def bayer_mosaic(planes: list[np.ndarray]) -> np.ndarray:
    """The RGGB mosaic a colour sensor records of three planes (red, green, blue)."""
    mosaic = np.zeros(planes[0].shape)
    mosaic[0::2, 0::2] = planes[0][0::2, 0::2]
    mosaic[0::2, 1::2] = planes[1][0::2, 1::2]
    mosaic[1::2, 0::2] = planes[1][1::2, 0::2]
    mosaic[1::2, 1::2] = planes[2][1::2, 1::2]
    return mosaic


def write_night(root: Path, name: str, sky: np.ndarray, angles: list[float], shape: tuple[int, int] = SHAPE,
                drift: float = 0.0, blank: set[int] = frozenset(), target: str = "M 45",
                colour: tuple[float, float, float] | None = None, filter_name: str | None = None,
                exposure: float = 30.0) -> Path:
    """Mono frames of ``sky`` turned about the centre of the frame by ``angles`` degrees, in ``<root>/<name>/<target>/lights``.

    ``blank``: indexes of frames without a star.  ``colour``: the relative brightness of the stars in the red,
    green and blue planes of a colour sensor (RGGB mosaic, ``BAYERPAT`` in the header) instead of a mono frame;
    ``filter_name`` and ``exposure`` go to the header.  Returns the folder of the lights.
    """
    lights = root / name / target / "lights"
    lights.mkdir(parents=True)
    rng = np.random.default_rng(len(name))
    height, width = shape
    yy, xx = np.mgrid[0:height, 0:width]
    for index, angle in enumerate(angles):
        c, s = math.cos(math.radians(angle)), math.sin(math.radians(angle))
        field = np.zeros(shape)
        if index not in blank:
            for v, u, flux in sky:
                y = height / 2 + c * v - s * u + drift * index
                x = width / 2 + s * v + c * u - drift * index
                if -8 < y < height + 8 and -8 < x < width + 8:
                    field += flux * np.exp(-((yy - y) ** 2 + (xx - x) ** 2) / (2 * 1.8 ** 2))
        if colour is None:
            frame = rng.poisson(np.clip(400 + 3000 * field, 0, None)).astype(np.uint16)
        else:
            mosaic = bayer_mosaic([400 + 3000 * weight * field for weight in colour])
            frame = rng.poisson(np.clip(mosaic, 0, None)).astype(np.uint16)
        header = fits.Header()
        for key, value in (("IMAGETYP", "Light"), ("OBJECT", target), ("EXPTIME", exposure), ("GAIN", 200),
                           ("XBINNING", 1), ("CCD-TEMP", 20.0), ("INSTRUME", "Synthetic")):
            header[key] = value
        if filter_name:
            header["FILTER"] = filter_name
        if colour is not None:
            header["BAYERPAT"] = "RGGB"
        fits.PrimaryHDU(frame, header).writeto(lights / f"Light_{index:03d}.fit")
    return lights


# ---- camera RAW files -------------------------------------------------------------------------------

import struct

_TYPE_SIZE = {1: 1, 2: 1, 3: 2, 4: 4, 5: 8, 10: 8}


def _entry(tag: int, kind: int, values: object) -> tuple[int, int, int, bytes]:
    """``(tag, type, count, data)`` of one TIFF entry (little endian)."""
    if kind == 2:
        data = str(values).encode("ascii") + b"\0"
        return tag, kind, len(data), data
    items = list(values) if isinstance(values, (list, tuple)) else [values]
    pack = {1: "B", 3: "H", 4: "I", 5: "II", 10: "ii"}[kind]
    data = b"".join(struct.pack("<" + pack, *(item if isinstance(item, tuple) else (item,))) for item in items)
    return tag, kind, len(items), data


def _ifd(entries: list[tuple[int, int, int, bytes]], start: int) -> bytes:
    """An IFD at offset ``start``: its entries, then the data that does not fit in four bytes."""
    entries = sorted(entries)
    data_start = start + 2 + 12 * len(entries) + 4
    head, tail = struct.pack("<H", len(entries)), b""
    for tag, kind, count, data in entries:
        if len(data) <= 4:
            head += struct.pack("<HHI", tag, kind, count) + data.ljust(4, b"\0")
        else:
            head += struct.pack("<HHII", tag, kind, count, data_start + len(tail))
            tail += data + (b"\0" if len(data) % 2 else b"")
    return head + struct.pack("<I", 0) + tail


def write_dng(path: Path, mosaic: np.ndarray, exposure: float, iso: int, taken: str, offset: str | None = "+09:00",
              model: str = "Synthetic RAW 1") -> Path:
    """A 16-bit uncompressed DNG (RGGB) of ``mosaic``, with the EXIF block a camera writes.

    ``taken``: ``YYYY:MM:DD HH:MM:SS`` (the camera clock); ``offset``: ``OffsetTimeOriginal`` (None: the file
    does not say its time zone).  A real DNG of a real camera is what Siril reads; this one is the smallest file
    LibRaw takes for one, for the tests.
    """
    height, width = mosaic.shape
    pixels = np.ascontiguousarray(mosaic.astype("<u2")).tobytes()
    exposure_ratio = (int(round(exposure * 1000)), 1000)
    exif = [
        _entry(33434, 5, [exposure_ratio]),
        _entry(34855, 3, iso),
        _entry(36867, 2, taken),
    ]
    if offset is not None:
        exif.append(_entry(36881, 2, offset))

    def main(image_offset: int, exif_offset: int) -> list[tuple[int, int, int, bytes]]:
        return [
            _entry(254, 4, 0), _entry(256, 4, width), _entry(257, 4, height), _entry(258, 3, 16), _entry(259, 3, 1),
            _entry(262, 3, 32803), _entry(271, 2, "Sqound Test"), _entry(272, 2, model),
            _entry(273, 4, image_offset), _entry(274, 3, 1), _entry(277, 3, 1), _entry(278, 4, height),
            _entry(279, 4, len(pixels)), _entry(284, 3, 1), _entry(306, 2, taken),
            _entry(33421, 3, [2, 2]), _entry(33422, 1, [0, 1, 1, 2]), _entry(34665, 4, exif_offset),
            _entry(50706, 1, [1, 4, 0, 0]), _entry(50707, 1, [1, 1, 0, 0]), _entry(50708, 2, model),
            _entry(50717, 4, 65535), _entry(50721, 10, [(10000, 10000) if i % 4 == 0 else (0, 10000) for i in range(9)]),
            _entry(50728, 5, [(1, 1)] * 3), _entry(50778, 3, 21),
        ]

    first = len(_ifd(main(0, 0), 8))
    exif_at = 8 + first
    second = len(_ifd(exif, exif_at))
    image_at = exif_at + second
    blob = b"II*\0" + struct.pack("<I", 8) + _ifd(main(image_at, exif_at), 8) + _ifd(exif, exif_at) + pixels
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(blob)
    return path


def write_raw_night(root: Path, name: str, sky: np.ndarray, angles: list[float], shape: tuple[int, int] = SHAPE,
                    target: str = "M 45", exposure: float = 30.0, iso: int = 800, offset: str | None = "+09:00",
                    darks: int = 3, flats: int = 3, biases: int = 3, evening: str = "2026:10:07",
                    colour: tuple[float, float, float] = (1.0, 0.9, 0.8)) -> Path:
    """A night of camera RAW files (DNG): ``<root>/<name>/<target>/lights/Light_###.dng`` of ``sky`` turned by
    ``angles``, and ``calibration/{darks,flats,biases}`` of the same camera (a dark has the lights' exposure and
    ISO, a flat is a bright flat field, a bias is the shortest exposure).  Returns the night's folder."""
    night = root / name
    rng = np.random.default_rng(len(name) + 1)
    height, width = shape
    yy, xx = np.mgrid[0:height, 0:width]
    hot = rng.uniform(0, 1, shape) > 0.9995  # a few hot pixels, which the darks hold too
    pedestal = 512.0

    def stamp(seconds: int) -> str:
        return f"{evening} 23:{seconds // 60 % 60:02d}:{seconds % 60:02d}"

    for index, angle in enumerate(angles):
        c, s = math.cos(math.radians(angle)), math.sin(math.radians(angle))
        field = np.zeros(shape)
        for v, u, flux in sky:
            y, x = height / 2 + c * v - s * u, width / 2 + s * v + c * u
            if -8 < y < height + 8 and -8 < x < width + 8:
                field += flux * np.exp(-((yy - y) ** 2 + (xx - x) ** 2) / (2 * 1.8 ** 2))
        mosaic = bayer_mosaic([pedestal + 4000 * weight * field + 300 * hot for weight in colour])
        frame = np.clip(rng.poisson(np.clip(mosaic, 0, None)), 0, 65535)
        write_dng(night / target / "lights" / f"Light_{index + 1:04d}.dng", frame, exposure, iso, stamp(index * 40), offset)
    calibration = night / "calibration"
    for index in range(darks):
        frame = rng.poisson(np.full(shape, pedestal) + 300 * hot)
        write_dng(calibration / "darks" / f"Dark_{index + 1:04d}.dng", frame, exposure, iso, stamp(1200 + index * 40), offset)
    for index in range(flats):
        vignette = 1.0 - 0.3 * (((yy - height / 2) / height) ** 2 + ((xx - width / 2) / width) ** 2)
        frame = rng.poisson(np.full(shape, pedestal) + 20000 * vignette)
        write_dng(calibration / "flats" / f"Flat_{index + 1:04d}.dng", frame, 0.01, iso, stamp(1400 + index * 5), offset)
    for index in range(biases):
        frame = rng.poisson(np.full(shape, pedestal))
        write_dng(calibration / "biases" / f"Bias_{index + 1:04d}.dng", frame, 0.0001, iso, stamp(1500 + index * 5), offset)
    return night
