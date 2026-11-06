# `ASSISTANT-IA.md`: its source

`assistant-file.json` is the source of truth of `ASSISTANT-IA.md`, the English
file Astro Sync writes at the root of the working folder: the folders, the rules,
and the step-by-step procedure an AI assistant follows to gather a night's files
`Assistant-file.schema.json` is its schema.
`flow.rs` reads the JSON and renders it; `snapshots/ASSISTANT-IA.md.txt` is that
rendering, committed, and it is the `ASSISTANT_FILE_TEXT` constant.

The copied prompt (`build_prompt`, in the ten languages of the app) stays short:
it gives the night's folder, its date (the evening, then the morning after), its
targets, and asks the assistant to follow this file one question at a time.

## Add a step

1. Add an object to `steps`, where it belongs in the order (the steps are
   numbered in that order; the first one is where the procedure starts):
   - `id`: a new, unique id;
   - `ask`: a key of `questions` (add the question there; `{files}` is replaced
     by the labels of the step's `files`), and `options`: one
     `{ "answer": <key of answers>, "next": <step id> }` per answer;
   - or `action`, one of `search`, `ask_path`, `place`, `summarize` (their words
     are in `actions`), with `files` (ids of `kinds`) and `next`;
   - `say`: an optional sentence said before the rest.
2. Point an earlier step's `next` (or one of its `options`) at the new id.
3. Only a `summarize` step ends the procedure; no step may lead back to an
   earlier one.
4. Rewrite the renderings and read the diff:
   `UPDATE_SNAPSHOTS=1 cargo test -p astro-sync-core assistant_prompt`, then
   `cargo test -p astro-sync-core assistant_prompt` again.
5. Add the digest of the text the previous version wrote to
   `EARLIER_ASSISTANT_FILE_SHA256` (in `assistant_prompt.rs`), so that an
   unchanged file is updated even when the settings lost its digest. A file the
   person changed is never overwritten.

The tests refuse an unknown step, kind, question, answer or placeholder, a step
that cannot be reached, a loop, and a path that does not end at `summarize`.

## Change a rule

Each rule is one entry of `rules`. The placement rule (`placement`: link, copy
or move) is the only text that says how a file is placed;
the `place` steps refer to it by its title.

The links rule (`links`) says how to search: symbolic links,
Finder aliases, `.lnk` shortcuts and junctions, loops. Every `search` step
refers to it by its title (`{links}` in `actions.search`).

## The multi-night plans

`multi_night` is a section written after the rules only
when the multi-night plans are open for the space (`analysis.progression` on the
hosted service): `snapshots/ASSISTANT-IA.multi-night.md.txt`, the constant
`ASSISTANT_FILE_TEXT_MULTI_NIGHT`. Each of the two texts is Astro Sync's for the
other, so the file follows the right as it comes and goes; a file the person
changed is never overwritten.
