#!/usr/bin/env python3
"""Read and validate the stacking pipeline definitions (``pipeline.json``).

A pipeline is one way to render the stacked image.  Each lives in its own
folder, ``<pipelines folder>/<id>/pipeline.json``; the format is written in
``pipelines/SCHEMA.md``.  Two kinds of folders are read, in this order:

1. the definitions shipped next to this file (``pipelines/``);
2. the folders given by the caller (Astro Sync passes
   ``<Astro Sync data>/pipelines/``), where people add their own.

What a pipeline needs installed (Siril, StarNet2, the Python environment) is
*not* written in its ``pipeline.json``: the manifest only references an
identifier of the dependency registry (``dependencies/registry.json``), the one
list of tools a pipeline may depend on, changed only by a pull request on the
repository.  This module reads the registry, validates it, and validates every
manifest against it.

A definition that is not valid is *ignored and reported*, never fatal: the
other pipelines stay usable.  The ``entry`` of a definition is either
``renders:<function>``, a render function shipped in ``renders.py`` (listed in
its ``RENDERS`` literal), or ``script:<relative path>``, a Python file of the
pipeline's own folder.  A script is never run on this module's say-so: its
SHA-256 is computed here and exposed (``scriptSha256``), and a script of a
pipeline the person added only runs when that digest was approved (see the
trust rule in ``pipelines/SCHEMA.md``).  This module reads ``renders.py`` with
``ast`` and imports none of the image libraries, so ``list-pipelines`` works
in any Python 3.

Run directly it prints the loaded definitions::

    python3 pipeline_defs.py [--pipelines-dir DIR]... [--lang CODE] [--json]
"""

from __future__ import annotations

import argparse
import ast
import hashlib
import json
import re
import sys
from dataclasses import dataclass, field
from pathlib import Path
from urllib.parse import urlsplit

HERE = Path(__file__).resolve().parent
SHIPPED_DIR = HERE / "pipelines"
RENDERS_MODULE = "renders"
DEFINITION_FILE = "pipeline.json"
FALLBACK_LANGUAGE = "en"
DEFAULT_PIPELINE = "raw"
# The id of the option "no processing" (``astro_sync_stack.py --linear-only``):
# the linear stack as a TIFF, no render.  It is not a pipeline, and no
# definition may take the id (it would clash with ``<name>_linear.tif``).
LINEAR_PIPELINE = "linear"
RESERVED_IDS = frozenset({LINEAR_PIPELINE})


def linear_file_name(stem: str) -> str:
    """``<stem>_linear.tif``: the file of the option "no processing", named like a render (``<stem>_<id>.tif``)."""
    return f"{stem}_{LINEAR_PIPELINE}.tif"

ID_PATTERN = re.compile(r"^[a-z0-9][a-z0-9-]{0,39}$")
LANGUAGE_PATTERN = re.compile(r"^[a-z]{2,3}(-[A-Za-z0-9]{2,8})*$")
FILTER_PATTERN = re.compile(r"^[a-z0-9]+(-[a-z0-9]+)*$")
ENTRY_PATTERN = re.compile(r"^([a-z_][a-z0-9_]*):([a-z_][a-z0-9_]*)$")
SCRIPT_PREFIX = "script:"
SCRIPT_PATH_PATTERN = re.compile(r"^[A-Za-z0-9_][A-Za-z0-9_.-]*(/[A-Za-z0-9_][A-Za-z0-9_.-]*)*\.py$")
MAX_SCRIPT_BYTES = 256 * 1024
REQUIRES_PATTERN = re.compile(r"^([a-z0-9][a-z0-9-]*)(\?)?$")

REGISTRY_FILE = HERE / "dependencies" / "registry.json"
REGISTRY_FORMAT = 1
SYSTEMS = ("macos-arm64", "macos-x64", "windows-x64", "linux-x64")
KINDS = ("tool", "python-packages")
# The detection command of the registry can only ask for a version.
VERSION_QUERIES = (("--version",), ("-version",), ("-V",), ("-v",), ("version",))
COMMAND_PATTERN = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$")
VERSION_PATTERN = re.compile(r"^\d+(\.\d+)*$")
SPEC_CLAUSE_PATTERN = re.compile(r"^(>=|<=|==|>|<)(\d+(?:\.\d+)*)$")
SHA256_PATTERN = re.compile(r"^[0-9a-f]{64}$")
REQUIREMENT_PATTERN = re.compile(r"^([A-Za-z0-9][A-Za-z0-9._-]*)==([^\s;]+)")
HASH_PATTERN = re.compile(r"--hash=sha256:([0-9a-f]{64})")
NOT_ALLOWED_HINT = "propose it with a pull request on the repository (dépendance non autorisée ; proposez-la par une PR sur le dépôt)"
# Keys a manifest dependency may carry: nothing that downloads or runs.
MANIFEST_DEPENDENCY_KEYS = ("id", "required", "reason", "minVersion")

