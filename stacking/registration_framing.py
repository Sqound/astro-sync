"""The framing of a registration, chosen from the field rotation it measured.

Siril registers a night's frames on a reference one, then ``seqapplyreg``
brings each of them onto a common frame.  ``-framing=min`` (the common area:
every pixel of the result is covered by every frame) is right for an
equatorial mount, whose field hardly turns, and for the 180° turn of a
meridian flip.  It is wrong for an alt-azimuth mount, whose field turns all
night: on the 430 frames of the Seestar S50 Pro of the 10/11 October 2026
(5 h 30, 125° of rotation) the area common to every frame is a patch of about
270 px on a 2160 x 3840 frame, and Siril stops with « The intersection of all
images is null or negative » (TD1; the frame of 04:53 that Siril could not
match was a red herring, it is only left out).

The mount cannot be read in the headers (the Seestar says ``EQMODE = 0``, the other
capture software say nothing), so the alignment measures it: ``register`` writes the
transform of each frame in the sequence file (``<sequence>.seq``, lines
``R<layer> fwhm wfwhm roundness quality background stars H h00 h01 h02 h10 h11
h12 h20 h21 h22``), whose rotation is ``atan2(h10, h00)``.

* **The field rotation** is the span of that angle along the night, in the
  order of the frames, each angle folded modulo 180° first (a meridian flip
  turns the frame by 180° and leaves the stars where they are: it is no
  rotation of the field).
* **Below** ``ROTATION_THRESHOLD_DEGREES`` the field does not turn (equatorial
  mount): the common area, as before.
* **At or above** it the field turns progressively (alt-azimuth mount): the
  whole field of the reference frame (``-framing=current``, like the Seestar's own stacking).
  The result keeps the size of one frame
  and the sky of the reference one; its edges are covered by fewer frames, so a
  little noisier.  Siril's stacking leaves out the pixels a frame does not cover:
  there is no dark border.

A multi-night plan (``plan_stack.py``) takes the same decision on the masters of
its nights (and **never assumes a fixed field for a master that cannot say**: the image a device
stacked says nothing of how the field turned during its night, so a plan that holds one keeps the
whole field): ``combine_nights`` measures the rotation over *all the frames of all the nights*
(the orientation of each night's master on the reference one, plus the rotation of the frames
within that night, which the master keeps in its header), and ``apply_command`` is the one line
of Siril that applies the framing chosen, for a night and for a plan.

Only the standard library is used.
"""

from __future__ import annotations

import math
from dataclasses import dataclass, replace
from pathlib import Path

COMMON_AREA = "common"
FULL_FIELD = "full"
# The word of Siril's ``seqapplyreg -framing=`` for each framing.
SIRIL_FRAMING = {COMMON_AREA: "min", FULL_FIELD: "current"}

# The span of the field rotation from which the common area is no longer
# kept.  Measured on real nights:
# the Askar 50P on an equatorial mount, polar aligned, turns by well under a
# degree whatever the length of the night; the Seestar S50 Pro, alt-azimuth,
# by 125° in 5 h 30.  At 3° the common area of a 2160 x 3840 frame still keeps
# more than 95% of it, and the whole field of the reference frame is then
# worth more than that sliver.
ROTATION_THRESHOLD_DEGREES = 3.0


def apply_command(sequence: str, framing: str) -> str:
    """The Siril command that brings the registered frames of ``sequence`` onto the frame ``framing`` names."""
    return f"seqapplyreg {sequence} -prefix=r_ -framing={SIRIL_FRAMING[framing]} -interp=lanczos4"


