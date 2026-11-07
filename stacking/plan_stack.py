#!/usr/bin/env python3
"""The image of a multi-night plan: the masters of several nights into one image.

Astro Sync writes a job (``--job FILE``, JSON, ``astro_sync_core::plan_image::
PlanImageJob``) naming the masters kept by the stacking of each night
(``<night>/<target>/stacked/<family>_integration.fit``), the reference one
(the longest), the render chosen and the plan's folder.  This script then:

1. **aligns** the masters on the reference night: Siril converts them into a
   sequence, the WCS is removed from the working copies (as for a night), and
   Siril registers them in two passes with an affine transform
   (``register -2pass -transf=affine``).  The framing is then chosen as the
   stacking of a night chooses it (``registration_framing.py``, one rule for
   both): the field rotation is measured over all the frames of all the nights
   (the orientation of each master on the reference one plus the rotation
   within its night, kept in the master's header; a meridian flip is no
   rotation), the common area when it is under 3°, the whole frame of the
   reference night (``-framing=current``) above: two alt-azimuth nights
   oriented differently have no useful common area.  The transform is applied
   with Lanczos4.  A master Siril cannot align is left out of the plan and named;
   the plan stops only when fewer than two masters are left;
   A night that only has the image its device stacked (a Seestar's ``Stacked_*.fit``, job ``source: "device"``)
   is converted first (``device_master.py``: 32-bit floats, divided by 65535, as our masters are) so that
   Siril registers masters of one depth; such an image says nothing of how the field turned during its night, so
   the plan then keeps the whole field of the reference night;
2. **combines** them (``plan_combine.py``): each master brought to the level
   of the longest of its band, the masters of a band averaged with a weight
   equal to their integration time.
   The scale between two masters is read on the light of the stars in an aperture (``level_match.py``), for every
 plan, never on the sharp peaks of the stars;

   **A plan of a colour sensor with a dual-band master** takes neither step 1 nor step 2: it follows the recipe of
   ``dualband_reference.py``, so that the same masters give the same image to the bit.
   The masters of each band are registered from their stars on the longest one of the band and averaged, then
   the broadband master is registered on the dual-band one and added to it at 0.18; the frame of the image is
   the dual-band master's, and what the broadband master does not cover is black.  The ``registration`` event
   says the same as Siril's (``method: "stars"``);

   **The crop** (``plan_crop.py``), whatever the path: the linear image is cut to an axis-aligned rectangle the
   job's ``planField`` chooses -- ``clean`` (the default: the largest without a pixel no master covers, so no black
   bevel), ``common`` (the largest every master retained covers) or ``whole`` (nothing cut).  It is cut before the render, so every render
   and every finish of the plan has the same rectangle; the ``crop`` event and the record say it (the pixels
   lost on each edge, the share of the frame kept), and the ``CRPIX`` of a WCS moves with it.  A milestone of
   the progression is given the frame master and the rectangle of the image of the plan (``frameMaster``,
   ``crop``): it is registered on that master when its own frame is another one, then cut the same way;

   **The display** (``renders.prepare_native``): the black point (per channel) and the stretch scale a render shows
   an image with are measured on it, and the record says them (``display``: ``black``, ``scale``, ``imposed``).  A
   milestone of the progression is given those of the image of the plan (job ``display``:
   ``{"black": [r, g, b], "scale": s}``) and shows its poses at the same level -- a stack of one night is
   noisier, its own percentile would fall lower and its sky would look brighter --; ``imposed`` is then true in its
   record.  The natural and the polished preparations have it -- the polished one also records the sky levels its
   two matches aim at (``sky``, ``finalWithStars``), and aims at the imposed ones instead of the natural render
   of its own, noisier, poses --; "No processing" has none (``display: null``);
3. **renders** the result like the stacking of a night does
   (``astro_sync_stack.render_outputs``: "Natural render", "Dual-band
   correction", or a pipeline the person added) or, for "No
   processing", writes the linear result as a 16-bit TIFF
   (``linear_export.py``).

4. **frames**: the image is always made whole and kept in ``out``; when the job names a
   ``frame`` (the four integers of a notch of ``coverage.framings`` in the record of an earlier image), every image
   written is cropped to it after the render (``plan_framing.py``), as ``<file>_framed.<ext>``: nothing is stacked
   again, and the whole image stays.  The record keeps the notches in ``coverage.framings`` and a small
   ``<record>.preview.png`` of the whole image for the screen.

What it reads: the masters, never modified.  What it writes: a scratch tree
in the job's ``work`` folder (``<plan>/.astro-sync-work/plan-image/``,
marked, removed at the end unless ``--keep-work``), and in the job's ``out``
folder (``<plan>/stacked/``) the render and ``<name>_plan.json`` (``<name>.json`` when the name already ends in ``_plan``), the record
of what was combined.  Nothing leaves the computer.

``--json`` prints the events of ``astro_sync_stack.py`` (``plan``, ``step``,
``registration`` -- the same as a night's, in ``masters`` instead of ``lights``: the
rotation measured, the framing chosen and the nights left out --, ``crop`` -- the rectangle kept --, ``coverage`` -- the share of the
frame without dual-band data, only when it is not nil --, ``output``, ``done``, ``error``), one JSON object per line; ``--dry-run``
prints the plan and writes nothing.  Exit codes and error codes are those of
``astro_sync_stack.py``; ``bad_job`` for a job that cannot be read,
``combination_failed`` when the masters cannot be combined.
"""

from __future__ import annotations

import argparse
import json
import math
import shutil
import signal
import sys
import time
from dataclasses import dataclass
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import astro_sync_stack as stack  # noqa: E402
import device_master  # noqa: E402
import dualband_reference  # noqa: E402
import pipeline_defs  # noqa: E402
import plan_combine  # noqa: E402
import plan_crop  # noqa: E402
import plan_framing  # noqa: E402
import registration_framing  # noqa: E402
import render_finish  # noqa: E402
import siril_config  # noqa: E402

