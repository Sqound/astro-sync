"""« Régler l'image »: five settings a person applies to a finished image, never to the linear data.

The settings, in the order they are applied:

``smoothing``   0..100 %: NL-means on the finished image, its strength in widths of the sky's noise (0: none).
``stars``       -100..100 %: the star layer (the image minus StarNet2's starless image) taken away or added back.
``blackPoint``  0..20 %: a black level on the luminance, as a share of the full scale (0: the render's).
``brightness``  -50..50: a power on the luminance, the blacks and the whites kept (0: the render's).
``colour``      0..3: a factor on the chroma of the signal, the sky left neutral (1: the render's).

A setting at its neutral value is not applied at all, so an image with every setting neutral comes out
unchanged to the bit.  Every operation but the smoothing is one per pixel: the window applies them itself on
the preview (``app/src/lib/imageAdjust.ts``, the same formulas), and this script on the whole image when the
person saves.  The masks the colour needs are computed here once per scale (``prepare`` for the preview,
``save`` for the whole image), from the image as the render left it.

Commands, each answering one JSON object on its standard output:

    prepare --image <png> --work <dir>                      the 1080 px preview and its colour mask
    stars   --image <png> --work <dir> --starnet <exe> [--tiff <tif>]   the star layer (StarNet2), whole and previewed
    smooth  --work <dir> --amount <0..100>                  the preview smoothed
    save    --image <png> --work <dir> --values <json> --out <png> [--tiff <tif> --out-tiff <tif>]   the whole image, set

A render the stacker wrote twice (``<name>.png``, 8-bit, and its twin ``<name>.tif``, 16-bit, the same pixels) is
set from its TIFF when ``--tiff`` names it: the star layer is separated from the 16-bit data, the settings are applied
to it once, and both files of the image set are written from that one result -- the TIFF in 16 bits, the PNG as its
8-bit reduction (each level divided by 257 and rounded: exact, never a second computation).  The PNG and the TIFF of
an image set are always the same image.
"""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
import tempfile
import time
from pathlib import Path

import numpy as np
from PIL import Image

Image.MAX_IMAGE_PIXELS = None

PREVIEW_WIDTH = 1080
NEUTRAL = {"smoothing": 0.0, "stars": 0.0, "blackPoint": 0.0, "brightness": 0.0, "colour": 1.0}
RANGES = {
    "smoothing": (0.0, 100.0),
    "stars": (-100.0, 100.0),
    "blackPoint": (0.0, 20.0),
    "brightness": (-50.0, 50.0),
    "colour": (0.0, 3.0),
}
# The strength of the smoothing at 100 %, in widths of the sky's noise (the study found that 1.1 already waxes).
SMOOTHING_FULL = 2.0
# StarNet2 needs at least two tiles of 256 px.
STARNET_MIN_SIDE = 512


class AdjustError(Exception):
    def __init__(self, code: str, message: str) -> None:
        super().__init__(message)
        self.code = code


def clean_values(values: dict) -> dict[str, float]:
    """The five values, each a number in its range; a missing one is neutral."""
    out = {}
    for name, neutral in NEUTRAL.items():
        raw = values.get(name, neutral)
        if not isinstance(raw, (int, float)) or not np.isfinite(raw):
            raise AdjustError("bad_values", f"{name} is not a number")
        low, high = RANGES[name]
        out[name] = float(min(max(raw, low), high))
    return out


def lum(img: np.ndarray) -> np.ndarray:
    return 0.2126 * img[..., 0] + 0.7152 * img[..., 1] + 0.0722 * img[..., 2]


def normalise(out: np.ndarray) -> np.ndarray:
    """Negative channels to 0, a pixel whose brightest channel passes 1 scaled back (its hue kept)."""
    out = np.clip(out, 0, None)
    return np.clip(out / np.maximum(out.max(axis=-1, keepdims=True), 1.0), 0, 1)


def with_luminance(img: np.ndarray, before: np.ndarray, after: np.ndarray) -> np.ndarray:
    return normalise(img * (after / np.maximum(before, 1e-6))[..., None])


def black_point(img: np.ndarray, percent: float) -> np.ndarray:
    if percent <= 0:
        return img
    b = percent / 100.0
    L = lum(img)
    return with_luminance(img, L, np.clip((L - b) / (1 - b), 0, 1))


def brightness(img: np.ndarray, value: float) -> np.ndarray:
    if value == 0:
        return img
    gamma = 2.0 ** (-value / 50.0)
    L = lum(img)
    return with_luminance(img, L, np.clip(L, 0, 1) ** gamma)


