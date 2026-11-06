"""Tests of the framing chosen from the field rotation.

Run from this folder, with the stacking environment:
    python3 -m unittest -v test_registration_framing

The first cases need only the standard library.  The end-to-end cases run
Siril (``siril-cli`` on the PATH, or ``ASTRO_SYNC_SIRIL``) on synthetic frames
of an alt-azimuth night (the field turns by 100°: finding TD1, the stacking that
stopped on « The intersection of all images is null or negative ») and of an
equatorial one; they are skipped without it.
"""

from __future__ import annotations

import json
import math
import os
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

import astro_sync_stack
import registration_framing
from registration_framing import (
    COMMON_AREA, FULL_FIELD, Registration, apply_command, combine_nights, combined_span, read_registration,
    rotation_span,
)

HERE = Path(__file__).resolve().parent

try:
    import numpy as np
    from astropy.io import fits
    from synthetic_nights import SHAPE, sky_around_the_pointing, write_night
except ImportError:  # the stacking environment has them; a bare interpreter does not
    np = fits = None
    SHAPE = (256, 320)

NEEDS_NUMPY = unittest.skipIf(np is None, "needs numpy and astropy (the stacking environment)")


def frames_of(lights: list[Path]) -> dict[str, list[Path]]:
    """The frames of a plan: lights only, no calibration."""
    return {**{kind: [] for kind in astro_sync_stack.KINDS}, "lights": lights}


def sequence(angles: list[float | None], layer: str = "R1", pivot: tuple[float, float] = (1080.0, 1920.0)) -> str:
    """The text of a Siril sequence file whose frame ``i`` is turned by ``angles[i]`` degrees (``None``: not registered)."""
    lines = [
        "#Siril sequence file. Contains list of images, selection, registration data and statistics",
        f"S 'light_' 1 {len(angles)} {sum(a is not None for a in angles)} 5 1 6 0 0 0",
        "L 3",
    ]
    lines += [f"I {number} {int(angle is not None)}" for number, angle in enumerate(angles, 1)]
    for angle in angles:
        if angle is None:
            lines.append(f"{layer} 0 0 0 0 0 0 H 0 0 0 0 0 0 0 0 0")
            continue
        c, s = math.cos(math.radians(angle)), math.sin(math.radians(angle))
        tx = pivot[0] - (c * pivot[0] - s * pivot[1])
        ty = pivot[1] - (s * pivot[0] + c * pivot[1])
        lines.append(f"{layer} 3.5 4.1 0.8 0 3167.9 676 H {c} {-s} {tx} {s} {c} {ty} 0 0 1")
    return "\n".join(lines) + "\n"


class FieldRotation(unittest.TestCase):
    def test_a_field_that_does_not_turn_has_no_span(self) -> None:
        self.assertEqual(rotation_span([0.2, 0.2, 0.2]), 0.0)
        self.assertEqual(rotation_span([]), 0.0)
        self.assertEqual(rotation_span([42.0]), 0.0)

    def test_a_progressive_rotation_is_measured_whole(self) -> None:
        # The Seestar of 10/11 October 2026: from 0° to 125° in 5 h 30.
        self.assertAlmostEqual(rotation_span([0.0, -2.9, -11.3, -19.5, -103.1, -114.0, -124.7]), 124.7, places=6)
        # Beyond a half turn too: the way is followed frame to frame.
        self.assertAlmostEqual(rotation_span([float(a) for a in range(0, 200, 10)]), 190.0, places=6)

    def test_a_meridian_flip_is_no_rotation(self) -> None:
        # The frame turns by 180° and the stars stay where they are.
        self.assertAlmostEqual(rotation_span([0.1, 0.1, 180.1, 180.0, -179.9, 0.2]), 0.2, places=6)

    def test_the_way_crosses_the_fold_without_a_jump(self) -> None:
        self.assertAlmostEqual(rotation_span([85.0, 95.0]), 10.0, places=6)
        self.assertAlmostEqual(rotation_span([-85.0, -95.0]), 10.0, places=6)


