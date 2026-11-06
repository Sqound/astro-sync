import assert from "node:assert/strict";
import test from "node:test";

import { createTranslator } from "../i18n/locale.ts";
import {
  canGenerate,
  failureText,
  milestones,
  progressionState,
  sirilLine,
  sirilText,
} from "./progressionTab.ts";
import { isReserved } from "./tabs.ts";
import {
  CAPABILITIES_ADVANCED,
  CAPABILITIES_PLUS,
  tools,
} from "./homeFixtures.ts";

const fr = createTranslator("fr");
const plain = (text: string) => text.replaceAll(" ", " ").replaceAll("'", "’");
const same = (actual: string, expected: string) =>
  assert.equal(plain(actual), plain(expected));

test("Progression: the head message, Siril in one line, « Générer la progression » the one primary", () => {
  const siril = sirilLine(tools());
  assert.deepEqual(siril, { state: "compatible", version: "1.4.0" });
  same(sirilText(fr, siril!), "Siril 1.4.0, compatible");
  const state = progressionState({ reserved: false, siril, generated: false });
  assert.equal(state, "choose");
  assert.equal(canGenerate({ state, family: "Ha_180s", running: false }), true);
  assert.equal(canGenerate({ state, family: null, running: false }), false);
  assert.equal(canGenerate({ state, family: "Ha_180s", running: true }), false);
  same(
    fr("progression.headTitle"),
    "Générez la progression de l’empilement au cours de votre nuit",
  );
  same(
    fr("progression.headLine"),
    "Astro Sync empile vos poses par jalons, d’une seule pose à toute la famille, pour que vous voyiez la nuit avancer.",
  );
  same(
    fr("progression.headNote"),
    "Une aide pratique, pas un outil de traitement.",
  );
  same(
    fr("progression.readFrom", { path: "NGC 7000/lights/" }),
    "Lue dans NGC 7000/lights/ de la nuit choisie.",
  );
  same(
    fr("progression.output"),
    "Les images vont dans le dossier progression/ de la cible, à côté de stacked/. Le travail se fait sur le disque de la nuit et n’y laisse rien d’autre.",
  );
  // Today's paragraph, word for word, behind the ⓘ.
  assert.match(
    fr("progression.description"),
    /^Une progression automatique optionnelle et indicative/,
  );
});

test("Progression: the milestones in order, « Envoyer ces images de progression »", () => {
  const made = {
    familyKey: "Ha_180s_g100_1x1_t-10C",
    images: [
      { index: 3, frameCount: 20, path: "c.jpg", sha256: "c", size: 1 },
      { index: 1, frameCount: 1, path: "a.jpg", sha256: "a", size: 1 },
      { index: 2, frameCount: 10, path: "b.jpg", sha256: "b", size: 1 },
    ],
  };
  const list = milestones(made);
  assert.deepEqual(
    list.map((m) =>
      fr("progression.milestoneLabel", { index: m.index, count: m.frameCount }),
    ),
    ["Jalon 1 — 1 pose(s)", "Jalon 2 — 10 pose(s)", "Jalon 3 — 20 pose(s)"],
  );
  assert.equal(
    progressionState({ reserved: false, siril: null, generated: true }),
    "generated",
  );
  same(fr("progression.send"), "Envoyer ces images de progression");
  assert.deepEqual(milestones(null), []);
});

test("Progression: Siril missing is a notice, generating is greyed", () => {
  const siril = sirilLine(tools().filter((row) => row.id !== "siril"));
  assert.deepEqual(siril, { state: "missing" });
  const state = progressionState({ reserved: false, siril, generated: false });
  assert.equal(state, "sirilMissing");
  assert.equal(canGenerate({ state, family: "Ha", running: false }), false);
  same(
    fr("progression.sirilMissing"),
    "Siril est introuvable : la progression ne peut pas être générée ici.",
  );
  same(fr("deps.open"), "Ouvrir Réglages › Dépendances");
  same(
    failureText(fr, { why: "siril_missing" }),
    fr("progression.sirilMissing"),
  );
});

test("Progression: reserved without analysis.progression (the tab stays, the form greyed), absent with it", () => {
  const advanced = { paired: true, capabilities: CAPABILITIES_ADVANCED };
  const plus = { paired: true, capabilities: CAPABILITIES_PLUS };
  assert.equal(isReserved(advanced, "analysis.progression"), true);
  assert.equal(isReserved(plus, "analysis.progression"), false);
  const state = progressionState({
    reserved: true,
    siril: sirilLine(tools()),
    generated: false,
  });
  assert.equal(state, "reserved");
  assert.equal(canGenerate({ state, family: "Ha", running: false }), false);
  same(
    fr("progression.reserved"),
    "La progression est réservée à Synchronisation avancée+.",
  );
  same(failureText(fr, { why: "failed", detail: "exit 1" }), "Motif : exit 1");
});
