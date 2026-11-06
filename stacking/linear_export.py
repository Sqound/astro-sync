#!/usr/bin/env python3
"""The "no processing" output of the stacking: the linear master as a 16-bit TIFF.

Astro Sync's option "No processing" (``astro_sync_stack.py --linear-only``)
runs no render pipeline.  What it delivers is Siril's linear master as it
came out of the stacking: calibrated, aligned and integrated, **not
stretched**, so the person can process it with their own tools.

Siril writes that master already normalised to 0..1 (``-output_norm``), so
the conversion is a plain scaling to 0..65535: no gradient removal, no black
point, no stretch, no colour change, no star separation.  A colour master is
written as an RGB TIFF, a mono master as a grayscale one.  The file is
``<name>_linear.tif`` (``pipeline_defs.linear_file_name``): the same naming rule as a
render, ``<name>_<pipeline id>.tif``.

Orientation: the rows are written in the order ``astropy`` reads them, exactly
as ``renders.py`` does for the rendered pipelines, so this image and the
renders of the same stacking have the same orientation.

Needs ``numpy``, ``astropy`` and ``tifffile`` (the stacking environment).
"""

from __future__ import annotations

from pathlib import Path

import numpy as np

from pipeline_defs import linear_file_name
from software_tag import software

MAX_16_BIT = 65535


def read_master(source: Path) -> np.ndarray:
    """The master as height x width (mono) or height x width x 3 (colour), float32."""
    from astropy.io import fits

    data = np.asarray(fits.getdata(source, memmap=False), np.float32)
    if data.ndim == 2:
        return data
    if data.ndim == 3 and data.shape[0] == 3:
        return np.moveaxis(data, 0, -1)
    raise ValueError(f"{source.name}: expected a 2-D or 3 x H x W FITS image, got {data.shape}")


def to_16_bit(image: np.ndarray) -> np.ndarray:
    """0..1 floats as 16-bit integers; values outside 0..1 are clipped, NaN becomes 0."""
    return np.round(np.clip(np.nan_to_num(image, nan=0.0), 0.0, 1.0) * MAX_16_BIT).astype(np.uint16)


def write_linear_tiff(master: Path, out_dir: Path, stem: str) -> Path:
    """Write ``master`` (a Siril FITS) to ``out_dir/<stem>_linear.tif`` as a lossless 16-bit TIFF; return the path.

    No contrast of presentation: its curve pivots on the sky level of a
    stretched image and does nothing visible on a linear one.
    """
    import tifffile

    image = to_16_bit(read_master(master))
    destination = out_dir / linear_file_name(stem)
    tifffile.imwrite(destination, image, photometric="rgb" if image.ndim == 3 else "minisblack", compression="deflate",
                     software=software())
    return destination