FORMAT_VERSION = 1
SEQUENCE = "plan"
PREVIEW_SUFFIX = ".preview.png"


def read_job(path: Path) -> dict[str, object]:
    """The job Astro Sync wrote; ``bad_job`` when it cannot be used."""
    try:
        job = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, ValueError) as error:
        raise stack.StackingError("bad_job", f"the job {path} cannot be read: {error}", stack.EXIT_INPUT) from error
    masters = job.get("masters") or []
    if not masters:
        raise stack.StackingError("bad_job", "the job names no master", stack.EXIT_INPUT)
    for master in masters:
        if not Path(master["path"]).is_file():
            raise stack.StackingError("folder_missing", f"the master {master['path']} is not here", stack.EXIT_INPUT)
        if master.get("source", plan_combine.OWN) not in (plan_combine.OWN, plan_combine.DEVICE):
            raise stack.StackingError("bad_job", f"the source '{master['source']}' is not master or device", stack.EXIT_INPUT)
    if not 0 <= int(job.get("reference", 0)) < len(masters):
        raise stack.StackingError("bad_job", "the reference is not one of the masters", stack.EXIT_INPUT)
    for key in ("out", "work", "name"):
        if not job.get(key):
            raise stack.StackingError("bad_job", f"the job has no {key}", stack.EXIT_INPUT)
    if Path(str(job["out"])).name.lower() == "final":
        raise stack.StackingError("output_is_final", "the image of the plan never goes to final/", stack.EXIT_INPUT)
    try:
        plan_crop.mode_of(job)
    except ValueError as error:
        raise stack.StackingError("bad_job", str(error), stack.EXIT_INPUT) from error
    return job


def imposed_display(job: dict[str, object]) -> dict[str, object] | None:
    """The display a milestone is shown with (job ``display``), ``None``: its own.

    ``{"black": [r, g, b], "scale": s}``, and for the polished render the sky levels its matches aim at:
    ``"sky": {"displayed": d, "final": f}`` and ``"finalWithStars": t``.  ``bad_job`` when it is not three finite
    black points, a finite positive scale and finite levels.
    """
    display = job.get("display")
    if display is None:
        return None
    try:
        black = [float(v) for v in display["black"]]
        scale = float(display["scale"])
        if len(black) != 3 or not all(math.isfinite(v) for v in black) or not (math.isfinite(scale) and scale > 0):
            raise ValueError("expected three black points and a scale")
        used: dict[str, object] = {"black": black, "scale": scale}
        if display.get("sky") is not None:
            used["sky"] = {key: float(display["sky"][key]) for key in ("displayed", "final")}
        if display.get("finalWithStars") is not None:
            used["finalWithStars"] = float(display["finalWithStars"])
        levels = [*used.get("sky", {}).values(), used.get("finalWithStars", 0.0)]
        if not all(math.isfinite(v) for v in levels):
            raise ValueError("a sky level is not finite")
    except (KeyError, TypeError, ValueError) as error:
        raise stack.StackingError("bad_job", f"the display of the job cannot be used: {error}", stack.EXIT_INPUT) from error
    return used


def render_args(job: dict[str, object]) -> argparse.Namespace:
    """The options ``astro_sync_stack``'s render steps read, from the job."""
    pipeline = job.get("pipeline") or {}
    pipeline_id = str(pipeline.get("pipelineId") or stack.DEFAULT_PIPELINE)
    linear_only = pipeline_id == stack.LINEAR_PIPELINE
    try:
        finish = render_finish.checked(job.get("contrast"), job.get("colour"), job.get("softStars"))
    except ValueError as error:
        raise stack.StackingError("bad_job", str(error), stack.EXIT_INPUT) from error
    contrast, colour, soft_stars = finish.contrast, finish.colour, finish.stars
    digest = pipeline.get("trustedSha256")
    return argparse.Namespace(
        work=Path(str(job["work"])),
        out=Path(str(job["out"])),
        name=str(job["name"]),
        pipeline=None if linear_only else [pipeline_id],
        pipelines_dir=[Path(p) for p in job.get("pipelinesDirs") or []],
        trusted=[f"{pipeline_id}={str(digest).lower()}"] if digest else [],
        linear_only=linear_only,
        contrast=contrast,
        colour=colour,
        soft_stars=soft_stars,
        siril=Path(str(job["siril"])) if job.get("siril") else None,
        starnet=Path(str(job["starnet"])) if job.get("starnet") else None,
        no_starnet=False,
        graxpert=Path(str(job["graxpert"])) if job.get("graxpert") else None,
        no_graxpert=False,
        gradient_degree=1,
        black_percentile=0.05,
        deconv_fwhm=None,
        keep_work=False,
        # The display (black point and stretch scale) the image is shown with: the job's, or measured on the image;
        # ``measured_display`` receives the pair used, for the record.
        display=imposed_display(job),
        measured_display={},
    )


def display_of(args: argparse.Namespace) -> dict[str, object] | None:
    """What the render showed the image with, for the record; ``None`` when the render did not use a shared display."""
    used = args.measured_display
    if args.linear_only or "black" not in used or "scale" not in used:
        return None
    return {**used, "imposed": args.display is not None}


def masters_of(job: dict[str, object]) -> list[plan_combine.Master]:
    return [
        plan_combine.Master(
            night_id=str(m["nightId"]),
            evening_date=str(m["eveningDate"]),
            filter=m.get("filter"),
            band=str(m.get("band") or plan_combine.BROADBAND),
            seconds=float(m.get("totalSeconds") or 0.0),
            source=str(m.get("source") or plan_combine.OWN),
        )
        for m in job["masters"]
    ]


