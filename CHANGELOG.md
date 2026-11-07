# Changelog

All notable changes to Astro Sync are documented in this file. The project
follows [Semantic Versioning](https://semver.org/).

## 1.0.2 (2026-11-07)

- Renders are uploaded as PNG. The TIFF of a render leaves only when you tick
  it: the import screen lists each TIFF unticked under its PNG, the summary
  shows the format of every render, and a new setting, « Uploads: PNG only /
  PNG + TIFF », chooses what is sent by default.
- The « already captured » bookmark has three states, told apart by their
  shape and not by colour alone.
- The German texts keep the informal register of the other languages: a line
  of the uploads screen no longer starts a sentence with « Sie ».
- French typography: no-break spaces before the double punctuation marks in
  the French strings.
- The installers are unchanged: the Windows installer is still unsigned.

## 1.0.1

- The contrast chosen for a pipeline in the stacking assistant now reaches
  the image of the multi-night plan: a strong contrast or a custom value no
  longer falls back to the default of the settings.
- Uploads that were still queued when the computer was linked to a space
  (version 1.0.0 kept them « before the link ») are adopted by the linked
  space at the next start and leave with the next « Send all ». Without a
  linked space they are shown as « To import again », never as « Waiting ».
- When the portal does not know the plan an upload refers to, Astro Sync
  says so exactly (« This plan does not exist in this space: import the
  night again from Sessions », in the ten languages) instead of a generic
  failure.
- The installers are unchanged: the Windows installer is still unsigned.

## 1.0.0

First public release of the repository.

- The desktop application (Tauri 2) and the command line (`astro-sync`) over
  the same library.
- Association of the folders where your acquisition software writes a night,
  read-only: no source file is ever copied, moved, renamed or deleted. The
  folders are read when you start an import, never watched in the background.
- Detection of the nights, targets and calibration frames; the package of each
  session; resumable uploads to your private server or to the hosted service.
- Analysis of the night on your computer: the quality and the guiding of each
  frame.
- Optional stacking of the FITS files of a target (Python, in `stacking/`).
- Installers for macOS (Apple silicon and Intel, signed and notarized),
  Windows (x64, not yet signed: Windows may show a SmartScreen warning) and
  Linux (x64, Ubuntu 22.04 or newer: a `.deb` and an `.AppImage`, not signed),
  and the command line for the four of them, all built by the release
  workflow of the repository.
- Ten interface languages: English, French, German, Spanish, Italian,
  Japanese, Brazilian Portuguese, Russian, Simplified and Traditional Chinese.
- The command line speaks English; the refusals and failures it shares with
  the application are said in the language of your computer when the
  application has it, or the one you give with `--lang <locale>` (or
  `ASTRO_SYNC_LANG`).
- The integration lines of a night are sent to a multi-night plan as the complete
  set of the night and its target: when a night is stacked again under another
  filter, the line of the filter it no longer integrates is removed from the plan
  by the service (`replace` in `PUT …/integrations`), so the night is counted once
  in the totals and the plan's page. A server that predates this is sent the earlier
  form (lines added or replaced, none removed). Nothing is sent for a night whose
  masters cannot be read, so a disk that is not mounted never empties a plan.
- The image of a multi-night plan with a dual-band night no longer loses its
  corners when a broadband night was taken with the camera turned: each
  broadband master is now registered directly on the dual-band frame and the
  broadband nights are averaged wherever at least one of them covers, levelled
  on the longest. Before, they were merged on the frame of the longest
  broadband night first, and whatever that night did not reach was left black
  (an octagon, read as a turned image). With a single broadband night the
  image is unchanged, to the bit.
- "Detect sessions": when the same folder is given as the FITS, the
  calibration and the logs folder, the band each night's names say
  (`…_dualband`, `…_RGB`) is preselected again. The FITS folders are read
  first; a light found in a calibration folder gets its band suggestion too; a
  calibration frame reached from a night through a link is still read as
  calibration.
- A night stacked again under another filter (its lights name none) keeps one
  master in the plan: of the masters of the same poses, the one whose filter
  the target folder records now, else the newest. The other stays on disk,
  untouched, out of the plan image and out of the integration lines, so the
  night no longer counts twice.
