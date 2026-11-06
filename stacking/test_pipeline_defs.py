"""Tests of the pipeline definitions loader (standard library only).

Run from this folder:  python3 -m unittest -v test_pipeline_defs
"""

from __future__ import annotations

import json
import os
import re
import stat
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

import astro_sync_stack
import check_environment
import pipeline_defs

HERE = Path(__file__).resolve().parent
LANGUAGES = {"fr", "en", "ja", "de", "es", "it", "pt-BR", "ru", "zh-Hans", "zh-Hant"}


def definition(pipeline_id: str, **changes: object) -> dict[str, object]:
    data: dict[str, object] = {
        "id": pipeline_id, "version": "1", "name": {"en": "Test"}, "description": {"en": "A test."},
        "filter": "none", "author": "Someone", "licence": "CC0-1.0", "entry": "renders:natural_deep",
        "requires": ["siril"], "outputs": ["tif"],
    }
    data.update(changes)
    return data


def write(root: Path, folder: str, data: object) -> None:
    (root / folder).mkdir(parents=True)
    (root / folder / "pipeline.json").write_text(data if isinstance(data, str) else json.dumps(data), encoding="utf-8")


class ShippedDefinitions(unittest.TestCase):
    def test_the_four_shipped_pipelines_load_in_ten_languages(self) -> None:
        pipelines, problems = pipeline_defs.load_pipelines()
        self.assertEqual(problems, [])
        self.assertEqual(list(pipelines), ["raw", "dualband-correction", "hoo", "polished"])
        for pipeline in pipelines.values():
            self.assertEqual(set(pipeline.name), LANGUAGES)
            self.assertEqual(set(pipeline.description), LANGUAGES)
            self.assertEqual(pipeline.licence, "MIT")
        self.assertEqual(pipelines["raw"].filter, "none")
        self.assertEqual(pipelines["dualband-correction"].filter, "dual-band")
        # "Polished render" is for any capture, the HOO palette for a dual-band one only.
        self.assertEqual((pipelines["polished"].filter, pipelines["hoo"].filter), ("none", "dual-band"))
        self.assertEqual(pipelines["polished"].text("name", "ja"), "丁寧な仕上がり")
        self.assertEqual(pipelines["polished"].text("name", "en"), "Polished render")
        self.assertEqual(pipelines["hoo"].text("name", "fr"), "Palette HOO")
        self.assertEqual(pipelines["polished"].optional_tools, ["starnet2", "graxpert"])

    def test_every_shipped_pipeline_details_its_steps_in_ten_languages(self) -> None:
        pipelines, _ = pipeline_defs.load_pipelines()
        for pipeline in pipelines.values():
            self.assertGreaterEqual(len(pipeline.details), 5, pipeline.id)
            for step in pipeline.details:
                self.assertEqual((set(step["title"]), set(step["text"])), (LANGUAGES, LANGUAGES), pipeline.id)
        listed = pipelines["polished"].as_json("fr")
        self.assertEqual(listed["details"], pipelines["polished"].details)
        self.assertEqual(listed["displayDetails"][0]["title"], "Fond de ciel")
        self.assertTrue(all(set(step) == {"title", "text"} for step in listed["displayDetails"]))
        # Without a language the steps stay per language, nothing resolved.
        self.assertNotIn("displayDetails", pipelines["raw"].as_json())

    def test_every_shipped_entry_is_in_the_renders_table(self) -> None:
        pipelines, _ = pipeline_defs.load_pipelines()
        self.assertTrue({p.render_function for p in pipelines.values()} <= pipeline_defs.shipped_render_names())

    def test_the_proprietary_word_is_nowhere_in_the_stacking_folder(self) -> None:
        word = "vesp" + "era"
        for path in HERE.rglob("*"):
            if path.is_file() and path.suffix in {".py", ".md", ".json", ".in", ".txt"} and "__pycache__" not in path.parts:
                self.assertNotIn(word, path.read_text(encoding="utf-8").lower(), str(path))


class Fallback(unittest.TestCase):
    def test_language_then_region_less_language_then_english(self) -> None:
        table = {"en": "English", "pt": "Portuguese", "ja": "Japanese"}
        self.assertEqual(pipeline_defs.resolve_text(table, "ja"), "Japanese")
        self.assertEqual(pipeline_defs.resolve_text(table, "pt-BR"), "Portuguese")
        self.assertEqual(pipeline_defs.resolve_text(table, "ru"), "English")
        self.assertEqual(pipeline_defs.resolve_text(table, None), "English")