def steps_of(job: dict[str, object], args: argparse.Namespace) -> list[str]:
    steps = ["alignment"] if len(job["masters"]) > 1 else []
    return steps + ["combination", "linear_export" if args.linear_only else "render"]


def alignment_scripts(reference: int, framing: str = registration_framing.COMMON_AREA) -> list[tuple[str, str]]:
    """The Siril scripts of the alignment, run from the work folder.

    ``register`` only computes the transforms; ``apply`` brings the masters onto
    the frame ``framing`` names, chosen from what ``register`` measured (as a
    night's stacking does, ``astro_sync_stack.siril_scripts``).

    ``register -2pass`` picks its own reference image (by image quality and framing:
    Siril's log says « we are choosing image N as new reference »), whatever
    ``setref`` said before it.  The common area does not depend on it.  The whole
    frame (``-framing=current``) is the frame of the reference image, so ``apply``
    sets the job's reference again first: the image of the plan has the frame of the
 reference night (the longest night without a dual-band filter, so
    that no pixel lacks its broadband colour).
    """
    convert = "\n".join(["requires 1.4.0", "cd input", f"convert {SEQUENCE} -out=../process", "close"]) + "\n"
    register = "\n".join([
        "requires 1.4.0",
        "cd process",
        f"setref {SEQUENCE} {reference + 1}",
        f"register {SEQUENCE} -2pass -transf=affine -minpairs=12 -maxstars=2000",
        "close",
    ]) + "\n"
    again = [f"setref {SEQUENCE} {reference + 1}"] if framing == registration_framing.FULL_FIELD else []
    apply = "\n".join(["requires 1.4.0", "cd process", *again, registration_framing.apply_command(SEQUENCE, framing), "close"]) + "\n"
    return [("convert", convert), ("register", register), ("apply", apply)]


def master_labels(job: dict[str, object]) -> list[str]:
    """What each master is called in the notes: its evening, its night, its filter."""
    return [
        " · ".join(str(part) for part in (m["eveningDate"], m["nightId"], m.get("filter")) if part)
        for m in job["masters"]
    ]


def master_rotation(path: Path, source: str = plan_combine.OWN) -> tuple[float, float] | None:
    """The range of the field rotation of the frames a master integrates (its ``ASROTMIN`` and ``ASROTMAX``).

    A master of ours that does not say (kept before the measure, or when it could not be
    made) stands for a field that did not turn: ``(0, 0)``.  The image of a device never
    says -- it is an alt-azimuth stack as often as not -- so its range is **unknown**
 (``None``) and the plan keeps the whole field.
    """
    if source == plan_combine.DEVICE:
        return None
    from astropy.io import fits

    header = fits.getheader(path, memmap=False)
    try:
        return float(header["ASROTMIN"]), float(header["ASROTMAX"])
    except (KeyError, ValueError):
        return 0.0, 0.0


@dataclass
class Alignment:
    """The masters brought onto the reference night."""

    # The aligned files, and the position in the job of each master they come from.
    paths: list[Path]
    kept: list[int]
    # What the registration said, over all the frames of all the nights; ``None`` for one master alone.
    registration: registration_framing.Registration | None
    # What each master of the job is, as the alignment read it (a device's image converted to a master),
    # in the job's order: the header of the plan's image comes from the reference one.
    inputs: list[Path]


@dataclass
class Combined:
    """The linear image of the masters, on the frame of one of them, before it is cut and written."""

    image: object  # ``(3, h, w)`` or ``(h, w)`` float32
    header: object  # the header of the master whose frame the image is on
    masters: list[plan_combine.Master]  # the masters retained, in the order of ``covers``
    covers: list[object]  # the pixels each covers, on the image's frame
    covered: object  # the pixels at least one master covers (the others have no data)
    colour: bool
    natural: int  # the position in the job of the master whose frame the image is on
    description: dict[str, object]  # what the combination did (``noData`` and ``coverage`` are set after the crop)


def align(job: dict[str, object], args: argparse.Namespace, report: stack.Reporter,
          runner: stack.Runner, index: int, total: int) -> Alignment:
    """Step 1: the masters on the reference night, in the job's order, those Siril cannot align left out and named."""
    masters = [Path(str(m["path"])) for m in job["masters"]]
    if len(masters) == 1:
        return Alignment(masters, [0], None, masters)
    work = args.work
    siril = stack.find_tool(args.siril, ("siril-cli", "siril"), "siril_missing")
    report.emit("step", step="alignment", index=index, total=total)
    folder = work / "input"
    folder.mkdir(parents=True)
    kinds = [str(m.get("source") or plan_combine.OWN) for m in job["masters"]]
    inputs: list[Path] = []
    for number, (source, kind) in enumerate(zip(masters, kinds), 1):
        link = folder / f"{SEQUENCE}_{number:05d}.fit"
        if kind == plan_combine.DEVICE:
            # The device's 16-bit cube becomes a 32-bit master (÷ 65535) before Siril reads it: Siril does not
            # register sequences of mixed depths.
            try:
                device_master.convert(source, link)
            except device_master.DeviceImageError as error:
                code = "device_stretched" if error.code == "stretched" else "device_unreadable"
                raise stack.StackingError(code, f"{source.name}: {error}", stack.EXIT_INPUT) from error
        else:
            stack.link_or_copy(source, link)
        inputs.append(link)
    (work / "process").mkdir()
    log = work / "siril.log"
    reference = int(job["reference"])
    registration: registration_framing.Registration | None = None
    scripts = alignment_scripts(reference)
    for number, (name, script) in enumerate(scripts, 1):
        if name == "apply":
            script = dict(alignment_scripts(reference, registration.framing))["apply"]
        script_file = work / f"{number:02d}_{name}.ssf"
        script_file.write_text(script, encoding="utf-8")
        if runner.run(siril_config.command(siril, work, script_file), log) != 0:
            raise stack.StackingError("registration_failed", f"Siril failed at {name}; see siril.log", stack.EXIT_TOOL_FAILED)
        if name == "convert":
            stack.strip_wcs(work / "process", SEQUENCE)
        if name == "register":
            registration = stack.measured_registration(work / "process" / f"{SEQUENCE}_.seq", len(masters), stack.MASTERS)
            if reference + 1 in registration.unregistered:
                raise stack.StackingError("registration_failed", "the reference night's master could not be registered", stack.EXIT_TOOL_FAILED)
            registration = registration_framing.combine_nights(
                registration, [master_rotation(source, kind) for source, kind in zip(masters, kinds)])
            report.emit("registration", **stack.registration_fields(registration, master_labels(job), stack.MASTERS))
    aligned = sorted((work / "process").glob(f"r_{SEQUENCE}_*.fit"))
    kept = [position for position in range(len(masters)) if position + 1 not in registration.unregistered]
    if len(aligned) != len(kept):
        raise stack.StackingError(
            "registration_failed",
            f"{len(kept) - len(aligned)} master(s) of {len(kept)} could not be applied on the reference night",
            stack.EXIT_TOOL_FAILED,
        )
    return Alignment(aligned, kept, registration, inputs)


