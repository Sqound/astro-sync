"""Tests of the master a person hands Astro Sync to measure.

Run from this folder, with the stacking environment:
    python3 -m unittest -v test_master_quality

What is covered: the XISF reader (``xisf_reader.py``: uncompressed, zlib and
LZ4, with and without byte shuffling, planar and normal storage, both byte
orders, attached, inline and embedded blocks, and the refusals), the same
measure on the same sky read from a FITS and from an XISF, the refusal of an
image that is not linear (an autostretch, a histogram transformation, an 8-bit
image, a JPEG), and the JSON entry the window uses (``--measure``).
"""

from __future__ import annotations

import base64
import io
import json
import shutil
import struct
import tempfile
import unittest
import zlib
from contextlib import redirect_stdout
from pathlib import Path

try:
    import numpy as np
    from astropy.io import fits

    import star_quality
    import xisf_reader
    from test_star_quality import star_field
except ImportError:  # the stacking environment has them; a bare interpreter does not
    np = fits = star_quality = xisf_reader = star_field = None

NEEDS_NUMPY = unittest.skipIf(np is None, "needs numpy, scipy and astropy (the stacking environment)")

SAMPLES = {"UInt8": "u1", "UInt16": "u2", "UInt32": "u4", "Float32": "f4", "Float64": "f8"}


def write_xisf(path: Path, data: "np.ndarray", sample: str = "Float32", *, compression: str | None = None,
               shuffle: bool = False, storage: str = "Planar", location: str = "attachment",
               order: str = "little") -> None:
    """A monolithic XISF of ``data`` (``H x W`` or ``C x H x W``), written as the options say."""
    channels, (height, width) = (1, data.shape) if data.ndim == 2 else (data.shape[0], data.shape[1:])
    marker = ">" if order == "big" else "<"
    dtype = np.dtype(marker + SAMPLES[sample])
    planes = data.reshape(1, height, width) if data.ndim == 2 else data
    if storage == "Normal":
        raw = np.ascontiguousarray(planes.transpose(1, 2, 0)).astype(dtype).tobytes()
    else:
        raw = np.ascontiguousarray(planes).astype(dtype).tobytes()
    block = raw
    attributes = ""
    if shuffle:
        count = len(raw) // dtype.itemsize
        block = np.frombuffer(raw, np.uint8).reshape(count, dtype.itemsize).T.reshape(-1).tobytes()
    if compression == "zlib":
        block = zlib.compress(block)
    elif compression in ("lz4", "lz4hc"):
        import imagecodecs

        block = bytes(imagecodecs.lz4_encode(block, hc=compression == "lz4hc", header=False))
    if compression:
        suffix = "+sh" if shuffle else ""
        item = f":{dtype.itemsize}" if shuffle else ""
        attributes = f' compression="{compression}{suffix}:{len(raw)}{item}"'
    body = ""
    if location == "attachment":
        place = 'location="attachment:{position:010d}:%d"' % len(block)
    elif location == "inline":
        place = 'location="inline:base64"'
        body = base64.b64encode(block).decode()
    else:
        place = 'location="embedded"'
        body = f'<Data encoding="base64">{base64.b64encode(block).decode()}</Data>'
    template = (
        '<?xml version="1.0" encoding="UTF-8"?><xisf version="1.0" xmlns="http://www.pixinsight.com/xisf">'
        f'<Image geometry="{width}:{height}:{channels}" sampleFormat="{sample}" byteOrder="{order}" '
        f'pixelStorage="{storage}" colorSpace="{"Gray" if channels == 1 else "RGB"}" {place}{attributes}>{body}</Image>'
        "</xisf>"
    )
    header_length = len(template.format(position=0).encode())
    position = 16 + header_length
    xml = template.format(position=position).encode()
    with open(path, "wb") as handle:
        handle.write(b"XISF0100" + struct.pack("<I", len(xml)) + b"\0\0\0\0" + xml)
        if location == "attachment":
            handle.write(block)