def colour(img: np.ndarray, factor: float, weight: np.ndarray) -> np.ndarray:
    if factor == 1:
        return img
    L = lum(img)[..., None]
    return normalise(L + (img - L) * (1 + (factor - 1) * weight[..., None]))


def stars(img: np.ndarray, percent: float, layer: np.ndarray | None) -> np.ndarray:
    if percent == 0:
        return img
    if layer is None:
        raise AdjustError("stars_missing", "the star layer is not there (StarNet2)")
    return np.clip(img + (percent / 100.0) * layer, 0, 1)


def sky_noise(img: np.ndarray) -> float:
    """The width of the grain where the image is darkest (its lower half), never of the nebula."""
    from scipy.ndimage import gaussian_filter

    L = lum(img)
    hp = L - gaussian_filter(L, 2.0)
    dark = hp[L <= np.median(L)]
    if dark.size == 0:
        dark = hp.ravel()
    return float(1.4826 * np.median(np.abs(dark - np.median(dark)))) + 1e-5


def colour_weight(img: np.ndarray) -> np.ndarray:
    """Where the signal is (1) and where the sky is (0): the colour factor leaves the sky neutral."""
    from scipy.ndimage import gaussian_filter

    Ls = gaussian_filter(lum(img), 2.0)
    sky = float(np.percentile(Ls, 10))
    return np.clip((Ls - sky) / (6 * sky_noise(img)), 0, 1).astype(np.float32)


def smoothing(img: np.ndarray, percent: float) -> np.ndarray:
    if percent <= 0:
        return img
    from skimage.restoration import denoise_nl_means

    sigma = sky_noise(img)
    out = denoise_nl_means(img, h=(percent / 100.0) * SMOOTHING_FULL * sigma, sigma=sigma, fast_mode=True,
                           patch_size=5, patch_distance=6, channel_axis=-1)
    return np.clip(out, 0, 1).astype(np.float32)


def apply(img: np.ndarray, values: dict, layer: np.ndarray | None, weight: np.ndarray | None) -> np.ndarray:
    """The five settings on ``img`` (float RGB in 0..1), in order; ``weight`` is computed when not given."""
    v = clean_values(values)
    out = smoothing(img, v["smoothing"])
    out = stars(out, v["stars"], layer)
    out = black_point(out, v["blackPoint"])
    out = brightness(out, v["brightness"])
    if v["colour"] != 1:
        out = colour(out, v["colour"], colour_weight(img) if weight is None else weight)
    return out


def read_image(path: Path) -> tuple[np.ndarray, int]:
    """The image as float RGB in 0..1, and its bits per channel (8 or 16)."""
    with Image.open(path) as opened:
        mode = opened.mode
        if mode in ("I;16", "I;16B", "I"):
            raw = np.asarray(opened).astype(np.float32) / 65535.0
            return np.repeat(raw[..., None], 3, axis=2), 16
        array = np.asarray(opened.convert("RGB"))
    return array.astype(np.float32) / 255.0, 8


def read_tiff(path: Path) -> np.ndarray:
    """A render's TIFF as float RGB in 0..1, at its own depth (16 bits from the stacker)."""
    import tifffile

    array = tifffile.imread(path)
    if not np.issubdtype(array.dtype, np.integer):
        raise AdjustError("bad_tiff", f"{path.name} is not an integer TIFF")
    data = array.astype(np.float32) / float(np.iinfo(array.dtype).max)
    if data.ndim == 2:
        data = np.repeat(data[..., None], 3, axis=2)
    return np.clip(data[..., :3], 0, 1)


def to_bytes(img: np.ndarray, bits: int) -> np.ndarray:
    top = 65535 if bits == 16 else 255
    return np.round(np.clip(img, 0, 1) * top).astype(np.uint16 if bits == 16 else np.uint8)


def write_png(img: np.ndarray, path: Path, bits: int = 8) -> None:
    """Written beside, then renamed: a reader never sees half a file."""
    path.parent.mkdir(parents=True, exist_ok=True)
    fd, temp = tempfile.mkstemp(prefix=".adjust-", suffix=".png", dir=path.parent)
    os.close(fd)
    try:
        data = to_bytes(img, bits)
        if bits == 16:
            import imageio.v3 as iio

            iio.imwrite(temp, data, extension=".png")
        else:
            Image.fromarray(data, "RGB").save(temp, format="PNG", compress_level=6)
        os.replace(temp, path)
    except BaseException:
        Path(temp).unlink(missing_ok=True)
        raise


