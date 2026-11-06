"""The adjustable frame of the image of a multi-night plan.

When the nights of a plan do not all cover the same corners (alt-azimuth nights turn the
field), the image of the plan is made **whole** and the person may keep a smaller frame, down
to the rectangle every integration covers.  This module measures the frames on offer and
cuts the one chosen; it is the single place both happen (the on-screen numbers are the ones
written here, the screen recomputes nothing).

**The notches.**  A slider with five stops, from the *cropped* image (the largest rectangle
every integration covers: the same exposure everywhere, no degraded pixel) to the *whole*
image (the default and the last stop).  The four edges go in a straight line from the first to
the last, so the rectangles are nested: the outline on the preview grows without jumping, and
a notch is four integers.  Between them sit the *mark* (the largest frame with at most
``RECOMMENDED_DEGRADED_SHARE`` of its pixels below the full exposure; 5 % by default) and
two stops evenly spaced from the mark to the whole image.  When the whole image is already
inside the mark, there are two stops only.

**What a notch says.**  Its size and share of the whole image, the share of the notch that is
*degraded* (below the full exposure time: the sum of the integrations that cover the best
covered pixel), the lowest exposure found in it and the dual-band exposure there (0: no Hα, no
OIII).

**Cropping comes after the stacking and the render**: ``crop_image`` cuts the rendered image;
nothing is stacked again, and the whole image stays in the plan's ``stacked/`` folder.

Only numpy, tifffile and Pillow are needed (the stacking environment).
"""

from __future__ import annotations

from pathlib import Path

import numpy as np

from software_tag import software

# The mark: the largest frame with at most this share of its pixels below the full exposure.
# One place; 5 % is the accepted loss.
RECOMMENDED_DEGRADED_SHARE = 0.05

# Stops of the slider between the mark and the whole image (so five in all: cropped, mark, 2, whole).
NOTCHES_BETWEEN = 2

# Below this share of the whole image being degraded, the screen offers no choice (« nothing shown under 1 % »).
OFFERED_FROM = 0.01

CROPPED, MARK, BETWEEN, WHOLE = "cropped", "mark", "between", "whole"


class Measure:
    """Measures a rectangle on the exposure maps, with integral images so each is a few lookups.

    ``seconds``: per pixel, the exposure time (the sum of the integrations that cover it);
    ``dual``: the dual-band part of it.
    """

    def __init__(self, seconds: np.ndarray, dual: np.ndarray):
        self.seconds, self.dual = seconds, dual
        self.rows, self.columns = seconds.shape
        self.full = int(seconds.max()) if seconds.size else 0
        self.degraded = (seconds < self.full) & (seconds > 0)
        self.empty = seconds == 0
        self.integral_degraded = self.integral(self.degraded)
        self.integral_empty = self.integral(self.empty)

    @staticmethod
    def integral(mask: np.ndarray) -> np.ndarray:
        table = np.zeros((mask.shape[0] + 1, mask.shape[1] + 1), np.int32)
        table[1:, 1:] = mask.astype(np.int32).cumsum(0, dtype=np.int32).cumsum(1, dtype=np.int32)
        return table

    @staticmethod
    def count(table: np.ndarray, top: int, left: int, bottom: int, right: int) -> int:
        return int(table[bottom, right]) - int(table[top, right]) - int(table[bottom, left]) + int(table[top, left])

    def whole_degraded_share(self) -> float:
        """The share of the whole image below the full exposure."""
        return float(self.degraded.mean()) if self.degraded.size else 0.0

    def rectangle(self, top: int, left: int, height: int, width: int) -> dict[str, object]:
        """What a rectangle holds: its place and size, its degraded share, its lowest exposure."""
        bottom, right = top + height, left + width
        area = height * width
        window = (slice(top, bottom), slice(left, right))
        inside = self.seconds[window]
        lowest = int(inside.min())
        # The dual-band exposure at the lowest-exposure pixels (the worst place of the rectangle).
        worst_dual = int(self.dual[window][inside == lowest].min())
        return {
            "top": top, "left": left, "height": height, "width": width,
            "shareOfFrame": round(area / (self.rows * self.columns), 4),
            "degradedShare": round(self.count(self.integral_degraded, top, left, bottom, right) / area, 4),
            "emptyShare": round(self.count(self.integral_empty, top, left, bottom, right) / area, 4),
            "lowestSeconds": lowest,
            "lowestDualBandSeconds": worst_dual,
        }


