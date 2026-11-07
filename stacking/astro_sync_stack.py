#!/usr/bin/env python3
"""Astro Sync stacking: one target's FITS (or camera RAW) lights into a stacked, rendered image.

Camera RAW files (``.cr2``, ``.cr3``, ``.nef``, ``.arw``, ``.dng``, ``.raf``, ``.pef``, ``.orf``,
``.rw2``) are given to Siril as they are, lights and calibration frames alike: its ``convert`` reads them
(LibRaw) into one-layer FITS that keep the sensor's mosaic, so the steps below are those of a
one-shot-colour FITS night (calibration with ``-cfa -equalize_cfa -debayer``, or ``convert -debayer`` when
there is no calibration frame). A folder that mixes RAW files and FITS frames is refused
(``mixed_formats``).

The recipe (see README.md):

1. Siril builds the calibration masters (dark-flats, biases, flats, darks) and
   calibrates the lights (cosmetic correction from the dark, CFA equalisation
   and debayering for a colour sensor);
2. the WCS is removed from the calibrated *working copies* only (Siril 1.4
   with OpenCV 5 fails on registered SIP/WCS copies);
3. Siril registers in two passes with an affine transform, then the field
   rotation the registration measured chooses the framing
   (``registration_framing.py``): the minimal common area when the field hardly
   turns (equatorial mount), the whole field of the reference frame when it
   turns progressively (alt-azimuth mount: the common area of such a night is
   empty).  The transform is applied with Lanczos4, and the frames
   are stacked with a winsorised 3-sigma rejection, additive-with-scale
   normalisation and wFWHM weighting into a 32-bit linear master.  A frame
   Siril cannot match is left out and named in the provenance.  When the field
   turns, ``night_framing.py`` measures the frames on offer from the transforms
   (five notches, from the area every light covers to the whole field) and
   ``--frame-size RANK`` cuts the master to one of them before the renders; the
   master of the whole field stays in ``--out`` as ``<name>_field.fit``, and a
   later run on the same poses cuts it again without stacking (``field_reused``);
4. ``renders.py`` renders each chosen pipeline (``--pipeline <id>``, repeatable,
   default ``raw``): ``raw`` (the sensor's colours), ``dualband-correction``
   (the colours of a dual-band capture brought back towards natural tones),
   ``polished`` (the natural render pushed further, its background and
   denoising by GraXpert when it is found: ``--graxpert``, or by Siril alone with
   ``--polished-engine siril``, which is also the fallback; the SCNR is skipped
   on a dual-band capture, see ``--capture-filter``) and ``hoo`` (the HOO palette
   of a dual-band capture), each as a 16-bit TIFF plus an 8-bit PNG.  The pipelines are described by
   ``pipelines/<id>/pipeline.json`` (format: ``pipelines/SCHEMA.md``), read from
   the shipped folder and from every ``--pipelines-dir``.  A pipeline may bring
   its own ``render(nebula, stars)`` script; when the person added it, it runs
   only with ``--trusted <id>=<sha256 of the script>``.
   ``--linear-only`` (the option "No processing") runs **no** render: step 4
   is replaced by ``linear_export.py``, which writes the linear master of
   step 3, not stretched, as ``<name>_linear.tif`` (16-bit); it takes no
   ``--pipeline`` and needs neither StarNet2 nor the render libraries.
   ``--linear`` ("No processing" chosen beside renders) writes
   that same linear image too, after the renders.

``--from-master <file>`` is the stacking **not made**: the file is
the image a device already stacked (a Seestar's ``Stacked_*.fit``: a linear 16-bit
colour cube).  Nothing is calibrated, registered or stacked; ``device_master.py``
scales that image to the 0..1 floats of Siril's master (``masters/stack_linear.fit``
in the work folder) and the steps that follow are the ones of a stacking of the
poses -- the renders, the contrast, ``--linear-only`` -- unchanged.  It replaces
``--lights``, ``--frames-from`` and the calibration folders (none can be given with
it), keeps no ``--integration`` master (the image of a multi-night plan is made
from poses), does not need Siril (unless the "Polished render" falls back on it), and
measures no quality by itself (the quality of the image of a device is measured on
request, ``star_quality.py --measure``).  Its provenance names it (``deviceMaster``).

``--integration <family>`` keeps the linear master of step 3 as
``<family>_integration.fit`` in ``--out`` (``integration_master.py``): the
only intermediate kept, the one a multi-night plan combines later
(``plan_stack.py``). It is announced by an ``integration`` event.

What it reads: the FITS under ``--lights`` (or only those named by
``--frames-from``, one family of exposures: Astro Sync never stacks two
filters, exposures or gains together) and the optional calibration
folders, never modified.  What it writes: a scratch tree under ``--work``
(created by this script, marked, removed at the end unless ``--keep-work``),
and the rendered files plus ``<name>_stacking.json`` under ``--out``.  It
refuses an ``--out`` named ``final`` and any ``--work`` or ``--out`` inside an
input folder.  Nothing leaves the computer.

``--json`` prints one JSON object per line on stdout (events ``plan``,
``step``, ``registration`` (the field rotation measured, the framing chosen,
the frame of the stacking with its notches (``frame``) and
the frames left out, once the frames are registered; ``unit`` says what the
frames are: ``lights``, or ``masters`` when ``plan_stack.py`` registers the nights
of a plan), ``output``, ``done``, ``error``) for Astro Sync's runner;
``--dry-run`` prints the plan and writes nothing.  Exit codes: 0 done,
2 bad input, 3 tool missing, 4 tool failed, 130 cancelled.  Error codes of
the ``error`` event: ``folder_missing``, ``unknown_pipeline``,
``script_not_trusted``, ``frames_outside_lights``, ``too_few_lights``, ``output_inside_input``, ``output_is_final``, ``work_not_empty``, ``bad_options``,
``mixed_formats``, ``siril_missing``, ``starnet_missing`` (only when ``--starnet`` names a
missing file), ``graxpert_missing`` (only when ``--graxpert`` names a missing
file), ``calibration_failed``, ``registration_failed``,
``integration_failed``, ``starnet_failed``, ``render_failed``,
``script_failed`` (an added script that cannot load), ``device_unreadable`` (the
image of ``--from-master`` is not a readable colour cube), ``device_stretched`` (it is
not linear), ``cancelled``.

``astro_sync_stack.py list-pipelines [--pipelines-dir DIR]... [--lang CODE]
--json`` prints the pipeline definitions that were loaded (and the ones that
were ignored, with the reason) and exits; it needs no input folder and no
image library.
"""

from __future__ import annotations

import argparse
import contextlib
import hashlib
import json
import os
import re
import shutil
import signal
import subprocess
import sys
import time
import types
from dataclasses import dataclass, field
from pathlib import Path

import device_master
import night_framing
import overscan
import pipeline_defs
import raw_files
import registration_framing
import render_finish
import siril_config

FORMAT_VERSION = 1
FITS_SUFFIXES = {".fit", ".fits", ".fts"}
# The poses Siril reads: FITS frames, and camera RAW files.
FRAME_SUFFIXES = FITS_SUFFIXES | raw_files.RAW_SUFFIXES
KINDS = ("dark-flats", "biases", "flats", "darks", "lights")
# What the frames of a registration are (the ``unit`` of the ``registration`` event): the lights of a night, or the masters of a plan.
LIGHTS, MASTERS = "lights", "masters"
MARKER = ".astro-sync-stacking"
DEFAULT_PIPELINE = pipeline_defs.DEFAULT_PIPELINE
LINEAR_PIPELINE = pipeline_defs.LINEAR_PIPELINE
WCS_PREFIXES = ("CTYPE", "CUNIT", "CRPIX", "CRVAL", "CDELT", "PC", "CD", "PV", "A_", "B_", "AP_", "BP_")
WCS_EXACT = {"WCSAXES", "LONPOLE", "LATPOLE", "RADESYS", "EQUINOX", "A_ORDER", "B_ORDER", "AP_ORDER", "BP_ORDER"}

EXIT_OK, EXIT_INPUT, EXIT_TOOL_MISSING, EXIT_TOOL_FAILED, EXIT_CANCELLED = 0, 2, 3, 4, 130


class StackingError(Exception):
    def __init__(self, code: str, message: str, exit_code: int) -> None:
        super().__init__(message)
        self.code = code
        self.exit_code = exit_code


class Cancelled(Exception):
    pass


@dataclass
class Plan:
    frames: dict[str, list[Path]]
    colour: bool
    pipelines: list[pipeline_defs.Pipeline]
    steps: list[str] = field(default_factory=list)
    # "No processing": no pipeline, the linear stack is the output.
    linear_only: bool = False
    # "No processing" beside renders: the linear stack is written too, after them.
    with_linear: bool = False
    # The image a device already stacked: nothing is stacked, the renders start from it.
    from_master: Path | None = None
    # The lights are Fujifilm X-Trans RAW files: debayered at the conversion, not at the calibration.
    xtrans: bool = False


class Reporter:
    def __init__(self, as_json: bool) -> None:
        self.as_json = as_json

    def emit(self, event: str, **fields: object) -> None:
        if self.as_json:
            print(json.dumps({"event": event, **fields}, ensure_ascii=False), flush=True)
        elif event == "step":
            print(f"[{fields['index']}/{fields['total']}] {fields['step']}", flush=True)
        elif event == "error":
            print(f"error ({fields['code']}): {fields['message']}", file=sys.stderr, flush=True)
        elif event == "registration":
            rotation = fields["fieldRotationDegrees"]
            print(f"registration: {fields['registered']} of {fields['frames']} frames aligned, "
                  f"field rotation {'not measured' if rotation is None else f'{rotation}°'}, framing {fields['framing']}", flush=True)
        elif event == "note":
            print(f"note: {fields['message']}", flush=True)
        elif event == "output":
            print(f"wrote {fields['file']} ({fields['bytes']} bytes, sha256 {fields['sha256']})", flush=True)
        elif event in ("plan", "done"):
            print(json.dumps({"event": event, **fields}, ensure_ascii=False, indent=2), flush=True)


