"""Tests of the frame of the stacking of a night (``night_framing.py``).

Run from this folder, with the stacking environment:
    python3 -m unittest -v test_night_framing

The first cases need numpy and astropy.  The end-to-end cases run Siril (``siril-cli`` on the PATH, or
``ASTRO_SYNC_SIRIL``) on synthetic nights: Automatic is the framing of before (non-regression), a notch is cut
from the whole field without stacking again, and the crop of the whole field is the stacking of the same
rectangle (the equivalence the frame relies on).
"""

from __future__ import annotations

import argparse
import json
import math
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

    import night_framing
    import registration_framing
    import siril_config
    from synthetic_nights import SHAPE, sky_around_the_pointing, write_night
except ImportError:  # the stacking environment has them; a bare interpreter does not
    np = fits = None

NEEDS_NUMPY = unittest.skipIf(np is None, "needs numpy and astropy (the stacking environment)")
SIRIL = os.environ.get("ASTRO_SYNC_SIRIL") or shutil.which("siril-cli")


def turned(degrees: float, shape: tuple[int, int], shift: tuple[float, float] = (0.0, 0.0)) -> tuple[float, ...]:
    """Siril's matrix of a frame turned by ``degrees`` about the centre of a frame of ``shape``, then shifted."""
    height, width = shape
    c, s = math.cos(math.radians(degrees)), math.sin(math.radians(degrees))
    cx, cy = width / 2, height / 2
    return (c, -s, cx - c * cx + s * cy + shift[0], s, c, cy - s * cx - c * cy + shift[1], 0.0, 0.0, 1.0)


@NEEDS_NUMPY
class Notches(unittest.TestCase):
    SHAPE = (400, 600)

    def report(self, angles: list[float], requested: str | int = "auto", progressive: bool = True) -> dict:
        return night_framing.frame_report([turned(a, self.SHAPE) for a in angles], self.SHAPE, progressive, requested)

    def test_frames_that_do_not_move_cover_everything(self) -> None:
        count = night_framing.coverage([turned(0.0, self.SHAPE)] * 3, self.SHAPE, 4)
        self.assertTrue((count == 3).all())

    def test_a_frame_shifted_leaves_a_band_to_the_others(self) -> None:
        # Shifted by 100 px to the right: the first sixth of the columns is covered by the first frame only.
        count = night_framing.coverage([turned(0.0, self.SHAPE), turned(0.0, self.SHAPE, (100.0, 0.0))], self.SHAPE, 1)
        self.assertTrue((count[:, :100] == 1).all())
        self.assertTrue((count[:, 100:] == 2).all())

    def test_the_y_of_siril_counts_from_the_bottom(self) -> None:
        # Shifted by 50 in Siril's y: the last 50 rows astropy reads (Siril's first 50) lose it.
        count = night_framing.coverage([turned(0.0, self.SHAPE), turned(0.0, self.SHAPE, (0.0, 50.0))], self.SHAPE, 1)
        self.assertTrue((count[-50:] == 1).all())
        self.assertTrue((count[:-50] == 2).all())

    def test_five_nested_notches_from_the_common_area_to_the_whole_field(self) -> None:
        stops = self.report([0.0, 15.0, 30.0, 45.0])["notches"]
        self.assertEqual([stop["rank"] for stop in stops], [0, 1, 2, 3, 4])
        self.assertEqual((stops[-1]["top"], stops[-1]["left"], stops[-1]["height"], stops[-1]["width"]), (0, 0, 400, 600))
        self.assertEqual(stops[-1]["shareOfFrame"], 1.0)
        # Every frame covers the whole of the common area; the share falls as the frame grows.
        self.assertEqual((stops[0]["fullShare"], stops[0]["lowestFrames"]), (1.0, 4))
        shares = [stop["fullShare"] for stop in stops]
        self.assertEqual(shares, sorted(shares, reverse=True))
        self.assertLess(stops[-1]["fullShare"], 1.0)
        self.assertEqual(stops[-1]["lowestFrames"], 1)
        for inner, outer in zip(stops, stops[1:]):
            self.assertLessEqual(outer["top"], inner["top"])
            self.assertLessEqual(outer["left"], inner["left"])
            self.assertGreaterEqual(outer["top"] + outer["height"], inner["top"] + inner["height"])
            self.assertGreaterEqual(outer["left"] + outer["width"], inner["left"] + inner["width"])

    def test_the_common_area_is_covered_by_every_frame_pixel_by_pixel(self) -> None:
        matrices = [turned(a, self.SHAPE) for a in (0.0, 20.0, 40.0)]
        common = self.report([0.0, 20.0, 40.0])["notches"][0]
        exact = night_framing.coverage(matrices, self.SHAPE, 1)
        inside = exact[common["top"]:common["top"] + common["height"], common["left"]:common["left"] + common["width"]]
        self.assertTrue((inside == 3).all())

    def test_automatic_keeps_the_whole_field_of_a_field_that_turns(self) -> None:
        report = self.report([0.0, 30.0])
        self.assertEqual((report["automatic"], report["rank"], report["requested"]), (4, None, "auto"))
        self.assertIsNone(night_framing.chosen_rectangle(report))
        self.assertIsNone(self.report([0.0, 30.0], 4)["rank"])  # the whole field asked for: nothing to cut

    def test_a_notch_asked_for_is_cut(self) -> None:
        report = self.report([0.0, 30.0], 1)
        self.assertEqual(report["rank"], 1)
        stop = report["notches"][1]
        self.assertEqual(night_framing.chosen_rectangle(report),
                         {key: stop[key] for key in ("top", "left", "height", "width")})

    def test_a_field_that_does_not_turn_offers_nothing(self) -> None:
        # Automatic is the common area of Siril's stacking, as before: no notch, nothing cut, whatever is asked.
        report = self.report([0.0, 0.5], 0, progressive=False)
        self.assertEqual((report["notches"], report["automatic"], report["rank"]), ([], None, None))
        self.assertEqual((report["frameHeight"], report["frameWidth"]), self.SHAPE)

    def test_no_area_every_frame_covers_offers_nothing(self) -> None:
        far = [turned(0.0, self.SHAPE), turned(0.0, self.SHAPE, (700.0, 0.0))]
        self.assertEqual(night_framing.frame_report(far, self.SHAPE, True, 0)["notches"], [])

    def test_the_option_takes_auto_or_a_rank(self) -> None:
        self.assertEqual(night_framing.frame_size_arg("auto"), "auto")
        self.assertEqual(night_framing.frame_size_arg("3"), 3)
        for wrong in ("5", "-1", "half"):
            with self.assertRaises(argparse.ArgumentTypeError):
                night_framing.frame_size_arg(wrong)