def linear_master(image: "np.ndarray") -> "np.ndarray":
    """The field as a real linear master has it: the sky within a few hundredths of the range, a bright
    pixel (the saturated core of a star, here in the margin where nothing is measured) at the top."""
    master = image.copy()
    master[0, 0] = 8 * float(master.max())
    return master


def stretched(image: "np.ndarray", target: float = 0.25) -> "np.ndarray":
    """What an automatic screen stretch does: the median of the image moved to ``target`` by a midtones transfer."""
    scaled = (image - image.min()) / (image.max() - image.min())
    median = float(np.median(scaled))
    balance = (median * (target - 1)) / (2 * target * median - target - median)
    clipped = np.clip(scaled, 0, 1)
    return ((balance - 1) * clipped / ((2 * balance - 1) * clipped - balance)).astype(np.float32)


class Folder(unittest.TestCase):
    def setUp(self) -> None:
        self.root = Path(tempfile.mkdtemp())
        self.addCleanup(lambda: shutil.rmtree(self.root, ignore_errors=True))


@NEEDS_NUMPY
class Reader(Folder):
    def setUp(self) -> None:
        super().setUp()
        rng = np.random.default_rng(3)
        self.mono = rng.uniform(0, 1, (37, 53)).astype(np.float32)
        self.colour = rng.uniform(0, 1, (3, 37, 53)).astype(np.float32)

    def read(self, data: "np.ndarray", sample: str = "Float32", **options) -> "np.ndarray":
        path = self.root / "image.xisf"
        write_xisf(path, data, sample, **options)
        return xisf_reader.read_xisf(path)

    def test_an_uncompressed_image_comes_back_pixel_for_pixel(self) -> None:
        np.testing.assert_array_equal(self.read(self.mono), self.mono)
        np.testing.assert_array_equal(self.read(self.colour), self.colour)
        np.testing.assert_array_equal(self.read(self.mono, "Float64"), self.mono.astype(np.float64).astype(np.float32))

    def test_every_sample_format_and_both_byte_orders(self) -> None:
        integers = (self.mono * 60000).astype(np.uint16)
        np.testing.assert_array_equal(self.read(integers, "UInt16"), integers)
        np.testing.assert_array_equal(self.read(integers, "UInt16", order="big"), integers)
        np.testing.assert_array_equal(self.read(self.mono, order="big"), self.mono)
        eight = (self.mono * 255).astype(np.uint8)
        np.testing.assert_array_equal(self.read(eight, "UInt8"), eight)
        wide = (self.mono * 4e9).astype(np.uint32)
        np.testing.assert_array_equal(self.read(wide, "UInt32"), wide)

    def test_normal_storage_is_put_back_in_planes(self) -> None:
        np.testing.assert_array_equal(self.read(self.colour, storage="Normal"), self.colour)

    def test_zlib_and_lz4_blocks_with_and_without_byte_shuffling(self) -> None:
        for compression in ("zlib", "lz4", "lz4hc"):
            for shuffle in (False, True):
                with self.subTest(compression=compression, shuffle=shuffle):
                    np.testing.assert_array_equal(
                        self.read(self.colour, compression=compression, shuffle=shuffle), self.colour)

    def test_inline_and_embedded_blocks(self) -> None:
        for location in ("inline", "embedded"):
            np.testing.assert_array_equal(self.read(self.mono, location=location, compression="zlib"), self.mono)

    def test_a_file_that_is_not_an_xisf_or_that_cannot_be_read_says_why(self) -> None:
        path = self.root / "image.xisf"
        path.write_bytes(b"not an xisf at all, but long enough to read a prefix")
        with self.assertRaises(xisf_reader.XisfError) as caught:
            xisf_reader.read_xisf(path)
        self.assertEqual(caught.exception.code, "not_xisf")
        write_xisf(path, self.mono)
        path.write_bytes(path.read_bytes()[:-100])  # the data block is cut short
        with self.assertRaises(xisf_reader.XisfError) as caught:
            xisf_reader.read_xisf(path)
        self.assertEqual(caught.exception.code, "damaged")

    def test_a_compression_it_does_not_cover_is_named(self) -> None:
        path = self.root / "image.xisf"
        write_xisf(path, self.mono, compression="zlib")
        path.write_bytes(path.read_bytes().replace(b'compression="zlib', b'compression="zstd'))
        with self.assertRaises(xisf_reader.XisfError) as caught:
            xisf_reader.read_xisf(path)
        self.assertEqual(caught.exception.code, "unsupported")
        self.assertIn("zstd", str(caught.exception))

    def test_integers_are_put_on_zero_to_one_by_their_type(self) -> None:
        integers = np.array([[0, 65535], [32768, 65535]], np.uint16)
        np.testing.assert_allclose(xisf_reader.normalise(integers), [[0, 1], [32768 / 65535, 1]])


