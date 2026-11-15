"""The weighting of the poses read back from a Siril sequence file (synthetic files only)."""

from __future__ import annotations

import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import frame_weights  # noqa: E402


def sequence(noises: list[float], skies: list[float], selected: list[bool] | None = None) -> Path:
    """A stacked ``r_light_.seq`` of colour frames: their registration (sky level) and statistics (noise)."""
    selected = selected or [True] * len(noises)
    lines = ["#Siril sequence file", f"S 'r_light_' 1 {len(noises)} {sum(selected)} 5 0 6 0 0 0", "L 3"]
    lines += [f"I {n + 1} {1 if keep else 0}" for n, keep in enumerate(selected)]
    for layer in range(3):
        for n, noise in enumerate(noises):
            lines.append(f"M{layer}-{n} 8294400 8200000 0.05 0.05 0.025 -999999 0.002 -999999 0.05 0.003 0 1.1 1 {noise}")
    lines += [f"R1 3.5 4.1 0.83 0 {sky} 600 H 1 0 0 0 1 0 0 0 1" for sky in skies]
    path = Path(tempfile.mkdtemp()) / "r_light_.seq"
    path.write_text("\n".join(lines) + "\n", encoding="utf-8")
    return path


class Weighting(unittest.TestCase):
    def test_the_frames_that_make_a_mean_noisier_are_counted_and_said(self) -> None:
        # Twenty frames of a dark sky, three at dawn: a sky ten times brighter, a noise up to three times higher.
        noises = [0.002] * 20 + [0.0035, 0.006, 0.006]
        skies = [3000.0] * 20 + [9000.0, 30000.0, 30000.0]
        weighting = frame_weights.read_weighting(sequence(noises, skies))
        self.assertEqual((weighting["method"], weighting["frames"]), ("wfwhm", 23))
        self.assertEqual((weighting["noisy"], weighting["noisyBrightSky"]), (3, 3))
        self.assertEqual(weighting["noisiest"], 3.0)
        line = frame_weights.sentence(weighting)
        self.assertIn("3 add more noise than they take away", line)
        self.assertIn("brighter sky", line)

    def test_the_bound_is_where_a_frame_makes_a_mean_of_equal_frames_noisier(self) -> None:
        # Nine frames of noise 1, one more of noise t: the mean of ten is noisier than the mean of nine
        # exactly when t is over sqrt(2 + 1/9).
        bound = frame_weights.harm_bound(10)
        for t, worse in ((bound - 0.01, False), (bound + 0.01, True)):
            nine, ten = 1 / 9, (9 + t * t) / 100
            self.assertEqual(ten > nine, worse, t)
        # Just under the bound, the frame is not counted; just over, it is.
        for t, counted in ((bound - 0.01, 0), (bound + 0.01, 1)):
            weighting = frame_weights.read_weighting(sequence([1.0] * 9 + [t], [3000.0] * 10))
            self.assertEqual(weighting["noisy"], counted, t)

    def test_an_even_night_has_no_noisy_frame(self) -> None:
        weighting = frame_weights.read_weighting(sequence([0.002, 0.0021, 0.0019, 0.002], [3000.0] * 4))
        self.assertEqual(weighting["noisy"], 0)
        self.assertIn("none adds more noise", frame_weights.sentence(weighting))

    def test_the_frames_left_out_of_the_stacking_are_not_counted(self) -> None:
        weighting = frame_weights.read_weighting(sequence([0.002, 0.002, 0.01], [3000.0] * 3, [True, True, False]))
        self.assertEqual((weighting["frames"], weighting["noisy"]), (2, 0))

    def test_a_file_without_statistics_says_nothing(self) -> None:
        path = Path(tempfile.mkdtemp()) / "r_light_.seq"
        path.write_text("S 'r_light_' 1 2 2 5 0 6 0 0 0\nI 1 1\nI 2 1\n", encoding="utf-8")
        self.assertIsNone(frame_weights.read_weighting(path))
        self.assertIsNone(frame_weights.read_weighting(path.parent / "missing.seq"))
        self.assertIsNone(frame_weights.sentence(None))


if __name__ == "__main__":
    unittest.main()