@dataclass(frozen=True)
class Registration:
    """What a registration said of its frames (read from the sequence file)."""

    frames: int
    # 1-based indexes, in the sequence, of the frames that have no transform.
    unregistered: tuple[int, ...]
    # The span of the field rotation in degrees; ``None`` when the sequence
    # file gave fewer than two transforms to measure it on.
    rotation_degrees: float | None
    # The range of that rotation, ``(low, high)`` in degrees relative to the
    # reference frame (``low`` <= 0 <= ``high``); ``None`` with the span.  A
    # master keeps it in its header, so that a plan can measure the rotation
    # over all the frames of all its nights.
    rotation_range: tuple[float, float] | None = None
    # The orientation of each frame's transform, in degrees folded to
    # [-90, 90) (``None``: not registered), in the order of the sequence.
    orientations: tuple[float | None, ...] = ()
    # A master of the plan could not say how the field turned within its night (the image of a device): the
    # rotation above is then only what is known, and the whole field is kept whatever its span.
    rotation_unknown: bool = False

    @property
    def registered(self) -> int:
        return self.frames - len(self.unregistered)

    @property
    def progressive(self) -> bool:
        return self.rotation_degrees is not None and self.rotation_degrees >= ROTATION_THRESHOLD_DEGREES

    @property
    def framing(self) -> str:
        return FULL_FIELD if self.progressive or self.rotation_unknown else COMMON_AREA


def _transform(fields: list[str]) -> tuple[float, float, float, float] | None:
    """``(h00, h01, h10, h11)`` of an ``R`` line, ``None`` for a frame Siril could not register."""
    if "H" not in fields:
        return None
    try:
        values = [float(v) for v in fields[fields.index("H") + 1:fields.index("H") + 10]]
    except ValueError:
        return None
    if len(values) != 9:
        return None
    h00, h01, _, h10, h11, _, _, _, h22 = values
    if h22 == 0.0 or h00 * h11 - h01 * h10 == 0.0:
        return None  # an all-zero matrix: the frame was skipped (« Cannot perform star matching »)
    return h00, h01, h10, h11


def rotation_path(angles: list[float]) -> list[float]:
    """The rotation of each frame followed along the night, in degrees.

    ``angles``: the rotation of each registered frame, in the order of the
    frames.  Each is folded into [-90, 90) first, then followed from frame to
    frame by the shortest way modulo 180°, so a frame turned by 180° counts
    as turned by none and a rotation of 125° is followed as 125°.
    """
    if not angles:
        return []
    follow = ((angles[0] + 90.0) % 180.0) - 90.0
    path = [follow]
    for angle in angles[1:]:
        step = (((angle + 90.0) % 180.0) - 90.0) - follow
        step -= 180.0 * round(step / 180.0)
        follow += step
        path.append(follow)
    return path


def rotation_span(angles: list[float]) -> float:
    """The span, in degrees, of the field rotation along a night (``rotation_path``)."""
    if len(angles) < 2:
        return 0.0
    path = rotation_path(angles)
    return max(path) - min(path)


def combined_span(arcs: list[tuple[float, float]]) -> float:
    """The smallest turn, in degrees, that holds every ``(start, length)`` arc of orientations.

    Orientations count modulo 180° (a meridian flip leaves the stars where
    they are), so the arcs lie on a circle of 180°: two nights 179° apart are
    1° apart.  An arc of 180° or more, a night that turned half a turn, holds
    everything.
    """
    if not arcs:
        return 0.0
    longest = max(length for _, length in arcs)
    if longest >= 180.0:
        return longest
    return min(max(((start - first) % 180.0) + length for start, length in arcs) for first, _ in arcs)


