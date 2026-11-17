"""Tests of « Régler l'image » (image_adjust.py): each setting neutral leaves the image unchanged to the bit,
moves it the right way otherwise, and the image saved never replaces the render.

Run from this folder, with the stacking environment:
    python3 -m unittest -v test_image_adjust
"""

from __future__ import annotations

import io
import json
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


def synthetic(shape=(240, 320), seed=3):
    """A finished image: a grainy sky at 0.08, a bluish nebula, a few stars; and its star layer."""
    rng = np.random.default_rng(seed)
    yy, xx = np.mgrid[0:shape[0], 0:shape[1]]
    cloud = np.exp(-((yy - shape[0] / 2) ** 2 + (xx - shape[1] / 2) ** 2) / (2 * 45.0 ** 2))
    layer = np.zeros(shape)
    for _ in range(40):
        y, x, f = rng.uniform(8, shape[0] - 8), rng.uniform(8, shape[1] - 8), rng.uniform(0.2, 0.7)
        layer += f * np.exp(-((yy - y) ** 2 + (xx - x) ** 2) / (2 * 1.2 ** 2))
    grain = rng.normal(0, 0.012, shape)
    base = np.stack([0.08 + 0.25 * cloud, 0.08 + 0.3 * cloud, 0.09 + 0.45 * cloud], axis=-1) + grain[..., None]
    stars = np.repeat(np.clip(layer, 0, 1)[..., None], 3, axis=2)
    img = np.clip(base + stars, 0, 1)
    # As the image comes from a PNG: 8-bit levels.
    img = np.round(img * 255) / 255
    return img.astype(np.float32), np.clip(img - np.clip(base, 0, 1), 0, 1).astype(np.float32), cloud


@NEEDS_NUMPY
class Neutral(unittest.TestCase):
    def test_every_setting_at_its_neutral_value_leaves_the_image_unchanged_to_the_bit(self) -> None:
        import image_adjust as A

        img, layer, _ = synthetic()
        out = A.apply(img, dict(A.NEUTRAL), layer, None)
        self.assertTrue(np.array_equal(A.to_bytes(out, 8), A.to_bytes(img, 8)))
        for name in A.NEUTRAL:
            with self.subTest(name=name):
                one = A.apply(img, {name: A.NEUTRAL[name]}, layer, None)
                self.assertTrue(np.array_equal(one, img))

    def test_values_out_of_range_are_brought_back_and_a_word_is_refused(self) -> None:
        import image_adjust as A

        self.assertEqual(A.clean_values({"blackPoint": 99})["blackPoint"], 20.0)
        self.assertEqual(A.clean_values({"colour": -1})["colour"], 0.0)
        with self.assertRaises(A.AdjustError):
            A.clean_values({"brightness": "much"})