def fits_files(folder: Path | None) -> list[Path]:
    """The frames of a folder: FITS files and camera RAW files, sorted; hidden entries are left out."""
    if folder is None:
        return []
    if not folder.is_dir():
        raise StackingError("folder_missing", f"not a folder: {folder}", EXIT_INPUT)
    found = [
        path for path in folder.rglob("*")
        if path.suffix.lower() in FRAME_SUFFIXES
        and path.is_file()
        and not any(part.startswith(".") for part in path.relative_to(folder).parts)
    ]
    return sorted(found)


def is_inside(child: Path, parent: Path) -> bool:
    try:
        child.resolve().relative_to(parent.resolve())
        return True
    except ValueError:
        return False


def listed_inside(path: Path, lights: Path) -> bool:
    """Whether a listed frame is in ``--lights``: by its path as listed, so a frame that is a link to a file elsewhere
 (the poses "Choose the treatments…" links into the night) is inside, as the folder's own walk finds it;
    a file that merely resolves there, or a path that climbs out of the folder, is judged on its resolved place."""
    try:
        Path(os.path.abspath(path)).relative_to(os.path.abspath(lights))
        return ".." not in path.parts
    except ValueError:
        return is_inside(path, lights)


def listed_frames(list_file: Path, lights: Path) -> list[Path]:
    """The light frames named by ``--frames-from`` (one path per line), each a FITS file inside ``--lights``."""
    try:
        lines = list_file.read_text(encoding="utf-8").splitlines()
    except OSError as error:
        raise StackingError("folder_missing", f"cannot read {list_file}: {error}", EXIT_INPUT) from error
    frames = []
    for line in lines:
        if not line.strip():
            continue
        path = Path(line.strip())
        if path.suffix.lower() not in FRAME_SUFFIXES or not path.is_file() or not listed_inside(path, lights):
            raise StackingError("frames_outside_lights", f"{path} is not a FITS or RAW light frame inside {lights}", EXIT_INPUT)
        frames.append(path)
    return sorted(set(frames))


def is_colour(first_light: Path) -> bool:
    if raw_files.is_raw(first_light):
        return True  # a camera RAW file is a mosaic (Siril debayers it); a monochrome RAW camera is not met
    from astropy.io import fits

    header = fits.getheader(first_light, memmap=False)
    return int(header.get("NAXIS", 2)) == 2 and bool(str(header.get("BAYERPAT", "")).strip())


def flats_filters(flats: list[Path]) -> list[str]:
    """The distinct ``FILTER`` values of the flats stacked, sorted.

    A family is stacked with the flats of its own filter; this is written in ``<name>_stacking.json``
    (``flatsFilters``) so that a master flat made of several filters can be seen afterwards.  A flat
    that cannot be read, or that names no filter, adds nothing.
    """
    from astropy.io import fits

    names: set[str] = set()
    for path in flats:
        try:
            name = str(fits.getheader(path, memmap=False).get("FILTER", "")).strip()
        except (OSError, ValueError):
            continue
        if name:
            names.add(name)
    return sorted(names)


def trusted_scripts(items: list[str] | None) -> dict[str, str]:
    """``--trusted <id>=<sha256>`` (repeatable) as ``{id: sha256}``; ValueError on a malformed item."""
    trusted: dict[str, str] = {}
    for item in items or []:
        pipeline_id, _, digest = item.partition("=")
        if not pipeline_defs.ID_PATTERN.match(pipeline_id) or not pipeline_defs.SHA256_PATTERN.match(digest):
            raise ValueError(f"--trusted expects <pipeline id>=<64 hexadecimal characters of the script's SHA-256>, got '{item}'")
        trusted[pipeline_id] = digest
    return trusted


def check_script_trust(pipeline: pipeline_defs.Pipeline, trusted: dict[str, str]) -> None:
    """A script of a pipeline the person added runs only when its digest was approved.

    A pipeline shipped with Astro Sync (its folder is part of the app, put there
    by a reviewed pull request) runs without question.  For the others the
    caller lists the approved digests with ``--trusted <id>=<sha256>``; a
    missing or different digest stops the run (exit code 2).
    """
    if pipeline.script_path is None or pipeline.source == "shipped":
        return
    approved = trusted.get(pipeline.id)
    if approved is None:
        raise StackingError(
            "script_not_trusted",
            f"pipeline '{pipeline.id}' brings its own script ({pipeline.script_sha256}) and was added by the person: "
            f"it is not run until it is trusted (--trusted {pipeline.id}=<sha256>)",
            EXIT_INPUT,
        )
    if approved != pipeline.script_sha256:
        raise StackingError(
            "script_not_trusted",
            f"the script of pipeline '{pipeline.id}' is not the one that was trusted "
            f"(trusted {approved}, found {pipeline.script_sha256}): it changed since, ask again",
            EXIT_INPUT,
        )


def load_script_render(pipeline: pipeline_defs.Pipeline, trusted: dict[str, str]):
    """The ``render(nebula, stars)`` function of the pipeline's script.

    The file is read **once**: the digest that is checked is the digest of the
    bytes that are compiled and run, so a file replaced after the first check
    cannot slip in.
    """
    assert pipeline.script_path is not None
    source = pipeline.script_path.read_bytes()
    digest = pipeline_defs.sha256_of(source)
    if digest != pipeline.script_sha256:
        raise StackingError("script_not_trusted", f"the script of pipeline '{pipeline.id}' changed while it was being loaded", EXIT_INPUT)
    check_script_trust(pipeline, trusted)
    module = types.ModuleType(f"pipeline_script_{re.sub('[^a-z0-9]', '_', pipeline.id)}")
    module.__file__ = str(pipeline.script_path)
    try:
        exec(compile(source, str(pipeline.script_path), "exec"), module.__dict__)
    except Exception as error:  # a user script may fail in any way
        raise StackingError("script_failed", f"the script of pipeline '{pipeline.id}' failed to load: {error}", EXIT_TOOL_FAILED) from error
    function = getattr(module, "render", None)
    if not callable(function):
        raise StackingError("script_failed", f"the script of pipeline '{pipeline.id}' defines no render(nebula, stars) function", EXIT_TOOL_FAILED)
    return function


def render_pipeline(pipeline: pipeline_defs.Pipeline, nebula, stars, trusted: dict[str, str]):
    """One pipeline's image: a shipped render function, or the pipeline's own script."""
    if pipeline.script_path is not None:
        return load_script_render(pipeline, trusted)(nebula, stars)
    import renders

    return renders.RENDERS[pipeline.render_function](nebula, stars)


def choose_pipelines(args: argparse.Namespace) -> list[pipeline_defs.Pipeline]:
    """The requested pipelines in order, each once; an unknown id, or a script that is not trusted, is refused.

    ``--linear-only`` chooses none: that mode runs no render, so a pipeline asked beside it is refused.
    """
    if getattr(args, "linear_only", False):
        if args.pipeline:
            raise StackingError("bad_options", "--linear-only runs no pipeline: do not give --pipeline with it", EXIT_INPUT)
        args.ignored_pipelines = []
        return []
    available, ignored = pipeline_defs.load_pipelines(args.pipelines_dir)
    args.ignored_pipelines = ignored
    wanted = list(dict.fromkeys(args.pipeline or [DEFAULT_PIPELINE]))
    unknown = [pipeline_id for pipeline_id in wanted if pipeline_id not in available]
    if unknown:
        hint = f"; ignored definitions: {', '.join(item['folder'] for item in ignored)}" if ignored else ""
        raise StackingError(
            "unknown_pipeline",
            f"unknown pipeline {', '.join(repr(item) for item in unknown)}; known: {', '.join(available)}{hint}",
            EXIT_INPUT,
        )
    chosen = [available[pipeline_id] for pipeline_id in wanted]
    trusted = trusted_scripts(getattr(args, "trusted", None))
    for pipeline in chosen:
        check_script_trust(pipeline, trusted)
    return chosen


def build_device_plan(args: argparse.Namespace) -> Plan:
    """The plan of ``--from-master``: the image of a device, no frame, nothing to stack."""
    pipelines = choose_pipelines(args)
    image = args.from_master
    if image.suffix.lower() not in FITS_SUFFIXES or not image.is_file():
        raise StackingError("folder_missing", f"not a FITS file: {image}", EXIT_INPUT)
    if args.out.name.lower() == "final":
        raise StackingError("output_is_final", "the stacked image never goes to final/", EXIT_INPUT)
    for other in (args.work, args.out):
        if other.resolve() == image.resolve() or is_inside(image, other):
            raise StackingError("output_inside_input", f"{image} is inside {other}", EXIT_INPUT)
    plan = Plan(frames={kind: [] for kind in KINDS}, colour=True, pipelines=pipelines, linear_only=args.linear_only,
                with_linear=bool(getattr(args, "linear", False)) and not args.linear_only, from_master=image)
    plan.steps += ["device_master", "linear_export" if plan.linear_only else "render"]
    if plan.with_linear:
        plan.steps.append("linear_export")
    return plan


