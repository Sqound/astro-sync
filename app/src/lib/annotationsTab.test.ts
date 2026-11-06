import assert from "node:assert/strict";
import test from "node:test";

import { createTranslator } from "../i18n/locale.ts";
import {
  annotationsState,
  captureLine,
  captureTitle,
  countText,
  failureText,
  rowsToSolve,
  runningText,
  solverPresent,
  sourceText,
  stateText,
} from "./annotationsTab.ts";
import { MISSING_CAPTURES, tools } from "./homeFixtures.ts";

const fr = createTranslator("fr");
const plain = (text: string) => text.replaceAll(" ", " ").replaceAll("'", "’");
const same = (actual: string, expected: string) =>
  assert.equal(plain(actual), plain(expected));

const [m31, ngc, ic, m31Old] = MISSING_CAPTURES;

test("Annotations: each row says what it is and where its image is, from the phone, the web or a night", () => {
  same(
    fr("annotations.help"),
    "Les captures déjà envoyées sans solution de ciel : depuis le téléphone, le web ou une nuit.",
  );
  same(countText(fr, "fr", 4), "4 captures sans solution de ciel");
  same(countText(fr, "fr", 1), "1 capture sans solution de ciel");
  same(captureTitle(m31), "Galaxie d’Andromède");
  same(captureLine(fr, "fr", m31), "M31 · Nuit du 5 oct. · M31_final.jpg");
  same(
    captureLine(fr, "fr", ngc),
    "NGC 7000 · Nuit du 3 oct. · NGC7000_v2.png",
  );
  same(
    sourceText(fr, m31, false),
    "Envoyée depuis le téléphone · l’original sera téléchargé de votre espace",
  );
  same(
    sourceText(fr, ngc, false),
    "Ajoutée depuis le web · l’original sera téléchargé de votre espace",
  );
  same(
    sourceText(fr, ic, true),
    "Dans le dossier de la nuit, sur cet ordinateur",
  );
  same(stateText(fr, "todo"), "À résoudre");
  same(fr("annotations.solveAll"), "Tout résoudre");
  same(fr("annotations.solve"), "Résoudre et envoyer");
  same(
    failureText(fr, { code: "too_few_stars" }),
    "Trop peu d’étoiles reconnues dans l’image.",
  );
  // WEB.17b.5: astrometry.net without index files says where to put them.
  same(
    failureText(fr, { code: "no_index" }),
    "astrometry.net n’a aucun fichier d’index\u00a0: installez-les dans Réglages › Dépendances.",
  );
  same(stateText(fr, "failed"), "Échec");
  assert.equal(
    annotationsState({
      captures: MISSING_CAPTURES,
      solverPresent: true,
      running: false,
    }),
    "list",
  );
});

test("Annotations: one capture at a time, the download of the original said before the resolution", () => {
  assert.equal(
    annotationsState({
      captures: MISSING_CAPTURES,
      solverPresent: true,
      running: true,
    }),
    "running",
  );
  same(
    runningText(fr, "fr", m31, false),
    "Téléchargement de l’original (14 Mo), puis résolution sur cet ordinateur",
  );
  same(runningText(fr, "fr", ic, true), "Résolution sur cet ordinateur");
  same(fr("annotations.running"), "Résolution en cours…");
  same(fr("annotations.stop"), "Arrêter la résolution");
  same(stateText(fr, "running"), "En cours");
  same(stateText(fr, "done"), "Résolue et envoyée");
  // « Tout résoudre » goes through what is left: never a solved row again.
  assert.deepEqual(
    rowsToSolve(MISSING_CAPTURES, {
      [m31.id]: { state: "done" },
      [ngc.id]: { state: "failed", failure: { code: "not_solved" } },
    }).map((capture) => capture.id),
    [ngc.id, ic.id, m31Old.id],
  );
});

test("Annotations: empty, one sentence and no button", () => {
  assert.equal(
    annotationsState({ captures: [], solverPresent: false, running: false }),
    "empty",
  );
  same(
    fr("annotations.empty"),
    "Toutes vos captures ont une solution de ciel.",
  );
});

test("Annotations: no solver, the notice towards Réglages › Dépendances, the actions greyed", () => {
  const withoutAstap = tools({ astap: true });
  assert.equal(solverPresent(withoutAstap), false);
  assert.equal(
    solverPresent(null),
    true,
    "an unknown state holds nothing back",
  );
  assert.equal(
    solverPresent([
      ...tools(),
      {
        ...tools()[1],
        id: "astap-database",
        name: { fr: "Base", en: "Database" },
      },
    ]),
    true,
  );
  assert.equal(
    annotationsState({
      captures: MISSING_CAPTURES,
      solverPresent: false,
      running: false,
    }),
    "noSolver",
  );
  same(
    fr("annotations.noSolver"),
    "Aucun solveur n’est installé : les solutions de ciel ne peuvent pas être calculées ici.",
  );
  same(
    failureText(fr, { code: "timeout", seconds: 120 }),
    "Pas de solution après 120 s : la résolution a été arrêtée.",
  );
});