def combine(job: dict[str, object], aligned: Alignment) -> Combined:
    """Step 2: one linear image of the masters that were aligned (written by :func:`finish_combination`)."""
    import numpy as np
    from astropy.io import fits

    images = [np.asarray(fits.getdata(path, memmap=False), dtype=np.float32) for path in aligned.paths]
    masters = masters_of(job)
    kept = [masters[position] for position in aligned.kept]
    try:
        image, description = plan_combine.combine(kept, images)
    except ValueError as error:
        raise stack.StackingError("combination_failed", str(error), stack.EXIT_TOOL_FAILED) from error
    # The pixels each master covers, as the combination read them: the crop and the record are measured on them.
    covers = [plan_combine.coverage(image) for image in images]
    header = fits.getheader(aligned.inputs[int(job["reference"])], memmap=False)
    header["HISTORY"] = "Astro Sync: image of a multi-night plan (plan_stack.py)"
    colour = images[0].ndim == 3 and images[0].shape[0] == 3
    return Combined(image=image, header=header, masters=kept, covers=covers,
                    covered=np.logical_or.reduce(covers), colour=colour, natural=int(job["reference"]),
                    description=description)


def record_name(name: str) -> str:
    """The file of the record of the combination: ``<name>_plan.json``, and ``<name>.json`` when the name already says "plan"."""
    return f"{name}.json" if name.endswith("_plan") else f"{name}_plan.json"


def plan_fields(job: dict[str, object], args: argparse.Namespace) -> dict[str, object]:
    pipeline = stack.LINEAR_PIPELINE if args.linear_only else args.pipeline[0]
    outputs = [pipeline_defs.linear_file_name(args.name)] if args.linear_only else [f"{args.name}_{pipeline}.tif"]
    finish = stack.finish_of(args)
    if not finish.is_plain() and not args.linear_only:
        # "No processing" has no twin.
        outputs.append(f"{stack.finish_name(outputs[0].removesuffix('.tif'), finish)}.tif")
    if job.get("jpeg") and not args.linear_only:
        outputs.append(f"{args.name}_{pipeline}.jpg")
    return {
        "format": FORMAT_VERSION,
        "masters": len(job["masters"]),
        "nights": sorted({str(m["nightId"]) for m in job["masters"]}),
        "reference": int(job["reference"]),
        "pipeline": pipeline,
        "linearOnly": args.linear_only,
        "contrast": "none" if args.linear_only else args.contrast,
        "colour": "none" if args.linear_only else args.colour,
        "softStars": "none" if args.linear_only else args.soft_stars,
        "steps": steps_of(job, args),
        "outputs": outputs + [record_name(args.name)],
    }


FRAMED_SUFFIX = "_framed"
CROPPABLE = (".tif", ".tiff", ".png")


def frame_outputs(job: dict[str, object], args: argparse.Namespace, report: stack.Reporter,
                  outputs: list[dict[str, object]], coverage: dict[str, object] | None) -> dict[str, int] | None:
    """Step 4: the rendered images cut to the frame the job names, written beside the whole ones.

    The whole image is the one the render wrote and it stays in ``out``.  The frame is the four integers
    of a notch of ``coverage.framings``; each TIFF or PNG output gets a twin ``<name>_framed.<ext>`` (reported
    as an ``output`` of the same pipeline and contrast, appended to ``outputs``).  Returns the frame cut, or
    ``None`` when the job asks for none or for the whole image.  A frame that does not fit the image of
    the plan is an error (``bad_job``): sending the whole image under a choice of a smaller one would
    be a silent lie.  The notches are measured on the image as it is written (cut by ``crop``): a frame is one of
    the notches of an image of the same nights and the same ``planField`` (Astro Sync reads them so).
    """
    asked = job.get("frame")
    if not asked or not coverage:
        return None
    size = coverage["fullCoverage"]
    height, width = int(size["frameHeight"]), int(size["frameWidth"])
    frame = plan_framing.valid_frame(asked, height, width)
    if frame is None:
        raise stack.StackingError(
            "bad_job", f"the frame {asked} does not fit the image of the plan ({width} x {height})", stack.EXIT_INPUT)
    if plan_framing.is_whole(frame, height, width):
        return None
    for entry in list(outputs):
        path = args.out / str(entry["file"])
        if path.suffix.lower() not in CROPPABLE:
            continue
        framed = path.with_name(f"{path.stem}{FRAMED_SUFFIX}{path.suffix}")
        try:
            plan_framing.crop_image(path, framed, frame)
        except (OSError, ValueError) as error:
            raise stack.StackingError("render_failed", f"the frame could not be cut: {error}", stack.EXIT_TOOL_FAILED) from error
        outputs.append(stack.write_output(report, str(entry["pipeline"]), framed, str(entry.get("contrast") or "none"),
                                          str(entry.get("colour") or "none"), str(entry.get("softStars") or "none")))
    return frame


