"""The image of a multi-night plan cut to a clean axis-aligned frame.

The masters of a plan are brought onto one frame (``plan_stack.py``): a night taken with the camera turned, or a
master a little smaller or shifted, leaves its edges in the image as bevels -- black where no night has data.
The image is cut to an axis-aligned rectangle, chosen by the job's ``planField``:

* ``clean`` (« Bords nets », the default): the largest rectangle **without a pixel no master covers** (the union of
  the footprints): the black bevels go, the parts fewer nights cover stay (97 % of the field on a real plan of three
  nights, one of them turned by 39°);
* ``common`` (« Zone commune à toutes les nuits »): the largest rectangle inside the area **every** master retained
  covers (the intersection of their footprints, the rotations included): one exposure everywhere, a smaller field
  (50 % on the same plan); kept whole when that area is under ``MIN_KEPT_SHARE`` of the frame;
* ``whole`` (« Tout le champ »): nothing is cut, the union as it is.

The crop is made on the **linear** image, before the render: every render of the plan (``raw``,
``dualband-correction``, ``hoo``, ``polished``, their presentations and finishes, "No processing") is cut by
the same rectangle, since the masters give the same rectangle to every pipeline.  A milestone of the
progression is given the rectangle of the image of the plan (``crop`` of the job) once it is on the same
frame, so the milestones and the image stay aligned.  When the linear master carries a WCS, its ``CRPIX`` moves
with the crop; the sky solution of an image sent is made on that image anyway (``sky_solution.rs``).

The rectangle is ``plan_combine.largest_rectangle``; nothing here resamples a pixel.

Only numpy and scipy are needed (the stacking environment).
"""

from __future__ import annotations

import numpy as np

import plan_combine

#: Below this share of the frame the common area is no image worth keeping (two alt-azimuth nights turned far
#: apart): the whole field is kept, and the record says why (the ``common`` mode only).
MIN_KEPT_SHARE = 0.25

# What the job asks (``planField``).
CLEAN, COMMON, WHOLE = "clean", "common", "whole"
MODES = (CLEAN, COMMON, WHOLE)

# Why the image was cut, or not (``reason`` of the record and of the ``crop`` event).
NO_EMPTY_PIXEL = "noEmptyPixel"  # cut to the largest rectangle without a pixel no master covers
COMMON_AREA = "commonArea"  # cut to the area every master covers
PLAN_RECTANGLE = "plan"  # a milestone of the progression, cut to the rectangle of the image of the plan
WHOLE_FIELD = "wholeField"  # the person keeps the whole field
ALREADY_WHOLE = "alreadyWhole"  # nothing to cut (every pixel has data, or every master covers the whole frame)
NO_COMMON_AREA = "noCommonArea"  # no pixel every master covers (or none any covers)
TOO_SMALL = "commonAreaTooSmall"  # the common area is under MIN_KEPT_SHARE of the frame


def mode_of(job: dict[str, object]) -> str:
    """The ``planField`` of a job, ``clean`` when it says none; :class:`ValueError` for another word."""
    mode = str(job.get("planField") or CLEAN)
    if mode not in MODES:
        raise ValueError(f"the plan field '{mode}' is not one of {', '.join(MODES)}")
    return mode


def common_area(covers: list[np.ndarray]) -> np.ndarray | None:
    """The pixels every mask of ``covers`` holds (all of the same shape); ``None`` without a mask."""
    if not covers:
        return None
    every = np.ones(covers[0].shape, bool)
    for cover in covers:
        every &= cover
    return every


def inscribed_rectangle(covers: list[np.ndarray]) -> tuple[int, int, int, int]:
    """The largest axis-aligned rectangle every mask of ``covers`` holds: ``(top, left, height, width)``.

    ``(0, 0, 0, 0)`` when there is no mask or no pixel common to all of them.
    """
    every = common_area(covers)
    if every is None or not every.any():
        return 0, 0, 0, 0
    return plan_combine.largest_rectangle(every)


def record_of(rectangle: tuple[int, int, int, int], shape: tuple[int, int], applied: bool, reason: str,
              common_share: float | None = None, mode: str = CLEAN) -> dict[str, object]:
    """What the record and the ``crop`` event say of a crop: the mode asked, the rectangle kept (the whole frame
    when nothing is cut), the frame it was cut from, the pixels lost on each edge and the share of the frame kept."""
    rows, columns = shape
    top, left, height, width = rectangle if applied else (0, 0, rows, columns)
    entry: dict[str, object] = {
        "mode": mode,
        "applied": applied,
        "reason": reason,
        "top": int(top), "left": int(left), "height": int(height), "width": int(width),
        "frameHeight": int(rows), "frameWidth": int(columns),
        "lost": {"top": int(top), "left": int(left), "bottom": int(rows - top - height),
                 "right": int(columns - left - width)},
        "keptShare": round(height * width / (rows * columns), 4) if rows and columns else 0.0,
    }
    if common_share is not None:
        # The largest rectangle inside the common area, as a share of the frame: what the crop would keep.
        entry["commonShare"] = round(common_share, 4)
    return entry


