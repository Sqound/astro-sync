"""Tests of the measure of one pose by itself.

Run from this folder, with the stacking environment:
    python3 -m unittest -v test_pose_stats

Synthetic frames (Gaussian stars on a sky with noise, written as FITS): the
figures are checked against what was drawn, with the precision of
``star_quality.py`` (the same stars), and the expected cases -- few
stars (a cloud), a brighter sky (dawn), trailed stars -- read the way the
core will judge them.
"""

from __future__ import annotations

import io
import json
import shutil
import tempfile
import unittest
from contextlib import redirect_stdout
from pathlib import Path

try:
    import numpy as np
    from astropy.io import fits

    import pose_stats
    from test_star_quality import star_field
except ImportError:  # the stacking environment has them; a bare interpreter does not
    np = fits = pose_stats = star_field = None

NEEDS_NUMPY = unittest.skipIf(np is None, "needs numpy, scipy and astropy (the stacking environment)")


def write_frame(path: Path, image: "np.ndarray", **header: object) -> Path:
    hdu = fits.PrimaryHDU((image * 20000).astype(np.uint16))
    for key, value in header.items():
        hdu.header[key.replace("_", "-")] = value
    hdu.writeto(path, overwrite=True)
    return path


def mosaic(image: "np.ndarray", pattern: str = "RGGB", gains: tuple[float, float, float] = (0.8, 1.0, 0.65)) -> "np.ndarray":
    """``image`` as a Bayer sensor sees it: every pixel keeps one colour's share of the light (red, green, blue gains)."""
    rows, columns = np.indices(image.shape)
    # The colour of each pixel of a 2 x 2 cell, for the four patterns.
    cell = {"RGGB": "RGGB", "BGGR": "BGGR", "GRBG": "GRBG", "GBRG": "GBRG"}[pattern]
    letter = np.array(list(cell)).reshape(2, 2)[rows % 2, columns % 2]
    share = np.where(letter == "R", gains[0], np.where(letter == "G", gains[1], gains[2]))
    return (image * share).astype(np.float32)


