"""Tests of the master kept of one family.

Run from this folder, with the stacking environment:
    python3 -m unittest -v test_integration_master
"""

from __future__ import annotations

import io
import json
import shutil
import tempfile
import unittest
from contextlib import redirect_stderr, redirect_stdout
from pathlib import Path

import astro_sync_stack
import integration_master

try:
    import numpy
    from astropy.io import fits
except ImportError:  # the stacking environment has them; a bare interpreter does not
    numpy = fits = None


class Options(unittest.TestCase):
    """``--integration`` is a plain stem, and the plan announces the master."""

    def test_the_family_must_be_a_plain_stem(self) -> None:
        base = ["--lights", "l", "--work", "w", "--out", "o", "--name", "M31"]
        args = astro_sync_stack.parse_args([*base, "--integration", "Ha_300s_g100"])
        self.assertEqual(args.integration, "Ha_300s_g100")
        for bad in ("../Ha", ".hidden", "a/b"):
            with self.assertRaises(SystemExit), redirect_stderr(io.StringIO()):
                astro_sync_stack.parse_args([*base, "--integration", bad])

    def test_the_master_is_announced_among_the_outputs(self) -> None:
        plan = astro_sync_stack.Plan(frames={"lights": []}, colour=True, pipelines=[], linear_only=True)
        self.assertEqual(
            astro_sync_stack.planned_outputs(plan, "M31_2026-10-07", "Ha_300s_g100"),
            ["M31_2026-10-07_linear.tif", "Ha_300s_g100_integration.fit", "M31_2026-10-07_stacking.json"],
        )
        self.assertNotIn("integration", " ".join(astro_sync_stack.planned_outputs(plan, "M31")))


@unittest.skipIf(fits is None, "needs numpy and astropy (the stacking environment)")
class ExposureUnit(unittest.TestCase):
    """The exposure of a frame in seconds: the rule of the Rust readers (``analysis::fits::exposure_seconds``)."""

    @staticmethod
    def header(*cards: tuple[str, object, str]) -> "fits.Header":
        header = fits.Header()
        for key, value, comment in cards:
            header[key] = (value, comment)
        return header

    def test_exptime_is_always_seconds(self) -> None:
        header = self.header(("EXPTIME", 300.0, ""), ("EXPOSURE", 10000, "[ms] Total Exposure Time"))
        self.assertEqual(integration_master.exposure_seconds(header), 300.0)

    def test_exposure_takes_the_unit_its_comment_states(self) -> None:
        self.assertEqual(integration_master.exposure_seconds(self.header(("EXPOSURE", 10000, "[ms] Total Exposure Time"))), 10.0)
        self.assertEqual(integration_master.exposure_seconds(self.header(("EXPOSURE", 10, "[s] Exposure"))), 10.0)
        self.assertEqual(integration_master.exposure_seconds(self.header(("EXPOSURE", 2500, "(ms) per frame"))), 2.5)

    def test_a_known_instrument_gives_the_unit_when_the_comment_does_not(self) -> None:
        header = self.header(("INSTRUME", "stellina-7ba7ce", ""), ("EXPOSURE", 10000, ""))
        self.assertEqual(integration_master.exposure_seconds(header), 10.0)
        stated = self.header(("INSTRUME", "stellina-7ba7ce", ""), ("EXPOSURE", 10, "[s] Exposure"))
        self.assertEqual(integration_master.exposure_seconds(stated), 10.0)

    def test_otherwise_it_is_seconds_and_no_exposure_is_none(self) -> None:
        self.assertEqual(integration_master.exposure_seconds(self.header(("EXPOSURE", 10, "Exposure time"))), 10.0)
        self.assertIsNone(integration_master.exposure_seconds(self.header(("GAIN", 100, ""))))
        self.assertEqual(integration_master.exposure_seconds({"EXPOSURE": 12}), 12.0)  # a plain mapping

    def test_the_facts_of_a_family_of_stellina_frames_say_ten_seconds(self) -> None:
        root = Path(tempfile.mkdtemp())
        self.addCleanup(lambda: shutil.rmtree(root, ignore_errors=True))
        lights = []
        for i in range(3):
            hdu = fits.PrimaryHDU(numpy.zeros((8, 8), numpy.uint16))
            hdu.header["INSTRUME"] = "stellina-7ba7ce"
            hdu.header["EXPOSURE"] = (10000, "[ms] Total Exposure Time")
            lights.append(root / f"img-{i}.fits")
            hdu.writeto(lights[-1])
        self.assertEqual(integration_master.frame_facts(lights)["exposureSeconds"], 10.0)