KNOWN_TOOLS = ("siril", "starnet2", "graxpert")
KNOWN_OUTPUTS = ("tif", "png")
SUPPORTED_VERSIONS = ("1",)
MAX_TEXT = 600
MAX_FILE_BYTES = 64 * 1024
MAX_DETAILS = 12  # steps of ``details``
# ``preview`` (optional): a relative image of the pipeline's folder, PNG or JPEG, 512 KiB at most.
PREVIEW_PATH_PATTERN = re.compile(r"^[A-Za-z0-9_][A-Za-z0-9_.-]*(/[A-Za-z0-9_][A-Za-z0-9_.-]*)*\.(png|jpe?g)$", re.IGNORECASE)
MAX_PREVIEW_BYTES = 512 * 1024
PNG_MAGIC = b"\x89PNG\r\n\x1a\n"
JPEG_MAGIC = b"\xff\xd8\xff"


@dataclass
class Pipeline:
    id: str
    version: str
    name: dict[str, str]
    description: dict[str, str]
    filter: str
    author: str
    licence: str
    entry: str
    requires: list[str]
    outputs: list[str]
    source: str  # "shipped" or "user"
    folder: Path = field(default_factory=Path)
    dependencies: list["Dependency"] = field(default_factory=list)
    script_path: Path | None = None  # the pipeline's own script (``script:`` entry), else None
    script_sha256: str | None = None  # its SHA-256 as read when the definition was loaded
    # ``details`` (optional): the ordered steps, each {"title": texts, "text": texts}.
    details: list[dict[str, dict[str, str]]] = field(default_factory=list)
    # ``preview`` (optional): the validated relative path of the image, else None.
    preview: str | None = None
    # Why a ``preview`` that was written was ignored (the pipeline stays valid, without a preview).
    preview_issue: str | None = None

    @property
    def render_function(self) -> str | None:
        """The shipped render function (``renders:<function>`` entry), None for a script."""
        return None if self.script_path else self.entry.split(":", 1)[1]

    @property
    def required_tools(self) -> list[str]:
        return [item for item in self.requires if not item.endswith("?")]

    @property
    def optional_tools(self) -> list[str]:
        return [item[:-1] for item in self.requires if item.endswith("?")]

    def text(self, which: str, language: str | None) -> str:
        """``name`` or ``description`` in ``language``, else in English (the fallback)."""
        table = self.name if which == "name" else self.description
        return resolve_text(table, language)

    def as_json(self, language: str | None = None) -> dict[str, object]:
        data: dict[str, object] = {
            "id": self.id,
            "version": self.version,
            "name": self.name,
            "description": self.description,
            "filter": self.filter,
            "author": self.author,
            "licence": self.licence,
            "entry": self.entry,
            "scriptSha256": self.script_sha256,
            "requires": self.requires,
            "outputs": self.outputs,
            "source": self.source,
            "dependencies": [dependency.as_json(language) for dependency in self.dependencies],
            "details": self.details,
            "preview": self.preview,
        }
        if self.preview_issue:
            data["previewIssue"] = self.preview_issue
        if language:
            data["displayName"] = self.text("name", language)
            data["displayDescription"] = self.text("description", language)
            data["displayDetails"] = [
                {"title": resolve_text(step["title"], language), "text": resolve_text(step["text"], language)}
                for step in self.details
            ]
        return data


def resolve_text(table: dict[str, str], language: str | None) -> str:
    """The text for ``language``; then the same language without its region
    (``pt-BR`` -> ``pt``); then English."""
    candidates = []
    if language:
        candidates.append(language)
        if "-" in language:
            candidates.append(language.split("-", 1)[0])
    candidates.append(FALLBACK_LANGUAGE)
    for key in candidates:
        if table.get(key):
            return table[key]
    return next(iter(table.values()))


def shipped_render_names(renders_file: Path | None = None) -> set[str]:
    """Keys of the ``RENDERS`` dict literal of ``renders.py``, read without importing it."""
    path = renders_file or HERE / f"{RENDERS_MODULE}.py"
    tree = ast.parse(path.read_text(encoding="utf-8"))
    for node in tree.body:
        if isinstance(node, ast.Assign) and any(isinstance(t, ast.Name) and t.id == "RENDERS" for t in node.targets):
            if isinstance(node.value, ast.Dict):
                return {key.value for key in node.value.keys if isinstance(key, ast.Constant) and isinstance(key.value, str)}
    return set()