def build_plan(args: argparse.Namespace) -> Plan:
    if getattr(args, "from_master", None) is not None:
        return build_device_plan(args)
    pipelines = choose_pipelines(args)
    folders = {
        "dark-flats": args.dark_flats,
        "biases": args.biases,
        "flats": args.flats,
        "darks": args.darks,
        "lights": args.lights,
    }
    for kind, folder in folders.items():
        if folder is None:
            continue
        for other in (args.work, args.out):
            if is_inside(other, folder):
                raise StackingError("output_inside_input", f"{other} is inside the {kind} folder", EXIT_INPUT)
    if args.out.name.lower() == "final":
        raise StackingError("output_is_final", "the stacked image never goes to final/", EXIT_INPUT)
    frames = {kind: fits_files(folder) for kind, folder in folders.items()}
    if args.frames_from is not None:
        frames["lights"] = listed_frames(args.frames_from, args.lights)
    if len(frames["lights"]) < 2:
        raise StackingError("too_few_lights", f"{len(frames['lights'])} FITS or RAW light(s); at least 2 are needed", EXIT_INPUT)
    check_one_format(frames)
    plan = Plan(frames=frames, colour=is_colour(frames["lights"][0]), pipelines=pipelines, linear_only=args.linear_only,
                with_linear=bool(getattr(args, "linear", False)) and not args.linear_only,
                xtrans=raw_files.is_xtrans(frames["lights"][0]))
    for kind in ("dark-flats", "biases", "flats", "darks"):
        if frames[kind]:
            plan.steps.append(f"master_{kind}")
    plan.steps += ["calibration", "strip_wcs", "registration", "integration", "linear_export" if plan.linear_only else "render"]
    if plan.with_linear:
        plan.steps.append("linear_export")
    return plan


def check_one_format(frames: dict[str, list[Path]]) -> None:
    """Camera RAW files and FITS frames are never stacked together: two kinds of sensor, two sizes.

    Astro Sync's families keep them apart; this refuses a folder that mixes them all the same (a stacking run by
    hand), naming the folders.
    """
    raw = sorted({kind for kind, files in frames.items() if any(raw_files.is_raw(path) for path in files)})
    fits_kinds = sorted({kind for kind, files in frames.items() if any(not raw_files.is_raw(path) for path in files)})
    if raw and fits_kinds:
        raise StackingError(
            "mixed_formats",
            f"camera RAW files ({', '.join(raw)}) and FITS frames ({', '.join(fits_kinds)}) cannot be stacked together: "
            "stack each kind of frame on its own",
            EXIT_INPUT,
        )


def contrast_name(stem: str, contrast: str) -> str:
    """``<stem>_presentation`` / ``<stem>_strong``: a render with the contrast of presentation."""
    return f"{stem}_{contrast}"


def finish_of(args: argparse.Namespace, pipeline: str | None = None) -> "render_finish.Finish":
    """The finish (contrast, colour intensity, softened stars) the options ask for ``pipeline``.

    A ``--finish <pipeline>=...`` of that pipeline (the last one when it is given twice) wins;
    otherwise, or with no pipeline named, ``--contrast``, ``--colour`` and ``--soft-stars``: the default.
    """
    finishes = pipeline_finishes(args)
    if pipeline is not None and pipeline in finishes:
        return finishes[pipeline]
    try:
        return render_finish.checked(
            getattr(args, "contrast", "none"), getattr(args, "colour", "none"), getattr(args, "soft_stars", "none"))
    except ValueError as error:
        raise StackingError("bad_option", str(error), EXIT_INPUT) from error


def pipeline_finishes(args: argparse.Namespace) -> dict[str, "render_finish.Finish"]:
    """The ``--finish`` options by pipeline id, the last one of an id kept."""
    return dict(getattr(args, "finish", None) or [])


def finishes_fields(plan: Plan, args: argparse.Namespace) -> dict[str, dict[str, str]]:
    """The finish of each rendered pipeline, by id, for the plan event and the provenance; none for "No processing"."""
    if plan.linear_only:
        return {}
    return {pipeline.id: render_finish.fields(finish_of(args, pipeline.id)) for pipeline in plan.pipelines}


def finish_name(stem: str, finish: "render_finish.Finish") -> str:
    """``<stem>_strong``, ``<stem>_strong_colour-enhanced_stars-light``...: a render with its finish."""
    return f"{stem}_{finish.suffix()}"


def planned_outputs(plan: Plan, name: str, integration: str | None = None, contrast: str = "none",
                    colour: str = "none", soft_stars: str = "none",
                    finishes: dict[str, "render_finish.Finish"] | None = None) -> list[str]:
    """The files the run will write in ``--out``, the master kept then the provenance last.

    With a contrast, stronger colours or softened stars, each rendered image is
    followed by the same image with that finish (``<stem>_presentation``,
    ``<stem>_strong_colour-enhanced_stars-light``...); the image without it
    stays.  ``finishes`` (``--finish``) gives a pipeline a finish of its own
    instead of the default three settings.  "No processing" has no such twin:
    the curve pivots on the sky level of a stretched image and does nothing
    visible on a linear one.
    """
    default = render_finish.checked(contrast, colour, soft_stars)
    finishes = finishes or {}
    if plan.linear_only:
        files = [pipeline_defs.linear_file_name(name)]
    else:
        files = []
        for pipeline in plan.pipelines:
            finish = finishes.get(pipeline.id, default)
            for ext in pipeline.outputs:
                files.append(f"{name}_{pipeline.id}.{ext}")
                if not finish.is_plain():
                    files.append(f"{finish_name(f'{name}_{pipeline.id}', finish)}.{ext}")
        if plan.with_linear:
            files.append(pipeline_defs.linear_file_name(name))
    if integration:
        files.append(f"{integration}_integration.fit")
    return files + [f"{name}_stacking.json"]


def plan_fields(plan: Plan, args: argparse.Namespace) -> dict[str, object]:
    return {
        "format": FORMAT_VERSION,
        "frames": {kind: len(files) for kind, files in plan.frames.items()},
        "flatsFilters": flats_filters(plan.frames["flats"]),
        # A colour sensor (``colour`` below is the colour intensity of the finish).
        "colourSensor": plan.colour,
        "lightFormat": "raw" if plan.frames["lights"] and raw_files.is_raw(plan.frames["lights"][0]) else "fits",
        "xtrans": plan.xtrans,
        "pipelines": [pipeline.id for pipeline in plan.pipelines],
        "ignoredPipelines": args.ignored_pipelines,
        "steps": plan.steps,
        "linearOnly": plan.linear_only,
        "withLinear": plan.with_linear,
        "fromMaster": plan.from_master.name if plan.from_master else None,
        "contrast": getattr(args, "contrast", "none"),
        "colour": getattr(args, "colour", "none"),
        "softStars": getattr(args, "soft_stars", "none"),
        # The frame of the stacking asked for (``auto`` or the rank of a notch: ``night_framing.py``).
        "frameSize": getattr(args, "frame_size", night_framing.AUTO),
        # The finish of each rendered pipeline: the three above, or its own --finish.
        "finishes": finishes_fields(plan, args),
        "outputs": planned_outputs(plan, args.name, getattr(args, "integration", None), getattr(args, "contrast", "none"),
                                   getattr(args, "colour", "none"), getattr(args, "soft_stars", "none"),
                                   pipeline_finishes(args)),
    }


def find_tool(given: Path | None, names: tuple[str, ...], code: str) -> Path:
    if given is not None:
        if given.is_file() and os.access(given, os.X_OK):
            return given
        raise StackingError(code, f"not an executable: {given}", EXIT_TOOL_MISSING)
    for name in names:
        found = shutil.which(name)
        if found:
            return Path(found)
    raise StackingError(code, f"none of {', '.join(names)} found on PATH", EXIT_TOOL_MISSING)


def prepare_work(work: Path) -> None:
    if work.exists() and any(work.iterdir()) and not (work / MARKER).exists():
        raise StackingError("work_not_empty", f"{work} is not empty and was not created by this script", EXIT_INPUT)
    if work.exists():
        shutil.rmtree(work)
    work.mkdir(parents=True)
    (work / MARKER).write_text("created by astro_sync_stack.py; removed at the end\n", encoding="utf-8")


def link_or_copy(source: Path, link: Path) -> None:
    """A symbolic link; where that is refused (Windows without developer mode), a hard link (same volume, no
    room taken); where that is refused too (another volume), a copy."""
    try:
        link.symlink_to(source.resolve())
        return
    except OSError:
        pass
    try:
        os.link(source, link)
    except OSError:
        shutil.copy2(source, link)


def link_inputs(plan: Plan, work: Path) -> None:
    """Expose each kind under work/input/<kind>/ with uniform names Siril can convert."""
    for kind, files in plan.frames.items():
        if not files:
            continue
        folder = work / "input" / kind
        folder.mkdir(parents=True)
        for index, source in enumerate(files, 1):
            link = folder / f"frame_{index:05d}{source.suffix.lower()}"
            link_or_copy(source, link)


# A calibrated frame whose lowest pixel is under this is not a clean frame of a sky in [0, 1]: the
# usual noise of a dark-subtracted frame stays far above it.
NORMALISATION_FLOOR = -0.05


