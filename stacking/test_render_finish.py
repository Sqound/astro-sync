"""Tests of the finish of a render: softened stars and colour intensity, beside the contrast.

Run from this folder, with the stacking environment:
    python3 -m unittest -v test_render_finish
"""

from __future__ import annotations

import io
import json
import shutil
import tempfile
import unittest
from contextlib import redirect_stderr, redirect_stdout
from pathlib import Path

try:
    import numpy as np
    from astropy.io import fits
    from test_plan_image import blob, random_stars, star_field
except ImportError:  # the stacking environment has them; a bare interpreter does not
    np = fits = None

NEEDS_NUMPY = unittest.skipIf(np is None, "needs numpy and astropy (the stacking environment)")


def sky_with_stars_and_nebula():
    """A display image: a nebula 40 px wide, a dense field of narrow stars, a coloured sky."""
    shape = (300, 300)
    stars = star_field(shape, random_stars(120, shape, 11), sigma=0.8).astype(np.float32)
    cloud = blob(shape, (150, 150), 40).astype(np.float32)
    rgb = np.stack([0.04 + 0.55 * cloud + stars, 0.04 + 0.2 * cloud + stars, 0.05 + 0.3 * cloud + stars], axis=-1)
    return np.clip(rgb, 0, 1).astype(np.float32), stars, cloud


@NEEDS_NUMPY
class Saturation(unittest.TestCase):
    def test_a_gain_of_one_changes_nothing_and_a_grey_image_stays_grey(self) -> None:
        import render_finish

        image, _, _ = sky_with_stars_and_nebula()
        np.testing.assert_allclose(render_finish.saturate(image, 1.0), image)
        grey = np.repeat(image[..., :1], 3, axis=-1)
        np.testing.assert_allclose(render_finish.saturate(grey, 1.4), grey, atol=1e-6)

    def test_the_colours_are_multiplied_and_the_luminance_is_kept(self) -> None:
        import render_finish
        import renders

        image = np.full((8, 8, 3), (0.30, 0.18, 0.22), np.float32)
        for percent in (10, 20, 40, 60):
            gain = 1 + percent / 100
            out = render_finish.saturate(image, gain)
            np.testing.assert_allclose(renders.lum(out), renders.lum(image), atol=1e-5)
            before = image[..., 0] - renders.lum(image)
            after = out[..., 0] - renders.lum(out)
            np.testing.assert_allclose(after, before * gain, rtol=1e-4)

    def test_a_pixel_that_would_leave_the_range_keeps_its_hue_and_never_clips(self) -> None:
        import render_finish
        import renders

        image = np.full((4, 4, 3), (0.98, 0.40, 0.30), np.float32)
        out = render_finish.saturate(image, 1.4)
        self.assertLessEqual(float(out.max()), 1.0)
        self.assertGreaterEqual(float(out.min()), 0.0)
        np.testing.assert_allclose(renders.lum(out), renders.lum(image), atol=1e-4)
        ratio_before = (image[..., 0] - renders.lum(image)) / (image[..., 2] - renders.lum(image))
        ratio_after = (out[..., 0] - renders.lum(out)) / (out[..., 2] - renders.lum(out))
        np.testing.assert_allclose(ratio_after, ratio_before, rtol=1e-3)

    def test_the_colours_are_stronger_on_the_nebula(self) -> None:
        import render_finish

        image, _, _ = sky_with_stars_and_nebula()
        spread = lambda a: float((a.max(axis=-1) - a.min(axis=-1))[130:170, 130:170].mean())  # noqa: E731
        self.assertGreater(spread(render_finish.saturate(image, 1.4)), spread(render_finish.saturate(image, 1.2)))
        self.assertGreater(spread(render_finish.saturate(image, 1.2)), spread(image))


