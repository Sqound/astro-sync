#!/usr/bin/env python3
"""The pixels of a monolithic XISF image (PixInsight's format), read for a measure.

Astro Sync's Rust side reads the *header* of an XISF frame (``xisf.rs``) to
detect it; this module reads the **pixels** of the first image of a file so
that a master stacked in PixInsight can be measured as a master stacked by
Astro Sync is (``star_quality.measure_file``).

Covered:

- the monolithic format, ``XISF0100``, one XML header then the data blocks;
- the first ``<Image>``: ``geometry="width:height:channels"``;
- ``sampleFormat`` ``UInt8``, ``UInt16``, ``UInt32``, ``Float32``, ``Float64``
  and ``byteOrder`` ``little`` or ``big`` (``UInt64`` and the complex formats
  are refused);
- ``pixelStorage`` ``Planar`` (the default) and ``Normal``;
- the data block stored ``attachment:position:size``, ``inline:base64`` /
  ``inline:hex``, or ``embedded`` (a ``<Data encoding="base64">`` child);
- the compression ``zlib`` (the standard library), ``lz4`` and ``lz4hc``
  (through ``imagecodecs``, a pinned dependency of the stacking environment),
  each with or without byte shuffling (``zlib+sh``, ``lz4+sh``, ``lz4hc+sh``).

Not covered, said in a clear error (:class:`XisfError`, ``code``
``unsupported``): ``zstd`` compression, the distributed format (``.xish``),
complex or 64-bit integer samples, a file without an image.  The checksum of
a block is not verified (the measure of a damaged image is refused by what it
finds in it, not by its digest).

Returns the image as a ``float32``-compatible array: ``H x W`` for one
channel, ``C x H x W`` for several, in the file's own range (integers are not
rescaled here: see :func:`normalise`).
"""

from __future__ import annotations

import base64
import struct
import xml.etree.ElementTree as ET
import zlib
from pathlib import Path

import numpy as np

SIGNATURE = b"XISF0100"
MAX_HEADER_BYTES = 16 * 1024 * 1024

SAMPLE_FORMATS = {
    "UInt8": "u1",
    "UInt16": "u2",
    "UInt32": "u4",
    "Float32": "f4",
    "Float64": "f8",
}


class XisfError(Exception):
    """A file that cannot be read as an XISF image; ``code`` is ``not_xisf``, ``unsupported`` or ``damaged``."""

    def __init__(self, code: str, message: str) -> None:
        super().__init__(message)
        self.code = code


def local(tag: str) -> str:
    """The name of an XML tag without its namespace."""
    return tag.rsplit("}", 1)[-1]


def read_header(handle) -> ET.Element:
    """The XML header of an XISF file open in binary mode."""
    prefix = handle.read(16)
    if len(prefix) < 16 or prefix[:8] != SIGNATURE:
        raise XisfError("not_xisf", "the file does not start with the XISF signature")
    (length,) = struct.unpack("<I", prefix[8:12])
    if length == 0 or length > MAX_HEADER_BYTES:
        raise XisfError("damaged", "the XISF header has an impossible length")
    text = handle.read(length)
    if len(text) < length:
        raise XisfError("damaged", "the XISF header is cut short")
    try:
        return ET.fromstring(text.rstrip(b"\x00"))
    except ET.ParseError as error:
        raise XisfError("damaged", f"the XISF header is not valid XML: {error}") from error


def parse_geometry(value: str) -> tuple[int, int, int]:
    try:
        parts = [int(part) for part in value.split(":")]
    except ValueError as error:
        raise XisfError("damaged", f"unreadable geometry {value!r}") from error
    if len(parts) != 3 or min(parts) < 1:
        raise XisfError("unsupported", f"geometry {value!r} is not a 2-D image with channels")
    return parts[0], parts[1], parts[2]


def unshuffle(data: bytes, item_size: int) -> bytes:
    """Undoes the byte shuffling of a block (``+sh``): the bytes of one rank of all the items come together."""
    if item_size <= 1:
        return data
    count = len(data) // item_size
    body = np.frombuffer(data, dtype=np.uint8, count=count * item_size)
    restored = body.reshape(item_size, count).T.reshape(-1).tobytes()
    return restored + data[count * item_size:]