def _text_table(data: object, label: str) -> dict[str, str]:
    if not isinstance(data, dict) or not data:
        raise ValueError(f"{label} must be an object with one text per language")
    table: dict[str, str] = {}
    for language, text in data.items():
        if not LANGUAGE_PATTERN.match(language):
            raise ValueError(f"{label}: '{language}' is not a language code")
        if not isinstance(text, str) or not text.strip():
            raise ValueError(f"{label}.{language} must be a non-empty text")
        if len(text) > MAX_TEXT:
            raise ValueError(f"{label}.{language} is longer than {MAX_TEXT} characters")
        table[language] = text
    if FALLBACK_LANGUAGE not in table:
        raise ValueError(f"{label} needs an '{FALLBACK_LANGUAGE}' text (the fallback of the other languages)")
    return table


def _details(data: object) -> list[dict[str, dict[str, str]]]:
    """``details``: absent, or a list of 1 to ``MAX_DETAILS`` steps, each exactly ``title`` and ``text`` per language."""
    if data is None:
        return []
    if not isinstance(data, list) or not data or len(data) > MAX_DETAILS:
        raise ValueError(f"'details' must be a list of 1 to {MAX_DETAILS} steps")
    steps = []
    for number, step in enumerate(data, 1):
        if not isinstance(step, dict) or set(step) != {"title", "text"}:
            raise ValueError(f"details step {number} must be an object with exactly 'title' and 'text'")
        steps.append({
            "title": _text_table(step["title"], f"details step {number} title"),
            "text": _text_table(step["text"], f"details step {number} text"),
        })
    return steps


def _preview(data: dict[str, object], folder: Path) -> tuple[str | None, str | None]:
    """``preview``: the relative path of an image of the pipeline's folder, else ``(None, reason)``.

    A broken preview never makes the pipeline invalid: the field is ignored and the reason reported.
    The format is read from the first bytes, not from the extension.
    """
    if "preview" not in data or data["preview"] is None:
        return None, None
    relative = data["preview"]
    if not isinstance(relative, str) or not PREVIEW_PATH_PATTERN.match(relative) or ".." in relative.split("/"):
        return None, "preview: the path must be relative to the pipeline's folder, with '/' separators, no '..', ending in .png, .jpg or .jpeg"
    path = (folder / relative).resolve()
    if folder.resolve() not in path.parents:
        return None, "preview: the image must stay inside the pipeline's folder"
    if not path.is_file():
        return None, f"preview: no such file '{relative}' in the pipeline's folder"
    try:
        size = path.stat().st_size
        if size > MAX_PREVIEW_BYTES:
            return None, f"preview: the image is larger than {MAX_PREVIEW_BYTES // 1024} KiB"
        with path.open("rb") as handle:
            head = handle.read(8)
    except OSError as error:
        return None, f"preview: unreadable: {error}"
    if not (head.startswith(PNG_MAGIC) or head.startswith(JPEG_MAGIC)):
        return None, "preview: the file is neither a PNG nor a JPEG image"
    return relative, None


def _string(data: dict[str, object], key: str) -> str:
    value = data.get(key)
    if not isinstance(value, str) or not value.strip():
        raise ValueError(f"'{key}' must be a non-empty text")
    return value.strip()


# --- versions ---------------------------------------------------------------


def parse_version(text: str) -> tuple[int, ...]:
    """``"1.4.4"`` -> ``(1, 4, 4)``; raises ValueError when it is not dotted numbers."""
    if not VERSION_PATTERN.match(text):
        raise ValueError(f"'{text}' is not a version (dotted numbers, e.g. 1.4 or 1.4.4)")
    return tuple(int(part) for part in text.split("."))


def compare_versions(left: tuple[int, ...], right: tuple[int, ...]) -> int:
    """-1, 0 or 1; a missing component counts as 0 (``1.4`` equals ``1.4.0``)."""
    size = max(len(left), len(right))
    a = left + (0,) * (size - len(left))
    b = right + (0,) * (size - len(right))
    return (a > b) - (a < b)


def parse_specifier(spec: str) -> list[tuple[str, tuple[int, ...]]]:
    """``">=3.12,<3.15"`` -> clauses; a bare version (``"1.4"``) means ``>=``."""
    if VERSION_PATTERN.match(spec):
        return [(">=", parse_version(spec))]
    clauses = []
    for part in spec.split(","):
        found = SPEC_CLAUSE_PATTERN.match(part.strip())
        if not found:
            raise ValueError(f"'{spec}' is not a version range (clauses >=, >, <=, <, == joined by ',', e.g. \">=3.12,<3.15\")")
        clauses.append((found.group(1), parse_version(found.group(2))))
    return clauses


def satisfies(version: tuple[int, ...], spec: str) -> bool:
    for operator, bound in parse_specifier(spec):
        order = compare_versions(version, bound)
        if not {">=": order >= 0, ">": order > 0, "<=": order <= 0, "<": order < 0, "==": order == 0}[operator]:
            return False
    return True


