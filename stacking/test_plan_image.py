"""Tests of the image of a multi-night plan.

Run from this folder, with the stacking environment:
    python3 -m unittest -v test_plan_image

The end-to-end case runs Siril (``siril-cli`` on the PATH, or
``ASTRO_SYNC_SIRIL``) on synthetic frames of two nights, and on two alt-azimuth nights oriented differently (``AltAzimuthNights``); it is
skipped without it.
"""

from __future__ import annotations

import io
import json
import os
import shutil
import subprocess
import sys
import tempfile
import unittest
from contextlib import redirect_stderr, redirect_stdout
from pathlib import Path

HERE = Path(__file__).resolve().parent

try:
    import numpy as np
    from astropy.io import fits
    from synthetic_nights import SHAPE, sky_around_the_pointing, write_night
except ImportError:  # the stacking environment has them; a bare interpreter does not
    np = fits = None

NEEDS_NUMPY = unittest.skipIf(np is None, "needs numpy and astropy (the stacking environment)")
SIRIL = os.environ.get("ASTRO_SYNC_SIRIL") or shutil.which("siril-cli")


def star_field(shape: tuple[int, int], stars: list[tuple[float, float, float]], sigma: float = 1.6):
    """A field of Gaussian stars ``(y, x, flux)`` on zero."""
    yy, xx = np.mgrid[0:shape[0], 0:shape[1]]
    field = np.zeros(shape, np.float64)
    for y, x, flux in stars:
        field += flux * np.exp(-((yy - y) ** 2 + (xx - x) ** 2) / (2 * sigma ** 2))
    return field


def random_stars(count: int, shape: tuple[int, int], seed: int) -> list[tuple[float, float, float]]:
    rng = np.random.default_rng(seed)
    return [(float(rng.uniform(12, shape[0] - 12)), float(rng.uniform(12, shape[1] - 12)), float(rng.uniform(0.2, 1.0)))
            for _ in range(count)]


def blob(shape: tuple[int, int], centre: tuple[int, int], radius: float):
    yy, xx = np.mgrid[0:shape[0], 0:shape[1]]
    return np.exp(-((yy - centre[0]) ** 2 + (xx - centre[1]) ** 2) / (2 * radius ** 2))


@NEEDS_NUMPY
class Contrast(unittest.TestCase):
    """``renders.presentation_contrast``: the S-curve of the print export, on the luminance."""

    @staticmethod
    def gradient():
        """A display image: a coloured gradient from sky to nebula, with a bright corner."""
        height, width = 120, 200
        ramp = np.linspace(0.02, 0.9, width, dtype=np.float32)[None, :].repeat(height, axis=0)
        rgb = np.stack([ramp * 0.9, ramp * 0.6, ramp * 1.0], axis=-1)
        return np.clip(rgb, 0, 1)

    def test_the_sky_level_does_not_move(self) -> None:
        import renders

        image = self.gradient()
        pivot = float(np.percentile(renders.lum(image), renders.PIVOT_PERCENTILE))
        for gain in renders.CONTRAST_GAINS.values():
            curved = renders.presentation_contrast(image, gain)
            self.assertAlmostEqual(float(np.percentile(renders.lum(curved), renders.PIVOT_PERCENTILE)), pivot, places=2)
            self.assertAlmostEqual(float(renders.s_curve(np.array([pivot]), pivot, gain)[0]), pivot, places=6)

    def test_the_luminance_is_spread_and_the_stronger_gain_spreads_more(self) -> None:
        import renders

        image = self.gradient()
        level = renders.lum(image)
        pivot = float(np.percentile(level, renders.PIVOT_PERCENTILE))
        spread = {}
        for name, gain in renders.CONTRAST_GAINS.items():
            curved = renders.lum(renders.presentation_contrast(image, gain))
            self.assertTrue(np.all(curved[level < pivot * 0.8] <= level[level < pivot * 0.8] + 1e-4))
            self.assertTrue(np.all(curved[(level > pivot * 1.2) & (level < 0.8)] >= level[(level > pivot * 1.2) & (level < 0.8)] - 1e-4))
            self.assertGreater(float(np.percentile(curved, 95) - np.percentile(curved, 5)),
                               float(np.percentile(level, 95) - np.percentile(level, 5)))
            spread[name] = float(curved.std())
        self.assertGreater(spread["strong"], spread["presentation"])
        self.assertEqual(renders.CONTRAST_GAINS, {"presentation": 2.0, "strong": 3.6})

    def test_the_colours_are_kept(self) -> None:
        import renders

        image = self.gradient()
        for gain in renders.CONTRAST_GAINS.values():
            curved = renders.presentation_contrast(image, gain)
            self.assertLessEqual(float(curved.max()), 1.0)
            body = renders.lum(image) > 0.05
            for channel in (0, 2):
                before = image[..., channel][body] / image[..., 1][body]
                after = curved[..., channel][body] / np.maximum(curved[..., 1][body], 1e-6)
                self.assertLess(float(np.max(np.abs(after - before))), 5e-3)

    def test_the_master_is_never_touched_and_the_linear_image_has_no_curve(self) -> None:
        import argparse
        import hashlib

        import astro_sync_stack as stack
        import linear_export

        root = Path(tempfile.mkdtemp())
        self.addCleanup(lambda: shutil.rmtree(root, ignore_errors=True))
        master = root / "stack_integration.fit"
        fits.PrimaryHDU(np.moveaxis(self.gradient(), -1, 0)).writeto(master)
        before = hashlib.sha256(master.read_bytes()).hexdigest()
        plain = linear_export.write_linear_tiff(master, root, "IC_1805")
        self.assertEqual(plain.name, "IC_1805_linear.tif")
        self.assertEqual(hashlib.sha256(master.read_bytes()).hexdigest(), before)
        # "No processing" has no contrast twin: the curve
        # does nothing visible on a linear image.
        plan = stack.Plan(frames={}, colour=True, pipelines=[], linear_only=True)
        for contrast in ("none", "presentation", "strong"):
            self.assertEqual(stack.planned_outputs(plan, "IC_1805", None, contrast),
                             ["IC_1805_linear.tif", "IC_1805_stacking.json"])
        args = argparse.Namespace(out=root / "out", name="IC_1805", contrast="strong")
        with redirect_stdout(io.StringIO()):
            outputs = stack.linear_outputs(args, stack.Reporter(False), stack.Runner(), master)
        self.assertEqual([o["file"] for o in outputs], ["IC_1805_linear.tif"])
        self.assertNotIn("contrast", outputs[0])
        self.assertEqual(sorted(p.name for p in (root / "out").iterdir()), ["IC_1805_linear.tif"])

    def test_the_images_of_a_render_come_in_pairs(self) -> None:
        import argparse

        import astro_sync_stack as stack
        import pipeline_defs

        available, _ = pipeline_defs.load_pipelines([])
        pipeline = available[stack.DEFAULT_PIPELINE]
        plan = stack.Plan(frames={}, colour=True, pipelines=[pipeline])
        files = stack.planned_outputs(plan, "N", None, "strong")
        self.assertIn(f"N_{pipeline.id}.tif", files)
        self.assertIn(f"N_{pipeline.id}_strong.tif", files)
        args = stack.parse_args(["--lights", ".", "--work", "w", "--out", "o", "--name", "N", "--contrast", "strong"])
        self.assertEqual(args.contrast, "strong")
        self.assertEqual(stack.parse_args(["--lights", ".", "--work", "w", "--out", "o", "--name", "N"]).contrast, "none")
        with self.assertRaises(SystemExit), redirect_stderr(io.StringIO()):
            stack.parse_args(["--lights", ".", "--work", "w", "--out", "o", "--name", "N", "--contrast", "extreme"])

        # The render itself, on a synthetic master: the plain image and the one with the curve, in that order.
        root = Path(tempfile.mkdtemp())
        self.addCleanup(lambda: shutil.rmtree(root, ignore_errors=True))
        (root / "work" / "render").mkdir(parents=True)
        shape = (160, 200)
        stars = star_field(shape, random_stars(40, shape, 5))
        cloud = blob(shape, (80, 100), 30)
        master = root / "master.fit"
        fits.PrimaryHDU(np.stack([0.02 + 0.5 * cloud + stars, 0.02 + 0.3 * cloud + stars, 0.02 + 0.2 * cloud + stars]).astype(np.float32)).writeto(master)
        for contrast in ("none", "presentation"):
            out = root / contrast
            args = argparse.Namespace(work=root / "work", out=out, name="N", trusted=[], pipelines_dir=[], pipeline=[pipeline.id],
                                      gradient_degree=1, black_percentile=0.05, deconv_fwhm=None, contrast=contrast)
            events: list[dict[str, object]] = []

            class Quiet(stack.Reporter):
                def emit(self, event: str, **fields: object) -> None:
                    events.append({"event": event, **fields})

            with redirect_stdout(io.StringIO()):
                outputs = stack.render_outputs(args, plan, Quiet(False), stack.Runner(), master, None)
            self.assertEqual([o["file"] for o in outputs][0], f"N_{pipeline.id}.{pipeline.outputs[0]}")
            self.assertEqual(len(outputs), len(pipeline.outputs) * (1 if contrast == "none" else 2))
            self.assertEqual(["contrast" in o for o in outputs], [False] * len(pipeline.outputs) + [True] * (len(outputs) - len(pipeline.outputs)))
            self.assertEqual(len(events), len(outputs))
        paired = [o for o in outputs if "contrast" in o]
        self.assertEqual({o["file"] for o in paired}, {f"N_{pipeline.id}_presentation.{ext}" for ext in pipeline.outputs})
        self.assertEqual({o["contrast"] for o in paired}, {"presentation"})
        # The image without the curve is the same with and without the option.
        self.assertEqual((root / "none" / f"N_{pipeline.id}.tif").read_bytes() if "tif" in pipeline.outputs else b"",
                         (root / "presentation" / f"N_{pipeline.id}.tif").read_bytes() if "tif" in pipeline.outputs else b"")


