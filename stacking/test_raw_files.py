"""Tests of the camera RAW files in the stacking.

Run from this folder, with the stacking environment:
    python3 -m unittest -v test_raw_files

The unit tests need numpy and astropy.  The end-to-end cases run Siril (``siril-cli`` on the PATH, or
``ASTRO_SYNC_SIRIL``) on a synthetic night of DNG files (``synthetic_nights.write_raw_night``) and are skipped
without it.  ``ASTRO_SYNC_RAW_SAMPLES`` (a folder of real camera files) adds a case that converts them.
"""

from __future__ import annotations

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

    import astro_sync_stack
    import pose_stats
    from synthetic_nights import sky_around_the_pointing, write_dng, write_night, write_raw_night
except ImportError:  # the stacking environment has them; a bare interpreter does not
    np = fits = astro_sync_stack = pose_stats = None

import raw_files

NEEDS_NUMPY = unittest.skipIf(np is None, "needs numpy and astropy (the stacking environment)")
SIRIL = os.environ.get("ASTRO_SYNC_SIRIL") or shutil.which("siril-cli")
NEEDS_SIRIL = unittest.skipIf(SIRIL is None, "needs siril-cli (Siril 1.4)")


class Recognition(unittest.TestCase):
    def test_the_nine_extensions_in_any_case_are_raw_and_a_fits_is_not(self) -> None:
        for name in ("a.cr2", "a.CR3", "a.Nef", "a.ARW", "a.dng", "a.raf", "a.PEF", "a.orf", "a.RW2"):
            self.assertTrue(raw_files.is_raw(Path(name)), name)
        for name in ("a.fit", "a.fits", "a.xisf", "a.jpg", "a.tif", "raw", "cr2"):
            self.assertFalse(raw_files.is_raw(Path(name)), name)

    def test_the_extensions_are_the_nine_of_the_core(self) -> None:
        import re

        source = (HERE.parent / "crates" / "astro-sync-core" / "src" / "raw_header.rs").read_text(encoding="utf-8")
        listed = re.search(r"pub const EXTENSIONS: \[&str; \d+\] = \[(.*?)\];", source, re.S)
        self.assertIsNotNone(listed)
        self.assertEqual(set(re.findall(r'"(\w+)"', listed.group(1))), {suffix.lstrip(".") for suffix in raw_files.RAW_SUFFIXES})

    def raf(self, folder: Path, pattern: bytes) -> Path:
        """A RAF reduced to its signature and its CFA header: a count of records (tag, size, bytes), big endian."""
        records = b"".join(tag.to_bytes(2, "big") + len(data).to_bytes(2, "big") + data
                           for tag, data in ((0x130, b"\x0c\x0c"), (0x131, pattern)))
        cfa = (2).to_bytes(4, "big") + records
        head = b"FUJIFILMCCD-RAW 0201".ljust(84, b"\0") + (120).to_bytes(4, "big") + (0).to_bytes(4, "big")
        head += (120).to_bytes(4, "big") + len(cfa).to_bytes(4, "big")
        path = folder / "frame.RAF"
        path.write_bytes(head.ljust(120, b"\0") + cfa)
        return path

    def test_an_x_trans_raf_is_told_by_the_36_bytes_of_its_pattern(self) -> None:
        folder = Path(tempfile.mkdtemp(prefix="raw-raf-"))
        self.addCleanup(shutil.rmtree, folder, ignore_errors=True)
        self.assertTrue(raw_files.is_xtrans(self.raf(folder, bytes([1] * 36))))
        self.assertFalse(raw_files.is_xtrans(self.raf(folder, bytes([1, 0, 2, 1]))))
        not_a_raf = folder / "frame.cr2"
        not_a_raf.write_bytes(b"x" * 200)
        self.assertFalse(raw_files.is_xtrans(not_a_raf))
        self.assertFalse(raw_files.is_xtrans(folder / "missing.raf"))

    @NEEDS_NUMPY
    def test_an_x_trans_night_is_debayered_at_every_conversion_and_calibrated_as_colour(self) -> None:
        frames = {"dark-flats": [], "biases": [], "flats": [Path("f.raf")], "darks": [Path("d.raf")], "lights": [Path("l.raf")]}
        plan = astro_sync_stack.Plan(frames=frames, colour=True, pipelines=[], linear_only=True, xtrans=True)
        text = "\n".join(script for _, script in astro_sync_stack.siril_scripts(plan)[0])
        for kind in ("flat", "dark", "light"):
            self.assertIn(f"convert {kind} -debayer -out=", text)
        calibrate = next(line for line in text.splitlines() if line.startswith("calibrate light"))
        self.assertNotIn("-cfa", calibrate)
        self.assertNotIn("-debayer", calibrate)
        # A Bayer night keeps the usual recipe: the mosaic is calibrated, then debayered.
        plan = astro_sync_stack.Plan(frames=frames, colour=True, pipelines=[], linear_only=True)
        text = "\n".join(script for _, script in astro_sync_stack.siril_scripts(plan)[0])
        self.assertIn("convert dark -out=", text)
        calibrate = next(line for line in text.splitlines() if line.startswith("calibrate light"))
        self.assertIn("-cfa -equalize_cfa -debayer", calibrate)

    def test_a_missing_siril_is_none_and_a_given_one_must_exist(self) -> None:
        self.assertIsNone(raw_files.find_siril(Path("/nonexistent/siril-cli")))


