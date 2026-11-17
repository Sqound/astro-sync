"""Tests of the diffraction spikes of « Régler l'image » (diffraction_spikes.py and their place in image_adjust.py):
off, the image comes out unchanged to the bit; on, light is only added, along the two axes, and a quarter turn commutes
with the drawing; the window's drawing and this one agree (the fixture the window's test reads); the whole image set is
written with them, the PNG still the exact reduction of the TIFF.

Run from this folder, with the stacking environment:
    python3 -m unittest -v test_diffraction_spikes
Set WRITE_SPIKES_FIXTURE=1 to rewrite the window's fixture (``app/src/lib/diffractionSpikes.fixture.json``).
"""

from __future__ import annotations

import io
import json
import os
import tempfile
import unittest
from contextlib import redirect_stdout
from pathlib import Path

try:
    import numpy as np
    from PIL import Image
except ImportError:  # the stacking environment has them; a bare interpreter does not
    np = Image = None

NEEDS_NUMPY = unittest.skipIf(np is None, "needs numpy and pillow (the stacking environment)")

FIXTURE = Path(__file__).resolve().parent.parent / "app/src/lib/diffractionSpikes.fixture.json"

# A few stars at full size (x, y, flux, radius, colour): bright and white, bright and orange, medium and blue, and a
# faint one that no reach lets through, and one near the border.
STARS = [
    (60.0, 50.0, 55.0, 2.5, (1.0, 1.0, 1.0)),
    (150.0, 90.0, 30.0, 2.0, (1.2, 1.0, 0.7)),
    (110.0, 140.0, 9.0, 1.6, (0.8, 0.95, 1.2)),
    (30.0, 120.0, 1.0, 1.2, (1.0, 1.0, 1.0)),
    (4.0, 20.0, 20.0, 2.0, (1.0, 1.0, 1.0)),
]


def star_list(rows=STARS) -> dict:
    return {"x": np.array([r[0] for r in rows]), "y": np.array([r[1] for r in rows]),
            "flux": np.array([r[2] for r in rows]), "radius": np.array([r[3] for r in rows]),
            "colour": np.array([r[4] for r in rows], dtype=np.float64),
            "saturated": np.zeros(len(rows), dtype=bool)}


def sky(shape=(160, 200), seed=4) -> np.ndarray:
    rng = np.random.default_rng(seed)
    base = 0.07 + rng.normal(0, 0.01, shape + (1,)) + np.array([0.0, 0.01, 0.03])
    return np.clip(np.round(base * 255) / 255, 0, 1).astype(np.float32)


@NEEDS_NUMPY
class Off(unittest.TestCase):
    def test_no_strength_leaves_the_image_unchanged_to_the_bit(self) -> None:
        import image_adjust as A

        img = sky()
        spots = star_list()
        for reach in (0, 50, 100):
            with self.subTest(reach=reach):
                out = A.apply(img, {"spikes": 0, "spikesReach": reach}, None, None, spots)
                self.assertTrue(np.array_equal(out, img))
                self.assertTrue(out is img)

    def test_a_pixel_the_spikes_do_not_reach_keeps_its_exact_value(self) -> None:
        import diffraction_spikes as D

        img = sky()
        light = D.spike_layer(img.shape[:2], star_list(), 60, 50)
        out = D.with_spikes(img, light)
        untouched = ~(light > 0).any(-1)
        self.assertTrue(untouched.any())
        self.assertTrue(np.array_equal(out[untouched], img[untouched]))

    def test_all_the_stars_taken_away_takes_their_spikes_away(self) -> None:
        import image_adjust as A

        img = sky()
        out = A.apply(img, {"spikes": 80, "spikesReach": 100, "stars": -100}, np.zeros_like(img), None, None)
        self.assertTrue(np.array_equal(out, img))

    def test_spikes_without_the_list_of_stars_are_refused(self) -> None:
        import image_adjust as A

        with self.assertRaises(A.AdjustError) as raised:
            A.apply(sky(), {"spikes": 50}, None, None, None)
        self.assertEqual(raised.exception.code, "stars_missing")

    def test_the_reach_means_nothing_without_spikes(self) -> None:
        import image_adjust as A

        self.assertEqual(A.clean_values({"spikes": 0, "spikesReach": 10})["spikesReach"], 50.0)
        self.assertEqual(A.clean_values({"spikes": 20, "spikesReach": 10})["spikesReach"], 10.0)
        self.assertEqual(A.clean_values({"spikes": 500, "spikesReach": 900})["spikes"], 100.0)
        self.assertEqual(A.clean_values({})["spikes"], 0.0)