def combine_nights(masters: Registration, nights: list[tuple[float, float] | None]) -> Registration:
    """The registration of a plan's masters seen as one of all the frames of all the nights.

    ``masters``: what the registration of the masters (one per night and band)
    on the reference one said.  ``nights[i]``: the range of the rotation of the
    frames within master ``i``, relative to its own reference frame (what the
    master keeps in its header), or ``None`` when the master cannot say (the image
    of a device, or a header without the range).  The rotation of the result is the
    span of every frame of every night: the orientation of its master on the
    reference one, plus the rotation within its night; a master that cannot say counts
    for its orientation only, and the result is marked ``rotation_unknown`` (the whole
    field is kept: a field assumed fixed would crop the plan for nothing).
    """
    if masters.rotation_degrees is None:
        # Nothing measured (a sequence file that could not be read): the common area, as before.
        return replace(masters, rotation_unknown=True) if any(n is None for n in nights) else masters
    if len(nights) != masters.frames or len(masters.orientations) != masters.frames:
        raise ValueError("one range of rotation per master is needed")
    arcs = [
        (orientation + (night[0] if night else 0.0), (night[1] - night[0]) if night else 0.0)
        for orientation, night in zip(masters.orientations, nights)
        if orientation is not None
    ]
    unknown = any(night is None for orientation, night in zip(masters.orientations, nights) if orientation is not None)
    if len(arcs) < 2:
        return replace(masters, rotation_unknown=True) if unknown else masters
    return replace(masters, rotation_degrees=round(combined_span(arcs), 2), rotation_range=None, rotation_unknown=unknown)


def _registered_layer(sequence: Path) -> list[list[str] | None] | None:
    """The ``R`` lines of the layer Siril registered, one per image (``None``: not registered), or ``None``."""
    try:
        text = sequence.read_text(encoding="utf-8", errors="replace")
    except OSError:
        return None
    images = 0
    layers: dict[str, list[list[str] | None]] = {}
    for line in text.splitlines():
        fields = line.split()
        if not fields:
            continue
        if fields[0] == "I":
            images += 1
        elif fields[0][0] == "R" and fields[0][1:].isdigit():
            layers.setdefault(fields[0], []).append(fields if _transform(fields) is not None else None)
    if images == 0 or not layers:
        return None
    # Siril registers a colour image on one layer (the green): the layer that has data for the frames.
    found = max(layers.values(), key=lambda lines: sum(f is not None for f in lines))
    return found if len(found) == images else None


def read_matrices(sequence: Path) -> list[tuple[float, ...] | None] | None:
    """The 3 x 3 transform of each image of a Siril sequence file, row by row (``None``: not registered).

    Siril's matrix brings a point of the image onto the reference one, in its own coordinates: ``x`` the
    column plus a half, ``y`` counted from the **last** of the rows ``astropy`` reads (measured on the
    registered frames of a turning, drifting synthetic night: ``night_framing.py``).
    """
    lines = _registered_layer(sequence)
    if lines is None:
        return None
    return [None if fields is None else tuple(float(v) for v in fields[fields.index("H") + 1:fields.index("H") + 10])
            for fields in lines]


def read_registration(sequence: Path) -> Registration | None:
    """The registration written in a Siril sequence file; ``None`` when it cannot be read."""
    lines = _registered_layer(sequence)
    if lines is None:
        return None
    images = len(lines)
    transforms = [None if fields is None else _transform(fields) for fields in lines]
    unregistered = tuple(number for number, found in enumerate(transforms, 1) if found is None)
    found = [t for t in transforms if t is not None]
    angles = [math.degrees(math.atan2(t[2], t[0])) for t in found]
    orientations = tuple(
        None if t is None else ((math.degrees(math.atan2(t[2], t[0])) + 90.0) % 180.0) - 90.0 for t in transforms
    )
    if len(found) < 2:
        return Registration(frames=images, unregistered=unregistered, rotation_degrees=None, orientations=orientations)
    path = rotation_path(angles)
    # The reference frame (Siril gives it the identity) is the origin of the range.
    reference = min(range(len(found)), key=lambda i: _distance_from_identity(found[i]))
    origin = 180.0 * round(path[reference] / 180.0)
    return Registration(
        frames=images,
        unregistered=unregistered,
        rotation_degrees=round(max(path) - min(path), 2),
        rotation_range=(round(min(path) - origin, 2), round(max(path) - origin, 2)),
        orientations=orientations,
    )


def _distance_from_identity(transform: tuple[float, float, float, float]) -> float:
    h00, h01, h10, h11 = transform
    return abs(h00 - 1.0) + abs(h11 - 1.0) + abs(h01) + abs(h10)
