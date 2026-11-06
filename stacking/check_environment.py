#!/usr/bin/env python3
"""Check the stacking environment: Python, each pinned package, Siril, StarNet2, GraXpert.

Each package is imported, its version compared with ``requirements.in`` and
exercised by one small call.  Siril, StarNet2 and GraXpert are looked up on PATH
(or at the paths given) and asked for their version.  The pipeline definitions
(shipped and ``--pipelines-dir``) are read and listed, with the ones that were
ignored and why.  The dependencies the pipelines declare are looked up in the
dependency registry and each one's ``check`` is evaluated: ``present``,
``missing`` or ``outdated`` (``dependencies`` in the JSON).  Nothing is written
anywhere.

``--json`` prints one JSON object; the exit code is 0 when everything the
stacking needs is present (StarNet2 and GraXpert are optional), 1 otherwise.
"""

from __future__ import annotations

import argparse
import importlib
import importlib.metadata
import json
import re
import shutil
import subprocess
import sys
from pathlib import Path

import pipeline_defs

# The range the pinned packages really support (checked on 3.12 and 3.13, see
# DEPENDENCIES.md): numpy, scipy, tifffile and imagecodecs need 3.12 or later, and
# sep (the star finder of astroalign) has no wheel for 3.14.
PYTHON_MIN = (3, 12)
PYTHON_BELOW = (3, 14)


def pinned_versions() -> dict[str, str]:
    pins = {}
    for line in (Path(__file__).resolve().parent / "requirements.in").read_text(encoding="utf-8").splitlines():
        line = line.split("#", 1)[0].strip()
        if "==" in line:
            name, version = line.split("==", 1)
            pins[name.strip().lower()] = version.strip()
    return pins


def smoke(name: str) -> None:
    """One small, real call per package; raises when the package is unusable."""
    import numpy as np

    if name == "numpy":
        assert np.median(np.arange(5, dtype=np.float32)) == 2
    elif name == "astropy":
        from astropy.io import fits
        from astropy.stats import sigma_clip

        hdu = fits.PrimaryHDU(np.zeros((4, 4), np.float32))
        assert hdu.header["NAXIS"] == 2
        assert sigma_clip(np.array([1.0, 1.0, 1.0, 50.0]), sigma=1.5).mask[-1]
    elif name == "scipy":
        from scipy.ndimage import gaussian_filter
        from scipy.signal import fftconvolve

        assert gaussian_filter(np.ones((8, 8)), 1.0).shape == (8, 8)
        assert fftconvolve(np.ones(4), np.ones(2)).size == 5
    elif name == "scikit-image":
        from skimage.color import hsv2rgb, rgb2hsv

        rgb = np.array([[[0.2, 0.4, 0.6]]])
        assert np.allclose(hsv2rgb(rgb2hsv(rgb)), rgb)
    elif name == "tifffile":
        import io

        import tifffile

        buffer = io.BytesIO()
        tifffile.imwrite(buffer, np.zeros((4, 4, 3), np.uint16), photometric="rgb", compression="deflate")
        buffer.seek(0)
        assert tifffile.imread(buffer).shape == (4, 4, 3)
    elif name == "pillow":
        from PIL import Image

        assert Image.fromarray(np.zeros((4, 4, 3), np.uint8), "RGB").size == (4, 4)
    elif name == "astroalign":
        import astroalign  # it finds the stars with sep: both are exercised

        yy, xx = np.mgrid[0:256, 0:256]
        rng = np.random.default_rng(1)
        stars = [(int(y), int(x)) for y, x in rng.uniform(20, 236, (15, 2))]
        image = sum(np.exp(-((yy - y) ** 2 + (xx - x) ** 2) / 4.0) * (1 + i / 4) for i, (y, x) in enumerate(stars))
        image = image + rng.normal(0, 0.01, image.shape)
        transform, _ = astroalign.find_transform(np.roll(image, (3, -2), axis=(0, 1)), image)
        assert np.allclose(transform.translation, (2, -3), atol=0.1)
    elif name == "imagecodecs":
        import imagecodecs

        data = imagecodecs.lzw_encode(b"astro sync astro sync")
        assert imagecodecs.lzw_decode(data) == b"astro sync astro sync"
    else:
        importlib.import_module(name.replace("-", "_"))