@unittest.skipIf(fits is None, "needs numpy and astropy (the stacking environment)")
class KeepMaster(unittest.TestCase):
    """The linear master copied beside the renders, its header completed."""

    def setUp(self) -> None:
        self.root = Path(tempfile.mkdtemp())
        self.addCleanup(lambda: shutil.rmtree(self.root, ignore_errors=True))

    def light(self, name: str, temperature: float) -> Path:
        header = fits.Header()
        for key, value in (("IMAGETYP", "Light"), ("FILTER", "L-eXtreme"), ("EXPTIME", 180.0),
                           ("GAIN", 100), ("XBINNING", 1), ("CCD-TEMP", temperature),
                           ("INSTRUME", "ZWO ASI2600MC Pro"), ("XPIXSZ", 3.76), ("FOCALLEN", 250.0)):
            header[key] = value
        path = self.root / "lights" / name
        path.parent.mkdir(exist_ok=True)
        fits.PrimaryHDU(numpy.zeros((4, 4), numpy.uint16), header).writeto(path)
        return path

    def test_the_master_is_kept_with_what_it_integrates(self) -> None:
        import integration_master

        lights = [self.light("a.fits", -10.0), self.light("b.fits", -10.2), self.light("c.fits", -9.0)]
        stacked = self.root / "stack_linear.fit"
        hdu = fits.PrimaryHDU(numpy.full((3, 5, 6), 0.25, numpy.float32))
        hdu.header["STACKCNT"] = 3
        hdu.writeto(stacked)
        out = self.root / "IC 1805" / "stacked"
        kept = integration_master.keep_master(stacked, out, "L-eXtreme_180s_g100", lights)

        master = out / "L-eXtreme_180s_g100_integration.fit"
        self.assertEqual(sorted(p.name for p in out.iterdir()), [master.name])  # no partial file left
        header = fits.getheader(master)
        self.assertEqual(header["ASFAMILY"], "L-eXtreme_180s_g100")
        self.assertEqual((header["FILTER"], header["EXPTIME"], header["STACKCNT"]), ("L-eXtreme", 180.0, 3))
        self.assertEqual(header["LIVETIME"], 540.0)
        self.assertEqual(header["CCD-TEMP"], -10.0)  # the median of the three
        self.assertEqual(header["INSTRUME"], "ZWO ASI2600MC Pro")
        # The optics of the frames travel with the master: a plan's image reads its pixel scale there.
        self.assertEqual((header["XPIXSZ"], header["FOCALLEN"]), (3.76, 250.0))
        self.assertNotIn("PIXSCALE", header)
        self.assertEqual(fits.getdata(master).dtype.kind, "f")  # still 32-bit linear
        self.assertEqual((kept["frames"], kept["totalSeconds"], kept["file"]), (3, 540.0, master.name))
        self.assertEqual(kept["bytes"], master.stat().st_size)
        self.assertEqual(len(kept["sha256"]), 64)

    def test_the_step_reports_the_master_and_a_new_stacking_replaces_it(self) -> None:
        lights = [self.light("a.fits", -10.0), self.light("b.fits", -10.0)]
        stacked = self.root / "stack_linear.fit"
        fits.PrimaryHDU(numpy.zeros((4, 4), numpy.float32)).writeto(stacked)
        out = self.root / "stacked"
        args = astro_sync_stack.argparse.Namespace(out=out, integration="F")
        plan = astro_sync_stack.Plan(frames={"lights": lights}, colour=False, pipelines=[])
        for _ in range(2):
            buffer = io.StringIO()
            with redirect_stdout(buffer):
                kept = astro_sync_stack.keep_integration(args, plan, astro_sync_stack.Reporter(True), stacked)
            event = json.loads(buffer.getvalue())
            self.assertEqual((event["event"], event["file"], event["frames"]), ("integration", "F_integration.fit", 2))
        self.assertEqual(sorted(p.name for p in out.iterdir()), ["F_integration.fit"])
        self.assertEqual(kept["totalSeconds"], 360.0)
        args.integration = None
        self.assertIsNone(astro_sync_stack.keep_integration(args, plan, astro_sync_stack.Reporter(True), stacked))


if __name__ == "__main__":
    unittest.main()


@unittest.skipIf(fits is None, "needs numpy and astropy (the stacking environment)")
class AssumedFilter(unittest.TestCase):
    """Lights whose headers name no filter (an ASIAIR writes none) take the one the caller records."""

    def setUp(self) -> None:
        self.root = Path(tempfile.mkdtemp())
        self.addCleanup(lambda: shutil.rmtree(self.root, ignore_errors=True))

    def light(self, name: str, **cards: object) -> Path:
        header = fits.Header()
        for key, value in {"IMAGETYP": "Light", "EXPTIME": 180.0, "GAIN": 100, **cards}.items():
            header[key] = value
        path = self.root / name
        fits.PrimaryHDU(numpy.zeros((4, 4), numpy.uint16), header).writeto(path)
        return path

    def test_the_master_carries_the_filter_the_frames_do_not_name_and_the_frames_stay_as_they_are(self) -> None:
        import integration_master

        lights = [self.light("a.fits"), self.light("b.fits")]
        before = [path.read_bytes() for path in lights]
        stacked = self.root / "stack.fit"
        hdu = fits.PrimaryHDU(numpy.full((3, 5, 6), 0.25, numpy.float32))
        hdu.header["STACKCNT"] = 2
        hdu.writeto(stacked)
        kept = integration_master.keep_master(stacked, self.root / "stacked", "Dualband_180s_g100", lights,
                                              assumed_filter="Dualband")
        self.assertEqual(kept["filter"], "Dualband")
        self.assertEqual(fits.getheader(self.root / "stacked" / "Dualband_180s_g100_integration.fit")["FILTER"], "Dualband")
        self.assertEqual([path.read_bytes() for path in lights], before)

    def test_a_filter_the_header_names_wins_over_the_assumed_one(self) -> None:
        import integration_master

        named = [self.light("c.fits", FILTER="UV/IR cut")]
        self.assertEqual(integration_master.frame_facts(named, assumed_filter="Dualband")["filter"], "UV/IR cut")
        self.assertIsNone(integration_master.frame_facts([self.light("d.fits")])["filter"])
