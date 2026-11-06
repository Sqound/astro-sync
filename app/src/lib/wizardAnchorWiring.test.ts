import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

// What the pure tests cannot see: that the windows are wired to them. The
// captures (`detection-anchor-*`, `detection-result-*`) show the same in a
// real window.

const source = (relative: string) =>
  readFileSync(new URL(`../${relative}`, import.meta.url), "utf8");

test("every way to « Mesurer les poses » names the target it comes from", () => {
  const treatments = source("screens/night/TreatmentsStep.tsx");
  assert.match(treatments, /onMeasure=\{\(\) => onMeasure\(card\.targetId\)\}/);
  const wizard = source("screens/night/NightWizard.tsx");
  assert.match(
    wizard,
    /onMeasure=\{\(targetId\) => goTo\("fichiers", targetId\)\}/g,
  );
  assert.equal(
    wizard.match(/goTo\("fichiers", targetId\)/g)?.length,
    2,
    "« Traitements » and « Récapitulatif »",
  );
  assert.match(
    source("screens/night/SummaryStep.tsx"),
    /onMeasure=\{onMeasure\}/,
  );
});

test("« Fichiers » gives each target's card an id, takes the anchor, and holds the card", () => {
  const files = source("screens/night/FilesStep.tsx");
  assert.match(files, /id=\{targetCardId\(card\.targetId\)\}/);
  assert.match(files, /tabIndex=\{-1\}/, "a card can hold the focus");
  assert.match(files, /holdTargetCard\(card\)/);
  assert.match(source("screens/night/NightWizard.tsx"), /anchor=\{anchor\}/);
});

test("the summary of « Détecter les sessions » shows the list of incidents under the sessions created", () => {
  const result = source("screens/detect/ImportSummary.tsx");
  const sessions = result.indexOf('className="as-result__list"');
  const incidents = result.indexOf("<ResultIncidents");
  const files = result.indexOf('t("detect.files.title")');
  assert.ok(sessions > 0 && incidents > sessions && files > incidents);
  // The shortcut « Choisir les traitements… » is gone: « Traitements » is a
  // step of the thread.
  assert.ok(!result.includes("detect.treat.choose"));
  // « Mesurer les poses » of an imported session opens its wizard at « Fichiers ».
  assert.match(
    source("screens/detect/ResultIncidents.tsx"),
    /onOpenNight\(id, "fichiers", targetId\)/,
  );
});
