"""Tests of the crop of the image of a plan to the area every night covers (``plan_crop.py``, ``plan_stack.py``).

Run from this folder, with the stacking environment:
    python3 -m unittest -v test_plan_crop

``Rectangle``: the rectangle inscribed in the common area of synthetic masks (no rotation: nothing cut; a turned
night; no common area).  ``RealVeil``: when ``ASTRO_SYNC_VEIL_JOB`` names a job of the three real masters of the Veil (Sh2-103, August
2026), the shares measured on them; skipped otherwise.  ``ThreeNights``: three synthetic colour masters -- a dual-band night, a broadband one, a
dual-band one taken with the camera turned by 39° -- made into the image of a plan by ``plan_stack.py`` ("No
processing", no Siril needed): the image is cut to the expected rectangle, holds no pixel without data, its WCS
moves with it, and a milestone of the progression on another frame is brought onto the plan's and cut the same.
"""

from __future__ import annotations

import io
import json
import os
import shutil
import tempfile
import unittest
from contextlib import redirect_stdout
from pathlib import Path

try:
    import numpy as np
    from astropy.io import fits

    import astroalign  # noqa: F401
except ImportError:  # the stacking environment has them; a bare interpreter does not
    np = fits = None

NEEDS_ENVIRONMENT = unittest.skipIf(np is None, "needs numpy, astropy and astroalign (the stacking environment)")


def turned(shape: tuple[int, int], angle: float, shift: tuple[float, float] = (0.0, 0.0)) -> "np.ndarray":
    """The pixels of a frame of ``shape`` covered by the same frame turned by ``angle`` degrees about its centre
    and moved by ``shift`` (rows, columns)."""
    yy, xx = np.mgrid[0:shape[0], 0:shape[1]]
    turn = np.radians(angle)
    dy, dx = yy - shape[0] / 2 - shift[0], xx - shape[1] / 2 - shift[1]
    u = dx * np.cos(turn) + dy * np.sin(turn)
    v = -dx * np.sin(turn) + dy * np.cos(turn)
    return (np.abs(u) < shape[1] / 2) & (np.abs(v) < shape[0] / 2)