@NEEDS_NUMPY
class Measure(Folder):
    def setUp(self) -> None:
        super().setUp()
        self.sky = linear_master(star_field(150, 1.6, 1.6))
        self.expected = star_quality.measure(self.sky)

    def fits_of(self, image: "np.ndarray", name: str = "master.fit") -> Path:
        path = self.root / name
        fits.PrimaryHDU(image).writeto(path)
        return path

    def xisf_of(self, image: "np.ndarray", name: str = "master.xisf", **options) -> Path:
        path = self.root / name
        write_xisf(path, image, **options)
        return path

    def test_the_same_sky_gives_the_same_figures_from_a_fits_and_from_an_xisf(self) -> None:
        from_fits = star_quality.measure_file(self.fits_of(self.sky))
        for options in ({}, {"compression": "zlib", "shuffle": True}, {"compression": "lz4"}):
            from_xisf = star_quality.measure_file(self.xisf_of(self.sky, **options))
            for key in ("fwhmPixels", "eccentricity", "starCount", "measuredOn"):
                self.assertEqual(from_xisf[key], from_fits[key], key)
        self.assertEqual(from_fits["fwhmPixels"], self.expected["fwhmPixels"])
        self.assertRegex(from_fits["measuredAt"], r"^\d{4}-\d\d-\d\dT\d\d:\d\d:\d\dZ$")

    def test_a_colour_xisf_is_measured_on_its_luminance(self) -> None:
        colour = np.stack([self.sky, self.sky, self.sky])
        quality = star_quality.measure_file(self.xisf_of(colour, storage="Normal"))
        self.assertEqual(quality["fwhmPixels"], self.expected["fwhmPixels"])

    def test_a_sixteen_bit_master_on_a_bright_sky_is_still_linear(self) -> None:
        # Sky at 12 % above the lowest value and stars at the top: a bright site, not a stretch
        # (the lowest value rises with the sky, the median stays near the bottom of the range).
        bright = np.clip((self.sky - 0.05) * 1.6 + 0.12, 0, 1)
        integers = (bright * 65535).astype(np.uint16)
        self.assertIsNone(star_quality.stretch_reason(star_quality.luminance(integers.astype(np.float32) / 65535)))
        quality = star_quality.measure_file(self.xisf_of(integers, sample="UInt16"))
        self.assertGreaterEqual(quality["starCount"], star_quality.MINIMUM_STARS)

    def refused(self, path: Path) -> "star_quality.MasterError":
        with self.assertRaises(star_quality.MasterError) as caught:
            star_quality.measure_file(path)
        return caught.exception

    def test_an_autostretched_image_is_refused_in_both_formats(self) -> None:
        flat = stretched(self.sky)
        self.assertGreater(float(np.median(flat)), 0.2)  # the fixture is what it claims to be
        for path in (self.fits_of(flat, "stretched.fit"), self.xisf_of(flat, "stretched.xisf")):
            self.assertEqual(self.refused(path).code, "stretched", path.name)
        self.assertIn("range", str(self.refused(self.xisf_of(flat, "again.xisf"))))

    def test_the_threshold_of_a_stretched_image_is_two_percent_of_the_range(self) -> None:
        # A linear master (median at 0.05 % of the range) and one at 1 % are
        # measured; at 3 % the image is refused for what it is, a stretch, and
        # not for its stars, whether the stars survive (3 %) or not (10 %).
        self.assertEqual(star_quality.STRETCHED_MEDIAN, 0.02)
        for target in (0.0005, 0.01):
            level = stretched(self.sky, target)
            self.assertIsNone(star_quality.stretch_reason(star_quality.luminance(level)), f"{target:.2%}")
            quality = star_quality.measure_file(self.fits_of(level, f"linear-{target}.fit"))
            self.assertEqual(quality["measuredOn"], "linear", f"{target:.2%}")
        for target in (0.03, 0.10, 0.18):
            error = self.refused(self.fits_of(stretched(self.sky, target), f"stretched-{target}.fit"))
            self.assertEqual(error.code, "stretched", f"{target:.0%}")
            self.assertIn("stretched image", str(error))
        # Too few stars is its own word, whatever the sky.
        few = self.refused(self.fits_of(stretched(star_field(5, 1.6, 1.6), 0.001), "few-stars.fit"))
        self.assertEqual(few.code, "too_few_stars")

    def test_a_gently_stretched_colour_image_and_an_eight_bit_image_are_refused_too(self) -> None:
        colour = np.stack([stretched(self.sky, 0.3)] * 3)
        self.assertEqual(self.refused(self.xisf_of(colour)).code, "stretched")
        eight = (stretched(self.sky) * 255).astype(np.uint8)
        self.assertEqual(self.refused(self.fits_of(eight, "eight.fit")).code, "stretched")
        self.assertEqual(self.refused(self.xisf_of(eight, "eight.xisf", sample="UInt8")).code, "stretched")

    def test_a_jpeg_a_tiff_or_a_missing_file_is_not_a_master(self) -> None:
        for name in ("picture.jpg", "picture.tif", "picture.png"):
            (self.root / name).write_bytes(b"\xff\xd8\xff data")
            self.assertEqual(self.refused(self.root / name).code, "unsupported_format", name)
        self.assertEqual(self.refused(self.root / "missing.fit").code, "unreadable")
        self.assertEqual(self.refused(self.root / "missing.xisf").code, "unreadable")

    def test_few_stars_a_blank_sky_and_a_tiny_image_each_have_their_own_word(self) -> None:
        self.assertEqual(self.refused(self.fits_of(linear_master(star_field(5, 1.6, 1.6)), "few.fit")).code, "too_few_stars")
        self.assertEqual(self.refused(self.fits_of(np.full((200, 200), 0.05, np.float32), "blank.fit")).code, "not_an_image")
        self.assertEqual(self.refused(self.fits_of(np.zeros((20, 20), np.float32), "tiny.fit")).code, "not_an_image")
        self.assertEqual(self.refused(self.fits_of(np.zeros((2, 40, 40), np.float32), "two.fit")).code, "not_an_image")