class ReadRegistration(unittest.TestCase):
    def read(self, text: str) -> Registration | None:
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / "light_.seq"
            path.write_text(text, encoding="utf-8")
            return read_registration(path)

    def test_an_alt_azimuth_night_keeps_the_whole_field(self) -> None:
        found = self.read(sequence([-a for a in range(0, 126, 5)]))
        self.assertEqual((found.frames, found.registered, found.unregistered), (26, 26, ()))
        self.assertAlmostEqual(found.rotation_degrees, 125.0, places=1)
        self.assertTrue(found.progressive)
        self.assertEqual(found.framing, FULL_FIELD)

    def test_an_equatorial_night_keeps_the_common_area(self) -> None:
        found = self.read(sequence([0.0, 0.05, -0.1, 0.2, 0.15, 0.1]))
        self.assertFalse(found.progressive)
        self.assertEqual(found.framing, COMMON_AREA)
        flipped = self.read(sequence([0.0, 0.1, 180.1, 180.0, 0.2]))
        self.assertEqual(flipped.framing, COMMON_AREA)

    def test_the_threshold_decides(self) -> None:
        below = self.read(sequence([0.0, registration_framing.ROTATION_THRESHOLD_DEGREES - 0.5]))
        above = self.read(sequence([0.0, registration_framing.ROTATION_THRESHOLD_DEGREES + 0.5]))
        self.assertEqual((below.framing, above.framing), (COMMON_AREA, FULL_FIELD))

    def test_a_frame_siril_could_not_match_is_named_and_left_out(self) -> None:
        # Finding TD1: the frame of 04:53 (« Cannot perform star matching: try #3 »), index 4 here.
        found = self.read(sequence([0.0, -30.0, -60.0, None, -90.0]))
        self.assertEqual((found.frames, found.registered, found.unregistered), (5, 4, (4,)))
        self.assertAlmostEqual(found.rotation_degrees, 90.0, places=1)

    def test_a_mono_sequence_is_read_on_its_own_layer(self) -> None:
        self.assertEqual(self.read(sequence([0.0, -40.0], layer="R0")).framing, FULL_FIELD)

    def test_what_cannot_be_read_gives_nothing(self) -> None:
        self.assertIsNone(read_registration(Path("/nonexistent/light_.seq")))
        self.assertIsNone(self.read("S 'light_' 1 0 0 5 1 6 0 0 0\n"))
        # No registration data: the common area, as before the measure.
        self.assertIsNone(self.read("S 'light_' 1 2 2 5 1 6 0 0 0\nI 1 1\nI 2 1\n"))
        # The data does not match the images.
        self.assertIsNone(self.read(sequence([0.0, -40.0]) + "R1 1 1 1 0 1 1 H 1 0 0 0 1 0 0 0 1\n"))

    def test_one_frame_alone_has_nothing_to_measure_a_rotation_on(self) -> None:
        found = self.read(sequence([0.0, None, None]))
        self.assertIsNone(found.rotation_degrees)
        self.assertEqual(found.framing, COMMON_AREA)


class Range(unittest.TestCase):
    """The range of the rotation of a night, relative to its reference frame: what a master keeps for a plan."""

    def read(self, angles: list[float | None]) -> Registration:
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / "light_.seq"
            path.write_text(sequence(angles), encoding="utf-8")
            return read_registration(path)

    def test_the_range_is_read_from_the_reference_frame(self) -> None:
        # The reference (identity) is the third frame: the field turned both ways around it.
        found = self.read([30.0, 15.0, 0.0, -20.0, -45.0])
        self.assertEqual(found.rotation_range, (-45.0, 30.0))
        self.assertAlmostEqual(found.rotation_degrees, 75.0, places=1)

    def test_the_range_does_not_depend_on_the_fold(self) -> None:
        # A night turned from the reference frame by 100° the one way: followed through the fold at 90°.
        found = self.read([0.0, -50.0, -100.0, -130.0])
        self.assertEqual(found.rotation_range, (-130.0, 0.0))
        # A meridian flip in the night does not count.
        self.assertEqual(self.read([0.0, 0.1, 180.1, 180.0]).rotation_range, (0.0, 0.1))

    def test_nothing_to_measure_gives_no_range_but_the_orientations(self) -> None:
        found = self.read([0.0, None, None])
        self.assertIsNone(found.rotation_range)
        self.assertEqual(found.orientations, (0.0, None, None))

    def test_the_orientation_of_each_frame_is_folded(self) -> None:
        found = self.read([10.0, None, 100.0, -170.0])
        self.assertEqual([None if o is None else round(o, 3) for o in found.orientations], [10.0, None, -80.0, 10.0])


