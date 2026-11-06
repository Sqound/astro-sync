# Dependencies of the Astro Sync stacking

Survey of 2026-10-08, checked on a Mac (Apple Silicon,
macOS 26) in a throwaway virtual environment, outside the repository. Nothing
listed below is bundled in Astro Sync: the Python environment is created and filled
by Settings › "Dependencies"; Siril and StarNet2 are detected and
their download page is offered.

> Each pipeline **declares** what it needs (Siril, StarNet2, the Python
> environment) by naming entries of the registry `dependencies/registry.json`, which
> carries the detection rule, the downloads (URL and SHA-256) and the page of
> each tool. The format, the safeguards and what a pipeline cannot do
> are in `README.md` § "Declaring a pipeline and its dependencies" and in
> `pipelines/SCHEMA.md`. This document remains the survey of versions and sizes.

## Python

| Item     | Value                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                              |
| -------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Version  | **3.12 or 3.13** (`>=3.12,<3.14`), since 2026-10-27: sep 1.4.1, the star finder of astroalign (the registration of a plan's masters on the dual-band one, `dualband_reference.py`), has no wheel for 3.14 (its last release, February 2025, stops at 3.13); installation with `pip --require-hashes --only-binary=:all:` then `check_environment.py` succeeded, 16 packages "ok", on 3.12.13 and 3.13.14, and fails on 3.14.7 (no sep). The range established on 2026-10-08 was 3.12 to 3.14. **3.11 and earlier: no** (numpy 2.5.1, scipy 1.18.0, tifffile 2026.7.31 and imagecodecs 2026.6.26 require 3.12). An environment made with a Python out of the range is « outdated » and made again by « Install » (`python_env.rs`). |
| Size     | ≈ 100 MB (standalone interpreter from `uv python install 3.13`)                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                    |
| Licence  | PSF-2.0                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                            |
| Pitfalls | The `python3` that Homebrew installs today (3.14) **is not suitable**: `brew install python@3.13` or the python.org installer of 3.13. On macOS, a `python3` in `/usr/bin` is 3.9: ignore it, like any Python outside the range. Astro Sync looks for a Python from 3.12 to 3.13 (the most recent found), otherwise offers the python.org installer.                                                                                                                                                                                                                                                                                                                                                                               |

## Python packages (`requirements.txt`, pinned by version and SHA-256)

Installation: `python3 -m venv <env>` (a Python in the range above) then
`<env>/bin/python -m pip install --require-hashes -r requirements.txt` (checked
with pip 26.1 and with `uv pip install --require-hashes`). Download: 64 MB
of wheels for macOS arm64; installed environment: 213 MB (uv) to 303 MB (pip,
which precompiles). All the wheels bundle their native libraries:
**no system library** to install.

| Package           | Version           | Installed size | Licence                            | Usage                                                              | Smoke check                      |
| ----------------- | ----------------- | -------------: | ---------------------------------- | ------------------------------------------------------------------ | -------------------------------- |
| numpy             | 2.5.1             |        20.3 MB | BSD-3-Clause (and 0BSD, MIT, Zlib) | computation on the images                                          | median of an array               |
| astropy           | 8.0.1             |        24.0 MB | BSD-3-Clause                       | reading and writing FITS, removing the WCS from the working copies | FITS header, `sigma_clip`        |
| scipy             | 1.18.0            |        70.2 MB | BSD-3-Clause                       | Gaussian filters, convolution (render, optional deconvolution)     | `gaussian_filter`, `fftconvolve` |
| scikit-image      | 0.26.0            |        25.9 MB | BSD-3-Clause                       | RGB ↔ HSV conversions of the two renders                           | round trip RGB → HSV → RGB       |
| tifffile          | 2026.7.31         |         1.2 MB | BSD-3-Clause                       | writing the 16-bit output TIFF, reading StarNet2's TIFF            | deflate write and read back      |
| pillow            | 12.3.0            |        13.3 MB | MIT-CMU                            | writing the 8-bit PNG                                              | 4 × 4 RGB image                  |
| imagecodecs       | 2026.6.26         |        36.6 MB | BSD-3-Clause                       | LZW decoding of the TIFF written by StarNet2                       | LZW round trip                   |
| astropy-iers-data | 0.2026.8.3.0.53.6 |         8.9 MB | BSD-3-Clause                       | dependency of astropy                                              | import                           |
| pyerfa            | 2.0.1.5           |         1.6 MB | BSD-3-Clause                       | dependency of astropy                                              | import                           |
| pyyaml            | 6.0.3             |         0.6 MB | MIT                                | dependency of astropy                                              | import                           |
| packaging         | 26.3              |         0.5 MB | Apache-2.0 or BSD-2-Clause         | dependency of astropy and scikit-image                             | import                           |
| networkx          | 3.6.1             |         7.0 MB | BSD-3-Clause                       | dependency of scikit-image                                         | import                           |
| imageio           | 2.37.4            |         1.2 MB | BSD-2-Clause                       | dependency of scikit-image                                         | import                           |
| lazy-loader       | 0.5               |         0.0 MB | BSD-3-Clause                       | dependency of scikit-image                                         | import                           |
| astroalign        | 2.6.2             |         0.0 MB | MIT                                | registering the masters of a plan on the dual-band one             | a shifted star field found again |
| sep               | 1.4.1             |         0.7 MB | LGPL-3.0-or-later (C library), MIT | the star finder of astroalign                                      | through astroalign               |

`check_environment.py` redoes these checks (import, version compared with
`requirements.in`, one real call per direct package) and looks for Siril and
StarNet2; it writes nothing. Result on the Mac: everything "ok", Siril 1.4.4,
StarNet2 2.5.4.

**Left out of the environment** (useless for the stacking): ccdproc, fitsio,
matplotlib, opencv-python-headless, openpyxl, photutils, rawpy, reproject,
tqdm.

## External tools

| Tool         | Version checked | Size                                                                         | Licence                                                                                                                              | Installation                                                                                              | Usage                                                                                                          | Required      |
| ------------ | --------------- | ---------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------ | --------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------- | ------------- |
| Siril        | 1.4.4           | 16 MB (Homebrew formula, plus its dependencies)                              | GPL-3.0-or-later                                                                                                                     | macOS: application from siril.org or `brew install siril`; Windows: installer; Linux: Flatpak or AppImage | `siril-cli`: masters, calibration, demosaicing, alignment, integration; already detected for the "Progression" | **yes**       |
| StarNet2 CLI | 2.5.4           | ≈ 140 MB (libraries and CoreML weights)                                      | free; terms not published on its download page ("All rights reserved"): **never bundled or redistributed**, downloaded by the person | macOS package from starnetastro.com; also exists for Windows and Linux                                    | separates the stars from the nebula before the render                                                          | no (optional) |
| GraXpert     | 3.0.2           | 130 to 393 MB depending on the system, plus 664 MB of AI models on first use | GPL-3.0-only (program); AI models CC BY-NC-SA 4.0, downloaded by GraXpert on first use: **never bundled or redistributed**           | macOS: `.dmg`; Windows: `.msi`; Linux: ZIP unpacked by Astro Sync (§ GraXpert below)                      | sky background and light denoising of the "Polished render"                                                    | no (optional) |

**Registry downloads (2026-10-09).** The `siril` and `starnet2` entries of
`dependencies/registry.json` only have their **download page**
(`https://siril.org/download/`, `https://starnetastro.com/cli-tools/starnet/`,
checked, response 200) and their detection rule (checked on the Mac:
`siril 1.4.4`, `starnet2  version: 2.5.4`). No `downloads` (URL + SHA-256)
is filled in: the fingerprint of a file can only be computed after downloading
it, and no installer was downloaded. Filling it in
is a registry PR, with the fingerprint of the real file, system by system.
`astrometry-net` appears there only to document the format: page only, **no
detection rule** and no verified licence.

### ASTAP and its star index

ASTAP (the sky solver for the captures) and a star index now
have their download, pinned in `dependencies/registry.json` (entries `astap`
and `astap-database`, two rows of the screen merged into "ASTAP and its
indexes"). Official sources only: the SourceForge of the ASTAP project
(`downloads.sourceforge.net/project/astap-program/`, the mirror of the page
`https://www.hnsky.org/astap.htm`), over https, each file pinned by its
SHA-256 **computed on the real file** (2026-10-10) and by its size.

| File                                          | Systems     | Version                                        | Size (zip)                                            | SHA-256 (start) |
| --------------------------------------------- | ----------- | ---------------------------------------------- | ----------------------------------------------------- | --------------- |
| `astap_command-line_version_macOS_M1.zip`     | macos-arm64 | CLI-2026.09.01 (read by `astap_cli -h`)        | 1,308,120 bytes                                       | `6a59b61c…`     |
| `astap_command-line_version_macOS_x86_64.zip` | macos-x64   | CLI-2026.09.01 (read by `astap_cli -h`)        | 1,424,473 bytes                                       | `3cc08ef3…`     |
| `astap_command-line_version_win64.zip`        | windows-x64 | 2026.09.19 (per the page; binary not run here) | 322,357 bytes                                         | `e9b94a44…`     |
| `astap_command-line_version_Linux_amd64.zip`  | linux-x64   | 2026.09.19 (per the page; binary not run here) | 329,328 bytes                                         | `5e7dca28…`     |
| `star_databases/d20_star_database.zip` (D20)  | all four    | files from 2023, without a number              | 399,561,156 bytes (422,762,340 unpacked, 1,476 files) | `c1cb092c…`     |

- **Chosen form: the command-line solver, as a ZIP.** A single binary
  (`astap_cli`, `astap_cli.exe`), which the application unpacks into
  `tools/astap/` of the settings folder: **no administrator rights**,
  nothing outside the Astro Sync folder. The macOS `.pkg` files (installing into
  `/Applications` and `/usr/local/opt/astap`, administrator rights) and the
  Windows/Linux installers are excluded. The "interface" version is not
  needed: Astro Sync only calls `-f … -o … -d …`.
- **Default index: D20.** ASTAP gives D20 for a field of 0.3° to 6°
  (the project's `readme astap.txt`); D05 only covers 0.6° and more, D50 (901 MB)
  and D80 bring nothing to refractors of 0.5 to 3° and weigh twice as much
  or more. A field
  smaller than 0.3° would need D50 or D80: another file, another line of the
  registry.
- **Licences.** The program: **MPL-2.0** (declared by the project on
  SourceForge, recalled by the binary itself: "License MPL 2.0, Webpage:
  www.hnsky.org"). The MPL-2.0 allows redistribution; Astro Sync
  redistributes nothing anyway: it downloads from the official source,
  as the person would. The index: an extract of the **Gaia DR3** catalogue
  (ESA / DPAC) in the HNSKY/ASTAP format, put online freely by the project
  with the sole request to acknowledge the use of Gaia ("We acknowledge the
  usage of the Gaia database", `readme astap.txt`); no clause forbids
  downloading by a third party. The Gaia credit is in the index's sheet.
- **macOS, quarantine and Gatekeeper.** The file is downloaded by the
  application itself (not by a browser or by the Finder): it does **not** carry
  the `com.apple.quarantine` attribute, and Gatekeeper only checks the
  files that carry it. Nothing is shown to the person, then. As a
  precaution, the application removes the attribute if it is there, and applies an
  ad hoc signature (`codesign --force -s -`, which ASTAP's page asks to do by
  hand) to a program that `codesign --verify` would refuse. The binary as shipped
  is already ad hoc signed by the linker (checked): it runs as is
  on Apple silicon.
- **Detection.** `tools/astap/astap_cli` is the first place looked at, then
  the usual locations of each system, then the `PATH`; the database is
  looked for in `tools/astap-database/`, then next to ASTAP. The person's
  existing installation (`/Applications/ASTAP.app`, `/usr/local/opt/astap`,
  on Windows `C:\Program Files\astap` then `%LOCALAPPDATA%\Programs\astap`,
  the folder of an installation without administrator rights) is still recognised and never touched.
- An interrupted download resumes where it stopped (the mirror cuts the
  connection from time to time: observed for real); a file whose fingerprint
  differs is deleted and nothing is installed.

### GraXpert (2026-10-15)

GraXpert serves the `polished` pipeline alone ("Polished render"): extraction of the sky
background and light denoising, on the still linear image. **Optional**: without
it (or if it fails), **Siril** does the background (`subsky -rbf`) and the denoising
(`denoise`) with its own commands, with no AI model or new dependency;
without Siril either, the render is done with the natural render's polynomial and without
denoising. The task chooses the engine (`--polished-engine graxpert-ai`, the
default, or `siril`) and `<name>_stacking.json` says which one worked
(`recipe.render.polished.engine`, with `engineRequested` and `fallbackReason` if
it is not the one requested; `tools.graxpert`). `graxpert` entry of the registry; official source:
the project repository's releases (`github.com/Steffenhir/GraXpert/releases`,
linked from `https://graxpert.com/`), version **3.0.2** (the latest published;
3.0 at least: command-line denoising dates from 3.0).

| File (`releases/download/3.0.2/`) | System      | Size              | SHA-256 (start) | Installation                                                                                             |
| --------------------------------- | ----------- | ----------------- | --------------- | -------------------------------------------------------------------------------------------------------- |
| `graxpert-macos-arm64.dmg`        | macos-arm64 | 130,103,079 bytes | `35804ba2…`     | disk image left to the person (drag `GraXpert.app` into Applications; 309 MB installed, signed)          |
| `graxpert-macos-amd64.dmg`        | macos-x64   | 168,538,872 bytes | `b4d973d2…`     | same                                                                                                     |
| `graxpert-windows-amd64.msi`      | windows-x64 | 160,124,928 bytes | `767fac98…`     | installer left to the person (binary not run here)                                                       |
| `graxpert-linux-amd64.zip`        | linux-x64   | 392,722,792 bytes | `0a7364c3…`     | unpacked by Astro Sync into `tools/graxpert/GraXpert-linux/` (1.13 GB, 5,757 files; binary not run here) |

- Fingerprints **computed on the real files**, downloaded on 2026-10-15.
  Every platform has a file; only Linux's is a ZIP, hence
  the only one Astro Sync installs itself. The `.dmg` and the `.msi` are
  downloaded and checked, then left to the person (like any file that
  is not a ZIP, `tools.rs`).
- **Detection**: `graxpert` or `GraXpert` with `--version`; GraXpert writes its
  version in a log line (`… INFO GraXpert version: 3.0.2 release:
Umbriel`), which the registry's pattern reads. Places looked at: `tools/graxpert/`,
  `/Applications/GraXpert.app/Contents/MacOS/GraXpert` (and
  `~/Applications/…`), the unpacked Linux binary, `C:\Program
Files\GraXpert\GraXpert.exe`, then the `PATH` (a `graxpert` installed by
  `pip`). Beware: a `pip` installation without `onnxruntime` is detected
  but fails on the first model; Siril then takes over (automatic fallback)
  and the render says so (`graxpertFailed`, `engineRequested`, `fallbackReason`).
- **AI models**: the background extraction (GraXpert's RBF interpolation or
  splines model needs sampling points placed by hand or
  a preferences file; the AI model is the only one the command
  line can do alone) and the denoising (AI only) **download
  their model on first use**, by GraXpert itself, into its own data folder
  (macOS: `~/Library/Application Support/GraXpert/`): background
  **1.0.1, 208 MB** (217,663,463 bytes), denoising **3.0.2, 456 MB**
  (477,703,971 bytes), that is **664 MB** once. The versions are pinned
  (`renders.GRAXPERT_BACKGROUND_MODEL`, `GRAXPERT_DENOISE_MODEL`): same model,
  same image, and no network call once the model is there ("latest" queries
  the server every time). GraXpert also records the version used in
  its preferences (`preferences.json` of the same folder).
- **Licences**: the program is under **GPL-3.0** (the repository's `License.md`
  file, without an "or later" clause: `GPL-3.0-only`). Its **AI models**
  are under **CC BY-NC-SA 4.0** (`LICENSE.html` downloaded with each model,
  "GraXpert Development Team"): **no commercial use**. Astro Sync
  bundles and redistributes nothing: GraXpert is installed by the person,
  who downloads its models themselves. **Decision**: Astro Sync
  is free and open source (MIT), the non-commercial clause is not a
  problem; the default engine of the "Polished render" remains GraXpert. The
  licence of the models ("AI models CC BY-NC-SA 4.0, downloaded by GraXpert
  on first use") is in the `licence` field of the registry, hence in the
  tool's badge in the window, like the other licences.
- **Timings** (2973 × 2964 master, Apple silicon Mac): background extraction
  ≈ 10 s, denoising ≈ 6 min 30 (CoreML and CPU; `-gpu false` is not
  faster). The "Polished render" therefore costs 6 to 7 min more than the natural one.

Without StarNet2, the stacking succeeds, but the stars stay in the nebula
layer: they are neither softened nor reduced, so the render is
different (more stars, brighter). With StarNet2, the stars are separated
and the render is the complete one.

## Camera RAW files

No library is added for them. Siril reads a RAW file itself (LibRaw, which Siril
ships under its own licence: GPL-3.0-or-later for the whole application), and
`convert` turns it into a one-layer FITS that keeps the sensor's mosaic; the
stacking and the measure of the poses give the RAW files to Siril and read its
copies. Astro Sync reads the EXIF block of a RAW file (exposure, ISO, date,
camera) itself, in Rust, with no dependency. The stacking environment gets no
`rawpy`.

## Working space on disk

Measured: about **210 MB per frame** of 3008 × 3008 (calibrated copy and aligned
copy, 32-bit, in colour), plus 35 MB per flat; 3.2 GB for 12 lights,
10 darks, 10 flats and 10 dark-flats. A night of 120 frames therefore needs about
**25 GB** of temporary space, freed at the end. Siril links the original FITS
without copying them.

## The render pipelines

A final image is sorted along two independent axes: the
**render** (the sensor's colours, or a second render that corrects a
dual-band filter) and the **curve** ("raw", "presentation", "print"). Each
render is a **pipeline**, described by a
JSON file read by Astro Sync. The full format, the validation
rules and the language fallback are in `pipelines/SCHEMA.md`.

### The format, in brief

(A pipeline's dependencies, the registry and the scripts specific to a
pipeline: `README.md` § "Declaring a pipeline and its dependencies".)

`pipelines/<id>/pipeline.json`: `id` (equal to the folder name), `version`
("1"), `name` and `description` (one text per language, `en` required:
fallback, then the language without its region, then English), `filter` (the target
capture filter: `none`, `dual-band`…), `author`, `licence` (MIT for the shipped
definitions), `entry` (`renders:<function>`, a **shipped** function of
`renders.py`, or `script:<path>`, a Python script of the pipeline's folder, run
only if the person trusts it), `requires`
(`["siril", "starnet2?"]`, the question mark makes the tool optional),
`outputs` (`["tif", "png"]`). The shipped definitions are in
`stacking/pipelines/`; those the person adds go in
`<Astro Sync data>/pipelines/<id>/pipeline.json` (option
`--pipelines-dir`, repeatable). An invalid definition is **ignored and reported**
(`list-pipelines --json` › `ignored`, event `plan` › `ignoredPipelines`,
`check_environment.py`); an identifier already taken by a shipped definition
cannot be redefined.

### The four shipped pipelines

| Identifier (`--pipeline`) | Name (en) and target filter         | What sets it apart                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                            | What it produces                                                      |
| ------------------------- | ----------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------- |
| `raw` (default)           | "Natural render"; `none`            | the sensor's native colours, never remapped: gradient removal (degree 1), black point at the 0.05 percentile, common arcsinh stretch, soft toe in the shadows (power 0.71), weak local contrast protected in the highlights, reduced stars (× 0.52)                                                                                                                                                                                                                                                                                                           | `<name>_raw.tif` (16-bit RGB, deflate) and `<name>_raw.png`           |
| `dualband-correction`     | "Dual-band correction"; `dual-band` | **the same luminance**, then a single global, continuous colour transformation (green toward cyan, red toward red-violet, slight blue-violet balance, bounded saturation), smoothed chrominance; stars × 0.58; brings a dual-band capture back toward natural hues                                                                                                                                                                                                                                                                                            | `<name>_dualband-correction.tif` and `<name>_dualband-correction.png` |
| `polished`                | "Polished render"; `none`           | the natural render pushed further without changing the colours, from its own linear preparation: background and light denoising (0.3) by GraXpert (or Siril's `subsky -rbf` + `denoise`, fallback and task choice), neutral sky then SCNR at 50% (skipped on a dual-band capture), deconvolution aimed at 0.85 × the measured FWHM (skipped below 2.0 px), stretch with a soft shoulder, moderate local contrast on the nebula alone, sky level brought back to that of the natural render, stars × 0.48; without GraXpert or Siril: polynomial, no denoising | `<name>_polished.tif` and `<name>_polished.png`                       |
| `hoo`                     | "HOO palette"; `dual-band`          | the preparation and brightness of the natural render; Hα (red pixels) toward red, OIII (greens and blues) toward blue-green, OIII brought closer to Hα by a bounded factor (1 to 2), a palette that takes over from the natural render through a smoothed mask (no threshold), sky identical to the natural render's; stars in natural colours (× 0.52)                                                                                                                                                                                                       | `<name>_hoo.tif` and `<name>_hoo.png`                                 |

Each definition describes its steps in `details` (ten languages), which the
window shows under "Details" (`pipelines/SCHEMA.md` § Details).

**Inputs**: identical for both, the linear RGB master produced by the
stacking; the renders share everything up to the star separation, so
ticking both (`--pipeline raw --pipeline dualband-correction`) only
costs a few seconds more (measured: 32 s in all for 12 frames, of which
21 s of rendering for the two). **Format**: identical (full-resolution 16-bit
TIFF, the image that is sent; 8-bit PNG for viewing; no JPEG).
**Fingerprints**: the TIFF and PNG files of `raw` and `dualband-correction` are
identical byte for byte to those of the former `--variant natural` and
`--variant aesthetic`, on Python 3.12, 3.13 and 3.14.

**Inherited renders**: `natural_v23` and `dualband_v7` stay in `renders.py`
as references (command line of that module); no definition
names them.

**Discarded candidate**: the "presentation curve" (an S curve of gain 2.0
pivoted on the sky level). It is a second axis applied to each render: it may
become a pipeline parameter later, not a third pipeline.

### Several renders per upload

- When the service allows variants (`capture.variant_limit` > 0),
  "Processing" offers to **tick
  several pipelines**; Astro Sync runs the script with one `--pipeline` per
  box and sends each render as an image of the capture. A capture has a
  **main image** and **variants** (this is not versioning: no
  history, no return to a previous version). If the target has no
  image online yet, the first render is the main image of the new capture
  and each additional render is a variant; if the target already has a main
  image, each render goes as a variant, private on arrival, and only the owner
  makes it the main image from the web ("Use as main image"). The service refuses beyond what
  `capture.variant_limit` allows (the variants in addition to the main image).
- When the service allows none (`capture.variant_limit` 0), the
  treatments can be applied,
  but the person **chooses a single render before the upload** (a single choice, not
  boxes); the others stay in `stacked/`, locally.
