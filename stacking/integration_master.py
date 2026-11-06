"""The master kept of one family of a night.

When a target of a night is stacked, Astro Sync keeps one file besides the
renders: the 32-bit linear master of the family that was integrated (same
filter, exposure, gain and binning), as::

    <night>/<target>/stacked/<family>_integration.fit

It is the only intermediate file kept: with
it, a later night of the same multi-night plan can be combined with this one
without stacking this night again (``plan_stack.py``).  A new stacking of the
same family replaces it.

The master describes itself in its own FITS header, so nothing else is
needed to read it back (``astro_sync_core::multi_night::read_master``):

``ASFAMILY``  the family's label (``Ha_300s_g100``), the stem of the file;
``FILTER``    the filter as the frames name it (absent: no filter);
``EXPTIME``   the exposure of ONE frame, seconds;
``GAIN``, ``XBINNING``, ``YBINNING``  as the frames say;
``CCD-TEMP``  the median sensor temperature of the frames, °C;
``INSTRUME``  the sensor;
``STACKCNT``  the frames integrated (Siril's own count when it wrote one);
``LIVETIME``  STACKCNT × EXPTIME, seconds;
``XPIXSZ``, ``FOCALLEN``, ``PIXSCALE``  the optics of the frames when they say
 it (the pixel scale of a plan's image, hence the
              seeing of its quality in arc seconds), copied only when the
              master does not carry them already;
``ASROTMIN``, ``ASROTMAX``  the range of the field rotation of the frames, in
              degrees relative to the reference frame, as the registration
 measured it (a plan measures its field
              rotation over all the frames of all its nights); absent when it
              was not measured, and in the masters kept before that lot, which
              stood for a field that did not turn.

Only the standard library, numpy and astropy (already in the stacking
environment) are used.
"""

from __future__ import annotations

import hashlib
import os
import shutil
import statistics
from pathlib import Path

import raw_files

SUFFIX = "_integration.fit"
# What the frames say of their optics: the pixel size (binning included, as
# N.I.N.A. writes it), the focal length, a scale keyword.
OPTICS_KEYS = ("XPIXSZ", "FOCALLEN", "PIXSCALE")


def master_path(out: Path, family: str) -> Path:
    """Where the master of ``family`` is kept in ``out`` (the target's ``stacked/``)."""
    return out / f"{family}{SUFFIX}"


def _number(value: object) -> float | None:
    try:
        number = float(value)  # type: ignore[arg-type]
    except (TypeError, ValueError):
        return None
    return number if number == number else None  # NaN is no value


def _comment_unit_seconds(comment: object) -> float | None:
    """The time unit a header comment states between brackets or parentheses, as a factor to seconds.

    ``[ms] Total Exposure Time`` gives 0.001, ``[s] ...`` gives 1.0, a comment
    that states no time unit gives None (the Rust readers' ``time_unit_of_comment``).
    """
    text = str(comment or "").lstrip()
    closing = {"[": "]", "(": ")"}.get(text[:1])
    if closing is None or closing not in text:
        return None
    unit = text[1:text.index(closing)].strip().lower()
    if unit in ("ms", "msec", "millisecond", "milliseconds"):
        return 0.001
    if unit in ("s", "sec", "second", "seconds"):
        return 1.0
    return None


def exposure_seconds(header: object) -> float | None:
    """The exposure of ONE frame, in seconds, from a FITS header (the rule of the Rust readers).

    ``EXPTIME`` is always seconds.  Else ``EXPOSURE``, whose unit is not
    fixed: the unit its comment states (``[ms]``, ``[s]``), else the unit of a
    known instrument (a Vaonis Stellina writes milliseconds: ``INSTRUME``
    starting with ``stellina``), else seconds.
    """
    seconds = _number(header.get("EXPTIME"))  # type: ignore[attr-defined]
    if seconds is not None:
        return seconds
    raw = _number(header.get("EXPOSURE"))  # type: ignore[attr-defined]
    if raw is None:
        return None
    comments = getattr(header, "comments", None)
    try:
        stated = _comment_unit_seconds(comments["EXPOSURE"]) if comments is not None else None
    except KeyError:
        stated = None
    if stated is None:
        instrument = str(header.get("INSTRUME") or "").strip().lower()  # type: ignore[attr-defined]
        stated = 0.001 if instrument.startswith("stellina") else 1.0
    return raw * stated


def _text(value: object) -> str | None:
    text = str(value).strip() if value is not None else ""
    return text or None


