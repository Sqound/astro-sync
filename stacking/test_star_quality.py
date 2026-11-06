"""Tests of the quality read on the stars of a linear master.

Run from this folder, with the stacking environment:
    python3 -m unittest -v test_star_quality

Synthetic fields of Gaussian stars on a flat sky with noise: the figures
are checked against what was drawn, with the precision ``star_quality.py``
announces (the FWHM within 5 % from 3.5 px, an eccentricity near 0 for round
stars and near the true ratio for elongated ones).
"""

from __future__ import annotations

import io
import json
import shutil
import tempfile
import unittest
from contextlib import redirect_stderr
from pathlib import Path

import astro_sync_stack

try:
    import numpy as np
    from astropy.io import fits

    import star_quality
except ImportError:  # the stacking environment has them; a bare interpreter does not
    np = fits = star_quality = None

NEEDS_NUMPY = unittest.skipIf(np is None, "needs numpy, scipy and astropy (the stacking environment)")

SHAPE = (700, 900)


def star_field(count: int, sigma_major: float, sigma_minor: float, angle: float = 0.0, seed: int = 1,
               shape: tuple[int, int] = SHAPE) -> "np.ndarray":
    """``count`` Gaussian stars (axes ``sigma_major`` / ``sigma_minor``, turned by ``angle``) on a noisy sky."""
    rng = np.random.default_rng(seed)
    image = np.zeros(shape, np.float64)
    cos, sin = np.cos(angle), np.sin(angle)
    yy, xx = np.mgrid[0:shape[0], 0:shape[1]]
    for y, x, flux in zip(rng.uniform(15, shape[0] - 15, count), rng.uniform(15, shape[1] - 15, count),
                          rng.uniform(0.15, 0.5, count)):
        y0, y1, x0, x1 = int(y) - 12, int(y) + 13, int(x) - 12, int(x) + 13
        dy, dx = yy[y0:y1, x0:x1] - y, xx[y0:y1, x0:x1] - x
        along, across = cos * dx + sin * dy, -sin * dx + cos * dy
        image[y0:y1, x0:x1] += flux * np.exp(-(along ** 2 / (2 * sigma_major ** 2) + across ** 2 / (2 * sigma_minor ** 2)))
    return (image + 0.05 + rng.normal(0, 0.002, shape)).astype(np.float32)


