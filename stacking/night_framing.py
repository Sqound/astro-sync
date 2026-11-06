"""The frame of the stacking of a night, chosen from what every frame covers.

When the field turns (an alt-azimuth mount: ``registration_framing.py``), the night is stacked on the
**whole field** of the reference frame, whose edges fewer frames cover.  The person may keep a smaller frame,
down to the rectangle every frame covers.  This module measures the frames on offer, from the transforms of
the registration, and cuts the one chosen out of the linear master; it is the single place both happen (the
numbers on the screen are the ones written here).

**The coverage.**  Each frame covers, on the reference one, its own rectangle carried by its transform
(``registration_framing.read_matrices``: Siril's matrix brings a point of the frame onto the reference one; its
``y`` counts from the last of the rows ``astropy`` reads, its ``x`` and ``y`` are the pixel's centre).  The count
of frames is computed on a grid of about ``GRID_SIDE`` points on the long side, not on every pixel: a night of
430 frames of 2160 x 3840 is measured in a second.  The convention was measured, not assumed: on a synthetic
night that turns by 20° and drifts by 3 px a frame, the predicted count matches the zeros of the frames Siril
registered on 97.7 % of the pixels (the rest are the border Lanczos leaves; the seven other conventions, 71 to
85 %).

**The notches** (``plan_framing.Measure``, the same logic as the frame of a multi-night plan): five stops
whose four edges go in a straight line from the largest rectangle every frame covers (the *common area*, rank 0)
to the whole field (rank 4), so the rectangles are nested.  Each says its size, its share of the whole field,
the share of it every frame covers and the frames at its lowest point.

**Automatic** is the framing of before, unchanged: the common area when the field does not turn (Siril's
``-framing=min``, nothing is measured, nothing is offered), the whole field when it turns.  A notch asked for
(``--frame-size``) is applied only to a field that turns; on one that does not, there is nothing to choose.

**The crop** comes after the stacking, before the renders: the linear master of the whole field is cut, then
brought back to 0..1 as Siril's ``-output_norm`` brings a stacking (when it did).  Nothing is stacked again,
and the master of the whole field stays in ``stacked/`` (``<name>_field.fit``) so another frame is chosen later
without stacking.  Measured with Siril 1.4 on synthetic nights (a crop of the whole field against the stacking
of that same rectangle by ``-framing=min``): the same image to 0.01 % of the range in rms on an equatorial night,
0.03 % on a night that turns by 20°, 0.07 % in colour; the largest gap, on a few star peaks and the first column,
is 1 to 3 % of the range (the rejection and the weights see a few more pixels).

Only numpy and astropy are needed (the stacking environment).
"""

from __future__ import annotations

import math
from pathlib import Path

import numpy as np

import plan_combine
import plan_framing

# The long side of the grid the coverage is counted on.
GRID_SIDE = 480

# The stops of the slider: the common area, three frames between, the whole field.
NOTCH_COUNT = 5

AUTO = "auto"


def frame_size_arg(value: str) -> str | int:
    """``--frame-size``: ``auto`` or the rank of a notch, 0 (the common area) to 4 (the whole field)."""
    if value == AUTO:
        return AUTO
    try:
        rank = int(value)
    except ValueError:
        rank = -1
    if not 0 <= rank < NOTCH_COUNT:
        import argparse

        raise argparse.ArgumentTypeError(f"'{value}' is not auto nor a notch from 0 to {NOTCH_COUNT - 1}")
    return rank


