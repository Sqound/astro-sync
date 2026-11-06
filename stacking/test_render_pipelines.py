#!/usr/bin/env python3
"""The "Polished render" (``polished``) and "HOO palette" (``hoo``) renders.

On synthetic images: the steps of the polished preparation on a gradient, with
a stand-in GraXpert, a stand-in Siril and without either (the polynomial, no
denoising, and it is said), the sky level it keeps, the SCNR it skips
on a dual-band capture, the deconvolution floor, and the HOO palette on a
synthetic dual-band nebula.
"""

from __future__ import annotations

import argparse
import io
import json
import shlex
import shutil
import stat
import sys
import tempfile
import unittest
from contextlib import redirect_stdout
from pathlib import Path
from unittest import mock

import numpy as np
from astropy.io import fits

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))

import astro_sync_stack as stack  # noqa: E402
import pipeline_defs  # noqa: E402
import renders  # noqa: E402

SHAPE = (160, 200)
# The polynomial samples the sky by tiles of 160 px: a master of 3 x 4 tiles.
MASTER = (480, 640)


def blob(shape: tuple[int, int], centre: tuple[float, float], sigma: float) -> np.ndarray:
    yy, xx = np.mgrid[: shape[0], : shape[1]]
    return np.exp(-((yy - centre[0]) ** 2 + (xx - centre[1]) ** 2) / (2 * sigma * sigma))