def decide(covers: list[np.ndarray], covered: np.ndarray | None = None, mode: str = CLEAN,
           given: object = None) -> dict[str, object]:
    """The crop of an image whose masters cover ``covers`` (one mask per master retained, on the image's frame).

    ``covered``: the pixels with data (the union of ``covers`` when not given), what ``clean`` keeps.
    ``given``: the rectangle of the image of the plan (``crop`` of a milestone's job: ``top``, ``left``,
    ``height``, ``width``, ``frameHeight``, ``frameWidth``); taken as it is when it was cut on a frame of the
    same size, else the milestone measures its own.  ``whole``: nothing is cut.
    """
    shape = covers[0].shape if covers else (covered.shape if covered is not None else (0, 0))
    whole_frame = (0, 0, *shape)
    if mode == WHOLE:
        return record_of(whole_frame, shape, False, WHOLE_FIELD, mode=mode)
    rectangle = given_rectangle(given, shape)
    if rectangle is not None:
        whole = rectangle == whole_frame
        return record_of(rectangle, shape, not whole, ALREADY_WHOLE if whole else PLAN_RECTANGLE, mode=mode)
    if mode == CLEAN:
        data = covered if covered is not None else plan_combine_union(covers, shape)
        if data is None or not data.any():
            return record_of(whole_frame, shape, False, NO_COMMON_AREA, 0.0, mode=mode)
        rectangle = plan_combine.largest_rectangle(data)
        if rectangle == whole_frame:
            return record_of(whole_frame, shape, False, ALREADY_WHOLE, mode=mode)
        return record_of(rectangle, shape, True, NO_EMPTY_PIXEL, mode=mode)
    top, left, height, width = inscribed_rectangle(covers)
    if height == 0 or width == 0:
        return record_of(whole_frame, shape, False, NO_COMMON_AREA, 0.0, mode=mode)
    share = height * width / (shape[0] * shape[1])
    if (top, left, height, width) == whole_frame:
        return record_of(whole_frame, shape, False, ALREADY_WHOLE, mode=mode)
    if share < MIN_KEPT_SHARE:
        return record_of(whole_frame, shape, False, TOO_SMALL, share, mode=mode)
    return record_of((top, left, height, width), shape, True, COMMON_AREA, mode=mode)


def plan_combine_union(covers: list[np.ndarray], shape: tuple[int, int]) -> np.ndarray | None:
    """The pixels at least one mask of ``covers`` holds; ``None`` without a mask."""
    if not covers:
        return None
    union = np.zeros(shape, bool)
    for cover in covers:
        union |= cover
    return union


def given_rectangle(given: object, shape: tuple[int, int]) -> tuple[int, int, int, int] | None:
    """The rectangle of ``given`` when it was measured on a frame of ``shape`` and fits in it, else ``None``."""
    if not isinstance(given, dict):
        return None
    try:
        top, left, height, width, rows, columns = (int(given[key]) for key in
                                                    ("top", "left", "height", "width", "frameHeight", "frameWidth"))
    except (KeyError, TypeError, ValueError):
        return None
    if (rows, columns) != tuple(shape) or height <= 0 or width <= 0 or top < 0 or left < 0:
        return None
    if top + height > rows or left + width > columns:
        return None
    return top, left, height, width


def window(crop: dict[str, object]) -> tuple[slice, slice]:
    """The rows and the columns a crop keeps."""
    top, left = int(crop["top"]), int(crop["left"])
    return slice(top, top + int(crop["height"])), slice(left, left + int(crop["width"]))


def cut(array: np.ndarray, crop: dict[str, object]) -> np.ndarray:
    """``array`` (``(h, w)``, ``(h, w, c)`` or ``(c, h, w)`` with ``c`` ≤ 4) cut to the rectangle of ``crop``.

    ``(c, h, w)`` is told from ``(h, w, c)`` by the frame's size the crop was measured on.
    """
    if not crop["applied"]:
        return array
    rows, columns = window(crop)
    frame = (int(crop["frameHeight"]), int(crop["frameWidth"]))
    if array.ndim == 3 and tuple(array.shape[1:]) == frame:
        return np.ascontiguousarray(array[:, rows, columns])
    return np.ascontiguousarray(array[rows, columns])


def shift_wcs(header: object, crop: dict[str, object]) -> bool:
    """The reference pixel of a WCS moved by the crop (``CRPIX1 -= left``, ``CRPIX2 -= top``: the rows of the
    array are the FITS rows, whatever way they are shown).  ``False`` when the header has no WCS."""
    if not crop["applied"] or "CRPIX1" not in header or "CRPIX2" not in header:  # type: ignore[operator]
        return False
    header["CRPIX1"] = float(header["CRPIX1"]) - int(crop["left"])  # type: ignore[index]
    header["CRPIX2"] = float(header["CRPIX2"]) - int(crop["top"])  # type: ignore[index]
    return True