@NEEDS_NUMPY
class OnePose(unittest.TestCase):
    def setUp(self) -> None:
        self.folder = Path(tempfile.mkdtemp(prefix="pose-stats-"))

    def tearDown(self) -> None:
        shutil.rmtree(self.folder, ignore_errors=True)

    def test_round_stars_are_read_with_their_count_and_their_sky(self) -> None:
        frame = write_frame(self.folder / "a.fit", star_field(150, 1.6, 1.6), DATE_OBS="2026-10-10T16:31:24.617", EXPTIME=30.0)
        answer = pose_stats.measure_pose(frame)
        self.assertEqual(answer["status"], "measured")
        self.assertAlmostEqual(answer["fwhmPixels"], 2.3548 * 1.6, delta=0.05 * 2.3548 * 1.6)
        self.assertLess(answer["eccentricity"], 0.25)
        self.assertGreaterEqual(answer["starCount"], 20)
        self.assertAlmostEqual(answer["skyBackground"], 0.05 * 20000, delta=60)
        self.assertEqual(answer["dateObs"], "2026-10-10T16:31:24.617")
        self.assertEqual(answer["exposure"], 30.0)

    def test_trailed_stars_are_eccentric(self) -> None:
        frame = write_frame(self.folder / "trail.fit", star_field(150, 2.4, 1.2, angle=0.6))
        self.assertAlmostEqual(pose_stats.measure_pose(frame)["eccentricity"], 0.866, delta=0.06)

    def test_under_twenty_stars_the_count_is_given_and_no_shape(self) -> None:
        frame = write_frame(self.folder / "cloud.fit", star_field(9, 1.6, 1.6))
        answer = pose_stats.measure_pose(frame)
        self.assertLess(answer["starCount"], 20)
        self.assertIsNone(answer["fwhmPixels"])
        self.assertIsNone(answer["eccentricity"])

    def test_a_raised_sky_is_read_in_the_background(self) -> None:
        dark = pose_stats.measure_pose(write_frame(self.folder / "n.fit", star_field(120, 1.6, 1.6)))
        bright = pose_stats.measure_pose(write_frame(self.folder / "d.fit", star_field(120, 1.6, 1.6) + 0.2))
        self.assertGreater(bright["skyBackground"], 3 * dark["skyBackground"])

    def test_a_colour_mosaic_is_read_in_super_pixels_and_the_width_in_frame_pixels(self) -> None:
        image = star_field(200, 3.2, 3.2, shape=(1400, 1800))
        frame = write_frame(self.folder / "bayer.fit", image, BAYERPAT="GRBG")
        answer = pose_stats.measure_pose(frame)
        self.assertIsNotNone(answer["fwhmPixels"])
        self.assertAlmostEqual(answer["fwhmPixels"], 2.3548 * 3.2, delta=0.15 * 2.3548 * 3.2)

    def test_a_real_mosaic_of_three_pixel_stars_is_read_on_its_green_pixels(self) -> None:
        # Stars of 3 px (FWHM) on a sensor whose red, green and blue pixels
        # catch different shares of the light, with a few hot pixels: every
        # pattern gives the width of the stars, the hot pixels are no star.
        field = star_field(220, 3.0 / 2.3548, 3.0 / 2.3548, shape=(1200, 1600))
        rng = np.random.default_rng(5)
        for pattern in ("RGGB", "BGGR", "GRBG", "GBRG"):
            sensor = mosaic(field, pattern)
            hot = rng.integers(20, 1100, size=(80, 2)), rng.integers(20, 1500, size=(80, 2))
            sensor[hot[0][:, 0], hot[1][:, 0]] += 0.6
            frame = write_frame(self.folder / f"{pattern}.fit", sensor, BAYERPAT=pattern)
            answer = pose_stats.measure_pose(frame)
            self.assertIsNotNone(answer["fwhmPixels"], pattern)
            self.assertAlmostEqual(answer["fwhmPixels"], 3.0, delta=0.15 * 3.0, msg=pattern)
            self.assertGreaterEqual(answer["readableStars"], 20)

    def test_the_green_pixels_are_kept_and_the_others_are_filled_from_them(self) -> None:
        data = np.arange(36, dtype=np.float32).reshape(6, 6)
        for pattern, offset, green in (("RGGB", 0, 1), ("GRBG", 0, 0), ("RGGB", 1, 0), ("BGGR", 0, 1)):
            plane = pose_stats.green_plane(data, pattern, offset)
            rows, columns = np.indices(data.shape)
            sites = (rows + columns) % 2 == green
            self.assertTrue(np.array_equal(plane[sites], data[sites]), (pattern, offset))
            # An interior red or blue pixel is the mean of its four green neighbours.
            for y in range(1, 5):
                for x in range(1, 5):
                    if not sites[y, x]:
                        around = (data[y - 1, x] + data[y + 1, x] + data[y, x - 1] + data[y, x + 1]) / 4
                        self.assertAlmostEqual(float(plane[y, x]), float(around), places=4)
        self.assertIsNone(pose_stats.green_plane(data, "CYGM"))

    def test_a_sparse_field_hands_over_its_stars_for_a_width_of_the_family(self) -> None:
        # Sixteen poses of a field with a few stars each: no pose has a
        # width of its own, each hands over the stars it read.
        answers = []
        for i in range(16):
            frame = write_frame(self.folder / f"sparse{i:02d}.fit", star_field(6, 1.6, 1.6, seed=100 + i))
            answers.append(pose_stats.measure_pose(frame))
        self.assertTrue(all(answer["fwhmPixels"] is None and answer["eccentricity"] is None for answer in answers))
        pooled = [width for answer in answers for width in answer["starWidths"]]
        self.assertEqual(len(pooled), sum(answer["readableStars"] for answer in answers))
        self.assertGreaterEqual(len(pooled), 25)
        for answer in answers:
            self.assertEqual(len(answer["starWidths"]), len(answer["starEccentricities"]))
        self.assertAlmostEqual(float(np.median(pooled)), 2.3548 * 1.6, delta=0.15 * 2.3548 * 1.6)

    def test_a_pose_with_a_width_of_its_own_hands_over_no_list(self) -> None:
        answer = pose_stats.measure_pose(write_frame(self.folder / "own.fit", star_field(150, 1.6, 1.6)))
        self.assertIsNotNone(answer["fwhmPixels"])
        self.assertGreaterEqual(answer["readableStars"], 20)
        self.assertNotIn("starWidths", answer)

    def test_the_exposure_is_in_seconds_whatever_the_unit_the_file_states(self) -> None:
        image = star_field(60, 1.6, 1.6)
        stellina = write_frame(self.folder / "stellina.fit", image, INSTRUME="stellina-7ba7ce", EXPOSURE=(10000, "[ms] Total Exposure Time"))
        self.assertEqual(pose_stats.measure_pose(stellina)["exposure"], 10.0)
        plain = write_frame(self.folder / "plain.fit", image, EXPOSURE=(30.0, "Exposure time"))
        self.assertEqual(pose_stats.measure_pose(plain)["exposure"], 30.0)
        both = write_frame(self.folder / "both.fit", image, EXPTIME=300.0, EXPOSURE=(10000, "[ms] Total Exposure Time"))
        self.assertEqual(pose_stats.measure_pose(both)["exposure"], 300.0)

    def test_the_command_line_prints_one_line_per_frame_and_survives_an_unreadable_one(self) -> None:
        good = write_frame(self.folder / "good.fit", star_field(120, 1.6, 1.6))
        broken = self.folder / "broken.fit"
        broken.write_bytes(b"not a fits file")
        listing = self.folder / "list.txt"
        listing.write_text(f"{broken}\n{good}\n", encoding="utf-8")
        out = io.StringIO()
        with redirect_stdout(out):
            code = pose_stats.main(["--frames-from", str(listing)])
        self.assertEqual(code, 0)
        lines = [json.loads(line) for line in out.getvalue().splitlines()]
        self.assertEqual([line["status"] for line in lines], ["unreadable", "measured"])
        self.assertEqual(lines[1]["path"], str(good))


if __name__ == "__main__":
    unittest.main()