DIRECT = ("numpy", "astropy", "scipy", "scikit-image", "tifffile", "pillow", "imagecodecs", "astroalign")


def tool_version(executable: Path, flag: str) -> str | None:
    try:
        result = subprocess.run([str(executable), flag], capture_output=True, text=True, timeout=30)
    except (OSError, subprocess.TimeoutExpired):
        return None
    for line in (result.stdout + result.stderr).splitlines():
        lowered = line.lower()
        if lowered.startswith(("siril", "starnet2")) and any(char.isdigit() for char in line):
            return line.strip()
        if "graxpert version" in lowered:  # GraXpert prints it inside a log line
            return line[lowered.index("graxpert version"):].strip()
    return None


def find(given: str | None, names: tuple[str, ...]) -> Path | None:
    if given:
        return Path(given) if Path(given).is_file() else None
    for name in names:
        found = shutil.which(name)
        if found:
            return Path(found)
    return None


def probe_tool(entry: pipeline_defs.RegistryEntry, given: Path | None) -> tuple[str | None, str]:
    """Run the registry's version query: ``(version or None, detail)``.

    The command is a bare name looked up on PATH (or the path the caller gave
    for Siril and StarNet2); its arguments are limited to a version query by
    the registry validation.
    """
    check = entry.check or {}
    commands = check["command"] if isinstance(check["command"], list) else [check["command"]]
    pattern = re.compile(str(check["versionPattern"]))
    candidates = [given] if given else [Path(found) for found in (shutil.which(name) for name in commands) if found]
    if not candidates:
        return None, "not found"
    for executable in candidates:
        try:
            result = subprocess.run([str(executable), *check["args"]], capture_output=True, text=True, timeout=30)
        except (OSError, subprocess.TimeoutExpired):
            continue
        found = pattern.search(result.stdout + "\n" + result.stderr)
        if found:
            return found.group(1), "ok"
    return None, "no readable version"


def probe_python_packages(entry: pipeline_defs.RegistryEntry) -> tuple[str, list[str], list[str]]:
    """``(interpreter version, packages not installed, packages at another version)``."""
    missing, other = [], []
    for name, pinned in entry.packages:
        try:
            if importlib.metadata.version(name) != pinned:
                other.append(name)
        except importlib.metadata.PackageNotFoundError:
            missing.append(name)
    return ".".join(str(part) for part in sys.version_info[:3]), missing, other