def siril_scripts(plan: Plan, framing: str = registration_framing.COMMON_AREA,
                  output_norm: bool = True) -> tuple[list[tuple[str, str]], str]:
    """Return one Siril script per step, in order, and the light sequence prefix.

    ``registration`` only computes the transforms; ``alignment`` applies them
    on the frame ``framing`` names (``registration_framing.COMMON_AREA`` or
    ``FULL_FIELD``), which is chosen from what ``registration`` measured: the
    scripts after it are asked again with the framing it gave.

    ``output_norm`` is false when a calibrated frame went under ``NORMALISATION_FLOOR``: the stack is
    then left in the units of the frames, because ``-output_norm`` would stretch the lowest pixel to 0
 and lift the sky of the master with it.

    Every script runs from the work folder (``siril-cli -d``), so no path with
    spaces ever appears inside a script.
    """
    have = {kind: bool(files) for kind, files in plan.frames.items()}
    scripts: list[tuple[str, str]] = []
    lines: list[str] = []

    # Siril's ``calibrate -debayer`` leaves a Fujifilm X-Trans mosaic as it is: such a night is debayered when
    # every frame is converted, the calibration frames too, and calibrated as colour frames.
    debayer_here = " -debayer" if plan.xtrans else ""

    def convert(kind: str, base: str, extra: str = "") -> None:
        lines.extend([f"cd input/{kind}", f"convert {base}{debayer_here}{extra} -out=../../process", "cd ../../process"])

    def close(step: str) -> None:
        scripts.append((step, "\n".join(["requires 1.4.0", *lines, "close"]) + "\n"))
        lines.clear()

    if have["dark-flats"]:
        convert("dark-flats", "darkflat")
        lines.append("stack darkflat rej 3 3 -nonorm -32b -out=../masters/master_darkflat")
        close("master_dark-flats")
    if have["biases"]:
        convert("biases", "bias")
        lines.append("stack bias rej 3 3 -nonorm -32b -out=../masters/master_bias")
        close("master_biases")
    if have["flats"]:
        convert("flats", "flat")
        if have["dark-flats"]:
            lines += ["calibrate flat -dark=../masters/master_darkflat", "stack pp_flat rej 3 3 -norm=mul -32b -out=../masters/master_flat"]
        elif have["biases"]:
            lines += ["calibrate flat -bias=../masters/master_bias", "stack pp_flat rej 3 3 -norm=mul -32b -out=../masters/master_flat"]
        else:
            lines += ["stack flat rej 3 3 -norm=mul -32b -out=../masters/master_flat"]
        close("master_flats")
    if have["darks"]:
        convert("darks", "dark")
        lines.append("stack dark rej 3 3 -nonorm -32b -out=../masters/master_dark")
        close("master_darks")

    options = []
    if have["darks"]:
        options += ["-dark=../masters/master_dark", "-cc=dark"]
    elif have["biases"]:
        options.append("-bias=../masters/master_bias")
    if have["flats"]:
        options.append("-flat=../masters/master_flat")
    if options:
        convert("lights", "light")
        if plan.colour and not plan.xtrans:
            options += ["-cfa", "-equalize_cfa", "-debayer"]
        lines.append("calibrate light " + " ".join(options))
        prefix = "pp_light"
    else:
        convert("lights", "light", " -debayer" if plan.colour and not plan.xtrans else "")
        prefix = "light"
    close("calibration")

    lines += [
        "cd process",
        f"register {prefix} -2pass -transf=affine -minpairs=12 -maxstars=2000",
    ]
    close("registration")
    lines += [
        "cd process",
        registration_framing.apply_command(prefix, framing),
    ]
    close("alignment")
    lines += [
        "cd process",
        f"stack r_{prefix} rej w 3 3 -norm=addscale -weight=wfwhm"
        + (" -rgb_equal" if plan.colour else "")
        + (" -output_norm" if output_norm else "")
        + " -32b -out=../masters/stack_linear",
    ]
    close("integration")
    return scripts, prefix


class Runner:
    def __init__(self) -> None:
        self.child: subprocess.Popen[bytes] | None = None
        self.cancelled = False

    def cancel(self, *_: object) -> None:
        self.cancelled = True
        if self.child is not None and self.child.poll() is None:
            self.child.terminate()

    def run(self, command: list[str], log: Path, cwd: Path | None = None) -> int:
        if self.cancelled:
            raise Cancelled()
        with log.open("ab") as handle:
            self.child = subprocess.Popen(command, cwd=cwd, stdout=handle, stderr=subprocess.STDOUT)
            code = self.child.wait()
            self.child = None
        if self.cancelled:
            raise Cancelled()
        return code