@NEEDS_ENVIRONMENT
class Rectangle(unittest.TestCase):
    SHAPE = (300, 360)

    def test_masters_that_cover_the_whole_frame_cut_nothing(self) -> None:
        import plan_crop

        whole = np.ones(self.SHAPE, bool)
        for mode in (plan_crop.CLEAN, plan_crop.COMMON):
            crop = plan_crop.decide([whole, whole.copy(), whole.copy()], mode=mode)
            self.assertEqual((crop["applied"], crop["reason"], crop["mode"]), (False, plan_crop.ALREADY_WHOLE, mode))
            self.assertEqual((crop["top"], crop["left"], crop["height"], crop["width"]), (0, 0, *self.SHAPE))
            self.assertEqual((crop["keptShare"], crop["lost"]), (1.0, {"top": 0, "left": 0, "bottom": 0, "right": 0}))
            image = np.ones((3, *self.SHAPE), np.float32)
            self.assertIs(plan_crop.cut(image, crop), image)

    def test_a_night_moved_without_rotation_cuts_its_shift_in_the_common_mode_only(self) -> None:
        import plan_crop

        whole = np.ones(self.SHAPE, bool)
        moved = np.zeros(self.SHAPE, bool)
        moved[4:, :-7] = True
        crop = plan_crop.decide([whole, moved], mode=plan_crop.COMMON)
        self.assertTrue(crop["applied"])
        self.assertEqual((crop["top"], crop["left"], crop["height"], crop["width"]), (4, 0, self.SHAPE[0] - 4, self.SHAPE[1] - 7))
        self.assertEqual(crop["lost"], {"top": 4, "left": 0, "bottom": 0, "right": 7})
        # « Bords nets »: every pixel has data (the first night covers it all), nothing to cut.
        self.assertEqual(plan_crop.decide([whole, moved])["reason"], plan_crop.ALREADY_WHOLE)

    def test_clean_edges_cut_only_the_pixels_no_night_covers(self) -> None:
        import plan_combine
        import plan_crop

        # A night a little smaller and shifted (a broadband master), and a night turned by 39°: the union has
        # empty slivers along the left edge and in a corner.
        smaller = np.zeros(self.SHAPE, bool)
        smaller[3:, 12:-2] = True
        covers = [turned(self.SHAPE, 0.0, (0, 30)) & smaller, smaller, turned(self.SHAPE, 39.0)]
        union = covers[0] | covers[1] | covers[2]
        crop = plan_crop.decide(covers)
        self.assertEqual((crop["applied"], crop["reason"], crop["mode"]), (True, plan_crop.NO_EMPTY_PIXEL, plan_crop.CLEAN))
        top, left, height, width = (crop[k] for k in ("top", "left", "height", "width"))
        self.assertTrue(union[top:top + height, left:left + width].all(), "no pixel without data")
        exact = plan_combine.exact_rectangle(union)
        self.assertEqual(height * width, exact[2] * exact[3])
        # Far more of the field than the area every night covers.
        common = plan_crop.decide(covers, mode=plan_crop.COMMON)
        self.assertGreater(crop["keptShare"], 0.9)
        self.assertLess(common["keptShare"], 0.65)
        # The union given by the combination is the one taken.
        self.assertEqual(plan_crop.decide(covers, covered=union)["keptShare"], crop["keptShare"])

    def test_a_night_turned_by_39_degrees_gives_the_largest_rectangle_inside_every_footprint(self) -> None:
        import plan_combine
        import plan_crop

        covers = [np.ones(self.SHAPE, bool), turned(self.SHAPE, 0.0, (3, -5)), turned(self.SHAPE, 39.0)]
        top, left, height, width = plan_crop.inscribed_rectangle(covers)
        every = covers[0] & covers[1] & covers[2]
        self.assertTrue(every[top:top + height, left:left + width].all())
        # The largest one: the search on the whole mask finds no bigger rectangle.
        exact = plan_combine.exact_rectangle(every)
        self.assertEqual(height * width, exact[2] * exact[3])
        crop = plan_crop.decide(covers, mode=plan_crop.COMMON)
        self.assertEqual((crop["applied"], crop["reason"]), (True, plan_crop.COMMON_AREA))
        # About half of a frame of 6:5 turned by 39° (the corners of the turned frame cut the sides).
        self.assertTrue(0.4 < crop["keptShare"] < 0.65, crop["keptShare"])
        self.assertEqual(sum(crop["lost"][edge] for edge in ("top", "bottom")) + height, self.SHAPE[0])
        self.assertEqual(sum(crop["lost"][edge] for edge in ("left", "right")) + width, self.SHAPE[1])

    def test_no_common_area_and_no_mask_cut_nothing(self) -> None:
        import plan_crop

        left_half, right_half = np.zeros(self.SHAPE, bool), np.zeros(self.SHAPE, bool)
        left_half[:, :100], right_half[:, 200:] = True, True
        crop = plan_crop.decide([left_half, right_half], mode=plan_crop.COMMON)
        self.assertEqual((crop["applied"], crop["reason"], crop["commonShare"]), (False, plan_crop.NO_COMMON_AREA, 0.0))
        self.assertEqual((crop["height"], crop["width"]), self.SHAPE)
        self.assertEqual(plan_crop.inscribed_rectangle([]), (0, 0, 0, 0))
        self.assertIsNone(plan_crop.common_area([]))
        # No pixel with data at all: nothing to cut either.
        empty = np.zeros(self.SHAPE, bool)
        self.assertEqual(plan_crop.decide([empty, empty])["reason"], plan_crop.NO_COMMON_AREA)

    def test_a_common_area_too_small_keeps_the_whole_field_and_says_why(self) -> None:
        import plan_crop

        small = np.zeros(self.SHAPE, bool)
        small[100:160, 100:200] = True
        crop = plan_crop.decide([np.ones(self.SHAPE, bool), small], mode=plan_crop.COMMON)
        self.assertEqual((crop["applied"], crop["reason"]), (False, plan_crop.TOO_SMALL))
        self.assertAlmostEqual(crop["commonShare"], 60 * 100 / (300 * 360), places=4)

    def test_the_whole_field_kept_on_request_and_the_mode_of_a_job(self) -> None:
        import plan_crop

        crop = plan_crop.decide([np.ones(self.SHAPE, bool), turned(self.SHAPE, 39.0)], mode=plan_crop.WHOLE)
        self.assertEqual((crop["applied"], crop["reason"], crop["keptShare"]), (False, plan_crop.WHOLE_FIELD, 1.0))
        self.assertEqual(plan_crop.mode_of({}), plan_crop.CLEAN)
        self.assertEqual(plan_crop.mode_of({"planField": "common"}), plan_crop.COMMON)
        self.assertEqual(plan_crop.mode_of({"planField": "whole"}), plan_crop.WHOLE)
        with self.assertRaises(ValueError):
            plan_crop.mode_of({"planField": "round"})

    def test_the_rectangle_of_the_plan_is_taken_on_a_frame_of_its_size_only(self) -> None:
        import plan_crop

        covers = [np.ones(self.SHAPE, bool)]  # a milestone of one night, which would cut nothing
        given = {"top": 20, "left": 30, "height": 200, "width": 250, "frameHeight": 300, "frameWidth": 360}
        crop = plan_crop.decide(covers, given=given)
        self.assertEqual((crop["applied"], crop["reason"]), (True, plan_crop.PLAN_RECTANGLE))
        self.assertEqual((crop["top"], crop["left"], crop["height"], crop["width"]), (20, 30, 200, 250))
        # A rectangle measured on another frame: the milestone measures its own.
        crop = plan_crop.decide(covers, given={**given, "frameWidth": 400})
        self.assertEqual(crop["reason"], plan_crop.ALREADY_WHOLE)
        # The whole frame of the plan (nothing was cut there): nothing is cut here either.
        crop = plan_crop.decide([turned(self.SHAPE, 20.0)], given={**given, "top": 0, "left": 0, "height": 300, "width": 360})
        self.assertEqual((crop["applied"], crop["reason"]), (False, plan_crop.ALREADY_WHOLE))

    def test_the_cut_and_the_wcs(self) -> None:
        import plan_crop

        crop = plan_crop.record_of((10, 20, 100, 200), self.SHAPE, True, plan_crop.COMMON_AREA)
        planes = np.arange(3 * 300 * 360, dtype=np.float32).reshape(3, 300, 360)
        self.assertEqual(plan_crop.cut(planes, crop).shape, (3, 100, 200))
        self.assertEqual(plan_crop.cut(planes, crop)[1, 0, 0], planes[1, 10, 20])
        self.assertEqual(plan_crop.cut(np.moveaxis(planes, 0, -1), crop).shape, (100, 200, 3))
        self.assertEqual(plan_crop.cut(planes[0], crop).shape, (100, 200))
        header = fits.Header()
        self.assertFalse(plan_crop.shift_wcs(header, crop))
        header["CRPIX1"], header["CRPIX2"] = 180.5, 150.5
        self.assertTrue(plan_crop.shift_wcs(header, crop))
        self.assertEqual((header["CRPIX1"], header["CRPIX2"]), (160.5, 140.5))