def rectangle_at(measure: Measure, core: tuple[int, int, int, int], t: float) -> dict[str, object]:
    """The rectangle whose four edges are a fraction ``t`` of the way from ``core`` (0) to the whole image (1)."""
    top, left, height, width = core
    bottom, right = top + height, left + width
    t_top, t_left = round(top * (1 - t)), round(left * (1 - t))
    t_bottom = round(bottom + (measure.rows - bottom) * t)
    t_right = round(right + (measure.columns - right) * t)
    return measure.rectangle(t_top, t_left, t_bottom - t_top, t_right - t_left)


def recommended_t(measure: Measure, core: tuple[int, int, int, int],
                  budget: float = RECOMMENDED_DEGRADED_SHARE) -> float:
    """The largest ``t`` (steps of 1/100) whose rectangle has at most ``budget`` of its pixels degraded."""
    for step in range(100, -1, -1):
        if rectangle_at(measure, core, step / 100)["degradedShare"] <= budget:  # type: ignore[operator]
            return step / 100
    return 0.0


def notches(measure: Measure, core: tuple[int, int, int, int]) -> list[dict[str, object]]:
    """The stops of the slider, cropped first and whole last, each with what it says.

    ``kind`` is ``cropped`` (the first), ``whole`` (the last), ``between`` (the stops between the
    mark and the whole image) or ``mark`` (the recommended one, which is also the first when the
    mark is the cropped image and the last when the whole image is inside the mark);
    ``recommended`` says which stop is the mark; ``default`` the whole image.
    Empty when the rectangle every integration covers does not exist (nothing to choose).
    """
    if core[2] == 0 or core[3] == 0:
        return []
    best = recommended_t(measure, core)
    ts = [0.0, best] if best > 0 else [0.0]
    if best < 1:
        ts += [best + (1 - best) * k / (NOTCHES_BETWEEN + 1) for k in range(1, NOTCHES_BETWEEN + 2)]
    # The last stop is the whole image exactly, whatever the rounding of the edges.
    ts[-1] = 1.0 if ts[-1] > 1 - 1e-9 else ts[-1]
    stops: list[dict[str, object]] = []
    for position, t in enumerate(ts):
        item = rectangle_at(measure, core, t)
        item["t"] = round(t, 4)
        item["recommended"] = abs(t - best) < 1e-9
        if position == len(ts) - 1:
            item["kind"] = WHOLE
        elif position == 0:
            item["kind"] = CROPPED
        else:
            item["kind"] = MARK if item["recommended"] else BETWEEN
        item["default"] = position == len(ts) - 1
        stops.append(item)
    return stops