@NEEDS_NUMPY
class Plan(unittest.TestCase):
    """The plan of a stacking of RAW files, no Siril needed."""

    def setUp(self) -> None:
        self.scratch = Path(tempfile.mkdtemp(prefix="raw-plan-"))
        self.addCleanup(shutil.rmtree, self.scratch, ignore_errors=True)

    def night(self, **kwargs: object) -> Path:
        return write_raw_night(self.scratch, "night", sky_around_the_pointing(), [0.0, 1.0, 2.0, 3.0], **kwargs)

    def test_raw_files_are_frames_of_every_kind_and_the_night_is_colour(self) -> None:
        night = self.night()
        frames = astro_sync_stack.fits_files(night / "M 45" / "lights")
        self.assertEqual([path.suffix for path in frames], [".dng"] * 4)
        self.assertEqual(len(astro_sync_stack.fits_files(night / "calibration" / "darks")), 3)
        self.assertTrue(astro_sync_stack.is_colour(frames[0]))

    def test_a_listed_raw_light_is_accepted_and_a_jpeg_is_not(self) -> None:
        night = self.night()
        lights = night / "M 45" / "lights"
        listing = self.scratch / "list.txt"
        listing.write_text("\n".join(str(path) for path in sorted(lights.glob("*.dng"))[:3]) + "\n", encoding="utf-8")
        self.assertEqual(len(astro_sync_stack.listed_frames(listing, lights)), 3)
        (lights / "Light_9999.jpg").write_bytes(b"x")
        listing.write_text(str(lights / "Light_9999.jpg") + "\n", encoding="utf-8")
        with self.assertRaises(astro_sync_stack.StackingError) as caught:
            astro_sync_stack.listed_frames(listing, lights)
        self.assertEqual(caught.exception.code, "frames_outside_lights")

    def test_raw_and_fits_frames_are_never_stacked_together(self) -> None:
        raw = [Path("a.cr2"), Path("b.CR2")]
        fits_only = [Path("a.fit")]
        astro_sync_stack.check_one_format({"lights": raw, "darks": raw, "flats": []})
        astro_sync_stack.check_one_format({"lights": fits_only, "darks": fits_only})
        for frames in ({"lights": raw, "darks": fits_only}, {"lights": fits_only + raw}):
            with self.assertRaises(astro_sync_stack.StackingError) as caught:
                astro_sync_stack.check_one_format(frames)
            self.assertEqual(caught.exception.code, "mixed_formats")
            self.assertEqual(caught.exception.exit_code, astro_sync_stack.EXIT_INPUT)

    def test_a_raw_night_declares_no_overscan(self) -> None:
        import overscan

        night = self.night()
        frames = {"lights": astro_sync_stack.fits_files(night / "M 45" / "lights")}
        self.assertIsNone(overscan.trim_overscan(frames, self.scratch / "work-input", cfa=True))

    def test_the_plan_of_a_raw_night_names_the_format(self) -> None:
        night = self.night()
        arguments = astro_sync_stack.parse_args([
            "--lights", str(night / "M 45" / "lights"), "--darks", str(night / "calibration" / "darks"),
            "--flats", str(night / "calibration" / "flats"), "--biases", str(night / "calibration" / "biases"),
            "--work", str(self.scratch / "work"), "--out", str(self.scratch / "out"), "--name", "t", "--linear-only",
            "--dry-run", "--json",
        ])
        plan = astro_sync_stack.build_plan(arguments)
        fields = astro_sync_stack.plan_fields(plan, arguments)
        self.assertEqual(fields["lightFormat"], "raw")
        self.assertEqual(fields["frames"], {"dark-flats": 0, "biases": 3, "flats": 3, "darks": 3, "lights": 4})
        self.assertIs(fields["colourSensor"], True)
        self.assertEqual(fields["colour"], "none")

    def test_a_fits_night_is_still_a_fits_night(self) -> None:
        lights = write_night(self.scratch, "fits-night", sky_around_the_pointing(), [0.0, 1.0, 2.0], colour=(1.0, 0.9, 0.8))
        arguments = astro_sync_stack.parse_args([
            "--lights", str(lights), "--work", str(self.scratch / "work"), "--out", str(self.scratch / "out"),
            "--name", "t", "--linear-only", "--dry-run", "--json",
        ])
        fields = astro_sync_stack.plan_fields(astro_sync_stack.build_plan(arguments), arguments)
        self.assertEqual(fields["lightFormat"], "fits")


