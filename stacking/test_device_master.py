"""Tests of the image a device already stacked, taken as the master of the renders.

Run from this folder, with the stacking environment:
    python3 -m unittest -v test_device_master

What is covered: the conversion of a Seestar-like colour cube (16 bits, the unsigned
offset) to the 0..1 floats of Siril's master, the refusal of a stretched image, of a
mono one and of one that is not readable, the command line of ``--from-master`` (it
replaces ``--lights`` and refuses what only a stacking of poses has), and a complete
run with ``--linear-only`` -- no Siril, no StarNet2 -- whose provenance names the image
and writes no quality of its own and keeps no master.
"""

from __future__ import annotations

import contextlib
import io
import json
import tempfile
import unittest
from pathlib import Path

try:
    import numpy as np
    from astropy.io import fits

    import astro_sync_stack
    import device_master
    import star_quality
    from test_star_quality import star_field
except ImportError:  # the stacking environment has them; a bare interpreter does not
    np = fits = astro_sync_stack = device_master = star_quality = star_field = None

NEEDS_NUMPY = unittest.skipIf(np is None, "needs numpy, scipy and astropy (the stacking environment)")


def write_stack(path: Path, sky: "np.ndarray", poses: int = 430, creator: str = "ZWO Seestar S50 Pro") -> None:
    """A device-like image: the same field in three planes, 16 bits with the unsigned offset, STACKCNT and friends."""
    cube = np.stack([sky, sky * 0.9, sky * 0.8]).clip(0, 1)
    data = np.round(cube * 65535).astype(np.uint16)
    hdu = fits.PrimaryHDU(data)  # astropy writes BZERO = 32768 for an unsigned 16-bit array
    hdu.header["CREATOR"] = creator
    hdu.header["STACKCNT"] = poses
    hdu.header["EXPTIME"] = 30.0
    hdu.header["TOTALEXP"] = 30.0 * poses
    hdu.header["FILTER"] = "IRCUT"
    hdu.header["GAIN"] = 200
    hdu.header["OBJECT"] = "M 45"
    hdu.writeto(path, overwrite=True)


def stretched_sky() -> "np.ndarray":
    """An image made to be looked at: its sky at 5 % of the range, black and white points in place."""
    sky = star_field(150, 1.6, 1.6).copy()  # the sky sits at 0.05
    sky[0, 0], sky[1, 1] = 0.0, 1.0
    return sky


def run(argv: list[str]) -> tuple[int, list[dict], str]:
    """``astro_sync_stack.main`` with ``--json``: the exit code, the events and what went to stderr."""
    out, err = io.StringIO(), io.StringIO()
    with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
        code = astro_sync_stack.main([*argv, "--json"])
    events = [json.loads(line) for line in out.getvalue().splitlines() if line.startswith("{")]
    return code, events, err.getvalue()


