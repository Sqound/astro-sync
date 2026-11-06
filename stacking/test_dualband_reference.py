"""Tests of ``dualband_reference.py``: the recipe, kept to the bit.

``python3 -m pytest test_dualband_reference.py`` (or ``unittest``) in the stacking environment.

* ``Recipe``: on small synthetic masters, the module gives exactly what a reference transcription gives
  (below), and what the recipe says it does;
* ``Parity``: on real masters, when ``ASTRO_SYNC_PARITY_DIR`` names a folder holding a ``parity.json``
  (the masters, and the expected image); skipped otherwise.
"""

from __future__ import annotations

import hashlib
import json
import os
import unittest
from pathlib import Path

try:
    import numpy as np
    from astropy.io import fits

    import astroalign  # noqa: F401
except ImportError:  # the stacking environment has them; a bare interpreter does not
    np = fits = None

NEEDS_ENVIRONMENT = unittest.skipIf(np is None, "needs numpy, astropy and astroalign (the stacking environment)")
SHAPE = (400, 480)


def star_field(shape, stars, sigma=1.7):
    yy, xx = np.mgrid[0:shape[0], 0:shape[1]]
    field = np.zeros(shape, np.float64)
    for y, x, flux in stars:
        field += flux * np.exp(-((yy - y) ** 2 + (xx - x) ** 2) / (2 * sigma ** 2))
    return field


def sky(seed: int, shift=(0.0, 0.0), angle=0.0, colour=(1.0, 1.0, 1.0), level=0.1, gradient=0.0, cloud=0.0):
    """A ``(3, h, w)`` master: the same stars moved by ``shift`` and turned by ``angle`` degrees around the
    centre, a sky, a gradient along the rows, an Hα cloud in the red, noise."""
    rng = np.random.default_rng(seed)
    stars = np.random.default_rng(11)
    centre = np.array(SHAPE) / 2
    turn = np.deg2rad(angle)
    placed = []
    for _ in range(90):
        y, x = stars.uniform(20, SHAPE[0] - 20), stars.uniform(20, SHAPE[1] - 20)
        flux = stars.uniform(0.05, 1.0)
        dy, dx = y - centre[0], x - centre[1]
        placed.append((centre[0] + dy * np.cos(turn) - dx * np.sin(turn) + shift[0],
                       centre[1] + dy * np.sin(turn) + dx * np.cos(turn) + shift[1], flux))
    field = star_field(SHAPE, placed)
    yy, xx = np.mgrid[0:SHAPE[0], 0:SHAPE[1]]
    nebula = np.exp(-((yy - 200) ** 2 + (xx - 240) ** 2) / (2 * 60 ** 2))
    planes = []
    for channel, weight in enumerate(colour):
        plane = level + gradient * yy / SHAPE[0] + weight * field
        if channel == 0:
            plane = plane + cloud * nebula
        planes.append(plane + rng.normal(0, 0.002, SHAPE))
    return np.stack(planes).astype(np.float32)


# --- The reference transcription of the recipe ---------------------------------------------------------------


def original_fuse(dual_planes, broad_planes, weight=0.18):
    """The fusion of the two bands, on arrays."""
    import astroalign
    from renders import remove_gradient

    from dualband_reference import alignment_view

    dual = np.moveaxis(np.asarray(dual_planes, np.float32), 0, -1)
    broad = np.moveaxis(np.asarray(broad_planes, np.float32), 0, -1)
    dual = remove_gradient(dual)
    broad = remove_gradient(broad)
    transform, (source_pts, target_pts) = astroalign.find_transform(
        alignment_view(broad), alignment_view(dual), max_control_points=80, detection_sigma=4
    )
    registered, footprint = astroalign.apply_transform(transform, broad, dual, propagate_mask=True)
    valid = ~footprint
    scales = []
    for channel in range(3):
        ref = np.percentile(dual[..., channel][valid], 99.0)
        src = np.percentile(registered[..., channel][valid], 99.0)
        scales.append(ref / max(src, 1e-12))
    scaled_broad = registered * np.asarray(scales, dtype=np.float32)[None, None, :]
    hybrid = dual + weight * scaled_broad
    hybrid[~valid] = 0
    return np.moveaxis(hybrid, -1, 0).astype(np.float32)


