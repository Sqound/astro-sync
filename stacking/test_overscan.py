"""Tests of the overscan trim and of the normalisation guard.

Run from this folder, with the stacking environment:
    python3 -m unittest -v test_overscan

The unit tests need numpy and astropy.  The end-to-end cases run Siril (``siril-cli`` on the PATH, or
``ASTRO_SYNC_SIRIL``) on synthetic frames and are skipped without it.
"""

from __future__ import annotations

import hashlib
import json
import os
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent

try:
    import numpy as np
    from astropy.io import fits
except ImportError:  # the stacking environment has them; a bare interpreter does not
    np = fits = None

import astro_sync_stack
import overscan

NEEDS_NUMPY = unittest.skipIf(np is None, "needs numpy and astropy (the stacking environment)")
SIRIL = os.environ.get("ASTRO_SYNC_SIRIL") or shutil.which("siril-cli")

# A sensor of 64 rows and 64 useful columns, with 13 columns of overscan on the right.
ROWS, USEFUL, BAND = 64, 64, 13
SECTION = f"[1:{USEFUL}, :]"


def digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def write_frame(path: Path, value: float, section: str | None = SECTION, band: float | None = None,
                seed: int = 0, extra: dict[str, object] | None = None, columns: int = USEFUL + BAND) -> Path:
    """A frame of ``value`` ADU (with a little noise) whose last ``BAND`` columns are at ``band`` ADU."""
    rng = np.random.default_rng(seed)
    data = rng.normal(value, 1.0, (ROWS, columns))
    if band is not None:
        data[:, USEFUL:] = band
    header = fits.Header()
    if section is not None:
        header["TRIMSEC"] = section
        header["BIASSEC"] = f"[{USEFUL + 1}:{columns}]"
    for key, item in (extra or {}).items():
        header[key] = item
    fits.PrimaryHDU(np.clip(data, 0, 65535).astype(np.uint16), header).writeto(path)
    return path


@NEEDS_NUMPY
class ParseSection(unittest.TestCase):
    def test_the_plain_rectangles(self) -> None:
        self.assertEqual(overscan.parse_section("[1:4096, :]"), overscan.Section((1, 4096), None))
        self.assertEqual(overscan.parse_section("[1:10,5:20]"), overscan.Section((1, 10), (5, 20)))
        self.assertEqual(overscan.parse_section("[*, 3:4]"), overscan.Section(None, (3, 4)))

    def test_anything_else_is_not_a_rectangle(self) -> None:
        for text in (None, 5, "", "1:4096", "[1:4096]", "[1:4096:2, :]", "[4096:1, :]", "[0:10, :]", "[a:b, :]", "[1:2,3:4,5:6]"):
            self.assertIsNone(overscan.parse_section(text), text)

    def test_the_slices_are_zero_based_and_must_fit(self) -> None:
        section = overscan.Section((1, 64), None)
        self.assertEqual(section.slices(77, 64), (0, 64, 0, 64))
        self.assertEqual(overscan.Section((3, 10), (2, 5)).slices(77, 64), (2, 10, 1, 5))
        self.assertIsNone(overscan.Section((1, 100), None).slices(77, 64))