def evaluate_dependencies(
    pipelines: dict[str, pipeline_defs.Pipeline], given: dict[str, Path | None] | None = None
) -> list[dict[str, object]]:
    """The state of every dependency the pipelines declare, from the registry's ``check``.

    One row per pipeline and dependency: ``state`` is ``present``, ``missing``
    (nothing ran, or no version could be read, or a package is not installed),
    ``outdated`` (a version was read but is below the effective minimum or
    outside the range) or ``not-checked`` (the registry has no detection rule
    for it).  A tool is probed once however many pipelines declare it.
    """
    given = given or {}
    cache: dict[str, tuple] = {}
    rows: list[dict[str, object]] = []
    for pipeline in pipelines.values():
        for dependency in pipeline.dependencies:
            entry = dependency.entry
            row: dict[str, object] = {
                "pipeline": pipeline.id, "id": entry.id, "kind": entry.kind, "required": dependency.required,
                "minVersion": dependency.min_version, "state": "not-checked", "version": None, "detail": "",
            }
            if entry.kind == "python-packages":
                if entry.id not in cache:
                    cache[entry.id] = probe_python_packages(entry)
                version, missing, other = cache[entry.id]
                row["version"] = version
                if not pipeline_defs.satisfies(pipeline_defs.parse_version(version), str(entry.min_version)):
                    row["state"], row["detail"] = "outdated", f"Python {version} is outside {entry.min_version}"
                elif missing:
                    row["state"], row["detail"] = "missing", "not installed: " + ", ".join(missing)
                elif other:
                    row["state"], row["detail"] = "outdated", "another version than the pin: " + ", ".join(other)
                else:
                    row["state"] = "present"
            elif entry.check is not None:
                if entry.id not in cache:
                    cache[entry.id] = probe_tool(entry, given.get(entry.id))
                version, note = cache[entry.id]
                row["version"] = version
                if version is None:
                    row["state"], row["detail"] = "missing", note
                elif pipeline_defs.compare_versions(pipeline_defs.parse_version(version), pipeline_defs.parse_version(str(dependency.min_version))) < 0:
                    row["state"], row["detail"] = "outdated", f"{version} is older than {dependency.min_version}"
                else:
                    row["state"] = "present"
            else:
                row["detail"] = "no detection rule in the registry"
            rows.append(row)
    return rows


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--siril")
    parser.add_argument("--starnet")
    parser.add_argument("--graxpert")
    parser.add_argument("--pipelines-dir", type=Path, action="append", default=[], metavar="DIR",
                        help="a folder of <id>/pipeline.json definitions to list after the shipped ones (repeatable)")
    parser.add_argument("--json", action="store_true")
    args = parser.parse_args()

    pins = pinned_versions()
    report: dict[str, object] = {
        "python": {"version": sys.version.split()[0], "ok": PYTHON_MIN <= sys.version_info[:2] < PYTHON_BELOW,
                   "supported": ">=3.12,<3.14"},
        "packages": [],
    }
    ok = bool(report["python"]["ok"])
    for name, wanted in pins.items():
        entry: dict[str, object] = {"name": name, "pinned": wanted}
        try:
            entry["installed"] = importlib.metadata.version(name)
            if name in DIRECT:
                smoke(name)
                entry["smoke"] = "ok"
            entry["ok"] = entry["installed"] == wanted
        except Exception as error:  # report, never crash: this is a checker
            entry["ok"] = False
            entry["error"] = f"{type(error).__name__}: {error}"
        ok = ok and bool(entry["ok"])
        report["packages"].append(entry)

    siril = find(args.siril, ("siril-cli", "siril"))
    starnet = find(args.starnet, ("starnet2",))
    report["siril"] = {"path_found": siril is not None, "version": tool_version(siril, "--version") if siril else None}
    report["starnet2"] = {"path_found": starnet is not None, "version": tool_version(starnet, "--version") if starnet else None,
                          "optional": True}
    graxpert = find(args.graxpert, ("graxpert", "GraXpert"))
    report["graxpert"] = {"path_found": graxpert is not None, "version": tool_version(graxpert, "--version") if graxpert else None,
                          "optional": True}
    ok = ok and siril is not None and bool(report["siril"]["version"])
    registry = pipeline_defs.registry_state()
    pipelines, ignored = pipeline_defs.load_pipelines(args.pipelines_dir, registry=registry)
    report["pipelines"] = pipeline_defs.listing(pipelines, ignored, registry=registry)
    report["dependencies"] = evaluate_dependencies(
        pipelines, {"siril": Path(args.siril) if args.siril else None, "starnet2": Path(args.starnet) if args.starnet else None,
                    "graxpert": Path(args.graxpert) if args.graxpert else None})
    # A broken definition added by the user is reported, never a reason to fail:
    # the environment is fine; only the shipped definitions must load.
    ok = ok and not any(item["source"] == "shipped" for item in ignored)
    report["ok"] = ok

    if args.json:
        print(json.dumps(report, indent=2, ensure_ascii=False))
    else:
        print(f"Python {report['python']['version']} ({'ok' if report['python']['ok'] else 'need 3.12 or 3.13'})")
        for entry in report["packages"]:
            state = "ok" if entry["ok"] else f"PROBLEM {entry.get('error', '')}".strip()
            print(f"  {entry['name']:<20} {entry.get('installed', '-'):<22} pinned {entry['pinned']:<22} {state}")
        print(f"Siril: {report['siril']['version'] or 'not found'}")
        print(f"StarNet2 (optional): {report['starnet2']['version'] or 'not found'}")
        print(f"GraXpert (optional): {report['graxpert']['version'] or 'not found'}")
        print("Pipelines: " + (", ".join(item["id"] for item in report["pipelines"]["pipelines"]) or "none"))
        for item in ignored:
            print(f"  ignored {item['folder']} ({item['source']}): {item['reason']}")
        for row in report["dependencies"]:
            print(f"  {row['pipeline']}: {row['id']:<12} {row['state']:<12} {row['version'] or '-'} {row['detail']}".rstrip())
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