@NEEDS_NUMPY
@NEEDS_SIRIL
class WithSiril(unittest.TestCase):
    def setUp(self) -> None:
        self.scratch = Path(tempfile.mkdtemp(prefix="raw-siril-"))
        self.addCleanup(shutil.rmtree, self.scratch, ignore_errors=True)

    def test_the_converter_gives_a_mosaic_fits_and_removes_its_copies(self) -> None:
        night = write_raw_night(self.scratch, "night", sky_around_the_pointing(), [0.0, 1.0, 2.0])
        poses = sorted((night / "M 45" / "lights").glob("*.dng"))
        converter = raw_files.Converter(Path(SIRIL), self.scratch / "convert")
        converter.scratch.mkdir()
        first = converter.fits_of(poses[0], poses[1:])
        header = fits.getheader(first)
        self.assertEqual(str(header["BAYERPAT"]).strip(), "RGGB")
        self.assertEqual(header["NAXIS"], 2)
        self.assertAlmostEqual(float(header["EXPTIME"]), 30.0)
        self.assertEqual(float(header["ISOSPEED"]), 800.0)
        # The next poses were converted with it: no second run of Siril.
        self.assertEqual(converter.round, 1)
        self.assertIn(poses[1], converter.converted)
        converter.fits_of(poses[1])
        self.assertEqual(converter.round, 1)
        for pose in poses[:2]:
            converter.release(pose)
        converter.close()
        self.assertEqual(list(converter.scratch.iterdir()), [])
        # The poses themselves are untouched.
        self.assertTrue(all(pose.is_file() for pose in poses))

    def test_a_file_siril_cannot_read_is_an_error_naming_it(self) -> None:
        broken = self.scratch / "broken.cr2"
        broken.write_bytes(b"not a raw file at all")
        converter = raw_files.Converter(Path(SIRIL), self.scratch / "convert")
        converter.scratch.mkdir()
        with self.assertRaises(raw_files.ConversionError) as caught:
            converter.fits_of(broken)
        self.assertIn("broken.cr2", str(caught.exception))

    def test_without_siril_a_raw_pose_is_unreadable_and_says_why(self) -> None:
        night = write_raw_night(self.scratch, "night", sky_around_the_pointing(), [0.0, 1.0], darks=0, flats=0, biases=0)
        listing = self.scratch / "frames.txt"
        pose = next((night / "M 45" / "lights").glob("*.dng"))
        listing.write_text(f"{pose}\n", encoding="utf-8")
        environment = {**os.environ, "PATH": "/nonexistent"}
        result = subprocess.run([sys.executable, str(HERE / "pose_stats.py"), "--frames-from", str(listing),
                                 "--siril", "/nonexistent/siril-cli"], capture_output=True, text=True, env=environment)
        answer = json.loads(result.stdout.splitlines()[0])
        self.assertEqual(answer["status"], "unreadable")
        self.assertIn("Siril", answer["message"])

    def test_the_measure_of_a_raw_pose_reads_its_stars_and_leaves_the_date_to_the_caller(self) -> None:
        night = write_raw_night(self.scratch, "night", sky_around_the_pointing(), [0.0, 1.0, 2.0], darks=0, flats=0, biases=0)
        poses = sorted((night / "M 45" / "lights").glob("*.dng"))
        listing = self.scratch / "frames.txt"
        listing.write_text("".join(f"{pose}\n" for pose in poses), encoding="utf-8")
        result = subprocess.run([sys.executable, str(HERE / "pose_stats.py"), "--frames-from", str(listing),
                                 "--siril", SIRIL], capture_output=True, text=True)
        answers = [json.loads(line) for line in result.stdout.splitlines()]
        self.assertEqual([answer["path"] for answer in answers], [str(pose) for pose in poses])
        for answer in answers:
            self.assertEqual(answer["status"], "measured", answer)
            self.assertGreater(answer["starCount"], 20)
            self.assertIsNone(answer["dateObs"])
            self.assertAlmostEqual(answer["exposure"], 30.0)
            self.assertIsNotNone(answer["fwhmPixels"])
        # No converted copy is left beside the list.
        self.assertEqual(sorted(path.name for path in self.scratch.iterdir() if path.is_dir() and path.name.startswith("raw-poses-")), [])

    def stack(self, night: Path, extra: list[str] | None = None) -> tuple[int, list[dict]]:
        out = night / "M 45" / "stacked"
        command = [sys.executable, "-B", str(HERE / "astro_sync_stack.py"),
                   "--lights", str(night / "M 45" / "lights"), "--work", str(self.scratch / "work"), "--out", str(out),
                   "--name", "M45", "--integration", "family", "--linear-only", "--siril", SIRIL, "--json"]
        for kind in ("darks", "flats", "biases"):
            if (night / "calibration" / kind).is_dir():
                command += [f"--{kind}", str(night / "calibration" / kind)]
        result = subprocess.run(command + (extra or []), capture_output=True, text=True)
        return result.returncode, [json.loads(line) for line in result.stdout.splitlines() if line.startswith("{")]

    def test_a_raw_night_with_its_calibration_is_stacked_end_to_end(self) -> None:
        night = write_raw_night(self.scratch, "night", sky_around_the_pointing(), [0.0, 1.5, 3.0, 4.5, 6.0, 7.5])
        code, events = self.stack(night)
        self.assertEqual(code, 0, events[-3:])
        steps = [event["step"] for event in events if event["event"] == "step"]
        self.assertEqual(steps[:3], ["master_biases", "master_flats", "master_darks"])
        registration = next(event for event in events if event["event"] == "registration")
        self.assertEqual((registration["frames"], registration["registered"]), (6, 6))
        kept = next(event for event in events if event["event"] == "integration")
        self.assertEqual(kept["frames"], 6)
        # The master's header comes from the converted copies: the exposure, the ISO as the gain, the camera.
        self.assertAlmostEqual(kept["exposureSeconds"], 30.0)
        self.assertEqual(kept["gain"], 800.0)
        self.assertIn("Synthetic RAW 1", kept["sensor"])
        provenance = json.loads((night / "M 45" / "stacked" / "M45_stacking.json").read_text(encoding="utf-8"))
        self.assertEqual(provenance["lightFormat"], "raw")
        self.assertIs(provenance["colourSensor"], True)
        self.assertEqual(provenance["stackedFrames"], 6)
        self.assertEqual(provenance["frames"], {"dark-flats": 0, "biases": 3, "flats": 3, "darks": 3, "lights": 6})
        import tifffile

        image = tifffile.imread(night / "M 45" / "stacked" / "M45_linear.tif")
        self.assertEqual(image.shape[2], 3)
        self.assertGreater(float(image.max()), 0)
        # The RAW files are untouched.
        self.assertEqual(len(list((night / "M 45" / "lights").glob("*.dng"))), 6)

    def test_a_raw_night_without_calibration_is_debayered_at_the_conversion(self) -> None:
        night = write_raw_night(self.scratch, "night", sky_around_the_pointing(), [0.0, 1.5, 3.0, 4.5],
                                darks=0, flats=0, biases=0)
        code, events = self.stack(night)
        self.assertEqual(code, 0, events[-3:])
        steps = [event["step"] for event in events if event["event"] == "step"]
        self.assertNotIn("master_darks", steps)
        provenance = json.loads((night / "M 45" / "stacked" / "M45_stacking.json").read_text(encoding="utf-8"))
        recipe = " ".join(provenance["recipe"]["siril"])
        self.assertIn("convert light -debayer", recipe)

    def test_a_folder_that_mixes_raw_and_fits_is_refused_before_siril_runs(self) -> None:
        night = write_raw_night(self.scratch, "night", sky_around_the_pointing(), [0.0, 1.0, 2.0], darks=0, flats=0, biases=0)
        fits_lights = write_night(self.scratch, "other", sky_around_the_pointing(), [0.0, 1.0], colour=(1.0, 0.9, 0.8))
        shutil.copy(next(fits_lights.glob("*.fit")), night / "M 45" / "lights" / "Light_FITS.fit")
        code, events = self.stack(night)
        self.assertEqual(code, astro_sync_stack.EXIT_INPUT)
        self.assertEqual(events[-1]["event"], "error")
        self.assertEqual(events[-1]["code"], "mixed_formats")