@NEEDS_NUMPY
class Combination(unittest.TestCase):
    """``plan_combine``: one level per band, a mean weighted by time."""

    def test_a_master_is_brought_to_the_level_of_the_reference(self) -> None:
        import plan_combine

        shape = (80, 90)
        stars = star_field(shape, random_stars(40, shape, 5))
        reference = (0.1 + stars).astype(np.float32)
        other = (0.03 + 0.5 * stars).astype(np.float32)  # another night: other scale, other sky
        matched = plan_combine.match_to(reference, other)
        self.assertLess(float(np.max(np.abs(matched - reference))), 1e-3)

    def test_the_mean_weighs_each_night_by_its_time(self) -> None:
        import plan_combine

        one, two = np.full((2, 2), 1.0, np.float32), np.full((2, 2), 4.0, np.float32)
        every = np.ones((2, 2), bool)
        mean, covered = plan_combine.weighted_mean([one, two], [3600.0, 7200.0], [every, every])
        self.assertAlmostEqual(float(mean[0, 0]), 1.0 / 3 + 4.0 * 2 / 3, places=5)
        self.assertTrue(covered.all())

    def test_a_colour_plan_with_a_dual_band_master_is_not_averaged_here(self) -> None:
        import plan_combine

        shape = (60, 70)
        stars = star_field(shape, random_stars(30, shape, 9))
        rgb = np.stack([0.1 + stars] * 3).astype(np.float32)
        dual = (0.07 * rgb).astype(np.float32)
        masters = [
            plan_combine.Master("n-09", "2026-08-09", "L-eXtreme", "dualband", 17_640),
            plan_combine.Master("n-10", "2026-08-10", "RGB", "broadband", 8_010),
        ]
        # It is combined on its dual-band master (dualband_reference.py), never by a mean.
        with self.assertRaises(ValueError):
            plan_combine.combine(masters, [dual, rgb])
        masters[0] = plan_combine.Master("n-12", "2026-08-12", "UV/IR-cut", "broadband", 14_400)
        image, done = plan_combine.combine(masters, [rgb * 2, rgb])
        self.assertEqual(done["method"], "weighted_mean")
        self.assertEqual(image.shape, rgb.shape)

    def test_masters_of_different_sizes_are_refused(self) -> None:
        import plan_combine

        masters = [plan_combine.Master("a", "d", None, "broadband", 1)] * 2
        with self.assertRaises(ValueError):
            plan_combine.combine(masters, [np.zeros((4, 4)), np.zeros((5, 4))])

    # Masters that do not cover the same field (nights turned differently, on the whole frame of the
    # reference night): Siril fills what a master's frame does not cover with exact zeros.

    @staticmethod
    def sky_master(stars, level: float, scale: float, columns: slice) -> "np.ndarray":
        image = np.zeros(stars.shape, np.float32)
        image[:, columns] = (level + scale * stars)[:, columns]
        return image

    def test_a_pixel_is_averaged_over_the_masters_that_cover_it(self) -> None:
        import plan_combine

        shape = (80, 90)
        stars = star_field(shape, random_stars(40, shape, 5))
        reference = self.sky_master(stars, 0.1, 1.0, slice(0, 90))
        other = self.sky_master(stars, 0.03, 0.5, slice(0, 54))  # another night: other scale, other sky, 60 % of the field
        masters = [plan_combine.Master("n-1", "d", None, "broadband", 7200), plan_combine.Master("n-2", "d", None, "broadband", 3600)]
        image, done = plan_combine.combine(masters, [reference, other])
        # Zeros averaged in would halve the level where only the reference covers; it is the reference's own.
        np.testing.assert_allclose(image[:, 60:], reference[:, 60:], atol=1e-6)
        np.testing.assert_allclose(image[:, :50], reference[:, :50], atol=2e-3)
        self.assertEqual(done["noData"], 0.0)

    def test_what_no_master_covers_is_said_and_left_empty(self) -> None:
        import plan_combine

        shape = (80, 90)
        stars = star_field(shape, random_stars(40, shape, 5))
        masters = [plan_combine.Master("n-1", "d", None, "broadband", 7200), plan_combine.Master("n-2", "d", None, "broadband", 3600)]
        image, done = plan_combine.combine(masters, [self.sky_master(stars, 0.1, 1.0, slice(0, 60)), self.sky_master(stars, 0.1, 1.0, slice(0, 40))])
        self.assertTrue(0.30 < done["noData"] < 0.40, done["noData"])
        self.assertEqual(float(np.abs(image[:, 62:]).max()), 0.0)

    def test_masters_that_barely_overlap_cannot_be_brought_to_one_level(self) -> None:
        import plan_combine

        shape = (80, 90)
        stars = star_field(shape, random_stars(40, shape, 5))
        masters = [plan_combine.Master("n-1", "d", None, "broadband", 7200), plan_combine.Master("n-2", "d", None, "broadband", 3600)]
        with self.assertRaises(ValueError):
            plan_combine.combine(masters, [self.sky_master(stars, 0.1, 1.0, slice(0, 40)), self.sky_master(stars, 0.1, 1.0, slice(86, 90))])


