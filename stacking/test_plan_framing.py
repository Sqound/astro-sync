"""Tests of the adjustable frame and of a device's image in a plan.

Run from this folder, with the stacking environment:
    python3 -m unittest -v test_plan_framing

The unit tests need numpy, scipy, astropy, tifffile and Pillow.  The end-to-end cases run Siril
(``siril-cli`` on the PATH, or ``ASTRO_SYNC_SIRIL``) on synthetic frames and are skipped without it.
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
from unittest import mock

HERE = Path(__file__).resolve().parent

try:
    import numpy as np
    from astropy.io import fits
    from synthetic_nights import SHAPE, sky_around_the_pointing, write_night
except ImportError:  # the stacking environment has them; a bare interpreter does not
    np = fits = None

NEEDS_NUMPY = unittest.skipIf(np is None, "needs numpy and astropy (the stacking environment)")
SIRIL = os.environ.get("ASTRO_SYNC_SIRIL") or shutil.which("siril-cli")


def disc(shape: tuple[int, int], angle: float, half_rows: float, half_columns: float) -> "np.ndarray":
    """The pixels of a rectangle turned by ``angle`` degrees about the centre of a frame of ``shape``."""
    yy, xx = np.mgrid[0:shape[0], 0:shape[1]]
    turn = np.radians(angle)
    u = (xx - shape[1] / 2) * np.cos(turn) + (yy - shape[0] / 2) * np.sin(turn)
    v = -(xx - shape[1] / 2) * np.sin(turn) + (yy - shape[0] / 2) * np.cos(turn)
    return (np.abs(u) < half_columns) & (np.abs(v) < half_rows)


def report_of(shape: tuple[int, int], parts: list[tuple[str, float, "np.ndarray"]]) -> dict:
    """``coverage_report`` of masters ``(band, seconds, cover)`` on a frame of ``shape``."""
    import plan_combine

    masters = [plan_combine.Master(f"n-{n}", "d", None, band, seconds) for n, (band, seconds, _) in enumerate(parts, 1)]
    return plan_combine.coverage_report(masters, [cover for _, _, cover in parts], colour=True)


@NEEDS_NUMPY
class Notches(unittest.TestCase):
    """The stops of the slider: cropped, the mark, two between, the whole image."""

    SHAPE = (400, 240)

    def two_nights(self) -> dict:
        whole = np.ones(self.SHAPE, bool)
        turned = disc(self.SHAPE, 30, 0.55 * self.SHAPE[0], 0.62 * self.SHAPE[1])
        return report_of(self.SHAPE, [("broadband", 7200, whole), ("dualband", 3600, turned)])

    def test_five_notches_from_the_cropped_image_to_the_whole_one(self) -> None:
        report = self.two_nights()
        stops = report["framings"]
        self.assertEqual([s["kind"] for s in stops], ["cropped", "mark", "between", "between", "whole"])
        self.assertEqual([s["default"] for s in stops], [False, False, False, False, True])
        self.assertEqual([s["recommended"] for s in stops], [False, True, False, False, False])
        # The first is what every integration covers: no degraded pixel, the same exposure everywhere.
        first, whole = stops[0], stops[-1]
        core = report["fullCoverage"]
        self.assertEqual((first["top"], first["left"], first["height"], first["width"]),
                         (core["top"], core["left"], core["height"], core["width"]))
        self.assertEqual(first["degradedShare"], 0.0)
        self.assertEqual(first["lowestSeconds"], 10800)
        self.assertEqual(first["lowestDualBandSeconds"], 3600)
        # The last is the whole image.
        self.assertEqual((whole["top"], whole["left"], whole["height"], whole["width"]), (0, 0, *self.SHAPE))
        self.assertEqual(whole["shareOfFrame"], 1.0)
        self.assertEqual(whole["degradedShare"], report["degradedShare"])
        self.assertEqual(whole["lowestSeconds"], 7200)
        self.assertEqual(whole["lowestDualBandSeconds"], 0)

    def test_the_frames_are_nested_and_grow_with_the_slider(self) -> None:
        stops = self.two_nights()["framings"]
        for before, after in zip(stops, stops[1:]):
            self.assertLessEqual(after["top"], before["top"])
            self.assertLessEqual(after["left"], before["left"])
            self.assertGreaterEqual(after["top"] + after["height"], before["top"] + before["height"])
            self.assertGreaterEqual(after["left"] + after["width"], before["left"] + before["width"])
            self.assertGreater(after["shareOfFrame"], before["shareOfFrame"])
            self.assertGreaterEqual(after["degradedShare"], before["degradedShare"] - 1e-9)

    def test_the_mark_is_the_largest_frame_with_at_most_five_percent_degraded(self) -> None:
        import plan_framing

        self.assertEqual(plan_framing.RECOMMENDED_DEGRADED_SHARE, 0.05)
        stops = self.two_nights()["framings"]
        mark = stops[1]
        self.assertLessEqual(mark["degradedShare"], 0.05)
        # The next step of 1/100 of the way to the whole image would be over the budget.
        measure_one_more = stops[2]
        self.assertGreater(measure_one_more["degradedShare"], 0.05)

    def test_the_two_stops_between_are_evenly_spaced(self) -> None:
        stops = self.two_nights()["framings"]
        mark, one, two, whole = stops[1]["t"], stops[2]["t"], stops[3]["t"], stops[4]["t"]
        self.assertEqual(whole, 1.0)
        self.assertAlmostEqual(one - mark, two - one, places=3)
        self.assertAlmostEqual(whole - two, two - one, places=3)

    def test_the_whole_image_inside_the_mark_leaves_two_stops(self) -> None:
        # A second night shifted by a few rows: 3 % of the image is degraded, under the 5 % of the mark.
        whole = np.ones(self.SHAPE, bool)
        shifted = np.zeros(self.SHAPE, bool)
        shifted[6:, :] = True
        report = report_of(self.SHAPE, [("broadband", 3600, whole), ("broadband", 3600, shifted)])
        self.assertAlmostEqual(report["degradedShare"], 6 / 400, places=3)
        stops = report["framings"]
        self.assertEqual([s["kind"] for s in stops], ["cropped", "whole"])
        self.assertTrue(stops[-1]["recommended"] and stops[-1]["default"])
        self.assertEqual(stops[0]["height"], 394)

    def test_nothing_degraded_leaves_nothing_to_choose(self) -> None:
        whole = np.ones(self.SHAPE, bool)
        report = report_of(self.SHAPE, [("broadband", 3600, whole), ("broadband", 7200, whole)])
        self.assertEqual(report["framings"], [])
        self.assertEqual(report["degradedShare"], 0.0)
        self.assertEqual(report["fullCoverage"]["shareOfFrame"], 1.0)

    def test_without_a_rectangle_every_night_covers_there_is_nothing_to_choose(self) -> None:
        left = np.zeros(self.SHAPE, bool)
        left[:, :100] = True
        right = np.zeros(self.SHAPE, bool)
        right[:, 140:] = True
        report = report_of(self.SHAPE, [("broadband", 3600, left), ("broadband", 3600, right)])
        self.assertEqual(report["fullCoverage"]["shareOfFrame"], 0.0)
        self.assertEqual(report["framings"], [])

    def test_the_block_is_offered_from_one_percent(self) -> None:
        import plan_framing

        self.assertEqual(plan_framing.OFFERED_FROM, 0.01)

    def test_the_area_every_night_covers_is_outlined_by_a_few_corners(self) -> None:
        report = self.two_nights()
        polygon = np.array(report["fullPolygon"], float)
        self.assertTrue(3 <= len(polygon) <= 14)
        # The outline encloses the area of the intersection of the nights (the rotated rectangle cut by the frame).
        turned = disc(self.SHAPE, 30, 0.55 * self.SHAPE[0], 0.62 * self.SHAPE[1])
        x, y = polygon[:, 0], polygon[:, 1]
        area = 0.5 * abs(float(np.dot(x, np.roll(y, -1)) - np.dot(y, np.roll(x, -1))))
        self.assertAlmostEqual(area / float(turned.sum()), 1.0, delta=0.03)
        # Nothing to outline when nothing is degraded.
        whole = np.ones(self.SHAPE, bool)
        self.assertNotIn("fullPolygon", report_of(self.SHAPE, [("broadband", 3600, whole), ("broadband", 3600, whole)]))

    def test_the_dual_band_part_is_said_only_with_a_dual_band_night(self) -> None:
        whole = np.ones(self.SHAPE, bool)
        turned = disc(self.SHAPE, 30, 0.55 * self.SHAPE[0], 0.62 * self.SHAPE[1])
        broad = report_of(self.SHAPE, [("broadband", 7200, whole), ("broadband", 3600, turned)])
        self.assertFalse(broad["hasDualBand"])
        self.assertEqual(broad["noDualBandShare"], 0.0)
        self.assertEqual(broad["framings"][-1]["lowestDualBandSeconds"], 0)
        mixed = self.two_nights()
        self.assertTrue(mixed["hasDualBand"])
        self.assertAlmostEqual(mixed["noDualBandShare"], mixed["degradedShare"], places=3)
        self.assertEqual(mixed["fullDualBandSeconds"], 3600)


@NEEDS_NUMPY
class Cropping(unittest.TestCase):
    """The frame is cut from the rendered image, after the render: pixels copied, never resampled."""

    def setUp(self) -> None:
        self.root = Path(tempfile.mkdtemp())
        self.addCleanup(lambda: shutil.rmtree(self.root, ignore_errors=True))

    def test_a_16_bit_tiff_is_cut_pixel_for_pixel(self) -> None:
        import plan_framing
        import tifffile

        image = np.random.default_rng(1).integers(0, 65535, (60, 50, 3), dtype=np.uint16)
        source, destination = self.root / "plan.tif", self.root / "plan_framed.tif"
        tifffile.imwrite(source, image, photometric="rgb", compression="deflate")
        plan_framing.crop_image(source, destination, {"top": 5, "left": 7, "height": 30, "width": 20})
        cut = tifffile.imread(destination)
        self.assertEqual((cut.dtype, cut.shape), (np.dtype("uint16"), (30, 20, 3)))
        self.assertTrue(np.array_equal(cut, image[5:35, 7:27]))
        self.assertEqual(plan_framing.image_size(destination), (30, 20))

    def test_the_tiff_names_astro_sync_as_its_software_never_the_library(self) -> None:
        import plan_framing
        import tifffile

        image = np.random.default_rng(3).integers(0, 65535, (20, 20, 3), dtype=np.uint16)
        source, destination = self.root / "plan.tif", self.root / "plan_framed.tif"
        tifffile.imwrite(source, image, photometric="rgb")
        for version, expected in (("9.8.7", "Astro Sync 9.8.7"), ("", "Astro Sync")):
            with mock.patch.dict(os.environ, {"ASTRO_SYNC_VERSION": version}):
                plan_framing.crop_image(source, destination, {"top": 1, "left": 1, "height": 10, "width": 10})
            with tifffile.TiffFile(destination) as handle:
                self.assertEqual(handle.pages[0].tags["Software"].value, expected)
                self.assertNotIn("tifffile", handle.pages[0].tags["Software"].value)

    def test_a_png_is_cut_too(self) -> None:
        import plan_framing
        from PIL import Image

        image = np.random.default_rng(2).integers(0, 255, (40, 30, 3), dtype=np.uint8)
        source, destination = self.root / "plan.png", self.root / "plan_framed.png"
        Image.fromarray(image).save(source)
        plan_framing.crop_image(source, destination, {"top": 2, "left": 3, "height": 10, "width": 12})
        self.assertTrue(np.array_equal(np.asarray(Image.open(destination)), image[2:12, 3:15]))

    def test_a_frame_outside_the_image_is_refused(self) -> None:
        import plan_framing

        good = {"top": 0, "left": 0, "height": 10, "width": 10}
        self.assertEqual(plan_framing.valid_frame(good, 10, 10), good)
        for bad in ({"top": 1, "left": 0, "height": 10, "width": 10}, {"top": -1, "left": 0, "height": 5, "width": 5},
                    {"top": 0, "left": 0, "height": 0, "width": 5}, {"top": 0, "left": 0, "height": 5}, "x", None):
            self.assertIsNone(plan_framing.valid_frame(bad, 10, 10))
        self.assertTrue(plan_framing.is_whole(good, 10, 10))
        self.assertFalse(plan_framing.is_whole({**good, "width": 9}, 10, 10))

    def test_the_preview_is_small_and_a_linear_image_is_stretched_for_the_eye(self) -> None:
        import plan_framing
        import tifffile
        from PIL import Image

        rng = np.random.default_rng(3)
        linear = (rng.normal(300, 20, (1200, 800, 3)).clip(0, 65535)).astype(np.uint16)
        linear[600, 400] = 40000
        source = self.root / "plan_linear.tif"
        tifffile.imwrite(source, linear, photometric="rgb", compression="deflate")
        preview = self.root / "plan.preview.png"
        plan_framing.write_preview(source, preview)
        picture = Image.open(preview)
        self.assertLessEqual(max(picture.size), 480)
        self.assertAlmostEqual(picture.size[1] / picture.size[0], 1200 / 800, places=1)
        # A square-root stretch lifts the sky well above the 0.5 % of the range it has in the file.
        self.assertGreater(float(np.median(np.asarray(picture))), 8)
        # An image already stretched is shown as it is.
        stretched = np.full((300, 200, 3), 30000, np.uint16)
        shown = self.root / "plan_raw.tif"
        tifffile.imwrite(shown, stretched, photometric="rgb")
        plan_framing.write_preview(shown, preview)
        self.assertAlmostEqual(float(np.median(np.asarray(Image.open(preview)))), 30000 / 65535 * 255, delta=2)


@NEEDS_NUMPY
class LevelByLight(unittest.TestCase):
    """The scale between two masters is the ratio of the light of the stars, not of their peaks (every plan)."""

    SHAPE = (480, 560)

    @staticmethod
    def stars(sigma: float, flux: float, centres: "np.ndarray", shape: tuple[int, int]) -> "np.ndarray":
        """Gaussian stars of the same total ``flux`` and width ``sigma`` at ``centres`` (y, x)."""
        yy, xx = np.mgrid[0:shape[0], 0:shape[1]]
        field = np.zeros(shape)
        for y, x in centres:
            field += flux / (2 * np.pi * sigma ** 2) * np.exp(-((yy - y) ** 2 + (xx - x) ** 2) / (2 * sigma ** 2))
        return field

    def pair(self, ratio: float = 1.25, seed: int = 4, reference_wide: bool = False):
        """An aligned reference and an image of the same stars, one sharp and one wider, ``ratio`` times less light in the image.

        By default the reference is the sharp one (the image of a device brought to one of ours); with
        ``reference_wide`` the reference is the wider one (one of ours brought to a device's image).
        """
        rng = np.random.default_rng(seed)
        centres = np.column_stack([rng.uniform(45, self.SHAPE[0] - 45, 300), rng.uniform(45, self.SHAPE[1] - 45, 300)])
        # Stars fainter than the brightest few are enough; fluxes span a decade (peaks 100 to 1000 sigma of the noise).
        fluxes = rng.uniform(600, 6000, len(centres))
        yy, xx = np.mgrid[0:self.SHAPE[0], 0:self.SHAPE[1]]

        def field(sigma: float, scale: float) -> "np.ndarray":
            total = np.zeros(self.SHAPE)
            for (y, x), flux in zip(centres, fluxes):
                total += scale * flux / (2 * np.pi * sigma ** 2) * np.exp(-((yy - y) ** 2 + (xx - x) ** 2) / (2 * sigma ** 2))
            return total

        sharp, wide = (1.7, 1.95) if not reference_wide else (1.95, 1.7)
        reference = field(sharp, 1.0) + 100.0 + rng.normal(0, 0.6, self.SHAPE)
        image = field(wide, 1.0 / ratio) + 60.0 + rng.normal(0, 0.6, self.SHAPE)
        return reference.astype(np.float32), image.astype(np.float32)

    def test_the_scale_is_the_ratio_of_the_light_whatever_the_width_of_the_stars(self) -> None:
        import level_match

        reference, image = self.pair(1.25)
        found = level_match.aperture_scale(reference, image)
        self.assertIsNotNone(found)
        self.assertGreater(found.stars, 30)
        self.assertAlmostEqual(found.scales[0], 1.25, delta=0.025)

    def test_the_peaks_alone_read_it_wrong_which_is_the_bias_the_bench_measured(self) -> None:
        import level_match

        # A sharper reference: the wider star has lower peaks for the same light, the scale is read high.
        reference, image = self.pair(1.25)
        self.assertGreater(level_match.robust_fit(image, reference).slope, 1.25 * 1.10)
        # A wider reference (one of our masters brought to a device's image): the scale is read low.
        reference, image = self.pair(1.25, reference_wide=True)
        self.assertLess(level_match.robust_fit(image, reference).slope, 1.25 * 0.90)
        import level_match

        self.assertAlmostEqual(level_match.aperture_scale(reference, image).scales[0], 1.25, delta=0.025)

    def test_a_matched_image_gives_the_stars_back_their_light(self) -> None:
        import level_match
        import plan_combine

        reference, image = self.pair(1.25)
        stack = np.stack([reference, reference, reference])
        other = np.stack([image, image, image])
        valid = np.ones(self.SHAPE, bool)
        matched, report = plan_combine.match_report(stack, other, valid)
        self.assertEqual(report["method"], "aperture")
        self.assertGreater(report["stars"], 30)
        # The light of the stars after the match is the reference's, within 2 % (1 +/- 0.02 of the brief).
        radius = level_match.aperture_radius(reference, image)
        centres, _ = level_match.find_stars(reference, valid)
        ratios = [level_match.aperture_flux(matched[1], y, x, radius) / level_match.aperture_flux(stack[1], y, x, radius)
                  for y, x in centres]
        self.assertAlmostEqual(float(np.median(ratios)), 1.0, delta=0.02)
        # And the sky is brought to the reference's.
        self.assertAlmostEqual(float(np.median(matched[0])), float(np.median(reference)), delta=1.0)
        # The former scale, read on the peaks, left the same pair 10 % off.
        import level_match

        slopes = [level_match.robust_fit(stack[c], other[c], valid).slope for c in range(3)]
        by_peaks = np.stack([other[c] * slopes[c] for c in range(3)])
        off = [level_match.aperture_flux(by_peaks[1], y, x, radius) / level_match.aperture_flux(stack[1], y, x, radius)
               for y, x in centres]
        self.assertGreater(abs(float(np.median(off)) - 1.0), 0.10)

    def test_a_saturated_star_does_not_decide_the_scale(self) -> None:
        import level_match

        reference, image = self.pair(1.25)
        for y, x in ((90, 90), (240, 300), (400, 160)):
            reference[y - 1:y + 2, x - 1:x + 2] += 20000
            image[y - 1:y + 2, x - 1:x + 2] = 20000  # the other one clipped
        found = level_match.aperture_scale(reference, image)
        self.assertIsNotNone(found)
        self.assertAlmostEqual(found.scales[0], 1.25, delta=0.03)

    def test_a_field_without_stars_has_no_aperture_scale_and_keeps_the_peaks(self) -> None:
        import plan_combine

        rng = np.random.default_rng(5)
        empty = rng.normal(100, 1, (3, 80, 90)).astype(np.float32)
        also = rng.normal(80, 1, (3, 80, 90)).astype(np.float32)
        self.assertIsNone(__import__("level_match").aperture_scale(empty, also))
        _, report = plan_combine.match_report(empty, also, None)
        self.assertEqual(report["method"], "peaks")
        self.assertTrue(report["apertureUnavailable"])

    def test_two_of_our_masters_are_matched_by_light_too(self) -> None:
        import level_match
        import level_match
        import plan_combine

        # Two stacks of the same stars with the same light but not the same width (our own nights differ by
        # -12 % to +8 % in the peaks): the aperture scale is 1, the peaks' scale is not.
        reference, image = self.pair(1.0)
        stack = np.stack([reference] * 3)
        other = np.stack([image] * 3)
        valid = np.ones(self.SHAPE, bool)
        matched, report = plan_combine.match_report(stack, other, valid)
        self.assertEqual(report["method"], "aperture")
        for scale in report["scales"]:
            self.assertAlmostEqual(scale, 1.0, delta=0.02)
        self.assertGreater(abs(level_match.robust_fit(image, reference, valid).slope - 1.0), 0.08)
        self.assertTrue(np.array_equal(matched, plan_combine.match_to(stack, other, valid)))
        radius = level_match.aperture_radius(reference, image)
        centres, _ = level_match.find_stars(reference, valid)
        ratios = [level_match.aperture_flux(matched[2], y, x, radius) / level_match.aperture_flux(stack[2], y, x, radius)
                  for y, x in centres]
        self.assertAlmostEqual(float(np.median(ratios)), 1.0, delta=0.02)

    def test_there_is_no_switch_back_to_the_peaks(self) -> None:
        import inspect

        import plan_combine

        for function in (plan_combine.match_to, plan_combine.match_report, plan_combine.band_mean):
            self.assertNotIn("aperture", inspect.signature(function).parameters)

    def test_every_pair_is_matched_by_light_whatever_its_source(self) -> None:
        import plan_combine

        reference, image = self.pair(1.25)
        images = [np.stack([reference] * 3), np.stack([image] * 3)]
        covers = [np.ones(self.SHAPE, bool)] * 2
        own = [plan_combine.Master("n-1", "d", None, "broadband", 7200), plan_combine.Master("n-2", "d", None, "broadband", 3600)]
        _, said = plan_combine.combine(own, images)
        self.assertEqual([m["method"] for m in said["levelMatch"]], ["aperture"])
        self.assertNotIn("source", said["masters"][0])
        device = [own[0], plan_combine.Master("n-2", "d", None, "broadband", 3600, plan_combine.DEVICE)]
        mixed_image, said = plan_combine.combine(device, images)
        self.assertEqual([m["method"] for m in said["levelMatch"]], ["aperture"])
        self.assertEqual(said["masters"][1]["source"], "device")
        # The device's night is the shorter here: the stars of the plan weigh what they weigh in the reference.
        import level_match

        centres, _ = level_match.find_stars(reference, covers[0])
        radius = level_match.aperture_radius(reference, image)
        plan_flux = np.median([level_match.aperture_flux(mixed_image[1], y, x, radius) /
                               level_match.aperture_flux(images[0][1], y, x, radius) for y, x in centres])
        self.assertAlmostEqual(float(plan_flux), 1.0, delta=0.02)
        del covers


@NEEDS_NUMPY
class RotationOfAnImageThatCannotSay(unittest.TestCase):
    """The image of a device says nothing of how the field turned in its night: the whole field is kept."""

    def test_a_master_that_cannot_say_counts_for_its_orientation_and_forces_the_whole_field(self) -> None:
        from registration_framing import COMMON_AREA, FULL_FIELD, Registration, combine_nights

        masters = Registration(frames=2, unregistered=(), rotation_degrees=0.2, orientations=(0.0, 0.2))
        # Two fixed fields: the common area.
        fixed = combine_nights(masters, [(0.0, 0.0), (0.0, 0.0)])
        self.assertEqual((fixed.framing, fixed.rotation_unknown), (COMMON_AREA, False))
        # The second is a device's image: the rotation within its night is unknown.
        unknown = combine_nights(masters, [(0.0, 0.0), None])
        self.assertTrue(unknown.rotation_unknown)
        self.assertEqual(unknown.framing, FULL_FIELD)
        self.assertFalse(unknown.progressive)
        self.assertAlmostEqual(unknown.rotation_degrees, 0.2, places=2)

    def test_the_event_announces_no_rotation_it_does_not_know(self) -> None:
        import astro_sync_stack as stack
        from registration_framing import Registration, combine_nights

        masters = Registration(frames=2, unregistered=(), rotation_degrees=0.2, orientations=(0.0, 0.2))
        fields = stack.registration_fields(combine_nights(masters, [(0.0, 0.0), None]), ["a", "b"], stack.MASTERS)
        self.assertEqual((fields["framing"], fields["fieldRotation"], fields["fieldRotationDegrees"]), ("full", None, None))
        self.assertTrue(fields["fieldRotationUnknown"])
        known = stack.registration_fields(combine_nights(masters, [(0.0, 0.0), (0.0, 0.0)]), ["a", "b"], stack.MASTERS)
        self.assertNotIn("fieldRotationUnknown", known)
        self.assertEqual(known["fieldRotation"], "none")

    def test_the_rotation_of_a_device_image_is_unknown_and_ours_is_read_or_fixed(self) -> None:
        import plan_stack

        root = Path(tempfile.mkdtemp())
        self.addCleanup(lambda: shutil.rmtree(root, ignore_errors=True))
        with_range, without = root / "a.fit", root / "b.fit"
        fits.PrimaryHDU(np.zeros((4, 4), np.float32), fits.Header([("ASROTMIN", -3.5), ("ASROTMAX", 41.0)])).writeto(with_range)
        fits.PrimaryHDU(np.zeros((4, 4), np.float32)).writeto(without)
        self.assertEqual(plan_stack.master_rotation(with_range), (-3.5, 41.0))
        self.assertEqual(plan_stack.master_rotation(without), (0.0, 0.0))
        self.assertIsNone(plan_stack.master_rotation(without, "device"))
        self.assertIsNone(plan_stack.master_rotation(with_range, "device"))


def device_file(master: Path, destination: Path, blur: float = 0.0, device: str = "ZWO Seestar S50 Pro") -> Path:
    """A device's ``Stacked_*.fit`` made from one of our masters: a 16-bit unsigned colour cube (BZERO 32768), wider stars."""
    from scipy.ndimage import gaussian_filter

    cube = np.asarray(fits.getdata(master, memmap=False), np.float32)
    if blur:
        cube = np.stack([gaussian_filter(plane, blur) for plane in cube])
    luminance = 0.2126 * cube[0] + 0.7152 * cube[1] + 0.0722 * cube[2]
    # The device's own pedestal and scale: its image is linear, its sky well under 2 % of the range.
    cube = cube * (0.012 / max(float(np.median(luminance)), 1e-6)) + 0.047 * 0.1
    data = np.clip(np.round(cube * 65535), 0, 65535).astype(np.uint16)
    header = fits.Header()
    for key, value in (("CREATOR", device), ("STACKCNT", 11), ("EXPTIME", 30.0), ("TOTALEXP", 330.0), ("FILTER", "IRCUT"),
                       ("GAIN", 200), ("OBJECT", "M 45")):
        header[key] = value
    fits.PrimaryHDU(data, header).writeto(destination)
    return destination


@NEEDS_NUMPY
@unittest.skipIf(SIRIL is None, "needs siril-cli (Siril 1.4)")
class EndToEnd(unittest.TestCase):
    """Two synthetic alt-azimuth nights made into the image of a plan (Siril): the frames on offer, the frame cut, the
    device's image in place of a night."""

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
        lights = write_night(self.root, name, self.sky, [first_angle - 3.0 * n for n in range(11)],
                             colour=(0.9, 1.0, 1.1))
        return self.stack(lights, "Light_30s_g200")

    def make_plan(self, masters: list[Path], sources: list[str] | None = None, pipeline: str = "linear",
                  frame: dict | None = None, bands: list[str] | None = None, name: str = "M_45_plan",
                  field: str | None = None) -> tuple:
        sources = sources or ["master"] * len(masters)
        bands = bands or ["broadband"] * len(masters)
        plan = self.root / "M 45 · p-m45"
        job = {
            "format": 1, "projectId": "p-m45", "targetId": "m45", "name": name,
            "out": str(plan / "stacked"), "work": str(plan / ".astro-sync-work" / "plan-image"),
            "reference": 0, "pipeline": {"pipelineId": pipeline, "trustedSha256": None}, "siril": SIRIL,
            "masters": [
                {"path": str(path), "nightId": f"n-{number}", "eveningDate": f"2026-10-{9 + number}", "filter": None,
                 "band": band, "frameCount": 11, "totalSeconds": 330, **({"source": kind} if kind != "master" else {})}
                for number, (path, kind, band) in enumerate(zip(masters, sources, bands), 1)
            ],
        }
        if frame is not None:
            job["frame"] = frame
        if field is not None:
            job["planField"] = field
        job_file = plan / ".astro-sync-work" / f"{name}.job.json"
        job_file.parent.mkdir(parents=True, exist_ok=True)
        job_file.write_text(json.dumps(job), encoding="utf-8")
        result = subprocess.run([sys.executable, str(HERE / "plan_stack.py"), "--job", str(job_file), "--json"],
                                capture_output=True, text=True, timeout=900)
        events = [json.loads(line) for line in result.stdout.splitlines() if line.startswith("{")]
        return result, events, plan

    def test_the_notches_are_in_the_record_and_the_frame_chosen_is_cut_after_the_render(self) -> None:
        import plan_framing
        import tifffile
        from PIL import Image

        first, second = self.night("2026-10-10 · n1", 0.0), self.night("2026-10-11 · n2", -50.0)
        # The frames on offer are those of the whole field: the person kept it (« Garder tout le champ »).
        result, events, plan = self.make_plan([first, second], field="whole")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        record = json.loads((plan / "stacked" / "M_45_plan.json").read_text())
        self.assertEqual((record["crop"]["applied"], record["crop"]["reason"]), (False, "wholeField"))
        coverage = record["coverage"]
        stops = coverage["framings"]
        # Two nights turned 50° apart: the second does not reach the corners of the first.
        self.assertGreaterEqual(len(stops), 2)
        self.assertEqual(stops[-1]["kind"], "whole")
        self.assertGreater(coverage["degradedShare"], plan_framing.OFFERED_FROM)
        self.assertIsNone(record["frame"])
        # A preview of the whole image sits beside the record, for the screen.
        preview = plan / "stacked" / record["preview"]
        self.assertTrue(preview.is_file())
        with Image.open(preview) as picture:
            self.assertLessEqual(max(picture.size), 480)
        # No frame asked: no twin, the whole image is the output.
        self.assertFalse(list((plan / "stacked").glob("*_framed*")))
        whole = tifffile.imread(plan / "stacked" / "M_45_plan_linear.tif")
        # Cut the first notch from the same masters: nothing is stacked again, the whole image stays and is unchanged.
        kept = {key: stops[0][key] for key in ("top", "left", "height", "width")}
        again, events, plan = self.make_plan([first, second], frame=kept, field="whole")
        self.assertEqual(again.returncode, 0, again.stdout + again.stderr)
        framed = plan / "stacked" / "M_45_plan_linear_framed.tif"
        self.assertTrue(framed.is_file())
        self.assertTrue(np.array_equal(tifffile.imread(plan / "stacked" / "M_45_plan_linear.tif"), whole))
        cut = tifffile.imread(framed)
        self.assertEqual(cut.shape[:2], (kept["height"], kept["width"]))
        self.assertTrue(np.array_equal(cut, whole[kept["top"]:kept["top"] + kept["height"], kept["left"]:kept["left"] + kept["width"]]))
        record = json.loads((plan / "stacked" / "M_45_plan.json").read_text())
        self.assertEqual(record["frame"], kept)
        files = [o["file"] for o in record["outputs"]]
        self.assertEqual(files[:2], ["M_45_plan_linear.tif", "M_45_plan_linear_framed.tif"])
        # The framed file is announced like any output, for the same pipeline.
        outputs = [e for e in events if e["event"] == "output"]
        self.assertEqual([o["pipeline"] for o in outputs], ["linear", "linear"])
        self.assertEqual(outputs[1]["file"], "M_45_plan_linear_framed.tif")
        # « Zone commune à toutes les nuits »: the first notch of the whole field, the same pixels, and nothing
        # left to choose.
        cropped, events, plan = self.make_plan([first, second], name="M_45_cropped_plan", field="common")
        self.assertEqual(cropped.returncode, 0, cropped.stdout + cropped.stderr)
        crop = next(e for e in events if e["event"] == "crop")
        self.assertEqual((crop["applied"], crop["reason"]), (True, "commonArea"))
        self.assertEqual({key: crop[key] for key in kept}, kept)
        image = tifffile.imread(plan / "stacked" / "M_45_cropped_plan_linear.tif")
        self.assertTrue(np.array_equal(image, cut))
        record = json.loads((plan / "stacked" / "M_45_cropped_plan.json").read_text())
        self.assertEqual(record["crop"], {k: v for k, v in crop.items() if k != "event"})
        self.assertEqual((record["combination"]["noData"], record["coverage"]["degradedShare"]), (0.0, 0.0))
        self.assertEqual(record["coverage"]["framings"], [])
        # « Bords nets » (the default): only what no night covers would go; the reference night covers its whole
        # frame here, so nothing is cut and the frames on offer are those of the whole field.
        clean, events, plan = self.make_plan([first, second], name="M_45_clean_plan")
        self.assertEqual(clean.returncode, 0, clean.stdout + clean.stderr)
        record = json.loads((plan / "stacked" / "M_45_clean_plan.json").read_text())
        self.assertEqual((record["crop"]["mode"], record["crop"]["applied"], record["crop"]["reason"]),
                         ("clean", False, "alreadyWhole"))
        self.assertTrue(np.array_equal(tifffile.imread(plan / "stacked" / "M_45_clean_plan_linear.tif"), whole))
        self.assertEqual(record["coverage"]["framings"], stops)

    def test_the_whole_image_as_a_frame_cuts_nothing_and_a_frame_off_the_image_is_an_error(self) -> None:
        first, second = self.night("2026-10-10 · n1", 0.0), self.night("2026-10-11 · n2", -50.0)
        result, _, plan = self.make_plan([first, second], frame={"top": 0, "left": 0, "height": SHAPE[0], "width": SHAPE[1]})
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertFalse(list((plan / "stacked").glob("*_framed*")))
        self.assertIsNone(json.loads((plan / "stacked" / "M_45_plan.json").read_text())["frame"])
        bad, events, _ = self.make_plan([first, second], frame={"top": 10, "left": 0, "height": SHAPE[0], "width": SHAPE[1]}, name="Other_plan")
        self.assertEqual(bad.returncode, 2)
        self.assertEqual(events[-1]["code"], "bad_job")

    def test_an_equatorial_plan_has_nothing_to_choose_and_its_image_is_the_whole_one(self) -> None:
        # Two nights of a field that does not turn: no corner is missing, no frame on offer.
        first = self.stack(write_night(self.root, "2026-10-10 · n1", self.sky, [0.0] * 11, colour=(0.9, 1.0, 1.1)), "Light_30s_g200")
        second = self.stack(write_night(self.root, "2026-10-11 · n2", self.sky, [0.0] * 11, colour=(0.9, 1.0, 1.1)), "Light_30s_g200")
        result, events, plan = self.make_plan([first, second])
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        record = json.loads((plan / "stacked" / "M_45_plan.json").read_text())
        self.assertEqual(record["coverage"]["framings"], [])
        self.assertIsNone(record["preview"])
        self.assertEqual(list((plan / "stacked").glob("*.preview.png")), [])

    def test_a_night_with_only_the_image_of_its_device_enters_the_plan(self) -> None:
        import tifffile

        first, second = self.night("2026-10-10 · n1", 0.0), self.night("2026-10-11 · n2", -20.0)
        stacked = device_file(second, self.root / "Stacked_11_M 45_30.0s_IRCUT_20261011.fit", blur=0.9)
        raw = fits.getheader(stacked)
        self.assertEqual(raw["BITPIX"], 16)  # 16 bits integers, BZERO 32768: Siril cannot mix them with a float master
        result, events, plan = self.make_plan([first, stacked], ["master", "device"])
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        registration = next(e for e in events if e["event"] == "registration")
        # Nothing is said of a rotation nobody knows; the whole field of the reference night is kept.
        self.assertEqual((registration["framing"], registration["fieldRotation"], registration["fieldRotationDegrees"]),
                         ("full", None, None))
        self.assertTrue(registration["fieldRotationUnknown"])
        self.assertEqual((registration["frames"], registration["registered"]), (2, 2))
        record = json.loads((plan / "stacked" / "M_45_plan.json").read_text())
        self.assertEqual([m["source"] for m in record["masters"]], ["master", "device"])
        self.assertEqual([m["source"] for m in record["combination"]["masters"][1:]], ["device"])
        level = record["combination"]["levelMatch"]
        self.assertEqual(len(level), 1)
        self.assertEqual(level[0]["method"], "aperture")
        self.assertTrue(record["registration"]["fieldRotationUnknown"])
        # The frame of the reference night, cut to the area both images cover.
        crop = record["crop"]
        self.assertEqual((crop["frameHeight"], crop["frameWidth"]), SHAPE)
        self.assertEqual(tifffile.imread(plan / "stacked" / "M_45_plan_linear.tif").shape[:2], (crop["height"], crop["width"]))
        # The device's file is read, never written (a plan only reads its masters).
        self.assertEqual(fits.getheader(stacked)["BITPIX"], 16)

    def test_a_stretched_device_image_is_refused_with_its_name(self) -> None:
        first = self.night("2026-10-10 · n1", 0.0)
        broken = self.root / "Stacked_stretched.fit"
        stretched = np.random.default_rng(6).integers(20000, 50000, (3, 64, 64)).astype(np.uint16)
        fits.PrimaryHDU(stretched, fits.Header([("CREATOR", "ZWO Seestar S50 Pro")])).writeto(broken)
        result, events, _ = self.make_plan([first, broken], ["master", "device"])
        self.assertEqual(result.returncode, 2)
        self.assertEqual(events[-1]["code"], "device_stretched")
        self.assertIn("Stacked_stretched.fit", events[-1]["message"])


if __name__ == "__main__":
    unittest.main()