def decompress(codec: str, data: bytes, uncompressed: int, item_size: int) -> bytes:
    """``data`` decompressed by ``codec`` (``zlib``, ``lz4``, ``lz4hc``, each possibly ``+sh``)."""
    shuffled = codec.endswith("+sh")
    name = codec[:-3] if shuffled else codec
    if name == "zlib":
        try:
            raw = zlib.decompress(data)
        except zlib.error as error:
            raise XisfError("damaged", f"the zlib block is damaged: {error}") from error
    elif name in ("lz4", "lz4hc"):
        try:
            import imagecodecs
        except ImportError as error:
            raise XisfError("unsupported", "LZ4 compression needs the imagecodecs library") from error
        try:
            raw = bytes(imagecodecs.lz4_decode(data, out=bytearray(uncompressed)))
        except Exception as error:  # noqa: BLE001 - imagecodecs raises its own error classes
            raise XisfError("damaged", f"the LZ4 block is damaged: {error}") from error
    else:
        raise XisfError("unsupported", f"the compression {codec!r} is not supported (zlib and LZ4 are)")
    if len(raw) != uncompressed:
        raise XisfError("damaged", "a compressed block does not have its announced size")
    return unshuffle(raw, item_size) if shuffled else raw


def block_bytes(image: ET.Element, handle) -> bytes:
    """The (decompressed) bytes of the image's data block."""
    location = image.get("location", "")
    kind, _, rest = location.partition(":")
    if kind == "attachment":
        try:
            position, size = (int(part) for part in rest.split(":")[:2])
        except ValueError as error:
            raise XisfError("damaged", f"unreadable location {location!r}") from error
        handle.seek(position)
        data = handle.read(size)
        if len(data) < size:
            raise XisfError("damaged", "the data block is cut short")
    elif kind == "inline":
        encoding = rest.split(":")[0]
        text = image.text or ""
        data = bytes.fromhex(text.strip()) if encoding == "hex" else base64.b64decode(text)
    elif kind == "embedded":
        child = next((c for c in image if local(c.tag) == "Data"), None)
        if child is None:
            raise XisfError("damaged", "an embedded image without its Data element")
        data = base64.b64decode(child.text or "")
    else:
        raise XisfError("unsupported", f"the data location {location!r} is not supported")
    compression = image.get("compression")
    if compression:
        parts = compression.split(":")
        try:
            uncompressed = int(parts[1])
            item_size = int(parts[2]) if len(parts) > 2 else 1
        except (IndexError, ValueError) as error:
            raise XisfError("damaged", f"unreadable compression {compression!r}") from error
        data = decompress(parts[0], data, uncompressed, item_size)
    return data


def read_xisf(path: Path) -> np.ndarray:
    """The first image of the XISF file ``path``: ``H x W`` or ``C x H x W``, in the file's own range."""
    with open(path, "rb") as handle:
        header = read_header(handle)
        image = next((element for element in header.iter() if local(element.tag) == "Image"), None)
        if image is None:
            raise XisfError("unsupported", "the file holds no image")
        width, height, channels = parse_geometry(image.get("geometry", ""))
        sample = image.get("sampleFormat", "UInt16")
        if sample not in SAMPLE_FORMATS:
            raise XisfError("unsupported", f"the sample format {sample} is not supported")
        order = ">" if image.get("byteOrder", "little") == "big" else "<"
        dtype = np.dtype(order + SAMPLE_FORMATS[sample])
        data = block_bytes(image, handle)
    count = width * height * channels
    if len(data) < count * dtype.itemsize:
        raise XisfError("damaged", "the data block is shorter than the image's geometry")
    pixels = np.frombuffer(data, dtype=dtype, count=count).astype(dtype.newbyteorder("="))
    if image.get("pixelStorage", "Planar") == "Normal":
        pixels = pixels.reshape(height, width, channels).transpose(2, 0, 1)
    else:
        pixels = pixels.reshape(channels, height, width)
    return pixels[0] if channels == 1 else pixels


def normalise(data: np.ndarray) -> np.ndarray:
    """``data`` as ``float32`` on the scale of the image's format: integers over their range, floats as they are.

    An integer image (a stretched 8-bit export, a 16-bit master) is put on
    0..1 by the range of its integer type, so the same figures mean the same
    thing for both formats; a float image already is (PixInsight and Siril
    write 0..1; one written in ADU keeps its range).
    """
    if np.issubdtype(data.dtype, np.integer):
        return data.astype(np.float32) / float(np.iinfo(data.dtype).max)
    return data.astype(np.float32)