@NEEDS_NUMPY
class Measure(unittest.TestCase):
    def test_round_stars_of_a_known_width(self) -> None:
        quality = star_quality.measure(star_field(150, 1.6, 1.6))
        self.assertIsNotNone(quality)
        self.assertAlmostEqual(quality["fwhmPixels"], 2.3548 * 1.6, delta=0.05 * 2.3548 * 1.6)
        self.assertLess(quality["eccentricity"], 0.25)  # a round star: the pixel grid and the noise only
        self.assertGreaterEqual(quality["starCount"], star_quality.MINIMUM_STARS)
        self.assertEqual(quality["measuredOn"], "linear")

    def test_stars_stretched_along_one_axis_are_eccentric(self) -> None:
        for angle in (0.0, 0.6, 1.2):  # whatever the direction of the trail
            quality = star_quality.measure(star_field(150, 2.4, 1.2, angle=angle))
            self.assertIsNotNone(quality)
            # axes 2:1 -> eccentricity sqrt(1 - 1/4) = 0.866
            self.assertAlmostEqual(quality["eccentricity"], 0.866, delta=0.05)

    def test_a_wider_sky_reads_wider_and_the_two_figures_do_not_mix(self) -> None:
        sharp = star_quality.measure(star_field(150, 1.6, 1.6))
        soft = star_quality.measure(star_field(150, 2.5, 2.5))
        self.assertGreater(soft["fwhmPixels"], sharp["fwhmPixels"] * 1.4)
        self.assertLess(soft["eccentricity"], 0.25)

    def test_fewer_than_twenty_stars_give_no_measure(self) -> None:
        self.assertIsNone(star_quality.measure(star_field(8, 1.6, 1.6)))
        self.assertIsNone(star_quality.measure(np.full(SHAPE, 0.05, np.float32)))  # a blank sky

    def test_the_two_stars_of_a_pair_are_never_measured(self) -> None:
        # 60 lone stars and 30 pairs 7 px apart: the stars of a pair would read
        # as trailed, so only the lone ones are kept.
        rng = np.random.default_rng(5)
        image = star_field(60, 1.6, 1.6, seed=2).astype(np.float64)
        lone = len(star_quality.candidate_stars(image.astype(np.float32), 10_000, True)[1])
        yy, xx = np.mgrid[0:SHAPE[0], 0:SHAPE[1]]
        pairs = list(zip(rng.uniform(25, SHAPE[0] - 25, 30), rng.uniform(25, SHAPE[1] - 25, 30)))
        for y, x in pairs:
            for shift in (-3.5, 3.5):
                image += 0.3 * np.exp(-(((yy - y) ** 2 + (xx - x - shift) ** 2) / (2 * 1.6 ** 2)))
        field = image.astype(np.float32)
        everything = star_quality.candidate_stars(field, 10_000, False)[1]
        alone = star_quality.candidate_stars(field, 10_000, True)
        self.assertGreater(len(everything), len(alone[1]) + 30)  # the pairs were seen...
        for y, x in zip(alone[1], alone[2]):  # ...and none of their stars is kept
            self.assertTrue(all(abs(y - py) > 8 or abs(x - px) > 12 for py, px in pairs))
        self.assertGreaterEqual(len(alone[1]), lone // 2)

    def test_a_colour_master_is_measured_on_its_luminance(self) -> None:
        plane = star_field(150, 1.6, 1.6)
        master = np.stack([plane, plane, plane])
        self.assertEqual(star_quality.measure(star_quality.luminance(master))["fwhmPixels"],
                         star_quality.measure(plane)["fwhmPixels"])
        with self.assertRaises(ValueError):
            star_quality.luminance(np.zeros((2, 4, 4)))

    def test_the_deconvolution_reads_the_same_stars_as_before(self) -> None:
        # ``renders.star_fwhm`` is the median of the same widths (no pixel changes).
        field = star_field(150, 1.6, 1.6)
        widths = [fwhm for fwhm, _ in star_quality.star_widths(field, 250)]
        self.assertEqual(star_quality.median_fwhm(field), float(np.median(widths)))
        self.assertTrue(np.isnan(star_quality.median_fwhm(np.full(SHAPE, 0.05, np.float32))))


@NEEDS_NUMPY
class Stacking(unittest.TestCase):
    """What the stacker writes of it: ``quality`` of the provenance, and a measure that never fails a stacking."""

    def setUp(self) -> None:
        self.root = Path(tempfile.mkdtemp())
        self.addCleanup(lambda: shutil.rmtree(self.root, ignore_errors=True))

    def master(self, image: "np.ndarray") -> Path:
        path = self.root / "stack_linear.fit"
        fits.PrimaryHDU(image).writeto(path)
        return path

    def test_the_master_gives_its_quality_with_the_hour_it_was_measured(self) -> None:
        quality = astro_sync_stack.measure_quality(self.master(star_field(150, 1.6, 1.6)))
        self.assertEqual(sorted(quality), ["eccentricity", "fwhmPixels", "measuredAt", "measuredOn", "starCount"])
        self.assertRegex(quality["measuredAt"], r"^\d{4}-\d\d-\d\dT\d\d:\d\d:\d\dZ$")
        json.dumps(quality)  # it goes into ``<name>_stacking.json``

    def test_too_few_stars_or_an_unreadable_master_leave_the_quality_out_without_failing(self) -> None:
        logged = io.StringIO()
        with redirect_stderr(logged):
            self.assertIsNone(astro_sync_stack.measure_quality(self.master(star_field(5, 1.6, 1.6))))
            self.assertIsNone(astro_sync_stack.measure_quality(self.root / "missing.fit"))
        self.assertIn("fewer than twenty stars", logged.getvalue())
        self.assertIn("not measured", logged.getvalue().splitlines()[-1])


if __name__ == "__main__":
    unittest.main()