SAMPLES = os.environ.get("ASTRO_SYNC_RAW_SAMPLES")


@NEEDS_NUMPY
@NEEDS_SIRIL
@unittest.skipIf(not SAMPLES, "needs ASTRO_SYNC_RAW_SAMPLES (a folder of real camera files)")
class RealCameraFiles(unittest.TestCase):
    def test_every_sample_converts_to_a_mosaic_and_is_measured(self) -> None:
        scratch = Path(tempfile.mkdtemp(prefix="raw-real-"))
        self.addCleanup(shutil.rmtree, scratch, ignore_errors=True)
        converter = raw_files.Converter(Path(SIRIL), scratch)
        failures = []
        for path in sorted(Path(SAMPLES).iterdir()):
            if not raw_files.is_raw(path):
                continue
            try:
                converted = converter.fits_of(path)
                header = fits.getheader(converted)
                self.assertGreater(int(header["NAXIS1"]), 1000, path.name)
                self.assertTrue(str(header.get("BAYERPAT", "")).strip(), path.name)
                answer = pose_stats.measure_pose(converted)
                self.assertEqual(answer["status"], "measured", path.name)
            except (raw_files.ConversionError, AssertionError) as error:
                failures.append(f"{path.name}: {error}")
            finally:
                converter.release(path)
        self.assertEqual(failures, [])
        raf = Path(SAMPLES) / "fuji-xe1.RAF"
        if raf.is_file():
            self.assertTrue(raw_files.is_xtrans(raf))
        for path in sorted(Path(SAMPLES).iterdir()):
            if raw_files.is_raw(path) and path.suffix.lower() != ".raf":
                self.assertFalse(raw_files.is_xtrans(path), path.name)


if __name__ == "__main__":
    unittest.main()