# --- the dependency registry -------------------------------------------------


@dataclass
class RegistryEntry:
    id: str
    name: dict[str, str]
    kind: str
    min_version: str | None
    check: dict[str, object] | None
    downloads: dict[str, dict[str, object]]
    sizes: dict[str, int]
    requirements: str | None
    requirements_path: Path | None
    packages: list[tuple[str, str]]
    page: str
    licence: str

    def as_json(self, language: str | None = None) -> dict[str, object]:
        data: dict[str, object] = {"id": self.id, "name": self.name, "kind": self.kind}
        if self.min_version is not None:
            data["minVersion"] = self.min_version
        if self.check is not None:
            data["check"] = self.check
        if self.kind == "tool":
            data["downloads"] = self.downloads
        if self.sizes:
            data["sizes"] = self.sizes
        if self.requirements is not None:
            data["requirements"] = self.requirements
            data["requirementsPath"] = str(self.requirements_path)
            data["packages"] = [{"name": name, "version": version} for name, version in self.packages]
        data["page"] = self.page
        data["licence"] = self.licence
        if language:
            data["displayName"] = resolve_text(self.name, language)
        return data


@dataclass
class Dependency:
    """A dependency of one pipeline: its registry entry plus what the manifest says."""

    entry: RegistryEntry
    required: bool
    reason: dict[str, str]
    min_version: str | None  # the effective minimum: the manifest's, never below the registry's

    @property
    def id(self) -> str:
        return self.entry.id

    def as_json(self, language: str | None = None) -> dict[str, object]:
        data = self.entry.as_json(language)
        data.pop("displayName", None)
        data["required"] = self.required
        data["reason"] = self.reason
        if self.min_version is not None:
            data["minVersion"] = self.min_version
        if language:
            data["displayName"] = resolve_text(self.entry.name, language)
            data["displayReason"] = resolve_text(self.reason, language)
        return data


def parse_requirements(path: Path) -> list[tuple[str, str]]:
    """``(name, version)`` of a pinned ``requirements.txt``; every package must be
    ``==`` pinned and carry at least one ``--hash=sha256:`` digest."""
    logical: list[str] = []
    current = ""
    for raw in path.read_text(encoding="utf-8").splitlines():
        line = raw.strip()
        if not current and (not line or line.startswith("#")):
            continue
        if line.endswith("\\"):
            current += " " + line[:-1].strip()
            continue
        logical.append((current + " " + line).strip())
        current = ""
    if current:
        logical.append(current.strip())
    pins: list[tuple[str, str]] = []
    for line in logical:
        found = REQUIREMENT_PATTERN.match(line)
        if not found:
            raise ValueError(f"{path.name}: '{line[:60]}' is not a name==version pin")
        if not HASH_PATTERN.search(line):
            raise ValueError(f"{path.name}: {found.group(1)} has no --hash=sha256: digest")
        pins.append((found.group(1).lower(), found.group(2)))
    if not pins:
        raise ValueError(f"{path.name} lists no package")
    return pins


def _https_url(value: object, label: str) -> str:
    if not isinstance(value, str) or len(value) > 2000:
        raise ValueError(f"{label} must be a text")
    parts = urlsplit(value)
    if parts.scheme != "https" or not parts.hostname or parts.username or parts.password:
        raise ValueError(f"{label} '{value}' must be an https URL without credentials")
    return value


