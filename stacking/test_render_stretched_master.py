#!/usr/bin/env python3
"""A master that a device already stretched is shown with a straight scale, not stretched a second time.

On synthetic images only: a linear stack (a faint sky, a cloud, stars) and the same stack stretched the way a
smart telescope's autostretch does (a midtones transfer function that puts the sky at a given level).  The
arcsinh display stretch, applied again to the second, lifted its sky to the middle of the range -- a grey
background on every render, ``_strong`` included, because the S-curve pivots inside the sky.
"""

from __future__ import annotations

import sys
import tempfile
import unittest
from contextlib import redirect_stdout
from io import StringIO
from pathlib import Path
from unittest import mock

import numpy as np
from astropy.io import fits

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))

import renders  # noqa: E402

HEIGHT, WIDTH = 640, 480  # the polynomial samples the sky by tiles of 160 px


def linear_stack(seed: int = 5) -> np.ndarray:
    rng = np.random.default_rng(seed)
    yy, xx = np.mgrid[:HEIGHT, :WIDTH]
    stack = np.full((3, HEIGHT, WIDTH), 0.010, np.float32)
    stack += rng.normal(0, 0.0015, stack.shape).astype(np.float32)
    cloud = np.exp(-(((xx - WIDTH * 0.5) / (WIDTH * 0.25)) ** 2 + ((yy - HEIGHT * 0.5) / (HEIGHT * 0.2)) ** 2)) * 0.05
    stack += np.stack([cloud, cloud * 0.5, cloud * 0.7]).astype(np.float32)
    for _ in range(120):
        x, y = rng.integers(8, WIDTH - 8), rng.integers(8, HEIGHT - 8)
        stack += (rng.uniform(0.02, 0.9) * np.exp(-((xx - x) ** 2 + (yy - y) ** 2) / (2 * 1.8**2)))[None].astype(np.float32)
    return np.clip(stack, 0, 1)


def autostretch(linear: np.ndarray, sky: float) -> np.ndarray:
    """A midtones transfer function that puts the median of the image at ``sky``."""
    median = float(np.median(linear))
    midtone = median * (1 - sky) / (median * (1 - 2 * sky) + sky)
    return np.clip(((midtone - 1) * linear) / ((2 * midtone - 1) * linear - midtone), 0, 1).astype(np.float32)


def prepared(root: Path, data: np.ndarray) -> np.ndarray:
    path = root / "master.fits"
    fits.PrimaryHDU(data).writeto(path, overwrite=True)
    return renders.calibrated_linear(path, 2, 0.25, None)


class StretchedMaster(unittest.TestCase):
    def setUp(self) -> None:
        self.root = Path(tempfile.mkdtemp())
        self.linear = linear_stack()

    def sky_of(self, render, data: np.ndarray) -> float:
        path = self.root / "m.fits"
        fits.PrimaryHDU(data).writeto(path, overwrite=True)
        with redirect_stdout(StringIO()):
            native = renders.prepare_native(path)
            nebula, stars = renders.star_layer(native, self.root, "x", None)
        return float(np.median(renders.lum(render(nebula, stars))))

    def test_a_linear_stack_is_not_a_stretched_master(self) -> None:
        self.assertFalse(renders.already_stretched(prepared(self.root, self.linear)))

    def test_a_stack_stretched_to_the_usual_sky_is_one(self) -> None:
        for sky in (0.25, 0.40):
            with self.subTest(sky=sky):
                self.assertTrue(renders.already_stretched(prepared(self.root, autostretch(self.linear, sky))))

    def test_a_stack_stretched_to_a_low_sky_keeps_the_arcsinh_display(self) -> None:
        # Not recognised on purpose (see ``STRETCHED_SKY_FRACTION``): today's renders.
        self.assertFalse(renders.already_stretched(prepared(self.root, autostretch(self.linear, 0.05))))

    def test_the_display_curve_of_a_linear_master_is_unchanged(self) -> None:
        level = np.linspace(0, 1, 11, dtype=np.float32)
        expected = np.arcsinh(18.0 * level / 0.8) / np.arcsinh(18.0)
        np.testing.assert_allclose(renders.display_curve(level, 0.8, False), np.clip(expected, 0, 1))
        np.testing.assert_allclose(renders.display_curve(level, 0.8, True), np.clip(level / 0.8, 0, 1))

    def test_the_sky_of_the_renders_of_a_stretched_master_is_lower_than_with_a_second_stretch(self) -> None:
        for sky in (0.25, 0.40):
            data = autostretch(self.linear, sky)
            for name, render in (
                ("natural", renders.natural_deep),
                ("dualband", renders.dualband_deep),
                ("hoo", renders.hoo_deep),
            ):
                with self.subTest(sky=sky, render=name):
                    # The second stretch gave 0.55 to 0.79; the straight scale keeps the sky near the device's.
                    self.assertLess(self.sky_of(render, data), 0.5)

    def test_the_sky_of_the_strong_contrast_does_not_come_back_grey(self) -> None:
        data = autostretch(self.linear, 0.25)
        path = self.root / "m.fits"
        fits.PrimaryHDU(data).writeto(path, overwrite=True)
        with redirect_stdout(StringIO()):
            native = renders.prepare_native(path)
            nebula, stars = renders.star_layer(native, self.root, "x", None)
        image = renders.natural_deep(nebula, stars)
        strong = renders.presentation_contrast(image, 3.6)
        self.assertLess(float(np.median(renders.lum(strong))), 0.45)

    def test_a_linear_master_keeps_its_renders(self) -> None:
        # The renders of a linear stack are those of the arcsinh display, to the last digit: forcing that
        # display changes nothing.
        for name, render in (
            ("natural", renders.natural_deep),
            ("dualband", renders.dualband_deep),
            ("hoo", renders.hoo_deep),
        ):
            with self.subTest(render=name):
                with mock.patch.object(renders, "already_stretched", return_value=False):
                    forced = self.sky_of(render, self.linear)
                self.assertEqual(self.sky_of(render, self.linear), forced)

    def test_the_polished_render_of_a_stretched_master_has_a_dark_sky(self) -> None:
        # The curve of the polished render reads the sky and its noise on the image: a sky a device already lifted
        # is brought down like any other.
        linear = prepared(self.root, autostretch(self.linear, 0.25))
        scale = renders.stretch_scale(linear)
        display = renders.polished_stretch(linear, scale)
        with redirect_stdout(StringIO()):
            nebula, stars = renders.polished_layers(linear, display, display, scale)
            natural = renders.natural_deep(renders.native_from_linear(linear, scale), np.zeros_like(linear))
        polished = float(np.median(renders.lum(renders.polished_deep(nebula, stars))))
        self.assertLess(polished, 0.5 * float(np.median(renders.lum(natural))))


if __name__ == "__main__":
    unittest.main()