@NEEDS_NUMPY
class Coverage(unittest.TestCase):
    """``plan_combine.coverage_report``: what each part of the frame holds."""

    @staticmethod
    def brute_force(mask) -> int:
        """The area of the largest rectangle of ``True``, by trying every one."""
        rows, columns = mask.shape
        best = 0
        for top in range(rows):
            for left in range(columns):
                for bottom in range(top + 1, rows + 1):
                    if not mask[bottom - 1, left]:
                        break
                    for right in range(left + 1, columns + 1):
                        if not mask[top:bottom, right - 1].all():
                            break
                        best = max(best, (bottom - top) * (right - left))
        return best

    def test_the_largest_rectangle_is_the_one_a_brute_force_finds(self) -> None:
        import plan_combine

        rng = np.random.default_rng(4)
        for trial in range(40):
            mask = rng.uniform(size=(rng.integers(2, 9), rng.integers(2, 9))) < 0.75
            top, left, height, width = plan_combine.largest_rectangle(mask)
            self.assertEqual(height * width, self.brute_force(mask), f"trial {trial}\n{mask.astype(int)}")
            if height:
                self.assertTrue(mask[top:top + height, left:left + width].all())
        self.assertEqual(plan_combine.largest_rectangle(np.zeros((4, 5), bool)), (0, 0, 0, 0))
        self.assertEqual(plan_combine.largest_rectangle(np.ones((4, 5), bool)), (0, 0, 4, 5))

    def test_a_big_frame_gets_a_rectangle_nothing_can_enlarge(self) -> None:
        import plan_combine

        # A frame of the Seestar's size, covered but for the corners a 35° turn leaves out.
        rows, columns = 2160, 3840
        yy, xx = np.mgrid[0:rows, 0:columns]
        angle = np.radians(35)
        u = (xx - columns / 2) * np.cos(angle) + (yy - rows / 2) * np.sin(angle)
        v = -(xx - columns / 2) * np.sin(angle) + (yy - rows / 2) * np.cos(angle)
        mask = (np.abs(u) < columns / 2 - 40) & (np.abs(v) < rows / 2 - 40)
        top, left, height, width = plan_combine.largest_rectangle(mask)
        self.assertTrue(mask[top:top + height, left:left + width].all())
        self.assertEqual(plan_combine.grow_rectangle(mask, (top, left, height, width)), (top, left, height, width))
        # Within a few percent of the exact search (the same code, on the full mask).
        exact = plan_combine.exact_rectangle(mask)
        self.assertGreater(height * width, 0.97 * exact[2] * exact[3])

    @staticmethod
    def covered(rows: slice, columns: slice, shape=(40, 60)):
        image = np.zeros((3, *shape), np.float32)
        image[:, rows, columns] = 1.0
        return image

    def report(self, parts):
        """``coverage_report`` of masters ``(band, seconds, rows, columns)`` on a frame of 40 x 60."""
        import plan_combine

        masters = [plan_combine.Master(f"n-{n}", "d", None, band, seconds) for n, (band, seconds, _, _) in enumerate(parts, 1)]
        covers = [plan_combine.coverage(self.covered(rows, columns)) for _, _, rows, columns in parts]
        return plan_combine.coverage_report(masters, covers, colour=True)

    def test_each_zone_adds_up_the_integrations_that_cover_it(self) -> None:
        # « centre: 7 h, of which 3 h in Hα · corners: 4 h, no Hα »: a 4 h broadband night over the whole frame, a
        # 3 h dual-band one that only reaches the middle columns.
        whole = slice(0, 40), slice(0, 60)
        middle = slice(0, 40), slice(15, 45)
        report = self.report([("broadband", 4 * 3600, *whole), ("dualband", 3 * 3600, *middle)])
        centre, corners = report["zones"]
        self.assertEqual((centre["seconds"], centre["dualBandSeconds"]), (7 * 3600, 3 * 3600))
        self.assertEqual((corners["seconds"], corners["dualBandSeconds"]), (4 * 3600, 0))
        self.assertAlmostEqual(centre["share"] + corners["share"], 1.0, places=3)
        # The dual-band night covers 30 columns less its two eroded edges on each side of 60: the rest has no Hα.
        self.assertAlmostEqual(report["noDualBandShare"], corners["share"], places=4)
        self.assertGreater(report["noDualBandShare"], 0.4)
        rectangle = report["fullCoverage"]
        self.assertEqual((rectangle["height"], rectangle["frameHeight"], rectangle["frameWidth"]), (40, 40, 60))
        self.assertEqual(rectangle["width"], 26)
        self.assertEqual(rectangle["shareOfFrame"], round(40 * 26 / (40 * 60), 4))

    def test_a_plan_every_night_covers_has_one_zone_and_no_corner_to_say(self) -> None:
        whole = slice(0, 40), slice(0, 60)
        report = self.report([("broadband", 4 * 3600, *whole), ("dualband", 3 * 3600, *whole)])
        self.assertEqual(report["zones"], [{"share": 1.0, "seconds": 7 * 3600, "dualBandSeconds": 3 * 3600}])
        self.assertEqual(report["noDualBandShare"], 0.0)
        self.assertEqual(report["fullCoverage"]["shareOfFrame"], 1.0)

    def test_broadband_nights_alone_have_no_dual_band_share_whatever_they_cover(self) -> None:
        report = self.report([("broadband", 3600, slice(0, 40), slice(0, 60)), ("broadband", 3600, slice(0, 40), slice(0, 30))])
        self.assertEqual(report["noDualBandShare"], 0.0)
        self.assertEqual([z["seconds"] for z in report["zones"]], [7200, 3600])

    def test_the_report_changes_nothing_of_the_image(self) -> None:
        import plan_combine

        shape = (60, 70)
        stars = star_field(shape, random_stars(30, shape, 9))
        rgb = np.stack([0.1 + stars] * 3).astype(np.float32)
        dual = (0.07 * rgb).astype(np.float32)
        masters = [plan_combine.Master("n-9", "d", "UV/IR-cut", "broadband", 17_640), plan_combine.Master("n-10", "d", "RGB", "broadband", 8_010)]
        image, done = plan_combine.combine(masters, [dual, rgb])
        original = plan_combine.coverage_report
        plan_combine.coverage_report = lambda *args: {}
        try:
            without, _ = plan_combine.combine(masters, [dual, rgb])
        finally:
            plan_combine.coverage_report = original
        self.assertTrue(np.array_equal(image, without))
        self.assertEqual(done["coverage"]["noDualBandShare"], 0.0)