@NEEDS_NUMPY
class Direction(unittest.TestCase):
    def setUp(self) -> None:
        import image_adjust as A

        self.A = A
        self.img, self.layer, self.cloud = synthetic()
        self.sky = self.cloud < 0.05
        self.neb = self.cloud > 0.6

    def test_the_black_point_darkens_the_sky(self) -> None:
        out = self.A.apply(self.img, {"blackPoint": 6}, None, None)
        self.assertLess(self.A.lum(out)[self.sky].mean(), self.A.lum(self.img)[self.sky].mean() - 0.04)

    def test_brightness_lifts_or_lowers_the_middle_and_keeps_the_blacks(self) -> None:
        L = self.A.lum(self.img)[self.neb].mean()
        up = self.A.lum(self.A.apply(self.img, {"brightness": 12}, None, None))[self.neb].mean()
        down = self.A.lum(self.A.apply(self.img, {"brightness": -12}, None, None))[self.neb].mean()
        self.assertGreater(up, L)
        self.assertLess(down, L)
        black = np.zeros((4, 4, 3), np.float32)
        self.assertTrue(np.array_equal(self.A.brightness(black, 30), black))

    def test_colour_strengthens_the_nebula_and_leaves_the_sky_nearly_grey(self) -> None:
        def chroma(x):
            return (x.max(-1) - x.min(-1))

        out = self.A.apply(self.img, {"colour": 1.7}, None, None)
        self.assertGreater(chroma(out)[self.neb].mean(), 1.4 * chroma(self.img)[self.neb].mean())
        self.assertLess(chroma(out)[self.sky].mean(), 1.15 * chroma(self.img)[self.sky].mean() + 1e-3)
        grey = self.A.apply(self.img, {"colour": 0}, None, None)
        self.assertLess(chroma(grey)[self.neb].mean(), 0.2 * chroma(self.img)[self.neb].mean())

    def test_smoothing_reduces_the_grain_of_the_sky(self) -> None:
        out = self.A.apply(self.img, {"smoothing": 40}, None, None)
        self.assertLess(self.A.sky_noise(out), 0.8 * self.A.sky_noise(self.img))

    def test_stars_reduced_or_strengthened_without_touching_the_nebula_away_from_them(self) -> None:
        on_star = self.layer[..., 0] > 0.2
        clear = (self.layer[..., 0] < 1e-4) & self.neb
        less = self.A.apply(self.img, {"stars": -40}, self.layer, None)
        more = self.A.apply(self.img, {"stars": 30}, self.layer, None)
        self.assertLess(self.A.lum(less)[on_star].mean(), self.A.lum(self.img)[on_star].mean())
        self.assertGreater(self.A.lum(more)[on_star].mean(), self.A.lum(self.img)[on_star].mean())
        self.assertLess(np.abs(less - self.img)[clear].max(), 1e-3)

    def test_a_bright_coloured_pixel_is_never_darkened_so_no_ring_is_dug_around_a_star(self) -> None:
        """Brightness and colour change the luminance of a pixel by their curve alone (a monotone curve: a star whose
        profile falls in one piece keeps falling in one piece).  A bright, coloured halo pixel that a setting lifts
        past 1 on one channel goes towards white at its luminance; divided by that channel, it came out darker than
        its curve says, under the white core: a dark ring."""
        halo = np.array([[[0.55, 0.60, 1.00], [0.80, 0.85, 0.95], [1.0, 1.0, 1.0], [0.30, 0.32, 0.60]]], np.float32)
        L = self.A.lum(halo)
        lifted = self.A.apply(halo, {"brightness": 50}, None, None)
        np.testing.assert_allclose(self.A.lum(lifted), L ** 0.5, atol=1e-6)
        stronger = self.A.apply(halo, {"colour": 3.0}, None, np.ones(halo.shape[:2], np.float32))
        np.testing.assert_allclose(self.A.lum(stronger), L, atol=1e-6)
        self.assertTrue(np.all(stronger[..., 2] >= stronger[..., 1]), "the halo stays blue, its hue kept")

    def test_stars_without_their_layer_say_so(self) -> None:
        with self.assertRaises(self.A.AdjustError) as caught:
            self.A.apply(self.img, {"stars": -40}, None, None)
        self.assertEqual(caught.exception.code, "stars_missing")


@NEEDS_NUMPY
class Commands(unittest.TestCase):
    def run_main(self, *argv: str) -> tuple[int, dict]:
        import image_adjust as A

        buffer = io.StringIO()
        with redirect_stdout(buffer):
            code = A.main(list(argv))
        return code, json.loads(buffer.getvalue())

    def test_prepare_then_save_writes_beside_and_never_over_the_render(self) -> None:
        import image_adjust as A

        img, _, _ = synthetic((600, 1600))
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            render = root / "M 31_natural.png"
            Image.fromarray(A.to_bytes(img, 8), "RGB").save(render)
            before = render.read_bytes()
            work = root / "work"
            code, info = self.run_main("prepare", "--image", str(render), "--work", str(work))
            self.assertEqual(code, 0)
            self.assertEqual((info["previewWidth"], info["previewHeight"]), (1080, 405))
            self.assertTrue((work / "base.png").is_file() and (work / "mask.png").is_file())
            out = root / "adjusted" / "M 31_natural_adjusted.png"
            values = json.dumps({"blackPoint": 8, "brightness": 12, "colour": 1.7})
            code, saved = self.run_main("save", "--image", str(render), "--work", str(work), "--values", values,
                                        "--out", str(out))
            self.assertEqual(code, 0, saved)
            self.assertEqual(render.read_bytes(), before)
            written = np.asarray(Image.open(out).convert("RGB"))
            self.assertEqual(written.shape, img.shape)
            self.assertFalse(np.array_equal(written, A.to_bytes(img, 8)))
            code, refused = self.run_main("save", "--image", str(render), "--work", str(work), "--values", values,
                                          "--out", str(render))
            self.assertEqual((code, refused["error"]), (2, "would_overwrite"))
            self.assertEqual(render.read_bytes(), before)

    def test_the_preview_formulas_match_the_whole_image_ones(self) -> None:
        """The window applies the per-pixel settings on the preview; on the same pixels they give the same image."""
        import image_adjust as A

        img, layer, _ = synthetic()
        weight = A.colour_weight(img)
        values = {"blackPoint": 5, "brightness": -20, "colour": 2.2, "stars": -50}
        whole = A.apply(img, values, layer, weight)
        fixture = Path(__file__).resolve().parent.parent / "app/src/lib/imageAdjust.fixture.json"
        if not fixture.is_file():
            self.skipTest("the window's fixture is not there")
        data = json.loads(fixture.read_text(encoding="utf-8"))
        pixels = np.asarray(data["input"], np.float32).reshape(-1, 1, 3)
        stars_px = np.asarray(data["stars"], np.float32).reshape(-1, 1, 3)
        mask = np.asarray(data["mask"], np.float32).reshape(-1, 1)
        expected = np.asarray(data["output"], np.float32).reshape(-1, 1, 3)
        got = A.apply(pixels, data["values"], stars_px, mask)
        self.assertLess(float(np.abs(got - expected).max()), 1e-4)
        self.assertEqual(whole.shape, img.shape)