def _validate_entry(data: object, base_dir: Path) -> RegistryEntry:
    if not isinstance(data, dict):
        raise ValueError("an entry must be a JSON object")
    entry_id = _string(data, "id")
    if not ID_PATTERN.match(entry_id):
        raise ValueError(f"id '{entry_id}' must be 1-40 characters: a-z, 0-9 and '-', not starting with '-'")
    try:
        allowed = {"id", "name", "kind", "minVersion", "check", "downloads", "sizes", "requirements", "page", "licence"}
        unknown = sorted(set(data) - allowed)
        if unknown:
            raise ValueError(f"unknown field '{unknown[0]}'")
        name = _text_table(data.get("name"), "name")
        kind = data.get("kind")
        if kind not in KINDS:
            raise ValueError(f"kind '{kind}' is unknown (known: {', '.join(KINDS)})")
        page = _https_url(data.get("page"), "page")
        licence = _string(data, "licence")
        min_version = data.get("minVersion")
        check = data.get("check")
        downloads = data.get("downloads")
        sizes = _validate_sizes(data.get("sizes", {}))
        requirements = data.get("requirements")
        packages: list[tuple[str, str]] = []
        requirements_path: Path | None = None
        if kind == "tool":
            if requirements is not None:
                raise ValueError("'requirements' is only for python-packages")
            if (min_version is None) != (check is None):
                raise ValueError("'minVersion' and 'check' go together: both or neither")
            if min_version is not None:
                if not isinstance(min_version, str) or not VERSION_PATTERN.match(min_version):
                    raise ValueError("minVersion of a tool must be dotted numbers, e.g. \"1.4\"")
                check = _validate_check(check)
            downloads = _validate_downloads(downloads if downloads is not None else {})
        else:
            if downloads is not None:
                raise ValueError("a python-packages entry has no 'downloads' (the packages are installed from their pinned digests)")
            if check is not None:
                raise ValueError("a python-packages entry has no 'check' (the interpreter range and the pinned packages are checked)")
            if not isinstance(min_version, str):
                raise ValueError("minVersion of python-packages must be the Python range, e.g. \">=3.12,<3.15\"")
            parse_specifier(min_version)
            if not isinstance(requirements, str) or not requirements.strip():
                raise ValueError("python-packages needs 'requirements' (path of the pinned requirements.txt)")
            if Path(requirements).is_absolute() or "\\" in requirements:
                raise ValueError("'requirements' must be a relative path with '/' separators")
            requirements_path = (base_dir / requirements).resolve()
            if base_dir.parent.resolve() not in requirements_path.parents:
                raise ValueError("'requirements' must stay inside the stacking folder")
            if not requirements_path.is_file():
                raise ValueError(f"requirements file '{requirements}' does not exist")
            packages = parse_requirements(requirements_path)
            downloads = {}
    except ValueError as error:
        raise ValueError(f"entry '{entry_id}': {error}") from None
    return RegistryEntry(
        id=entry_id, name=name, kind=kind, min_version=min_version, check=check, downloads=downloads, sizes=sizes,
        requirements=requirements, requirements_path=requirements_path, packages=packages, page=page, licence=licence,
    )


def _validate_check(check: object) -> dict[str, object]:
    if not isinstance(check, dict) or set(check) != {"command", "args", "versionPattern"}:
        raise ValueError("check needs exactly: command, args, versionPattern")
    command = check["command"]
    names = [command] if isinstance(command, str) else command
    if not isinstance(names, list) or not names or not all(isinstance(n, str) and COMMAND_PATTERN.match(n) for n in names):
        raise ValueError("check.command must be a bare program name, or a list of names (no path, no space)")
    args = check["args"]
    if not isinstance(args, list) or tuple(args) not in VERSION_QUERIES:
        raise ValueError("check.args must be a version query only: " + ", ".join(json.dumps(list(q)) for q in VERSION_QUERIES))
    pattern = check["versionPattern"]
    if not isinstance(pattern, str) or not pattern or len(pattern) > 300:
        raise ValueError("check.versionPattern must be a regular expression (up to 300 characters)")
    try:
        compiled = re.compile(pattern)
    except re.error as error:
        raise ValueError(f"check.versionPattern is not a valid regular expression: {error}") from None
    if compiled.groups != 1:
        raise ValueError("check.versionPattern must have exactly one group (the version)")
    return {"command": command, "args": list(args), "versionPattern": pattern}


def _validate_sizes(sizes: object) -> dict[str, int]:
    """``sizes``: the bytes the person downloads by hand, or of the pinned packages, by system."""
    if not isinstance(sizes, dict):
        raise ValueError("sizes must be an object keyed by system")
    for system, size in sizes.items():
        if system not in SYSTEMS:
            raise ValueError(f"sizes: unknown system '{system}' (known: {', '.join(SYSTEMS)})")
        if not isinstance(size, int) or isinstance(size, bool) or size <= 0:
            raise ValueError(f"sizes.{system} must be a positive number of bytes")
    return dict(sizes)


def _validate_downloads(downloads: object) -> dict[str, dict[str, object]]:
    if not isinstance(downloads, dict):
        raise ValueError("downloads must be an object keyed by system")
    result: dict[str, dict[str, object]] = {}
    for system, item in downloads.items():
        if system not in SYSTEMS:
            raise ValueError(f"downloads: unknown system '{system}' (known: {', '.join(SYSTEMS)})")
        if not isinstance(item, dict) or not set(item) <= {"url", "sha256", "bytes"}:
            raise ValueError(f"downloads.{system} takes url, sha256 and optionally bytes")
        if ("url" in item) != ("sha256" in item) or "url" not in item:
            raise ValueError(f"downloads.{system}: 'url' and 'sha256' go together (a download without its digest is refused)")
        _https_url(item["url"], f"downloads.{system}.url")
        if not isinstance(item["sha256"], str) or not SHA256_PATTERN.match(item["sha256"]):
            raise ValueError(f"downloads.{system}.sha256 must be 64 lower-case hexadecimal characters")
        if "bytes" in item and (not isinstance(item["bytes"], int) or isinstance(item["bytes"], bool) or item["bytes"] <= 0):
            raise ValueError(f"downloads.{system}.bytes must be a positive integer")
        result[system] = dict(item)
    return result