class Nights(unittest.TestCase):
    """The rotation over all the frames of all the nights of a plan, a meridian flip not counted."""

    @staticmethod
    def masters(orientations: list[float | None]) -> Registration:
        unregistered = tuple(number for number, o in enumerate(orientations, 1) if o is None)
        return Registration(frames=len(orientations), unregistered=unregistered, rotation_degrees=1.0,
                            orientations=tuple(orientations))

    def test_two_nights_that_do_not_turn_but_are_oriented_differently(self) -> None:
        found = combine_nights(self.masters([0.0, 60.0]), [(0.0, 0.0), (0.0, 0.0)])
        self.assertAlmostEqual(found.rotation_degrees, 60.0)
        self.assertEqual(found.framing, FULL_FIELD)

    def test_the_rotation_within_the_nights_adds_to_the_difference_between_them(self) -> None:
        # The second night's reference frame is 40° from the first's; each night turned by 20°: from -20° to 60° in all.
        found = combine_nights(self.masters([0.0, 40.0]), [(-20.0, 0.0), (0.0, 20.0)])
        self.assertAlmostEqual(found.rotation_degrees, 80.0)
        # The same orientation, and the nights' own turns overlapping.
        found = combine_nights(self.masters([0.0, 10.0]), [(0.0, 30.0), (-10.0, 5.0)])
        self.assertAlmostEqual(found.rotation_degrees, 30.0)

    def test_one_night_alone_that_turns_is_enough(self) -> None:
        found = combine_nights(self.masters([0.0, 0.2]), [(-62.0, 0.0), (0.0, 0.0)])
        self.assertEqual(found.framing, FULL_FIELD)
        self.assertAlmostEqual(found.rotation_degrees, 62.0, delta=0.5)

    def test_equatorial_nights_stay_on_the_common_area(self) -> None:
        found = combine_nights(self.masters([0.0, 0.15, -0.1]), [(-0.1, 0.1), (0.0, 0.2), (-0.3, 0.0)])
        self.assertEqual(found.framing, COMMON_AREA)

    def test_a_meridian_flip_between_nights_is_no_rotation(self) -> None:
        # The second night was shot the other side of the pier: its master is turned by 180° and nothing else.
        found = combine_nights(self.masters([0.0, 179.9]), [(0.0, 0.1), (-0.1, 0.0)])
        self.assertLess(found.rotation_degrees, 0.5)
        self.assertEqual(found.framing, COMMON_AREA)

    def test_a_master_that_could_not_be_registered_does_not_count(self) -> None:
        found = combine_nights(self.masters([0.0, None, 0.3]), [(0.0, 0.0), (-90.0, 90.0), (0.0, 0.0)])
        self.assertLess(found.rotation_degrees, 1.0)
        self.assertEqual(found.unregistered, (2,))

    def test_the_smallest_arc_is_taken_round_the_circle_of_half_a_turn(self) -> None:
        self.assertAlmostEqual(combined_span([(-80.0, 0.0), (80.0, 0.0)]), 20.0)
        self.assertAlmostEqual(combined_span([(0.0, 190.0), (50.0, 0.0)]), 190.0)
        self.assertEqual(combined_span([]), 0.0)

    def test_nothing_measured_keeps_the_common_area(self) -> None:
        unread = Registration(frames=2, unregistered=(), rotation_degrees=None)
        self.assertIs(combine_nights(unread, [(0.0, 0.0), (0.0, 0.0)]), unread)
        self.assertEqual(unread.framing, COMMON_AREA)

    def test_one_range_per_master_is_needed(self) -> None:
        with self.assertRaises(ValueError):
            combine_nights(self.masters([0.0, 1.0]), [(0.0, 0.0)])
        # With a single master registered there is nothing to combine: the registration comes back as it was.
        alone = self.masters([0.0, None])
        self.assertIs(combine_nights(alone, [(0.0, 0.0), (0.0, 0.0)]), alone)


