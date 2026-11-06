# Pipeline definitions

A **pipeline** is one way to render the stacked image: it is described by one
`pipeline.json` file, read by Astro Sync (the "Processing" screen lists the
pipelines; the stacking script applies them). Four render pipelines ship with
Astro Sync, in this folder; people can add their own.

```
<pipelines folder>/
  <id>/
    pipeline.json
```

Where definitions are read, in this order:

1. `stacking/pipelines/` (shipped with the app): `raw`,
   `dualband-correction`, `polished` ("Polished render") and `hoo` ("HOO
   palette").
2. `<Astro Sync data>/pipelines/` (added by the person; the app passes it to
   the script with `--pipelines-dir`). The `--pipelines-dir` option can be
   repeated.

The first definition of an `id` wins: an added definition can never replace a
shipped one.

## Fields

All fields are required, except `dependencies`, `details` and `preview` (optional).

| Field          | Type                       | Rule                                                                                                                                                                                                                                                                                                                                                 |
| -------------- | -------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `id`           | text                       | 1 to 40 characters: `a-z`, `0-9`, `-`; not starting with `-`. **Equal to the folder name.** Used in the file names (`<name>_<id>.tif`) and in `--pipeline <id>`. `linear` is reserved (the option "No processing", `<name>_linear.tif`): a definition with that id is ignored.                                                                       |
| `version`      | text                       | The format version of this file: `"1"`. A version the script does not know makes the definition ignored.                                                                                                                                                                                                                                             |
| `name`         | object, text per language  | The short name shown to the person, e.g. "Natural render". Up to 600 characters per text.                                                                                                                                                                                                                                                            |
| `description`  | object, text per language  | One or two sentences telling what the render does and when to choose it. Up to 600 characters per text.                                                                                                                                                                                                                                              |
| `filter`       | text                       | The capture filter this render is meant for: lower-case words joined by `-`. Known values: `none` (no filter), `dual-band`. Others are accepted and shown as they are. It says for which targets the pipeline is **recommended** and for which it is **discouraged** (`recommendedFor`, `discouragedFor` in `list-pipelines --json`), never refused. |
| `author`       | text                       | Who wrote the definition.                                                                                                                                                                                                                                                                                                                            |
| `licence`      | text                       | The licence of the definition, as an SPDX identifier when there is one (`MIT`, `CC0-1.0`). The shipped ones are `MIT`.                                                                                                                                                                                                                               |
| `entry`        | text                       | What renders the image, in one of two forms: `renders:<function>`, a render function **shipped with Astro Sync** in `renders.py` (listed in its `RENDERS` table), or `script:<relative path>`, **a Python file of the pipeline's own folder** (see « Pipeline scripts and trust »).                                                                  |
| `requires`     | list of text               | The external tools the render needs: `siril` (always listed), `starnet2` and `graxpert`. A trailing `?` makes a tool optional: `["siril", "starnet2?"]`.                                                                                                                                                                                             |
| `outputs`      | list of text               | The files written next to the stacked image: `tif` (16-bit, the image that is sent; required) and `png` (8-bit, for looking at it).                                                                                                                                                                                                                  |
| `dependencies` | list of objects (optional) | What the pipeline needs installed, by identifier in the **dependency registry** (see below): why, required or not. Absent = the pipeline declares nothing more than `requires`.                                                                                                                                                                      |
| `details`      | list of objects (optional) | The steps of the render, **in order**, as the "Details" line under the pipeline shows them: 1 to 12 steps, each exactly `{ "title": {…}, "text": {…} }`, two texts per language (same language rules as `name`, `en` required, up to 600 characters each). See « Details » below.                                                                    |
| `preview`      | text (optional)            | The path, **relative to the pipeline's folder**, of an image showing what the render looks like (`preview.jpg`): PNG or JPEG. See « Preview » below. A preview that cannot be used is ignored, never the pipeline.                                                                                                                                   |

### Texts and languages

`name` and `description` are objects whose keys are language codes: `fr`, `en`,
`ja`, `de`, `es`, `it`, `pt-BR`, `ru`, `zh-Hans`, `zh-Hant` for the shipped
ones. **`en` is required**: it is the fallback. To show a text in a language,
the app takes, in this order:

1. the text for that exact code (`pt-BR`);
2. the text for the same language without its region (`pt`);
3. the `en` text.

So a definition may give only the languages its author speaks. A person who
adds a pipeline writes in the second person ("vous") in French and in the
polite register of the other languages, like the shipped ones.

### Details

`details` tells, step by step, what the render does and **why**, in the tone of
`description`: the title is a few words ("Sky background by GraXpert"), the
text one or two sentences that say what the step changes in the image and what
happens when an optional tool is missing. The steps follow the order in which
they run. A definition without `details` is valid; the window then shows no
"Details" line.

```json
"details": [
  {
    "title": { "en": "Sky background", "fr": "Fond de ciel" },
    "text": {
      "en": "A smooth model of the background is subtracted, then the black point is set.",
      "fr": "Un modèle lisse du fond est soustrait, puis le point noir est posé."
    }
  }
]
```

A step that is not an object with exactly `title` and `text`, a text table
without `en`, an empty list or more than 12 steps make the definition
**ignored**, like any other broken field.

### Preview

`preview` lets the window that lists the pipelines show what each render looks
like, in the card of the pipeline, under its description. It is optional: a
pipeline without one has a card without an image, nothing is invented for it.
(« No processing » is not a pipeline and has no definition: Astro Sync provides
its image, the same stack, not stretched, in grey.)

```json
"preview": "preview.jpg"
```

| Rule             | Value                                                                                                                                                                                                                                                  |
| ---------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Path             | Relative to the pipeline's folder (`<id>/preview.jpg`), `/` separators, no `..`, no absolute path, ending in `.png`, `.jpg` or `.jpeg`. A link that leads out of the folder is refused.                                                                |
| Format           | PNG or JPEG, **checked on the first bytes**, not on the extension.                                                                                                                                                                                     |
| Size (pixels)    | **1200 × 300** recommended: a 4:1 banner, shown under the description (the card shows it cropped in width, `object-fit: cover`, so keep the subject in the middle).                                                                                    |
| Size (bytes)     | About 100 KiB advised; **above 512 KiB the image is ignored**.                                                                                                                                                                                         |
| Content          | The shipped pipelines all show **the same crop of the same target** (the Heart Nebula, IC 1805), each rendered by its own pipeline, so that the cards compare. Keep personal data out of the image and its metadata (strip EXIF, ICC and text chunks). |
| Alternative text | Generated by the app (« Preview of the "<name>" render »): nothing to write in the JSON.                                                                                                                                                               |

A `preview` that is not a text, leaves the folder, is missing, is too large or is
neither a PNG nor a JPEG is **ignored as a field**: the pipeline stays valid and
listed, without a preview, and the reason is reported as `previewIssue` on that
pipeline in `list-pipelines --json` (not in `ignored`, which lists the
definitions left out). `preview` is the checked relative path (`null` when
absent or ignored); the app reads the image itself, when the list is made, and
passes it to the window.

## Dependencies

A pipeline says **what it needs** and **why**; it never says where to get it.
Everything that can be installed, detected or downloaded lives in the
**dependency registry** of the repository, `dependencies/registry.json` (next to
`pipelines/`, see « The dependency registry » below). A `pipeline.json` carries
no URL, no checksum, no detection command: only references to the registry.

`dependencies` is a list; each entry has these fields and no other (a
`url`, `downloads`, `check`, `page`… in a manifest makes it invalid):

| Field        | Type                      | Rule                                                                                                                                                                                                                                                                               |
| ------------ | ------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `id`         | text                      | An identifier of the registry. An identifier that is not in the registry makes the **whole manifest invalid**: « dependency '…' is not allowed […]; propose it with a pull request on the repository » (the message also carries its French equivalent). Listed once per pipeline. |
| `required`   | boolean                   | `true`: the render cannot be made without it; `false`: the render is made without it, less well (the optional tool of `requires`, with `?`). For `siril`, `starnet2` and `graxpert`, it must agree with `requires` (`"starnet2?"` ⇔ `false`).                                      |
| `reason`     | object, text per language | Why **this pipeline** needs it: one sentence, in the second person ("vous") in French and in the polite register of the other languages. Same language rules as `name` (`en` required, fallback).                                                                                  |
| `minVersion` | text (optional)           | A minimum version stricter than the registry's: dotted numbers (`"1.4.4"`), **never lower** than the registry's own (otherwise the manifest is invalid). Only for a `tool`; a `python-packages` dependency takes the registry's range as it is.                                    |

```json
"dependencies": [
  { "id": "siril", "required": true, "reason": { "en": "Siril stacks your exposures.", "fr": "Siril additionne vos poses." } },
  { "id": "starnet2", "required": false, "reason": { "en": "StarNet2 reduces the stars." } },
  { "id": "python-env", "required": true, "reason": { "en": "The Python environment runs the script." } }
]
```

## The dependency registry

`stacking/dependencies/registry.json`: `{"format": 1, "dependencies":
[...]}`, one entry per tool that a pipeline is allowed to depend on. **The only
way to add an entry is a pull request on the repository**: a person who adds a
pipeline in their data folder can only use what is already in the registry.
Each entry:

| Field          | Type                      | Rule                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                      |
| -------------- | ------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `id`           | text                      | 1 to 40 characters: `a-z`, `0-9`, `-`. Unique in the registry.                                                                                                                                                                                                                                                                                                                                                                                                                                                                            |
| `name`         | object, text per language | The name shown on the "Dependencies" screen, ten languages for the shipped entries (`en` required, same fallback as the pipelines).                                                                                                                                                                                                                                                                                                                                                                                                       |
| `kind`         | text                      | `tool` (a program found by a command and downloaded as a file) or `python-packages` (the pinned packages of the stacking environment).                                                                                                                                                                                                                                                                                                                                                                                                    |
| `minVersion`   | text                      | `tool`: the minimum version, dotted numbers (`"1.4"`; `1.4.4` satisfies it). `python-packages`: the range of the Python interpreter, e.g. `">=3.12,<3.15"` (clauses `>=`, `>`, `<=`, `<`, `==` joined by `,`). For a `tool`, `minVersion` and `check` go together: both or neither.                                                                                                                                                                                                                                                       |
| `check`        | object                    | `tool` only, with `minVersion`. `command`: a bare program name, or a list of names tried in order (`["siril-cli", "siril"]`); `args`: **a version query only**, one of `["--version"]`, `["-version"]`, `["-V"]`, `["-v"]`, `["version"]`; `versionPattern`: a regular expression with **one** group that captures the dotted version in the output (standard output and error). Used for the states present / missing / version too old. An entry without `check` can only be shown « not checked »; Astro Sync's own detection decides. |
| `downloads`    | object (optional)         | `tool` only. Keyed by system: `macos-arm64`, `macos-x64`, `windows-x64`, `linux-x64` (no other key). Each value: `url` (`https`) and `sha256` (64 lower-case hexadecimal characters) — **both or neither, never one without the other**; `bytes` (a positive integer, optional) for the size. A system with no entry has no automatic download. A URL ending in `.zip` is unpacked into `tools/<id>/` once its digest is checked (no administrator right), then deleted.                                                                  |
| `sizes`        | object (optional)         | `tool` or `python-packages`. Keyed by system like `downloads`; each value a positive integer: the bytes the person downloads **by hand** for that system when there is no automatic download (the archive of the `page`), or the pinned packages of a `python-packages` entry. Read only to say the size ("Download (90 MB)"); it never makes a download possible.                                                                                                                                                                        |
| `requirements` | text                      | `python-packages` only: the path, relative to the registry file, of the pinned `requirements.txt` (every package `==` pinned and carrying `--hash=sha256:…`). A `python-packages` entry has **no `downloads`** (the packages come from the index, checked by their pinned digests).                                                                                                                                                                                                                                                       |
| `page`         | text                      | The `https` URL of the manual download page (the "Download page" button). An entry without `downloads` only offers this page.                                                                                                                                                                                                                                                                                                                                                                                                             |
| `licence`      | text                      | The licence of the thing installed, SPDX when there is one; free text for a tool that has none (« Freeware (all rights reserved) »).                                                                                                                                                                                                                                                                                                                                                                                                      |

A registry that breaks one of these rules is **rejected as a whole** (it ships
with the app, a human fixes it); the pipelines that declare dependencies are
then ignored with the reason « the dependency registry cannot be read ».

### What the app does with them

- **Detection** (`check`): run the command with the version query, read the
  version with `versionPattern`, compare with the effective minimum (the larger
  of the registry's and the manifest's): `present`, `missing` (nothing ran, or no
  version could be read), `outdated` (a version was read but is below the
  minimum or outside the range). For `python-packages`: the interpreter must
  satisfy `minVersion` and every pinned package must be installed at its pin.
- **Download** (`downloads`): the file for the person's system, checked against
  its `sha256` before any use; a system with no entry, or a failure, leaves the
  `page`. Nothing is ever bundled in the app.
- **"Install all"** covers the required dependencies that are missing, for
  every pipeline the app found, **including the ones the person added**: since
  the registry is the only source, a pipeline added in the data folder cannot
  bring anything the repository has not already allowed.

## Pipeline scripts and trust

A pipeline can bring its own render: `"entry": "script:render.py"`. The path is
relative to the pipeline's folder (`<id>/render.py`), ends in `.py`, uses `/`
separators, has no `..` and stays inside the folder (a link out of it is
refused; up to 256 KiB). The script is run **in the environment of the
registry** (`python-env`: the Python and the pinned packages of the stacking
environment, nothing else is installed for it) and must define:

```python
def render(nebula, stars):
    """Both arguments: float arrays of shape (height, width, 3), values 0..1,
    the nebula and the stars after StarNet2, already gradient-corrected and
    stretched (or the whole image and zeros when StarNet2 is not used). Return the image to save: the same shape and range."""
```

It may import the pinned packages (numpy, scipy, scikit-image…) and the helpers
of `renders.py` (`lum`, `controlled_stars`…). It writes nothing itself: Astro
Sync saves the returned image as `<name>_<id>.tif` / `.png` -- and, when the
person asked for the contrast of presentation (`--contrast`), once more with the
curve applied to it, as `<name>_<id>_presentation` or `_strong`: the script
itself never sees the option.

`list-pipelines --json` returns, for every pipeline, `entry` and
**`scriptSha256`**: the SHA-256 (64 lower-case hexadecimal characters) of the
script's bytes, `null` for a `renders:` entry.

### The trust rule

A script is code that runs on the person's computer, on their photographs.

| Pipeline                                                                                          | What happens                                                                                                                                                    |
| ------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Shipped with Astro Sync** (its folder is in the app, added by a pull request on the repository) | Runs without a question.                                                                                                                                        |
| **Added by the person** (`source: "user"`) with a script, first use                               | Astro Sync warns: "You added this pipeline yourself; Astro Sync has not verified it. Do you trust it?" (shown in the app's language)                            |
| …the person says yes                                                                              | Astro Sync records the script's `scriptSha256` **in Astro Sync's own data** (never in the pipeline's folder, which the pipeline's author controls) and runs it. |
| …the script's SHA-256 is different later                                                          | The question is asked again (the file changed since it was trusted).                                                                                            |
| …the person says no                                                                               | The pipeline stays listed but **disabled**, and is never run.                                                                                                   |

The question, the recording of the digest and the disabled state belong to the
app. **The script enforces the rule too**: the command line
`astro_sync_stack.py --trusted <id>=<sha256>` (repeatable) lists the digests the
person approved, and a pipeline added by the person that brings a script is
**refused with exit code 2 and the error `script_not_trusted`** unless its
digest is listed (a different digest gives the same error: « it changed since »).
The script is read once: the digest that is checked is that of the bytes that
are run. A shipped pipeline is never refused. A pipeline with a `renders:` entry
needs no approval.

### What a pipeline cannot do (v1)

- Its manifest carries **no download, no URL, no detection command**: a
  dependency names an entry of the registry, the only list of what can be
  installed (a pull request to extend it).
- A script is trusted by the person, one digest at a time; there is no
  other way to run code from a definition: no `entry` outside the two forms,
  nothing run when the definitions are listed.

## Example

```json
{
  "id": "raw",
  "version": "1",
  "name": { "fr": "Rendu naturel", "en": "Natural render" },
  "description": {
    "fr": "Les couleurs du capteur, sans retouche de teinte.",
    "en": "The sensor’s colours, with no hue adjustment."
  },
  "filter": "none",
  "author": "Astro Sync",
  "licence": "MIT",
  "entry": "renders:natural_deep",
  "requires": ["siril", "starnet2?"],
  "outputs": ["tif", "png"],
  "dependencies": [
    {
      "id": "siril",
      "required": true,
      "reason": { "en": "Siril stacks your exposures." }
    },
    {
      "id": "starnet2",
      "required": false,
      "reason": { "en": "StarNet2 reduces the stars." }
    }
  ]
}
```

## An invalid definition

A definition that cannot be read, is not valid JSON, breaks a rule above, has
an `id` already taken, or is in a folder without a `pipeline.json` is
**ignored and reported**; the other pipelines stay available.
`astro_sync_stack.py list-pipelines --json` returns it in `ignored`
(`folder`, `source`, `reason`), the stacking `plan` event returns it in
`ignoredPipelines`, and `check_environment.py` prints it. The app should show
the reason on the "Processing" screen, not hide it.

## Using the pipelines

```sh
python3 astro_sync_stack.py list-pipelines --json [--pipelines-dir DIR]... [--lang fr]
python3 astro_sync_stack.py --pipeline raw --pipeline dualband-correction … # one image per pipeline
```

`list-pipelines --json` prints `{"format": 1, "pipelines": [...], "ignored":
[...], "registry": [...]}`. Each pipeline carries every field above plus
`source` (`shipped` or `user`); with `--lang <code>` it also carries
`displayName` and `displayDescription`, already resolved with the fallback
above, and `displayDetails`: the steps of `details` as `[{"title", "text"}]`
in that language (an empty list when the definition has none; `details` itself
is always listed, as written). Its `dependencies` are **resolved**: the entry of the registry (`name`,
`kind`, `check`, `downloads`, `page`, `licence`, and for `python-packages`
`requirements` and `requirementsPath`, the file's location on this computer)
plus the manifest's `required`, `reason` and the effective `minVersion`
(with `--lang`: `displayName`, `displayReason`). `registry` is the whole
registry, so a screen can list what is allowed. The default
pipeline (`raw`) comes first. An unknown id given to `--pipeline` stops the
script with exit code 2 and the error `unknown_pipeline`, which lists the known
ids.