def validate_registry(data: object, base_dir: Path) -> dict[str, RegistryEntry]:
    """The entries by id, or ValueError: a registry with one broken entry is rejected whole."""
    if not isinstance(data, dict) or data.get("format") != REGISTRY_FORMAT:
        raise ValueError(f"the registry must be an object with \"format\": {REGISTRY_FORMAT}")
    entries = data.get("dependencies")
    if not isinstance(entries, list) or not entries:
        raise ValueError("'dependencies' must be a non-empty list")
    registry: dict[str, RegistryEntry] = {}
    for item in entries:
        entry = _validate_entry(item, base_dir)
        if entry.id in registry:
            raise ValueError(f"id '{entry.id}' appears twice")
        registry[entry.id] = entry
    return registry


def load_registry(path: Path | None = None) -> dict[str, RegistryEntry]:
    """Read ``dependencies/registry.json``; ValueError (with the reason) when it cannot be used."""
    file = path or REGISTRY_FILE
    try:
        data = json.loads(file.read_text(encoding="utf-8"))
    except (OSError, UnicodeDecodeError) as error:
        raise ValueError(f"unreadable: {error}") from None
    except json.JSONDecodeError as error:
        raise ValueError(f"not valid JSON: {error}") from None
    return validate_registry(data, file.parent)


def registry_state(path: Path | None = None) -> tuple[dict[str, RegistryEntry], str | None]:
    """``(registry, None)``, or ``({}, reason)`` when the registry cannot be read."""
    try:
        return load_registry(path), None
    except ValueError as error:
        return {}, str(error)


def _validate_dependencies(
    data: object, requires: list[str], registry: dict[str, RegistryEntry], registry_error: str | None
) -> list[Dependency]:
    if data is None:
        return []
    if registry_error is not None:
        raise ValueError(f"the dependency registry cannot be read: {registry_error}")
    if not isinstance(data, list):
        raise ValueError("'dependencies' must be a list")
    optional_tools = {item[:-1] for item in requires if item.endswith("?")}
    result: list[Dependency] = []
    seen: set[str] = set()
    for item in data:
        if not isinstance(item, dict):
            raise ValueError("each dependency must be an object")
        extra = sorted(set(item) - set(MANIFEST_DEPENDENCY_KEYS))
        if extra:
            raise ValueError(
                f"dependency field '{extra[0]}' is not allowed in a pipeline.json (allowed: {', '.join(MANIFEST_DEPENDENCY_KEYS)}); "
                "downloads, checks and pages belong to the dependency registry"
            )
        dependency_id = _string(item, "id")
        entry = registry.get(dependency_id)
        if entry is None:
            raise ValueError(f"dependency '{dependency_id}' is not allowed: it is not in the dependency registry; {NOT_ALLOWED_HINT}")
        if dependency_id in seen:
            raise ValueError(f"dependency '{dependency_id}' is listed twice")
        seen.add(dependency_id)
        required = item.get("required")
        if not isinstance(required, bool):
            raise ValueError(f"dependency '{dependency_id}': 'required' must be true or false")
        if dependency_id in KNOWN_TOOLS and dependency_id in {r.rstrip("?") for r in requires}:
            if required == (dependency_id in optional_tools):
                raise ValueError(f"dependency '{dependency_id}': 'required' disagrees with 'requires' ('{dependency_id}?' means optional)")
        reason = _text_table(item.get("reason"), f"dependency '{dependency_id}' reason")
        minimum = entry.min_version
        if "minVersion" in item:
            wanted = item["minVersion"]
            if entry.kind != "tool" or entry.min_version is None:
                raise ValueError(f"dependency '{dependency_id}': 'minVersion' is only accepted for a checked tool")
            if not isinstance(wanted, str) or not VERSION_PATTERN.match(wanted):
                raise ValueError(f"dependency '{dependency_id}': minVersion must be dotted numbers, e.g. \"1.4.4\"")
            if compare_versions(parse_version(wanted), parse_version(entry.min_version)) < 0:
                raise ValueError(
                    f"dependency '{dependency_id}': minVersion {wanted} is lower than the registry's {entry.min_version}"
                )
            minimum = wanted
        result.append(Dependency(entry=entry, required=required, reason=reason, min_version=minimum))
    return result