@NEEDS_NUMPY
class Drawing(unittest.TestCase):
    def test_light_is_only_added_and_follows_the_two_axes(self) -> None:
        import diffraction_spikes as D

        img = sky()
        light = D.spike_layer(img.shape[:2], star_list(), 70, 50)
        out = D.with_spikes(img, light)
        self.assertTrue((out >= img - 1e-7).all())
        self.assertLessEqual(float(out.max()), 1.0)
        # Right of the brightest star on its row, and under it on its column: lit; on the diagonal: not.
        self.assertGreater(float(light[50, 70].max()), 0.02)
        self.assertGreater(float(light[62, 60].max()), 0.02)
        self.assertEqual(float(light[50 + 12, 60 + 12].max()), 0.0)
        # The faint star (flux 1) is under every reach: nothing around it.
        self.assertEqual(float(light[120, 38:80].max()), 0.0)

    def test_a_longer_reach_lets_more_stars_through(self) -> None:
        import diffraction_spikes as D

        spots = star_list()
        counts = [D.star_count(spots, r) for r in (0, 50, 100)]
        self.assertEqual(counts, sorted(counts))
        self.assertLess(counts[0], counts[-1])
        lit = [float((D.spike_layer((160, 200), spots, 70, r) > 0).sum()) for r in (0, 50, 100)]
        self.assertEqual(lit, sorted(lit))

    def test_a_quarter_turn_commutes_with_the_drawing(self) -> None:
        import diffraction_spikes as D
        import image_adjust as A

        side = 180
        spots = star_list()
        light = D.spike_layer((side, side), spots, 75, 80)
        turned_after = A.turn_of(light, 1)
        # Turned clockwise, a pixel (x, y) goes to (side - 1 - y, x).
        spots_turned = {**spots, "x": side - 1 - spots["y"], "y": spots["x"].copy()}
        turned_before = D.spike_layer((side, side), spots_turned, 75, 80)
        self.assertLess(float(np.abs(turned_after - turned_before).max()), 1e-5)

    def test_the_same_values_make_the_same_picture_at_two_sizes(self) -> None:
        import diffraction_spikes as D

        full = D.spike_layer((400, 400), star_list(), 70, 60)
        half = D.spike_layer((200, 200), star_list(), 70, 60, scale=0.5)
        shrunk = full.reshape(200, 2, 200, 2, 3).mean(axis=(1, 3))
        self.assertLess(float(np.abs(shrunk - half).mean()), 0.004)

    def test_the_list_of_stars_goes_to_a_file_and_comes_back(self) -> None:
        import diffraction_spikes as D

        spots = star_list()
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / "stars.json"
            D.write_stars(spots, path)
            back = D.read_stars(path)
            # The faint star (flux 1 < 3) can never get a spike: it is not kept.
            self.assertEqual(len(back["flux"]), 4)
            light = D.spike_layer((160, 200), spots, 70, 100)
            self.assertTrue(np.array_equal(D.spike_layer((160, 200), back, 70, 100), light))
            D.write_stars(D.read_stars(path), path)
            self.assertEqual(json.loads(path.read_text())["version"], D.FILE_VERSION)

    def test_the_window_draws_what_the_script_draws(self) -> None:
        """The fixture is what the window's own test (``diffractionSpikes.test.ts``) reads."""
        import diffraction_spikes as D

        shape = (48, 64)
        cases = []
        for strength, reach, scale, factor in ((60, 50, 0.25, 1.0), (100, 100, 0.25, 1.5), (30, 0, 0.3, 1.0)):
            light = D.spike_layer(shape, star_list(), strength, reach, scale, factor)
            cases.append({"strength": strength, "reach": reach, "scale": scale, "starFactor": factor,
                          "layer": [round(float(v), 6) for v in light.ravel()]})
        data = {"width": shape[1], "height": shape[0],
                "thresholds": {str(r): D.flux_threshold(r) for r in (0, 25, 50, 75, 100)},
                "stars": [{"x": r[0], "y": r[1], "flux": r[2], "radius": r[3], "colour": list(r[4])} for r in STARS],
                "cases": cases}
        if os.environ.get("WRITE_SPIKES_FIXTURE"):
            FIXTURE.write_text(json.dumps(data, separators=(",", ":")) + "\n", encoding="utf-8")
        if not FIXTURE.is_file():
            self.skipTest("the window's fixture is not there")
        self.assertEqual(json.loads(FIXTURE.read_text(encoding="utf-8")), data)


