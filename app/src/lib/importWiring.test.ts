import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

// What the pure tests cannot see: that the windows are wired to them.

const source = (relative: string) =>
  readFileSync(new URL(`../${relative}`, import.meta.url), "utf8");

test("every place that reads a night reads it on the clock of its files when the import corrected the session's", () => {
  for (const file of [
    "screens/night/NightWizard.tsx",
    "screens/SessionsScreen.tsx",
    "screens/HomeScreen.tsx",
  ]) {
    const text = source(file);
    assert.match(text, /withImportClock\(/, file);
    assert.match(text, /loadImportClock\(/, file);
  }
  assert.match(
    source("components/ImportProvider.tsx"),
    /saveImportClocks\(windowStorage\(\), clocksOf\(proposal, outcome\)\)/,
    "the correction is kept when the sessions are created",
  );
  assert.match(source("screens/night/IncidentsPanel.tsx"), /shiftSeconds/);
});

test("every way to open a session from the result prepares its folder first", () => {
  const result = source("screens/detect/ImportSummary.tsx");
  assert.match(
    result,
    /sessionPreparation\(\s*proposal,\s*created\.outcome,\s*sessionId,?\s*\)/,
  );
  assert.match(result, /detectTreatmentsPrepare\(/);
  // The rows and « Mesurer les poses » go through it, never straight to the wizard.
  assert.doesNotMatch(result, /onClick=\{\(\) => onOpenNight\(/);
  assert.match(result, /openSession\(row\.sessionId, null\)/);
  assert.match(result, /openSession\(sessionId, step, targetId\)/);
});