def gradient_master(root: Path, shape: tuple[int, int] = MASTER) -> tuple[Path, np.ndarray]:
    """A linear RGB master: a sky with a strong left-to-right gradient, a reddish cloud, a little noise."""
    rng = np.random.default_rng(3)
    xx = np.mgrid[: shape[0], : shape[1]][1] / shape[1]
    sky = 0.02 + 0.03 * xx
    cloud = blob(shape, (shape[0] // 2, shape[1] // 2), 40)
    rgb = np.stack([sky + 0.20 * cloud, sky + 0.12 * cloud, sky + 0.09 * cloud])
    rgb = rgb + rng.normal(0, 0.0008, rgb.shape)
    path = root / "master.fit"
    fits.PrimaryHDU(rgb.astype(np.float32)).writeto(path)
    return path, cloud


def stand_in_tool(root: Path, name: str, body: str) -> Path:
    """Write a stand-in command-line tool: ``<name>.py`` holds ``body``, and ``<name>`` is a tiny shell
    wrapper that runs it with this very interpreter.  A ``#!`` line cannot carry a path with a space (the
    application's own Python lives under "Application Support"), so the interpreter is quoted in the wrapper."""
    (root / f"{name}.py").write_text(body, encoding="utf-8")
    script = root / name
    script.write_text(
        f"#!/bin/sh\nexec {shlex.quote(sys.executable)} {shlex.quote(str(root / f'{name}.py'))} \"$@\"\n",
        encoding="utf-8",
    )
    script.chmod(script.stat().st_mode | stat.S_IXUSR)
    return script


def fake_graxpert(root: Path, fail: bool = False, pedestal: float = 0.0) -> Path:
    """A stand-in for GraXpert's command line: records its arguments, subtracts a
    fitted plane (background extraction) or smooths a little (denoising), and
    writes ``<output>.fits`` as GraXpert does."""
    record = root / "graxpert-calls.jsonl"
    return stand_in_tool(
        root,
        "graxpert",
        "import json, sys\n"
        "import numpy as np\n"
        "from astropy.io import fits\n"
        "from scipy.ndimage import gaussian_filter\n"
        f"PEDESTAL = {pedestal!r}\n"
        f"open({str(record)!r}, 'a').write(json.dumps(sys.argv[1:]) + '\\n')\n"
        f"if {fail!r}:\n"
        "    sys.exit(3)\n"
        "args = sys.argv[1:]\n"
        "command, output, source = args[args.index('-cmd') + 1], args[args.index('-output') + 1], args[-1]\n"
        "data = fits.getdata(source).astype(np.float32)\n"
        "if command == 'background-extraction':\n"
        "    xx = np.mgrid[:data.shape[1], :data.shape[2]][1] / data.shape[2]\n"
        "    data = data - 0.03 * xx[None] - PEDESTAL\n"
        "else:\n"
        "    data = gaussian_filter(data, sigma=(0, 0.7, 0.7))\n"
        "fits.PrimaryHDU(data).writeto(output + '.fits', overwrite=True)\n",
    )


def fake_starnet(root: Path) -> Path:
    """A stand-in for StarNet2: ``-i <tif> -o <starless> -m <mask>``; keeps 60 % of the natural image as starless
    and 80 % of the polished one -- it leaves more of the halos in a denoised image, as the real one does.
    Like the real one, it writes a two-dimensional TIFF for an input whose three channels are all equal."""
    return stand_in_tool(
        root,
        "starnet2",
        "import sys\n"
        "import numpy as np\n"
        "import tifffile\n"
        "args = sys.argv[1:]\n"
        "source, starless, mask = args[args.index('-i') + 1], args[args.index('-o') + 1], args[args.index('-m') + 1]\n"
        "data = tifffile.imread(source).astype(np.float32)\n"
        "keep = 0.8 if 'polished' in source else 0.6\n"
        "starless_data = np.round(data * keep).astype(np.uint16)\n"
        "mask_data = np.zeros_like(data).astype(np.uint16)\n"
        "if np.array_equal(data[..., 0], data[..., 1]) and np.array_equal(data[..., 0], data[..., 2]):\n"
        "    # the real one: \"Grayscale image is detected!\" and a two-dimensional TIFF comes out\n"
        "    tifffile.imwrite(starless, starless_data[..., 0])\n"
        "    tifffile.imwrite(mask, mask_data[..., 0])\n"
        "else:\n"
        "    tifffile.imwrite(starless, starless_data, photometric='rgb')\n"
        "    tifffile.imwrite(mask, mask_data, photometric='rgb')\n",
    )


def fake_siril(root: Path, fail: bool = False, pedestal: float = 0.0) -> Path:
    """A stand-in for Siril's command line (``siril -d <folder> -s <script>``): reads the script,
    records it, and does what ``load``, ``subsky -rbf``, ``denoise`` and ``save`` ask on a FITS."""
    record = root / "siril-calls.jsonl"
    return stand_in_tool(
        root,
        "siril",
        "import json, shlex, sys\n"
        "from pathlib import Path\n"
        "import numpy as np\n"
        "from astropy.io import fits\n"
        "from scipy.ndimage import gaussian_filter\n"
        f"PEDESTAL = {pedestal!r}\n"
        "args = sys.argv[1:]\n"
        "folder, script = Path(args[args.index('-d') + 1]), Path(args[args.index('-s') + 1])\n"
        "lines = script.read_text().splitlines()\n"
        f"open({str(record)!r}, 'a').write(json.dumps({{'cwd': str(folder), 'script': lines, 'args': args}}) + '\\n')\n"
        f"if {fail!r}:\n"
        "    sys.exit(1)\n"
        "data = None\n"
        "for line in lines:\n"
        "    words = shlex.split(line)\n"
        "    if words[0] == 'load':\n"
        "        data = fits.getdata(words[1]).astype(np.float32)\n"
        "    elif words[0] == 'subsky':\n"
        "        xx = np.mgrid[:data.shape[1], :data.shape[2]][1] / data.shape[2]\n"
        "        data = data - 0.03 * xx[None] - PEDESTAL\n"
        "    elif words[0] == 'denoise':\n"
        "        data = gaussian_filter(data, sigma=(0, 0.7, 0.7))\n"
        "    elif words[0] == 'save':\n"
        "        fits.PrimaryHDU(data).writeto(words[1] + '.fit', overwrite=True)\n",
    )


def siril_calls(root: Path) -> list[dict[str, object]]:
    path = root / "siril-calls.jsonl"
    return [json.loads(line) for line in path.read_text(encoding="utf-8").splitlines()] if path.exists() else []


def calls(root: Path) -> list[list[str]]:
    path = root / "graxpert-calls.jsonl"
    return [json.loads(line) for line in path.read_text(encoding="utf-8").splitlines()] if path.exists() else []


class Table(unittest.TestCase):
    def test_the_two_renders_are_shipped_and_the_polished_one_has_its_own_preparation(self) -> None:
        self.assertTrue({"polished_deep", "hoo_deep"} <= set(renders.RENDERS))
        self.assertTrue({"polished_deep", "hoo_deep"} <= pipeline_defs.shipped_render_names())
        self.assertEqual(renders.preparation_of("polished_deep"), "polished")
        for name in ("natural_deep", "dualband_deep", "hoo_deep", None):
            self.assertEqual(renders.preparation_of(name), "native")


class PolishedSteps(unittest.TestCase):
    def setUp(self) -> None:
        self.root = Path(tempfile.mkdtemp())
        self.addCleanup(lambda: shutil.rmtree(self.root, ignore_errors=True))

    def test_the_shoulder_never_burns_and_leaves_the_tones_below_it(self) -> None:
        x = np.linspace(0, 1.6, 1601, dtype=np.float32)
        y = renders.soft_shoulder(x, 0.7)
        below = x <= 0.7
        np.testing.assert_allclose(y[below], x[below], atol=1e-6)
        self.assertTrue(np.all(np.diff(y) > 0), "increasing")
        self.assertLess(float(y.max()), 1.0)
        # The same slope on both sides of the knee: no visible step.
        self.assertAlmostEqual(float(y[701] - y[700]), float(x[701] - x[700]), places=4)

    def test_scnr_takes_back_half_of_the_green_excess_and_nothing_else(self) -> None:
        pixels = np.array([[[0.3, 0.3, 0.3], [0.2, 0.5, 0.2], [0.6, 0.2, 0.3], [0.1, 0.3, 0.5]]], np.float32)
        out = renders.scnr_green(pixels)
        np.testing.assert_allclose(out[0, 0], pixels[0, 0])  # grey
        np.testing.assert_allclose(out[0, 1], [0.2, 0.35, 0.2], atol=1e-6)  # green cast: 0.3 of excess, half taken back
        np.testing.assert_allclose(out[0, 2], pixels[0, 2])  # red: no green excess
        np.testing.assert_allclose(out[0, 3], pixels[0, 3])  # blue-green: green under the mean of red and blue
        with self.assertRaises(ValueError):
            renders.scnr_green(pixels, 1.5)

    def test_without_graxpert_the_polynomial_removes_the_gradient_and_it_is_said(self) -> None:
        master, cloud = gradient_master(self.root)
        printed = io.StringIO()
        with redirect_stdout(printed):
            image, done = renders.prepare_polished(master, None, self.root / "work", gradient_degree=1, black_percentile=0.05)
        self.assertEqual((done["engine"], done["background"], done["denoise"]), ("polynomial", "polynomial", None))
        self.assertEqual(done["engineRequested"], "graxpert-ai")
        self.assertIsNone(done["deconvolutionFwhm"])
        self.assertIn("GraXpert not given", printed.getvalue())
        self.assertEqual(image.shape, (*MASTER, 3))
        self.assertGreaterEqual(float(image.min()), 0.0)
        self.assertLessEqual(float(image.max()), 1.0)
        sky = renders.lum(image)[cloud < 0.01]
        columns = np.mgrid[: MASTER[0], : MASTER[1]][1][cloud < 0.01]
        left, right = sky[columns < 60].mean(), sky[columns > MASTER[1] - 60].mean()
        self.assertLess(abs(float(left - right)), 0.03, "the gradient is gone")
        self.assertFalse((self.root / "work").exists(), "nothing written without GraXpert")

    def test_with_graxpert_its_two_steps_run_pinned_in_the_work_folder(self) -> None:
        master, cloud = gradient_master(self.root)
        graxpert = fake_graxpert(self.root)
        before = sorted(p.name for p in self.root.iterdir())
        with redirect_stdout(io.StringIO()):
            image, done = renders.prepare_polished(master, graxpert, self.root / "work", "N_polished")
        self.assertEqual((done["background"], done["denoise"]), ("graxpert", "graxpert"))
        background, denoise = calls(self.root)
        self.assertEqual(background[:3], ["-cli", "-cmd", "background-extraction"])
        self.assertIn(renders.GRAXPERT_BACKGROUND_MODEL, background)
        self.assertEqual(background[-1], str(master))
        self.assertEqual(denoise[:3], ["-cli", "-cmd", "denoising"])
        self.assertEqual(denoise[denoise.index("-strength") + 1], "0.3")
        self.assertIn(renders.GRAXPERT_DENOISE_MODEL, denoise)
        self.assertEqual(denoise[-1], str((self.root / "work" / "N_polished_graxpert_background.fits").resolve()))
        # Everything went to the work folder: nothing beside the master.
        self.assertEqual(sorted(p.name for p in self.root.iterdir()), sorted([*before, "graxpert-calls.jsonl", "work"]))
        self.assertTrue((self.root / "work" / "N_polished_graxpert_denoised.fits").is_file())
        self.assertTrue(np.all((image >= 0) & (image <= 1)))

    def test_with_graxpert_no_work_folder_is_refused(self) -> None:
        master, _ = gradient_master(self.root)
        with self.assertRaises(ValueError):
            renders.prepare_polished(master, fake_graxpert(self.root), None)

    def test_the_local_contrast_spares_the_sky_and_keeps_the_colours(self) -> None:
        rng = np.random.default_rng(5)
        cloud = blob(SHAPE, (80, 100), 20) + 0.5 * blob(SHAPE, (60, 60), 6)
        nebula = np.stack([0.05 + 0.6 * cloud, 0.05 + 0.35 * cloud, 0.05 + 0.25 * cloud], axis=-1)
        nebula = np.clip(nebula + rng.normal(0, 0.004, nebula.shape), 0, 1).astype(np.float32)
        natural = renders.deep_nebula_tone(nebula)
        polished = renders.polished_nebula_tone(nebula)
        sky = cloud < 0.005
        noise = lambda image: float(np.std(renders.lum(image)[sky]))  # noqa: E731
        self.assertLess(noise(polished), 1.05 * noise(natural), "the sky's noise is not lifted")
        body = cloud > 0.2
        for channel in (0, 2):
            before = nebula[..., channel][body] / nebula[..., 1][body]
            after = polished[..., channel][body] / np.maximum(polished[..., 1][body], 1e-6)
            self.assertLess(float(np.max(np.abs(after - before))), 0.02, "colours kept")
        self.assertLessEqual(float(polished.max()), 1.0)


def median_of_display(image: np.ndarray) -> float:
    return float(np.median(renders.lum(image)))


class SkyLevel(unittest.TestCase):
    """the polished render keeps the sky as deep as the natural one (its median within 2 %)."""

    def setUp(self) -> None:
        self.root = Path(tempfile.mkdtemp())
        self.addCleanup(lambda: shutil.rmtree(self.root, ignore_errors=True))
        self.master, _ = gradient_master(self.root)

    def natural(self) -> np.ndarray:
        nebula = renders.prepare_native(self.master, 1, 0.05)
        return renders.natural_deep(nebula, np.zeros_like(nebula))

    def polished(self, **options: object) -> tuple[np.ndarray, dict[str, object]]:
        with redirect_stdout(io.StringIO()):
            image, done = renders.prepare_polished(self.master, work=self.root / "work", gradient_degree=1,
                                                   black_percentile=0.05, **options)
        return renders.polished_deep(image, np.zeros_like(image)), done

    def test_graxpert_background_that_eats_the_pedestal_does_not_darken_the_sky(self) -> None:
        natural = median_of_display(self.natural())
        polished, done = self.polished(graxpert=fake_graxpert(self.root, pedestal=0.012))
        self.assertEqual(done["engine"], "graxpert-ai")
        self.assertGreater(float(done["skyLevelShift"]), 0.0, "the sky was brought back up")
        self.assertAlmostEqual(median_of_display(polished) / natural, 1.0, delta=0.02)

    def test_siril_background_that_eats_the_pedestal_does_not_darken_the_sky(self) -> None:
        natural = median_of_display(self.natural())
        polished, done = self.polished(engine="siril", siril=fake_siril(self.root, pedestal=0.012))
        self.assertEqual(done["engine"], "siril")
        self.assertAlmostEqual(median_of_display(polished) / natural, 1.0, delta=0.02)

    def test_siril_runs_with_its_own_configuration_never_the_persons(self) -> None:
        """``-i`` points inside the work folder, so ``~/.config/siril`` is not rewritten."""
        self.polished(engine="siril", siril=fake_siril(self.root, pedestal=0.012))
        calls_made = siril_calls(self.root)
        self.assertTrue(calls_made, "Siril was called")
        for call in calls_made:
            args = [str(word) for word in call["args"]]
            self.assertIn("-i", args)
            ini = Path(args[args.index("-i") + 1])
            self.assertEqual(ini.parent, Path(str(call["cwd"])))
            self.assertEqual(ini.name, "siril-config.ini")

    def test_without_the_match_the_same_background_would_be_darker(self) -> None:
        """The test above would pass for any background: here it is shown that the match is what holds it."""
        natural = median_of_display(self.natural())
        graxpert = fake_graxpert(self.root, pedestal=0.012)
        cleaned = renders.graxpert_linear(self.master, graxpert, self.root / "work", "N")
        raw = renders.read_linear_rgb(cleaned)
        linear = np.clip(raw - np.percentile(raw, 0.05, axis=(0, 1)), 0, None)
        with redirect_stdout(io.StringIO()):
            bare = renders.polished_deep(renders.polished_display(linear), np.zeros_like(linear))
        self.assertLess(median_of_display(bare) / natural, 0.97)

    def test_the_polynomial_route_is_left_alone(self) -> None:
        _, done = self.polished()
        self.assertEqual(done["engine"], "polynomial")
        self.assertNotIn("skyLevelShift", done)


class SkyLevelAfterStarnet(unittest.TestCase):
    """once the stars are split off, the polished sky is brought to the natural render's level."""

    def test_the_final_level_search_reaches_its_target(self) -> None:
        cloud = blob(SHAPE, (80, 100), 20)
        nebula = np.stack([0.05 + 0.5 * cloud, 0.05 + 0.3 * cloud, 0.05 + 0.2 * cloud], axis=-1).astype(np.float32)
        stars = np.zeros_like(nebula)
        stars[30, 30] = [0.8, 0.8, 0.8]
        now = float(np.median(renders.lum(renders.polished_deep(nebula, stars))))
        for target in (0.9 * now, 1.12 * now):
            lifted, shift = renders.match_final_sky_level(nebula, stars, target)
            reached = float(np.median(renders.lum(renders.polished_deep(lifted, stars))))
            self.assertAlmostEqual(reached / target, 1.0, delta=0.005)
            self.assertEqual(shift > 0, target > now)
        same, shift = renders.match_final_sky_level(nebula, stars, now)
        self.assertEqual(shift, 0.0)
        np.testing.assert_array_equal(same, nebula)

    def test_the_polished_render_keeps_the_natural_sky_level_through_starnet(self) -> None:
        root = Path(tempfile.mkdtemp())
        self.addCleanup(lambda: shutil.rmtree(root, ignore_errors=True))
        (root / "work" / "render").mkdir(parents=True)
        master, _ = gradient_master(root, (640, 720))
        available, _ = pipeline_defs.load_pipelines([])
        plan = stack.Plan(frames={}, colour=True, pipelines=[available["polished"], available["raw"]])
        args = argparse.Namespace(work=root / "work", out=root / "out", name="N", trusted=[], pipelines_dir=[],
                                  pipeline=["polished", "raw"], gradient_degree=1, black_percentile=0.05, deconv_fwhm=None,
                                  contrast="none", polished_engine="siril", capture_filter=None)
        notes: dict[str, object] = {}
        with redirect_stdout(io.StringIO()), _stderr(io.StringIO()):
            stack.render_outputs(args, plan, stack.Reporter(False), stack.Runner(), master, fake_starnet(root), None, notes,
                                 fake_siril(root, pedestal=0.012))
        import tifffile

        level = lambda name: float(np.median(renders.lum(tifffile.imread(root / "out" / name).astype(np.float32) / 65535)))  # noqa: E731
        self.assertAlmostEqual(level("N_polished.tif") / level("N_raw.tif"), 1.0, delta=0.02)
        self.assertNotEqual(notes["polished"]["skyLevelFinalShift"], 0.0, "the stand-in StarNet leaves more of the halos: a shift was needed")


class StarnetModelCleanup(unittest.TestCase):
    """StarNet2 compiles its model (~120 MB) into a temporary folder at every run and never removes it."""

    def setUp(self) -> None:
        self.root = Path(tempfile.mkdtemp())
        self.addCleanup(lambda: shutil.rmtree(self.root, ignore_errors=True))
        self.temp = self.root / "T"
        self.temp.mkdir()
        original = renders.starnet_temp_dirs
        renders.starnet_temp_dirs = lambda: [self.temp]
        self.addCleanup(lambda: setattr(renders, "starnet_temp_dirs", original))
        # A model of someone else's, there before the run.
        self.older = self.temp / "StarNet2_weights_00000000-0000-0000-0000-000000000000.mlmodelc"
        self.older.mkdir()
        (self.older / "weights.bin").write_bytes(b"x")

    def starnet_that_compiles(self, fail: bool = False) -> Path:
        return stand_in_tool(
            self.root,
            "starnet2",
            "import sys, uuid\n"
            "from pathlib import Path\n"
            "import numpy as np\n"
            "import tifffile\n"
            f"model = Path({str(self.temp)!r}) / f'StarNet2_weights_{{uuid.uuid4()}}.mlmodelc'\n"
            "model.mkdir()\n"
            "(model / 'weights.bin').write_bytes(b'0' * 1000)\n"
            f"if {fail!r}:\n"
            "    sys.exit(3)\n"
            "args = sys.argv[1:]\n"
            "source, starless, mask = args[args.index('-i') + 1], args[args.index('-o') + 1], args[args.index('-m') + 1]\n"
            "data = tifffile.imread(source)\n"
            "tifffile.imwrite(starless, (data // 2), photometric='rgb')\n"
            "tifffile.imwrite(mask, np.zeros_like(data), photometric='rgb')\n",
        )

    def models(self) -> list[str]:
        return sorted(path.name for path in self.temp.glob(renders.STARNET_MODEL_PATTERN))

    def native(self) -> np.ndarray:
        return np.full((600, 640, 3), 0.2, dtype=np.float32)

    def test_the_model_a_run_compiled_is_removed_and_the_others_stay(self) -> None:
        (self.root / "out").mkdir()
        with redirect_stdout(io.StringIO()):
            nebula, stars = renders.star_layer(self.native(), self.root / "out", "N", self.starnet_that_compiles())
        self.assertEqual(nebula.shape, (600, 640, 3))
        self.assertEqual(self.models(), [self.older.name])

    def test_the_model_is_removed_when_starnet_fails_too(self) -> None:
        (self.root / "out").mkdir()
        import subprocess

        with redirect_stdout(io.StringIO()), self.assertRaises(subprocess.CalledProcessError):
            renders.star_layer(self.native(), self.root / "out", "N", self.starnet_that_compiles(fail=True))
        self.assertEqual(self.models(), [self.older.name])


class MonoMaster(unittest.TestCase):
    """A mono master (a two-dimensional FITS) renders; StarNet2 gives it back as a grayscale TIFF."""

    def render(self, pipelines: list[str], size: tuple[int, int] = (640, 720)) -> tuple[Path, dict[str, object]]:
        root = Path(tempfile.mkdtemp())
        self.addCleanup(lambda: shutil.rmtree(root, ignore_errors=True))
        (root / "work" / "render").mkdir(parents=True)
        rng = np.random.default_rng(5)
        sky = 0.02 + 0.03 * np.mgrid[: size[0], : size[1]][1] / size[1]
        mono = sky + 0.25 * blob(size, (size[0] // 2, size[1] // 2), 60) + rng.normal(0, 0.0008, size)
        mono[100, 100] += 0.6
        master = root / "master.fit"
        fits.PrimaryHDU(mono.astype(np.float32)).writeto(master)
        available, _ = pipeline_defs.load_pipelines([])
        plan = stack.Plan(frames={}, colour=False, pipelines=[available[name] for name in pipelines])
        args = argparse.Namespace(work=root / "work", out=root / "out", name="N", trusted=[], pipelines_dir=[],
                                  pipeline=pipelines, gradient_degree=1, black_percentile=0.05, deconv_fwhm=None,
                                  contrast="none", polished_engine="siril", capture_filter=None)
        notes: dict[str, object] = {}
        with redirect_stdout(io.StringIO()), _stderr(io.StringIO()):
            stack.render_outputs(args, plan, stack.Reporter(False), stack.Runner(), master, fake_starnet(root), None, notes,
                                 fake_siril(root))
        return root, notes

    def test_the_stand_in_starnet_writes_two_dimensions_for_equal_channels_like_the_real_one(self) -> None:
        import tifffile

        root = Path(tempfile.mkdtemp())
        self.addCleanup(lambda: shutil.rmtree(root, ignore_errors=True))
        source = root / "in.tif"
        tifffile.imwrite(source, np.full((600, 700, 3), 1000, np.uint16), photometric="rgb")
        import subprocess

        subprocess.run([str(fake_starnet(root)), "-i", str(source), "-o", str(root / "s.tif"), "-m", str(root / "m.tif")], check=True)
        self.assertEqual(tifffile.imread(root / "s.tif").shape, (600, 700))

    def test_star_layer_repeats_a_two_dimensional_starless_into_the_three_channels(self) -> None:
        native = np.repeat(np.linspace(0, 1, 600 * 700, dtype=np.float32).reshape(600, 700)[..., None], 3, axis=2)
        root = Path(tempfile.mkdtemp())
        self.addCleanup(lambda: shutil.rmtree(root, ignore_errors=True))
        with redirect_stdout(io.StringIO()):
            starless, stars = renders.star_layer(native, root, "mono", fake_starnet(root))
        self.assertEqual(starless.shape, native.shape)
        self.assertEqual(stars.shape, native.shape)
        for layer in (starless, stars):
            np.testing.assert_array_equal(layer[..., 0], layer[..., 1])
            np.testing.assert_array_equal(layer[..., 0], layer[..., 2])
        np.testing.assert_allclose(starless[..., 0], native[..., 0] * 0.6, atol=2e-5)

    def test_raw_polished_and_hoo_renders_of_a_mono_master_are_grey_images(self) -> None:
        import tifffile

        root, _ = self.render(["raw", "polished", "hoo"])
        for name in ("raw", "polished", "hoo"):
            image = tifffile.imread(root / "out" / f"N_{name}.tif")
            self.assertEqual(image.shape, (640, 720, 3), name)
            self.assertTrue((root / "out" / f"N_{name}.png").exists(), name)
            if name != "hoo":  # the HOO palette colours a signal it is given; the grey of a mono is the point of the other two
                np.testing.assert_array_equal(image[..., 0], image[..., 1], name)
                np.testing.assert_array_equal(image[..., 0], image[..., 2], name)
            level = renders.lum(image.astype(np.float32) / 65535)
            self.assertLess(float(np.median(level)), 0.2, f"{name}: a dark sky")
            self.assertGreater(float(level.max()), float(np.median(level)) + 0.3, f"{name}: the star and the cloud stand out")


class DualBand(unittest.TestCase):
    """the green of a dual-band capture is OIII, signal -- the SCNR is skipped."""

    def setUp(self) -> None:
        self.root = Path(tempfile.mkdtemp())
        self.addCleanup(lambda: shutil.rmtree(self.root, ignore_errors=True))

    def green_master(self) -> Path:
        """A flat sky and a cloud whose green and blue are strong (OIII): green above the mean of red and blue."""
        rng = np.random.default_rng(8)
        cloud = blob(MASTER, (240, 320), 50)
        rgb = np.stack([0.02 + 0.05 * cloud, 0.02 + 0.30 * cloud, 0.02 + 0.10 * cloud])
        rgb = rgb + rng.normal(0, 0.0006, rgb.shape)
        path = self.root / "oiii.fit"
        fits.PrimaryHDU(rgb.astype(np.float32)).writeto(path)
        return path

    def greenness(self, image: np.ndarray) -> float:
        """The share of green in the cloud's chromaticity (it does not depend on the stretch's brightness)."""
        body = image[200:280, 280:360]
        return float(np.mean(body[..., 1] / body.sum(axis=-1)))

    def test_the_scnr_is_skipped_on_a_dual_band_capture_and_kept_otherwise(self) -> None:
        master = self.green_master()
        with redirect_stdout(io.StringIO()):
            plain, done_plain = renders.prepare_polished(master, gradient_degree=1, black_percentile=0.05)
            dual, done_dual = renders.prepare_polished(master, gradient_degree=1, black_percentile=0.05, dualband=True)
        self.assertEqual(done_plain["scnr"], renders.SCNR_AMOUNT)
        self.assertIsNone(done_dual["scnr"])
        self.assertGreater(self.greenness(dual), self.greenness(plain) + 0.03)

    def test_the_capture_is_dual_band_by_its_filter_keyword_its_option_or_its_pipeline(self) -> None:
        available, _ = pipeline_defs.load_pipelines([])
        light = self.root / "light.fit"
        header = fits.Header()
        header["FILTER"] = "L-eXtreme"
        fits.PrimaryHDU(np.zeros((8, 8), np.float32), header).writeto(light)
        plain_light = self.root / "plain.fit"
        fits.PrimaryHDU(np.zeros((8, 8), np.float32)).writeto(plain_light)
        polished = [available["polished"]]
        args = argparse.Namespace(capture_filter=None)
        self.assertTrue(stack.capture_is_dualband(args, stack.Plan(frames={"lights": [light]}, colour=True, pipelines=polished)))
        self.assertFalse(stack.capture_is_dualband(args, stack.Plan(frames={"lights": [plain_light]}, colour=True, pipelines=polished)))
        self.assertTrue(stack.capture_is_dualband(argparse.Namespace(capture_filter="Optolong L-Ultimate"),
                                                  stack.Plan(frames={"lights": [plain_light]}, colour=True, pipelines=polished)))
        for chosen in ("dualband-correction", "hoo"):
            self.assertTrue(stack.capture_is_dualband(args, stack.Plan(frames={"lights": [plain_light]}, colour=True,
                                                                       pipelines=[available[chosen], available["polished"]])), chosen)
        self.assertFalse(stack.capture_is_dualband(args, stack.Plan(frames={}, colour=True, pipelines=polished)))
        self.assertFalse(stack.is_dualband_filter("Baader UV/IR cut"))


class DeconvolutionFloor(unittest.TestCase):
    """the polished deconvolution aims at 0.85 x FWHM and is skipped under a 2.0 px target."""

    def field(self, sigma: float) -> np.ndarray:
        rng = np.random.default_rng(1)
        image = np.zeros((700, 900), np.float64)
        yy, xx = np.mgrid[0:700, 0:900]
        for y, x, flux in zip(rng.uniform(15, 685, 150), rng.uniform(15, 885, 150), rng.uniform(0.15, 0.5, 150)):
            y0, y1, x0, x1 = int(y) - 12, int(y) + 13, int(x) - 12, int(x) + 13
            image[y0:y1, x0:x1] += flux * np.exp(-((yy[y0:y1, x0:x1] - y) ** 2 + (xx[y0:y1, x0:x1] - x) ** 2) / (2 * sigma ** 2))
        return (image + 0.05 + rng.normal(0, 0.002, image.shape)).astype(np.float32)

    def test_the_floor_is_two_pixels_and_the_target_85_percent_of_the_fwhm(self) -> None:
        self.assertEqual(renders.POLISHED_DECONV_FLOOR, 2.0)
        self.assertEqual(renders.POLISHED_DECONV_RATIO, 0.85)
        # Stars the measure reads at 2.39 px (the real masters read 2.4 to 2.5; the measure reads a little
        # wide on a sampled-at-2-px star): the target, 2.03 px, is above the floor -- the former one, 2.2, skipped it.
        wide = np.repeat(self.field(2.0 / 2.3548)[..., None], 3, axis=2)
        with redirect_stdout(io.StringIO()):
            target = renders.polished_deconv_target(wide)
        self.assertIsNotNone(target)
        self.assertAlmostEqual(target / renders.star_fwhm(renders.lum(wide)), 0.85, places=6)
        self.assertGreaterEqual(target, 2.0)
        self.assertLess(target, 2.2)
        # Stars read at 2.24 px: the target, 1.90 px, would square the stars off -- skipped, and it is said.
        narrow = np.repeat(self.field(1.8 / 2.3548)[..., None], 3, axis=2)
        printed = io.StringIO()
        with redirect_stdout(printed):
            self.assertIsNone(renders.polished_deconv_target(narrow))
        self.assertIn("undersampled", printed.getvalue())


class DeconvolutionRings(unittest.TestCase):
    """the deconvolution digs no dark ring around the stars whose core the sensor clipped.

    Seen on a real master of the Heart (IC 1805): the polished render showed dark crescents 6 to 14 px
    from the brightest stars.  Richardson-Lucy cannot recover a clipped core -- ``measure_psf`` leaves those
    stars out of the PSF -- and every iteration ate more of their wings; which master reached 25 iterations
    instead of 5 depended on the least change upstream (GraXpert's background and denoising were innocent:
    the same master, polynomial background, rings from 25 iterations too).
    """

    CLIP = 0.5

    def field(self) -> tuple[np.ndarray, list[tuple[int, int]]]:
        """Moffat stars (wings like a real one), faint ones and five bright ones clipped at ``CLIP``."""
        rng = np.random.default_rng(5)
        shape = (700, 900)
        alpha = 2.5 / (2 * np.sqrt(2 ** (1 / 2.5) - 1))
        yy, xx = np.mgrid[: shape[0], : shape[1]]
        image = np.zeros(shape)

        def star(y: float, x: float, peak: float) -> None:
            y0, y1, x0, x1 = max(0, int(y) - 40), min(shape[0], int(y) + 41), max(0, int(x) - 40), min(shape[1], int(x) + 41)
            image[y0:y1, x0:x1] += peak * (1 + ((yy[y0:y1, x0:x1] - y) ** 2 + (xx[y0:y1, x0:x1] - x) ** 2) / alpha**2) ** -2.5

        for y, x, peak in zip(rng.uniform(20, 680, 150), rng.uniform(20, 880, 150), rng.uniform(0.02, 0.12, 150)):
            star(y, x, peak)
        bright = [(120, 150), (120, 700), (350, 420), (560, 160), (560, 740)]
        for y, x in bright:
            star(y, x, 4.0)
        image = np.minimum(image + 0.0003, self.CLIP) + rng.normal(0, 2e-5, shape)
        return np.repeat(image[..., None], 3, axis=2).astype(np.float32), bright

    @staticmethod
    def wing_loss(before: np.ndarray, after: np.ndarray, centre: tuple[int, int]) -> float:
        """The worst, over the annuli 3 to 16 px from the star, of (after - before) / (before - sky): -1 is a wing eaten away."""
        yy, xx = np.mgrid[: before.shape[0], : before.shape[1]]
        radius = np.hypot(yy - centre[0], xx - centre[1])
        sky = np.median(before[(radius >= 40) & (radius <= 55)])
        return min(
            float((after[ring].mean() - before[ring].mean()) / (before[ring].mean() - sky))
            for ring in ((radius >= k - 0.5) & (radius < k + 0.5) for k in range(3, 17))
        )

    def test_the_wings_of_a_clipped_star_stay_whatever_the_target(self) -> None:
        image, bright = self.field()
        before = renders.lum(image)
        for target in (2.15, 1.6):  # the second asks for ten times more iterations than the first
            with self.subTest(target=target), redirect_stdout(io.StringIO()):
                after = renders.lum(renders.deconvolve_linear(image, target))
            for centre in bright:
                self.assertGreater(self.wing_loss(before, after, centre), -0.25, f"star at {centre}: a dark ring")

    def test_the_faint_stars_are_still_sharpened(self) -> None:
        image, _ = self.field()
        with redirect_stdout(io.StringIO()):
            sharper = renders.deconvolve_linear(image, 2.15)
        self.assertLess(renders.star_fwhm(renders.lum(sharper)), renders.star_fwhm(renders.lum(image)) - 0.1)

    def test_the_hold_covers_the_clipped_cores_and_the_sky_around_them_and_nothing_else(self) -> None:
        image, bright = self.field()
        weight = renders.clipped_core_weight(renders.lum(image))
        for y, x in bright:
            self.assertEqual(weight[y, x], 1.0)
            self.assertGreater(weight[y, x + 8], 0.99)  # inside the hold
            self.assertLess(weight[y, x + 32], 0.01)  # past the fade
        self.assertLess(weight[300:330, 250:300].max(), 0.01)  # a field of faint stars: untouched
        self.assertFalse(renders.clipped_core_weight(np.full((64, 64), 0.1, np.float32)).any())

    def test_a_step_that_gains_nothing_is_not_kept(self) -> None:
        image, _ = self.field()
        widths = iter([2.9, 2.6, 2.59, 2.0])  # the start, then the widths of steps 5, 10 and 15
        printed = io.StringIO()
        with mock.patch.object(renders, "star_fwhm", side_effect=lambda _: next(widths)), redirect_stdout(printed):
            renders.deconvolve_linear(image, 2.0)
        self.assertIn("deconvolution: 5 iterations", printed.getvalue())


class Engines(unittest.TestCase):
    """the background and denoising engine is chosen by the task; Siril is the fallback."""

    def setUp(self) -> None:
        self.root = Path(tempfile.mkdtemp())
        self.addCleanup(lambda: shutil.rmtree(self.root, ignore_errors=True))
        self.master, _ = gradient_master(self.root)

    def prepare(self, **options: object) -> tuple[np.ndarray, dict[str, object], str]:
        printed = io.StringIO()
        with redirect_stdout(printed):
            image, done = renders.prepare_polished(self.master, work=self.root / "work", stem="N",
                                                   gradient_degree=1, black_percentile=0.05, **options)
        return image, done, printed.getvalue()

    def test_the_default_is_graxpert_and_the_names_are_the_two_engines(self) -> None:
        self.assertEqual(renders.DEFAULT_POLISHED_ENGINE, "graxpert-ai")
        self.assertEqual(renders.POLISHED_ENGINES, ("graxpert-ai", "siril"))
        self.assertEqual(stack.parse_args(["--lights", "l", "--work", "w", "--out", "o", "--name", "N"]).polished_engine, "graxpert-ai")
        self.assertEqual(stack.parse_args(["--lights", "l", "--work", "w", "--out", "o", "--name", "N",
                                           "--polished-engine", "siril"]).polished_engine, "siril")
        with self.assertRaises(ValueError):
            self.prepare(engine="magic")

    def test_graxpert_engine_runs_graxpert_and_not_siril(self) -> None:
        _, done, _ = self.prepare(graxpert=fake_graxpert(self.root), siril=fake_siril(self.root))
        self.assertEqual((done["engine"], done["background"], done["denoise"]), ("graxpert-ai", "graxpert", "graxpert"))
        self.assertNotIn("engineRequested", done)
        self.assertEqual(len(calls(self.root)), 2)
        self.assertEqual(siril_calls(self.root), [])

    def test_siril_engine_runs_subsky_rbf_then_denoise_in_the_work_folder_and_not_graxpert(self) -> None:
        graxpert, siril = fake_graxpert(self.root), fake_siril(self.root)
        before = sorted(p.name for p in self.root.iterdir())
        image, done, _ = self.prepare(engine="siril", graxpert=graxpert, siril=siril)
        self.assertEqual((done["engine"], done["background"], done["denoise"]), ("siril", "siril", "siril"))
        self.assertNotIn("engineRequested", done)
        self.assertEqual(calls(self.root), [])
        (call,) = siril_calls(self.root)
        self.assertEqual(call["cwd"], str((self.root / "work").resolve()))
        words = [line.split()[0] for line in call["script"]]
        self.assertEqual(words, ["requires", "load", "subsky", "denoise", "save"])
        self.assertIn("-rbf", call["script"][2])
        self.assertIn(str(self.master.resolve()), call["script"][1])
        # Nothing beside the master; the file Siril wrote is in the work folder.
        self.assertEqual(sorted(p.name for p in self.root.iterdir()), sorted([*before, "siril-calls.jsonl", "work"]))
        self.assertTrue((self.root / "work" / "N_siril_denoised.fit").is_file())
        self.assertTrue(np.all((image >= 0) & (image <= 1)))

    def test_siril_engine_never_looks_for_graxpert(self) -> None:
        available, _ = pipeline_defs.load_pipelines([])
        plan = stack.Plan(frames={}, colour=True, pipelines=[available["polished"]])
        args = argparse.Namespace(graxpert=None, no_graxpert=False, polished_engine="siril")
        self.assertIsNone(stack.find_graxpert(args, plan))

    def test_graxpert_missing_falls_back_on_siril_and_says_so(self) -> None:
        _, done, said = self.prepare(siril=fake_siril(self.root))
        self.assertEqual((done["engine"], done["engineRequested"]), ("siril", "graxpert-ai"))
        self.assertEqual(done["fallbackReason"], "GraXpert not given")
        self.assertEqual((done["background"], done["denoise"]), ("siril", "siril"))
        self.assertIn("GraXpert not given", said)

    def test_graxpert_failing_falls_back_on_siril_and_says_why(self) -> None:
        _, done, said = self.prepare(graxpert=fake_graxpert(self.root, fail=True), siril=fake_siril(self.root))
        self.assertEqual((done["engine"], done["engineRequested"]), ("siril", "graxpert-ai"))
        self.assertIn("GraXpert failed", done["fallbackReason"])
        self.assertIn("graxpertFailed", done)
        self.assertIn("GraXpert failed", said)

    def test_with_nothing_that_runs_the_polynomial_is_the_last_resort(self) -> None:
        _, done, said = self.prepare(graxpert=fake_graxpert(self.root, fail=True), siril=fake_siril(self.root, fail=True))
        self.assertEqual((done["engine"], done["background"], done["denoise"]), ("polynomial", "polynomial", None))
        self.assertIn("sirilFailed", done)
        self.assertIn("Siril failed", said)
        _, done, _ = self.prepare(engine="siril")
        self.assertEqual((done["engine"], done["engineRequested"]), ("polynomial", "siril"))

    def test_the_stacking_script_gives_the_engine_to_the_render_and_writes_it_in_the_provenance_notes(self) -> None:
        root = Path(tempfile.mkdtemp())
        self.addCleanup(lambda: shutil.rmtree(root, ignore_errors=True))
        (root / "work" / "render").mkdir(parents=True)
        master, _ = gradient_master(root)
        available, _ = pipeline_defs.load_pipelines([])
        plan = stack.Plan(frames={}, colour=True, pipelines=[available["polished"]])
        notes: dict[str, object] = {}
        for engine in ("graxpert-ai", "siril"):
            args = argparse.Namespace(work=root / "work", out=root / "out", name="N", trusted=[], pipelines_dir=[],
                                      pipeline=["polished"], gradient_degree=1, black_percentile=0.05, deconv_fwhm=None,
                                      contrast="none", polished_engine=engine, capture_filter=None)
            with redirect_stdout(io.StringIO()), _stderr(io.StringIO()):
                stack.render_outputs(args, plan, stack.Reporter(False), stack.Runner(), master, None,
                                     fake_graxpert(root), notes, fake_siril(root))
            self.assertEqual(notes["polished"]["engine"], engine)
        self.assertEqual(len(calls(root)), 2, "GraXpert ran once, for the first engine only")
        self.assertEqual(len(siril_calls(root)), 1)


class PolishedInTheScript(unittest.TestCase):
    """The stacking script's render step: one preparation per kind, GraXpert's failure turned into the polynomial."""

    def render(self, pipeline_ids: list[str], graxpert: Path | None,
               **finish: object) -> tuple[list[dict[str, object]], dict[str, object], str]:
        root = Path(tempfile.mkdtemp())
        self.addCleanup(lambda: shutil.rmtree(root, ignore_errors=True))
        (root / "work" / "render").mkdir(parents=True)
        master, _ = gradient_master(root)
        available, _ = pipeline_defs.load_pipelines([])
        plan = stack.Plan(frames={}, colour=True, pipelines=[available[i] for i in pipeline_ids])
        args = argparse.Namespace(work=root / "work", out=root / "out", name="N", trusted=[], pipelines_dir=[],
                                  pipeline=pipeline_ids, gradient_degree=1, black_percentile=0.05, deconv_fwhm=None,
                                  **{"contrast": "none", **finish})
        notes: dict[str, object] = {}
        log = io.StringIO()
        with redirect_stdout(io.StringIO()), _stderr(log):
            outputs = stack.render_outputs(args, plan, stack.Reporter(False), stack.Runner(), master, None, graxpert, notes)
        return outputs, notes, log.getvalue()

    def test_raw_and_polished_together_each_with_its_preparation(self) -> None:
        outputs, notes, said = self.render(["raw", "polished"], None)
        self.assertEqual([o["file"] for o in outputs], ["N_raw.tif", "N_raw.png", "N_polished.tif", "N_polished.png"])
        self.assertEqual(notes["polished"]["background"], "polynomial")
        self.assertIn("GraXpert not given", said)

    def test_each_pipeline_writes_its_own_finish_and_the_others_the_default(self) -> None:
        import render_finish

        outputs, _, _ = self.render(["raw", "hoo"], None, contrast="presentation",
                                    finish=[("hoo", render_finish.checked("strong", 20, 50))])
        self.assertEqual([o["file"] for o in outputs],
                         ["N_raw.tif", "N_raw.png", "N_raw_presentation.tif", "N_raw_presentation.png",
                          "N_hoo.tif", "N_hoo.png", "N_hoo_strong_colour-20_stars-50.tif", "N_hoo_strong_colour-20_stars-50.png"])
        hoo_twin = outputs[-2]
        self.assertEqual((hoo_twin["pipeline"], hoo_twin["contrast"], hoo_twin["colour"], hoo_twin["softStars"]),
                         ("hoo", "strong", "20", "50"))
        self.assertNotIn("colour", outputs[2])
        # A pipeline asked with no finish at all writes its render alone.
        outputs, _, _ = self.render(["raw", "hoo"], None, contrast="strong",
                                    finish=[("raw", render_finish.checked("none", "none", "none"))])
        self.assertEqual([o["file"] for o in outputs],
                         ["N_raw.tif", "N_raw.png", "N_hoo.tif", "N_hoo.png", "N_hoo_strong.tif", "N_hoo_strong.png"])

    def test_a_failing_graxpert_leaves_the_render_made_without_it_and_says_why(self) -> None:
        root = Path(tempfile.mkdtemp())
        self.addCleanup(lambda: shutil.rmtree(root, ignore_errors=True))
        outputs, notes, said = self.render(["polished"], fake_graxpert(root, fail=True))
        self.assertEqual([o["file"] for o in outputs], ["N_polished.tif", "N_polished.png"])
        self.assertEqual((notes["polished"]["background"], notes["polished"]["denoise"]), ("polynomial", None))
        self.assertIn("graxpertFailed", notes["polished"])
        self.assertIn("GraXpert failed", said)

    def test_graxpert_is_looked_for_only_for_a_pipeline_that_uses_it(self) -> None:
        available, _ = pipeline_defs.load_pipelines([])
        args = argparse.Namespace(graxpert=Path("/does/not/exist"), no_graxpert=False)
        raw = stack.Plan(frames={}, colour=True, pipelines=[available["raw"]])
        self.assertIsNone(stack.find_graxpert(args, raw))
        polished = stack.Plan(frames={}, colour=True, pipelines=[available["polished"]])
        with self.assertRaises(stack.StackingError) as caught:
            stack.find_graxpert(args, polished)
        self.assertEqual(caught.exception.code, "graxpert_missing")
        self.assertIsNone(stack.find_graxpert(argparse.Namespace(graxpert=None, no_graxpert=True), polished))
        linear = stack.Plan(frames={}, colour=True, pipelines=[], linear_only=True)
        self.assertIsNone(stack.find_graxpert(args, linear))


class _stderr:
    """Collect what the render step writes to stderr (its prints are redirected there)."""

    def __init__(self, buffer: io.StringIO) -> None:
        self.buffer = buffer

    def __enter__(self) -> None:
        self.saved = sys.stderr
        sys.stderr = self.buffer

    def __exit__(self, *exc: object) -> None:
        sys.stderr = self.saved


class HooPalette(unittest.TestCase):
    def nebula(self) -> tuple[np.ndarray, np.ndarray, np.ndarray]:
        """A stretched dual-band nebula layer: an Hα cloud in the red, an OIII cloud in green and blue."""
        rng = np.random.default_rng(11)
        ha = blob(SHAPE, (80, 60), 14)
        oiii = blob(SHAPE, (80, 145), 14)
        sky = 0.05
        nebula = np.stack([sky + 0.5 * ha, sky + 0.2 * oiii, sky + 0.22 * oiii], axis=-1)
        nebula = nebula + rng.normal(0, 0.003, nebula.shape)
        return np.clip(nebula, 0, 1).astype(np.float32), ha, oiii

    def test_h_alpha_goes_to_red_oiii_to_blue_green_and_the_sky_stays_neutral(self) -> None:
        from skimage.color import rgb2hsv

        nebula, ha, oiii = self.nebula()
        out = renders.hoo_palette(nebula)
        hsv = rgb2hsv(out)
        hue = lambda mask: float(np.median(hsv[..., 0][mask]))  # noqa: E731
        red = hue(ha > 0.6)
        self.assertTrue(red < 0.03 or red > 0.97, red)
        self.assertAlmostEqual(hue(oiii > 0.6), 0.5, delta=0.03)
        self.assertGreater(float(np.median(hsv[..., 1][ha > 0.6])), 0.4)
        self.assertLessEqual(float(hsv[..., 1].max()), renders.HOO_MAX_SATURATION + 1e-6)

    def test_the_sky_is_the_natural_neutral_sky_not_a_grey(self) -> None:
        """where there is no signal the pixel is the natural tone's own (same colour, same level)."""
        from skimage.color import rgb2hsv

        nebula, ha, oiii = self.nebula()
        sky = (ha < 0.003) & (oiii < 0.003)
        out, natural = renders.hoo_palette(nebula), renders.deep_nebula_tone(nebula)
        self.assertGreater(int(sky.sum()), 5000)
        self.assertLess(float(np.mean(np.abs(out - natural)[sky])), 0.002)
        self.assertAlmostEqual(float(np.median(renders.lum(out)[sky])), float(np.median(renders.lum(natural)[sky])), delta=0.002)
        # Its colour is the natural sky's, whatever that is: a tinted sky stays tinted, it is not flattened to grey.
        tinted = nebula * np.array([1.0, 1.0, 1.0], np.float32) + np.array([0.0, 0.004, 0.0], np.float32)
        out_tinted, natural_tinted = renders.hoo_palette(tinted), renders.deep_nebula_tone(tinted)
        saturation = lambda image: float(np.median(rgb2hsv(image)[..., 1][sky]))  # noqa: E731
        self.assertGreater(saturation(natural_tinted), 0.02)
        self.assertAlmostEqual(saturation(out_tinted), saturation(natural_tinted), delta=0.01)

    def test_a_faint_red_zone_fades_in_with_no_hard_edge(self) -> None:
        """a faint Hα halo (a ramp from the sky) takes the palette gradually: the mask has no threshold."""
        rng = np.random.default_rng(4)
        width = 1400
        ramp = np.clip((np.arange(width) - 600) / 400.0, 0, 1)[None, :] * np.ones((SHAPE[0], 1))
        nebula = np.stack([0.05 + 0.02 * ramp, 0.05 + 0.0015 * ramp, 0.05 + 0.0015 * ramp], axis=-1)
        nebula = np.clip(nebula + rng.normal(0, 0.003, nebula.shape), 0, 1).astype(np.float32)
        weight = renders.hoo_weight(nebula)
        row = weight[40:120].mean(axis=0)
        self.assertLess(float(row[:500].max()), 0.05, "the sky takes no palette")
        self.assertGreater(float(row[-100:].min()), 0.8, "the zone is coloured")
        # A threshold would be a step of 0.5 and more between two neighbours.
        self.assertLess(float(np.abs(np.diff(row)).max()), 0.06, "a step of the mask between two neighbours")
        self.assertGreater(int(((row > 0.1) & (row < 0.9)).sum()), 20, "the passage is spread over many pixels")
        # And the colour follows: no jump of the output either.
        out = renders.hoo_palette(nebula)
        redness = (out[..., 0] - out[..., 1])[40:120].mean(axis=0)
        self.assertLess(float(np.abs(np.diff(redness)).max()), 0.03)

    def test_the_brightness_is_the_natural_one(self) -> None:
        nebula, _, _ = self.nebula()
        np.testing.assert_allclose(renders.hoo_palette(nebula).max(axis=-1),
                                   renders.deep_nebula_tone(nebula).max(axis=-1), atol=2e-3)

    def test_the_stars_keep_their_natural_colours(self) -> None:
        nebula = np.zeros((*SHAPE, 3), np.float32)
        stars = np.zeros_like(nebula)
        stars[40, 40] = [0.9, 0.7, 0.5]
        out = renders.hoo_deep(nebula, stars)
        np.testing.assert_allclose(out[40, 40], renders.controlled_stars(stars, 0.52)[40, 40], atol=1e-6)


if __name__ == "__main__":
    unittest.main()