def strip_wcs(process: Path, prefix: str) -> int:
    from astropy.io import fits

    count = 0
    for path in sorted(process.glob(f"{prefix}_*.fit")):
        if path.is_symlink():
            # Siril's ``convert`` links a FITS file it has nothing to convert (a master of a plan, already
            # a colour FITS): editing the header through the link would edit the file it comes from.
            source = path.resolve()
            path.unlink()
            shutil.copy2(source, path)
        with fits.open(path, mode="update", memmap=False) as hdul:
            header = hdul[0].header
            for key in list(header):
                if key in WCS_EXACT or key.startswith(WCS_PREFIXES):
                    del header[key]
            header["HISTORY"] = "WCS removed from working copy for Siril/OpenCV compatibility"
            hdul.flush()
        count += 1
    return count


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for block in iter(lambda: handle.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


def siril_version(siril: Path) -> str:
    try:
        result = subprocess.run([str(siril), "--version"], capture_output=True, text=True, timeout=30)
    except (OSError, subprocess.TimeoutExpired):
        return "unknown"
    for line in (result.stdout + result.stderr).splitlines():
        if line.lower().startswith("siril"):
            return line.strip()
    return "unknown"


def find_starnet(args: argparse.Namespace, plan: Plan) -> Path | None:
    """StarNet2 when it can be used; an error only if a chosen pipeline requires it or ``--starnet`` names a missing file.

    The linear-only mode runs no render, so it never looks for it.
    """
    if plan.linear_only:
        return None
    required = any("starnet2" in pipeline.required_tools for pipeline in plan.pipelines)
    if args.no_starnet and required:
        raise StackingError("starnet_missing", "a chosen pipeline requires StarNet2, but --no-starnet was given", EXIT_TOOL_MISSING)
    if args.no_starnet:
        return None
    try:
        return find_tool(args.starnet, ("starnet2",), "starnet_missing")
    except StackingError:
        if args.starnet is not None or required:
            raise
        return None


def find_graxpert(args: argparse.Namespace, plan: Plan) -> Path | None:
    """GraXpert when a chosen pipeline uses it and it can be found; else None.

    No shipped pipeline requires it: without it the "Polished render" takes
    Siril's background and denoising (or, without Siril, the polynomial and
    no denoising), and says so in the provenance.
    ``--graxpert`` naming a missing file is an error, as for StarNet2.  With
    ``--polished-engine siril`` GraXpert is not looked for at all.
    """
    wanted = [pipeline for pipeline in plan.pipelines if "graxpert" in (*pipeline.required_tools, *pipeline.optional_tools)]
    if plan.linear_only or not wanted:
        return None
    required = any("graxpert" in pipeline.required_tools for pipeline in wanted)
    if getattr(args, "polished_engine", "graxpert-ai") == "siril" and not required:
        return None  # the task chose Siril's own engine: GraXpert is not looked for
    if getattr(args, "no_graxpert", False):
        if required:
            raise StackingError("graxpert_missing", "a chosen pipeline requires GraXpert, but --no-graxpert was given", EXIT_TOOL_MISSING)
        return None
    given = getattr(args, "graxpert", None)
    try:
        return find_tool(given, ("graxpert", "GraXpert"), "graxpert_missing")
    except StackingError:
        if given is not None or required:
            raise
        return None


def write_output(report: Reporter, pipeline: str, path: Path, contrast: str = "none",
                 colour: str = "none", soft_stars: str = "none") -> dict[str, object]:
    """Report one file written in ``--out`` (the ``output`` event) and describe it for the provenance.

    ``contrast``, ``colour``, ``soft_stars``: the finish the image carries (the curve of
    presentation, the colour intensity, the softened stars); each named only when there is
    one, so an image without it is described as before.
    """
    entry = {"pipeline": pipeline, "file": path.name, "bytes": path.stat().st_size, "sha256": sha256(path)}
    if contrast != "none":
        entry["contrast"] = contrast
    if colour != "none":
        entry["colour"] = colour
    if soft_stars != "none":
        entry["softStars"] = soft_stars
    report.emit("output", **entry)
    return entry


def prepared_layers(args: argparse.Namespace, kind: str, master: Path, starnet: Path | None,
                    graxpert: Path | None, render_log, notes: dict[str, object],
                    siril: Path | None = None, dualband: bool = False,
                    natural_layers=None, imposed: dict[str, object] | None = None,
                    measured: dict[str, object] | None = None):
    """The nebula and star layers of one preparation (``renders.PREPARATIONS``): ``native`` or ``polished``.

    ``polished`` runs the engine the task chose (``--polished-engine``:
    GraXpert's AI models, or Siril's own background and denoising).  When
    GraXpert is missing or fails, Siril does it; when nothing can run, the render
    is made with the polynomial background and no denoising -- and ``notes`` says
    which engine did the work and why, rather than losing the whole stacking.
    ``dualband``: the SCNR is skipped (the green of the OIII is signal).

    Once StarNet2 has split the stars, the polished sky is brought to the level
    of the natural render of the same master: ``natural_layers``, the
    natural nebula and star layers when they were made anyway, or made here
    for the level alone.

    The preparation shows the master with a display -- a black point per channel and a stretch scale, and for
    the polished one the sky levels its matches aim at (``sky``: ``displayed`` and ``final``, ``finalWithStars``)
    -- that it measures, unless ``imposed`` (``{"black": [r, g, b], "scale": s, ...}``) gives it; ``measured``
    (when given) receives the one used.
    """
    import renders

    work = args.work / "render"
    if kind == "polished":
        native, done = renders.prepare_polished(
            master, graxpert, work, f"{args.name}_polished", render_log,
            args.gradient_degree, args.black_percentile, args.deconv_fwhm,
            getattr(args, "polished_engine", renders.DEFAULT_POLISHED_ENGINE), siril, dualband,
            imposed, measured,
        )
        notes["polished"] = done
        nebula, stars = renders.star_layer(native, work, f"{args.name}_polished", starnet, render_log)
        if starnet is not None and stars.any():
            # The level of the natural render's sky, stars included: the master's own, unless the display imposes it.
            target = (imposed or {}).get("finalWithStars")
            if target is None:
                if natural_layers is None:
                    natural_layers = prepared_layers(args, "native", master, starnet, graxpert, render_log, {})
                target = renders.natural_final_level(*natural_layers)
            if measured is not None:
                measured["finalWithStars"] = float(target)
            nebula, shift = renders.match_final_sky_level(nebula, stars, target)
            done["skyLevelFinalShift"] = round(shift, 6)
        return nebula, stars
    imposed = imposed or {}
    native = renders.prepare_native(
        master, args.gradient_degree, args.black_percentile, args.deconv_fwhm,
        imposed.get("scale"), imposed.get("black"), measured,
    )
    return renders.star_layer(native, work, args.name, starnet, render_log)


DUALBAND_WORDS = (
    "dual", "duo", "bibande", "extreme", "enhance", "ultimate", "alpt", "nbz", "hao3", "haoiii",
    "hoo", "triad", "quadband",
)


def is_dualband_filter(name: str | None) -> bool:
    """Whether a filter name is a dual-band one (the words of ``plan_image::DUALBAND_WORDS``, letters and digits only)."""
    letters = "".join(c for c in (name or "") if c.isascii() and c.isalnum()).lower()
    return any(word in letters for word in DUALBAND_WORDS)


def capture_is_dualband(args: argparse.Namespace, plan: Plan) -> bool:
    """Whether the lights went through a dual-band filter: the green of such a capture is OIII, signal.

    Three ways to know, any one is enough: ``--capture-filter`` names the
    filter; the lights' ``FILTER`` keyword names one; a pipeline of the dual-band
    filter (``dualband-correction``, ``hoo``) was chosen, which says what the
    person shot.
    """
    if is_dualband_filter(getattr(args, "capture_filter", None)):
        return True
    if any(pipeline.filter == "dual-band" for pipeline in plan.pipelines):
        return True
    lights = plan.frames.get("lights") or ([plan.from_master] if plan.from_master else [])
    if not lights:
        return False
    try:
        from astropy.io import fits

        return is_dualband_filter(str(fits.getheader(lights[0], memmap=False).get("FILTER", "")))
    except (OSError, ValueError):
        return False


def render_outputs(args: argparse.Namespace, plan: Plan, report: Reporter, runner: Runner,
                   master: Path, starnet: Path | None, graxpert: Path | None = None,
                   notes: dict[str, object] | None = None,
                   siril: Path | None = None) -> list[dict[str, object]]:
    """Step ``render``: the layers once per preparation, then each chosen pipeline's image (TIFF and/or PNG).

    Most renders share the natural preparation; the "Polished render" has its
    own (``renders.PREPARATIONS``).  ``notes`` (a dict, filled when given)
    receives what a preparation did, for the provenance.
    """
    work, out = args.work, args.out
    sys.path.insert(0, str(Path(__file__).resolve().parent))
    import renders

    notes = {} if notes is None else notes
    dualband = capture_is_dualband(args, plan)
    kinds = list(dict.fromkeys(
        "native" if pipeline.script_path is not None else renders.preparation_of(pipeline.render_function)
        for pipeline in plan.pipelines
    ))
    kinds.sort(key=lambda kind: kind != "native")  # the natural layers first: the polished sky is matched to them
    layers = {}
    # Keep stdout for the JSON events: the renderer's prints and StarNet2's
    # and GraXpert's progress go to render.log in the work folder.
    try:
        with (work / "render.log").open("ab") as render_log, contextlib.redirect_stdout(sys.stderr):
            # The display (imposed and recorded) is the image's own: the one preparation its pipeline uses.
            shown = len(kinds) == 1
            for kind in kinds:
                layers[kind] = prepared_layers(args, kind, master, starnet, graxpert, render_log, notes, siril, dualband,
                                               layers.get("native"),
                                               getattr(args, "display", None) if shown else None,
                                               getattr(args, "measured_display", None) if shown else None)
                if runner.cancelled:
                    raise Cancelled()
    except subprocess.CalledProcessError as error:
        raise StackingError("starnet_failed", f"StarNet2 failed ({error.returncode})", EXIT_TOOL_FAILED) from error
    except (RuntimeError, ValueError) as error:
        raise StackingError("render_failed", str(error), EXIT_TOOL_FAILED) from error
    if runner.cancelled:
        raise Cancelled()
    out.mkdir(parents=True, exist_ok=True)
    outputs = []
    for pipeline in plan.pipelines:
        finish = finish_of(args, pipeline.id)
        kind = "native" if pipeline.script_path is not None else renders.preparation_of(pipeline.render_function)
        nebula, stars = layers[kind]
        image = render_pipeline(pipeline, nebula, stars, trusted_scripts(args.trusted))
        stem = f"{args.name}_{pipeline.id}"
        for path in renders.save(image, out, stem, jpeg=False, formats=tuple(pipeline.outputs)):
            outputs.append(write_output(report, pipeline.id, path))
        if not finish.is_plain():
            # The same render with its finish, beside it: the plain image stays the main one.
            finished = finish.apply(image)
            for path in renders.save(finished, out, finish_name(stem, finish), jpeg=False, formats=tuple(pipeline.outputs)):
                outputs.append(write_output(report, pipeline.id, path, finish.contrast, finish.colour, finish.stars))
    return outputs


def linear_outputs(args: argparse.Namespace, report: Reporter, runner: Runner, master: Path) -> list[dict[str, object]]:
    """Step ``linear_export`` ("no processing"): the linear master as ``<name>_linear.tif``, nothing else.

    No twin, whatever ``--contrast``, ``--colour`` or ``--soft-stars`` say: the curve pivots on the sky
    level of a stretched image and does nothing visible on a linear one.
    """
    sys.path.insert(0, str(Path(__file__).resolve().parent))
    import linear_export

    args.out.mkdir(parents=True, exist_ok=True)
    try:
        path = linear_export.write_linear_tiff(master, args.out, args.name)
    except (OSError, ValueError) as error:
        raise StackingError("render_failed", f"the linear image could not be written: {error}", EXIT_TOOL_FAILED) from error
    if runner.cancelled:
        raise Cancelled()
    return [write_output(report, LINEAR_PIPELINE, path)]


def measure_quality(master: Path) -> dict[str, object] | None:
    """The quality of the linear master, read on its stars (``star_quality.py``).

    ``{"fwhmPixels", "eccentricity", "starCount", "measuredOn": "linear",
    "measuredAt"}``, or ``None`` when the image has fewer than twenty readable
    stars or the measure raised: a measure never fails a stacking, the reason is
    written to stderr and the image is sent without a quality.
    """
    sys.path.insert(0, str(Path(__file__).resolve().parent))
    try:
        import star_quality

        quality = star_quality.measure_master(master)
    except Exception as error:  # noqa: BLE001 - whatever the measure raises, the stacking goes on
        print(f"quality: not measured ({type(error).__name__}: {error})", file=sys.stderr, flush=True)
        return None
    if quality is None:
        print("quality: not measured (fewer than twenty stars)", file=sys.stderr, flush=True)
        return None
    quality["measuredAt"] = time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())
    return quality


def keep_integration(args: argparse.Namespace, plan: Plan, report: Reporter, master: Path,
                     registration: registration_framing.Registration | None = None) -> dict[str, object] | None:
    """``--integration``: keeps the linear master of the family in ``--out`` (``integration_master.py``).

    The field rotation the registration measured goes into the master's header: a plan
    reads it back to measure its own rotation over all the frames of all its nights.
    """
    family = getattr(args, "integration", None)
    if not family:
        return None
    sys.path.insert(0, str(Path(__file__).resolve().parent))
    import integration_master

    try:
        # A camera RAW light is read through the FITS copy Siril made of it.
        converted = None
        if plan.frames["lights"] and raw_files.is_raw(plan.frames["lights"][0]):
            converted = sorted((args.work / "process").glob("light_*.fit"))
        kept = integration_master.keep_master(
            master, args.out, family, plan.frames["lights"], registration.rotation_range if registration else None,
            converted, getattr(args, "capture_filter", None))
    except OSError as error:
        raise StackingError("integration_failed", f"the master of the integration could not be kept: {error}", EXIT_TOOL_FAILED) from error
    report.emit("integration", **kept)
    return kept


def measured_registration(sequence: Path, frames: int, unit: str = LIGHTS) -> registration_framing.Registration:
    """What the registration said of its ``frames`` (lights, or masters of a plan), read from its sequence file.

    A sequence file that cannot be read keeps the framing of before (the common
    area, nothing measured).  Fewer than two frames aligned stop the stacking:
    a few lights Siril could not match (a frame at dawn, a cloud) are only left
    out and named, they never stop it.
    """
    found = registration_framing.read_registration(sequence)
    if found is None:
        found = registration_framing.Registration(frames=frames, unregistered=(), rotation_degrees=None)
    if found.registered < 2:
        raise StackingError(
            "registration_failed",
            f"only {found.registered} of {found.frames} {unit} could be aligned; see siril.log in the work folder",
            EXIT_TOOL_FAILED,
        )
    return found