def original_merge(old_planes, new_planes, old_exposures, new_exposures):
    """The merge of two nights of one band, on arrays."""
    import astroalign
    from renders import remove_gradient

    from dualband_reference import alignment_view

    old = remove_gradient(np.moveaxis(np.asarray(old_planes, np.float32), 0, -1))
    new = remove_gradient(np.moveaxis(np.asarray(new_planes, np.float32), 0, -1))
    transform, (source_pts, target_pts) = astroalign.find_transform(
        alignment_view(new), alignment_view(old), max_control_points=100, detection_sigma=4
    )
    registered, footprint = astroalign.apply_transform(transform, new, old, propagate_mask=True)
    valid = ~footprint
    scales = []
    for channel in range(3):
        reference = np.percentile(old[..., channel][valid], 99.0)
        incoming = np.percentile(registered[..., channel][valid], 99.0)
        scales.append(reference / max(incoming, 1e-12))
    registered *= np.asarray(scales, dtype=np.float32)[None, None, :]
    combined = old.copy()
    combined[valid] = (
        old_exposures * old[valid] + new_exposures * registered[valid]
    ) / (old_exposures + new_exposures)
    combined[~valid] = old[~valid]
    return np.moveaxis(combined, -1, 0).astype(np.float32)


def digest(planes) -> str:
    return hashlib.sha256(np.ascontiguousarray(planes, dtype="<f4").tobytes()).hexdigest()