def deep_synthetic(shape=(240, 320), seed=5):
    """A finished image at full depth (not on 8-bit levels): the render's pixels before the stacker writes them."""
    rng = np.random.default_rng(seed)
    yy, xx = np.mgrid[0:shape[0], 0:shape[1]]
    cloud = np.exp(-((yy - shape[0] / 2) ** 2 + (xx - shape[1] / 2) ** 2) / (2 * 45.0 ** 2))
    grain = rng.normal(0, 0.006, shape)
    base = np.stack([0.06 + 0.25 * cloud, 0.06 + 0.3 * cloud, 0.07 + 0.45 * cloud], axis=-1) + grain[..., None]
    return np.clip(base, 0, 1)


def write_render_pair(folder: Path, rgb: np.ndarray, stem: str = "M 31_natural") -> tuple[Path, Path]:
    """The two files of a render as the stacker writes them (renders.save): a 16-bit TIFF and an 8-bit PNG."""
    import tifffile

    png = folder / f"{stem}.png"
    tif = folder / f"{stem}.tif"
    tifffile.imwrite(tif, np.round(rgb * 65535).astype(np.uint16), photometric="rgb", compression="deflate")
    Image.fromarray(np.round(rgb * 255).astype(np.uint8), "RGB").save(png)
    return png, tif


