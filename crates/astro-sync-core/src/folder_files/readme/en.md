# Astro Sync: your working folder

This is the working folder of Astro Sync. It holds your astrophotography nights in the layout Astro Sync reads. Everything here is a plain file or folder: you can open, move, edit or delete it yourself.

## What is in it

- One folder per night, named "date · plan or place · short identifier", for example `2026-10-07 · Col des Étoiles · 7f3a2c`. You can rename it: a hidden file keeps its identity.
- Inside a night:
  - `logs/`: the NINA, PHD2 and ASIAIR logs of the night;
  - `calibration/`: `darks/`, `flats/`, `dark-flats/` and `biases/`;
  - one folder per target (`M31`, `NGC 7000`…), with `lights/` for the raw FITS frames and `final/` for your processed image (TIFF, JPEG or PNG);
  - `stacked/` and `progression/`, which Astro Sync creates only when it makes an image of its own.
- Files and folders whose name starts with a dot (`.astro-sync-night.json`, `.astro-sync-target.json`, `.astro-sync-work/`) are Astro Sync's: they keep the identity of a night or a target and the scratch work of a job in progress. Leave them where they are.

## The .md files at the root

- `README.md`: this file.
- `ASSISTANT-IA.md`: the procedure an AI assistant follows to tidy the files of a night. It is in English, whatever your language.
- `GUIDE-ASTRO-SYNC.<language>.md` (for example `GUIDE-ASTRO-SYNC.en.md`): the text of the Guide page of Astro Sync, written when you use "Ask an AI a question" in the Guide. There is one file per language you asked for.
- `<name>.modifie-<date>.md` (for example `ASSISTANT-IA.modifie-2026-10-17.md`): the copy of one of these files that you had changed, kept when you chose "Replace with Astro Sync's version".

## What Astro Sync rewrites, and what it never touches

- It rewrites the `.md` files above only while they are exactly as it wrote them, so that they follow the application's updates. A file you changed is left as it is: Astro Sync tells you, and offers "Replace with Astro Sync's version", which first renames yours `<name>.modifie-<date>.md`.
- It creates folders. It never moves, renames, modifies or deletes your frames, your logs, your images or any file you copied, moved or linked here. Its own processing files (masters, stacking results, temporary files) are deleted only when you ask, in Settings › General › Processing files.

## Nothing is sent to an AI

Astro Sync does not contact any AI service. "Ask an AI assistant" and "Ask an AI a question" only copy a text to your clipboard: you give it to the assistant of your choice, which reads these files with its own access to your computer. The FITS frames and the raw logs never leave your computer; only the results you choose to send go to your Astro Portal space.