@NEEDS_NUMPY
class Trim(unittest.TestCase):
    def setUp(self) -> None:
        self.root = Path(tempfile.mkdtemp())
        self.addCleanup(lambda: shutil.rmtree(self.root, ignore_errors=True))
        self.frames: dict[str, list[Path]] = {}

    def night(self, section: str | None = SECTION, flats_section: str | None = SECTION) -> None:
        folder = self.root / "originals"
        for kind, count, value, kind_section in (("lights", 3, 500, section), ("biases", 2, 100, section),
                                                 ("flats", 2, 20000, flats_section)):
            (folder / kind).mkdir(parents=True)
            self.frames[kind] = [
                write_frame(folder / kind / f"{kind}_{index}.fit", value, kind_section, band=100 if kind != "lights" else 90,
                            seed=index) for index in range(count)
            ]

    def link(self) -> Path:
        plan = astro_sync_stack.Plan(frames=self.frames, colour=False, pipelines=[])
        work = self.root / "work"
        work.mkdir()
        astro_sync_stack.link_inputs(plan, work)
        return work / "input"

    def test_every_frame_of_the_night_is_cut_to_the_trimsec(self) -> None:
        self.night()
        before = {path: digest(path) for files in self.frames.values() for path in files}
        inputs = self.link()
        note = overscan.trim_overscan(self.frames, inputs, cfa=False)
        self.assertEqual(note["trimmed"], True)
        self.assertEqual((note["width"], note["height"], note["fromWidth"], note["frames"]), (USEFUL, ROWS, USEFUL + BAND, 7))
        for kind, files in self.frames.items():
            for index in range(1, len(files) + 1):
                copy = inputs / kind / f"frame_{index:05d}.fit"
                self.assertFalse(copy.is_symlink())
                with fits.open(copy) as hdul:
                    self.assertEqual(hdul[0].data.shape, (ROWS, USEFUL))
                    self.assertEqual(hdul[0].data.dtype, np.uint16)
                    self.assertEqual(hdul[0].header["NAXIS1"], USEFUL)
                    self.assertNotIn("TRIMSEC", hdul[0].header)
                    self.assertNotIn("BIASSEC", hdul[0].header)
                    original = fits.getdata(files[index - 1])
                    np.testing.assert_array_equal(hdul[0].data, original[:, :USEFUL])
        # The originals were never opened for writing.
        self.assertEqual(before, {path: digest(path) for files in self.frames.values() for path in files})

    def test_the_band_is_gone_from_the_stack_of_the_trimmed_calibrated_frames(self) -> None:
        # What Siril does, in numpy: (light - bias) / normalised flat, then the mean.  The flat is ~0 in the band.
        self.night()
        inputs = self.link()
        overscan.trim_overscan(self.frames, inputs, cfa=False)
        read = lambda kind, n: fits.getdata(inputs / kind / f"frame_{n:05d}.fit").astype(float)
        bias, flat = read("biases", 1), read("flats", 1)
        flat = (flat - bias) / np.median(flat - bias)
        calibrated = np.mean([(read("lights", n) - bias) / flat for n in (1, 2, 3)], axis=0)
        self.assertEqual(calibrated.shape, (ROWS, USEFUL))
        self.assertLess(abs(float(np.median(calibrated)) - 400) / 65535, 0.01)

    def test_a_night_without_trimsec_is_left_exactly_as_it_was(self) -> None:
        self.night(section=None, flats_section=None)
        inputs = self.link()
        self.assertIsNone(overscan.trim_overscan(self.frames, inputs, cfa=False))
        for kind, files in self.frames.items():
            for index, source in enumerate(files, 1):
                copy = inputs / kind / f"frame_{index:05d}.fit"
                self.assertTrue(copy.is_symlink())
                self.assertEqual(copy.resolve(), source.resolve())

    def test_a_different_trimsec_on_one_frame_cuts_nothing_and_says_so(self) -> None:
        self.night()
        write_frame(self.root / "other.fit", 500, f"[1:{USEFUL - 4}, :]", band=90)
        self.frames["lights"].append(self.root / "other.fit")
        inputs = self.link()
        note = overscan.trim_overscan(self.frames, inputs, cfa=False)
        self.assertEqual((note["trimmed"], note["reason"]), (False, "disagree"))
        self.assertIn("other.fit", note["message"])
        self.assertTrue(all(path.is_symlink() for path in inputs.rglob("*.fit")))

    def test_a_calibration_frame_without_trimsec_cuts_nothing(self) -> None:
        self.night(flats_section=None)
        inputs = self.link()
        note = overscan.trim_overscan(self.frames, inputs, cfa=False)
        self.assertEqual((note["trimmed"], note["reason"]), (False, "disagree"))
        self.assertTrue(all(path.is_symlink() for path in inputs.rglob("*.fit")))

    def test_a_section_that_is_not_a_rectangle_or_does_not_fit_is_left_alone(self) -> None:
        for section, reason in (("[1:64:2, :]", "unreadable"), (f"[1:{USEFUL + BAND + 5}, :]", "outside")):
            with self.subTest(section=section):
                self.frames = {}
                shutil.rmtree(self.root / "originals", ignore_errors=True)
                shutil.rmtree(self.root / "work", ignore_errors=True)
                self.night(section=section, flats_section=section)
                note = overscan.trim_overscan(self.frames, self.link(), cfa=False)
                self.assertEqual((note["trimmed"], note["reason"]), (False, reason))

    def test_a_section_covering_the_whole_frame_is_nothing_to_cut(self) -> None:
        self.night(section=f"[1:{USEFUL + BAND}, :]", flats_section=f"[1:{USEFUL + BAND}, :]")
        self.assertIsNone(overscan.trim_overscan(self.frames, self.link(), cfa=False))

    def test_a_trim_that_would_move_the_bayer_pattern_is_refused(self) -> None:
        self.night(section="[2:64, :]", flats_section="[2:64, :]")
        note = overscan.trim_overscan(self.frames, self.link(), cfa=True)
        self.assertEqual((note["trimmed"], note["reason"]), (False, "bayer"))
        self.frames = {}
        shutil.rmtree(self.root / "originals")
        shutil.rmtree(self.root / "work")
        self.night(section="[3:64, :]", flats_section="[3:64, :]")
        self.assertEqual(overscan.trim_overscan(self.frames, self.link(), cfa=True)["trimmed"], True)

    def test_the_sky_coordinates_follow_the_cut(self) -> None:
        self.night(section="[5:64, 3:60]", flats_section="[5:64, 3:60]")
        write_frame(self.root / "wcs.fit", 500, "[5:64, 3:60]", band=90, extra={"CRPIX1": 30.0, "CRPIX2": 20.0})
        self.frames["lights"] = [self.root / "wcs.fit", *self.frames["lights"][1:]]
        inputs = self.link()
        overscan.trim_overscan(self.frames, inputs, cfa=False)
        header = fits.getheader(inputs / "lights" / "frame_00001.fit")
        self.assertEqual((header["CRPIX1"], header["CRPIX2"]), (26.0, 18.0))
        self.assertEqual((header["NAXIS1"], header["NAXIS2"]), (60, 58))

    def test_the_lowest_pixel_of_the_calibrated_frames(self) -> None:
        paths = []
        for index, floor in enumerate((0.01, -0.4, 0.2)):
            data = np.full((8, 8), 0.3, dtype="float32")
            data[2, 3] = floor
            path = self.root / f"pp_{index}.fit"
            fits.PrimaryHDU(data).writeto(path)
            paths.append(path)
        self.assertAlmostEqual(overscan.lowest_pixel(paths), -0.4, places=5)
        self.assertIsNone(overscan.lowest_pixel([]))

    def test_whole_adu_frames_are_not_read_as_a_unit_scale(self) -> None:
        # Frames that were not calibrated stay as 16-bit counts (with a BZERO): nothing to measure there.
        path = self.root / "light.fit"
        fits.PrimaryHDU(np.full((8, 8), 700, dtype=np.uint16)).writeto(path)
        self.assertIsNone(overscan.lowest_pixel([path]))