class Scripts(unittest.TestCase):

    PLAN = astro_sync_stack.Plan(frames=frames_of([Path("a.fit"), Path("b.fit")]), colour=True, pipelines=[], linear_only=True)

    def scripts(self, framing: str) -> dict[str, str]:
        return dict(astro_sync_stack.siril_scripts(self.PLAN, framing)[0])

    def test_the_registration_only_computes_the_transforms(self) -> None:
        scripts = self.scripts(COMMON_AREA)
        self.assertEqual(list(scripts), ["calibration", "registration", "alignment", "integration"])
        self.assertIn("register light -2pass -transf=affine", scripts["registration"])
        self.assertNotIn("seqapplyreg", scripts["registration"])

    def test_the_alignment_takes_the_framing_the_measure_chose(self) -> None:
        self.assertIn("seqapplyreg light -prefix=r_ -framing=min -interp=lanczos4", self.scripts(COMMON_AREA)["alignment"])
        self.assertIn("seqapplyreg light -prefix=r_ -framing=current -interp=lanczos4", self.scripts(FULL_FIELD)["alignment"])

    def test_the_framing_command_is_the_one_both_stackings_use(self) -> None:
        self.assertEqual(apply_command("plan", FULL_FIELD), "seqapplyreg plan -prefix=r_ -framing=current -interp=lanczos4")
        self.assertEqual(apply_command("light", COMMON_AREA), "seqapplyreg light -prefix=r_ -framing=min -interp=lanczos4")
        self.assertIn(apply_command("light", FULL_FIELD), self.scripts(FULL_FIELD)["alignment"])

    def test_the_default_is_the_framing_of_before(self) -> None:
        self.assertIn("-framing=min", dict(astro_sync_stack.siril_scripts(self.PLAN)[0])["alignment"])

    def test_the_integration_does_not_depend_on_the_framing(self) -> None:
        self.assertEqual(self.scripts(COMMON_AREA)["integration"], self.scripts(FULL_FIELD)["integration"])


class Fields(unittest.TestCase):
    NAMES = [f"Light_{number}.fit" for number in range(1, 6)]

    def test_the_event_names_the_frames_left_out(self) -> None:
        found = Registration(frames=5, unregistered=(4,), rotation_degrees=90.0)
        self.assertEqual(astro_sync_stack.registration_fields(found, self.NAMES), {
            "unit": "lights", "frames": 5, "registered": 4, "unregistered": ["Light_4.fit"],
            "fieldRotationDegrees": 90.0, "fieldRotation": "progressive", "framing": "full",
        })

    def test_the_rotation_is_none_when_it_was_not_measured(self) -> None:
        fields = astro_sync_stack.registration_fields(Registration(frames=5, unregistered=(), rotation_degrees=None), self.NAMES)
        self.assertEqual((fields["fieldRotation"], fields["framing"]), (None, "common"))
        fields = astro_sync_stack.registration_fields(Registration(frames=5, unregistered=(), rotation_degrees=0.3), self.NAMES)
        self.assertEqual((fields["fieldRotation"], fields["framing"]), ("none", "common"))

    def test_a_sequence_that_cannot_be_read_keeps_the_framing_of_before(self) -> None:
        found = astro_sync_stack.measured_registration(Path("/nonexistent/light_.seq"), 5)
        self.assertEqual((found.framing, found.rotation_degrees, found.registered), (COMMON_AREA, None, 5))

    def test_fewer_than_two_aligned_lights_stop_the_stacking(self) -> None:
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / "light_.seq"
            path.write_text(sequence([0.0, None, None]), encoding="utf-8")
            with self.assertRaises(astro_sync_stack.StackingError) as raised:
                astro_sync_stack.measured_registration(path, 3)
        self.assertEqual(raised.exception.code, "registration_failed")
        self.assertIn("only 1 of 3", str(raised.exception))