def registration_fields(registration: registration_framing.Registration, names: list[str], unit: str = LIGHTS,
                        frame: dict[str, object] | None = None) -> dict[str, object]:
    """The ``registration`` event, and the same block of the provenance.

    ``names``: what each frame is called, in the order of the sequence (the
    lights' file names; the nights of a plan's masters).  ``unit``: ``"lights"``
    or ``"masters"``, what the frames are.  ``fieldRotation``: ``"progressive"``
    (the field turns, alt-azimuth mount) or ``"none"``, ``None`` when it could
    not be measured; ``fieldRotationDegrees``: the span of the rotation along
    the night (over all the nights of a plan); ``framing``: ``"full"`` (the
    whole field of the reference frame) or ``"common"`` (the area every frame
    covers); ``unregistered``: the frames Siril could not align, left out; ``frame`` (a night only): the
    frame of the stacking, its notches and the one kept (``night_framing.frame_report``).
    """
    # A plan with the image of a device cannot say how the field turned: no figure is announced, the framing
    # is the whole field and ``fieldRotationUnknown`` says why.
    unknown = registration.rotation_unknown
    measured = registration.rotation_degrees is not None and not unknown
    return {
        "unit": unit,
        "frames": registration.frames,
        "registered": registration.registered,
        "unregistered": [names[number - 1] for number in registration.unregistered if number <= len(names)],
        "fieldRotationDegrees": registration.rotation_degrees if not unknown else None,
        "fieldRotation": ("progressive" if registration.progressive else "none") if measured else None,
        "framing": registration.framing,
        **({"fieldRotationUnknown": True} if unknown else {}),
        **({"frame": frame} if frame is not None else {}),
    }


def execute_device(args: argparse.Namespace, plan: Plan, report: Reporter, runner: Runner) -> dict[str, object]:
    """``--from-master``: the image of a device scaled to a master, then the renders of a stacking.

    No Siril step: the scratch holds the converted master and the render's own files.  Siril is looked for
    only to be given to the "Polished render" (its fallback), and its absence is not an error here.
    """
    try:
        siril = find_tool(args.siril, ("siril-cli", "siril"), "siril_missing")
    except StackingError:
        siril = None
    starnet = find_starnet(args, plan)
    graxpert = find_graxpert(args, plan)
    started = time.monotonic()
    work, out = args.work, args.out
    prepare_work(work)
    for folder in ("masters", "render"):
        (work / folder).mkdir()
    total = len(plan.steps)
    report.emit("step", step="device_master", index=1, total=total)
    master = work / "masters" / "stack_linear.fit"
    try:
        described = device_master.convert(plan.from_master, master)
    except device_master.DeviceImageError as error:
        code = "device_stretched" if error.code == "stretched" else "device_unreadable"
        raise StackingError(code, str(error), EXIT_INPUT) from error
    if runner.cancelled:
        raise Cancelled()
    stacked_at = time.monotonic()
    notes: dict[str, object] = {}
    index = 2
    report.emit("step", step="linear_export" if plan.linear_only else "render", index=index, total=total)
    if plan.linear_only:
        outputs = linear_outputs(args, report, runner, master)
    else:
        outputs = render_outputs(args, plan, report, runner, master, starnet, graxpert, notes, siril)
        if plan.with_linear:
            index += 1
            report.emit("step", step="linear_export", index=index, total=total)
            outputs += linear_outputs(args, report, runner, master)
    finished = time.monotonic()
    provenance = {
        "format": FORMAT_VERSION,
        "frames": {kind: 0 for kind in KINDS},
        "flatsFilters": [],
        "stackedFrames": described["stackedFrames"],
        "registration": None,
        "colourSensor": True,
        "width": described["width"],
        "height": described["height"],
        "target": args.out.parent.name,
        "pipelines": [{"id": pipeline.id, "version": pipeline.version} for pipeline in plan.pipelines]
        + ([{"id": LINEAR_PIPELINE, "version": None}] if plan.linear_only or plan.with_linear else []),
        "linearOnly": plan.linear_only,
        "contrast": "none" if plan.linear_only else getattr(args, "contrast", "none"),
        "colour": "none" if plan.linear_only else getattr(args, "colour", "none"),
        "softStars": "none" if plan.linear_only else getattr(args, "soft_stars", "none"),
        "finishes": finishes_fields(plan, args),
        "recipe": {
            "siril": [],
            "render": None if plan.linear_only else {
                "gradientDegree": args.gradient_degree, "blackPercentile": args.black_percentile,
                "deconvFwhm": args.deconv_fwhm, "starSeparation": starnet is not None, **notes},
        },
        # Siril stacked nothing: it is named only when the "Polished render" fell back on it.
        "tools": {"siril": siril_version(siril) if siril is not None and notes.get("polished", {}).get("engine") == "siril" else None,
                  "starnet2": starnet is not None, "graxpert": graxpert is not None},
        "seconds": {"stack": round(stacked_at - started, 1), "render": round(finished - stacked_at, 1),
                    "total": round(finished - started, 1)},
        "outputs": outputs,
        "integration": None,
        # Measured on request ("Measure this image"), never by the render: the quality of the image is the device's.
        "quality": None,
        "deviceMaster": {**described, "sha256": sha256(plan.from_master)},
    }
    (out / f"{args.name}_stacking.json").write_text(json.dumps(provenance, indent=2) + "\n", encoding="utf-8")
    if not args.keep_work:
        shutil.rmtree(work)
    return provenance


FIELD_SUFFIX = "_field.fit"
FIELD_RECORD_SUFFIX = "_field.json"


def stack_poses(args: argparse.Namespace, plan: Plan, report: Reporter, runner: Runner, siril: Path,
                names: list[str]) -> tuple[list[str], registration_framing.Registration | None, dict[str, object],
                                           dict[str, object], int]:
    """Siril's steps, calibration to integration, into ``masters/stack_linear.fit`` of the work folder.

    Returns the lines of the scripts run, the registration, the normalisation, the frame of the stacking
    (``night_framing.frame_report``) and the index of the last step announced.
    """
    work = args.work
    scripts, prefix = siril_scripts(plan)
    normalisation: dict[str, object] = {"outputNorm": True, "lowestPixel": None}
    log = work / "siril.log"
    total = len(plan.steps)
    index = 0
    failure = {"registration": "registration_failed", "alignment": "registration_failed", "integration": "integration_failed"}
    registration: registration_framing.Registration | None = None
    frame = night_framing.frame_report(None, None, False, args.frame_size)
    executed: list[tuple[str, str]] = []
    pending = list(scripts)
    number = 0
    while pending:
        step, script = pending.pop(0)
        number += 1
        if step == "registration":
            index += 1
            report.emit("step", step="strip_wcs", index=index, total=total)
            if strip_wcs(work / "process", prefix) < 2:
                raise StackingError("calibration_failed", "Siril produced fewer than 2 calibrated lights", EXIT_TOOL_FAILED)
        if step != "alignment":  # the alignment is the second half of the registration step
            index += 1
            report.emit("step", step=step, index=index, total=total)
        path = work / f"{number:02d}_{step}.ssf"
        path.write_text(script, encoding="utf-8")
        if runner.run(siril_config.command(siril, work, path), log) != 0:
            code = failure.get(step, "calibration_failed")
            raise StackingError(code, f"Siril failed at step {step}; see siril.log in the work folder", EXIT_TOOL_FAILED)
        executed.append((step, script))
        if step == "calibration":
            lowest = overscan.lowest_pixel(sorted((work / "process").glob(f"{prefix}_*.fit")))
            normalisation["lowestPixel"] = None if lowest is None else round(lowest, 5)
            if lowest is not None and lowest < NORMALISATION_FLOOR:
                normalisation["outputNorm"] = False
                report.emit("note", code="normalisation_skipped", lowestPixel=normalisation["lowestPixel"],
                            message=f"a calibrated frame goes down to {lowest:.4f}; the stack is not rescaled to [0, 1], "
                                    "which would lift the sky of the master")
        if step == "registration":
            sequence = work / "process" / f"{prefix}_.seq"
            registration = measured_registration(sequence, len(names))
            # The frames on offer when the field turns: measured from the transforms, before they are applied.
            first = next(iter(sorted((work / "process").glob(f"{prefix}_*.fit"))), None)
            frame = night_framing.frame_report(
                registration_framing.read_matrices(sequence) if registration.progressive else None,
                night_framing.frame_shape(first) if first is not None else None,
                registration.progressive, args.frame_size)
            report.emit("registration", **registration_fields(registration, names, frame=frame))
            # What follows applies the transforms on the frame the measured rotation calls for.
            pending = [item for item in siril_scripts(plan, registration.framing, bool(normalisation["outputNorm"]))[0]
                       if item[0] in ("alignment", "integration")]
    calibration = "".join(script for step, script in executed if step not in failure)
    integration = "".join(script for step, script in executed if step in failure)
    return (calibration + integration).splitlines(), registration, normalisation, frame, index