@NEEDS_NUMPY
class NormalisationGuard(unittest.TestCase):
    def test_the_stack_is_rescaled_by_default_and_left_alone_when_asked(self) -> None:
        plan = astro_sync_stack.Plan(frames={kind: [Path(f"{kind}.fit")] for kind in astro_sync_stack.KINDS}, colour=False, pipelines=[])
        integration = dict(astro_sync_stack.siril_scripts(plan)[0])["integration"]
        self.assertIn(" -output_norm -32b ", integration)
        kept = dict(astro_sync_stack.siril_scripts(plan, output_norm=False)[0])["integration"]
        self.assertNotIn("output_norm", kept)
        self.assertIn("-32b -out=../masters/stack_linear", kept)
        # The framing chosen after the registration carries the choice.
        self.assertNotIn("output_norm", dict(astro_sync_stack.siril_scripts(plan, "full", False)[0])["integration"])


# The end-to-end frames are wider than the unit ones: Siril needs a dozen stars to align them.
WIDE, EDGE = 240, 13
WIDE_SECTION = f"[1:{WIDE}, :]"


def overscan_floor() -> float:
    return astro_sync_stack.NORMALISATION_FLOOR


def sky(rows: int, columns: int, shift: tuple[int, int], seed: int) -> "np.ndarray":
    """Gaussian stars at fixed places, moved by ``shift`` (a dither)."""
    rng = np.random.default_rng(99)
    yy, xx = np.mgrid[0:rows, 0:columns]
    field = np.zeros((rows, columns))
    for _ in range(150):
        y, x, flux = rng.uniform(8, rows - 8), rng.uniform(8, WIDE - 8), rng.uniform(0.2, 1.0)
        field += flux * np.exp(-((yy - y - shift[0]) ** 2 + (xx - x - shift[1]) ** 2) / (2 * 1.6 ** 2))
    return field