@NEEDS_NUMPY
class Convert(unittest.TestCase):
    def setUp(self) -> None:
        self.dir = tempfile.TemporaryDirectory()
        self.root = Path(self.dir.name)

    def tearDown(self) -> None:
        self.dir.cleanup()

    def test_the_cube_is_scaled_to_zero_one_and_keeps_its_rows(self) -> None:
        # A linear sky: the median far below 2 % of the range, stars well above it.
        sky = (star_field(150, 1.6, 1.6) - 0.05).clip(0, None) * 0.9 + 0.0005
        source = self.root / "Stacked_430_M 45_30.0s_IRCUT_20260101-000000.fit"
        write_stack(source, sky)
        master = self.root / "work" / "stack_linear.fit"
        described = device_master.convert(source, master)
        data = fits.getdata(master)
        self.assertEqual((data.dtype.kind, data.dtype.itemsize), ("f", 4))
        self.assertEqual(data.shape, (3,) + sky.shape)
        self.assertTrue(0.0 <= float(data.min()) and float(data.max()) <= 1.0)
        # Nothing is flipped: the first plane is the sky as it was written (to the 16-bit step).
        self.assertTrue(np.allclose(data[0], sky.clip(0, 1), atol=2 / 65535))
        header = fits.getheader(master)
        self.assertEqual(header["STACKCNT"], 430)
        self.assertEqual(header["TOTALEXP"], 12900.0)
        self.assertEqual(described["stackedFrames"], 430)
        self.assertEqual(described["device"], "Seestar S50 Pro")
        self.assertEqual(described["filter"], "IRCUT")
        self.assertEqual(described["gain"], 200.0)
        self.assertFalse(described["mirrored"])

    def test_a_device_that_were_mirrored_is_mirrored_once_here(self) -> None:
        sky = (star_field(60, 1.6, 1.6) - 0.05).clip(0, None) * 0.9 + 0.0005
        source = self.root / "Stacked_8.fit"
        write_stack(source, sky, poses=8)
        master = self.root / "mirrored.fit"
        described = device_master.convert(source, master, mirror=True)
        self.assertTrue(described["mirrored"])
        self.assertTrue(np.allclose(fits.getdata(master)[0], sky.clip(0, 1)[::-1], atol=2 / 65535))

    def test_a_stretched_image_is_refused(self) -> None:
        source = self.root / "Stacked_8.fit"
        write_stack(source, stretched_sky())
        with self.assertRaises(device_master.DeviceImageError) as raised:
            device_master.convert(source, self.root / "x.fit")
        self.assertEqual(raised.exception.code, "stretched")

    def test_what_is_not_a_colour_cube_or_not_a_file_is_refused(self) -> None:
        mono = self.root / "mono.fit"
        fits.PrimaryHDU(np.zeros((32, 32), np.uint16)).writeto(mono)
        with self.assertRaises(device_master.DeviceImageError) as raised:
            device_master.convert(mono, self.root / "x.fit")
        self.assertEqual(raised.exception.code, "not_an_image")
        junk = self.root / "junk.fit"
        junk.write_bytes(b"not a fits file")
        with self.assertRaises(device_master.DeviceImageError) as raised:
            device_master.convert(junk, self.root / "x.fit")
        self.assertEqual(raised.exception.code, "unreadable")