def sha256_of(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def _read_script(relative: str, folder: Path) -> tuple[Path, str]:
    """The script of a ``script:`` entry and its SHA-256, or ValueError."""
    if not SCRIPT_PATH_PATTERN.match(relative) or ".." in relative.split("/"):
        raise ValueError(
            f"entry 'script:{relative}': the script must be a relative path of the pipeline's folder, "
            "ending in .py, with '/' separators and no '..'"
        )
    path = (folder / relative).resolve()
    if folder.resolve() not in path.parents:
        raise ValueError(f"entry 'script:{relative}': the script must stay inside the pipeline's folder")
    if not path.is_file():
        raise ValueError(f"entry 'script:{relative}': no such file in the pipeline's folder")
    if path.stat().st_size > MAX_SCRIPT_BYTES:
        raise ValueError(f"entry 'script:{relative}': the script is larger than {MAX_SCRIPT_BYTES // 1024} KiB")
    return path, sha256_of(path.read_bytes())


def validate(
    data: object,
    folder_name: str,
    source: str,
    folder: Path,
    renders: set[str],
    registry: dict[str, RegistryEntry] | None = None,
    registry_error: str | None = None,
) -> Pipeline:
    """Build a Pipeline from parsed JSON, or raise ValueError with the reason."""
    if not isinstance(data, dict):
        raise ValueError("the file must contain a JSON object")
    pipeline_id = _string(data, "id")
    if not ID_PATTERN.match(pipeline_id):
        raise ValueError(f"id '{pipeline_id}' must be 1-40 characters: a-z, 0-9 and '-', not starting with '-'")
    if pipeline_id in RESERVED_IDS:
        raise ValueError(f"id '{pipeline_id}' is reserved: it names the option 'no processing', which runs no pipeline")
    if pipeline_id != folder_name:
        raise ValueError(f"id '{pipeline_id}' differs from the folder name '{folder_name}'")
    version = _string(data, "version")
    if version not in SUPPORTED_VERSIONS:
        raise ValueError(f"format version '{version}' is not supported (known: {', '.join(SUPPORTED_VERSIONS)})")
    name = _text_table(data.get("name"), "name")
    description = _text_table(data.get("description"), "description")
    filter_name = _string(data, "filter")
    if not FILTER_PATTERN.match(filter_name):
        raise ValueError(f"filter '{filter_name}' must be lower-case words joined by '-', e.g. 'none' or 'dual-band'")
    author = _string(data, "author")
    licence = _string(data, "licence")
    entry = _string(data, "entry")
    script_path: Path | None = None
    script_sha256: str | None = None
    if entry.startswith(SCRIPT_PREFIX):
        script_path, script_sha256 = _read_script(entry[len(SCRIPT_PREFIX):], folder)
    else:
        match = ENTRY_PATTERN.match(entry)
        if not match or match.group(1) != RENDERS_MODULE:
            raise ValueError(
                f"entry '{entry}' must be '{RENDERS_MODULE}:<function>' (a render shipped with Astro Sync) "
                f"or '{SCRIPT_PREFIX}<relative path>' (a Python file of the pipeline's folder)"
            )
        if match.group(2) not in renders:
            raise ValueError(f"entry '{entry}': no shipped render function named '{match.group(2)}'")
    requires = data.get("requires")
    if not isinstance(requires, list) or not requires or not all(isinstance(item, str) for item in requires):
        raise ValueError("'requires' must be a list of tool names, e.g. [\"siril\", \"starnet2?\"]")
    seen_tools = set()
    for item in requires:
        found = REQUIRES_PATTERN.match(item)
        if not found or found.group(1) not in KNOWN_TOOLS:
            raise ValueError(f"requires '{item}': known tools are {', '.join(KNOWN_TOOLS)} ('?' after the name = optional)")
        if found.group(1) in seen_tools:
            raise ValueError(f"requires lists '{found.group(1)}' twice")
        seen_tools.add(found.group(1))
    if "siril" not in requires:
        raise ValueError("'requires' must include \"siril\" (the stacking always runs it)")
    outputs = data.get("outputs")
    if not isinstance(outputs, list) or not outputs or any(item not in KNOWN_OUTPUTS for item in outputs):
        raise ValueError(f"'outputs' must be a non-empty list of {', '.join(KNOWN_OUTPUTS)}")
    if len(set(outputs)) != len(outputs) or "tif" not in outputs:
        raise ValueError("'outputs' must not repeat a format and must include \"tif\" (the image that is sent)")
    dependencies = _validate_dependencies(data.get("dependencies"), list(requires), registry or {}, registry_error)
    details = _details(data.get("details"))
    preview, preview_issue = _preview(data, folder)
    return Pipeline(
        id=pipeline_id, version=version, name=name, description=description, filter=filter_name,
        author=author, licence=licence, entry=entry, requires=list(requires), outputs=list(outputs),
        source=source, folder=folder, dependencies=dependencies,
        script_path=script_path, script_sha256=script_sha256, details=details,
        preview=preview, preview_issue=preview_issue,
    )


def load_folder(
    root: Path,
    source: str,
    renders: set[str],
    registry: dict[str, RegistryEntry] | None = None,
    registry_error: str | None = None,
) -> tuple[list[Pipeline], list[dict[str, str]]]:
    """Every ``<root>/<id>/pipeline.json``; a missing ``root`` is simply empty."""
    found: list[Pipeline] = []
    problems: list[dict[str, str]] = []
    if not root.is_dir():
        return found, problems
    # The default pipeline comes first (it is the one preselected), then by id.
    for folder in sorted((path for path in root.iterdir() if path.is_dir() and not path.name.startswith(".")),
                         key=lambda path: (path.name != DEFAULT_PIPELINE, path.name)):
        file = folder / DEFINITION_FILE
        if not file.is_file():
            problems.append({"folder": folder.name, "source": source, "reason": f"no {DEFINITION_FILE}"})
            continue
        try:
            if file.stat().st_size > MAX_FILE_BYTES:
                raise ValueError(f"{DEFINITION_FILE} is larger than {MAX_FILE_BYTES // 1024} KiB")
            data = json.loads(file.read_text(encoding="utf-8"))
            found.append(validate(data, folder.name, source, folder, renders, registry, registry_error))
        except (OSError, UnicodeDecodeError) as error:
            problems.append({"folder": folder.name, "source": source, "reason": f"unreadable: {error}"})
        except json.JSONDecodeError as error:
            problems.append({"folder": folder.name, "source": source, "reason": f"not valid JSON: {error}"})
        except ValueError as error:
            problems.append({"folder": folder.name, "source": source, "reason": str(error)})
    return found, problems


def load_pipelines(
    user_dirs: list[Path] | None = None,
    shipped_dir: Path | None = None,
    registry: tuple[dict[str, RegistryEntry], str | None] | None = None,
) -> tuple[dict[str, Pipeline], list[dict[str, str]]]:
    """The usable pipelines by id (shipped first), and the definitions that were ignored.

    A user definition cannot replace a shipped one, nor one found earlier in
    the folder list: the first definition of an id wins and the later one is
    reported.  ``registry`` is ``registry_state()``'s result (read when omitted).
    """
    renders = shipped_render_names()
    entries, registry_error = registry if registry is not None else registry_state()
    pipelines: dict[str, Pipeline] = {}
    problems: list[dict[str, str]] = []
    roots = [(shipped_dir or SHIPPED_DIR, "shipped")] + [(path, "user") for path in (user_dirs or [])]
    for root, source in roots:
        found, issues = load_folder(root, source, renders, entries, registry_error)
        problems += issues
        for pipeline in found:
            if pipeline.id in pipelines:
                problems.append({"folder": pipeline.folder.name, "source": source,
                                 "reason": f"id '{pipeline.id}' is already defined by a {pipelines[pipeline.id].source} pipeline"})
            else:
                pipelines[pipeline.id] = pipeline
    return pipelines, problems


def listing(
    pipelines: dict[str, Pipeline],
    problems: list[dict[str, str]],
    language: str | None = None,
    registry: tuple[dict[str, RegistryEntry], str | None] | None = None,
) -> dict[str, object]:
    """The object printed by ``list-pipelines --json`` (and embedded by ``check_environment.py``)."""
    entries, registry_error = registry if registry is not None else registry_state()
    result: dict[str, object] = {
        "format": 1,
        "pipelines": [pipeline.as_json(language) for pipeline in pipelines.values()],
        "ignored": problems,
        "registry": [entry.as_json(language) for entry in entries.values()],
    }
    if registry_error is not None:
        result["registryError"] = registry_error
    return result


def add_arguments(parser: argparse.ArgumentParser) -> None:
    parser.add_argument("--pipelines-dir", type=Path, action="append", default=[], metavar="DIR",
                        help="a folder of <id>/pipeline.json definitions to read after the shipped ones (repeatable)")
    parser.add_argument("--lang", metavar="CODE", help="add displayName and displayDescription in this language (fallback: en)")
    parser.add_argument("--json", action="store_true", help="print one JSON object")


def run_listing(args: argparse.Namespace) -> int:
    registry = registry_state()
    pipelines, problems = load_pipelines(args.pipelines_dir, registry=registry)
    result = listing(pipelines, problems, args.lang, registry)
    if args.json:
        print(json.dumps(result, ensure_ascii=False, indent=2))
    else:
        for pipeline in pipelines.values():
            print(f"{pipeline.id:<24} {pipeline.text('name', args.lang)}  [{pipeline.source}, filter {pipeline.filter}]")
        for problem in problems:
            print(f"ignored {problem['folder']} ({problem['source']}): {problem['reason']}", file=sys.stderr)
    return 0


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    add_arguments(parser)
    return run_listing(parser.parse_args(argv))


if __name__ == "__main__":
    sys.exit(main())