def write_jpegs(job: dict[str, object], args: argparse.Namespace, report: stack.Reporter,
                outputs: list[dict[str, object]]) -> None:
    """The JPEG of the image, beside it, when the job asks (``"jpeg": true``: a milestone of the progression of a plan).

    The picture is made from the image without a curve of contrast (the PNG, else the TIFF), whole;
    it is reported as an ``output`` of the same pipeline.
    """
    if not job.get("jpeg"):
        return
    candidates = [e for e in outputs if not e.get("contrast") and "_framed" not in str(e["file"])
                  and Path(str(e["file"])).suffix.lower() in CROPPABLE]
    candidates.sort(key=lambda e: Path(str(e["file"])).suffix.lower() != ".png")
    for entry in candidates[:1]:
        source = Path(args.out) / str(entry["file"])
        target = source.with_suffix(".jpg")
        try:
            plan_framing.write_jpeg(source, target)
        except (OSError, ValueError) as error:
            raise stack.StackingError("render_failed", f"the JPEG could not be written: {error}", stack.EXIT_TOOL_FAILED) from error
        outputs.append(stack.write_output(report, str(entry["pipeline"]), target))


def write_preview(args: argparse.Namespace, outputs: list[dict[str, object]], coverage: dict[str, object] | None) -> str | None:
    """A small picture of the whole image beside the record, when the person has a frame to choose.

    Returns its file name.  The preview is for the screen (where the frame sits on the image): it is not an
    output and never leaves the computer.  A failure to draw it is not one of the plan.
    """
    if not coverage or not coverage.get("framings"):
        return None
    sources = [Path(args.out) / str(e["file"]) for e in outputs
               if not (e.get("contrast") or e.get("colour") or e.get("softStars")) and Path(str(e["file"])).suffix.lower() in CROPPABLE]
    if not sources:
        return None
    name = Path(record_name(args.name)).stem + PREVIEW_SUFFIX
    try:
        plan_framing.write_preview(sources[0], Path(args.out) / name)
    except (OSError, ValueError) as error:
        print(f"the preview of the plan could not be drawn: {error}", file=sys.stderr)
        return None
    return name


def is_colour(path: Path) -> bool:
    """A master of three planes (the layout of a colour FITS)."""
    from astropy.io import fits

    header = fits.getheader(path, memmap=False)
    return int(header.get("NAXIS", 0)) == 3 and int(header.get("NAXIS3", 0)) == 3


def on_dualband(job: dict[str, object]) -> bool:
    """The plan is combined on its dual-band master (``dualband_reference.py``): a colour sensor and at least
    one dual-band master.  A device's image is colour (it is converted to a master of three planes)."""
    masters = job["masters"]
    if not any(str(m.get("band")) == plan_combine.DUALBAND for m in masters):
        return False
    return all(str(m.get("source") or plan_combine.OWN) == plan_combine.DEVICE or is_colour(Path(str(m["path"])))
               for m in masters)


def read_inputs(job: dict[str, object], work: Path) -> list[Path]:
    """The masters as files of 32-bit floats: ours as they are, a device's image converted (``device_master``)."""
    inputs: list[Path] = []
    folder = work / "input"
    for number, master in enumerate(job["masters"], 1):
        source = Path(str(master["path"]))
        if str(master.get("source") or plan_combine.OWN) == plan_combine.DEVICE:
            folder.mkdir(parents=True, exist_ok=True)
            target = folder / f"{SEQUENCE}_{number:05d}.fit"
            try:
                device_master.convert(source, target)
            except device_master.DeviceImageError as error:
                code = "device_stretched" if error.code == "stretched" else "device_unreadable"
                raise stack.StackingError(code, f"{source.name}: {error}", stack.EXIT_INPUT) from error
            inputs.append(target)
        else:
            inputs.append(source)
    return inputs