def field_fingerprint(plan: Plan, siril: Path) -> str | None:
    """What makes the master of the whole field: the poses and calibration frames, the recipe, Siril's version.

    ``None`` (never reused) for camera RAW lights: the master kept of their family reads the copies Siril
    makes of them, which only a stacking writes.
    """
    if plan.frames["lights"] and raw_files.is_raw(plan.frames["lights"][0]):
        return None
    digest = hashlib.sha256()
    digest.update(json.dumps({"format": FORMAT_VERSION, "siril": siril_version(siril), "colour": plan.colour,
                              "xtrans": plan.xtrans,
                              "scripts": siril_scripts(plan, registration_framing.FULL_FIELD)[0]}).encode())
    for kind in KINDS:
        for frame in plan.frames[kind]:
            stat = frame.stat()
            digest.update(f"{kind}\0{frame.resolve()}\0{stat.st_size}\0{stat.st_mtime_ns}\n".encode())
    return digest.hexdigest()


def field_paths(args: argparse.Namespace) -> tuple[Path, Path]:
    """``<out>/<name>_field.fit`` and the record beside it."""
    return args.out / f"{args.name}{FIELD_SUFFIX}", args.out / f"{args.name}{FIELD_RECORD_SUFFIX}"


def keep_field(args: argparse.Namespace, master: Path, fingerprint: str | None, recipe: list[str],
               registration: registration_framing.Registration | None, names: list[str],
               normalisation: dict[str, object], frame: dict[str, object]) -> None:
    """Keeps the master of the whole field in ``--out``, with what a later stacking of the same poses reuses.

    Only for a field that turns (there are notches): another frame is then cut from it without stacking.
    """
    if fingerprint is None or registration is None:
        return
    field, record = field_paths(args)
    args.out.mkdir(parents=True, exist_ok=True)
    partial = args.out / f".{field.name}.part"
    shutil.copyfile(master, partial)
    partial.replace(field)
    record.write_text(json.dumps({
        "format": FORMAT_VERSION,
        "fingerprint": fingerprint,
        "recipe": recipe,
        "registration": {"frames": registration.frames, "unregistered": list(registration.unregistered),
                         "rotationDegrees": registration.rotation_degrees,
                         "rotationRange": list(registration.rotation_range) if registration.rotation_range else None},
        "normalisation": normalisation,
        "frame": {key: frame[key] for key in ("frameHeight", "frameWidth", "notches", "automatic")},
    }, indent=2) + "\n", encoding="utf-8")


def reused_field(args: argparse.Namespace, fingerprint: str | None, master: Path) -> dict[str, object] | None:
    """The master of the whole field of the last stacking of the same poses, copied as ``master``; ``None`` otherwise."""
    field, record = field_paths(args)
    if fingerprint is None or not field.is_file() or not record.is_file():
        return None
    try:
        kept = json.loads(record.read_text(encoding="utf-8"))
        if kept.get("format") != FORMAT_VERSION or kept.get("fingerprint") != fingerprint:
            return None
        found = kept["registration"]
        registration = registration_framing.Registration(
            frames=int(found["frames"]), unregistered=tuple(int(n) for n in found["unregistered"]),
            rotation_degrees=found["rotationDegrees"],
            rotation_range=tuple(found["rotationRange"]) if found.get("rotationRange") else None)
        result = {"recipe": list(kept["recipe"]), "registration": registration,
                  "normalisation": dict(kept["normalisation"]), "frame": dict(kept["frame"])}
    except (OSError, ValueError, KeyError, TypeError):
        return None
    shutil.copyfile(field, master)
    return result


