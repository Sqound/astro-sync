"""How this package runs Siril without touching the person's own configuration.

``siril-cli`` writes its configuration (``~/.config/siril/config.<version>.ini`` on Linux and macOS) at every
exit unless it is given ``-i <file>``.  A run of Astro Sync must not rewrite the settings the person chose in
Siril, so every call carries ``-i`` and a configuration of its own, in the folder the run works in (the
scratch folder, removed with the run).  Siril refuses a configuration file that does not exist ("Could not
load or create settings file") and fills an empty one with its defaults, so the file is created empty.
"""

from __future__ import annotations

from pathlib import Path

CONFIG_NAME = "siril-config.ini"


def config_in(work: Path) -> Path:
    """The configuration file of the runs of Siril working in ``work``; created (empty) when missing."""
    work = Path(work)
    work.mkdir(parents=True, exist_ok=True)
    path = work / CONFIG_NAME
    if not path.exists():
        path.write_text("", encoding="utf-8")
    return path


def command(siril: str | Path, work: Path, script: Path) -> list[str]:
    """``siril -i <own configuration> -d <work> -s <script>``."""
    return [str(siril), "-i", str(config_in(work)), "-d", str(work), "-s", str(script)]