@NEEDS_NUMPY
class Entry(Folder):
    """``star_quality.py --measure FILE``: one JSON line, the figures or a code."""

    def run_entry(self, path: Path) -> dict:
        out = io.StringIO()
        with redirect_stdout(out):
            self.assertEqual(star_quality.main(["--measure", str(path)]), 0)
        return json.loads(out.getvalue())

    def test_a_measured_master_and_a_refused_one(self) -> None:
        sky = linear_master(star_field(150, 1.6, 1.6))
        good = self.root / "good.xisf"
        write_xisf(good, sky)
        answer = self.run_entry(good)
        self.assertEqual(answer["status"], "measured")
        self.assertEqual(sorted(answer["quality"]), ["eccentricity", "fwhmPixels", "measuredAt", "measuredOn", "starCount"])
        bad = self.root / "bad.xisf"
        write_xisf(bad, stretched(sky))
        refused = self.run_entry(bad)
        self.assertEqual((refused["status"], refused["code"]), ("refused", "stretched"))
        self.assertTrue(refused["message"])

    def test_the_old_command_line_still_prints_the_figures(self) -> None:
        path = self.root / "master.fit"
        fits.PrimaryHDU(linear_master(star_field(150, 1.6, 1.6))).writeto(path)
        out = io.StringIO()
        with redirect_stdout(out):
            self.assertEqual(star_quality.main([str(path)]), 0)
        self.assertEqual(json.loads(out.getvalue())["measuredOn"], "linear")


if __name__ == "__main__":
    unittest.main()