@NEEDS_NUMPY
class SoftStars(unittest.TestCase):
    def test_nothing_is_asked_nothing_changes(self) -> None:
        import render_finish

        image, _, _ = sky_with_stars_and_nebula()
        np.testing.assert_allclose(render_finish.soften_stars(image, 0), image)

    def test_the_stars_are_pulled_down_and_the_nebula_is_left(self) -> None:
        import render_finish
        import renders

        image, stars, cloud = sky_with_stars_and_nebula()
        stars_mask = stars > 0.25
        away = (stars < 1e-3) & (cloud > 0.5)  # the nebula's core, between the stars
        peaks = {}
        for name, amount in (("light", 0.5), ("medium", 0.7), ("strong", 0.85)):
            out = render_finish.soften_stars(image, amount)
            level, before = renders.lum(out), renders.lum(image)
            self.assertTrue(np.all(out <= image + 1e-6), "light is only taken away")
            peaks[name] = float(level[stars_mask].mean())
            self.assertLess(peaks[name], float(before[stars_mask].mean()))
            # The nebula between the stars keeps its level (the smoothing may soften it a little).
            self.assertLess(abs(float(level[away].mean() - before[away].mean())), 0.03)
        self.assertLess(peaks["strong"], peaks["medium"])
        self.assertLess(peaks["medium"], peaks["light"])
        # The strong level leaves a small part of the stars; no hole darker than the sky around.
        strong = renders.lum(render_finish.soften_stars(image, 0.85))
        self.assertLess(peaks["strong"], 0.5 * float(renders.lum(image)[stars_mask].mean()))
        self.assertGreater(float(strong.min()), float(renders.lum(image).min()) - 0.04)

    def test_the_mask_is_the_stars_and_not_the_nebula(self) -> None:
        import render_finish

        image, stars, cloud = sky_with_stars_and_nebula()
        mask = render_finish.star_mask(image)
        self.assertGreater(float(mask[stars > 0.4].mean()), 0.6)
        self.assertLess(float(mask[(stars < 1e-3) & (cloud > 0.5)].mean()), 0.05)
        self.assertTrue(0 <= float(mask.min()) and float(mask.max()) <= 1)

    def test_the_sizes_follow_the_image(self) -> None:
        import render_finish

        self.assertEqual(render_finish._scaled(7, (3000, 4000, 3)), 7)
        self.assertEqual(render_finish._scaled(7, (1500, 1500)), 3.5)


@NEEDS_NUMPY
class FinishOfAnImage(unittest.TestCase):
    def test_the_words_make_the_suffix_and_the_order_is_contrast_stars_colour(self) -> None:
        import render_finish
        import renders

        self.assertTrue(render_finish.Finish().is_plain())
        self.assertEqual(render_finish.Finish("strong").suffix(), "strong")
        self.assertEqual(render_finish.Finish("none", "20").suffix(), "colour-20")
        self.assertEqual(render_finish.Finish("presentation", "40", "50").suffix(), "presentation_colour-40_stars-50")
        self.assertEqual(render_finish.Finish("2.5", "35", "70").suffix(), "contrast-2.5_colour-35_stars-70")
        image, _, _ = sky_with_stars_and_nebula()
        expected = render_finish.saturate(
            render_finish.soften_stars(renders.presentation_contrast(image, 3.6), 0.85), 1.4)
        np.testing.assert_allclose(render_finish.checked("strong", "strong", "strong").apply(image), expected)
        # Any value in between is a setting too.
        middle = render_finish.saturate(
            render_finish.soften_stars(renders.presentation_contrast(image, 2.5), 0.7), 1.3)
        np.testing.assert_allclose(render_finish.checked(2.5, 30, 70).apply(image), middle)
        # The contrast alone is what it always was.
        np.testing.assert_allclose(render_finish.Finish("strong").apply(image), renders.presentation_contrast(image, 3.6))

    def test_a_word_that_is_not_a_choice_is_refused(self) -> None:
        import render_finish

        for words in (("loud", "none", "none"), ("none", "vivid", "none"), ("none", "none", "gone"),
                      (6.5, "none", "none"), ("none", 101, "none"), ("none", "none", -1), ("nan", "none", "none")):
            with self.assertRaises(ValueError):
                render_finish.checked(*words)

    def test_the_tokens_name_the_words_the_window_always_offered(self) -> None:
        import render_finish

        self.assertEqual([render_finish.contrast_token(v) for v in ("none", 0, "0", "2", 2.0, "3.6", 3.6, "presentation", 2.5, "4")],
                         ["none", "none", "none", "presentation", "presentation", "strong", "strong", "presentation", "2.5", "4"])
        self.assertEqual([render_finish.colour_token(v) for v in ("none", 0, 40, "20.0", "enhanced", "strong", 35)],
                         ["none", "none", "40", "20", "20", "40", "35"])
        self.assertEqual([render_finish.stars_token(v) for v in (None, 85, "light", "strong", 70.5)],
                         ["none", "85", "50", "85", "70.5"])
        self.assertEqual((render_finish.contrast_gain("strong"), render_finish.contrast_gain("2.5"), render_finish.contrast_gain("none")),
                         (3.6, 2.5, 0.0))


