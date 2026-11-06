import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

import { EDITABLE_GENRES, isEditableGenre, type Genre } from "./incidents.ts";
import { createTranslator } from "../i18n/locale.ts";
import { SUPPORTED_LOCALES } from "../i18n/locale.ts";

// The kind picker: the dialog offers only the four kinds a person
// can declare; the four Astro Sync detects are shown, never chosen.

const source = (relative: string) =>
  readFileSync(new URL(`../${relative}`, import.meta.url), "utf8");

const DETECTED: Genre[] = [
  "exposure_change",
  "guiding_timeouts",
  "sensor_temperature_drift",
  "guiding_spikes",
];

test("the picker is the four declarable kinds, in their display order", () => {
  assert.deepEqual(
    [...EDITABLE_GENRES],
    ["condition", "equipment_issue", "pause", "note"],
  );
  for (const genre of EDITABLE_GENRES)
    assert.equal(isEditableGenre(genre), true);
});

test("the four detected kinds are shown, not chosen: never editable", () => {
  for (const genre of DETECTED) assert.equal(isEditableGenre(genre), false);
});

test("a line of a detected kind has a lock in place of the pencil", () => {
  const card = source("components/IncidentsCard.tsx");
  assert.match(card, /isEditableGenre\(line\.genre\) \? \(/);
  assert.match(card, /className="as-incident__tool as-incident__lock"/);
  assert.match(card, /aria-label=\{t\("incidents\.lock\.title"\)\}/);
  assert.match(card, /title=\{t\("incidents\.lock\.title"\)\}/);
  assert.match(card, /<Icon name="lock"/);
  // A state, not a button: the lock is not a focus stop.
  const lock = card.slice(card.indexOf("as-incident__lock"));
  assert.doesNotMatch(
    lock.slice(0, lock.indexOf("</span>")),
    /onClick|tabIndex/,
  );
});

test("the grey help line is in « Ajouter » only, under the four segments", () => {
  const card = source("components/IncidentsCard.tsx");
  assert.match(
    card,
    /\{adding && \(\s*<p\s+className=\{`\$\{helpTextClassName\} as-incident-dialog__detected-help`\}\s*>\s*\{t\("incidents\.genre\.detectedHelp"\)\}/,
  );
  assert.ok(
    card.indexOf("EDITABLE_GENRES.map") <
      card.indexOf('t("incidents.genre.detectedHelp")'),
    "the sentence follows the segments",
  );
  assert.doesNotMatch(card, /Autre|incidents\.genre\.other/);
});

test("the lock label and the help line are in the ten languages, same concept", () => {
  assert.equal(SUPPORTED_LOCALES.length, 10);
  for (const locale of SUPPORTED_LOCALES) {
    const t = createTranslator(locale);
    for (const key of [
      "incidents.genre.detectedHelp",
      "incidents.lock.title",
    ]) {
      const text = t(key);
      assert.notEqual(text, key, `${locale}: ${key} is missing`);
      assert.ok(text.length > 20, `${locale}: ${key} is a sentence`);
    }
    assert.match(t("incidents.lock.title"), /Astro Sync/);
    assert.match(t("incidents.genre.detectedHelp"), /Astro Sync/);
  }
  assert.equal(
    createTranslator("fr")("incidents.lock.title"),
    "Détecté par Astro Sync\u00a0: il se retire, il ne se modifie pas.",
  );
});
