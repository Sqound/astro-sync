"""A run of Astro Sync never rewrites the configuration of the person's own Siril.

``siril-cli`` writes ``~/.config/siril/config.<version>.ini`` at every exit unless it is given ``-i``; each call
of Siril made by this package goes through :mod:`siril_config`, which gives it a configuration of its own.
"""

from __future__ import annotations

import re
import unittest
from pathlib import Path

import siril_config

HERE = Path(__file__).resolve().parent


class OwnConfiguration(unittest.TestCase):
    def test_the_command_carries_its_own_configuration_inside_the_work_folder(self) -> None:
        import tempfile

        with tempfile.TemporaryDirectory() as folder:
            work = Path(folder) / "work"
            command = siril_config.command("siril-cli", work, work / "01.ssf")
            self.assertEqual(command[0], "siril-cli")
            ini = Path(command[command.index("-i") + 1])
            self.assertEqual(ini.parent, work)
            self.assertTrue(ini.is_file(), "Siril refuses a configuration that does not exist")
            self.assertEqual(command[command.index("-d") + 1], str(work))
            self.assertEqual(command[command.index("-s") + 1], str(work / "01.ssf"))

    def test_an_existing_configuration_is_left_as_it_is(self) -> None:
        import tempfile

        with tempfile.TemporaryDirectory() as folder:
            work = Path(folder)
            (work / siril_config.CONFIG_NAME).write_text("[core]\nwd=/somewhere\n", encoding="utf-8")
            siril_config.config_in(work)
            self.assertEqual((work / siril_config.CONFIG_NAME).read_text(encoding="utf-8"), "[core]\nwd=/somewhere\n")

    def test_no_module_starts_siril_with_a_bare_command_line(self) -> None:
        """Every call is built by ``siril_config.command``: a hand-written ``"-d", ... "-s"`` list has no ``-i``."""
        offenders = []
        for path in sorted(HERE.glob("*.py")):
            if path.name.startswith("test_") or path.name == "siril_config.py":
                continue
            for number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
                if re.search(r'"-d"\s*,', line) and "siril" in line.lower():
                    offenders.append(f"{path.name}:{number}")
        self.assertEqual(offenders, [])


if __name__ == "__main__":
    unittest.main()