class AddedDefinitions(unittest.TestCase):
    def load(self, **folders: object) -> tuple[dict[str, pipeline_defs.Pipeline], list[dict[str, str]]]:
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        root = Path(directory.name)
        for folder, data in folders.items():
            write(root, folder.replace("_", "-"), data)
        return pipeline_defs.load_pipelines([root])

    def test_a_valid_added_definition_is_loaded_after_the_shipped_ones(self) -> None:
        pipelines, problems = self.load(mine=definition("mine"))
        self.assertEqual(problems, [])
        self.assertEqual(list(pipelines), ["raw", "dualband-correction", "hoo", "polished", "mine"])
        self.assertEqual(pipelines["mine"].source, "user")

    def test_invalid_definitions_are_ignored_and_reported_and_do_not_stop_the_others(self) -> None:
        pipelines, problems = self.load(
            good=definition("good"),
            broken="{oops",
            wrong_id=definition("other"),
            no_english=definition("no-english", name={"fr": "Rien"}),
            own_code=definition("own-code", entry="evil:run"),
            unknown_function=definition("unknown-function", entry="renders:lum"),
            unknown_tool=definition("unknown-tool", requires=["siril", "gimp"]),
            no_siril=definition("no-siril", requires=["starnet2"]),
            no_tif=definition("no-tif", outputs=["png"]),
            future=definition("future", version="2"),
            taken=definition("raw"),
        )
        self.assertIn("good", pipelines)
        self.assertEqual({item["folder"] for item in problems},
                         {"broken", "wrong-id", "no-english", "own-code", "unknown-function", "unknown-tool",
                          "no-siril", "no-tif", "future", "taken"})
        self.assertEqual(pipelines["raw"].source, "shipped")
        self.assertTrue(all(item["reason"] for item in problems))

    def test_details_are_optional_and_checked_when_given(self) -> None:
        step = {"title": {"en": "Background", "fr": "Fond"}, "text": {"en": "Removed."}}
        pipelines, problems = self.load(
            plain=definition("plain"),
            detailed=definition("detailed", details=[step]),
            empty=definition("empty", details=[]),
            no_english=definition("no-english", details=[{"title": {"fr": "Fond"}, "text": {"en": "x"}}]),
            extra_key=definition("extra-key", details=[{**step, "url": "https://example.org"}]),
            too_many=definition("too-many", details=[step] * 13),
        )
        self.assertEqual(pipelines["plain"].details, [])
        self.assertEqual(pipelines["detailed"].as_json("fr")["displayDetails"], [{"title": "Fond", "text": "Removed."}])
        self.assertEqual({item["folder"] for item in problems}, {"empty", "no-english", "extra-key", "too-many"})

    def test_a_preview_is_checked_and_a_broken_one_is_ignored_not_the_pipeline(self) -> None:
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        root = Path(directory.name)
        png = b"\x89PNG\r\n\x1a\n" + b"0" * 32
        jpeg = b"\xff\xd8\xff\xe0" + b"0" * 32
        cases = {
            "with-png": ("preview.png", png),
            "with-jpeg": ("images/look.jpg", jpeg),
            "lying": ("preview.jpg", b"GIF89a" + b"0" * 32),
            "too-big": ("preview.png", png + b"0" * (512 * 1024)),
            "missing": ("nothing.png", None),
            "dotdot": ("../outside.png", jpeg),
            "absolute": ("/etc/hosts", None),
        }
        for folder, (relative, content) in cases.items():
            write(root, folder, definition(folder, preview=relative))
            if content is not None and ".." not in relative:
                target = root / folder / relative
                target.parent.mkdir(parents=True, exist_ok=True)
                target.write_bytes(content)
        write(root, "plain", definition("plain"))
        link = root / "linked"
        write(root, "linked", definition("linked", preview="out.png"))
        (root / "elsewhere.png").write_bytes(png)
        os.symlink(root / "elsewhere.png", link / "out.png")
        pipelines, problems = pipeline_defs.load_pipelines([root])
        self.assertEqual(problems, [])
        self.assertEqual(pipelines["with-png"].preview, "preview.png")
        self.assertEqual(pipelines["with-jpeg"].preview, "images/look.jpg")
        self.assertIsNone(pipelines["plain"].preview)
        self.assertNotIn("previewIssue", pipelines["plain"].as_json())
        for folder in ("lying", "too-big", "missing", "dotdot", "absolute", "linked"):
            self.assertIsNone(pipelines[folder].preview, folder)
            self.assertTrue(pipelines[folder].preview_issue, folder)
            self.assertIn("previewIssue", pipelines[folder].as_json(), folder)

    def test_every_shipped_pipeline_has_a_preview_of_the_same_size(self) -> None:
        pipelines, problems = pipeline_defs.load_pipelines()
        self.assertEqual(problems, [])
        for pipeline in pipelines.values():
            self.assertEqual(pipeline.preview, "preview.jpg", pipeline.id)
            self.assertIsNone(pipeline.preview_issue, pipeline.id)
            data = (pipeline.folder / "preview.jpg").read_bytes()
            self.assertLessEqual(len(data), 150 * 1024, pipeline.id)

    def test_a_missing_folder_is_simply_empty(self) -> None:
        pipelines, problems = pipeline_defs.load_pipelines([HERE / "does-not-exist"])
        self.assertEqual((list(pipelines), problems), (["raw", "dualband-correction", "hoo", "polished"], []))


