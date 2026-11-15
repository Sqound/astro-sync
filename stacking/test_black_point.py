"""The black point of a short stack: the percentile, raised to a floor of 2.8 noise widths under the sky.

A short stack registered on a rotating field has thinly covered, noisier edges; its darkest pixels lie there, so
the percentile alone falls several noise widths under the sky and the sky shows grey.  Synthetic frames only.
"""

from __future__ import annotations

import sys
import unittest
from pathlib import Path

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parent))

import renders  # noqa: E402


def field(noise: float, edge_noise: float, size: int = 512, edge: int = 24, seed: int = 7) -> np.ndarray:
    """A neutral sky at 0.02 with a few stars, its outer ``edge`` px noisier (fewer frames cover them)."""
    rng = np.random.default_rng(seed)
    image = np.full((size, size, 3), 0.02, np.float32)
    image += rng.normal(0, noise, image.shape).astype(np.float32)
    border = np.ones((size, size), bool)
    border[edge:-edge, edge:-edge] = False
    image[border] += rng.normal(0, edge_noise, (int(border.sum()), 3)).astype(np.float32)
    for y, x in rng.integers(40, size - 40, (60, 2)):
        image[y - 1:y + 2, x - 1:x + 2] += 0.3
    return image


def sigmas_above(image: np.ndarray, point: np.ndarray) -> float:
    """How many noise widths the sky of the centre stands above ``point`` (green channel)."""
    centre = image[100:-100, 100:-100, 1]
    return float((np.median(centre) - point[1]) / renders._robust_sigma(centre))


class BlackPoint(unittest.TestCase):
    def test_noisy_edges_put_the_percentile_far_under_the_sky(self) -> None:
        image = field(0.002, 0.006)
        self.assertGreater(sigmas_above(image, np.percentile(image, 0.05, axis=(0, 1))), 5.0)

    def test_the_floor_keeps_the_sky_near_its_sigmas_above_the_black_point(self) -> None:
        image = field(0.002, 0.006)
        # The noise width is read on the whole frame, the noisier edges included: a little over the bound.
        above = sigmas_above(image, renders.black_point(image, 0.05))
        self.assertLess(above, renders.BLACK_FLOOR_SIGMAS + 0.5)
        self.assertGreater(above, renders.BLACK_FLOOR_SIGMAS - 0.5)

    def test_a_percentile_already_within_the_floor_is_kept_to_the_last_digit(self) -> None:
        # The first percentile of an even field sits 2.3 noise widths under its sky: the floor leaves it.
        image = field(0.002, 0.0)
        np.testing.assert_array_equal(renders.black_point(image, 1.0), np.percentile(image, 1.0, axis=(0, 1)))

    def test_the_floor_only_ever_raises_the_black_point(self) -> None:
        for edge_noise in (0.0, 0.003, 0.006, 0.012):
            image = field(0.002, edge_noise, seed=11)
            today = np.percentile(image, 0.05, axis=(0, 1))
            self.assertTrue(np.all(renders.black_point(image, 0.05) >= today))

    def test_the_empty_pixels_of_a_frame_do_not_weigh_on_the_floor(self) -> None:
        image = field(0.002, 0.006)
        padded = np.zeros((612, 612, 3), np.float32)
        padded[50:-50, 50:-50] = image
        np.testing.assert_allclose(renders.black_point(padded, 0.05)[1], renders.black_point(image, 0.05)[1],
                                   rtol=0.02)

    def test_the_natural_render_of_a_short_stack_has_a_darker_sky(self) -> None:
        image = field(0.002, 0.006)
        flat = renders.remove_gradient(image, degree=1, black_percentile=0.05, display=True)
        before = np.clip(image - np.percentile(image, 0.05, axis=(0, 1)), 0, None)
        self.assertLess(np.median(renders.lum(flat)), np.median(renders.lum(before)))


    def test_the_data_steps_keep_the_percentile_alone(self) -> None:
        # The alignment and the combination of a plan are not a display: nothing more is clipped there.
        image = field(0.002, 0.006)
        measured: dict[str, object] = {}
        renders.remove_gradient(image, degree=1, black_percentile=0.05, measured=measured)
        flat = renders.remove_gradient(image, degree=1, black_percentile=0.05, display=True, measured={})
        self.assertLess(measured["black"][1], float(np.median(image[..., 1])) - 4 * 0.002)
        self.assertGreater(float(np.mean(flat == 0)), 0.0)


if __name__ == "__main__":
    unittest.main()
