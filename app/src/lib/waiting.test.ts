import assert from "node:assert/strict";
import test from "node:test";

import { createTranslator } from "../i18n/locale.ts";
import { waitingKey, type WizardWait } from "./waiting.ts";

const fr = createTranslator("fr");
const start: WizardWait = {
  phase: null,
  viewReady: false,
  creatingFolder: false,
  toolsReady: false,
};

test("the sentence of a wait says what is awaited, per step", () => {
  const says = (wait: Partial<WizardWait>) =>
    fr(waitingKey({ ...start, ...wait }));
  // The opening, before any step is known, and « Cibles ».
  assert.equal(says({}), "Lecture de la session…");
  assert.equal(says({ phase: "cibles" }), "Lecture de la session…");
  // « Dossier »: the night first, then the folder, made or read.
  assert.equal(says({ phase: "dossier" }), "Lecture de la session…");
  assert.equal(
    says({ phase: "dossier", viewReady: true, creatingFolder: true }),
    "Création du dossier de la nuit…",
  );
  assert.equal(
    says({ phase: "dossier", viewReady: true }),
    "Lecture du dossier de la nuit…",
  );
  assert.equal(says({ phase: "fichiers" }), "Lecture du dossier de la nuit…");
  // « Traitements »: the tools are listed when the step opens.
  assert.equal(
    says({ phase: "traitements" }),
    "Recherche des outils installés…",
  );
  assert.equal(
    says({ phase: "traitements", toolsReady: true }),
    "Lecture de la session…",
  );
  assert.equal(
    says({ phase: "recapitulatif" }),
    "Vérification auprès du service…",
  );
});

test("every sentence of a wait exists in the ten dictionaries and ends with an ellipsis", async () => {
  const { readFile } = await import("node:fs/promises");
  const keys = new Set<string>();
  for (const phase of [
    null,
    "cibles",
    "dossier",
    "fichiers",
    "traitements",
    "recapitulatif",
    "sending",
    "sent",
  ] as const) {
    for (const viewReady of [false, true]) {
      for (const creatingFolder of [false, true]) {
        for (const toolsReady of [false, true]) {
          keys.add(
            waitingKey({ phase, viewReady, creatingFolder, toolsReady }),
          );
        }
      }
    }
  }
  for (const key of [
    "common.wait.nights",
    "common.wait.space",
    "common.wait.annotations",
    "common.wait.queue",
    "common.wait.dryRun",
  ]) {
    keys.add(key);
  }
  for (const locale of [
    "fr",
    "en",
    "ja",
    "de",
    "es",
    "it",
    "pt-BR",
    "ru",
    "zh-Hans",
    "zh-Hant",
  ]) {
    const dictionary = JSON.parse(
      await readFile(
        new URL(`../i18n/${locale}.json`, import.meta.url),
        "utf8",
      ),
    ) as Record<string, string>;
    for (const key of keys) {
      assert.ok(dictionary[key], `${locale} has no ${key}`);
      assert.match(
        dictionary[key],
        /(…|\.\.\.)$/u,
        `${locale} ${key} should end with an ellipsis`,
      );
    }
  }
});