SIRIL = os.environ.get("ASTRO_SYNC_SIRIL") or shutil.which("siril-cli")



@unittest.skipIf(SIRIL is None, "needs siril-cli (Siril 1.4)")
class EndToEnd(unittest.TestCase):
    """Synthetic mono frames of one star field, stacked by the real script and Siril."""

    SHAPE = SHAPE

    def setUp(self) -> None:
        self.root = Path(tempfile.mkdtemp())
        if not os.environ.get("ASTRO_SYNC_KEEP_E2E"):
            self.addCleanup(lambda: shutil.rmtree(self.root, ignore_errors=True))
        self.sky = sky_around_the_pointing()

    def night(self, name: str, angles: list[float], drift: float = 0.0, blank: set[int] = frozenset()) -> Path:
        return write_night(self.root, name, self.sky, angles, self.SHAPE, drift, blank)

    def stack(self, lights: Path) -> tuple[int, list[dict], dict | None]:
        out = lights.parent / "stacked"
        result = subprocess.run(
            [sys.executable, str(HERE / "astro_sync_stack.py"), "--lights", str(lights),
             "--work", str(lights.parent.parent / ".work"), "--out", str(out), "--name", "M45",
             "--linear-only", "--siril", SIRIL, "--json"],
            capture_output=True, text=True, timeout=900,
        )
        events = [json.loads(line) for line in result.stdout.splitlines()]
        provenance = out / "M45_stacking.json"
        return result.returncode, events, json.loads(provenance.read_text()) if provenance.exists() else None

    def test_an_alt_azimuth_night_is_stacked_on_the_whole_field(self) -> None:
        # Finding TD1: with the common area this night stops on « The intersection of all images is null or negative ».
        lights = self.night("altaz", [-10.0 * number for number in range(11)])
        code, events, provenance = self.stack(lights)
        self.assertEqual(code, 0, events[-1])
        registration = next(e for e in events if e["event"] == "registration")
        self.assertEqual((registration["framing"], registration["fieldRotation"]), ("full", "progressive"))
        self.assertAlmostEqual(registration["fieldRotationDegrees"], 100.0, delta=2.0)
        self.assertEqual((registration["registered"], registration["unregistered"]), (11, []))
        self.assertEqual(provenance["registration"], {k: v for k, v in registration.items() if k != "event"})
        self.assertEqual((provenance["width"], provenance["height"]), (self.SHAPE[1], self.SHAPE[0]))
        self.assertEqual(provenance["stackedFrames"], 11)
        self.assertTrue(any("-framing=current" in line for line in provenance["recipe"]["siril"]))
        self.assertEqual([e["step"] for e in events if e["event"] == "step"],
                         ["calibration", "strip_wcs", "registration", "integration", "linear_export"])

    def test_a_frame_that_cannot_be_matched_is_left_out_and_named(self) -> None:
        # Finding TD1: the frame of 04:53 only had to be left out, and said.
        lights = self.night("blank", [-8.0 * number for number in range(10)], blank={6})
        code, events, provenance = self.stack(lights)
        self.assertEqual(code, 0, events[-1])
        registration = next(e for e in events if e["event"] == "registration")
        self.assertEqual((registration["registered"], registration["unregistered"]), (9, ["Light_006.fit"]))
        self.assertEqual(provenance["stackedFrames"], 9)

    def test_an_equatorial_night_keeps_the_common_area(self) -> None:
        lights = self.night("equatorial", [0.0, 0.05, -0.08, 0.1, 0.02, -0.03, 0.06, 0.0], drift=0.4)
        code, events, provenance = self.stack(lights)
        self.assertEqual(code, 0, events[-1])
        registration = next(e for e in events if e["event"] == "registration")
        self.assertEqual((registration["framing"], registration["fieldRotation"]), ("common", "none"))
        self.assertLess(registration["fieldRotationDegrees"], 1.0)
        self.assertTrue(any("-framing=min" in line for line in provenance["recipe"]["siril"]))
        # The common area is a little smaller than the frame.
        self.assertLess(provenance["width"], self.SHAPE[1])


if __name__ == "__main__":
    unittest.main()