def combine_on_dualband(job: dict[str, object], args: argparse.Namespace, report: stack.Reporter,
                        total: int) -> tuple[Combined, dict[str, object]]:
    """Steps 1 and 2 of a plan with a dual-band master of a colour sensor: the recipe of ``dualband_reference.py``.

    The dual-band masters are registered from their stars on the longest of them and averaged
    (``dualband_reference.merge_band``); then every broadband master is registered on that dual-band master,
    levelled on the longest broadband one, averaged where any covers, and added at 0.18
    (``dualband_reference.fuse_band``): the frame of the image is the dual-band master's.
    Siril does not take part.  A master no transform can be found for is left out and named; the plan stops
    when fewer than two are left.  Returns the combined image (cut and written by :func:`finish_combination`)
    and the ``registration`` block (the same fields as Siril's alignment).
    """
    import numpy as np
    from astropy.io import fits

    report.emit("step", step="alignment", index=1, total=total)
    inputs = read_inputs(job, args.work)
    masters = masters_of(job)
    labels = master_labels(job)
    images = [np.asarray(fits.getdata(path, memmap=False), dtype=np.float32) for path in inputs]
    frames = [int(m.get("frameCount") or 0) for m in job["masters"]]
    dual = [i for i, m in enumerate(masters) if m.band == plan_combine.DUALBAND]
    broad = [i for i, m in enumerate(masters) if m.band != plan_combine.DUALBAND]

    def band(indices: list[int]) -> tuple[np.ndarray, dualband_reference.BandMerge]:
        return dualband_reference.merge_band([images[i] for i in indices], [masters[i].seconds for i in indices],
                                             [frames[i] for i in indices])

    left_out: list[int] = []
    rotations = [0.0]
    levels: list[dict[str, object]] = []
    dual_image, dual_merge = band(dual)
    left_out += [dual[i] for i in dual_merge.left_out]
    reference = dual[dual_merge.reference]
    covers: dict[int, np.ndarray] = {dual[i]: cover for i, cover in dual_merge.covers.items()}
    for entry in dual_merge.merged:
        rotations.append(float(entry["rotationDegrees"]))
        levels.append({"nightId": masters[dual[int(entry["index"])]].night_id, "to": masters[reference].night_id,
                       "method": "percentile99", "scales": [round(s, 5) for s in entry["scales"]], "stars": entry["stars"]})
    image = dual_image
    valid = np.ones(dual_image.shape[:2], bool)
    fusion_record: dict[str, object] | None = None
    # The share of each master in the image: its weight within its band (the broadband band counts 0.18).
    kept_dual = [k for k, i in enumerate(dual) if i not in left_out]
    shares = {dual[k]: dual_merge.weights[k] / sum(dual_merge.weights[j] for j in kept_dual) for k in kept_dual}
    if broad:
        # Each broadband master is registered directly on the dual-band frame (not on the longest broadband
        # one first): a broadband night taken with the camera turned would otherwise crop the image to its
        # own footprint, although the other broadband nights cover the rest.
        try:
            image, valid, fusion, broad_merge = dualband_reference.fuse_band(
                dual_image, [images[i] for i in broad], [masters[i].seconds for i in broad], [frames[i] for i in broad])
        except dualband_reference.RegistrationError:
            left_out += [i for i in broad if i not in left_out]
        else:
            left_out += [broad[i] for i in broad_merge.left_out]
            broad_reference = broad[broad_merge.reference]
            rotations.append(fusion.rotation_degrees)
            for entry in broad_merge.merged:
                rotations.append(fusion.rotation_degrees + float(entry["rotationDegrees"]))
                levels.append({"nightId": masters[broad[int(entry["index"])]].night_id, "to": masters[broad_reference].night_id,
                               "method": "percentile99", "scales": [round(s, 5) for s in entry["scales"]], "stars": entry["stars"]})
            levels.append({"nightId": masters[broad_reference].night_id, "to": masters[reference].night_id,
                           "method": "percentile99", "scales": [round(s, 5) for s in fusion.scales], "stars": fusion.stars})
            fusion_record = {"method": "dualband_reference", "weight": fusion.weight, "scales": fusion.scales,
                             "stars": fusion.stars, "coverage": fusion.coverage}
            # What no broadband master covers is black: no master has data there.
            for i in list(covers):
                covers[i] = covers[i] & valid
            kept_broad = [k for k, i in enumerate(broad) if i not in left_out]
            total_weight = sum(broad_merge.weights[k] for k in kept_broad)
            for k in kept_broad:
                covers[broad[k]] = broad_merge.covers[k]  # already on the dual-band frame
                shares[broad[k]] = fusion.weight * broad_merge.weights[k] / total_weight
    kept = [i for i in range(len(masters)) if i not in left_out]
    if len(kept) < 2:
        raise stack.StackingError("registration_failed",
                                  "fewer than two masters could be registered on the dual-band one", stack.EXIT_TOOL_FAILED)
    span = max(rotations) - min(rotations)
    registration = {
        "unit": stack.MASTERS, "frames": len(masters), "registered": len(kept),
        "unregistered": [labels[i] for i in left_out],
        "fieldRotationDegrees": round(span, 2),
        "fieldRotation": "progressive" if span >= registration_framing.ROTATION_THRESHOLD_DEGREES else "none",
        # The whole frame of the dual-band master, its edges covered by fewer nights.
        "framing": registration_framing.FULL_FIELD,
        "method": "stars",
    }
    report.emit("registration", **registration)
    weights = [{"nightId": masters[i].night_id, "filter": masters[i].filter, "band": masters[i].band,
                "seconds": masters[i].seconds, "weight": round(shares[i], 4),
                **({"source": masters[i].source} if masters[i].source != plan_combine.OWN else {})}
               for i in kept]
    header = fits.getheader(inputs[reference], memmap=False)
    header["HISTORY"] = "Astro Sync: image of a multi-night plan on its dual-band master (plan_stack.py)"
    # The reference master counts as covering its whole frame in the recipe; the empty borders of its own
    # night (a field that turned) are no data for the crop.
    covers[reference] = covers[reference] & dualband_reference.data_of(dualband_reference.to_pixels(images[reference]))
    description = {
        "method": "dualband_reference",
        "reference": masters[reference].night_id,
        "masters": weights,
        **({"fusion": fusion_record} if fusion_record else {}),
        "levelMatch": levels,
    }
    combined = Combined(image=dualband_reference.to_planes(image), header=header, masters=[masters[i] for i in kept],
                        covers=[covers[i] for i in kept], covered=valid & covers[reference], colour=True,
                        natural=reference, description=description)
    return combined, registration


FRAME_MASTER_FILE = "frame-master.fit"


def same_file(a: object, b: object) -> bool:
    """Two paths of one file (a link, a relative path)."""
    try:
        return Path(str(a)).resolve() == Path(str(b)).resolve()
    except OSError:
        return str(a) == str(b)