@NEEDS_NUMPY
class Saved(unittest.TestCase):
    """The whole image set, from the render and its TIFF twin, with a star layer and its list."""

    def run_main(self, *argv: str) -> tuple[int, dict]:
        import image_adjust as A

        out = io.StringIO()
        with redirect_stdout(out):
            code = A.main(list(argv))
        return code, json.loads(out.getvalue())

    def render(self, root: Path) -> tuple[Path, Path, Path]:
        import tifffile
        import diffraction_spikes as D

        rgb = np.clip(sky((160, 200)) + 0.0, 0, 1).astype(np.float64)
        png, tif = root / "R.png", root / "R.tif"
        tifffile.imwrite(tif, np.round(rgb * 65535).astype(np.uint16), photometric="rgb", compression="deflate")
        Image.fromarray(np.round(rgb * 255).astype(np.uint8), "RGB").save(png)
        work = root / "work"
        code, _ = self.run_main("prepare", "--image", str(png), "--work", str(work))
        self.assertEqual(code, 0)
        # A star layer as the separation leaves it (a star at each place of the list), and the list.
        layer = np.zeros((160, 200, 3), np.float32)
        for x, y, *_ in STARS:
            layer[int(y), int(x)] = 0.5
        tifffile.imwrite(work / "stars.tif", np.round(layer * 65535).astype(np.uint16), photometric="rgb")
        D.write_stars(star_list(), work / "stars.json")
        return png, tif, work

    def save(self, root: Path, values: dict, *extra: str) -> tuple[int, dict]:
        png, tif, work = root / "R.png", root / "R.tif", root / "work"
        return self.run_main("save", "--image", str(png), "--work", str(work), "--values", json.dumps(values),
                             "--out", str(root / "a" / "R_adjusted.png"), "--tiff", str(tif),
                             "--out-tiff", str(root / "a" / "R_adjusted.tif"), *extra)

    def test_the_spikes_are_written_into_the_png_and_its_tiff_twin(self) -> None:
        import tifffile
        import image_adjust as A

        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            png, tif, work = self.render(root)
            code, saved = self.save(root, {"spikes": 70, "spikesReach": 60})
            self.assertEqual(code, 0, saved)
            deep = tifffile.imread(root / "a" / "R_adjusted.tif")
            eight = np.asarray(Image.open(root / "a" / "R_adjusted.png").convert("RGB"))
            self.assertTrue(np.array_equal(A.eight_bit_of(deep), eight))
            original = tifffile.imread(tif)
            self.assertTrue((deep.astype(int) >= original.astype(int)).all())
            self.assertTrue((deep != original).any())
            # Far from every spike: not one level changed.
            self.assertTrue(np.array_equal(deep[130:, 170:], original[130:, 170:]))

    def test_with_the_spikes_off_the_image_set_is_the_render_to_the_bit(self) -> None:
        import tifffile

        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            png, tif, work = self.render(root)
            # Only the quarter turn is asked (a turn alone is written); the reach alone moves nothing.
            code, saved = self.save(root, {"spikes": 0, "spikesReach": 90}, "--turns", "0")
            self.assertEqual(code, 0, saved)
            self.assertTrue(np.array_equal(tifffile.imread(root / "a" / "R_adjusted.tif"), tifffile.imread(tif)))
            self.assertTrue(np.array_equal(np.asarray(Image.open(root / "a" / "R_adjusted.png")),
                                           np.asarray(Image.open(png).convert("RGB"))))

    def test_a_turn_after_the_spikes_is_the_spikes_on_the_turned_image(self) -> None:
        import tifffile
        import image_adjust as A

        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            self.render(root)
            self.assertEqual(self.save(root, {"spikes": 70})[0], 0)
            straight = tifffile.imread(root / "a" / "R_adjusted.tif")
            self.assertEqual(self.save(root, {"spikes": 70}, "--turns", "1")[0], 0)
            turned = tifffile.imread(root / "a" / "R_adjusted.tif")
            self.assertTrue(np.array_equal(turned, A.turn_of(straight, 1)))

    def test_a_star_layer_from_before_the_list_gets_its_list_without_separating_again(self) -> None:
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            png, tif, work = self.render(root)
            (work / "stars.json").unlink()
            code, answer = self.run_main("spots", "--image", str(png), "--work", str(work), "--tiff", str(tif))
            self.assertEqual(code, 0, answer)
            self.assertTrue((work / "stars.json").is_file())
            (work / "stars.json").unlink()
            # The saving does the same by itself.
            code, saved = self.save(root, {"spikes": 50})
            self.assertEqual(code, 0, saved)
            self.assertTrue((work / "stars.json").is_file())

    def test_no_star_layer_no_spikes(self) -> None:
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            png, tif, work = self.render(root)
            (work / "stars.json").unlink()
            (work / "stars.tif").unlink()
            code, refused = self.save(root, {"spikes": 50})
            self.assertEqual((code, refused["error"]), (2, "stars_missing"))
            self.assertFalse((root / "a" / "R_adjusted.png").exists())


if __name__ == "__main__":
    unittest.main()