def frame_facts(lights: list[Path], converted: list[Path] | None = None,
                assumed_filter: str | None = None) -> dict[str, object]:
    """What the light frames of one family share, read from their headers.

 Camera RAW lights have no FITS header: ``converted`` names the FITS copies Siril made of them
    (``process/light_*.fit``, the same lights in the same order), whose headers say the exposure, the ISO
    (``ISOSPEED``, taken as the gain: the key Astro Sync's families use for a camera), the camera and the
    optics.

    The filter, the binning and the sensor of the first frame (a family has
    one of each); the exposure and the gain as their median (one value in a
    family); the sensor temperature as its median (the stacking does not
    split a family by a degree).

    ``assumed_filter``: the filter the frames were taken with when their headers name none (an ASIAIR writes no
    ``FILTER`` card); the frames themselves are never written.
    """
    from astropy.io import fits

    sources = converted if converted and lights and raw_files.is_raw(lights[0]) else lights
    headers = [fits.getheader(path, memmap=False) for path in sources]
    first = headers[0] if headers else {}

    def median(*keys: str) -> float | None:
        values = []
        for header in headers:
            for key in keys:
                number = _number(header.get(key))
                if number is not None:
                    values.append(number)
                    break
        return statistics.median(values) if values else None

    exposures = [seconds for seconds in (exposure_seconds(header) for header in headers) if seconds is not None]

    return {
        "filter": _text(first.get("FILTER")) or _text(assumed_filter),
        "exposureSeconds": statistics.median(exposures) if exposures else None,
        "gain": median("GAIN", "ISOSPEED"),
        "binning": _text(first.get("XBINNING")),
        "temperatureC": median("CCD-TEMP", "SET-TEMP"),
        "sensor": _text(first.get("INSTRUME")),
        "optics": {key: first[key] for key in OPTICS_KEYS if _number(first.get(key)) is not None},
    }


def _sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for block in iter(lambda: handle.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


def keep_master(stacked: Path, out: Path, family: str, lights: list[Path],
                rotation_range: tuple[float, float] | None = None,
                converted: list[Path] | None = None,
                assumed_filter: str | None = None) -> dict[str, object]:
    """Copies the linear master ``stacked`` to ``out/<family>_integration.fit``.

    The copy is written beside its final name, then renamed, so a stop never
    leaves half a master.  Its header is completed with what the family is
    (see the module's documentation); ``rotation_range``: the range of the
    field rotation of the lights the registration measured, ``(low, high)``
    degrees relative to the reference light.  Returns the line of the
    provenance (``<name>_stacking.json``, ``integration``).
    """
    from astropy.io import fits

    out.mkdir(parents=True, exist_ok=True)
    final = master_path(out, family)
    partial = out / f".{final.name}.part"
    shutil.copyfile(stacked, partial)
    facts = frame_facts(lights, converted, assumed_filter)
    with fits.open(partial, mode="update", memmap=False) as hdul:
        header = hdul[0].header
        frames = int(_number(header.get("STACKCNT")) or _number(header.get("NCOMBINE")) or len(lights))
        exposure = facts["exposureSeconds"]
        header["ASFAMILY"] = (family, "Astro Sync: family of the frames integrated")
        if facts["filter"] is not None:
            header["FILTER"] = (facts["filter"], "filter of the frames")
        elif "FILTER" in header:
            del header["FILTER"]
        if exposure is not None:
            header["EXPTIME"] = (exposure, "[s] exposure of one frame")
            header["LIVETIME"] = (frames * float(exposure), "[s] frames x exposure")
        if facts["gain"] is not None:
            header["GAIN"] = (facts["gain"], "gain of the frames")
        if facts["binning"] is not None:
            header["XBINNING"] = (facts["binning"], "binning of the frames")
            header["YBINNING"] = (facts["binning"], "binning of the frames")
        if facts["temperatureC"] is not None:
            header["CCD-TEMP"] = (facts["temperatureC"], "[C] median sensor temperature")
        if facts["sensor"] is not None:
            header["INSTRUME"] = (facts["sensor"], "sensor")
        for key, value in facts["optics"].items():  # type: ignore[union-attr]
            if key not in header:
                header[key] = (value, "optics of the frames")
        header["STACKCNT"] = (frames, "frames integrated")
        if rotation_range is not None:
            header["ASROTMIN"] = (rotation_range[0], "[deg] field rotation of the frames, lowest")
            header["ASROTMAX"] = (rotation_range[1], "[deg] field rotation of the frames, highest")
        hdul.flush()
    os.replace(partial, final)
    return {
        "family": family,
        "file": final.name,
        "frames": frames,
        "exposureSeconds": exposure,
        "totalSeconds": frames * float(exposure) if exposure is not None else None,
        "filter": facts["filter"],
        "gain": facts["gain"],
        "binning": facts["binning"],
        "temperatureC": facts["temperatureC"],
        "sensor": facts["sensor"],
        "bytes": final.stat().st_size,
        "sha256": _sha256(final),
    }