class CommandLine(unittest.TestCase):
    def run_script(self, *arguments: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run([sys.executable, str(HERE / "astro_sync_stack.py"), *arguments],
                              capture_output=True, text=True, timeout=60)

    def test_list_pipelines_json(self) -> None:
        result = self.run_script("list-pipelines", "--json", "--lang", "fr")
        self.assertEqual(result.returncode, 0, result.stderr)
        listing = json.loads(result.stdout)
        self.assertEqual([item["id"] for item in listing["pipelines"]], ["raw", "dualband-correction", "hoo", "polished"])
        self.assertEqual(listing["pipelines"][0]["displayName"], "Rendu naturel")
        # Steps, as written and in the language asked.
        raw = listing["pipelines"][0]
        self.assertEqual(len(raw["details"]), len(raw["displayDetails"]))
        self.assertEqual(raw["displayDetails"][0]["title"], "Fond de ciel")
        self.assertEqual(listing["ignored"], [])

    def test_an_unknown_pipeline_is_refused_with_the_known_ids(self) -> None:
        result = self.run_script("--lights", "x", "--work", "w", "--out", "o", "--name", "T", "--pipeline", "nope", "--json")
        self.assertEqual(result.returncode, 2)
        event = json.loads(result.stdout)
        self.assertEqual(event["code"], "unknown_pipeline")
        self.assertIn("raw, dualband-correction", event["message"])


DIGEST = "a" * 64


def registry_data(**changes: object) -> dict[str, object]:
    """A small valid registry (one tool, one python-packages entry); ``changes`` replace the tool's fields."""
    tool: dict[str, object] = {
        "id": "tool-x", "name": {"en": "Tool X"}, "kind": "tool", "minVersion": "1.4",
        "check": {"command": "tool-x", "args": ["--version"], "versionPattern": r"tool-x (\d+(?:\.\d+)+)"},
        "downloads": {"macos-arm64": {"url": "https://example.org/tool-x.dmg", "sha256": DIGEST, "bytes": 1000}},
        "page": "https://example.org/download", "licence": "MIT",
    }
    tool.update(changes)
    tool = {key: value for key, value in tool.items() if value is not None}
    return {"format": 1, "dependencies": [tool, {
        "id": "py-x", "name": {"en": "Python X"}, "kind": "python-packages", "minVersion": ">=3.12,<3.15",
        "requirements": "../requirements.txt", "page": "https://example.org/python", "licence": "PSF-2.0"}]}


class RegistryRules(unittest.TestCase):
    """The shipped registry, and the ways a registry entry can be refused."""

    def load(self, data: object) -> dict[str, pipeline_defs.RegistryEntry]:
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        folder = Path(directory.name) / "dependencies"
        folder.mkdir()
        (Path(directory.name) / "requirements.txt").write_text(
            f"numpy==2.5.1 \\\n    --hash=sha256:{DIGEST}\n    # via -r requirements.in\n", encoding="utf-8")
        (folder / "registry.json").write_text(json.dumps(data), encoding="utf-8")
        return pipeline_defs.load_registry(folder / "registry.json")

    def refused(self, data: object, fragment: str) -> None:
        with self.assertRaises(ValueError) as caught:
            self.load(data)
        self.assertIn(fragment, str(caught.exception))

    def test_the_shipped_registry_is_valid_and_complete(self) -> None:
        registry = pipeline_defs.load_registry()
        self.assertEqual(set(registry), {"siril", "starnet2", "python-env", "graxpert", "astap", "astap-database", "astrometry-net"})
        # GraXpert, checked, with a pinned download for each system (real SHA-256 of the 3.0.2 files).
        graxpert = registry["graxpert"]
        self.assertEqual((graxpert.kind, graxpert.min_version), ("tool", "3.0"))
        self.assertEqual(set(graxpert.downloads), {"macos-arm64", "macos-x64", "windows-x64", "linux-x64"})
        self.assertTrue(all("/Steffenhir/GraXpert/releases/download/3.0.2/" in d["url"] for d in graxpert.downloads.values()))
        self.assertIn("CC-BY-NC-SA-4.0", graxpert.licence)
        pattern = re.compile(str(graxpert.check["versionPattern"]))
        line = "2026-10-14 23:33:20,156 MainProcess root INFO     GraXpert version: 3.0.2 release: Umbriel"
        self.assertEqual(pattern.search(line).group(1), "3.0.2")
        self.assertEqual(registry["siril"].min_version, "1.4")
        self.assertEqual(registry["python-env"].min_version, ">=3.12,<3.14")
        self.assertEqual(registry["python-env"].downloads, {})
        self.assertGreaterEqual(len(registry["python-env"].packages), 14)
        for entry in registry.values():
            self.assertTrue(entry.page.startswith("https://"))
        for entry in registry.values():
            if entry.kind == "tool" and entry.check is not None:
                self.assertEqual(set(entry.name), LANGUAGES)

    def test_a_valid_registry_with_a_download_loads(self) -> None:
        registry = self.load(registry_data())
        self.assertEqual(registry["tool-x"].downloads["macos-arm64"]["sha256"], DIGEST)
        self.assertEqual(registry["py-x"].packages, [("numpy", "2.5.1")])

    def test_a_url_without_a_digest_is_refused_and_a_digest_without_a_url_too(self) -> None:
        self.refused(registry_data(downloads={"macos-arm64": {"url": "https://example.org/x.dmg"}}), "go together")
        self.refused(registry_data(downloads={"macos-arm64": {"sha256": DIGEST}}), "go together")
        self.refused(registry_data(downloads={"macos-arm64": {"url": "https://example.org/x", "sha256": "abc"}}), "64 lower-case")
        self.refused(registry_data(downloads={"macos-arm64": {"url": "http://example.org/x", "sha256": DIGEST}}), "https")

    def test_an_unknown_system_or_kind_is_refused(self) -> None:
        self.refused(registry_data(downloads={"amiga": {"url": "https://example.org/x", "sha256": DIGEST}}), "unknown system")
        self.refused(registry_data(kind="script"), "kind 'script' is unknown")

    def test_python_packages_take_no_downloads_and_no_check(self) -> None:
        data = registry_data()
        data["dependencies"][1]["downloads"] = {"linux-x64": {"url": "https://example.org/x", "sha256": DIGEST}}
        self.refused(data, "no 'downloads'")
        data = registry_data()
        del data["dependencies"][1]["requirements"]
        self.refused(data, "needs 'requirements'")

    def test_the_detection_command_can_only_ask_for_a_version(self) -> None:
        for args in (["-rf"], ["--version", "--other"], ["-c", "print(1)"], []):
            self.refused(registry_data(check={"command": "tool-x", "args": args, "versionPattern": "(1)"}), "version query")
        self.refused(registry_data(check={"command": "/bin/sh", "args": ["--version"], "versionPattern": "(1)"}), "bare program name")
        self.refused(registry_data(check={"command": "tool-x", "args": ["--version"], "versionPattern": "no group"}), "exactly one group")
        self.refused(registry_data(check={"command": "tool-x", "args": ["--version"], "versionPattern": "(["}), "regular expression")
        self.refused(registry_data(check=None), "go together")

    def test_requirements_must_be_pinned_with_digests(self) -> None:
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        for content, fragment in (("numpy>=2\n", "name==version"), ("numpy==2.5.1\n", "no --hash")):
            path = Path(directory.name) / "requirements.txt"
            path.write_text(content, encoding="utf-8")
            with self.assertRaises(ValueError) as caught:
                pipeline_defs.parse_requirements(path)
            self.assertIn(fragment, str(caught.exception))

    def test_version_helpers(self) -> None:
        self.assertEqual(pipeline_defs.compare_versions((1, 4), (1, 4, 0)), 0)
        self.assertTrue(pipeline_defs.satisfies((1, 4, 4), "1.4"))
        self.assertFalse(pipeline_defs.satisfies((1, 2, 6), "1.4"))
        self.assertTrue(pipeline_defs.satisfies((3, 14, 7), ">=3.12,<3.15"))
        self.assertFalse(pipeline_defs.satisfies((3, 15, 0), ">=3.12,<3.15"))
        self.assertFalse(pipeline_defs.satisfies((3, 11, 9), ">=3.12,<3.15"))


class ManifestDependencies(unittest.TestCase):
    """A manifest references the registry; it carries no download of its own."""

    def load(self, dependencies: object, **changes: object) -> tuple[dict[str, pipeline_defs.Pipeline], list[dict[str, str]]]:
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        root = Path(directory.name) / "pipelines"
        write(root, "mine", definition("mine", dependencies=dependencies, **changes))
        return pipeline_defs.load_pipelines([root])

    @staticmethod
    def entry(dependency_id: str = "siril", **changes: object) -> dict[str, object]:
        data: dict[str, object] = {"id": dependency_id, "required": True, "reason": {"en": "Because."}}
        data.update(changes)
        return data

    def reason(self, dependencies: object, **changes: object) -> str:
        pipelines, problems = self.load(dependencies, **changes)
        self.assertNotIn("mine", pipelines)
        return next(item["reason"] for item in problems if item["folder"] == "mine")

    def test_the_shipped_manifests_declare_siril_starnet2_and_the_python_environment(self) -> None:
        pipelines, problems = pipeline_defs.load_pipelines()
        self.assertEqual(problems, [])
        for pipeline in pipelines.values():
            by_id = {dependency.id: dependency for dependency in pipeline.dependencies}
            if pipeline.id == "polished":
                # GraXpert, optional: without it, the polynomial and no denoising.
                self.assertEqual(list(by_id), ["siril", "starnet2", "graxpert", "python-env"])
                self.assertEqual([by_id[key].required for key in by_id], [True, False, False, True])
            else:
                self.assertEqual(list(by_id), ["siril", "starnet2", "python-env"])
                self.assertEqual([by_id[key].required for key in by_id], [True, False, True])
            for dependency in pipeline.dependencies:
                self.assertEqual(set(dependency.reason), LANGUAGES)

    def test_a_valid_added_manifest_is_resolved_against_the_registry(self) -> None:
        pipelines, problems = self.load([self.entry(minVersion="1.4.4")])
        self.assertEqual(problems, [])
        dependency = pipelines["mine"].dependencies[0]
        self.assertEqual((dependency.id, dependency.entry.kind, dependency.min_version), ("siril", "tool", "1.4.4"))

    def test_an_identifier_that_is_not_in_the_registry_refuses_the_manifest(self) -> None:
        reason = self.reason([self.entry("gimp")])
        self.assertIn("not allowed", reason)
        self.assertIn("pull request", reason)
        self.assertIn("proposez-la par une PR", reason)

    def test_a_minimum_lower_than_the_registry_is_refused(self) -> None:
        self.assertIn("lower than the registry", self.reason([self.entry(minVersion="1.2")]))
        self.assertIn("only accepted for a checked tool", self.reason([self.entry("python-env", minVersion="3.10")]))

    def test_a_manifest_cannot_bring_a_download_a_check_or_a_page(self) -> None:
        for key, value in (("url", "https://example.org/x"), ("downloads", {}), ("check", {}), ("page", "https://example.org")):
            reason = self.reason([self.entry(**{key: value})])
            self.assertIn(f"'{key}' is not allowed in a pipeline.json", reason)

    def test_required_must_agree_with_requires_and_ids_are_unique(self) -> None:
        self.assertIn("disagrees", self.reason([self.entry("starnet2", required=True)], requires=["siril", "starnet2?"]))
        self.assertIn("listed twice", self.reason([self.entry(), self.entry()]))
        self.assertIn("reason", self.reason([{"id": "siril", "required": True}]))

    def test_an_unreadable_registry_ignores_the_manifests_that_declare_dependencies(self) -> None:
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        bad = Path(directory.name) / "registry.json"
        bad.write_text("{oops", encoding="utf-8")
        state = pipeline_defs.registry_state(bad)
        self.assertEqual(state[0], {})
        pipelines, problems = pipeline_defs.load_pipelines(registry=state)
        self.assertEqual(pipelines, {})
        self.assertTrue(all("registry cannot be read" in item["reason"] for item in problems))

    def test_a_manifest_without_dependencies_stays_valid(self) -> None:
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        root = Path(directory.name)
        write(root, "plain", definition("plain"))
        pipelines, problems = pipeline_defs.load_pipelines([root])
        self.assertEqual((problems, pipelines["plain"].dependencies), ([], []))


class ListPipelinesDependencies(unittest.TestCase):
    def run_script(self, script: str, *arguments: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run([sys.executable, str(HERE / script), *arguments], capture_output=True, text=True, timeout=120)

    def test_list_pipelines_exposes_the_resolved_dependencies(self) -> None:
        result = self.run_script("astro_sync_stack.py", "list-pipelines", "--json", "--lang", "ja")
        self.assertEqual(result.returncode, 0, result.stderr)
        listing = json.loads(result.stdout)
        raw = listing["pipelines"][0]
        by_id = {item["id"]: item for item in raw["dependencies"]}
        self.assertEqual(list(by_id), ["siril", "starnet2", "python-env"])
        self.assertEqual(by_id["siril"]["minVersion"], "1.4")
        self.assertEqual(by_id["siril"]["check"]["args"], ["--version"])
        self.assertEqual(by_id["siril"]["page"], "https://siril.org/download/")
        self.assertEqual(by_id["siril"]["downloads"], {})
        self.assertFalse(by_id["starnet2"]["required"])
        self.assertTrue(by_id["siril"]["displayReason"].startswith("Siril が"))
        self.assertEqual(by_id["python-env"]["requirements"], "../requirements.txt")
        self.assertNotIn("downloads", by_id["python-env"])
        self.assertEqual({item["id"] for item in listing["registry"]},
                         {"siril", "starnet2", "python-env", "graxpert", "astap", "astap-database", "astrometry-net"})
        self.assertNotIn("registryError", listing)


@unittest.skipIf(os.name == "nt", "the fake tools are shell scripts")
class CheckEnvironmentDependencies(unittest.TestCase):
    def fake_tool(self, output: str) -> Path:
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        path = Path(directory.name) / "fake"
        path.write_text(f"#!/bin/sh\necho '{output}'\n", encoding="utf-8")
        path.chmod(path.stat().st_mode | stat.S_IXUSR)
        return path

    def states(self, siril: Path | None) -> dict[str, str]:
        pipelines, _ = pipeline_defs.load_pipelines()
        rows = check_environment.evaluate_dependencies({"raw": pipelines["raw"]}, {"siril": siril, "starnet2": siril})
        return {row["id"]: row["state"] for row in rows}

    def test_present_outdated_and_missing_are_told_apart(self) -> None:
        self.assertEqual(self.states(self.fake_tool("siril 1.4.4"))["siril"], "present")
        self.assertEqual(self.states(self.fake_tool("siril 1.2.6"))["siril"], "outdated")
        self.assertEqual(self.states(self.fake_tool("nothing useful"))["siril"], "missing")
        self.assertEqual(self.states(Path("/does/not/exist"))["siril"], "missing")

    def test_check_environment_json_lists_every_declared_dependency(self) -> None:
        result = subprocess.run([sys.executable, str(HERE / "check_environment.py"), "--json"],
                                capture_output=True, text=True, timeout=120)
        report = json.loads(result.stdout)
        rows = report["dependencies"]
        self.assertEqual([(row["pipeline"], row["id"]) for row in rows[:3]],
                         [("raw", "siril"), ("raw", "starnet2"), ("raw", "python-env")])
        # raw, dualband-correction, hoo: three each; polished: four (GraXpert).
        self.assertEqual(len(rows), 13)
        self.assertIn(("polished", "graxpert"), [(row["pipeline"], row["id"]) for row in rows])
        self.assertTrue({row["state"] for row in rows} <= {"present", "missing", "outdated", "not-checked"})
        self.assertEqual(report["pipelines"]["registry"][0]["id"], "siril")


SCRIPT = "def render(nebula, stars):\n    return ('mine', nebula, stars)\n"


class PipelineScripts(unittest.TestCase):
    """``script:<path>``: a pipeline's own script, its digest, and the trust rule."""

    def folder(self, script: str | None = SCRIPT, **changes: object) -> Path:
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        root = Path(directory.name)
        write(root, "mine", definition("mine", entry="script:render.py", **changes))
        if script is not None:
            (root / "mine" / "render.py").write_text(script, encoding="utf-8")
        return root

    def arguments(self, root: Path, *trusted: str) -> object:
        import argparse

        return argparse.Namespace(pipelines_dir=[root], pipeline=["mine"], trusted=list(trusted))

    def test_a_declared_script_is_read_and_its_digest_exposed(self) -> None:
        import hashlib

        root = self.folder()
        pipelines, problems = pipeline_defs.load_pipelines([root])
        self.assertEqual(problems, [])
        digest = hashlib.sha256(SCRIPT.encode()).hexdigest()
        self.assertEqual(pipelines["mine"].script_sha256, digest)
        self.assertIsNone(pipelines["mine"].render_function)
        listed = pipeline_defs.listing(pipelines, problems)["pipelines"]
        by_id = {item["id"]: item for item in listed}
        self.assertEqual((by_id["mine"]["entry"], by_id["mine"]["scriptSha256"]), ("script:render.py", digest))
        self.assertIsNone(by_id["raw"]["scriptSha256"])

    def test_a_script_that_is_missing_or_outside_the_folder_refuses_the_manifest(self) -> None:
        for entry, fragment in (("script:nope.py", "no such file"), ("script:../x.py", "no '..'"),
                                ("script:/etc/x.py", "relative path"), ("script:render.txt", "ending in .py")):
            directory = tempfile.TemporaryDirectory()
            self.addCleanup(directory.cleanup)
            write(Path(directory.name), "mine", definition("mine", entry=entry))
            _, problems = pipeline_defs.load_pipelines([Path(directory.name)])
            self.assertIn(fragment, problems[0]["reason"], entry)

    def test_a_symbolic_link_out_of_the_folder_is_refused(self) -> None:
        root = self.folder(script=None)
        outside = Path(tempfile.mkdtemp())
        self.addCleanup(lambda: __import__("shutil").rmtree(outside, ignore_errors=True))
        (outside / "x.py").write_text(SCRIPT, encoding="utf-8")
        (root / "mine" / "render.py").symlink_to(outside / "x.py")
        _, problems = pipeline_defs.load_pipelines([root])
        self.assertIn("inside the pipeline's folder", problems[0]["reason"])

    def test_an_added_script_is_refused_without_trusted(self) -> None:
        root = self.folder()
        with self.assertRaises(astro_sync_stack.StackingError) as caught:
            astro_sync_stack.choose_pipelines(self.arguments(root))
        self.assertEqual((caught.exception.code, caught.exception.exit_code), ("script_not_trusted", 2))
        self.assertIn("--trusted mine=<sha256>", str(caught.exception))

    def test_an_added_script_is_accepted_with_the_right_digest(self) -> None:
        root = self.folder()
        digest = pipeline_defs.load_pipelines([root])[0]["mine"].script_sha256
        chosen = astro_sync_stack.choose_pipelines(self.arguments(root, f"mine={digest}"))
        self.assertEqual([item.id for item in chosen], ["mine"])
        trusted = astro_sync_stack.trusted_scripts([f"mine={digest}"])
        self.assertEqual(astro_sync_stack.render_pipeline(chosen[0], 1, 2, trusted), ("mine", 1, 2))

    def test_an_added_script_is_refused_with_another_digest_and_when_it_changed(self) -> None:
        root = self.folder()
        with self.assertRaises(astro_sync_stack.StackingError) as caught:
            astro_sync_stack.choose_pipelines(self.arguments(root, "mine=" + "b" * 64))
        self.assertEqual(caught.exception.code, "script_not_trusted")
        self.assertIn("changed since", str(caught.exception))
        digest = pipeline_defs.load_pipelines([root])[0]["mine"].script_sha256
        chosen = astro_sync_stack.choose_pipelines(self.arguments(root, f"mine={digest}"))
        (root / "mine" / "render.py").write_text(SCRIPT + "# edited\n", encoding="utf-8")  # after the check
        with self.assertRaises(astro_sync_stack.StackingError):
            astro_sync_stack.render_pipeline(chosen[0], 1, 2, {"mine": digest})

    def test_a_shipped_script_runs_without_a_question(self) -> None:
        root = self.folder()
        pipelines, _ = pipeline_defs.load_pipelines(shipped_dir=root)
        self.assertEqual(pipelines["mine"].source, "shipped")
        self.assertEqual(astro_sync_stack.render_pipeline(pipelines["mine"], 1, 2, {}), ("mine", 1, 2))

    def test_a_malformed_trusted_option_stops_the_command_line(self) -> None:
        result = subprocess.run([sys.executable, str(HERE / "astro_sync_stack.py"), "--lights", "x", "--work", "w", "--out", "o",
                                 "--name", "T", "--trusted", "mine=oops", "--json"], capture_output=True, text=True, timeout=60)
        self.assertEqual(result.returncode, 2)
        self.assertIn("--trusted expects", result.stderr)

    def test_an_added_script_pipeline_is_refused_by_the_command_line(self) -> None:
        root = self.folder()
        result = subprocess.run([sys.executable, str(HERE / "astro_sync_stack.py"), "--lights", "x", "--work", "w", "--out", "o",
                                 "--name", "T", "--pipelines-dir", str(root), "--pipeline", "mine", "--json", "--dry-run"],
                                capture_output=True, text=True, timeout=60)
        self.assertEqual(result.returncode, 2)
        self.assertEqual(json.loads(result.stdout)["code"], "script_not_trusted")


class FramesFrom(unittest.TestCase):
    """``--frames-from``: Astro Sync stacks one family of exposures at a time."""

    def setUp(self) -> None:
        self.root = Path(tempfile.mkdtemp())
        self.addCleanup(lambda: __import__("shutil").rmtree(self.root, ignore_errors=True))
        self.lights = self.root / "M31" / "lights"
        (self.lights / "Ha").mkdir(parents=True)
        for name in ("Ha/Light_002.fits", "Ha/Light_001.fits", "Light_OIII_001.fit", "notes.txt"):
            (self.lights / name).write_bytes(b"x")
        (self.root / "elsewhere.fits").write_bytes(b"x")

    def write(self, *paths: Path) -> Path:
        listing = self.root / "M31_2026-10-07.lights.txt"
        listing.write_text("\n".join(str(path) for path in paths) + "\n\n", encoding="utf-8")
        return listing

    def test_the_listed_frames_are_read_sorted_and_once(self) -> None:
        listing = self.write(self.lights / "Ha/Light_002.fits", self.lights / "Ha/Light_001.fits",
                             self.lights / "Ha/Light_002.fits")
        frames = astro_sync_stack.listed_frames(listing, self.lights)
        self.assertEqual([path.name for path in frames], ["Light_001.fits", "Light_002.fits"])

    def test_a_frame_outside_lights_or_not_a_fits_is_refused(self) -> None:
        for wrong in (self.root / "elsewhere.fits", self.lights / "notes.txt", self.lights / "missing.fits"):
            with self.subTest(wrong=wrong.name):
                with self.assertRaises(astro_sync_stack.StackingError) as caught:
                    astro_sync_stack.listed_frames(self.write(self.lights / "Ha/Light_001.fits", wrong), self.lights)
                self.assertEqual(caught.exception.code, "frames_outside_lights")
                self.assertEqual(caught.exception.exit_code, astro_sync_stack.EXIT_INPUT)


class LinearOnly(unittest.TestCase):
    """``--linear-only`` (the option "No processing"): no pipeline, only the linear stack as a TIFF."""

    def setUp(self) -> None:
        self.root = Path(tempfile.mkdtemp())
        self.addCleanup(lambda: __import__("shutil").rmtree(self.root, ignore_errors=True))
        self.lights = self.root / "lights"
        self.lights.mkdir()
        for index in (1, 2, 3):
            (self.lights / f"Light_{index:03d}.fits").write_bytes(b"x")

    def arguments(self, *extra: str) -> object:
        return astro_sync_stack.parse_args(["--lights", str(self.lights), "--work", str(self.root / "work"),
                                            "--out", str(self.root / "stacked"), "--name", "M31_2026-10-07", *extra])

    def plan(self, *extra: str) -> tuple[object, object]:
        from unittest import mock

        args = self.arguments(*extra)
        with mock.patch.object(astro_sync_stack, "is_colour", return_value=True):
            return args, astro_sync_stack.build_plan(args)

    def test_the_id_is_reserved_so_no_definition_can_take_the_name_of_the_output(self) -> None:
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        write(Path(directory.name), "linear", definition("linear"))
        pipelines, problems = pipeline_defs.load_pipelines([Path(directory.name)])
        self.assertNotIn("linear", pipelines)
        self.assertIn("reserved", problems[0]["reason"])
        self.assertEqual(pipeline_defs.linear_file_name("M31_2026-10-07"), "M31_2026-10-07_linear.tif")

    def test_the_plan_has_no_render_and_one_output_file(self) -> None:
        args, plan = self.plan("--linear-only")
        self.assertTrue(plan.linear_only)
        self.assertEqual(plan.pipelines, [])
        self.assertEqual(plan.steps[-2:], ["integration", "linear_export"])
        self.assertNotIn("render", plan.steps)
        fields = astro_sync_stack.plan_fields(plan, args)
        self.assertEqual(fields["outputs"], ["M31_2026-10-07_linear.tif", "M31_2026-10-07_stacking.json"])
        self.assertEqual((fields["pipelines"], fields["linearOnly"]), ([], True))

    def test_without_the_option_the_plan_is_unchanged(self) -> None:
        args, plan = self.plan()
        self.assertFalse(plan.linear_only)
        self.assertEqual([p.id for p in plan.pipelines], ["raw"])
        self.assertEqual(plan.steps[-1], "render")
        self.assertEqual(astro_sync_stack.plan_fields(plan, args)["outputs"],
                         ["M31_2026-10-07_raw.tif", "M31_2026-10-07_raw.png", "M31_2026-10-07_stacking.json"])

    def test_a_pipeline_beside_the_option_is_refused(self) -> None:
        result = subprocess.run([sys.executable, str(HERE / "astro_sync_stack.py"), "--lights", str(self.lights), "--work",
                                 str(self.root / "w"), "--out", str(self.root / "o"), "--name", "T", "--linear-only",
                                 "--pipeline", "raw", "--json"], capture_output=True, text=True, timeout=60)
        self.assertEqual(result.returncode, 2)
        self.assertIn("--linear-only runs no pipeline", result.stderr)

    def test_beside_renders_the_linear_stack_is_written_last_and_noted(self) -> None:
        # "No processing" combinable with the pipelines of a target.
        args, plan = self.plan("--pipeline", "raw", "--linear")
        self.assertEqual(([p.id for p in plan.pipelines], plan.linear_only, plan.with_linear), (["raw"], False, True))
        self.assertEqual(plan.steps[-2:], ["render", "linear_export"])
        fields = astro_sync_stack.plan_fields(plan, args)
        self.assertEqual(fields["outputs"], ["M31_2026-10-07_raw.tif", "M31_2026-10-07_raw.png",
                                             "M31_2026-10-07_linear.tif", "M31_2026-10-07_stacking.json"])
        self.assertTrue(fields["withLinear"])
        with self.assertRaises(SystemExit):
            self.arguments("--linear-only", "--linear")

    def test_the_option_needs_neither_starnet_nor_a_pipeline_that_requires_it(self) -> None:
        args, plan = self.plan("--linear-only", "--no-starnet")
        self.assertIsNone(astro_sync_stack.find_starnet(args, plan))
        args, plan = self.plan("--linear-only", "--starnet", str(self.root / "missing"))
        self.assertIsNone(astro_sync_stack.find_starnet(args, plan))


try:
    import astropy  # noqa: F401
    import numpy
    import tifffile
except ImportError:  # the stacking environment has them; a bare interpreter does not
    numpy = tifffile = None


@unittest.skipIf(tifffile is None, "needs numpy, astropy and tifffile (the stacking environment)")
class LinearExport(unittest.TestCase):
    """The linear master as a 16-bit TIFF: scaled, never stretched."""

    def setUp(self) -> None:
        self.root = Path(tempfile.mkdtemp())
        self.addCleanup(lambda: __import__("shutil").rmtree(self.root, ignore_errors=True))

    def master(self, data: object) -> Path:
        from astropy.io import fits

        path = self.root / "stack_linear.fit"
        fits.PrimaryHDU(numpy.asarray(data, numpy.float32)).writeto(path)
        return path

    def test_a_colour_master_is_written_as_an_rgb_16_bit_tiff_scaled_not_stretched(self) -> None:
        import linear_export

        planes = numpy.zeros((3, 4, 5), numpy.float32)
        planes[0] += 0.5
        planes[1] += 0.01
        planes[2, 0, 0] = 1.5   # above the range: clipped
        planes[2, 1, 1] = -0.2  # below: clipped
        written = linear_export.write_linear_tiff(self.master(planes), self.root, "M31_2026-10-07")
        self.assertEqual(written.name, "M31_2026-10-07_linear.tif")
        image = tifffile.imread(written)
        self.assertEqual((image.dtype, image.shape), (numpy.uint16, (4, 5, 3)))
        self.assertEqual(int(image[2, 2, 0]), 32768)  # 0.5 stays 0.5: a plain scaling
        self.assertEqual(int(image[2, 2, 1]), 655)  # 0.01 is not lifted by any stretch
        self.assertEqual((int(image[0, 0, 2]), int(image[1, 1, 2])), (65535, 0))

    def test_a_mono_master_stays_a_grayscale_image(self) -> None:
        import linear_export

        written = linear_export.write_linear_tiff(self.master(numpy.full((6, 7), 0.25)), self.root, "T")
        image = tifffile.imread(written)
        self.assertEqual((image.dtype, image.shape, int(image[3, 3])), (numpy.uint16, (6, 7), 16384))

    def test_a_master_of_another_shape_is_refused(self) -> None:
        import linear_export

        with self.assertRaises(ValueError):
            linear_export.write_linear_tiff(self.master(numpy.zeros((2, 4, 5))), self.root, "T")

    def test_the_step_reports_the_file_and_writes_nothing_else(self) -> None:
        import io
        from contextlib import redirect_stdout

        master = self.master(numpy.full((3, 3, 4), 0.1))
        out = self.root / "stacked"
        args = astro_sync_stack.argparse.Namespace(out=out, name="T")
        buffer = io.StringIO()
        with redirect_stdout(buffer):
            outputs = astro_sync_stack.linear_outputs(args, astro_sync_stack.Reporter(True), astro_sync_stack.Runner(), master)
        event = json.loads(buffer.getvalue())
        self.assertEqual((event["event"], event["pipeline"], event["file"]), ("output", "linear", "T_linear.tif"))
        self.assertEqual([item["file"] for item in outputs], ["T_linear.tif"])
        self.assertEqual(sorted(path.name for path in out.iterdir()), ["T_linear.tif"])


class FlatsFilters(unittest.TestCase):
    """The filters the flats name are written beside their count."""

    def setUp(self) -> None:
        self.root = Path(tempfile.mkdtemp())
        self.addCleanup(lambda: __import__("shutil").rmtree(self.root, ignore_errors=True))
        self.lights = self.root / "lights"
        self.lights.mkdir()
        for index in (1, 2):
            (self.lights / f"Light_{index:03d}.fits").write_bytes(b"x")
        self.flats = self.root / "flats"
        self.flats.mkdir()

    def flat(self, name: str, filter_name: str | None) -> None:
        import numpy
        from astropy.io import fits

        header = fits.Header()
        if filter_name is not None:
            header["FILTER"] = filter_name
        fits.PrimaryHDU(numpy.zeros((2, 2), dtype="float32"), header=header).writeto(self.flats / name)

    def plan(self) -> tuple[object, object]:
        from unittest import mock

        args = astro_sync_stack.parse_args(["--lights", str(self.lights), "--flats", str(self.flats), "--work",
                                            str(self.root / "work"), "--out", str(self.root / "stacked"), "--name", "T"])
        with mock.patch.object(astro_sync_stack, "is_colour", return_value=False):
            return args, astro_sync_stack.build_plan(args)

    def test_a_folder_of_one_filter_names_that_filter_and_counts_its_flats(self) -> None:
        for index in range(3):
            self.flat(f"Flat_{index}.fits", "Red")
        args, plan = self.plan()
        fields = astro_sync_stack.plan_fields(plan, args)
        self.assertEqual(fields["frames"]["flats"], 3)
        self.assertEqual(fields["flatsFilters"], ["Red"])

    def test_a_folder_of_several_filters_names_them_all(self) -> None:
        self.flat("a.fits", "Red")
        self.flat("b.fits", "Blue")
        self.flat("c.fits", None)
        args, plan = self.plan()
        self.assertEqual(astro_sync_stack.plan_fields(plan, args)["flatsFilters"], ["Blue", "Red"])

    def test_no_flats_name_no_filter(self) -> None:
        self.assertEqual(astro_sync_stack.flats_filters([]), [])


if __name__ == "__main__":
    unittest.main()