def write_tiff(levels: np.ndarray, path: Path) -> None:
    """16-bit levels written as the stacker writes a render's TIFF; beside, then renamed."""
    import tifffile

    path.parent.mkdir(parents=True, exist_ok=True)
    fd, temp = tempfile.mkstemp(prefix=".adjust-", suffix=".tif", dir=path.parent)
    os.close(fd)
    try:
        tifffile.imwrite(temp, levels, photometric="rgb", compression="deflate")
        os.replace(temp, path)
    except BaseException:
        Path(temp).unlink(missing_ok=True)
        raise


def eight_bit_of(levels: np.ndarray) -> np.ndarray:
    """The 8-bit reduction of 16-bit levels: each divided by 257 (65535 = 255 x 257) and rounded.  257 is odd, so a
    quotient never falls on a half: the reduction is exact, with no tie to break."""
    return np.round(levels.astype(np.float64) / 257.0).astype(np.uint8)


def shrink(img: np.ndarray, width: int, height: int) -> np.ndarray:
    """Each channel resampled (Lanczos) to ``width`` × ``height``."""
    if img.shape[1] == width and img.shape[0] == height:
        return img.astype(np.float32)
    channels = [np.asarray(Image.fromarray(img[..., k].astype(np.float32), "F").resize((width, height), Image.LANCZOS))
                for k in range(img.shape[2])]
    return np.clip(np.stack(channels, axis=-1), 0, 1).astype(np.float32)


def preview_size(width: int, height: int) -> tuple[int, int]:
    if width <= PREVIEW_WIDTH:
        return width, height
    return PREVIEW_WIDTH, max(1, round(PREVIEW_WIDTH * height / width))


def prepare(image: Path, work: Path) -> dict:
    t0 = time.time()
    img, bits = read_image(image)
    height, width = img.shape[:2]
    pw, ph = preview_size(width, height)
    base = shrink(img, pw, ph)
    # The preview the window adjusts is the 8-bit picture it shows: its masks are computed from that one.
    base = to_bytes(base, 8).astype(np.float32) / 255.0
    write_png(base, work / "base.png")
    weight = colour_weight(base)
    Image.fromarray(np.round(weight * 255).astype(np.uint8), "L").save(work / "mask.png")
    info = {"width": width, "height": height, "bits": bits, "previewWidth": pw, "previewHeight": ph}
    (work / "info.json").write_text(json.dumps(info), encoding="utf-8")
    return {**info, "seconds": round(time.time() - t0, 3)}