@NEEDS_ENVIRONMENT
class Recipe(unittest.TestCase):
    """The module follows the reference transcription: same numbers, same order, same types."""

    @classmethod
    def setUpClass(cls) -> None:
        # A dual-band night (the continuum at 7 %, an Hα cloud), a broadband night moved and turned a little, with
        # a sky gradient, and a second dual-band night moved the other way.
        cls.dual = sky(1, colour=(0.07, 0.07, 0.07), level=0.02, cloud=0.05)
        cls.broad = sky(2, shift=(6.3, -4.8), angle=0.7, colour=(0.9, 1.0, 1.1), gradient=0.05)
        cls.dual_two = sky(3, shift=(-3.2, 5.1), angle=-0.4, colour=(0.07, 0.07, 0.07), level=0.025, cloud=0.05)

    def test_the_fusion_is_the_original_one_to_the_bit(self) -> None:
        import dualband_reference as dr

        hybrid, valid, fusion = dr.fuse(dr.to_pixels(self.dual), dr.to_pixels(self.broad))
        self.assertEqual(digest(dr.to_planes(hybrid)), digest(original_fuse(self.dual, self.broad)))
        self.assertEqual(fusion.weight, 0.18)
        self.assertGreaterEqual(fusion.stars, 20)

    def test_the_merge_of_two_nights_is_the_original_one_to_the_bit(self) -> None:
        import dualband_reference as dr

        # 120 and 81 poses of the same length: the weights are the frame counts.
        merged, record = dr.merge_band([self.dual, self.dual_two], [21_600.0, 14_580.0], [120, 81])
        self.assertEqual(record.weights, [120, 81])
        self.assertEqual(digest(dr.to_planes(merged)), digest(original_merge(self.dual, self.dual_two, 120, 81)))
        # The longest is the reference, whatever the order the masters come in.
        again, record = dr.merge_band([self.dual_two, self.dual], [14_580.0, 21_600.0], [81, 120])
        self.assertEqual(record.reference, 1)
        self.assertEqual(digest(dr.to_planes(again)), digest(dr.to_planes(merged)))

    def test_the_dual_band_master_gives_the_frame_and_the_broadband_one_a_fixed_share(self) -> None:
        import dualband_reference as dr
        from renders import remove_gradient

        hybrid, valid, fusion = dr.fuse(dr.to_pixels(self.dual), dr.to_pixels(self.broad))
        self.assertEqual(hybrid.shape[:2], SHAPE)
        # The broadband frame, moved by 6 px, leaves a black border on the dual-band one.
        self.assertTrue(0.9 < valid.mean() < 1.0)
        self.assertEqual(float(np.abs(hybrid[~valid]).max()), 0.0)
        # Inside, the image is the dual-band master (its background taken away) plus 0.18 of the broadband one
        # at the dual-band's scale: the broadband adds a fixed share, its time does not change it.
        added = hybrid - remove_gradient(dr.to_pixels(self.dual))
        expected = dr.register(remove_gradient(dr.to_pixels(self.broad)), remove_gradient(dr.to_pixels(self.dual)), 80)
        np.testing.assert_allclose(added[valid], (0.18 * expected.image * np.array(fusion.scales))[valid], atol=1e-6)
        # The sky gradient of the broadband night is not carried into the image.
        top, bottom = np.median(hybrid[20:60, 100:380, 1]), np.median(hybrid[340:380, 100:380, 1])
        self.assertLess(abs(float(top - bottom)), 0.002)

    def test_the_weights_are_the_frames_when_the_poses_are_alike(self) -> None:
        import dualband_reference as dr

        self.assertEqual(dr.weights_of([21_600.0, 14_580.0], [120, 81]), [120, 81])
        self.assertEqual(dr.weights_of([8_010.0, 14_400.0], [89, 120]), [8_010.0, 14_400.0])  # 90 s and 120 s
        self.assertEqual(dr.weights_of([3_600.0, 1_800.0], None), [3_600.0, 1_800.0])

    def test_three_masters_are_averaged_where_each_covers(self) -> None:
        import dualband_reference as dr

        third = sky(4, shift=(1.5, 2.5), colour=(0.07, 0.07, 0.07), level=0.02, cloud=0.05)
        merged, record = dr.merge_band([self.dual, self.dual_two, third], [7_200.0, 3_600.0, 3_600.0], [40, 20, 20])
        self.assertEqual(record.reference, 0)
        self.assertEqual(len(record.merged), 2)
        self.assertEqual(record.left_out, [])
        # Where every master covers, the pixel lies between them: a mean, not a sum.
        centre = merged[150:250, 200:280, 0]
        self.assertTrue(np.all(np.isfinite(centre)))
        self.assertLess(float(np.median(centre)), 0.1)

    def test_a_master_without_stars_is_left_out(self) -> None:
        import dualband_reference as dr

        cloudy = np.random.default_rng(5).normal(0.1, 0.001, (3, *SHAPE)).astype(np.float32)
        merged, record = dr.merge_band([self.dual, cloudy], [7_200.0, 3_600.0], [40, 20])
        self.assertEqual(record.left_out, [1])
        with self.assertRaises(dr.RegistrationError):
            dr.fuse(dr.to_pixels(self.dual), dr.to_pixels(cloudy))

    def test_the_empty_borders_of_a_turned_master_are_no_data(self) -> None:
        import dualband_reference as dr

        broad = self.broad.copy()
        broad[:, :, :40] = 0.0  # Siril's zeros where none of the night's frames reached
        _, valid, _ = dr.fuse(dr.to_pixels(self.dual), dr.to_pixels(broad))
        self.assertLess(valid.mean(), 0.93)
        inner = self.broad.copy()
        inner[:, 200, 200] = 0.0  # a single zero inside the data is data
        self.assertTrue(dr.data_of(dr.to_pixels(inner)).all())


    def test_one_broadband_master_on_the_dual_band_frame_is_the_fusion_to_the_bit(self) -> None:
        import dualband_reference as dr

        hybrid, valid, fusion, record = dr.fuse_band(dr.to_pixels(self.dual), [self.broad], [8_010.0], [89])
        alone, alone_valid, alone_fusion = dr.fuse(dr.to_pixels(self.dual), dr.to_pixels(self.broad))
        self.assertEqual(digest(dr.to_planes(hybrid)), digest(dr.to_planes(alone)))
        self.assertTrue(np.array_equal(valid, alone_valid))
        self.assertEqual((fusion.scales, fusion.stars, fusion.coverage), (alone_fusion.scales, alone_fusion.stars, alone_fusion.coverage))
        self.assertEqual((record.reference, record.merged, record.left_out), (0, [], []))

    def test_a_turned_broadband_night_does_not_crop_the_image(self) -> None:
        import dualband_reference as dr

        # The longest broadband night was taken with the camera turned by 39 degrees; a shorter one is on the
        # dual-band night's orientation.  Merged on the longest night's frame first, the image lost every
        # corner that night does not reach; registered each on the dual-band frame, the shorter night fills them.
        turned = sky(5, shift=(2.0, -1.5), angle=39.0, colour=(0.9, 1.0, 1.1), gradient=0.05)
        upright = sky(6, shift=(1.2, -0.8), angle=0.3, colour=(0.9, 1.0, 1.1), level=0.12)
        seconds, frames = [5_400.0, 3_600.0], [30, 20]
        dual = dr.to_pixels(self.dual)

        merged, _ = dr.merge_band([turned, upright], seconds, frames)
        _, cropped, _ = dr.fuse(dual, merged)
        self.assertGreater(1.0 - cropped.mean(), 0.10)  # the octagon of the old recipe

        hybrid, valid, fusion, record = dr.fuse_band(dual, [turned, upright], seconds, frames)
        self.assertLessEqual(1.0 - valid.mean(), 0.01)
        self.assertEqual(float(np.abs(hybrid[~valid]).max(initial=0.0)), 0.0)
        self.assertEqual(record.reference, 0)  # the longest gives the level
        self.assertEqual(record.left_out, [])
        self.assertEqual(len(record.merged), 1)
        self.assertAlmostEqual(abs(record.merged[0]["rotationDegrees"]), 39.0 - 0.3, delta=0.5)
        # The covers are on the dual-band frame: the turned night's misses the corners, the union is the cover.
        self.assertLess(record.covers[0].mean(), 0.9)
        self.assertTrue(np.array_equal(record.covers[0] | record.covers[1], valid))
        # One level: the corners (one night) and the centre (both) are the same sky, no step at the seam.
        added = hybrid - dr.remove_gradient(dual)
        only_upright = valid & ~record.covers[0]
        both = record.covers[0] & record.covers[1]
        self.assertLess(abs(float(np.median(added[only_upright][:, 1]) - np.median(added[both][:, 1]))), 0.002)
        # And the same stars: the broadband share at a star of a corner is there, not black.
        self.assertGreater(float(added[only_upright][:, 1].max()), 0.05 * float(added[both][:, 1].max()))