@NEEDS_NUMPY
class Crop(unittest.TestCase):
    def test_the_crop_is_brought_back_to_the_range_of_a_stacking(self) -> None:
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / "stack_linear.fit"
            data = np.linspace(0.0, 1.0, 3 * 20 * 30, dtype=np.float32).reshape(3, 20, 30)
            fits.PrimaryHDU(data, fits.Header({"STACKCNT": 12})).writeto(path)
            night_framing.crop_master(path, path, {"top": 5, "left": 4, "height": 10, "width": 20}, True)
            with fits.open(path) as hdul:
                cut, header = hdul[0].data, hdul[0].header
        self.assertEqual(cut.shape, (3, 10, 20))
        self.assertAlmostEqual(float(cut.min()), 0.0, places=6)
        self.assertAlmostEqual(float(cut.max()), 1.0, places=6)
        self.assertEqual((header["STACKCNT"], header["ASFRAME"]), (12, "5,4,10,20"))
        # The order of the values is kept, the planes brought back together (not each to its own range).
        expected = data[..., 5:15, 4:24]
        expected = (expected - expected.min()) / (expected.max() - expected.min())
        self.assertTrue(np.allclose(cut, expected, atol=1e-6))

    def test_a_stacking_left_in_the_units_of_its_frames_is_only_cut(self) -> None:
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / "stack_linear.fit"
            data = np.arange(20 * 30, dtype=np.float32).reshape(20, 30)
            fits.PrimaryHDU(data).writeto(path)
            night_framing.crop_master(path, path, {"top": 2, "left": 3, "height": 4, "width": 5}, False)
            self.assertTrue(np.array_equal(fits.getdata(path), data[2:6, 3:8]))