@NEEDS_NUMPY
class TwinTiff(unittest.TestCase):
    """A render written as PNG and TIFF: its image set is computed once from the 16-bit TIFF, and the PNG of the
    image set is that TIFF reduced to 8 bits."""

    def run_main(self, *argv: str) -> tuple[int, dict]:
        return Commands.run_main(self, *argv)

    def save_pair(self, root: Path, values: dict) -> tuple[Path, Path, Path, Path, dict]:
        png, tif = write_render_pair(root, deep_synthetic())
        work = root / "work"
        code, _ = self.run_main("prepare", "--image", str(png), "--work", str(work))
        self.assertEqual(code, 0)
        out = root / "adjusted" / "M 31_natural_adjusted.png"
        out_tif = root / "adjusted" / "M 31_natural_adjusted.tif"
        code, saved = self.run_main("save", "--image", str(png), "--work", str(work), "--values", json.dumps(values),
                                    "--out", str(out), "--tiff", str(tif), "--out-tiff", str(out_tif))
        self.assertEqual(code, 0, saved)
        self.assertEqual(saved["tiff"], str(out_tif))
        return png, tif, out, out_tif, saved

    def test_the_tiff_reduced_to_8_bits_is_the_png_to_the_bit(self) -> None:
        import tifffile
        import image_adjust as A

        values = {"blackPoint": 6, "brightness": 25, "colour": 1.8, "smoothing": 30}
        with tempfile.TemporaryDirectory() as folder:
            png, tif, out, out_tif, _ = self.save_pair(Path(folder), values)
            deep = tifffile.imread(out_tif)
            eight = np.asarray(Image.open(out).convert("RGB"))
            self.assertEqual(deep.dtype, np.uint16)
            self.assertEqual(deep.shape, eight.shape)
            # Rounded reduction (the one the PNG is written with): no difference at all.
            self.assertTrue(np.array_equal(A.eight_bit_of(deep), eight))
            self.assertTrue(np.array_equal(np.round(deep / 65535.0 * 255.0).astype(np.uint8), eight))
            # A reduction that truncates (the high byte) is at most one level under.
            self.assertLessEqual(int(np.abs((deep >> 8).astype(int) - eight.astype(int)).max()), 1)

    def test_the_tiff_keeps_its_depth_and_is_the_settings_applied_to_the_16_bit_data(self) -> None:
        import tifffile
        import image_adjust as A

        values = {"blackPoint": 4, "brightness": -15, "colour": 1.5}
        with tempfile.TemporaryDirectory() as folder:
            png, tif, out, out_tif, _ = self.save_pair(Path(folder), values)
            deep = tifffile.imread(out_tif)
            # Not the PNG widened: far more than 256 levels, and levels between two 8-bit ones.
            self.assertGreater(len(np.unique(deep[..., 1])), 1000)
            self.assertTrue(np.any(deep % 257 != 0))
            expected = A.to_bytes(A.apply(A.read_tiff(tif), values, None, None), 16)
            self.assertTrue(np.array_equal(deep, expected))
            # The same image as the image set computed from the 8-bit PNG alone, to the rounding of that PNG.
            from_png = A.to_bytes(A.apply(A.read_image(png)[0], values, None, None), 8).astype(int)
            self.assertLess(float(np.abs(from_png - np.asarray(Image.open(out).convert("RGB"))).mean()), 0.6)

    def test_the_render_and_its_tiff_are_never_written_over(self) -> None:
        values = json.dumps({"blackPoint": 5})
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            png, tif = write_render_pair(root, deep_synthetic())
            before = (png.read_bytes(), tif.read_bytes())
            work = root / "work"
            self.run_main("prepare", "--image", str(png), "--work", str(work))
            out = root / "adjusted" / "x_adjusted.png"
            code, refused = self.run_main("save", "--image", str(png), "--work", str(work), "--values", values,
                                          "--out", str(out), "--tiff", str(tif), "--out-tiff", str(tif))
            self.assertEqual((code, refused["error"]), (2, "would_overwrite"))
            code, refused = self.run_main("save", "--image", str(png), "--work", str(work), "--values", values,
                                          "--out", str(out), "--tiff", str(tif))
            self.assertEqual((code, refused["error"]), (2, "bad_arguments"))
            self.assertEqual((png.read_bytes(), tif.read_bytes()), before)
            self.assertFalse(out.exists())

    def test_a_tiff_that_is_not_the_same_image_is_refused(self) -> None:
        import tifffile

        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            png, tif = write_render_pair(root, deep_synthetic())
            tifffile.imwrite(tif, np.zeros((10, 10, 3), np.uint16), photometric="rgb")
            work = root / "work"
            self.run_main("prepare", "--image", str(png), "--work", str(work))
            code, refused = self.run_main("save", "--image", str(png), "--work", str(work), "--values",
                                          json.dumps({"blackPoint": 5}), "--out", str(root / "a" / "x_adjusted.png"),
                                          "--tiff", str(tif), "--out-tiff", str(root / "a" / "x_adjusted.tif"))
            self.assertEqual((code, refused["error"]), (2, "twin_mismatch"))


