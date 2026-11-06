#!/usr/bin/env python3
"""Camera RAW files (DSLR, mirrorless) in the stacking environment.

A RAW file (``.cr2``, ``.cr3``, ``.nef``, ``.arw``, ``.dng``, ``.raf``, ``.pef``, ``.orf``, ``.rw2``) is read by
Siril (LibRaw) and by nothing else here: there is no RAW library in this environment, and none is needed.

* The stacking gives the RAW files to Siril as they are (``astro_sync_stack.py`` links them under uniform
  names, ``frame_00001.cr2``, and Siril's ``convert`` turns each into a one-layer FITS that keeps the mosaic of
  the sensor -- ``BAYERPAT`` is written -- so the calibration, the debayering and the rest are the ones of a
  one-shot-colour FITS frame).
* The measure of the poses (``pose_stats.py``) needs pixels, so it asks :class:`Converter` for the converted
  FITS of a pose; the converted copies live in a scratch folder and are removed with it.

The metadata of a RAW file (its exposure, its ISO, its clock, its camera) is read by Astro Sync itself, in
Rust (``raw_header.rs``); this module reads none of it.  The date a converted file carries (``DATE-OBS``) is
the camera clock turned into UTC by the time zone of *this computer*: it is not used.
"""

from __future__ import annotations

import shutil
import subprocess
from pathlib import Path

import siril_config

RAW_SUFFIXES = {".cr2", ".cr3", ".nef", ".arw", ".dng", ".raf", ".pef", ".orf", ".rw2"}
# Poses converted by one run of Siril: Siril's start-up costs about a second, a converted pose 40 to 150 MB.
BATCH = 8


def is_raw(path: Path) -> bool:
    """Whether ``path`` is a camera RAW file, by its extension (any case)."""
    return path.suffix.lower() in RAW_SUFFIXES


def is_xtrans(path: Path) -> bool:
    """Whether ``path`` is a Fujifilm RAF of an X-Trans sensor (a 6 x 6 colour filter pattern).

    Siril converts such a file and debayers it when it converts it, but ``calibrate -debayer`` leaves it a
    mosaic: an X-Trans night is debayered at the conversion, calibration frames included, and calibrated
    afterwards (see ``astro_sync_stack.siril_scripts``).  The pattern is read from the RAF's CFA header: a
    count of records, each a tag, a size (16 bits each) and its bytes, all big endian; record 0x131 is the
    pattern, 36 bytes for an X-Trans.
    """
    if path.suffix.lower() != ".raf":
        return False
    try:
        with path.open("rb") as handle:
            head = handle.read(100)
            if not head.startswith(b"FUJIFILMCCD-RAW"):
                return False
            offset = int.from_bytes(head[92:96], "big")
            handle.seek(offset)
            header = handle.read(1024)
    except OSError:
        return False
    position = 4
    for _ in range(min(int.from_bytes(header[:4], "big"), 32)):
        if position + 4 > len(header):
            return False
        tag = int.from_bytes(header[position:position + 2], "big")
        size = int.from_bytes(header[position + 2:position + 4], "big")
        if tag == 0x131:
            return size == 36
        position += 4 + size
    return False


def find_siril(given: Path | None) -> Path | None:
    """The Siril to run: the one given, else ``siril-cli`` or ``siril`` on the PATH; None when there is none."""
    if given is not None:
        return given if given.is_file() else None
    for name in ("siril-cli", "siril"):
        found = shutil.which(name)
        if found:
            return Path(found)
    return None


class ConversionError(Exception):
    """Siril could not convert a pose."""


class Converter:
    """Converts RAW poses to FITS with Siril, a few at a time, in ``scratch``.

    ``fits_of(path, upcoming)`` returns the converted file of ``path``; when it is not converted yet, the next
    poses of ``upcoming`` (RAW files that will be asked for next, in order) are converted with it, up to
    :data:`BATCH`, so that one run of Siril serves several poses.  ``release(path)`` removes a converted copy.
    """

    def __init__(self, siril: Path | None, scratch: Path) -> None:
        self.siril = siril
        self.scratch = scratch
        self.converted: dict[Path, Path] = {}
        self.round = 0

    def fits_of(self, path: Path, upcoming: list[Path] | None = None) -> Path:
        if path in self.converted:
            return self.converted[path]
        if self.siril is None:
            raise ConversionError("Siril is needed to read a camera RAW file, and none was found")
        batch = [path] + [other for other in (upcoming or []) if other != path and other not in self.converted]
        tail = self.convert(batch[:BATCH])
        if path not in self.converted:
            raise ConversionError(f"Siril could not convert {path.name}" + (f" ({tail})" if tail else ""))
        return self.converted[path]

    def convert(self, batch: list[Path]) -> str:
        """Convert ``batch``; the converted files are kept in ``converted``.  Returns the last line Siril said."""
        self.round += 1
        folder = self.scratch / f"raw-{self.round:04d}"
        source, out = folder / "in", folder / "out"
        source.mkdir(parents=True)
        out.mkdir()
        for index, path in enumerate(batch, 1):
            link = source / f"pose_{index:05d}{path.suffix.lower()}"
            try:
                link.symlink_to(path.resolve())
            except OSError:
                shutil.copy2(path, link)
        script = folder / "convert.ssf"
        script.write_text("requires 1.4.0\ncd in\nconvert pose -out=../out\nclose\n", encoding="utf-8")
        assert self.siril is not None
        result = subprocess.run(siril_config.command(self.siril, folder, script),
                                capture_output=True, text=True, timeout=1800)
        for index, path in enumerate(batch, 1):
            made = out / f"pose_{index:05d}.fit"
            if made.is_file():
                self.converted[path] = made
        tail = [line for line in (result.stdout + result.stderr).splitlines() if line.strip()][-1:]
        return tail[0] if tail else ""

    def release(self, path: Path) -> None:
        """Remove the converted copy of ``path``; its batch's folder goes with its last copy."""
        made = self.converted.pop(path, None)
        if made is None:
            return
        made.unlink(missing_ok=True)
        folder = made.parent.parent
        if not any(made.parent.iterdir()):
            shutil.rmtree(folder, ignore_errors=True)

    def close(self) -> None:
        """Remove every converted copy still there."""
        for folder in self.scratch.glob("raw-*"):
            shutil.rmtree(folder, ignore_errors=True)
        self.converted.clear()