def footprint_polygon(mask: np.ndarray, tolerance: float | None = None) -> list[list[int]] | None:
    """The outline of a convex area of ``mask``, as a few ``[x, y]`` corners of the image (pixels), or ``None``.

    The area every integration covers is the intersection of the footprints of the integrations, each a rotated
    rectangle cut by the frame, so it is convex: its convex hull is its outline, simplified (Douglas-Peucker) to the
    few corners a drawing needs.  ``tolerance``: pixels (default: 0.3 % of the longest side).
    """
    from scipy.spatial import ConvexHull, QhullError

    step = max(1, max(mask.shape) // 1200)
    ys, xs = np.nonzero(mask[::step, ::step])
    if len(ys) < 3:
        return None
    points = np.column_stack([xs * step, ys * step]).astype(float)
    try:
        hull = points[ConvexHull(points).vertices]
    except QhullError:
        return None
    tolerance = 0.003 * max(mask.shape) if tolerance is None else tolerance
    # The closed polygon is cut at its two farthest points and each half simplified.
    far = max(range(len(hull)), key=lambda i: float(np.linalg.norm(hull[i] - hull[0])))
    kept: list[np.ndarray] = []
    for half in (hull[: far + 1], np.vstack([hull[far:], hull[:1]])):
        kept.extend(_simplify(half, tolerance)[:-1])
    return [[int(round(x)), int(round(y))] for x, y in kept]


def _simplify(line: np.ndarray, tolerance: float) -> list[np.ndarray]:
    """Douglas-Peucker: the points of ``line`` that stay, the ends included."""
    start, end = line[0], line[-1]
    direction = end - start
    length = float(np.linalg.norm(direction))
    if len(line) < 3 or length == 0:
        return [start, end]
    offsets = np.abs(direction[0] * (line[:, 1] - start[1]) - direction[1] * (line[:, 0] - start[0])) / length
    index = int(offsets.argmax())
    if offsets[index] <= tolerance:
        return [start, end]
    return _simplify(line[: index + 1], tolerance)[:-1] + _simplify(line[index:], tolerance)


def framing_report(seconds: np.ndarray, dual: np.ndarray, core: tuple[int, int, int, int],
                   full: np.ndarray | None = None) -> dict[str, object]:
    """What ``plan_combine.coverage_report`` adds for the choice of the frame.

    * ``degradedShare``: the share of the whole image below the full exposure (the block is offered from
      ``OFFERED_FROM``);
    * ``fullSeconds`` / ``fullDualBandSeconds``: the exposure of the best covered pixels and its dual-band
      part;
    * ``recommendedDegradedShare``: the budget of the mark;
    * ``fullPolygon``: the corners of the area every integration covers (``full``: its mask), what the screen
      leaves unhatched on the preview; only with ``framings``;
    * ``framings``: the notches (empty when there is nothing to choose: no degraded pixel, or no rectangle
      every integration covers).
    """
    measure = Measure(seconds, dual)
    degraded = round(measure.whole_degraded_share(), 4)
    stops = notches(measure, core) if measure.degraded.any() else []
    full_dual = int(dual[seconds == measure.full].max()) if measure.full > 0 else 0
    return {
        "degradedShare": degraded,
        "fullSeconds": measure.full,
        "fullDualBandSeconds": full_dual,
        "recommendedDegradedShare": RECOMMENDED_DEGRADED_SHARE,
        "framings": stops,
        **({"fullPolygon": footprint_polygon(full)} if stops and full is not None else {}),
    }


def is_whole(frame: dict[str, int], height: int, width: int) -> bool:
    """Whether ``frame`` is the whole image."""
    return (frame["top"], frame["left"], frame["height"], frame["width"]) == (0, 0, height, width)


def valid_frame(frame: object, height: int, width: int) -> dict[str, int] | None:
    """``frame`` (the four integers of a notch) as a dict when it is a non-empty rectangle inside the image, else ``None``."""
    if not isinstance(frame, dict):
        return None
    try:
        top, left, rows, columns = (int(frame[key]) for key in ("top", "left", "height", "width"))
    except (KeyError, TypeError, ValueError):
        return None
    if top < 0 or left < 0 or rows <= 0 or columns <= 0 or top + rows > height or left + columns > width:
        return None
    return {"top": top, "left": left, "height": rows, "width": columns}


def crop_image(source: Path, destination: Path, frame: dict[str, int]) -> None:
    """The rectangle ``frame`` of the rendered image ``source`` (TIFF or PNG), written to ``destination`` the same way.

    The pixels are copied, not resampled: a 16-bit TIFF stays a 16-bit TIFF, losslessly compressed.
    """
    rows = slice(frame["top"], frame["top"] + frame["height"])
    columns = slice(frame["left"], frame["left"] + frame["width"])
    suffix = source.suffix.lower()
    if suffix in (".tif", ".tiff"):
        import tifffile

        image = tifffile.imread(source)
        cut = np.ascontiguousarray(image[rows, columns])
        tifffile.imwrite(destination, cut, photometric="rgb" if cut.ndim == 3 else "minisblack", compression="deflate",
                         software=software())
    elif suffix == ".png":
        from PIL import Image

        with Image.open(source) as image:
            image.crop((frame["left"], frame["top"], frame["left"] + frame["width"], frame["top"] + frame["height"])).save(
                destination, optimize=True)
    else:
        raise ValueError(f"cannot crop a {suffix} file")


def image_size(source: Path) -> tuple[int, int]:
    """``(height, width)`` of a rendered TIFF or PNG, read from its header."""
    if source.suffix.lower() in (".tif", ".tiff"):
        import tifffile

        with tifffile.TiffFile(source) as handle:
            shape = handle.pages[0].shape
        return int(shape[0]), int(shape[1])
    from PIL import Image

    with Image.open(source) as image:
        return image.height, image.width


def write_preview(source: Path, destination: Path, longest_side: int = 480) -> None:
    """A small PNG of a rendered image for the screen: its longest side ``longest_side`` pixels at most.

    A render already stretched is shown as it is; an image that is linear (its median under 2 % of the
    range, "No processing") is stretched for the eye (a square-root curve on the 0.1..99.9 percentiles)
    -- the preview says where the frame is, it is never an output.
    """
    import tifffile
    from PIL import Image

    image = tifffile.imread(source) if source.suffix.lower() in (".tif", ".tiff") else np.asarray(Image.open(source))
    height, width = image.shape[:2]
    step = max(1, -(-max(height, width) // (longest_side * 2)))
    small = image[::step, ::step].astype(np.float32)
    top = 65535.0 if image.dtype == np.uint16 else 255.0
    small /= top
    if float(np.median(small)) < 0.02:
        low, high = np.percentile(small, [0.1, 99.9])
        small = np.sqrt(np.clip((small - low) / max(float(high - low), 1e-6), 0.0, 1.0))
    picture = Image.fromarray(np.round(np.clip(small, 0, 1) * 255).astype(np.uint8))
    picture.thumbnail((longest_side, longest_side), Image.Resampling.LANCZOS)
    destination.parent.mkdir(parents=True, exist_ok=True)
    picture.save(destination, optimize=True)


JPEG_QUALITY = 85
# The longest side of the picture of a progression: a display size (a retina screen shows it whole),
# never the 3 000 px of the image, whose JPEG weighs 2.5 to 3.5 MB and makes the player wait.
JPEG_LONG_SIDE = 2048


def write_jpeg(source: Path, destination: Path, quality: int = JPEG_QUALITY, long_side: int = JPEG_LONG_SIDE) -> None:
    """A JPEG of a rendered image, brought to a display size (a TIFF is brought to 8 bits).

    The picture of a progression: a few hundred kilobytes where the PNG of the
    same image weighs fifteen megabytes, and where the JPEG at full size weighs three.  The longest side
    is ``long_side`` pixels at most (never enlarged); the image is the render as it is, never stretched again.
    """
    import tifffile
    from PIL import Image

    if source.suffix.lower() in (".tif", ".tiff"):
        image = tifffile.imread(source)
        if image.dtype == np.uint16:
            image = np.round(image.astype(np.float32) / 257.0).astype(np.uint8)
    else:
        image = np.asarray(Image.open(source).convert("RGB"))
    picture = Image.fromarray(image)
    if picture.mode not in ("RGB", "L"):
        picture = picture.convert("RGB")
    picture.thumbnail((long_side, long_side), Image.Resampling.LANCZOS)
    destination.parent.mkdir(parents=True, exist_ok=True)
    partial = destination.with_name(f".{destination.name}.part")
    picture.save(partial, format="JPEG", quality=quality, optimize=True)
    partial.replace(destination)