def execute(args: argparse.Namespace, plan: Plan, report: Reporter, runner: Runner) -> dict[str, object]:
    if plan.from_master is not None:
        return execute_device(args, plan, report, runner)
    siril = find_tool(args.siril, ("siril-cli", "siril"), "siril_missing")
    starnet = find_starnet(args, plan)
    graxpert = find_graxpert(args, plan)
    started = time.monotonic()
    work, out = args.work, args.out
    prepare_work(work)
    link_inputs(plan, work)
    # The overscan of a scientific CCD, when the headers say where the image is.
    overscan_note = overscan.trim_overscan(plan.frames, work / "input", cfa=plan.colour)
    if overscan_note is not None:
        report.emit("note", code="overscan_trimmed" if overscan_note["trimmed"] else f"overscan_{overscan_note['reason']}",
                    **{key: value for key, value in overscan_note.items() if key != "trimmed"})
    for folder in ("process", "masters", "render"):
        (work / folder).mkdir()
    names = [light.name for light in plan.frames["lights"]]
    master = work / "masters" / "stack_linear.fit"
    total = len(plan.steps)
    fingerprint = field_fingerprint(plan, siril)
    reused = reused_field(args, fingerprint, master)
    if reused is None:
        stacked = stack_poses(args, plan, report, runner, siril, names)
        recipe_lines, registration, normalisation, frame, index = stacked
    else:
        # The same poses, the same recipe: the master of the whole field kept by the last stacking is cut
        # again, nothing is stacked (``night_framing.py``).
        recipe_lines, registration, normalisation = reused["recipe"], reused["registration"], reused["normalisation"]
        frame = night_framing.frame_report(None, None, False)
        frame.update({**reused["frame"], "requested": args.frame_size})
        frame["rank"] = (args.frame_size if isinstance(args.frame_size, int) and frame["notches"]
                         and args.frame_size != frame["automatic"] else None)
        report.emit("note", code="field_reused", message="the master of the whole field kept by the last stacking "
                    "of these poses is cut to the frame asked for; nothing is stacked again")
        report.emit("registration", **registration_fields(registration, names, frame=frame))
        index = total - (2 if plan.with_linear else 1)
    if not master.exists():
        raise StackingError("integration_failed", "Siril did not write the linear master", EXIT_TOOL_FAILED)
    stacked_at = time.monotonic()
    kept = keep_integration(args, plan, report, master, registration)
    if reused is None and frame["notches"]:
        keep_field(args, master, fingerprint, recipe_lines, registration, names, normalisation, frame)
    rectangle = night_framing.chosen_rectangle(frame)
    if rectangle is not None:
        night_framing.crop_master(master, master, rectangle, bool(normalisation["outputNorm"]))
        report.emit("note", code="frame_cut", rank=frame["rank"], **rectangle,
                    message=f"the master of the whole field is cut to notch {frame['rank']} "
                            f"({rectangle['width']} x {rectangle['height']} px)")
    quality = measure_quality(master)
    notes: dict[str, object] = {}

    index += 1
    report.emit("step", step="linear_export" if plan.linear_only else "render", index=index, total=total)
    if plan.linear_only:
        outputs = linear_outputs(args, report, runner, master)
    else:
        outputs = render_outputs(args, plan, report, runner, master, starnet, graxpert, notes, siril)
        if plan.with_linear:
            index += 1
            report.emit("step", step="linear_export", index=index, total=total)
            outputs += linear_outputs(args, report, runner, master)
    finished = time.monotonic()

    from astropy.io import fits

    shape = fits.getheader(master, memmap=False)
    provenance = {
        "format": FORMAT_VERSION,
        "frames": {kind: len(files) for kind, files in plan.frames.items()},
        # The filters the flats name: one when the family got its own filter's flats.
        "flatsFilters": flats_filters(plan.frames["flats"]),
        "stackedFrames": int(shape.get("STACKCNT", shape.get("NCOMBINE", 0)) or 0),
        # The field rotation the registration measured, the framing it chose and the lights it left out.
        "registration": registration_fields(registration, names, frame=frame) if registration is not None else None,
        # The overscan cut from the working copies (null: the lights carry no TRIMSEC), and whether the stack
        # was rescaled to [0, 1] (false: a calibrated frame went under the floor).
        "overscan": overscan_note,
        "normalisation": normalisation,
        # A colour sensor (``colour`` below is the colour intensity of the finish).
        "colourSensor": plan.colour,
        # What the lights were: FITS frames, or camera RAW files (converted by Siril).
        "lightFormat": "raw" if raw_files.is_raw(plan.frames["lights"][0]) else "fits",
        "width": int(shape.get("NAXIS1", 0)),
        "height": int(shape.get("NAXIS2", 0)),
        # The target the renders were made for, and its pipelines (chosen target by target);
        # "No processing" is the id `linear`, alone or after the renders.
        "target": args.lights.parent.name,
        "pipelines": [{"id": pipeline.id, "version": pipeline.version} for pipeline in plan.pipelines]
        + ([{"id": LINEAR_PIPELINE, "version": None}] if plan.linear_only or plan.with_linear else []),
        "linearOnly": plan.linear_only,
        # The curve of presentation of the extra images: "none", "presentation" (gain 2.0) or "strong" (3.6);
        # the linear master kept (`integration`) and "No processing" never carry it.
        "contrast": "none" if plan.linear_only else getattr(args, "contrast", "none"),
        "colour": "none" if plan.linear_only else getattr(args, "colour", "none"),
        "softStars": "none" if plan.linear_only else getattr(args, "soft_stars", "none"),
        "finishes": finishes_fields(plan, args),
        "recipe": {
            "siril": [
                line for line in recipe_lines
                if line.split(" ", 1)[0] in ("stack", "calibrate", "register", "seqapplyreg", "convert")
            ],
            "render": None if plan.linear_only else {
                "gradientDegree": args.gradient_degree, "blackPercentile": args.black_percentile,
                "deconvFwhm": args.deconv_fwhm, "starSeparation": starnet is not None,
                # "Polished render", under "polished": {"engine": "graxpert-ai" | "siril" | "polynomial"
                # (+ "engineRequested", "fallbackReason" when it is not the one chosen), "background", "denoise",
                # "scnr" (amount, or null on a dual-band capture), "deconvolutionFwhm", "skyLevelShift"},
                # present only when that pipeline was rendered.
                **notes},
        },
        "tools": {"siril": siril_version(siril), "starnet2": starnet is not None, "graxpert": graxpert is not None},
        "seconds": {"stack": round(stacked_at - started, 1), "render": round(finished - stacked_at, 1),
                    "total": round(finished - started, 1)},
        "outputs": outputs,
        "integration": kept,
        # The quality of the image, read on the linear master before any render;
        # null: fewer than twenty stars, or a measure that raised.
        "quality": quality,
    }
    (out / f"{args.name}_stacking.json").write_text(json.dumps(provenance, indent=2) + "\n", encoding="utf-8")
    if not args.keep_work:
        shutil.rmtree(work)
    return provenance


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--lights", type=Path, help="the target's lights/ folder (read only)")
    parser.add_argument("--from-master", type=Path, metavar="FILE",
                        help="the image a device already stacked (a Seestar's Stacked_*.fit, read only): nothing is "
                             "stacked, the renders start from it; replaces --lights and the calibration folders")
    parser.add_argument("--frames-from", type=Path, metavar="FILE",
                        help="stack only the light frames listed in FILE (one path per line, each inside --lights)")
    parser.add_argument("--darks", type=Path, help="calibration/darks/ (read only)")
    parser.add_argument("--flats", type=Path, help="calibration/flats/ (read only)")
    parser.add_argument("--dark-flats", type=Path, help="calibration/dark-flats/ (read only)")
    parser.add_argument("--biases", type=Path, help="calibration/biases/ (read only)")
    parser.add_argument("--work", type=Path, required=True, help="scratch folder, created and removed by this script")
    parser.add_argument("--out", type=Path, required=True, help="the target's stacked/ folder")
    parser.add_argument("--name", required=True, help="output stem, e.g. M31_2026-10-07")
    parser.add_argument("--integration", metavar="FAMILY",
                        help="keep the linear master as <FAMILY>_integration.fit in --out (the family's label, "
                             "e.g. Ha_300s_g100): the only intermediate kept, for a multi-night plan")
    parser.add_argument("--pipeline", action="append", metavar="ID",
                        help=f"a render pipeline, repeatable (default: {DEFAULT_PIPELINE}); see list-pipelines")
    parser.add_argument("--pipelines-dir", type=Path, action="append", default=[], metavar="DIR",
                        help="a folder of <id>/pipeline.json definitions read after the shipped ones (repeatable)")
    parser.add_argument("--trusted", action="append", default=[], metavar="ID=SHA256",
                        help="a script digest the person approved for an added pipeline (repeatable); "
                             "an added pipeline that brings its own script is refused without it (exit code 2)")
    parser.add_argument("--linear-only", action="store_true",
                        help="no render pipeline (the option 'no processing'): write only the linear stack as "
                             f"<name>_{LINEAR_PIPELINE}.tif, 16-bit, not stretched; cannot be combined with --pipeline")
    parser.add_argument("--linear", action="store_true",
                        help="beside the render pipelines, also write the linear stack as "
                             f"<name>_{LINEAR_PIPELINE}.tif (the option 'no processing' chosen with others)")
    parser.add_argument("--siril", type=Path, help="siril-cli executable (default: found on PATH)")
    parser.add_argument("--starnet", type=Path, help="StarNet2 executable (default: starnet2 on PATH, if any)")
    parser.add_argument("--no-starnet", action="store_true", help="never separate the stars")
    parser.add_argument("--graxpert", type=Path,
                        help="GraXpert executable, for the pipelines that use it (default: graxpert on PATH, if any)")
    parser.add_argument("--no-graxpert", action="store_true",
                        help="never run GraXpert (the 'Polished render' then takes the polynomial background, no denoising)")
    parser.add_argument("--polished-engine", choices=("graxpert-ai", "siril"), default="graxpert-ai",
                        help="the background and denoising engine of the 'Polished render': GraXpert's AI models "
                             "(default; Siril takes over when GraXpert is missing or fails) or Siril's own subsky -rbf "
                             "and denoise, with no model")
    parser.add_argument("--capture-filter", metavar="NAME",
                        help="the filter of the lights, when their FITS header does not name it: a dual-band one "
                             "keeps the green of the OIII (no SCNR in the 'Polished render')")
    parser.add_argument("--contrast", type=render_finish.contrast_arg, default="none", metavar="none|presentation|strong|GAIN",
                        help="also write each image with an S-curve on the luminance around the sky level, as "
                             "<name>_<id>_presentation (gain 2.0) or _strong (gain 3.6), or with any gain from 0 to "
                             f"{render_finish.CONTRAST_MAX:g} (_contrast-2.5); the image without it stays (default: none)")
    parser.add_argument("--colour", type=render_finish.colour_arg, default="none", metavar="none|PERCENT",
                        help="also write each image with its colours stronger by this percent (0 to "
                             f"{render_finish.COLOUR_MAX:g}), the luminance kept, after the contrast; the name says it "
                             "(_colour-40; default: none)")
    parser.add_argument("--soft-stars", type=render_finish.stars_arg, default="none", metavar="none|PERCENT",
                        help="also write each image with this percent (0 to "
                             f"{render_finish.STARS_MAX:g}) of its brightest stars taken out (a top-hat mask, the image "
                             "eroded under it), after the contrast; the name says it (_stars-85; default: none)")
    parser.add_argument("--finish", type=render_finish.pipeline_finish_arg, action="append", default=[],
                        metavar="ID=CONTRAST,COLOUR,STARS",
                        help="the finish of one pipeline instead of --contrast, --colour and --soft-stars, each value "
                             "written as for them (hoo=strong,20,50; none,none,none: the render alone); repeatable, "
                             "one per pipeline, the last one of an id kept")
    parser.add_argument("--frame-size", type=night_framing.frame_size_arg, default=night_framing.AUTO,
                        metavar="auto|RANK",
                        help="the frame of the stacking when the field turns: the rank of a notch, 0 (the area every "
                             f"light covers) to {night_framing.NOTCH_COUNT - 1} (the whole field), or auto (default: "
                             "the common area when the field does not turn, the whole field when it does); the master "
                             f"of the whole field stays in --out as <name>{FIELD_SUFFIX}, cut again without stacking "
                             "by a later run on the same poses")
    parser.add_argument("--gradient-degree", type=int, choices=(1, 2), default=1)
    parser.add_argument("--black-percentile", type=float, default=0.05)
    parser.add_argument("--deconv-fwhm", type=float, default=None, metavar="PX")
    parser.add_argument("--keep-work", action="store_true", help="keep the scratch folder for inspection")
    parser.add_argument("--json", action="store_true", help="one JSON event per line on stdout")
    parser.add_argument("--dry-run", action="store_true", help="print the plan, write nothing")
    args = parser.parse_args(argv)
    try:
        trusted_scripts(args.trusted)
    except ValueError as error:
        parser.error(str(error))
    if args.from_master is not None:
        given = [flag for flag, value in (("--lights", args.lights), ("--frames-from", args.frames_from),
                                          ("--darks", args.darks), ("--flats", args.flats),
                                          ("--dark-flats", args.dark_flats), ("--biases", args.biases),
                                          ("--integration", args.integration)) if value is not None]
        if given:
            parser.error(f"--from-master stacks nothing: do not give {', '.join(given)} with it")
    elif args.lights is None:
        parser.error("one of --lights or --from-master is required")
    if args.linear_only and args.pipeline:
        parser.error("--linear-only runs no pipeline: do not give --pipeline with it")
    if args.linear_only and args.finish:
        parser.error("--linear-only runs no render: do not give --finish with it")
    if any(ident == LINEAR_PIPELINE for ident, _ in args.finish):
        parser.error(f"'{LINEAR_PIPELINE}' (No processing) takes no finish: the linear master is never touched")
    if args.linear_only and args.linear:
        parser.error("--linear adds the linear stack to renders: use --linear-only alone for no render")
    if any(sep in args.name for sep in ("/", "\\")) or args.name.startswith("."):
        parser.error("--name must be a plain file stem")
    if args.integration is not None and (
        any(sep in args.integration for sep in ("/", "\\")) or args.integration.startswith(".") or not args.integration
    ):
        parser.error("--integration must be a plain file stem")
    return args


def main(argv: list[str] | None = None) -> int:
    arguments = sys.argv[1:] if argv is None else argv
    if arguments and arguments[0] in ("list-pipelines", "--list-pipelines"):
        parser = argparse.ArgumentParser(prog="astro_sync_stack.py list-pipelines",
                                         description="Print the pipeline definitions that were loaded.")
        pipeline_defs.add_arguments(parser)
        return pipeline_defs.run_listing(parser.parse_args(arguments[1:]))
    args = parse_args(argv)
    report = Reporter(args.json)
    runner = Runner()
    signal.signal(signal.SIGTERM, runner.cancel)
    signal.signal(signal.SIGINT, runner.cancel)
    try:
        plan = build_plan(args)
        report.emit("plan", **plan_fields(plan, args))
        if args.dry_run:
            return EXIT_OK
        provenance = execute(args, plan, report, runner)
        report.emit("done", seconds=provenance["seconds"], outputs=provenance["outputs"])
        return EXIT_OK
    except Cancelled:
        report.emit("error", code="cancelled", message="stopped on request; the work folder is left for inspection")
        return EXIT_CANCELLED
    except StackingError as error:
        report.emit("error", code=error.code, message=str(error))
        return error.exit_code


if __name__ == "__main__":
    sys.exit(main())