class Job(unittest.TestCase):
    """``plan_stack.py``: the job read, the plan said, nothing written in a dry run."""

    def setUp(self) -> None:
        self.root = Path(tempfile.mkdtemp())
        self.addCleanup(lambda: shutil.rmtree(self.root, ignore_errors=True))

    def job(self, **changes: object) -> Path:
        for name in ("a", "b"):
            (self.root / f"{name}_integration.fit").write_bytes(b"x")
        data: dict[str, object] = {
            "format": 1, "projectId": "p", "targetId": "ic-1805", "name": "IC_1805_plan",
            "out": str(self.root / "plan" / "stacked"), "work": str(self.root / "plan" / ".astro-sync-work" / "plan-image"),
            "reference": 0, "pipeline": {"pipelineId": "linear", "trustedSha256": None},
            "masters": [
                {"path": str(self.root / "a_integration.fit"), "nightId": "n-09", "eveningDate": "2026-08-09",
                 "filter": "L-eXtreme", "band": "dualband", "frameCount": 98, "totalSeconds": 17640},
                {"path": str(self.root / "b_integration.fit"), "nightId": "n-10", "eveningDate": "2026-08-10",
                 "filter": "RGB", "band": "broadband", "frameCount": 89, "totalSeconds": 8010},
            ],
        }
        data.update(changes)
        path = self.root / "job.json"
        path.write_text(json.dumps(data), encoding="utf-8")
        return path

    def run_script(self, *args: str) -> tuple[int, list[dict[str, object]]]:
        import plan_stack

        buffer = io.StringIO()
        with redirect_stdout(buffer):
            code = plan_stack.main([*args, "--json"])
        return code, [json.loads(line) for line in buffer.getvalue().splitlines()]

    def test_the_plan_is_said_and_nothing_written(self) -> None:
        code, events = self.run_script("--job", str(self.job()), "--dry-run")
        self.assertEqual(code, 0)
        plan = events[0]
        self.assertEqual((plan["event"], plan["masters"], plan["nights"]), ("plan", 2, ["n-09", "n-10"]))
        self.assertEqual(plan["steps"], ["alignment", "combination", "linear_export"])
        self.assertEqual(plan["outputs"], ["IC_1805_plan_linear.tif", "IC_1805_plan.json"])
        self.assertFalse((self.root / "plan").exists())

    def test_the_contrast_of_the_job_is_said_with_its_second_file(self) -> None:
        # "No processing" (the job's default here) takes no contrast twin.
        code, events = self.run_script("--job", str(self.job(contrast="strong")), "--dry-run")
        self.assertEqual(code, 0)
        self.assertEqual(events[0]["contrast"], "none")
        self.assertEqual(events[0]["outputs"], ["IC_1805_plan_linear.tif", "IC_1805_plan.json"])
        code, events = self.run_script("--job", str(self.job(contrast="strong", pipeline={"pipelineId": "raw"})), "--dry-run")
        self.assertEqual(code, 0)
        self.assertEqual(events[0]["contrast"], "strong")
        self.assertEqual(events[0]["outputs"][:2], ["IC_1805_plan_raw.tif", "IC_1805_plan_raw_strong.tif"])
        code, events = self.run_script("--job", str(self.job()), "--dry-run")
        self.assertEqual((events[0]["contrast"], len(events[0]["outputs"])), ("none", 2))
        code, events = self.run_script("--job", str(self.job(contrast="loud")), "--dry-run")
        self.assertNotEqual(code, 0)
        self.assertEqual(events[0]["code"], "bad_job")

    def test_a_job_that_asks_a_jpeg_says_it_in_its_plan(self) -> None:
        code, events = self.run_script(
            "--job", str(self.job(jpeg=True, pipeline={"pipelineId": "raw"})), "--dry-run")
        self.assertEqual(code, 0)
        self.assertEqual(events[0]["outputs"], ["IC_1805_plan_raw.tif", "IC_1805_plan_raw.jpg", "IC_1805_plan.json"])
        code, events = self.run_script("--job", str(self.job(pipeline={"pipelineId": "raw"})), "--dry-run")
        self.assertEqual(events[0]["outputs"], ["IC_1805_plan_raw.tif", "IC_1805_plan.json"])

    @NEEDS_NUMPY
    def test_the_jpeg_of_a_render_is_whole_and_light(self) -> None:
        import numpy
        import plan_framing
        import tifffile
        from PIL import Image

        picture = (numpy.random.default_rng(3).random((60, 80, 3)) * 255).astype(numpy.uint8)
        png = self.root / "render.png"
        Image.fromarray(picture).save(png)
        jpg = self.root / "out" / "render.jpg"
        plan_framing.write_jpeg(png, jpg)
        self.assertEqual(Image.open(jpg).size, (80, 60))
        self.assertEqual(sorted(p.name for p in jpg.parent.iterdir()), ["render.jpg"])  # no partial file left
        # A 16-bit TIFF is brought to 8 bits.
        tif = self.root / "render.tif"
        tifffile.imwrite(tif, picture.astype(numpy.uint16) * 257, photometric="rgb")
        plan_framing.write_jpeg(tif, self.root / "from-tif.jpg")
        self.assertEqual(Image.open(self.root / "from-tif.jpg").size, (80, 60))

    @NEEDS_NUMPY
    def test_the_jpeg_of_a_large_render_is_brought_to_a_display_size(self) -> None:
        """The player of the portal waits for each step: a JPEG of 3 000 px weighed 2.5 to 3.5 MB."""
        import numpy
        import plan_framing
        from PIL import Image

        rng = numpy.random.default_rng(4)
        yy, xx = numpy.mgrid[:1500, :2250]
        sky = (60 + 40 * numpy.sin(xx / 300.0) + rng.normal(0, 6, (1500, 2250))).clip(0, 255)
        picture = numpy.stack([sky, sky * 0.8, sky * 0.6], axis=2).astype(numpy.uint8)
        png = self.root / "large.png"
        Image.fromarray(picture).save(png)
        small = self.root / "out" / "small.jpg"
        plan_framing.write_jpeg(png, small, long_side=900)
        self.assertEqual(Image.open(small).size, (900, 600))
        shown = self.root / "out" / "shown.jpg"
        plan_framing.write_jpeg(png, shown)
        self.assertEqual(max(Image.open(shown).size), plan_framing.JPEG_LONG_SIDE)
        self.assertEqual(Image.open(shown).size, (2048, 1365))
        whole = self.root / "out" / "whole.jpg"
        plan_framing.write_jpeg(png, whole, long_side=10_000)
        self.assertEqual(Image.open(whole).size, (2250, 1500), "never enlarged, never cut")
        self.assertLess(shown.stat().st_size, whole.stat().st_size)
        self.assertLess(small.stat().st_size, 500_000)

    def test_a_job_without_its_masters_or_into_final_is_refused(self) -> None:
        code, events = self.run_script("--job", str(self.job(masters=[])), "--dry-run")
        self.assertEqual((code, events[-1]["code"]), (2, "bad_job"))
        missing = [{"path": str(self.root / "gone.fit"), "nightId": "n", "eveningDate": "d", "totalSeconds": 1}]
        code, events = self.run_script("--job", str(self.job(masters=missing)), "--dry-run")
        self.assertEqual((code, events[-1]["code"]), (2, "folder_missing"))
        code, events = self.run_script("--job", str(self.job(out=str(self.root / "final"))), "--dry-run")
        self.assertEqual(events[-1]["code"], "output_is_final")

    def test_the_alignment_puts_the_reference_first(self) -> None:
        import plan_stack

        scripts = dict(plan_stack.alignment_scripts(1))
        self.assertIn("setref plan 2", scripts["register"])
        self.assertIn("register plan -2pass -transf=affine", scripts["register"])
        # The registration only computes the transforms: the framing is chosen from what it measured.
        self.assertNotIn("seqapplyreg", scripts["register"])
        self.assertIn("seqapplyreg plan -prefix=r_ -framing=min", scripts["apply"])
        self.assertIn("seqapplyreg plan -prefix=r_ -framing=current", dict(plan_stack.alignment_scripts(1, "full"))["apply"])
        # ``register -2pass`` chooses its own reference: the whole frame is the reference night's only because it is set again.
        self.assertIn("setref plan 2\nseqapplyreg", dict(plan_stack.alignment_scripts(1, "full"))["apply"])
        self.assertNotIn("setref", scripts["apply"])

    @NEEDS_NUMPY
    def test_a_master_says_how_far_its_frames_turned(self) -> None:
        import plan_stack

        folder = self.root
        for name, keys in (("turned", {"ASROTMIN": -62.5, "ASROTMAX": 3.0}), ("silent", {})):
            header = fits.Header()
            for key, value in keys.items():
                header[key] = value
            fits.PrimaryHDU(np.zeros((4, 4), np.float32), header).writeto(folder / f"{name}.fit")
        self.assertEqual(plan_stack.master_rotation(folder / "turned.fit"), (-62.5, 3.0))
        # A master kept before the measure stands for a field that did not turn.
        self.assertEqual(plan_stack.master_rotation(folder / "silent.fit"), (0.0, 0.0))

    def test_the_masters_are_named_by_night_in_the_notes(self) -> None:
        import plan_stack

        job = json.loads(self.job().read_text(encoding="utf-8"))
        self.assertEqual(plan_stack.master_labels(job), ["2026-08-09 · n-09 · L-eXtreme", "2026-08-10 · n-10 · RGB"])