def star_layer_of(img: np.ndarray, work: Path, starnet: Path) -> np.ndarray:
    import tifffile

    source = work / "pre_starnet.tif"
    starless_path = work / "starless.tif"
    tifffile.imwrite(source, np.round(np.clip(img, 0, 1) * 65535).astype(np.uint16), photometric="rgb")
    try:
        # The compiled model StarNet2 leaves (~120 MB) is removed as the render's own call does.
        try:
            from renders import starnet_cleanup
        except ImportError:
            from contextlib import nullcontext as starnet_cleanup
        with starnet_cleanup():
            subprocess.run([str(starnet), "-i", str(source), "-o", str(starless_path), "-s", "256"], check=True,
                           stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        try:
            array = tifffile.imread(starless_path)
        except ValueError:
            array = np.asarray(Image.open(starless_path))
    finally:
        source.unlink(missing_ok=True)
        starless_path.unlink(missing_ok=True)
    starless = array.astype(np.float32) / np.iinfo(array.dtype).max
    if starless.ndim == 2:
        starless = np.repeat(starless[..., None], 3, axis=2)
    return np.clip(img - starless[..., :3], 0, 1).astype(np.float32)


def separate_stars(image: Path, work: Path, starnet: Path, tiff: Path | None = None) -> dict:
    import tifffile

    t0 = time.time()
    # With the render's TIFF, StarNet2 reads the 16-bit data the image set is computed from.
    img = read_tiff(tiff) if tiff is not None else read_image(image)[0]
    if min(img.shape[:2]) < STARNET_MIN_SIDE:
        return {"state": "small", "seconds": round(time.time() - t0, 3)}
    try:
        layer = star_layer_of(img, work, starnet)
    except (OSError, subprocess.CalledProcessError) as error:
        raise AdjustError("starnet_failed", f"StarNet2 failed: {error}") from error
    tifffile.imwrite(work / "stars.tif", np.round(layer * 65535).astype(np.uint16), photometric="rgb")
    info = json.loads((work / "info.json").read_text(encoding="utf-8"))
    preview = shrink(layer, info["previewWidth"], info["previewHeight"])
    write_png(preview, work / "stars.png")
    return {"state": "ready", "seconds": round(time.time() - t0, 3)}


def smooth_preview(work: Path, amount: float) -> dict:
    t0 = time.time()
    amount = clean_values({"smoothing": amount})["smoothing"]
    base, _ = read_image(work / "base.png")
    out = work / f"smooth-{round(amount)}.png"
    write_png(smoothing(base, amount), out)
    return {"path": str(out), "seconds": round(time.time() - t0, 3)}


def load_layer(work: Path) -> np.ndarray | None:
    path = work / "stars.tif"
    if not path.is_file():
        return None
    import tifffile

    return tifffile.imread(path).astype(np.float32) / 65535.0


def save(image: Path, work: Path, values: dict, out: Path, tiff: Path | None = None,
         out_tiff: Path | None = None) -> dict:
    """The whole image set.  With ``tiff`` (the render's 16-bit twin) and ``out_tiff``: computed once from the TIFF,
    written as a 16-bit TIFF and as the PNG that is its exact 8-bit reduction."""
    t0 = time.time()
    if (tiff is None) != (out_tiff is None):
        raise AdjustError("bad_arguments", "--tiff and --out-tiff go together")
    if out.resolve() == image.resolve() or (tiff is not None and out_tiff.resolve() in (tiff.resolve(), image.resolve())):
        raise AdjustError("would_overwrite", "the image set never replaces the render it comes from")
    if tiff is not None and out_tiff.resolve() == out.resolve():
        raise AdjustError("bad_arguments", "the PNG and the TIFF of the image set are two files")
    v = clean_values(values)
    img, bits = read_image(image)
    layer = load_layer(work) if v["stars"] != 0 else None
    if tiff is None:
        result = apply(img, v, layer, None)
        write_png(result, out, bits)
        return {"path": str(out), "width": img.shape[1], "height": img.shape[0],
                "seconds": round(time.time() - t0, 3)}
    full = read_tiff(tiff)
    if full.shape != img.shape:
        raise AdjustError("twin_mismatch", f"{tiff.name} is not the same image as {image.name}")
    if layer is not None and layer.shape != full.shape:
        raise AdjustError("stars_missing", "the star layer is not the size of the image")
    levels = to_bytes(apply(full, v, layer, None), 16)
    write_tiff(levels, out_tiff)
    path = out.parent
    path.mkdir(parents=True, exist_ok=True)
    fd, temp = tempfile.mkstemp(prefix=".adjust-", suffix=".png", dir=path)
    os.close(fd)
    try:
        if bits == 16:
            import imageio.v3 as iio

            iio.imwrite(temp, levels, extension=".png")
        else:
            Image.fromarray(eight_bit_of(levels), "RGB").save(temp, format="PNG", compress_level=6)
        os.replace(temp, out)
    except BaseException:
        Path(temp).unlink(missing_ok=True)
        raise
    return {"path": str(out), "tiff": str(out_tiff), "width": img.shape[1], "height": img.shape[0],
            "seconds": round(time.time() - t0, 3)}


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    sub = parser.add_subparsers(dest="command", required=True)
    p = sub.add_parser("prepare")
    p.add_argument("--image", type=Path, required=True)
    p.add_argument("--work", type=Path, required=True)
    s = sub.add_parser("stars")
    s.add_argument("--image", type=Path, required=True)
    s.add_argument("--work", type=Path, required=True)
    s.add_argument("--starnet", type=Path, required=True)
    s.add_argument("--tiff", type=Path)
    m = sub.add_parser("smooth")
    m.add_argument("--work", type=Path, required=True)
    m.add_argument("--amount", type=float, required=True)
    w = sub.add_parser("save")
    w.add_argument("--image", type=Path, required=True)
    w.add_argument("--work", type=Path, required=True)
    w.add_argument("--values", required=True)
    w.add_argument("--out", type=Path, required=True)
    w.add_argument("--tiff", type=Path)
    w.add_argument("--out-tiff", type=Path)
    args = parser.parse_args(argv)
    try:
        if args.command == "prepare":
            args.work.mkdir(parents=True, exist_ok=True)
            answer = prepare(args.image, args.work)
        elif args.command == "stars":
            answer = separate_stars(args.image, args.work, args.starnet, args.tiff)
        elif args.command == "smooth":
            answer = smooth_preview(args.work, args.amount)
        else:
            answer = save(args.image, args.work, json.loads(args.values), args.out, args.tiff, args.out_tiff)
    except AdjustError as error:
        print(json.dumps({"error": error.code, "message": str(error)}))
        return 2
    except (OSError, ValueError) as error:
        print(json.dumps({"error": "adjust_failed", "message": str(error)}))
        return 1
    print(json.dumps(answer))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
