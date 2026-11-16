# Changelog

All notable changes to Astro Sync are documented in this file. The project
follows [Semantic Versioning](https://semver.org/).

## 1.1.1 (2026-11-17)

- An image whose capture was deleted from the portal is sent again. Before an
  import is sent, Astro Sync asks the service which of the files it holds as
  sent the space no longer has, and puts them back in the queue with their
  variants. An image refused because its capture is gone now says so, in the
  ten languages.
- Multi-night plans: the final image of a plan, combined from its nights, is
  made and sent with Advanced Planning. The intermediate images (the plan's
  image after each night) still need Advanced Sync+, and the window, the
  import and the command line (`plan-image --progression`) say so.
- A target made only of XISF files no longer counts as having frames to stack:
  XISF files are read, not stacked. Every camera RAW format that Astro Sync
  reads is now taken as a frame by the queue.
- The message about a skipped target names the targets step in the language of
  the application.
- The guides inside Astro Sync, the folder's guide and the README now describe
  the stacking of camera RAW files, adjusting a finished image, the images to
  send by series, the command line, the polished render and the option that
  tidies frames into the working folder.
- The installers are otherwise unchanged: the Windows installer is still
  unsigned.

## 1.1.0 (2026-11-16)

- Adjust a finished image before it is sent. The new screen opens from each
  PNG of « Images to send » and from the PNG render of a night's summary. It
  has five settings (black point, brightness, colour, smoothing, stars) and a
  live preview. The adjusted image is written beside the render, never over
  it, and « Back to the original render » removes it. Settings can be saved
  under a name, renamed, exported and imported.
- An adjusted image takes the place of its render online. When the render is
  already the main image of a capture, the adjusted image replaces it instead
  of creating a new capture. Its TIFF carries the same settings and replaces
  the TIFF already online. The summary has a box, « Keep the existing TIFF »,
  to keep both, and the command line has `--keep-tiff`.
- Series of poses. A night that holds several series of the same target (for
  example 10 s frames and 30 s frames) gets one box per series and one capture
  per series. The main image comes from the longest series, and a series under
  a tenth of the time of another one opens unticked.
- Several sessions in one night are told apart by their clock window.
- The polished render is new. Colour is balanced on the field stars, clipped
  star cores are neutral, bright stars no longer have a dark ring, and the
  bright core of a nebula keeps its colour and texture.
- Renders of short stacks and of masters already stretched by their device no
  longer come out with a grey sky.
- Only the chosen finish of a pipeline is listed and sent. Renders sent from
  « Images to send » carry their title, steps and acquisition figures.
- After stacking, Astro Sync says how many frames were stacked and how many
  added more noise than they took away.
- Clearer failures. Astro Sync says why a tool could not be installed, why a
  polished render was made by another engine than GraXpert, and why an
  alignment failed.
- Tools installed with pipx, uv or `pip --user` are found, including when the
  window is opened from the Finder.
- The pinned digests of ASTAP's command-line archives follow the files served
  today, so ASTAP installs again.
- Sizes counted against the space are written in base 1 024, as the service
  counts them.
- The linear render of a night leaves as a file attached to the capture. It
  is unticked by default, can be downloaded from the capture, and is never
  shown in the gallery.
- Eleven findings of the Windows smoke are fixed, among them the link check
  before « Connected » is shown, an existing Python environment found and
  updated instead of installed again, the text of the uninstaller in ten
  languages and the language selector of the installer.
- The installers are otherwise unchanged: the Windows installer is still
  unsigned.

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
