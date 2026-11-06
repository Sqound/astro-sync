# Astro Sync stacking

The "Stacking from the FITS" treatment of Astro Sync: one target's
FITS lights, with the night's calibration frames, become a stacked and
rendered image in `<cible>/stacked/`, never in `final/`. Everything runs on the
computer; no FITS ever leaves it.

The recipe is the author's own deep-sky processing pipeline. No path, place,
name or setting of a machine is kept here.

| File                                                                                                                                                                                                | What it does                                                                                                                                                                                                                                                                                                                                                                                                                                                                          |
| --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `astro_sync_stack.py`                                                                                                                                                                               | The entry point: Siril calibration, WCS removal on working copies, affine registration, winsorised integration, then one render per `--pipeline`. `--json` events, `--dry-run`, cancel, `list-pipelines`.                                                                                                                                                                                                                                                                             |
| `linear_export.py`                                                                                                                                                                                  | The "no processing" output (`--linear-only`): the linear master as a 16-bit TIFF, scaled to 0..65535, never stretched.                                                                                                                                                                                                                                                                                                                                                                |
| `overscan.py`                                                                                                                                                                                       | The overscan of a scientific CCD: every frame of the night is cut to its `TRIMSEC` on the working copies, only when the whole night agrees on it; and the lowest pixel of the calibrated frames, which the guard on `-output_norm` reads.                                                                                                                                                                                                                                             |
| `raw_files.py`                                                                                                                                                                                      | Camera RAW files: the nine extensions, the Fujifilm X-Trans test, and the converter that asks Siril for a FITS copy of a pose (`pose_stats.py` measures those copies; the stacking itself hands the RAW files to Siril's `convert`).                                                                                                                                                                                                                                                  |
| `test_raw_files.py`                                                                                                                                                                                 | Tests of the RAW files (standard library plus the stacking environment; the end-to-end cases run Siril on a synthetic night of DNG files and are skipped without it, `ASTRO_SYNC_RAW_SAMPLES` adds real camera files).                                                                                                                                                                                                                                                                |
| `integration_master.py`                                                                                                                                                                             | `--integration <family>`: keeps the 32-bit linear master as `<family>_integration.fit` in `stacked/`, its header completed (filter, exposure, gain, binning, temperature, sensor, frames, live time).                                                                                                                                                                                                                                                                                 |
| `device_master.py`                                                                                                                                                                                  | `--from-master <file>`: the image a device already stacked (a Seestar's `Stacked_*.fit`, a linear 16-bit colour cube) scaled to the 0..1 floats of Siril's master, so that the renders start from it and nothing is stacked; refuses a stretched or a mono image.                                                                                                                                                                                                                     |
| `star_quality.py`                                                                                                                                                                                   | The quality of an image read on the stars of its **linear** master: median FWHM in pixels, median eccentricity, stars measured (twenty at least, else no measure); written as `quality` in `<name>_stacking.json` and `<name>_plan.json`, never on a processed image. `python3 star_quality.py master.fit` prints it; `star_quality.py --measure master.xisf` measures a master a person stacked elsewhere, FITS or XISF, refusing an image that is not linear. numpy and scipy only. |
| `xisf_reader.py`                                                                                                                                                                                    | The pixels of a monolithic XISF ("Measure a master…"): the first image, `UInt8` to `Float64`, planar or normal, attached, inline or embedded, uncompressed or zlib / LZ4 (`+sh` byte shuffling included); zstd and the distributed format are named as not read. numpy, `imagecodecs` for LZ4.                                                                                                                                                                                        |
| `plan_stack.py`                                                                                                                                                                                     | The image of a multi-night plan: reads Astro Sync's job, aligns the nights' masters with Siril (`register -2pass -transf=affine`) or, with a dual-band master of a colour sensor, on their stars (`dualband_reference.py`), combines them, renders like a night.                                                                                                                                                                                                                      |
| `night_framing.py`                                                                                                                                                                                  | The frame of the stacking of a night whose field turns: the coverage of each light from the transforms of the registration, five notches from the area every light covers to the whole field (`frame` of the `registration` event, `--frame-size`), the crop of the linear master before the renders; the whole field stays as `<name>_field.fit` and is cut again without stacking.                                                                                                  |
| `plan_framing.py`                                                                                                                                                                                   | The adjustable frame of the image of a plan: the five notches of the slider from the area every integration covers to the whole image (`coverage.framings` of the record, the mark at 5 % of the image at reduced exposure), the polygon of the area every night covers, the picture for the screen, and the crop of the rendered image to the frame kept (`<file>_framed.tif`, pixels copied).                                                                                       |
| `plan_crop.py`                                                                                                                                                                                      | The image of a plan cut to a clean axis-aligned frame (`planField`): by default the largest rectangle without a pixel no night covers (no black bevel), or the largest every night covers (`common`), or nothing (`whole`); cut on the linear image before the render so every render has it, the WCS moved.                                                                                                                                                                          |
| `level_match.py`                                                                                                                                                                                    | The scale between two aligned masters read on the **light** of the stars (aperture photometry), not on their peaks: used for every multi-night plan, whatever the source of the masters.                                                                                                                                                                                                                                                                                              |
| `plan_combine.py`                                                                                                                                                                                   | The arithmetic of the plan without a dual-band master of a colour sensor: each master brought to the level of the longest of its band, the masters averaged by integration time.                                                                                                                                                                                                                                                                                                      |
| `dualband_reference.py`                                                                                                                                                                             | A plan of a colour sensor with a dual-band master, the masters of each band registered from their stars (astroalign) and averaged, the broadband one added to the dual-band one at 0.18.                                                                                                                                                                                                                                                                                              |
| `renders.py`                                                                                                                                                                                        | The render functions the pipelines name (`RENDERS`), from a linear RGB master.                                                                                                                                                                                                                                                                                                                                                                                                        |
| `render_finish.py`                                                                                                                                                                                  | The finish of a rendered image beside the contrast: softened stars (`soften_stars`) and colour intensity (`saturate`), applied in that order after the curve to the twin image; `Finish` names the twin file.                                                                                                                                                                                                                                                                         |
| `pipeline_defs.py`                                                                                                                                                                                  | Reads and validates the `pipeline.json` definitions (shipped and added) against the dependency registry, and the registry itself; an invalid definition is ignored and reported.                                                                                                                                                                                                                                                                                                      |
| `pipelines/<id>/`                                                                                                                                                                                   | The shipped definitions: `raw`, `dualband-correction`, `polished`, `hoo`. Format and rules: `pipelines/SCHEMA.md`.                                                                                                                                                                                                                                                                                                                                                                    |
| `dependencies/registry.json`                                                                                                                                                                        | The dependency registry: every tool a pipeline may depend on (Siril, StarNet2, the Python environment; ASTAP and astrometry.net for the format), with its detection rule, its downloads (URL + SHA-256) and its page. Changed only by a pull request.                                                                                                                                                                                                                                 |
| `check_environment.py`                                                                                                                                                                              | Checks Python, each pinned package (import, version, one call), Siril and StarNet2, lists the pipelines, and evaluates the registry's `check` of every dependency they declare (present / missing / outdated). Writes nothing.                                                                                                                                                                                                                                                        |
| `test_integration_master.py`, `test_plan_image.py`, `test_plan_framing.py`, `test_plan_crop.py`, `test_star_quality.py`, `test_master_quality.py`, `test_device_master.py`, `test_night_framing.py` | Tests of the master kept and of the image of a plan (fusion on synthetic masters; end to end on Siril with synthetic frames of two nights, skipped without `siril-cli`). Run with the stacking environment.                                                                                                                                                                                                                                                                           |
| `test_pipeline_defs.py`                                                                                                                                                                             | Tests of the loader and of `list-pipelines` (standard library: `python3 -m unittest test_pipeline_defs`).                                                                                                                                                                                                                                                                                                                                                                             |
| `test_render_finish.py`                                                                                                                                                                             | Tests of the finish of a render (`render_finish.py`): softened stars (the mask is the stars, not the nebula; nothing darker than the sky around them), colour intensity (the luminance kept, no clipping), the order contrast → stars → colour, the names of the twin images, the options of the command line and of the plan's job. Run with the stacking environment.                                                                                                               |
| `test_render_pipelines.py`                                                                                                                                                                          | Tests of the "Polished render" and "HOO palette" renders: the polished steps on a synthetic gradient, with a stand-in GraXpert, a stand-in Siril and without either (the engines and their fallbacks, the sky level kept, the SCNR skipped on a dual-band capture, the deconvolution floor, no dark ring around clipped stars), the HOO palette (neutral sky, smooth mask) on a synthetic dual-band nebula. Run with the stacking environment.                                        |
| `requirements.in` / `.txt`                                                                                                                                                                          | The pins, and the same pins with every SHA-256 digest (`uv pip compile`, command in the header). Python `>=3.12,<3.14`.                                                                                                                                                                                                                                                                                                                                                               |
| `DEPENDENCIES.md`                                                                                                                                                                                   | Every dependency: version, size, licence, installation, use; the render pipelines.                                                                                                                                                                                                                                                                                                                                                                                                    |

## The render pipelines

A **pipeline** is one way to render the stacked image, described by a JSON file
(`pipelines/<id>/pipeline.json`): its name and description in ten languages, the
capture filter it is meant for, its author and licence, the render function it
calls and the tools it needs, and (optional) the steps it runs, shown under
"Details" in the window. Four ship with the app:

- **`raw`** ("Natural render", the default; filter `none`): the camera's native
  colours, never remapped.
- **`dualband-correction`** ("Dual-band correction"; filter
  `dual-band`): the same luminance, then one global colour transform (green
  towards cyan, red towards red-violet) that brings the colours of a capture
  made with a dual-band filter (H-alpha + OIII) back towards natural tones.
  Without such a filter it changes the image: take the natural render (`raw`).
- **`polished`** ("Polished render"; filter `none`): the natural render
  pushed further without changing its colours, from its own linear preparation
  (`renders.prepare_polished`): the background and a light denoising on the
  linear image by the engine the task chose (`--polished-engine`: `graxpert-ai`,
  the default, GraXpert's AI models 1.0.1 and 3.0.2 at strength 0.3, or `siril`,
  `subsky -rbf` and `denoise`, no model), the natural neutral sky and a moderate
  SCNR (half of the green excess; skipped on a dual-band capture, whose green is
  OIII), the masked deconvolution aimed at 0.85 × the measured FWHM (skipped on
  an undersampled field, under 2.0 px; the clipped cores of the brightest stars and
  10 px of sky around them are held out, and a step of 5 iterations that gains less
  than 0.03 px is not kept: otherwise Richardson-Lucy digs a dark ring around them), the natural stretch with a soft shoulder,
  a moderate local contrast on the nebula layer only, the stars held at × 0.48.
  The sky stays as deep as the natural render's (the median luminance
  within 2 %). Without GraXpert (`--graxpert`, else `graxpert` on `PATH`), or when
  it fails, Siril does the background and the denoising; without Siril too, the
  polynomial and no denoising. `<name>_stacking.json` says which engine worked
  (`recipe.render.polished.engine`, `engineRequested` and `fallbackReason` when it
  is not the one asked for).
- **`hoo`** ("HOO palette"; filter `dual-band`): Hα (the red pixels)
  to red, OIII (green and blue) to blue-green, an assumed palette, on the
  natural preparation and brightness; the sky is the natural render's own
  neutral sky, the palette fades in from it through a smooth mask (no threshold),
  and the stars keep their natural colours.

People can add definitions in `<Astro Sync data>/pipelines/<id>/pipeline.json`
(the app passes that folder with `--pipelines-dir`); a definition can reuse a
shipped render function or bring its own script (trusted by the person, see
below). The format, the validation rules and the **language fallback** (a text
missing in the person's language is shown in English, `en` being required) are
in `pipelines/SCHEMA.md`.

`--pipeline <id>` is repeatable (default `raw`): each pipeline writes
`<name>_<id>.tif` (16-bit RGB, the image sent) and `<name>_<id>.png` (8-bit).
An unknown id stops the script (exit code 2, `unknown_pipeline`, the known ids
in the message).

`--polished-engine graxpert-ai|siril` (default `graxpert-ai`) picks the
engine of the "Polished render"'s background and denoising (see `polished` below);
`--capture-filter <name>` names the filter of the lights when their `FILTER`
keyword does not (a dual-band one skips the SCNR of the polished render).

`--contrast none|presentation|strong` (default `none` for the script; the
`astro-sync stack` and `plan-image` commands and the window pass `presentation`
unless told otherwise) adds, beside
each image, the same image with the contrast of presentation: an S-curve on the
luminance around the sky level (`renders.presentation_contrast`, gain 2.0 for `presentation`, 3.6 for `strong`), the
RGB ratios kept. The files are `<name>_<id>_presentation.tif` / `.png` or
`_strong`, their `output` events and their entries of `<name>_stacking.json`
carry `"contrast"`, and the image without the curve stays the main one. It
applies to a script's render as well, but **not** to "No processing": the
curve pivots on the sky level of a stretched image and does nothing visible on
a linear one, so no `<name>_linear_<contrast>.tif` is written;
the linear master kept by `--integration` is never touched either.
`plan_stack.py` reads the same word from `contrast` in its job.

The three settings are continuous. `--contrast` also takes a gain from 0 to 6 (`2.5`) beside its two
words (`presentation` is 2.0, `strong` 3.6; a number equal to a word's is written as the word, so the
files and records of those two keep their names; any other gain is named `_contrast-2.5`).
`--colour none|PERCENT` and `--soft-stars none|PERCENT` (0 to 100, default `none`; the words of the
first version, `enhanced`, `strong` and `light`, read as 20, 40, 50 and 85) add the **finish** to that same
extra image (`render_finish.py`), after the contrast curve: the brightest stars pulled towards the
image without them (`--soft-stars`: a top-hat of the luminance finds what is narrower than a 7 px
disk, the mask is grown and blurred, the image eroded under it; the percent is the share of each
star taken out), then the colours strengthened (`--colour`: the chroma moved away from the luminance
by 1 + percent / 100, the luminance and the hue kept, never clipped). The sizes in pixels follow the
image (they are those of a 3000 px side). Asking for any of the three writes the twin:
`<name>_<id>_strong_colour-40_stars-85.tif`, and the `output` events and `<name>_stacking.json` carry
`"colour"` and `"softStars"` (the percent as text) as they carry `"contrast"`. The master and "No
processing" are never touched, and with the three at `none` nothing changes. `plan_stack.py` reads
`contrast`, `colour` and `softStars` from its job, as words or numbers.

Those three options are the **default** finish of the run. `--finish ID=CONTRAST,COLOUR,STARS`
(repeatable, one per pipeline, each value written as for the three options: `hoo=strong,20,50`)
gives one pipeline a finish of its own: its twin is named after that finish
(`<name>_hoo_strong_colour-20_stars-50.tif`), and `none,none,none` writes that pipeline's render alone
whatever the default says. The plan event and `<name>_stacking.json` carry `"finishes"`, the finish of
each rendered pipeline by id (`{"hoo": {"contrast": "strong", "colour": "20", "softStars": "50"}}`).
"No processing" takes none (`--finish linear=...` is refused).

**No processing** (`--linear-only`, the option "No processing") is not a
pipeline: it runs no render. `linear_export.py` writes Siril's linear master,
calibrated, aligned and integrated but **not stretched**, as
`<name>_linear.tif` (16-bit; RGB, or grayscale for a mono master) and nothing
else; StarNet2 and the render libraries are not used. It excludes
`--pipeline`, and the id `linear` is reserved (no definition may take it). `list-pipelines --json` prints the loaded definitions for the
"Processing" screen. Details in `DEPENDENCIES.md`.

**The overscan and the sky of the master**. A scientific CCD writes columns
of overscan beside the image and says where the image is: `TRIMSEC = '[1:4096, :]'`
(1-based, inclusive, x first). Divided by a master flat that is almost zero there, the
overscan sends the calibrated frames down to −1, and `-output_norm` stretches
[−1, 1] to [0, 1]: the sky of the master lands at 0.5. Two things prevent it.
`overscan.trim_overscan` cuts every frame of the night (lights and calibration frames)
to the `TRIMSEC` of the first light, on the working copies only (the originals are
never opened for writing), when **every** frame carries the same `TRIMSEC` and the same
size; otherwise nothing is cut and a `note` event says why (`overscan_disagree`,
`overscan_unreadable`, `overscan_outside`, `overscan_bayer` for a cut that would shift
the Bayer pattern). Lights without a `TRIMSEC` are left exactly as they were. Then,
after the calibration, a calibrated frame whose lowest pixel is under −0.05 (the
`normalisation_skipped` note) makes the integration run without `-output_norm`: the
stack keeps the units of the frames. `<name>_stacking.json` records both
(`overscan`, `normalisation`).

**The image of a device** (`--from-master <file>`) is the
stacking **not made**: the Seestar writes `Stacked_<n>_<target>_<exposure>_<filter>_<date>.fit`
beside its sub-frames (a linear colour cube, 16 bits, with a field solution), and
Astro Sync can take it as the master of a target. `device_master.py` scales it to
0..1 and writes `masters/stack_linear.fit` in the scratch folder, then the steps
after the stacking run as they do for a stacking of poses (the renders, the
contrast, `--linear-only`). It replaces `--lights`, `--frames-from` and the
calibration folders (none may be given with it), keeps no `--integration` master
(the image of a multi-night plan is made from poses), needs no Siril, and measures
no quality by itself (`star_quality.py --measure` does it on request); the
provenance carries `deviceMaster` (file, device, poses, SHA-256). A stretched image
stops the run (`device_stretched`), a file that is not a readable colour cube too
(`device_unreadable`). The rows are not flipped: a Seestar's image and the master
Astro Sync stacks from the same night's sub-frames have the same handedness
(measured).

**A device's image in a multi-night plan** (job `source: "device"`): a night of which only the device's `Stacked_*.fit` is here enters the plan
as that image. `plan_stack.py` converts it first (`device_master.convert`: 32-bit
floats, divided by 65 535, as Astro Sync's masters are; Siril
does not register a 16-bit integer image with a 32-bit master), then aligns it with the
other masters. Three rules follow:
the image is **never flipped**; its field rotation is **unknown** (no `ASROTMIN` /
`ASROTMAX`), so the plan keeps the whole field of the reference night and says no
rotation (`fieldRotationUnknown` in the `registration` event and the record); and the
scale between it and another master is read on the light of the stars in an aperture
(`level_match.py`, recorded in `combination.levelMatch`; the aperture is 4 FWHM wide, 10 to 20 px, to hold the
wings of the stars), because the peaks of the sharper image would read the scale 16 to 32 % wrong. This is the one rule
of every plan, including plans made only of Astro Sync's own masters (the peaks were
already -12 % to +8 % off between two of them): there is no switch back to the peaks; only
a field with fewer than 20 readable stars falls back to the former fit, and the record says
so (`apertureUnavailable`).

**The adjustable frame** (job `frame`): the image of the plan is
always made whole and kept in `stacked/`. The record's `coverage.framings` lists the
notches the person may keep (cropped, the mark, two between, whole; four integers each,
the share of the frame below the full exposure, the lowest exposure and its dual-band
part), `coverage.fullPolygon` the area every night covers, and `<record>.preview.png` is a
small picture of the whole image for the screen. A job that names one notch as `frame`
gets every rendered TIFF or PNG cut to it after the render (`<file>_framed.<ext>`);
nothing is stacked again. A frame that does not fit the image stops the run
(`bad_job`).

## Declaring a pipeline and its dependencies

What a person or the community writes: **a folder** `<id>/` holding a
`pipeline.json`, and, if the render is their own, a Python script next to it.
The folder goes in `<Astro Sync data>/pipelines/` (or, for a pipeline shipped
with the app, in `stacking/pipelines/` by a pull request).

```json
{
  "id": "mono-ha",
  "version": "1",
  "name": { "en": "Hα in warm tones", "fr": "Hα en tons chauds" },
  "description": { "en": "…", "fr": "…" },
  "filter": "h-alpha",
  "author": "Your name",
  "licence": "MIT",
  "entry": "script:render.py",
  "requires": ["siril", "starnet2?"],
  "outputs": ["tif", "png"],
  "dependencies": [
    {
      "id": "siril",
      "required": true,
      "reason": {
        "en": "Siril stacks your exposures.",
        "fr": "Siril additionne vos poses."
      }
    },
    {
      "id": "starnet2",
      "required": false,
      "reason": {
        "en": "StarNet2 separates the stars.",
        "fr": "StarNet2 sépare les étoiles."
      }
    },
    {
      "id": "python-env",
      "required": true,
      "reason": {
        "en": "The Python environment runs render.py.",
        "fr": "L’environnement Python exécute render.py."
      }
    }
  ]
}
```

1. `id` is the folder's name. `name`, `description` and every `reason` are one
   text per language: `en` is required, a missing language falls back to
   English; write "vous" in French and the polite register elsewhere.
2. `entry` is `renders:<function>` (a render shipped with Astro Sync) or
   `script:<path>`: a `render(nebula, stars)` function in a Python file of the
   folder (contract in `pipelines/SCHEMA.md`).
3. `requires` lists the tools the stacking runs (`siril` always; `starnet2?` =
   optional). `dependencies` says, for each one, **why this pipeline needs it**
   and whether it is `required`; it only **names** the dependency, by its
   identifier in `dependencies/registry.json`. An optional `minVersion` can ask
   for more than the registry does, never less.
4. `outputs`: `tif` always, `png` if you want it.

**The three safeguards.**

1. **Everything that can be installed is in the registry** (`dependencies/registry.json`),
   with a detection rule and, for each system with an automatic download, a
   URL **and its SHA-256** (the file is refused otherwise). A pipeline cannot
   carry a URL: an identifier that is not in the registry makes the manifest
   invalid (the message also carries its French equivalent). Adding a tool is a pull request on the registry.
2. **Detection**: each entry has a `check` (a command asked only for its
   version, a pattern that reads it, a minimum). It gives the states present /
   missing / version too old for the "Dependencies" screen and
   `check_environment.py --json`.
3. **A script is trusted by the person.** A script that ships with the app runs
   without a question; one the person added themselves is refused until they
   have trusted it, and again if its SHA-256 changes (`--trusted <id>=<sha256>`,
   exit code 2, `script_not_trusted`).

**What is not possible in v1.** A manifest cannot bring a download, a URL, a
detection command or a package of its own; it cannot install anything the
registry does not list; its script cannot run without the person's trust; the
Python packages are the pinned set of `requirements.txt` (a script cannot ask
for another one); there is no library of community pipelines yet (a person
copies the folder).

`astro_sync_stack.py list-pipelines --json` returns each pipeline with its
dependencies **resolved** (the registry's entry plus `required`, `reason` and the
effective minimum), `scriptSha256`, and the whole `registry`.

## In Astro Sync

What the app does with these files:

- **The environment** is made by "Dependencies" ("Install" of the row
  "Stacking pipeline", or "Install all") in Astro Sync's data
  (`stacking-env/`) from a Python in the registry's range, then
  `pip install --require-hashes --only-binary=:all: -r requirements.txt` and
  `check_environment.py --json`; a mark written after the check names the
  digest of `requirements.txt` (other pins: "to install again").
  Every script runs as `python -B -E -s -X utf8` (no bytecode beside the bundled
  scripts).
- **The section "Processing pipelines"** lists each pipeline found, its
  declared dependencies resolved by the registry, and the person's trust in
  a script they added; the answer is kept per digest in
  `<data>/pipeline-trust.json` and passed as `--trusted <id>=<sha256>`.
- **The runner** calls the script once per family of exposures
  (`--frames-from` when the target has several), passes `--siril`,
  `--starnet` and `--graxpert` where "Dependencies" found them, stops it
  with `SIGTERM`.
- **Still open**: Siril and StarNet2 have no `downloads` (page only; ASTAP
  and its D20 star database have theirs); ASTAP's
  and astrometry.net's `check` are not verified (`tools.rs` keeps its own
  rule for them); Siril's `siril_versions.json` (progression) stays a rule
  apart from the stacking's minimum `1.4`.

## Running it by hand

```sh
python3 -m venv <env>   # Python 3.12 or 3.13
<env>/bin/python -m pip install --require-hashes -r requirements.txt
<env>/bin/python check_environment.py
<env>/bin/python astro_sync_stack.py \
  --lights "<nuit>/M31/lights" --darks "<nuit>/calibration/darks" \
  --flats "<nuit>/calibration/flats" --dark-flats "<nuit>/calibration/dark-flats" \
  --work "<scratch>" --out "<nuit>/M31/stacked" --name M31_2026-10-07 \
  --pipeline raw --pipeline dualband-correction --json
# or, with no render at all, only the linear stack as <name>_linear.tif:
#   ... --linear-only --json
```

```sh
<env>/bin/python astro_sync_stack.py list-pipelines --json [--pipelines-dir DIR] [--lang fr]
```

Siril 1.4 (`siril-cli`) is required; StarNet2 is optional (without it the
stars are not separated and the render differs), and so is GraXpert 3.0 or
later, used by `polished` only. None is bundled: they are external tools,
detected at run time.

## Licence

MIT, like the rest of Astro Sync.