@NEEDS_NUMPY
@unittest.skipIf(SIRIL is None, "needs siril-cli (Siril 1.4)")
class EndToEnd(unittest.TestCase):
    """Synthetic mono frames stacked by the real script and Siril."""

    def setUp(self) -> None:
        self.root = Path(tempfile.mkdtemp())
        if not os.environ.get("ASTRO_SYNC_KEEP_E2E"):
            self.addCleanup(lambda: shutil.rmtree(self.root, ignore_errors=True))
        self.sky = sky_around_the_pointing()

    def stack(self, lights: Path, *extra: str) -> tuple[int, list[dict], dict | None]:
        out = lights.parent / "stacked"
        result = subprocess.run(
            [sys.executable, str(HERE / "astro_sync_stack.py"), "--lights", str(lights),
             "--work", str(lights.parent.parent / ".work"), "--out", str(out), "--name", "M45",
             "--linear-only", "--siril", SIRIL, "--json", *extra],
            capture_output=True, text=True, timeout=900,
        )
        events = [json.loads(line) for line in result.stdout.splitlines()]
        provenance = out / "M45_stacking.json"
        return result.returncode, events, json.loads(provenance.read_text()) if provenance.exists() else None

    def test_automatic_is_the_framing_of_before(self) -> None:
        # Non-regression: an equatorial night keeps the common area of Siril's stacking, nothing kept beside it.
        lights = write_night(self.root, "equatorial", self.sky, [0.0, 0.05, -0.08, 0.1, 0.02, -0.03], SHAPE, 0.4)
        code, events, provenance = self.stack(lights)
        self.assertEqual(code, 0, events[-1])
        registration = next(e for e in events if e["event"] == "registration")
        self.assertEqual(registration["framing"], "common")
        self.assertEqual((registration["frame"]["notches"], registration["frame"]["rank"]), ([], None))
        self.assertTrue(any("-framing=min" in line for line in provenance["recipe"]["siril"]))
        self.assertLess(provenance["width"], SHAPE[1])
        self.assertFalse((lights.parent / "stacked" / "M45_field.fit").exists())
        # An alt-azimuth night keeps the whole field, the notches measured, nothing cut.
        lights = write_night(self.root, "altaz", self.sky, [-10.0 * n for n in range(8)])
        code, events, provenance = self.stack(lights)
        self.assertEqual(code, 0, events[-1])
        registration = next(e for e in events if e["event"] == "registration")
        self.assertEqual((registration["framing"], registration["frame"]["automatic"]), ("full", 4))
        self.assertEqual(len(registration["frame"]["notches"]), 5)
        self.assertIsNone(registration["frame"]["rank"])
        self.assertEqual((provenance["width"], provenance["height"]), (SHAPE[1], SHAPE[0]))
        self.assertEqual(provenance["registration"]["frame"], registration["frame"])
        self.assertFalse(any(e["event"] == "note" and e["code"] == "frame_cut" for e in events))

    def test_a_notch_is_cut_from_the_whole_field_without_stacking_again(self) -> None:
        lights = write_night(self.root, "altaz", self.sky, [-10.0 * n for n in range(8)])
        code, events, _ = self.stack(lights)
        self.assertEqual(code, 0, events[-1])
        field = lights.parent / "stacked" / "M45_field.fit"
        self.assertEqual(fits.getdata(field).shape, SHAPE)
        code, events, provenance = self.stack(lights, "--frame-size", "1")
        self.assertEqual(code, 0, events[-1])
        notes = [e["code"] for e in events if e["event"] == "note"]
        self.assertEqual(notes[:2], ["field_reused", "frame_cut"])
        # No Siril step: the render (here the linear export) only.
        self.assertEqual([e["step"] for e in events if e["event"] == "step"], ["linear_export"])
        stop = provenance["registration"]["frame"]["notches"][1]
        self.assertEqual(provenance["registration"]["frame"]["rank"], 1)
        self.assertEqual((provenance["width"], provenance["height"]), (stop["width"], stop["height"]))
        self.assertEqual(fits.getdata(field).shape, SHAPE)  # the whole field stays
        # Other poses: stacked again.
        (lights / "Light_007.fit").unlink()
        code, events, _ = self.stack(lights, "--frame-size", "1")
        self.assertEqual(code, 0, events[-1])
        self.assertNotIn("field_reused", [e["code"] for e in events if e["event"] == "note"])

    def test_the_crop_of_the_whole_field_is_the_stacking_of_that_rectangle(self) -> None:
        # The night stacked on the whole field, then on the common area Siril chooses (-framing=min): the crop
        # of the first at the place of the second, brought back to 0..1, is the second.
        lights = write_night(self.root, "turning", self.sky, [-2.5 * n for n in range(9)], SHAPE, 3.0)
        code, events, _ = self.stack(lights, "--keep-work")
        self.assertEqual(code, 0, events[-1])
        work = lights.parent.parent / ".work"
        whole = fits.getdata(work / "masters" / "stack_linear.fit").astype(np.float64)
        script = "\n".join(["requires 1.4.0", "cd process",
                            registration_framing.apply_command("light", registration_framing.COMMON_AREA),
                            sorted(work.glob("*_integration.ssf"))[0].read_text().splitlines()[-2]
                            .replace("stack_linear", "stack_common"), "close"]) + "\n"
        (work / "common.ssf").write_text(script, encoding="utf-8")
        subprocess.run(siril_config.command(SIRIL, work, work / "common.ssf"), capture_output=True, timeout=600)
        common = fits.getdata(work / "masters" / "stack_common.fit").astype(np.float64)
        height, width = common.shape
        best = max(((float(np.corrcoef(whole[y:y + height, x:x + width].ravel(), common.ravel())[0, 1]), y, x)
                    for y in range(SHAPE[0] - height + 1) for x in range(SHAPE[1] - width + 1)))
        _, top, left = best
        path = work / "masters" / "cut.fit"
        fits.PrimaryHDU(whole.astype(np.float32)).writeto(path)
        night_framing.crop_master(path, path, {"top": top, "left": left, "height": height, "width": width}, True)
        gap = np.abs(fits.getdata(path).astype(np.float64) - common)
        self.assertLess(float(gap.std()), 0.002)  # under 0.2 % of the range
        self.assertLess(float(np.percentile(gap, 99.9)), 0.02)
        shutil.rmtree(work, ignore_errors=True)


if __name__ == "__main__":
    unittest.main()
