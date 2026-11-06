import assert from "node:assert/strict";
import { existsSync, readFileSync, statSync } from "node:fs";
import test from "node:test";

import { createTranslator } from "../i18n/locale.ts";
import {
  ANCHOR_LABEL_KEY,
  EXAMPLE_NIGHT,
  FINDER_CAPTURE_FILE,
  GUIDE_ANCHORS,
  NINA_PATTERN,
  STRUCTURE_NOTES,
  STRUCTURE_ROWS,
  assistantFile,
  finderCaptureFor,
  finderColumns,
  guidePlatform,
  linkCommand,
  ninaPath,
  sectionOf,
} from "./guide.ts";

const fr = createTranslator("fr");
const plain = (text: string) => text.replaceAll(" ", " ").replaceAll("'", "’");
const same = (actual: string, expected: string) =>
  assert.equal(plain(actual), plain(expected));

test("the guide opens on six anchors, in their order and words, each to its own section", () => {
  assert.deepEqual(
    GUIDE_ANCHORS.map((anchor) => plain(fr(ANCHOR_LABEL_KEY[anchor]))),
    [
      "La structure",
      "Dans le Finder",
      "Déplacer plutôt que copier",
      "Liens symboliques",
      "Régler NINA",
      "Assistant IA",
    ],
  );
  const sections = GUIDE_ANCHORS.map(sectionOf);
  assert.equal(new Set(sections).size, GUIDE_ANCHORS.length);
  same(
    fr("guide.lead"),
    "Comment Astro Sync range une nuit, et comment l’aider.",
  );
});

test("Guide: the structure table says what each folder receives and what Astro Sync does with it, results only", () => {
  assert.deepEqual(
    STRUCTURE_ROWS.map((row) =>
      typeof row.path === "string" ? row.path : plain(fr(row.path.key)),
    ),
    [
      "logs/",
      "calibration/darks/ · flats/ · dark-flats/ · biases/",
      "<cible>/lights/",
      "<cible>/final/",
    ],
  );
  same(
    fr(STRUCTURE_ROWS[0].useKey),
    "Lus : paliers, guidage. Les journaux bruts restent ici ; seul le rapport peut partir.",
  );
  same(
    fr(STRUCTURE_NOTES[2]),
    "Seuls des résultats sont envoyés : l’image finale ou empilée, le rapport des journaux, les solutions de ciel, les images de progression. Les poses FITS et les journaux bruts ne quittent jamais cet ordinateur.",
  );
  // A variant, never a revision nor a version.
  assert.ok(!/révision|version/u.test(fr(STRUCTURE_ROWS[3].useKey)));
  same(
    fr(STRUCTURE_ROWS[3].useKey),
    "Envoyée : c’est la capture de la cible (ou sa variante).",
  );
});

test("Guide: the Finder drawn grey, the night, its targets, a target's frames", () => {
  const columns = finderColumns("Astro Sync");
  assert.equal(columns.length, 4);
  assert.deepEqual(
    columns.map((column) => column.items.find((item) => item.open)?.name),
    ["Astro Sync", EXAMPLE_NIGHT, "M31", "lights"],
  );
  same(fr("guide.finder.drawn"), "Finder · dessiné, non capturé");
  same(
    fr("guide.finder.windows"),
    "Sous Windows, l’Explorateur montre la même structure ; « Ouvrir dans le Finder » devient « Ouvrir dans l’Explorateur ».",
  );
});

test("Guide: the real capture replaces the drawing on macOS and Linux, never on Windows", () => {
  assert.equal(
    guidePlatform(
      "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15",
    ),
    "macos",
  );
  assert.equal(
    guidePlatform(
      "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 Edg/140.0",
    ),
    "windows",
  );
  assert.equal(
    guidePlatform("Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/605.1.15"),
    "linux",
  );
  const present = [FINDER_CAPTURE_FILE, "other.png"];
  assert.equal(finderCaptureFor("macos", present), FINDER_CAPTURE_FILE);
  assert.equal(finderCaptureFor("linux", present), FINDER_CAPTURE_FILE);
  // No capture of the Explorer exists: the grey drawing stays.
  assert.equal(finderCaptureFor("windows", present), null);
  // No file found at build time: the drawing stays too.
  assert.equal(finderCaptureFor("macos", []), null);
  // Its caption and its description exist, in French as written for the page.
  same(
    fr("guide.finder.captured"),
    "Finder · macOS, sur un dossier de démonstration",
  );
  assert.match(plain(fr("guide.finder.alt")), /^Capture du Finder : /u);
});

test("Guide: the capture file is a real PNG of the demonstration folder, reduced", (t) => {
  const url = new URL(
    `../assets/guide/${FINDER_CAPTURE_FILE}`,
    import.meta.url,
  );
  if (!existsSync(url)) {
    // The capture is taken on a Mac whose person has allowed screen
    // recording; until it is, the drawing stays and
    // this test is reported as SKIPPED, never as passed.
    t.skip("assets/guide/finder-macos.png not captured yet");
    return;
  }
  const bytes = readFileSync(url);
  assert.deepEqual(
    [...bytes.subarray(0, 8)],
    [137, 80, 78, 71, 13, 10, 26, 10],
  );
  const width = bytes.readUInt32BE(16);
  const height = bytes.readUInt32BE(20);
  assert.ok(width >= 900 && width <= 2400, `width ${width}`);
  assert.ok(
    width / height >= 2 && width / height <= 6,
    `ratio ${width / height}`,
  );
  assert.ok(statSync(url).size < 600_000, "reduced: under 600 kB");
  // No text chunk (a path of the machine, a user name) travels with the image.
  for (const chunk of ["tEXt", "iTXt", "zTXt", "eXIf"]) {
    assert.ok(!bytes.includes(Buffer.from(chunk)), `no ${chunk} chunk`);
  }
});

test("Guide: the four tips, the command and the pattern to copy, ASSISTANT-IA.md to open", () => {
  const root = "~/Documents/Astro Sync";
  assert.equal(
    linkCommand(root, "<disque externe>"),
    'ln -s "<disque externe>/2026-10-07/Light" \\\n  "~/Documents/Astro Sync/2026-10-07 · Col des Étoiles · 7f3a2c/M31/lights"',
  );
  assert.equal(
    ninaPath(root),
    "~/Documents/Astro Sync/2026-10-07 · Col des Étoiles · 7f3a2c",
  );
  assert.equal(
    NINA_PATTERN,
    "$$TARGETNAME$$\\lights\\$$DATETIME$$_$$FILTER$$_$$EXPOSURETIME$$s_$$FRAMENR$$",
  );
  assert.equal(
    assistantFile("/home/camille/Documents/Astro Sync/"),
    "/home/camille/Documents/Astro Sync/ASSISTANT-IA.md",
  );
  same(fr("guide.copyCommand"), "Copier la commande");
  same(fr("guide.copyPattern"), "Copier le modèle");
  same(fr("guide.openAssistant"), "Ouvrir ASSISTANT-IA.md");
  // The AI tip says the file is one, in English, and the prompt in the
  // person's language.
  assert.match(plain(fr("guide.ai.line")), /unique et en anglais/u);
  assert.match(plain(fr("guide.ai.line")), /« Parle-moi en français. »/u);
});