def onto_frame_master(job: dict[str, object], args: argparse.Namespace, combined: Combined) -> dict[str, object]:
    """A milestone of the progression brought onto the frame of the image of the plan (``frameMaster`` of the job).

    The milestone's own frame is the master its nights give it (the longest of them); the image of the plan
    may be on another night's frame.  The combined image is registered on that master from its stars
    (``astroalign``, as the recipe registers the masters) and resampled; what each master covers is carried
    with it.  Changes ``combined`` in place and returns what was done; :class:`dualband_reference.RegistrationError`
    when no transform is found.
    """
    import astroalign
    import numpy as np
    from astropy.io import fits
    from renders import remove_gradient

    target = job["frameMaster"]
    path = Path(str(target["path"]))
    if str(target.get("source") or plan_combine.OWN) == plan_combine.DEVICE:
        converted = args.work / FRAME_MASTER_FILE
        try:
            device_master.convert(path, converted)
        except device_master.DeviceImageError as error:
            raise dualband_reference.RegistrationError(str(error)) from error
        path = converted
    reference = np.asarray(fits.getdata(path, memmap=False), dtype=np.float32)
    header = fits.getheader(path, memmap=False)

    def pixels(array: np.ndarray) -> np.ndarray:
        return dualband_reference.to_pixels(array) if array.ndim == 3 else np.repeat(array[..., None], 3, axis=-1)

    try:
        transform, (matched, _) = astroalign.find_transform(
            dualband_reference.alignment_view(remove_gradient(pixels(np.asarray(combined.image)))),
            dualband_reference.alignment_view(remove_gradient(pixels(reference))),
            max_control_points=dualband_reference.MERGE_CONTROL_POINTS,
            detection_sigma=dualband_reference.DETECTION_SIGMA,
        )
    except (astroalign.MaxIterError, ValueError) as error:
        raise dualband_reference.RegistrationError(str(error)) from error
    shape = reference.shape[-2:]
    source = dualband_reference.to_pixels(combined.image) if combined.colour else np.asarray(combined.image, np.float32)
    target_like = np.zeros(shape + source.shape[2:], np.float32)
    moved, footprint = astroalign.apply_transform(transform, source, target_like, propagate_mask=True)
    inside = ~footprint
    moved = np.where(inside[..., None] if moved.ndim == 3 else inside, moved, 0.0)
    combined.image = dualband_reference.to_planes(moved) if combined.colour else moved.astype(np.float32)
    combined.covers = [dualband_reference.carried(cover, transform, shape) & inside for cover in combined.covers]
    combined.covered = dualband_reference.carried(combined.covered, transform, shape) & inside
    header["HISTORY"] = "Astro Sync: milestone of a plan on the frame of the image of the plan (plan_stack.py)"
    combined.header = header
    return {"nightId": target.get("nightId"), "stars": len(matched),
            "rotationDegrees": round(math.degrees(float(transform.rotation)), 3)}


def frame_master_of(job: dict[str, object], combined: Combined, crop: dict[str, object]) -> dict[str, str]:
    """The master whose frame the image is on: the job's ``frameMaster`` when a milestone was brought onto it,
    else the one the nights gave (the longest dual-band master, or the job's reference)."""
    if crop.get("onPlanFrame"):
        target = job["frameMaster"]
        return {"path": str(target["path"]), "source": str(target.get("source") or plan_combine.OWN),
                "nightId": str(target.get("nightId") or "")}
    natural = job["masters"][combined.natural]
    return {"path": str(natural["path"]), "source": str(natural.get("source") or plan_combine.OWN),
            "nightId": str(natural["nightId"])}


def finish_combination(job: dict[str, object], args: argparse.Namespace, report: stack.Reporter,
                       combined: Combined, out_master: Path) -> tuple[dict[str, object], dict[str, object]]:
    """The combined image cut to the area every master covers, and written as the 32-bit FITS the render reads.

    1. A milestone of the progression is first brought onto the frame of the image of the plan when its own
       frame is another master's (:func:`onto_frame_master`); when that fails it stays on its own frame and
       its own common area is measured;
    2. the crop (``plan_crop.decide``): the largest axis-aligned rectangle every master retained covers, the
       rectangle of the image of the plan for a milestone (``crop`` of the job), nothing when the job keeps
       the whole field (``planField``: ``clean``, the default, ``common`` or ``whole``);
    3. the image, the masks and the WCS of the header (``CRPIX``) cut or moved by it; ``noData`` and
       ``coverage`` are measured on the image as it is written.

    Returns the description of the combination and the crop (the ``crop`` event and the record say it).
    """
    from astropy.io import fits

    description = dict(combined.description)
    moved: dict[str, object] | None = None
    on_plan_frame = True
    target = job.get("frameMaster")
    if isinstance(target, dict) and target.get("path"):
        natural = job["masters"][combined.natural]
        if not (same_file(natural["path"], target["path"])
                and str(natural.get("source") or plan_combine.OWN) == str(target.get("source") or plan_combine.OWN)):
            try:
                moved = onto_frame_master(job, args, combined)
            except dualband_reference.RegistrationError as error:
                on_plan_frame = False
                print(f"the milestone could not be brought onto the frame of the plan: {error}", file=sys.stderr)
    crop = plan_crop.decide(combined.covers, combined.covered, plan_crop.mode_of(job),
                            job.get("crop") if on_plan_frame else None)
    if moved is not None:
        crop["onPlanFrame"] = moved
    elif not on_plan_frame:
        crop["onPlanFrame"] = None
    image = plan_crop.cut(combined.image, crop)
    covers = [plan_crop.cut(cover, crop) for cover in combined.covers]
    covered = plan_crop.cut(combined.covered, crop)
    header = combined.header
    for key in ("ASFAMILY", "STACKCNT", "LIVETIME", "EXPTIME", "FILTER"):
        header.pop(key, None)
    if crop["applied"]:
        lost = crop["lost"]
        header["HISTORY"] = (f"Astro Sync: cropped ({crop['mode']}), {crop['width']} x {crop['height']} "
                             f"(lost {lost['top']}/{lost['left']}/{lost['bottom']}/{lost['right']} px)")
        crop["wcsShifted"] = plan_crop.shift_wcs(header, crop)
    description["noData"] = plan_combine.no_data(covered)
    description["coverage"] = plan_combine.coverage_report(combined.masters, covers, combined.colour)
    out_master.parent.mkdir(parents=True, exist_ok=True)
    fits.PrimaryHDU(image, header).writeto(out_master, overwrite=True)
    report.emit("crop", **crop)
    return description, crop