@NEEDS_NUMPY
class Turned(unittest.TestCase):
    """A quarter turn is written into the image: pixels moved, never recomputed, the PNG still the TIFF's reduction."""

    run_main = Commands.run_main
    save_pair = TwinTiff.save_pair

    def test_a_turn_moves_the_pixels_clockwise_and_swaps_the_sides(self) -> None:
        import image_adjust as A

        # 2 rows × 3 columns, each pixel its own level so a misplaced one shows.
        grid = np.arange(2 * 3 * 3, dtype=np.uint16).reshape(2, 3, 3)
        right = A.turn_of(grid, 1)
        self.assertEqual(right.shape, (3, 2, 3))
        # Clockwise: the top-left pixel goes to the top-right, the bottom-left to the top-left.
        self.assertTrue(np.array_equal(right[0, 1], grid[0, 0]))
        self.assertTrue(np.array_equal(right[0, 0], grid[1, 0]))
        self.assertTrue(np.array_equal(right[2, 1], grid[0, 2]))
        half = A.turn_of(grid, 2)
        self.assertEqual(half.shape, grid.shape)
        self.assertTrue(np.array_equal(half[0, 0], grid[1, 2]))
        left = A.turn_of(grid, 3)
        self.assertEqual(left.shape, (3, 2, 3))
        self.assertTrue(np.array_equal(left[0, 0], grid[0, 2]))
        # Four turns, or none: the same array.
        self.assertTrue(np.array_equal(A.turn_of(A.turn_of(grid, 1), 3), grid))
        self.assertTrue(A.turn_of(grid, 0) is grid)

    def test_a_turn_alone_writes_the_render_turned_to_the_bit(self) -> None:
        import tifffile
        import image_adjust as A

        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            png, tif = write_render_pair(root, deep_synthetic((200, 300)))
            work = root / "work"
            self.run_main("prepare", "--image", str(png), "--work", str(work))
            out = root / "adjusted" / "M 31_natural_adjusted.png"
            out_tif = root / "adjusted" / "M 31_natural_adjusted.tif"
            code, saved = self.run_main("save", "--image", str(png), "--work", str(work), "--values", "{}", "--out",
                                        str(out), "--tiff", str(tif), "--out-tiff", str(out_tif), "--turns", "1")
            self.assertEqual(code, 0, saved)
            self.assertEqual((saved["width"], saved["height"]), (200, 300))
            deep = tifffile.imread(out_tif)
            eight = np.asarray(Image.open(out).convert("RGB"))
            self.assertEqual(deep.shape, (300, 200, 3))
            # The render's own 16-bit pixels, only moved.
            self.assertTrue(np.array_equal(deep, A.turn_of(tifffile.imread(tif), 1)))
            # The PNG is still the exact reduction of the TIFF: the same image, turned.
            self.assertTrue(np.array_equal(A.eight_bit_of(deep), eight))

    def test_settings_then_a_turn_equal_the_turned_image_set(self) -> None:
        import tifffile
        import image_adjust as A

        values = {"blackPoint": 6, "brightness": 20, "colour": 1.6, "smoothing": 25}
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            (root / "a").mkdir()
            (root / "b").mkdir()
            _, _, _, flat_tif, _ = self.save_pair(root / "a", values)
            png, tif = write_render_pair(root / "b", deep_synthetic())
            work = root / "b" / "work"
            self.run_main("prepare", "--image", str(png), "--work", str(work))
            out = root / "b" / "adjusted" / "M 31_natural_adjusted.png"
            out_tif = root / "b" / "adjusted" / "M 31_natural_adjusted.tif"
            code, saved = self.run_main("save", "--image", str(png), "--work", str(work), "--values",
                                        json.dumps(values), "--out", str(out), "--tiff", str(tif), "--out-tiff",
                                        str(out_tif), "--turns", "3")
            self.assertEqual(code, 0, saved)
            self.assertTrue(np.array_equal(tifffile.imread(out_tif), A.turn_of(tifffile.imread(flat_tif), 3)))

    def test_a_turn_of_a_png_alone_and_a_wrong_number_of_turns(self) -> None:
        import image_adjust as A

        img, _, _ = synthetic((100, 160))
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            render = root / "M 31_natural.png"
            Image.fromarray(A.to_bytes(img, 8), "RGB").save(render)
            work = root / "work"
            self.run_main("prepare", "--image", str(render), "--work", str(work))
            out = root / "adjusted" / "M 31_natural_adjusted.png"
            code, saved = self.run_main("save", "--image", str(render), "--work", str(work), "--values", "{}",
                                        "--out", str(out), "--turns", "2")
            self.assertEqual(code, 0, saved)
            written = np.asarray(Image.open(out).convert("RGB"))
            self.assertTrue(np.array_equal(written, A.to_bytes(img, 8)[::-1, ::-1]))
            code, refused = self.run_main("save", "--image", str(render), "--work", str(work), "--values", "{}",
                                          "--out", str(root / "adjusted" / "other.png"), "--turns", "4")
            self.assertEqual((code, refused["error"]), (2, "bad_arguments"))

    def test_the_image_set_carries_no_sky_solution(self) -> None:
        """The solution of an image is computed on the file that is sent: the files written hold no WCS card."""
        import tifffile

        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            png, tif = write_render_pair(root, deep_synthetic())
            work = root / "work"
            self.run_main("prepare", "--image", str(png), "--work", str(work))
            out = root / "adjusted" / "M 31_natural_adjusted.png"
            out_tif = root / "adjusted" / "M 31_natural_adjusted.tif"
            self.run_main("save", "--image", str(png), "--work", str(work), "--values", "{}", "--out", str(out),
                          "--tiff", str(tif), "--out-tiff", str(out_tif), "--turns", "1")
            with tifffile.TiffFile(out_tif) as handle:
                description = handle.pages[0].description or ""
            for card in ("CRVAL1", "CD1_1", "CDELT1", "CTYPE1"):
                self.assertNotIn(card, description)
            self.assertFalse(out.with_suffix(".wcs").exists())


if __name__ == "__main__":
    unittest.main()