@NEEDS_ENVIRONMENT
class ThreeNights(unittest.TestCase):
    """The plan of the report: nights 1 and 3 dual-band, night 2 broadband, night 3 with the camera turned by 39°."""

    def setUp(self) -> None:
        from test_dualband_reference import SHAPE, sky

        self.shape = SHAPE
        self.root = Path(tempfile.mkdtemp())
        self.addCleanup(lambda: shutil.rmtree(self.root, ignore_errors=True))
        wcs = {"CTYPE1": "RA---TAN", "CTYPE2": "DEC--TAN", "CRVAL1": 310.0, "CRVAL2": 43.0,
               "CRPIX1": SHAPE[1] / 2 + 0.5, "CRPIX2": SHAPE[0] / 2 + 0.5,
               "CD1_1": -0.0004, "CD1_2": 0.0, "CD2_1": 0.0, "CD2_2": 0.0004}
        self.paths = []
        for number, (seed, angle, shift, colour, cloud, keys) in enumerate((
                (1, 0.0, (0.0, 0.0), (1.0, 0.35, 0.35), 0.4, wcs),
                (2, 0.0, (3.0, -4.0), (1.0, 1.0, 1.0), 0.0, {}),
                (3, 39.0, (0.0, 0.0), (1.0, 0.35, 0.35), 0.4, {})), 1):
            header = fits.Header()
            for key, value in keys.items():
                header[key] = value
            path = self.root / f"night{number}" / "stacked" / "integration.fit"
            path.parent.mkdir(parents=True)
            fits.PrimaryHDU(sky(seed, shift=shift, angle=angle, colour=colour, cloud=cloud), header).writeto(path)
            self.paths.append(path)

    def master(self, number: int) -> dict[str, object]:
        dual = number != 2
        return {"path": str(self.paths[number - 1]), "nightId": f"n-{number}", "eveningDate": f"2026-08-{6 + number:02d}",
                "filter": "Dual-band" if dual else "Broadband", "band": "dualband" if dual else "broadband",
                "frameCount": [120, 89, 100][number - 1], "totalSeconds": [21600, 8010, 18000][number - 1]}

    def run_plan(self, numbers: list[int], name: str, **changes: object) -> tuple[int, list[dict], dict, Path]:
        import plan_stack

        plan = self.root / name
        job = {"format": 1, "projectId": "p", "targetId": "sharpless-103", "name": "Sh2-103_plan",
               "out": str(plan / "stacked"), "work": str(plan / ".astro-sync-work" / "plan-image"), "reference": 0,
               "pipeline": {"pipelineId": "linear", "trustedSha256": None},
               "masters": [self.master(n) for n in numbers], **changes}
        job_file = self.root / f"{name}.job.json"
        job_file.write_text(json.dumps(job), encoding="utf-8")
        buffer = io.StringIO()
        with redirect_stdout(buffer):
            code = plan_stack.main(["--job", str(job_file), "--json", "--keep-work"])
        events = [json.loads(line) for line in buffer.getvalue().splitlines() if line.startswith("{")]
        record_file = plan / "stacked" / "Sh2-103_plan.json"
        record = json.loads(record_file.read_text()) if record_file.is_file() else {}
        return code, events, record, plan

    def linear(self, plan: Path) -> "np.ndarray":
        import tifffile

        return tifffile.imread(plan / "stacked" / "Sh2-103_plan_linear.tif")

    def test_by_default_the_edges_without_data_are_cut_and_nothing_else(self) -> None:
        code, events, record, plan = self.run_plan([1, 2, 3], "clean")
        self.assertEqual(code, 0, events)
        crop = next(e for e in events if e["event"] == "crop")
        self.assertEqual((crop["mode"], crop["applied"], crop["reason"]), ("clean", True, "noEmptyPixel"))
        # The broadband night, shifted by a few pixels, leaves a thin band without data: only that goes.
        self.assertGreater(crop["keptShare"], 0.95)
        image = self.linear(plan)
        self.assertEqual(image.shape[:2], (crop["height"], crop["width"]))
        self.assertTrue((image > 0).any(axis=-1).all())
        self.assertEqual(record["combination"]["noData"], 0.0)
        # The parts the turned night misses stay: the exposure is not the same everywhere.
        self.assertGreater(record["coverage"]["degradedShare"], 0.1)
        code, events, whole_record, whole_plan = self.run_plan([1, 2, 3], "whole", planField="whole")
        self.assertEqual(code, 0, events)
        top, left = crop["top"], crop["left"]
        np.testing.assert_array_equal(self.linear(whole_plan)[top:top + crop["height"], left:left + crop["width"]], image)

    def test_the_common_area_is_what_the_three_nights_cover(self) -> None:
        code, events, record, plan = self.run_plan([1, 2, 3], "all", planField="common")
        self.assertEqual(code, 0, events)
        crop = next(e for e in events if e["event"] == "crop")
        self.assertEqual((crop["mode"], crop["applied"], crop["reason"]), ("common", True, "commonArea"))
        self.assertEqual(record["crop"], {k: v for k, v in crop.items() if k != "event"})
        # The expected rectangle: the one inscribed in the three footprints drawn from their known placement.
        import plan_crop

        footprints = [np.ones(self.shape, bool), turned(self.shape, 0.0, (3.0, -4.0)), turned(self.shape, 39.0)]
        expected = plan_crop.inscribed_rectangle(footprints)
        # Of the same size (the largest rectangle of a turned area is one of many of nearly that size, placed a
        # little differently), and inside the three footprints but for the few pixels of the resampled edges.
        self.assertAlmostEqual(crop["height"] * crop["width"], expected[2] * expected[3], delta=0.03 * expected[2] * expected[3])
        every = plan_crop.common_area(footprints)
        inside = every[crop["top"]:crop["top"] + crop["height"], crop["left"]:crop["left"] + crop["width"]]
        self.assertGreater(float(inside.mean()), 0.995)
        image = self.linear(plan)
        self.assertEqual(image.shape[:2], (crop["height"], crop["width"]))
        # No bevel: every pixel of the image has data.
        self.assertTrue((image > 0).any(axis=-1).all())
        self.assertEqual(record["combination"]["noData"], 0.0)
        self.assertEqual(record["coverage"]["degradedShare"], 0.0)
        self.assertEqual(record["coverage"]["framings"], [])
        self.assertEqual(record["frameMaster"]["nightId"], "n-1")
        # The WCS of the reference night moves with the cut.
        header = fits.getheader(plan / ".astro-sync-work" / "plan-image" / "masters" / "plan_linear.fit")
        self.assertEqual((header["NAXIS1"], header["NAXIS2"]), (crop["width"], crop["height"]))
        self.assertAlmostEqual(header["CRPIX1"], self.shape[1] / 2 + 0.5 - crop["left"])
        self.assertAlmostEqual(header["CRPIX2"], self.shape[0] / 2 + 0.5 - crop["top"])
        self.assertTrue(record["crop"]["wcsShifted"])
        # « Garder tout le champ »: the union, with its black corners, and the same pixels inside the rectangle.
        code, events, whole_record, whole_plan = self.run_plan([1, 2, 3], "whole", planField="whole")
        self.assertEqual(code, 0, events)
        self.assertEqual((whole_record["crop"]["applied"], whole_record["crop"]["reason"]), (False, "wholeField"))
        whole = self.linear(whole_plan)
        self.assertEqual(whole.shape[:2], self.shape)
        top, left = crop["top"], crop["left"]
        np.testing.assert_array_equal(whole[top:top + crop["height"], left:left + crop["width"]], image)
        self.assertGreater(whole_record["coverage"]["degradedShare"], 0.1)

    def test_a_milestone_on_another_frame_is_brought_onto_the_plan_and_cut_the_same(self) -> None:
        code, events, record, _ = self.run_plan([1, 2, 3], "all", planField="common")
        self.assertEqual(code, 0, events)
        # The milestones of a plan whose longest night came last: nights 2 and 3 alone are on night 3's frame.
        code, events, own, _ = self.run_plan([2, 3], "own", planField="common")
        self.assertEqual(code, 0, events)
        self.assertEqual(own["frameMaster"]["nightId"], "n-3")
        code, events, milestone, plan = self.run_plan([2, 3], "milestone", planField="common", crop=record["crop"],
                                                      frameMaster=record["frameMaster"])
        self.assertEqual(code, 0, events)
        crop = milestone["crop"]
        self.assertEqual((crop["applied"], crop["reason"]), (True, "plan"))
        self.assertEqual([crop[k] for k in ("top", "left", "height", "width")],
                         [record["crop"][k] for k in ("top", "left", "height", "width")])
        self.assertAlmostEqual(abs(crop["onPlanFrame"]["rotationDegrees"]), 39.0, delta=0.5)
        self.assertEqual(milestone["frameMaster"]["nightId"], "n-1")
        image = self.linear(plan)
        self.assertEqual(image.shape[:2], (record["crop"]["height"], record["crop"]["width"]))
        # On the plan's frame: the stars of the milestone are where the plan's are.
        import dualband_reference

        plan_image = self.linear(self.root / "all").astype(np.float32)
        transform, _ = astroalign.find_transform(dualband_reference.alignment_view(image.astype(np.float32)),
                                                 dualband_reference.alignment_view(plan_image))
        self.assertLess(abs(np.degrees(transform.rotation)), 0.2)
        self.assertLess(float(np.hypot(*transform.translation)), 1.0)