@NEEDS_NUMPY
@unittest.skipIf(SIRIL is None, "needs siril-cli (Siril 1.4)")
class EndToEnd(unittest.TestCase):
    """A night of a CCD with an overscan, through Siril: the master has no band and its sky is near zero."""

    ROWS = 200

    def setUp(self) -> None:
        self.root = Path(tempfile.mkdtemp())
        if not os.environ.get("ASTRO_SYNC_KEEP_E2E"):
            self.addCleanup(lambda: shutil.rmtree(self.root, ignore_errors=True))

    def frames(self, section: str | None, dead_column: bool = False) -> None:
        columns = WIDE + EDGE
        rng = np.random.default_rng(5)
        header = {"EXPTIME": 30.0, "XBINNING": 1, "INSTRUME": "Synthetic CCD"}

        def save(folder: str, name: str, data: "np.ndarray", kind: str) -> None:
            (self.root / folder).mkdir(exist_ok=True)
            head = fits.Header()
            for key, value in {**header, "IMAGETYP": kind}.items():
                head[key] = value
            if section is not None:
                head["TRIMSEC"] = section
                head["BIASSEC"] = f"[{WIDE + 1}:{columns}]"
            fits.PrimaryHDU(np.clip(data, 0, 65535).astype(np.uint16), head).writeto(self.root / folder / name)

        for n in range(8):
            save("biases", f"bias_{n}.fit", rng.normal(1000, 3, (self.ROWS, columns)), "Bias")
        vignette = np.linspace(0.8, 1.0, WIDE)[None, :] * np.ones((self.ROWS, 1))
        for n in range(8):
            flat = np.full((self.ROWS, columns), 1000.0)
            flat[:, :WIDE] = 1000 + 20000 * vignette + rng.normal(0, 40, (self.ROWS, WIDE))
            flat[:, WIDE:] += rng.normal(0, 1.0, (self.ROWS, EDGE))  # the overscan: the bias level, not the light
            if dead_column:
                flat[:, 0] = 1000.0  # a column that sees no light: a master flat of ~0 inside the image
            save("flats", f"flat_{n}.fit", flat, "Flat")
        for n in range(6):
            shift = (int(rng.integers(-2, 3)), int(rng.integers(-2, 3)))
            light = np.full((self.ROWS, columns), 1000.0)
            light[:, :WIDE] = 1000 + 600 * vignette + 20000 * sky(self.ROWS, WIDE, shift, n)[:, :WIDE] * vignette
            light[:, WIDE:] += rng.normal(0, 1.0, (self.ROWS, EDGE))
            if dead_column:
                light[:, 0] = 990.0
            save("lights", f"light_{n}.fit", rng.poisson(np.clip(light, 0, None)), "Light")

    def stack(self, section: str | None, dead_column: bool = False) -> tuple[dict, "np.ndarray", subprocess.CompletedProcess]:
        self.frames(section, dead_column)
        result = subprocess.run(
            [sys.executable, str(HERE / "astro_sync_stack.py"), "--lights", str(self.root / "lights"),
             "--biases", str(self.root / "biases"), "--flats", str(self.root / "flats"),
             "--work", str(self.root / "work"), "--out", str(self.root / "out"), "--name", "N",
             "--integration", "F", "--linear-only", "--siril", SIRIL, "--json"],
            capture_output=True, text=True, timeout=900,
        )
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        record = json.loads((self.root / "out" / "N_stacking.json").read_text())
        return record, fits.getdata(self.root / "out" / "F_integration.fit"), result

    def test_the_master_has_no_band_and_its_sky_stays_near_zero(self) -> None:
        record, master, result = self.stack(WIDE_SECTION)
        self.assertEqual(record["overscan"]["trimmed"], True)
        self.assertLessEqual(master.shape[-1], WIDE)
        self.assertGreater(master.shape[-1], WIDE - 12)
        self.assertLess(float(np.median(master)), 0.01)
        notes = [json.loads(line) for line in result.stdout.splitlines() if line.startswith("{") and '"note"' in line]
        self.assertIn("overscan_trimmed", [note["code"] for note in notes])

    def test_a_calibrated_frame_far_below_zero_leaves_the_sky_where_it_is(self) -> None:
        # A column of the image the master flat does not see: the calibrated frames reach far below zero.  With
        # -output_norm the lowest pixel would be brought to 0 and the sky of the master would be lifted with it.
        record, master, result = self.stack(WIDE_SECTION, dead_column=True)
        self.assertFalse(record["normalisation"]["outputNorm"])
        self.assertLess(record["normalisation"]["lowestPixel"], overscan_floor())
        self.assertLess(float(np.median(master)), 0.01)
        notes = [json.loads(line) for line in result.stdout.splitlines() if line.startswith("{") and '"note"' in line]
        self.assertIn("normalisation_skipped", [note["code"] for note in notes])

    def test_a_clean_night_is_still_rescaled(self) -> None:
        record, _, _ = self.stack(WIDE_SECTION)
        self.assertTrue(record["normalisation"]["outputNorm"])
        self.assertGreater(record["normalisation"]["lowestPixel"], overscan_floor())

    def test_a_night_without_trimsec_is_not_touched(self) -> None:
        record, master, result = self.stack(None)
        self.assertIsNone(record["overscan"])
        self.assertGreater(master.shape[-1], WIDE)  # the whole frame, as before
        self.assertNotIn("overscan", result.stdout)


if __name__ == "__main__":
    unittest.main()