@NEEDS_NUMPY
class CommandLine(unittest.TestCase):
    def setUp(self) -> None:
        self.dir = tempfile.TemporaryDirectory()
        self.root = Path(self.dir.name)
        self.image = self.root / "M45" / "device" / "Stacked_430.fit"
        self.image.parent.mkdir(parents=True)
        sky = (star_field(150, 1.6, 1.6) - 0.05).clip(0, None) * 0.9 + 0.0005
        write_stack(self.image, sky)
        self.out = self.root / "M45" / "stacked"
        self.work = self.root / ".astro-sync-work" / "stacking" / "M45_2026-10-10"
        self.base = ["--from-master", str(self.image), "--work", str(self.work), "--out", str(self.out),
                     "--name", "M45_2026-10-10"]

    def tearDown(self) -> None:
        self.dir.cleanup()

    def test_from_master_replaces_the_lights_and_refuses_what_only_poses_have(self) -> None:
        for extra in (["--lights", str(self.root)], ["--darks", str(self.root)], ["--integration", "F"],
                      ["--frames-from", str(self.root / "list.txt")]):
            with self.assertRaises(SystemExit), contextlib.redirect_stderr(io.StringIO()):
                astro_sync_stack.parse_args([*self.base, *extra])
        with self.assertRaises(SystemExit), contextlib.redirect_stderr(io.StringIO()):
            astro_sync_stack.parse_args(["--work", "w", "--out", "o", "--name", "n"])  # neither lights nor master
        args = astro_sync_stack.parse_args(self.base)
        self.assertEqual(args.from_master, self.image)

    def test_the_plan_has_no_frame_and_no_stacking_step(self) -> None:
        code, events, _ = run([*self.base, "--linear-only", "--dry-run"])
        self.assertEqual(code, 0)
        plan = events[0]
        self.assertEqual(plan["fromMaster"], "Stacked_430.fit")
        self.assertEqual(plan["frames"]["lights"], 0)
        self.assertEqual(plan["steps"], ["device_master", "linear_export"])
        # No master kept for a multi-night plan: only the image and the provenance.
        self.assertEqual(plan["outputs"], ["M45_2026-10-10_linear.tif", "M45_2026-10-10_stacking.json"])
        self.assertFalse(self.out.exists())

    def test_a_linear_only_run_writes_the_image_and_a_provenance_that_names_the_device_image(self) -> None:
        code, events, _ = run([*self.base, "--linear-only"])
        self.assertEqual(code, 0, events)
        self.assertEqual([e["step"] for e in events if e["event"] == "step"], ["device_master", "linear_export"])
        files = sorted(path.name for path in self.out.iterdir())
        self.assertEqual(files, ["M45_2026-10-10_linear.tif", "M45_2026-10-10_stacking.json"])
        provenance = json.loads((self.out / "M45_2026-10-10_stacking.json").read_text())
        self.assertEqual(provenance["stackedFrames"], 430)
        self.assertEqual(provenance["frames"]["lights"], 0)
        self.assertIsNone(provenance["registration"])
        self.assertIsNone(provenance["integration"])
        # Measured on request, never by the render; Siril stacked nothing.
        self.assertIsNone(provenance["quality"])
        self.assertIsNone(provenance["tools"]["siril"])
        self.assertEqual(provenance["target"], "M45")
        named = provenance["deviceMaster"]
        self.assertEqual(named["file"], "Stacked_430.fit")
        self.assertEqual(named["stackedFrames"], 430)
        self.assertEqual(len(named["sha256"]), 64)
        # The scratch is the job's: gone with it.
        self.assertFalse(self.work.exists())
        # The device image was only read.
        self.assertTrue(self.image.is_file())

    def test_a_stretched_image_stops_the_run_with_its_own_code(self) -> None:
        stretched = self.root / "M45" / "device" / "Stacked_9.fit"
        write_stack(stretched, stretched_sky())
        argv = [*self.base]
        argv[1] = str(stretched)
        code, events, _ = run([*argv, "--linear-only"])
        self.assertEqual(code, astro_sync_stack.EXIT_INPUT)
        self.assertEqual(events[-1]["event"], "error")
        self.assertEqual(events[-1]["code"], "device_stretched")

    def test_a_missing_file_is_a_bad_input(self) -> None:
        argv = [*self.base]
        argv[1] = str(self.root / "nothing.fit")
        code, events, _ = run([*argv, "--linear-only"])
        self.assertEqual(code, astro_sync_stack.EXIT_INPUT)
        self.assertEqual(events[-1]["code"], "folder_missing")


@NEEDS_NUMPY
class ListedFrames(unittest.TestCase):
    """``--frames-from`` takes a frame that is a link into ``--lights`` (the night of "Choose the treatments…")."""

    def test_a_linked_pose_is_inside_the_lights_folder_and_a_foreign_file_is_not(self) -> None:
        with tempfile.TemporaryDirectory() as name:
            root = Path(name)
            (root / "originals").mkdir()
            (root / "night" / "lights").mkdir(parents=True)
            (root / "elsewhere").mkdir()
            original = root / "originals" / "a.fit"
            original.write_bytes(b"x")
            link = root / "night" / "lights" / "a.fit"
            try:
                link.symlink_to(original)
            except OSError:
                self.skipTest("no symbolic link here")
            foreign = root / "elsewhere" / "b.fit"
            foreign.write_bytes(b"x")
            listing = root / "list.txt"
            listing.write_text(f"{link}\n", encoding="utf-8")
            self.assertEqual(astro_sync_stack.listed_frames(listing, root / "night" / "lights"), [link])
            listing.write_text(f"{foreign}\n", encoding="utf-8")
            with self.assertRaises(astro_sync_stack.StackingError):
                astro_sync_stack.listed_frames(listing, root / "night" / "lights")
            # A path that climbs out of the folder is judged on where it lands.
            climbing = root / "night" / "lights" / ".." / ".." / "elsewhere" / "b.fit"
            listing.write_text(f"{climbing}\n", encoding="utf-8")
            with self.assertRaises(astro_sync_stack.StackingError):
                astro_sync_stack.listed_frames(listing, root / "night" / "lights")


if __name__ == "__main__":
    unittest.main()