VEIL_JOB = os.environ.get("ASTRO_SYNC_VEIL_JOB")


@NEEDS_ENVIRONMENT
@unittest.skipIf(not VEIL_JOB, "needs ASTRO_SYNC_VEIL_JOB: a job naming the three real masters of the Veil")
class RealVeil(unittest.TestCase):
    """The plan of the report on its real masters: two dual-band nights, one broadband night a little smaller,
    the third night turned by 39°."""

    def run_plan(self, mode: str) -> dict:
        import plan_stack

        root = Path(tempfile.mkdtemp())
        self.addCleanup(lambda: shutil.rmtree(root, ignore_errors=True))
        job = json.loads(Path(VEIL_JOB).read_text(encoding="utf-8"))
        job.update(out=str(root / "stacked"), work=str(root / "work"), planField=mode,
                   pipeline={"pipelineId": "linear", "trustedSha256": None})
        job_file = root / "job.json"
        job_file.write_text(json.dumps(job), encoding="utf-8")
        with redirect_stdout(io.StringIO()):
            self.assertEqual(plan_stack.main(["--job", str(job_file), "--json"]), 0)
        return json.loads(next((root / "stacked").glob("*_plan.json")).read_text())

    def test_clean_edges_keep_97_percent_and_no_empty_pixel(self) -> None:
        record = self.run_plan("clean")
        crop = record["crop"]
        self.assertEqual((crop["applied"], crop["reason"]), (True, "noEmptyPixel"))
        self.assertAlmostEqual(crop["keptShare"], 0.971, delta=0.002)
        self.assertEqual(record["combination"]["noData"], 0.0)

    def test_the_common_area_keeps_half_of_the_field(self) -> None:
        record = self.run_plan("common")
        self.assertAlmostEqual(record["crop"]["keptShare"], 0.502, delta=0.002)
        self.assertEqual(record["coverage"]["degradedShare"], 0.0)


if __name__ == "__main__":
    unittest.main()
