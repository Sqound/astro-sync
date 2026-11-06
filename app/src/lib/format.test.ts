import assert from "node:assert/strict";
import test from "node:test";

import {
  formatBytes,
  formatDateTime,
  formatDay,
  pluralText,
} from "./written.ts";
import { createTranslator } from "../i18n/locale.ts";
import { errorReason } from "./errorReason.ts";

// 2026-09-16, 21:40 UTC: the evening of Paris is 23:40, the evening of
// Tokyo is the 17th at 06:40.
const instant = "2026-09-16T21:40:00Z";

test("a date is written as people read it, in the language of the window: « mer. 16 sept., 23:40 »", () => {
  assert.equal(
    formatDateTime(instant, "fr", "Europe/Paris"),
    "mer. 16 sept., 23:40",
  );
  assert.equal(
    formatDateTime(instant, "en", "Europe/Paris"),
    "Wed, Sep 16, 23:40",
  );
  assert.equal(
    formatDateTime(instant, "ja", "Europe/Paris"),
    "9月16日(水) 23:40",
  );
  assert.equal(formatDay(instant, "fr", "Europe/Paris"), "mer. 16 sept.");
});

test("a date is read in the computer's time zone, and never as the ISO text of the service", () => {
  assert.equal(
    formatDateTime(instant, "fr", "Asia/Tokyo"),
    "jeu. 17 sept., 06:40",
  );
  assert.ok(!formatDateTime("2026-09-16T20:05:00Z", "fr").includes("2026-"));
  // An instant that cannot be read says so instead of « Invalid Date ».
  assert.equal(formatDateTime("pas une date", "fr"), "—");
  assert.equal(formatDateTime(null, "fr"), "—");
});

test("bytes carry one decimal in the decimal mark of the language: « 4,1 Go »", () => {
  const fr = createTranslator("fr");
  const en = createTranslator("en");
  assert.equal(formatBytes(fr, 4_100_000_000, "fr"), "4,1 Go");
  assert.equal(formatBytes(fr, 15_600_000_000, "fr"), "15,6 Go");
  assert.equal(formatBytes(en, 4_100_000_000, "en"), "4.1 GB");
  assert.equal(formatBytes(fr, 2_540_000, "fr"), "2,5 Mo");
  assert.equal(formatBytes(fr, 1_500, "fr"), "1,5 ko");
  assert.equal(formatBytes(fr, 512, "fr"), "512 octets");
});

test("a count picks the singular sentence where the language has one", () => {
  const fr = createTranslator("fr");
  const plain = (text: string) => text.replaceAll("\u00a0", " ");
  assert.equal(
    plain(pluralText(fr, "fr", "annotations.count", 4)),
    "4 captures sans solution de ciel",
  );
  assert.equal(
    plain(pluralText(fr, "fr", "annotations.count", 1)),
    "1 capture sans solution de ciel",
  );
  assert.equal(
    plain(pluralText(fr, "fr", "deps.missing", 2)),
    "2 outils nécessaires manquent.",
  );
  assert.equal(plain(pluralText(fr, "fr", "logs.steps", 1)), "1 palier");
});

test("a stacking that gave no image is said in words: by its reason, its script's code, or the kind of its exit code", async () => {
  const { stackingFailureText } = await import("./wizardSummary.ts");
  const { createTranslator } = await import("../i18n/locale.ts");
  const t = createTranslator("fr");
  const said = (details: Record<string, unknown>) =>
    stackingFailureText(t, details).replaceAll("\u00a0", " ");
  const equal = (actual: string, expected: string) =>
    assert.equal(actual, expected.replaceAll("\u00a0", " "));
  equal(
    said({ why: "failed", code: "too_few_lights", message: "x" }),
    "Il faut au moins deux poses FITS dans lights/.",
  );
  equal(
    said({ why: "failed", code: "bad_input", message: "x" }),
    "Les fichiers de la cible ne conviennent pas à l’empilement.",
  );
  equal(
    said({ why: "failed", code: "tool_missing", message: "x" }),
    "Un outil de l’empilement manque : voyez Réglages › Dépendances.",
  );
  equal(
    said({ why: "failed", code: "registration_failed", message: "x" }),
    "Siril a échoué pendant l’empilement.",
  );
  equal(
    said({
      why: "failed",
      code: "registration_failed",
      message: "x",
      logs: "/data/stacking-logs/M31_2026-10-07-20261012-140309",
    }),
    "Siril a échoué pendant l’empilement. Le journal est dans /data/stacking-logs/M31_2026-10-07-20261012-140309.",
  );
  equal(
    said({ why: "failed", code: "render_failed", message: "x", logs: "/d/l" }),
    "Un outil de l’empilement a échoué. Le journal est dans /d/l.",
  );
  equal(
    said({ why: "failed", code: "tool_failed", message: "x" }),
    "Un outil de l’empilement a échoué.",
  );
  equal(said({ why: "cancelled" }), "Empilement arrêté à votre demande.");
  equal(
    said({
      why: "not_enough_space",
      needed: 2_520_000_000,
      available: 1_000_000_000,
      folder: "2026-10-07 · Col des Étoiles · 7f3a2c",
    }),
    "Pas assez de place pour l’empilement dans 2026-10-07 · Col des Étoiles · 7f3a2c : 2,5 Go nécessaires, 1 Go libres. Libérez de la place sur ce disque ou déplacez la nuit.",
  );
  equal(
    said({ why: "failed", code: "who_knows" }),
    "L’empilement n’a pas pu être fait.",
  );
});

test("a failed read says its reason by the code, in the language of the window; the core's message only for a code with no sentence", () => {
  const fr = createTranslator("fr");
  const en = createTranslator("en");
  const known = {
    code: "night_folder_missing",
    message: "The night folder cannot be found.",
  };
  assert.equal(errorReason(fr, known), fr("errors.night_folder_missing"));
  assert.ok(!errorReason(fr, known).includes("cannot be found"));
  assert.notEqual(errorReason(fr, known), errorReason(en, known));
  // The sentence may carry the message.
  assert.equal(
    errorReason(en, { code: "invalid_night", message: "no date" }),
    "This night cannot be created as it is: no date",
  );
  // A code the dictionaries do not know: the core's message, not the key.
  assert.equal(
    errorReason(fr, { code: "a_code_from_the_future", message: "raw" }),
    "raw",
  );
  // Not a CommandError.
  assert.equal(errorReason(en, new Error("boom")), "boom");
  assert.equal(errorReason(en, "plain"), "plain");
});