PARITY_DIR = os.environ.get("ASTRO_SYNC_PARITY_DIR")


@NEEDS_ENVIRONMENT
@unittest.skipUnless(PARITY_DIR and (Path(PARITY_DIR) / "parity.json").is_file(),
                     "set ASTRO_SYNC_PARITY_DIR to a folder with a parity.json (real masters and the expected image)")
class Parity(unittest.TestCase):
    """Real masters, and the expected image (``parity.json``)::

        {"cases": [{"name": "...", "dualband": [{"path": "...", "seconds": 21600, "frames": 120}, ...],
                    "broadband": [{"path": "...", "seconds": 8010, "frames": 89}],
                    "expected": "<the expected linear FITS>"}]}

    Relative paths are read from the folder.  The data of the image must be the same, value for value.
    """

    def test_the_port_gives_the_original_image(self) -> None:
        import dualband_reference as dr

        folder = Path(PARITY_DIR)
        cases = json.loads((folder / "parity.json").read_text(encoding="utf-8"))["cases"]

        def read(entry):
            return np.asarray(fits.getdata(folder / entry["path"]), np.float32)

        for case in cases:
            with self.subTest(case["name"]):
                bands = []
                for key in ("dualband", "broadband"):
                    entries = case.get(key) or []
                    if entries:
                        image, _ = dr.merge_band([read(e) for e in entries], [float(e["seconds"]) for e in entries],
                                                 [int(e["frames"]) for e in entries])
                        bands.append(image)
                result = dr.fuse(*bands)[0] if len(bands) == 2 else bands[0]
                expected = np.asarray(fits.getdata(folder / case["expected"]), np.float32)
                self.assertEqual(digest(dr.to_planes(result)), digest(expected))


if __name__ == "__main__":
    unittest.main()
