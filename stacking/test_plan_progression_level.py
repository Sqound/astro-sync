"""Tests of the display level of the progression's milestones.

Run from this folder, with the stacking environment:
    python3 -m unittest -v test_plan_progression_level

Each milestone is the plan's image of the nights so far, so the first is the noisiest.  Shown with its own
black point (a low percentile of the noise) and its own stretch scale, its sky looked brighter than the
image of the plan's; shown with the display the image of the plan measured, every milestone has its level.
"""

from __future__ import annotations

import argparse
import contextlib
import io
import shutil
import tempfile
import unittest
from contextlib import redirect_stdout
from pathlib import Path

try:
    import numpy as np
    from astropy.io import fits
except ImportError:  # the stacking environment has them; a bare interpreter does not
    np = fits = None

from test_plan_image import blob, random_stars, star_field

NEEDS_NUMPY = unittest.skipIf(np is None, "needs numpy and astropy (the stacking environment)")
SHAPE = (160, 200)
# The noise of the cumulative stacks of 1, 2 and 3 nights (the last is the image of the plan).
NOISE = (0.020, 0.012, 0.007)


def cumulative_master(root: Path, index: int, shape: tuple[int, int] = SHAPE) -> Path:
    """The same sky, nebula and stars, with the noise of the stack of ``index + 1`` nights."""
    # The stars and the nebula are in the middle: the corners are bare sky, where the level is read.
    stars = star_field(shape, [(shape[0] / 4 + y * 0.5, shape[1] / 4 + x * 0.6, flux)
                               for y, x, flux in random_stars(40, shape, 5)])
    cloud = blob(shape, (shape[0] // 2, shape[1] // 2), 12)
    rng = np.random.default_rng(100 + index)
    planes = [0.05 + gain * cloud + stars + rng.normal(0, NOISE[index], shape)
              for gain in (0.5, 0.3, 0.2)]
    path = root / f"night-{index + 1}.fit"
    fits.PrimaryHDU(np.stack(planes).astype(np.float32)).writeto(path)
    return path


def shown_sky(display: np.ndarray) -> float:
    """The level of the sky on the screen: the median of the luminance of the displayed image, on the bare corners."""
    import renders

    level = renders.lum(display)
    return float(np.median(np.concatenate([level[:30, :30].ravel(), level[-30:, -30:].ravel()])))


@NEEDS_NUMPY
class MilestoneLevel(unittest.TestCase):
    def setUp(self) -> None:
        self.root = Path(tempfile.mkdtemp())
        self.addCleanup(lambda: shutil.rmtree(self.root, ignore_errors=True))
        self.masters = [cumulative_master(self.root, index) for index in range(3)]

    def prepare(self, master: Path, display: dict[str, object] | None = None) -> tuple[np.ndarray, dict[str, object]]:
        import renders

        measured: dict[str, object] = {}
        imposed = display or {}
        image = renders.prepare_native(master, 1, 0.05, None, imposed.get("scale"), imposed.get("black"), measured)
        return image, measured

    def test_each_milestone_alone_has_a_level_of_its_own(self) -> None:
        # The defect: the noisiest milestone is the brightest.
        levels = [shown_sky(self.prepare(master)[0]) for master in self.masters]
        self.assertGreater(levels[0] / levels[2], 1.5)
        self.assertGreater(levels[0], levels[1])
        self.assertGreater(levels[1], levels[2])

    def test_with_the_display_of_the_plan_image_every_milestone_has_its_level(self) -> None:
        plan_image, display = self.prepare(self.masters[-1])
        reference = shown_sky(plan_image)
        for master in self.masters:
            image, used = self.prepare(master, display)
            self.assertEqual(used["black"], display["black"])
            self.assertEqual(used["scale"], display["scale"])
            # Not exact: each milestone keeps its own neutral-sky gain (its colour balance), read on its own noise.
            self.assertAlmostEqual(shown_sky(image) / reference, 1.0, delta=0.05)

    def test_the_display_of_an_image_imposed_on_itself_changes_nothing(self) -> None:
        image, display = self.prepare(self.masters[1])
        again, used = self.prepare(self.masters[1], display)
        self.assertTrue(np.array_equal(image, again))
        self.assertEqual(used, display)
        self.assertEqual(len(display["black"]), 3)

    def test_the_display_goes_through_a_job_file(self) -> None:
        import json

        _, display = self.prepare(self.masters[2])
        round_trip = json.loads(json.dumps(display))
        image, _ = self.prepare(self.masters[0], display)
        again, _ = self.prepare(self.masters[0], round_trip)
        self.assertTrue(np.array_equal(image, again))


@NEEDS_NUMPY
class PolishedMilestoneLevel(unittest.TestCase):
    """The same for the « Rendu soigné »: a milestone takes the curve of the image of the plan, not its own."""

    def setUp(self) -> None:
        self.root = Path(tempfile.mkdtemp())
        self.addCleanup(lambda: shutil.rmtree(self.root, ignore_errors=True))
        (self.root / "work" / "render").mkdir(parents=True)
        self.masters = [cumulative_master(self.root, index) for index in range(3)]

    def prepare(self, index: int, display: dict[str, object] | None = None, graxpert: bool = True):
        import argparse

        import astro_sync_stack as stack
        import renders
        from test_render_pipelines import fake_graxpert

        args = argparse.Namespace(work=self.root / "work", name=f"N{index}", gradient_degree=1, black_percentile=0.05,
                                  deconv_fwhm=None)
        measured: dict[str, object] = {}
        notes: dict[str, object] = {}
        with redirect_stdout(io.StringIO()):
            nebula, stars = stack.prepared_layers(args, "polished", self.masters[index], None,
                                                  fake_graxpert(self.root) if graxpert else None, None, notes,
                                                  imposed=display, measured=measured)
        return renders.polished_deep(nebula, stars), measured, notes["polished"]

    @staticmethod
    def cloud(image: np.ndarray) -> float:
        import renders

        level = renders.lum(image)
        return float(np.median(level[70:90, 90:110]))

    def test_the_sky_of_every_polished_milestone_is_dark_whatever_its_noise(self) -> None:
        # The curve puts the sky at its level by the noise of each image: a noisier stack is not shown greyer.
        # (On these small frames the sky the curve reads, its 64 px tiles, sits a little under the bare corners.)
        levels = [shown_sky(self.prepare(index)[0]) for index in range(3)]
        self.assertLess(max(levels), 2.5 * renders_sky())
        self.assertLess(max(levels) / min(levels), 1.15)

    def test_with_the_display_of_the_plan_image_every_polished_milestone_shows_the_cloud_at_its_level(self) -> None:
        plan_image, display, done = self.prepare(2)
        self.assertEqual(done["background"], "graxpert")
        self.assertEqual(sorted(display), ["black", "curve", "scale"])
        self.assertEqual(sorted(display["curve"]), ["black", "gains", "lift", "midtones", "top"])
        reference = self.cloud(plan_image)
        own = self.cloud(self.prepare(0)[0])
        image, used, _ = self.prepare(0, display)
        self.assertEqual(used, display)
        self.assertLess(abs(self.cloud(image) / reference - 1.0), abs(own / reference - 1.0))
        self.assertAlmostEqual(self.cloud(image) / reference, 1.0, delta=0.08)

    def test_the_polynomial_polished_render_has_a_display_too(self) -> None:
        _, display, done = self.prepare(2, graxpert=False)
        self.assertEqual(done["background"], "polynomial")
        self.assertEqual(sorted(display), ["black", "curve", "scale"])

    def test_the_display_imposed_on_its_own_image_changes_nothing(self) -> None:
        import json

        image, display, _ = self.prepare(1)
        again, used, _ = self.prepare(1, json.loads(json.dumps(display)))
        np.testing.assert_allclose(image, again, atol=1e-5)
        self.assertEqual(used, display)


def renders_sky() -> float:
    import renders

    return renders.POLISHED.sky


@NEEDS_NUMPY
class JobDisplay(unittest.TestCase):
    def test_a_job_without_display_measures_its_own(self) -> None:
        import plan_stack

        self.assertIsNone(plan_stack.imposed_display({}))
        self.assertIsNone(plan_stack.render_args({"out": "o", "work": "w", "name": "N"}).display)

    def test_the_display_of_a_job_is_checked(self) -> None:
        import astro_sync_stack as stack
        import plan_stack

        good = {"display": {"black": [0.01, 0.02, 0.03], "scale": 0.5}}
        self.assertEqual(plan_stack.imposed_display(good), good["display"])
        curve = {"black": 0.001, "top": 0.8, "midtones": 0.007, "lift": 0.85, "gains": [0.95, 1.0, 1.05]}
        polished = {"display": {**good["display"], "curve": curve, "imposed": False}}
        self.assertEqual(plan_stack.imposed_display(polished), {**good["display"], "curve": curve})
        for bad in ({"black": [0.01, 0.02], "scale": 0.5}, {"black": [0.01, 0.02, 0.03], "scale": 0},
                    {"black": [0.01, 0.02, float("nan")], "scale": 1.0},
                    {"scale": 0.5}, "dark",
                    {"black": [0.01, 0.02, 0.03], "scale": 0.5, "curve": {"black": 0.0, "top": 0.8}},
                    {"black": [0.01, 0.02, 0.03], "scale": 0.5, "curve": {"black": 0.0, "top": 0.8, "midtones": 1.2}},
                    {"black": [0.01, 0.02, 0.03], "scale": 0.5, "curve": {"black": 0.0, "top": 0.8, "midtones": 0.01,
                                                                          "gains": [1.0, float("inf"), 1.0]}}):
            with self.assertRaises(stack.StackingError) as raised:
                plan_stack.imposed_display({"display": bad})
            self.assertEqual(raised.exception.code, "bad_job")

    def test_the_render_records_the_display_it_used(self) -> None:
        import astro_sync_stack as stack
        import pipeline_defs
        import plan_stack

        root = Path(tempfile.mkdtemp())
        self.addCleanup(lambda: shutil.rmtree(root, ignore_errors=True))
        (root / "work" / "render").mkdir(parents=True)
        master = cumulative_master(root, 0)
        available, _ = pipeline_defs.load_pipelines([])
        pipeline = available[stack.DEFAULT_PIPELINE]
        plan = stack.Plan(frames={}, colour=True, pipelines=[pipeline])

        def render(name: str, display: dict[str, object] | None) -> argparse.Namespace:
            args = plan_stack.render_args({"out": root / name, "work": root / "work", "name": "N", "display": display,
                                           "pipeline": {"pipelineId": pipeline.id}})
            with redirect_stdout(io.StringIO()):
                stack.render_outputs(args, plan, stack.Reporter(False), stack.Runner(), master, None)
            return args

        own = render("own", None)
        record = plan_stack.display_of(own)
        self.assertEqual(sorted(record), ["black", "imposed", "scale"])
        self.assertFalse(record["imposed"])
        shown = {"black": [v * 0.5 for v in record["black"]], "scale": record["scale"] * 2}
        given = plan_stack.display_of(render("given", shown))
        self.assertTrue(given["imposed"])
        self.assertEqual((given["black"], given["scale"]), (shown["black"], shown["scale"]))
        # The image made with the display it measured is the same, byte for byte.
        same = render("same", {"black": record["black"], "scale": record["scale"]})
        self.assertEqual(plan_stack.display_of(same)["black"], record["black"])
        for ext in pipeline.outputs:
            self.assertEqual((root / "own" / f"N_{pipeline.id}.{ext}").read_bytes(),
                             (root / "same" / f"N_{pipeline.id}.{ext}").read_bytes())

    def test_no_processing_has_no_display(self) -> None:
        import plan_stack

        args = plan_stack.render_args({"out": "o", "work": "w", "name": "N", "pipeline": {"pipelineId": "linear"}})
        self.assertIsNone(plan_stack.display_of(args))


if __name__ == "__main__":
    unittest.main()