def grid_step(shape: tuple[int, int]) -> int:
    """The pixels between two points of the grid of a frame of ``shape``."""
    return max(1, -(-max(shape) // GRID_SIDE))


def coverage(matrices: list[tuple[float, ...] | None], shape: tuple[int, int], step: int) -> np.ndarray:
    """How many frames cover each point of the grid (one point every ``step`` pixels, at the block's centre)."""
    height, width = shape
    rows = np.arange(0, height, step) + (min(step, height) - 1) / 2
    columns = np.arange(0, width, step) + (min(step, width) - 1) / 2
    row, column = np.meshgrid(rows, columns, indexing="ij")
    points = np.stack([column.ravel() + 0.5, height - row.ravel() - 0.5, np.ones(row.size)])
    count = np.zeros(row.size, np.int32)
    for matrix in matrices:
        if matrix is None:
            continue
        inverse = np.linalg.inv(np.array(matrix, float).reshape(3, 3))
        x, y, w = inverse @ points
        x, y = x / w, y / w
        count += (x >= 0) & (x < width) & (y >= 0) & (y < height)
    return count.reshape(row.shape)


def notches(count: np.ndarray, shape: tuple[int, int], step: int, frames: int) -> list[dict[str, object]]:
    """The stops of the slider, the common area first and the whole field last; empty without a common area.

    ``frames``: the frames registered.  Each stop: ``rank``, ``top``, ``left``, ``height``, ``width`` (pixels of the
    master), ``shareOfFrame`` (of the whole field), ``fullShare`` (the share of it every frame covers) and
    ``lowestFrames`` (the frames at its lowest point).
    """
    height, width = shape
    measure = plan_framing.Measure(count, np.zeros_like(count))
    core = plan_combine.largest_rectangle(count >= frames)
    if step > 1:
        # A point of the grid stands for a block: one block less on each side keeps every pixel inside.
        core = (core[0] + 1, core[1] + 1, core[2] - 2, core[3] - 2)
    if frames < 1 or core[2] <= 0 or core[3] <= 0:
        return []
    stops: list[dict[str, object]] = []
    for rank in range(NOTCH_COUNT):
        t = rank / (NOTCH_COUNT - 1)
        cells = plan_framing.rectangle_at(measure, core, t)
        top, left = cells["top"] * step, cells["left"] * step  # type: ignore[operator]
        if rank == NOTCH_COUNT - 1:
            top, left, bottom, right = 0, 0, height, width
        else:
            bottom = min(height, (cells["top"] + cells["height"]) * step)  # type: ignore[operator]
            right = min(width, (cells["left"] + cells["width"]) * step)  # type: ignore[operator]
        stops.append({
            "rank": rank, "top": int(top), "left": int(left), "height": int(bottom - top), "width": int(right - left),
            "shareOfFrame": round((bottom - top) * (right - left) / (height * width), 4),
            "fullShare": round(1.0 - float(cells["degradedShare"]) - float(cells["emptyShare"]), 4),  # type: ignore[arg-type]
            "lowestFrames": int(cells["lowestSeconds"]),  # type: ignore[arg-type]
        })
    return stops


def frame_report(matrices: list[tuple[float, ...] | None] | None, shape: tuple[int, int] | None, progressive: bool,
                 requested: str | int = AUTO) -> dict[str, object]:
    """The ``frame`` block of the ``registration`` event and of the provenance.

    * ``frameHeight`` / ``frameWidth``: the size of one frame (``None`` when it could not be read);
    * ``notches``: the stops on offer, only when the field turns (empty: nothing to choose);
    * ``automatic``: the rank Automatic gives (the whole field) when there are notches, else ``None``;
    * ``requested``: what ``--frame-size`` asked (``auto`` or a rank);
    * ``rank``: the notch applied, ``None`` when nothing is cut (Automatic, or nothing to choose).
    """
    stops: list[dict[str, object]] = []
    if progressive and matrices and shape:
        step = grid_step(shape)
        stops = notches(coverage(matrices, shape, step), shape, step, sum(m is not None for m in matrices))
    automatic = NOTCH_COUNT - 1 if stops else None
    rank = requested if isinstance(requested, int) and stops and requested != automatic else None
    return {
        "frameHeight": shape[0] if shape else None,
        "frameWidth": shape[1] if shape else None,
        "notches": stops,
        "automatic": automatic,
        "requested": requested,
        "rank": rank,
    }


def chosen_rectangle(report: dict[str, object]) -> dict[str, int] | None:
    """The rectangle to cut the master to, or ``None`` when the whole master is kept."""
    rank = report.get("rank")
    if not isinstance(rank, int):
        return None
    stop = report["notches"][rank]  # type: ignore[index]
    return {key: int(stop[key]) for key in ("top", "left", "height", "width")}


def crop_master(source: Path, destination: Path, rectangle: dict[str, int], renormalise: bool) -> None:
    """The ``rectangle`` of the linear master ``source`` (rows in the order ``astropy`` reads them), to ``destination``.

    ``renormalise``: the stacking was brought to 0..1 (Siril's ``-output_norm``); the crop is brought back to it,
    over all its planes, as a stacking of that rectangle would be.  ``source`` and ``destination`` may be the same.
    """
    from astropy.io import fits

    with fits.open(source, memmap=False) as hdul:
        header = hdul[0].header.copy()
        data = np.asarray(hdul[0].data, np.float32)
    top, left = rectangle["top"], rectangle["left"]
    cut = np.ascontiguousarray(data[..., top:top + rectangle["height"], left:left + rectangle["width"]])
    if renormalise:
        low, high = float(cut.min()), float(cut.max())
        if math.isfinite(low) and high > low:
            cut = ((cut - low) / (high - low)).astype(np.float32)
    header["ASFRAME"] = (f"{top},{left},{rectangle['height']},{rectangle['width']}",
                         "Astro Sync: frame cut (top,left,h,w)")
    partial = destination.with_name(f".{destination.name}.part")
    fits.PrimaryHDU(cut, header).writeto(partial, overwrite=True)
    partial.replace(destination)


def frame_shape(frame: Path) -> tuple[int, int] | None:
    """``(height, width)`` of a FITS frame, from its header; ``None`` when it cannot be read."""
    from astropy.io import fits

    try:
        header = fits.getheader(frame, memmap=False)
        return int(header["NAXIS2"]), int(header["NAXIS1"])
    except (OSError, KeyError, ValueError):
        return None