def execute(job: dict[str, object], args: argparse.Namespace, report: stack.Reporter, runner: stack.Runner) -> dict[str, object]:
    plan = stack.Plan(frames={}, colour=True, pipelines=stack.choose_pipelines(args), linear_only=args.linear_only)
    starnet = stack.find_starnet(args, plan)
    graxpert = stack.find_graxpert(args, plan)
    notes: dict[str, object] = {}
    started = time.monotonic()
    stack.prepare_work(args.work)
    steps = steps_of(job, args)
    total = len(steps)
    master = args.work / "masters" / "plan_linear.fit"
    if len(job["masters"]) > 1 and on_dualband(job):
        combined, registration = combine_on_dualband(job, args, report, total)
        report.emit("step", step="combination", index=2, total=total)
    else:
        aligned = align(job, args, report, runner, 1, total)
        index = 2 if len(job["masters"]) > 1 else 1
        report.emit("step", step="combination", index=index, total=total)
        combined = combine(job, aligned)
        registration = (stack.registration_fields(aligned.registration, master_labels(job), stack.MASTERS)
                        if aligned.registration else None)
    # The image cut to the area every night covers (unless the job keeps the whole field), then written.
    description, crop = finish_combination(job, args, report, combined, master)
    # What each part of the frame holds: the record keeps it, and the corners without dual-band
    # data are said to the person like the rotation is.
    coverage = description.pop("coverage", None)
    if coverage and coverage["noDualBandShare"] > 0:
        report.emit("coverage", noDualBandShare=coverage["noDualBandShare"])
    if runner.cancelled:
        raise stack.Cancelled()
    report.emit("step", step=steps[-1], index=total, total=total)
    (args.work / "render").mkdir(exist_ok=True)
    if args.linear_only:
        outputs = stack.linear_outputs(args, report, runner, master)
    else:
        outputs = stack.render_outputs(args, plan, report, runner, master, starnet, graxpert, notes)
    quality = stack.measure_quality(master)
    # The frame the person kept, cut from the whole image(s) the render wrote (nothing is stacked again).
    frame = frame_outputs(job, args, report, outputs, coverage)
    write_jpegs(job, args, report, outputs)
    preview = write_preview(args, outputs, coverage)
    record = {
        "format": FORMAT_VERSION,
        "projectId": job.get("projectId"),
        "targetId": job.get("targetId"),
        "masters": [{key: m.get(key) for key in ("nightId", "eveningDate", "filter", "band", "frameCount", "totalSeconds")}
                    | {"source": m.get("source") or plan_combine.OWN} for m in job["masters"]],
        "reference": int(job["reference"]),
        # What the alignment said of the frames of all the nights: the field rotation, the framing, the masters left out.
        "registration": registration,
        "combination": description,
        # The share of the frame without dual-band data, the exposure time of each zone and the largest
        # rectangle every night covers (the choice of the cropped image reads it).
        "coverage": coverage,
        # The frame cut from the whole image (the four integers of a notch), ``None`` when the whole image was kept;
        # the picture of the whole image the screen draws the notches on.
        "frame": frame,
        # The crop of the image to the area every night covers (``plan_crop``): the rectangle kept on the frame of
        # the master named, the pixels lost on each edge, why; the milestones of the progression take it again.
        "crop": crop,
        "frameMaster": frame_master_of(job, combined, crop),
        # The display of the image (``black`` per channel, ``scale`` of the stretch) and whether the job imposed it
        # (``imposed``: a milestone at the level of the image of the plan) or it is this image's own measure;
        # ``None`` for "No processing" and for a render that has its own display (the polished one).
        # The milestones of the progression take the one of the image of the plan.
        "display": display_of(args),
        "preview": preview,
        "tools": {"siril": stack.siril_version(args.siril) if args.siril else None, "starnet2": starnet is not None,
                  "graxpert": graxpert is not None},
        # What the "Polished render" did, when it was the render.
        **({"render": notes} if notes else {}),
        "contrast": "none" if args.linear_only else args.contrast,
        "colour": "none" if args.linear_only else args.colour,
        "softStars": "none" if args.linear_only else args.soft_stars,
        "seconds": round(time.monotonic() - started, 1),
        "outputs": outputs,
        # The quality of the combined linear image, before the render.
        "quality": quality,
    }
    (args.out / record_name(args.name)).write_text(json.dumps(record, indent=2) + "\n", encoding="utf-8")
    if not args.keep_work:
        shutil.rmtree(args.work, ignore_errors=True)
    return record


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--job", type=Path, required=True, help="the job Astro Sync wrote (JSON)")
    parser.add_argument("--json", action="store_true", help="one JSON event per line on stdout")
    parser.add_argument("--dry-run", action="store_true", help="print the plan, write nothing")
    parser.add_argument("--keep-work", action="store_true", help="keep the scratch folder for inspection")
    options = parser.parse_args(argv)
    report = stack.Reporter(options.json)
    runner = stack.Runner()
    signal.signal(signal.SIGTERM, runner.cancel)
    signal.signal(signal.SIGINT, runner.cancel)
    try:
        job = read_job(options.job)
        args = render_args(job)
        args.keep_work = options.keep_work
        report.emit("plan", **plan_fields(job, args))
        if options.dry_run:
            return stack.EXIT_OK
        record = execute(job, args, report, runner)
        report.emit("done", seconds=record["seconds"], outputs=record["outputs"])
        return stack.EXIT_OK
    except stack.Cancelled:
        report.emit("error", code="cancelled", message="stopped on request")
        return stack.EXIT_CANCELLED
    except stack.StackingError as error:
        report.emit("error", code=error.code, message=str(error))
        return error.exit_code


if __name__ == "__main__":
    sys.exit(main())