@NEEDS_NUMPY
@unittest.skipIf(SIRIL is None, "needs siril-cli (Siril 1.4)")
class EndToEnd(unittest.TestCase):
    """Two synthetic nights of a colour sensor -- one dual-band, one RGB -- stacked
    each with its master kept, then combined into the image of the plan on the
    dual-band master (``dualband_reference.py``)."""

    SHAPE = (256, 320)

    def setUp(self) -> None:
        self.root = Path(tempfile.mkdtemp())
        if not os.environ.get("ASTRO_SYNC_KEEP_E2E"):
            self.addCleanup(lambda: shutil.rmtree(self.root, ignore_errors=True))

    def night(self, name: str, filter_name: str, shift: tuple[float, float], dual: bool, frames: int, exposure: float) -> Path:
        """Bayer (RGGB) light frames of a field of stars shifted by ``shift``."""
        lights = self.root / name / "IC 1805" / "lights"
        lights.mkdir(parents=True)
        rng = np.random.default_rng(len(name))
        stars = [(y + shift[0], x + shift[1], f) for y, x, f in random_stars(120, self.SHAPE, 11)]
        field = star_field(self.SHAPE, stars, sigma=1.8)
        cloud = blob(self.SHAPE, (128, 160), 30)
        for index in range(frames):
            if dual:
                # Bright enough stars for Siril to register the frames.
                planes = [300 + 1000 * field + 600 * cloud, 300 + 1000 * field, 300 + 1000 * field]
            else:
                planes = [1500 + 2500 * field * 0.9 + 25 * cloud, 1500 + 2500 * field, 1500 + 2500 * field * 1.1]
            mosaic = np.zeros(self.SHAPE)
            mosaic[0::2, 0::2] = planes[0][0::2, 0::2]
            mosaic[0::2, 1::2] = planes[1][0::2, 1::2]
            mosaic[1::2, 0::2] = planes[1][1::2, 0::2]
            mosaic[1::2, 1::2] = planes[2][1::2, 1::2]
            mosaic = rng.poisson(np.clip(mosaic, 0, None)).astype(np.uint16)
            header = fits.Header()
            for key, value in (("IMAGETYP", "Light"), ("OBJECT", "IC 1805"), ("FILTER", filter_name),
                               ("EXPTIME", exposure), ("GAIN", 100), ("XBINNING", 1), ("CCD-TEMP", -10.0),
                               ("INSTRUME", "Synthetic IMX571"), ("BAYERPAT", "RGGB")):
                header[key] = value
            fits.PrimaryHDU(mosaic, header).writeto(lights / f"light_{index:03d}.fits")
        return lights

    def stack(self, lights: Path, family: str) -> Path:
        out = lights.parent / "stacked"
        result = subprocess.run(
            [sys.executable, str(HERE / "astro_sync_stack.py"), "--lights", str(lights),
             "--work", str(lights.parent.parent / ".astro-sync-work" / "stacking" / family),
             "--out", str(out), "--name", family, "--integration", family, "--linear-only",
             "--siril", SIRIL, "--json"],
            capture_output=True, text=True, timeout=900,
        )
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        events = [json.loads(line) for line in result.stdout.splitlines()]
        self.assertTrue(any(e["event"] == "integration" for e in events))
        return out / f"{family}_integration.fit"

    def test_two_nights_make_one_image_of_the_plan(self) -> None:
        dual = self.stack(self.night("2026-08-09 · n09", "L-eXtreme", (0, 0), True, 6, 180.0), "L-eXtreme_180s_g100")
        rgb = self.stack(self.night("2026-08-10 · n10", "RGB", (3.4, -2.2), False, 4, 90.0), "RGB_90s_g100")
        for master in (dual, rgb):
            header = fits.getheader(master)
            self.assertEqual(header["NAXIS"], 3)  # debayered: a colour master
            self.assertGreaterEqual(header["STACKCNT"], 4)
        plan = self.root / "IC 1805 · p-heart"
        job = {
            "format": 1, "projectId": "p-heart", "targetId": "ic-1805", "name": "IC_1805_plan",
            "out": str(plan / "stacked"), "work": str(plan / ".astro-sync-work" / "plan-image"),
            "reference": 0, "pipeline": {"pipelineId": "linear", "trustedSha256": None}, "siril": SIRIL,
            "masters": [
                {"path": str(dual), "nightId": "n-09", "eveningDate": "2026-08-09", "filter": "L-eXtreme",
                 "band": "dualband", "frameCount": 6, "totalSeconds": 1080},
                {"path": str(rgb), "nightId": "n-10", "eveningDate": "2026-08-10", "filter": "RGB",
                 "band": "broadband", "frameCount": 4, "totalSeconds": 360},
            ],
        }
        job_file = plan / ".astro-sync-work" / "plan-image.job.json"
        job_file.parent.mkdir(parents=True)
        job_file.write_text(json.dumps(job), encoding="utf-8")
        result = subprocess.run([sys.executable, str(HERE / "plan_stack.py"), "--job", str(job_file), "--json"],
                                capture_output=True, text=True, timeout=900)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        events = [json.loads(line) for line in result.stdout.splitlines()]
        self.assertEqual([e["step"] for e in events if e["event"] == "step"], ["alignment", "combination", "linear_export"])
        image = plan / "stacked" / "IC_1805_plan_linear.tif"
        self.assertTrue(image.is_file())
        record = json.loads((plan / "stacked" / "IC_1805_plan.json").read_text())
        self.assertEqual(record["combination"]["method"], "dualband_reference")
        self.assertEqual(record["combination"]["fusion"]["weight"], 0.18)
        self.assertEqual(record["registration"]["method"], "stars")
        self.assertEqual((record["registration"]["frames"], record["registration"]["registered"]), (2, 2))
        # The frame is the dual-band night's: no corner lacks the dual-band, no event; the RGB night, moved
        # by a few pixels, would leave a thin black border: the image is cut to what both nights cover.
        self.assertFalse(any(e["event"] == "coverage" for e in events))
        self.assertEqual(record["coverage"]["noDualBandShare"], 0.0)
        self.assertEqual([(z["seconds"], z["dualBandSeconds"]) for z in record["coverage"]["zones"]], [(1440, 1080)])
        crop = record["crop"]
        self.assertTrue(crop["applied"])
        self.assertGreater(crop["keptShare"], 0.9)
        self.assertGreater(sum(crop["lost"].values()), 0)
        self.assertEqual(record["combination"]["noData"], 0.0)
        self.assertEqual(record["coverage"]["fullCoverage"]["shareOfFrame"], 1.0)
        self.assertFalse((plan / ".astro-sync-work" / "plan-image").exists())
        import tifffile

        rendered = tifffile.imread(image).astype(np.float64)
        height, width = rendered.shape[:2]
        centre = rendered[height // 2 - 8:height // 2 + 8, width // 2 - 8:width // 2 + 8]
        corner = rendered[8:24, 8:24]
        # The Hα cloud of the dual-band night lifts the red at the centre.
        red_lift = np.median(centre[..., 0]) - np.median(corner[..., 0])
        green_lift = np.median(centre[..., 1]) - np.median(corner[..., 1])
        self.assertGreater(red_lift, 3 * max(green_lift, 1.0))


@NEEDS_NUMPY
@unittest.skipIf(SIRIL is None, "needs siril-cli (Siril 1.4)")
class AltAzimuthNights(unittest.TestCase):
    """Two nights of an alt-azimuth mount, oriented differently (for a plan): the image of
    the plan keeps the whole frame of the reference night instead of the area every night covers."""

    def setUp(self) -> None:
        self.root = Path(tempfile.mkdtemp())
        if not os.environ.get("ASTRO_SYNC_KEEP_E2E"):
            self.addCleanup(lambda: shutil.rmtree(self.root, ignore_errors=True))
        self.sky = sky_around_the_pointing()

    def stack(self, lights: Path, family: str) -> Path:
        out = lights.parent / "stacked"
        result = subprocess.run(
            [sys.executable, str(HERE / "astro_sync_stack.py"), "--lights", str(lights),
             "--work", str(lights.parent.parent / ".astro-sync-work" / "stacking" / family),
             "--out", str(out), "--name", family, "--integration", family, "--linear-only",
             "--siril", SIRIL, "--json"],
            capture_output=True, text=True, timeout=900,
        )
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        return out / f"{family}_integration.fit"

    def night(self, name: str, first_angle: float) -> Path:
        """The master of a night of 11 frames whose field turns by 60°, from ``first_angle``."""
        lights = write_night(self.root, name, self.sky, [first_angle - 6.0 * n for n in range(11)])
        return self.stack(lights, "Light_30s_g200")

    def cloudy_master(self, like: Path) -> Path:
        """A master with no star to match: a cloudy night."""
        path = self.root / "cloudy_integration.fit"
        noise = np.random.default_rng(3).normal(0.1, 0.01, SHAPE).astype(np.float32)
        fits.PrimaryHDU(noise, fits.getheader(like)).writeto(path)
        return path

    def make_plan(self, masters: list[Path], evenings: list[str], field: str | None = None,
                  folder: str = "M 45 · p-m45") -> tuple[subprocess.CompletedProcess, list[dict], Path]:
        plan = self.root / folder
        job = {
            "format": 1, "projectId": "p-m45", "targetId": "m45", "name": "M_45_plan",
            "out": str(plan / "stacked"), "work": str(plan / ".astro-sync-work" / "plan-image"),
            "reference": 0, "pipeline": {"pipelineId": "linear", "trustedSha256": None}, "siril": SIRIL,
            "masters": [
                {"path": str(path), "nightId": f"n-{number}", "eveningDate": evening, "filter": None,
                 "band": "broadband", "frameCount": 11, "totalSeconds": 330}
                for number, (path, evening) in enumerate(zip(masters, evenings), 1)
            ],
            **({"planField": field} if field else {}),
        }
        job_file = plan / ".astro-sync-work" / "plan-image.job.json"
        job_file.parent.mkdir(parents=True)
        job_file.write_text(json.dumps(job), encoding="utf-8")
        result = subprocess.run([sys.executable, str(HERE / "plan_stack.py"), "--job", str(job_file), "--json"],
                                capture_output=True, text=True, timeout=900)
        return result, [json.loads(line) for line in result.stdout.splitlines()], plan

    def test_two_nights_oriented_differently_make_one_image_of_the_whole_field(self) -> None:
        first, second = self.night("2026-10-10 · n1", 0.0), self.night("2026-10-11 · n2", -70.0)
        for master in (first, second):
            header = fits.getheader(master)
            # What each night keeps of its own rotation: the plan measures its field over all the frames.
            self.assertAlmostEqual(header["ASROTMAX"] - header["ASROTMIN"], 60.0, delta=2.0)
            self.assertLessEqual(header["ASROTMIN"], 0.0)
            self.assertGreaterEqual(header["ASROTMAX"], 0.0)
        # « Garder tout le champ »: the union of the nights, as before the crop.
        result, events, plan = self.make_plan([first, second], ["2026-10-10", "2026-10-11"], field="whole")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual([e["step"] for e in events if e["event"] == "step"], ["alignment", "combination", "linear_export"])
        registration = next(e for e in events if e["event"] == "registration")
        self.assertEqual((registration["unit"], registration["framing"], registration["fieldRotation"]), ("masters", "full", "progressive"))
        # From 0° to -130°: the nights' own turns and the difference of orientation between them.
        self.assertAlmostEqual(registration["fieldRotationDegrees"], 130.0, delta=8.0)
        self.assertEqual((registration["frames"], registration["registered"], registration["unregistered"]), (2, 2, []))
        record = json.loads((plan / "stacked" / "M_45_plan.json").read_text())
        self.assertEqual(record["registration"], {k: v for k, v in registration.items() if k != "event"})
        self.assertEqual([m["nightId"] for m in record["combination"]["masters"]], ["n-1", "n-2"])
        self.assertTrue(0.0 <= record["combination"]["noData"] < 0.5)
        # The whole frame of the reference night, not the (small) area both nights cover.
        import tifffile

        rendered = tifffile.imread(plan / "stacked" / "M_45_plan_linear.tif").astype(np.float64)
        self.assertEqual(rendered.shape[:2], SHAPE)
        # Averaged over the nights that cover it: no pixel is dimmed by the night that does not reach it.
        rows, columns = SHAPE[0] // 16, SHAPE[1] // 16
        blocks = rendered[:rows * 16, :columns * 16]
        blocks = np.median(blocks.reshape(rows, 16, columns, 16, *blocks.shape[2:]), axis=(1, 3))
        covered = blocks[blocks > 0]
        self.assertGreater(float(covered.min()), 0.8 * float(np.median(covered)))
        whole = record["coverage"]["fullCoverage"]
        # « Zone commune »: the largest rectangle both nights cover, the same pixels as the whole image there.
        result, events, plan = self.make_plan([first, second], ["2026-10-10", "2026-10-11"], field="common",
                                              folder="M 45 · cropped")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        crop = next(e for e in events if e["event"] == "crop")
        self.assertEqual((crop["applied"], crop["reason"]), (True, "commonArea"))
        self.assertEqual([crop[k] for k in ("top", "left", "height", "width")], [whole[k] for k in ("top", "left", "height", "width")])
        cut = tifffile.imread(plan / "stacked" / "M_45_plan_linear.tif").astype(np.float64)
        self.assertEqual(cut.shape[:2], (crop["height"], crop["width"]))
        top, left = crop["top"], crop["left"]
        np.testing.assert_array_equal(cut, rendered[top:top + crop["height"], left:left + crop["width"]])
        self.assertEqual(json.loads((plan / "stacked" / "M_45_plan.json").read_text())["combination"]["noData"], 0.0)

    def test_a_master_siril_cannot_align_is_left_out_and_named(self) -> None:
        first, second = self.night("2026-10-10 · n1", 0.0), self.night("2026-10-11 · n2", -70.0)
        result, events, plan = self.make_plan([first, self.cloudy_master(first), second], ["2026-10-10", "2026-10-12", "2026-10-11"])
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        registration = next(e for e in events if e["event"] == "registration")
        self.assertEqual((registration["frames"], registration["registered"]), (3, 2))
        self.assertEqual(registration["unregistered"], ["2026-10-12 · n-2"])
        record = json.loads((plan / "stacked" / "M_45_plan.json").read_text())
        self.assertEqual(record["registration"]["unregistered"], ["2026-10-12 · n-2"])
        # The plan goes on with the nights that were aligned.
        self.assertEqual([m["nightId"] for m in record["combination"]["masters"]], ["n-1", "n-3"])

    def test_a_plan_with_one_master_left_to_align_stops(self) -> None:
        first = self.night("2026-10-10 · n1", 0.0)
        result, events, _ = self.make_plan([first, self.cloudy_master(first)], ["2026-10-10", "2026-10-12"])
        self.assertEqual(result.returncode, 4)
        self.assertEqual(events[-1]["code"], "registration_failed")
        self.assertIn("only 1 of 2 masters", events[-1]["message"])


@NEEDS_NUMPY
@unittest.skipIf(SIRIL is None, "needs siril-cli (Siril 1.4)")
class DualBandReference(unittest.TestCase):
    """The longest night is dual-band and a shorter broadband night is rotated (colour sensor): the frame of
    the image is the dual-band night's, whatever the job's reference; what
    the broadband night does not reach is black and said (``noData``), and the record keeps the exposure of
    each zone."""

    def setUp(self) -> None:
        self.root = Path(tempfile.mkdtemp())
        if not os.environ.get("ASTRO_SYNC_KEEP_E2E"):
            self.addCleanup(lambda: shutil.rmtree(self.root, ignore_errors=True))
        self.sky = sky_around_the_pointing()

    def stack(self, lights: Path, family: str) -> Path:
        out = lights.parent / "stacked"
        result = subprocess.run(
            [sys.executable, str(HERE / "astro_sync_stack.py"), "--lights", str(lights),
             "--work", str(lights.parent.parent / ".astro-sync-work" / "stacking" / family),
             "--out", str(out), "--name", family, "--integration", family, "--linear-only",
             "--siril", SIRIL, "--json"],
            capture_output=True, text=True, timeout=900,
        )
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        return out / f"{family}_integration.fit"

    def masters(self) -> tuple[Path, Path]:
        """A dual-band night (12 frames of 180 s, the field still) and a broadband one (11 frames of 30 s, 60° of turn)."""
        dual = write_night(self.root, "2026-10-10 · n1", self.sky, [0.0] * 12, colour=(1.0, 0.35, 0.35),
                           filter_name="L-eXtreme", exposure=180.0)
        broad = write_night(self.root, "2026-10-11 · n2", self.sky, [-70.0 - 6.0 * n for n in range(11)],
                            colour=(1.0, 1.0, 1.0), filter_name="UV-IR-cut", exposure=30.0)
        return self.stack(dual, "L-eXtreme_180s_g200"), self.stack(broad, "UV-IR-cut_30s_g200")

    def make_plan(self, dual: Path, broad: Path, reference: int, folder: str,
                  field: str | None = None) -> tuple[subprocess.CompletedProcess, list[dict], Path]:
        plan = self.root / folder
        job = {
            "format": 1, "projectId": "p-m45", "targetId": "m45", "name": "M_45_plan",
            "out": str(plan / "stacked"), "work": str(plan / ".astro-sync-work" / "plan-image"),
            "reference": reference, "pipeline": {"pipelineId": "linear", "trustedSha256": None}, "siril": SIRIL,
            "masters": [
                {"path": str(dual), "nightId": "n-1", "eveningDate": "2026-10-10", "filter": "L-eXtreme",
                 "band": "dualband", "frameCount": 12, "totalSeconds": 2160},
                {"path": str(broad), "nightId": "n-2", "eveningDate": "2026-10-11", "filter": "UV/IR-cut",
                 "band": "broadband", "frameCount": 11, "totalSeconds": 330},
            ],
            **({"planField": field} if field else {}),
        }
        job_file = plan / ".astro-sync-work" / "plan-image.job.json"
        job_file.parent.mkdir(parents=True)
        job_file.write_text(json.dumps(job), encoding="utf-8")
        result = subprocess.run([sys.executable, str(HERE / "plan_stack.py"), "--job", str(job_file), "--json"],
                                capture_output=True, text=True, timeout=900)
        return result, [json.loads(line) for line in result.stdout.splitlines()], plan

    def test_the_dual_band_night_gives_the_frame_and_what_the_broadband_one_misses_is_black(self) -> None:
        dual, broad = self.masters()
        for master in (dual, broad):
            self.assertEqual(fits.getheader(master)["NAXIS"], 3)  # debayered: a colour master
        import hashlib

        digests = {path: hashlib.sha256(path.read_bytes()).hexdigest() for path in (dual, broad)}
        result, events, plan = self.make_plan(dual, broad, reference=1, folder="M 45 · job-reference-broadband", field="whole")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        # The masters of the nights are only read.
        self.assertEqual({path: hashlib.sha256(path.read_bytes()).hexdigest() for path in (dual, broad)}, digests)
        record = json.loads((plan / "stacked" / "M_45_plan.json").read_text())
        self.assertEqual(record["combination"]["method"], "dualband_reference")
        self.assertEqual(record["combination"]["reference"], "n-1")
        no_data = record["combination"]["noData"]
        self.assertGreater(no_data, 0.02)
        coverage = record["coverage"]
        # Every pixel with data has the dual-band night: no corner to say.
        self.assertEqual(coverage["noDualBandShare"], 0.0)
        self.assertFalse(any(e["event"] == "coverage" for e in events))
        self.assertEqual([(z["seconds"], z["dualBandSeconds"]) for z in coverage["zones"]], [(2490, 2160)])
        self.assertAlmostEqual(coverage["zones"][0]["share"], 1.0 - no_data, places=3)
        # The frame is the dual-band master's.
        frame = fits.getdata(dual).shape[1:]
        rectangle = coverage["fullCoverage"]
        self.assertEqual((rectangle["frameHeight"], rectangle["frameWidth"]), frame)
        self.assertGreater(rectangle["shareOfFrame"], 0.1)
        self.assertLess(rectangle["shareOfFrame"], 1.0 - no_data + 1e-6)
        import tifffile

        rendered = tifffile.imread(plan / "stacked" / "M_45_plan_linear.tif")
        self.assertEqual(rendered.shape[:2], frame)
        self.assertAlmostEqual(float(np.mean(~(rendered > 0).any(axis=-1))), no_data, delta=0.01)
        # By default (« Bords nets ») the black the broadband night leaves is cut away: with one dual-band night
        # covering the whole frame, the largest rectangle the broadband one covers.
        result, events, plan = self.make_plan(dual, broad, reference=1, folder="M 45 · cropped")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        crop = json.loads((plan / "stacked" / "M_45_plan.json").read_text())["crop"]
        self.assertEqual([crop[k] for k in ("top", "left", "height", "width")],
                         [rectangle[k] for k in ("top", "left", "height", "width")])
        cut = tifffile.imread(plan / "stacked" / "M_45_plan_linear.tif")
        top, left = crop["top"], crop["left"]
        np.testing.assert_array_equal(cut, rendered[top:top + crop["height"], left:left + crop["width"]])
        self.assertTrue((cut > 0).any(axis=-1).all())

    def test_the_job_reference_does_not_change_the_image(self) -> None:
        dual, broad = self.masters()
        images = []
        for reference in (0, 1):
            result, _, plan = self.make_plan(dual, broad, reference=reference, folder=f"M 45 · reference-{reference}")
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            import tifffile

            images.append(tifffile.imread(plan / "stacked" / "M_45_plan_linear.tif"))
        np.testing.assert_array_equal(images[0], images[1])


if __name__ == "__main__":
    unittest.main()