@NEEDS_NUMPY
class TwinImages(unittest.TestCase):
    def setUp(self) -> None:
        self.root = Path(tempfile.mkdtemp())
        self.addCleanup(lambda: shutil.rmtree(self.root, ignore_errors=True))

    def test_the_twin_is_named_and_planned_after_its_finish(self) -> None:
        import astro_sync_stack as stack
        import pipeline_defs

        available, _ = pipeline_defs.load_pipelines([])
        pipeline = available[stack.DEFAULT_PIPELINE]
        plan = stack.Plan(frames={}, colour=True, pipelines=[pipeline])
        files = stack.planned_outputs(plan, "N", None, "strong", "enhanced", "light")
        self.assertIn(f"N_{pipeline.id}_strong_colour-20_stars-50.tif", files)
        files = stack.planned_outputs(plan, "N", None, "2.5", 35, "70")
        self.assertIn(f"N_{pipeline.id}_contrast-2.5_colour-35_stars-70.tif", files)
        self.assertEqual(stack.planned_outputs(plan, "N", None, "strong"), stack.planned_outputs(plan, "N", None, "strong", "none", "none"))
        self.assertIn(f"N_{pipeline.id}_colour-40.tif", stack.planned_outputs(plan, "N", None, "none", "strong"))
        self.assertEqual(len(stack.planned_outputs(plan, "N", None)), len(stack.planned_outputs(plan, "N", None, "none", "none", "none")))
        # "No processing" takes no twin, whatever is asked.
        linear = stack.Plan(frames={}, colour=True, pipelines=[], linear_only=True)
        self.assertEqual(stack.planned_outputs(linear, "N", None, "strong", "strong", "strong"), ["N_linear.tif", "N_stacking.json"])

    def test_a_pipeline_of_its_own_finish_is_named_after_it_and_the_others_keep_the_default(self) -> None:
        import astro_sync_stack as stack
        import pipeline_defs
        import render_finish

        available, _ = pipeline_defs.load_pipelines([])
        raw, hoo = available["raw"], available["hoo"]
        plan = stack.Plan(frames={}, colour=True, pipelines=[raw, hoo])
        own = {"hoo": render_finish.checked("strong", 20, 50)}
        files = stack.planned_outputs(plan, "N", None, "presentation", "none", "none", own)
        self.assertIn("N_raw_presentation.tif", files)
        self.assertIn("N_hoo_strong_colour-20_stars-50.tif", files)
        self.assertNotIn("N_hoo_presentation.tif", files)
        # A pipeline set to "none" everywhere writes its render alone, whatever the default.
        files = stack.planned_outputs(plan, "N", None, "strong", "40", "85", {"raw": render_finish.checked("none", "none", "none")})
        self.assertEqual([f for f in files if f.startswith("N_raw")], [f"N_raw.{ext}" for ext in raw.outputs])
        self.assertIn("N_hoo_strong_colour-40_stars-85.tif", files)
        # No --finish: the run is what it was.
        self.assertEqual(stack.planned_outputs(plan, "N", None, "strong", "20", "50", {}),
                         stack.planned_outputs(plan, "N", None, "strong", "20", "50"))

    def test_the_finish_of_a_pipeline_on_the_command_line(self) -> None:
        import astro_sync_stack as stack

        base = ["--lights", ".", "--work", "w", "--out", "o", "--name", "N"]
        args = stack.parse_args([*base, "--contrast", "presentation", "--finish", "hoo=strong,20,50",
                                 "--finish", "raw=none,none,none", "--finish", "hoo=3.6,30,85"])
        self.assertEqual(stack.finish_of(args, "hoo"), stack.render_finish.Finish("strong", "30", "85"))
        self.assertTrue(stack.finish_of(args, "raw").is_plain())
        self.assertEqual(stack.finish_of(args, "polished"), stack.render_finish.Finish("presentation"))
        self.assertEqual(stack.finish_of(args), stack.render_finish.Finish("presentation"))
        self.assertEqual(stack.parse_args(base).finish, [])
        for bad in (["--finish", "hoo"], ["--finish", "hoo=strong,20"], ["--finish", "=strong,20,50"],
                    ["--finish", "hoo=strong,120,50"], ["--finish", "hoo=vivid,20,50"],
                    ["--finish", "linear=strong,20,50"], ["--linear-only", "--finish", "raw=strong,20,50"]):
            with self.assertRaises(SystemExit), redirect_stderr(io.StringIO()):
                stack.parse_args([*base, *bad])

    def test_the_plan_event_says_the_finish_of_each_pipeline(self) -> None:
        import argparse

        import astro_sync_stack as stack
        import pipeline_defs
        import render_finish

        available, _ = pipeline_defs.load_pipelines([])
        plan = stack.Plan(frames={kind: [] for kind in stack.KINDS}, colour=True, pipelines=[available["raw"], available["hoo"]])
        args = argparse.Namespace(name="N", ignored_pipelines=[], contrast="presentation", colour="none", soft_stars="none",
                                  finish=[("hoo", render_finish.checked("strong", 20, 50))])
        fields = stack.plan_fields(plan, args)
        self.assertEqual(fields["finishes"], {"raw": {"contrast": "presentation", "colour": "none", "softStars": "none"},
                                              "hoo": {"contrast": "strong", "colour": "20", "softStars": "50"}})
        self.assertIn("N_hoo_strong_colour-20_stars-50.tif", fields["outputs"])
        linear = stack.Plan(frames={kind: [] for kind in stack.KINDS}, colour=True, pipelines=[], linear_only=True)
        self.assertEqual(stack.finishes_fields(linear, args), {})

    def test_the_options_of_the_command_line(self) -> None:
        import astro_sync_stack as stack

        base = ["--lights", ".", "--work", "w", "--out", "o", "--name", "N"]
        args = stack.parse_args([*base, "--colour", "30", "--soft-stars", "85", "--contrast", "2.5"])
        self.assertEqual((args.contrast, args.colour, args.soft_stars), ("2.5", "30", "strong" if False else "85"))
        args = stack.parse_args([*base, "--colour", "enhanced", "--soft-stars", "strong", "--contrast", "3.6"])
        self.assertEqual((args.contrast, args.colour, args.soft_stars), ("strong", "20", "85"))
        args = stack.parse_args(base)
        self.assertEqual((args.colour, args.soft_stars), ("none", "none"))
        for option, bad in (("--colour", "extreme"), ("--soft-stars", "101"), ("--contrast", "7"), ("--contrast", "-1")):
            with self.assertRaises(SystemExit), redirect_stderr(io.StringIO()):
                stack.parse_args([*base, option, bad])

    def test_the_render_writes_the_plain_image_then_its_twin_with_its_finish(self) -> None:
        import argparse

        import astro_sync_stack as stack
        import pipeline_defs

        available, _ = pipeline_defs.load_pipelines([])
        pipeline = available[stack.DEFAULT_PIPELINE]
        plan = stack.Plan(frames={}, colour=True, pipelines=[pipeline])
        (self.root / "work" / "render").mkdir(parents=True)
        image, stars, cloud = sky_with_stars_and_nebula()
        master = self.root / "master.fit"
        fits.PrimaryHDU(np.moveaxis(image * 0.2, -1, 0).astype(np.float32)).writeto(master)
        written = {}
        for label, extra in (("none", {}), ("finish", {"contrast": "presentation", "colour": "enhanced", "soft_stars": "light"})):
            out = self.root / label
            args = argparse.Namespace(work=self.root / "work", out=out, name="N", trusted=[], pipelines_dir=[], pipeline=[pipeline.id],
                                      gradient_degree=1, black_percentile=0.05, deconv_fwhm=None, **extra)
            with redirect_stdout(io.StringIO()):
                written[label] = stack.render_outputs(args, plan, stack.Reporter(False), stack.Runner(), master, None)
        plain = written["none"]
        self.assertEqual(len(plain), len(pipeline.outputs))
        self.assertTrue(all(not {"contrast", "colour", "softStars"} & set(o) for o in plain))
        finished = written["finish"]
        self.assertEqual(len(finished), 2 * len(pipeline.outputs))
        twins = finished[len(pipeline.outputs):]
        self.assertEqual({o["file"] for o in twins},
                         {f"N_{pipeline.id}_presentation_colour-20_stars-50.{ext}" for ext in pipeline.outputs})
        for entry in twins:
            self.assertEqual((entry["contrast"], entry["colour"], entry["softStars"]), ("presentation", "20", "50"))
        # The image without the finish is the same with and without the options.
        if "tif" in pipeline.outputs:
            self.assertEqual((self.root / "none" / f"N_{pipeline.id}.tif").read_bytes(),
                             (self.root / "finish" / f"N_{pipeline.id}.tif").read_bytes())

    def test_a_job_names_its_finish_and_a_bad_word_is_refused(self) -> None:
        import plan_stack

        for name in ("a", "b"):
            (self.root / f"{name}_integration.fit").write_bytes(b"x")

        def run(**changes: object) -> tuple[int, list[dict[str, object]]]:
            data: dict[str, object] = {
                "format": 1, "projectId": "p", "targetId": "ic-1805", "name": "IC_1805_plan",
                "out": str(self.root / "plan" / "stacked"), "work": str(self.root / "plan" / ".astro-sync-work" / "plan-image"),
                "reference": 0, "pipeline": {"pipelineId": "raw", "trustedSha256": None},
                "masters": [{"path": str(self.root / f"{n}_integration.fit"), "nightId": f"n-{n}", "eveningDate": "2026-08-09",
                             "filter": "L-eXtreme", "band": "dualband", "frameCount": 98, "totalSeconds": 17640} for n in "ab"],
            }
            data.update(changes)
            job = self.root / "job.json"
            job.write_text(json.dumps(data), encoding="utf-8")
            buffer = io.StringIO()
            with redirect_stdout(buffer):
                code = plan_stack.main(["--job", str(job), "--json", "--dry-run"])
            return code, [json.loads(line) for line in buffer.getvalue().splitlines()]

        code, events = run(contrast="strong", colour=20, softStars=85)
        self.assertEqual(code, 0)
        self.assertEqual((events[0]["contrast"], events[0]["colour"], events[0]["softStars"]), ("strong", "20", "85"))
        self.assertEqual(events[0]["outputs"][:2], ["IC_1805_plan_raw.tif", "IC_1805_plan_raw_strong_colour-20_stars-85.tif"])
        code, events = run(contrast=2.5, colour=35, softStars=70.0)
        self.assertEqual((events[0]["contrast"], events[0]["colour"], events[0]["softStars"]), ("2.5", "35", "70"))
        self.assertEqual(events[0]["outputs"][1], "IC_1805_plan_raw_contrast-2.5_colour-35_stars-70.tif")
        code, events = run()
        self.assertEqual((events[0]["colour"], events[0]["softStars"], len(events[0]["outputs"])), ("none", "none", 2))
        # "No processing" takes no finish.
        code, events = run(pipeline={"pipelineId": "linear", "trustedSha256": None}, colour=40, softStars=85)
        self.assertEqual((events[0]["colour"], events[0]["softStars"], events[0]["outputs"][0]), ("none", "none", "IC_1805_plan_linear.tif"))
        for bad in ({"colour": "vivid"}, {"softStars": "gone"}, {"contrast": 9}, {"colour": 120}, {"softStars": -5}):
            code, events = run(**bad)
            self.assertNotEqual(code, 0)
            self.assertEqual(events[0]["code"], "bad_job")


if __name__ == "__main__":
    unittest.main()
