"""The ``Software`` tag of the TIFF files Astro Sync writes.

``tifffile`` stamps every TIFF with ``tifffile.py`` unless told otherwise, and the service reads that tag when a
capture arrives without a processing of its own: the page of the capture then said "Software: tifffile.py".
Every TIFF Astro Sync writes names Astro Sync instead, with its version when the application gave it
(``ASTRO_SYNC_VERSION`` in the environment of the script, set by ``python_env::script_command``).
"""

from __future__ import annotations

import os

VERSION_VARIABLE = "ASTRO_SYNC_VERSION"


def software() -> str:
    """« Astro Sync 0.9.1 », or « Astro Sync » when the version is not known."""
    version = os.environ.get(VERSION_VARIABLE, "").strip()
    return f"Astro Sync {version}" if version else "Astro Sync"
