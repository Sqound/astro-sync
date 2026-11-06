import assert from "node:assert/strict";
import test from "node:test";

import { createTranslator } from "../i18n/locale.ts";
import type { ProgressionEvent, StackEvent, WorkProgress } from "./bridge";
import {
  SILENT_AFTER_MS,
  beginProgress,
  counterText,
  elapsedMs,
  finishProgress,
  isStackingCancelled,
  progressFrameView,
  stackingCancelledError,
  stepText,
  withProgressionEvent,
  withStackEvent,
  withWorkProgress,
} from "./treatmentProgress.ts";

const fr = createTranslator("fr");
const T0 = 1_000_000;

test("a step of the stacking is written in words with its place, and starts the percentage again", () => {
  let state = beginProgress(T0);
  assert.equal(stepText(fr, state.step), "préparation");
  state = withStackEvent(state, {
    event: "progress",
    percent: 28.18,
    line: "Integrating 90 images…",
    at: T0 + 5000,
  });
  assert.equal(state.percent, 28.18);
  state = withStackEvent(state, {
    event: "step",
    step: "integration",
    index: 6,
    total: 7,
  });
  assert.equal(stepText(fr, state.step), "intégration, étape 6 sur 7");
  assert.equal(state.percent, null, "a new command of Siril starts at zero");
  assert.equal(state.lastLine, "Integrating 90 images…", "the line stays");
});

test("a step of the script the window does not know is written as the script names it", () => {
  const state = withStackEvent(beginProgress(T0), {
    event: "step",
    step: "denoise",
    index: 3,
    total: 8,
  });
  assert.equal(stepText(fr, state.step), "denoise, étape 3 sur 8");
});

test("the other events of the stacking change nothing, and a percentage is held between 0 and 100", () => {
  const state = beginProgress(T0);
  const family: StackEvent = {
    event: "family",
    family: "Ha",
    index: 1,
    total: 2,
  };
  const output: StackEvent = { event: "output", pipeline: "raw", file: "x" };
  // The family and the output files are kept for the run's list of steps
  // but change nothing the frame drew before.
  for (const event of [family, output]) {
    const next = withStackEvent(state, event, T0 + 1);
    assert.deepEqual(next.step, state.step);
    assert.equal(next.percent, state.percent);
    assert.equal(next.counter, state.counter);
    assert.deepEqual(next.notes, state.notes);
    assert.equal(next.overall, null, "a run without units has no overall bar");
  }
  const over = withStackEvent(state, {
    event: "progress",
    percent: 140,
    line: null,
    at: T0,
  });
  assert.equal(over.percent, 100);
  assert.equal(over.lastLine, null, "no line, the last one is kept (none yet)");
});

test("the progression says its part, its milestone and what Siril prints", () => {
  let state = beginProgress(T0);
  const events: ProgressionEvent[] = [
    { event: "stage", stage: "registration" },
    { event: "progress", percent: 62.5, line: "Registering…", at: T0 + 1 },
  ];
  for (const event of events) state = withProgressionEvent(state, event);
  assert.equal(stepText(fr, state.step), "alignement des poses");
  assert.equal(state.percent, 62.5);
  assert.equal(state.counter, null);
  state = withProgressionEvent(state, { event: "stage", stage: "milestones" });
  state = withProgressionEvent(state, {
    event: "milestone",
    index: 2,
    total: 3,
  });
  assert.equal(stepText(fr, state.step), "empilement des jalons");
  assert.equal(counterText(fr, state.counter!), "jalon 2 sur 3");
  assert.equal(state.percent, null);
});

test("the report of the logs counts its files and shows the file it reads", () => {
  const progress = (over: Partial<WorkProgress>): WorkProgress => ({
    kind: "analysis",
    done: 1,
    total: 2,
    bytesDone: 0,
    bytesTotal: 0,
    current: "NINA_20261007_190512.log",
    ...over,
  });
  const state = withWorkProgress(beginProgress(T0), progress({}), T0 + 10);
  assert.equal(counterText(fr, state.counter!), "fichier 1 sur 2");
  assert.equal(state.lastLine, "NINA_20261007_190512.log");
  assert.equal(state.lastLineAt, T0 + 10);
  // The work of a scan or of the detection is not the report's.
  assert.equal(
    withWorkProgress(state, progress({ kind: "scan", done: 9 }), T0 + 20),
    state,
  );
  // A total of zero says nothing.
  assert.equal(withWorkProgress(state, progress({ total: 0 }), T0 + 30), state);
});

test("the time advances to the end of the treatment and then stops", () => {
  const state = beginProgress(T0);
  assert.equal(elapsedMs(state, T0 + 134_000), 134_000);
  const done = finishProgress(state, T0 + 362_000);
  assert.equal(elapsedMs(done, T0 + 999_000), 362_000);
});

test("the frame writes the step, the counter, the percentage, the time and the last line", () => {
  let state = beginProgress(T0);
  state = withStackEvent(state, {
    event: "step",
    step: "integration",
    index: 6,
    total: 7,
  });
  state = withStackEvent(state, {
    event: "progress",
    percent: 28.18,
    line: "Integrating 90 images…",
    at: T0 + 130_000,
  });
  const view = progressFrameView(fr, {
    label: "Empilement de M31",
    state,
    now: T0 + 134_000,
  });
  assert.equal(view.label, "Empilement de M31");
  assert.equal(view.headRest, " · intégration, étape 6 sur 7");
  assert.equal(view.percent, 28.2);
  assert.equal(view.percentText, "28 %");
  assert.equal(view.counter, null, "no counter when the treatment has none");
  assert.equal(view.elapsed, "en cours depuis 2 min 14 s");
  assert.equal(view.lastLine, "Integrating 90 images…");
  assert.equal(view.silent, null);
  assert.equal(view.ariaLabel, "Empilement de M31, en cours");
});

test("without a percentage the bar is not determined", () => {
  const view = progressFrameView(fr, {
    label: "Le rapport des journaux",
    state: beginProgress(T0),
    now: T0 + 12_000,
  });
  assert.equal(view.percent, null);
  assert.equal(view.percentText, null);
  assert.equal(view.elapsed, "en cours depuis 12 s");
});

test("a journal that says nothing for a minute is said to be silent, with the time since its last line", () => {
  const state = withStackEvent(beginProgress(T0), {
    event: "progress",
    percent: null,
    line: "Integrating 90 images…",
    at: T0 + 10_000,
  });
  const quiet = (now: number) =>
    progressFrameView(fr, { label: "Empilement de M31", state, now }).silent;
  assert.equal(quiet(T0 + 10_000 + SILENT_AFTER_MS - 1), null);
  assert.equal(
    quiet(T0 + 10_000 + SILENT_AFTER_MS),
    "dernière ligne il y a 1 min 00 s",
  );
  assert.equal(quiet(T0 + 110_000), "dernière ligne il y a 1 min 40 s");
  // Refreshed with the clock, not with an event.
  assert.equal(quiet(T0 + 111_000), "dernière ligne il y a 1 min 41 s");
});

test("a journal that has said nothing yet is not called silent", () => {
  const view = progressFrameView(fr, {
    label: "Empilement de M31",
    state: beginProgress(T0),
    now: T0 + 600_000,
  });
  assert.equal(view.silent, null);
  assert.equal(view.lastLine, null);
});

test("the stop of the stacking is told from any other failure", () => {
  assert.equal(isStackingCancelled(stackingCancelledError()), true);
  assert.equal(
    isStackingCancelled({
      code: "stacking_failed",
      message: "x",
      details: { why: "failed", code: "siril_failed" },
    }),
    false,
  );
  assert.equal(isStackingCancelled(new Error("cancelled")), false);
  assert.equal(isStackingCancelled(null), false);
});

test("the registration of a night whose field turned says so, and names the lights left out", () => {
  let state = beginProgress(T0);
  state = withStackEvent(state, {
    event: "registration",
    unit: "lights",
    frames: 430,
    registered: 429,
    unregistered: ["Light_M 45_30.0s_IRCUT_20261011-045331.fit"],
    fieldRotationDegrees: 125.29,
    framing: "full",
  });
  assert.deepEqual(state.notes, [
    { kind: "fullField", degrees: 125, unit: "lights" },
    { kind: "unregistered", count: 1, total: 430, unit: "lights" },
  ]);
  const view = progressFrameView(fr, {
    label: "Empilement de M 45",
    state,
    now: T0 + 1000,
  });
  assert.deepEqual(view.notes, [
    "Le champ a tourné de 125° pendant la nuit : l’empilement garde le champ entier, ses bords sont couverts par moins de poses.",
    "Poses non alignées, laissées de côté : 1 sur 430.",
  ]);
});

test("the registration of a plan's nights says the same of its nights, in its own words", () => {
  let state = beginProgress(T0);
  state = withStackEvent(state, {
    event: "registration",
    unit: "masters",
    frames: 3,
    registered: 2,
    unregistered: ["2026-10-12 · n-2"],
    fieldRotationDegrees: 130,
    framing: "full",
  });
  assert.deepEqual(state.notes, [
    { kind: "fullField", degrees: 130, unit: "masters" },
    { kind: "unregistered", count: 1, total: 3, unit: "masters" },
  ]);
  const view = progressFrameView(fr, {
    label: "Image du plan",
    state,
    now: T0 + 1000,
  });
  assert.deepEqual(view.notes, [
    "Le champ a tourné de 130° sur l’ensemble des nuits du plan : l’image garde le champ entier de la nuit de référence, ses bords sont couverts par moins de nuits.",
    "Intégrations non alignées, laissées de côté : 1 sur 3.",
  ]);
});

test("the corners of a plan that no dual-band night reaches are said after the registration, with their share", () => {
  let state = beginProgress(T0);
  state = withStackEvent(state, {
    event: "registration",
    unit: "masters",
    frames: 2,
    registered: 2,
    unregistered: [],
    fieldRotationDegrees: 96,
    framing: "full",
  });
  state = withStackEvent(state, { event: "coverage", noDualBandShare: 0.1875 });
  assert.deepEqual(state.notes, [
    { kind: "fullField", degrees: 96, unit: "masters" },
    { kind: "noDualBand", percent: 19 },
  ]);
  const view = progressFrameView(fr, {
    label: "Image du plan",
    state,
    now: T0 + 1000,
  });
  assert.equal(
    view.notes[1],
    "Les coins n’ont pas l’Hα / OIII\u00a0: 19\u00a0% de l’image n’est couvert par aucune nuit bibande, la couleur y vient des nuits large bande seules.",
  );
  // Said once, whatever the number of times the event comes; under one percent it is not worth a note.
  const again = withStackEvent(state, {
    event: "coverage",
    noDualBandShare: 0.3,
  });
  assert.deepEqual(
    again.notes.filter((n) => n.kind === "noDualBand"),
    [{ kind: "noDualBand", percent: 30 }],
  );
  const tiny = withStackEvent(beginProgress(T0), {
    event: "coverage",
    noDualBandShare: 0.004,
  });
  assert.deepEqual(tiny.notes, []);
});

test("the image of a plan cropped to the area every night covers says so in place of the whole field", () => {
  const registration = {
    event: "registration" as const,
    unit: "masters" as const,
    frames: 3,
    registered: 3,
    unregistered: [],
    fieldRotationDegrees: 39,
    framing: "full" as const,
  };
  const crop = {
    event: "crop" as const,
    mode: "common" as const,
    applied: true,
    reason: "commonArea",
    top: 444,
    left: 419,
    height: 2095,
    width: 2137,
    frameHeight: 2977,
    frameWidth: 2995,
    keptShare: 0.5021,
  };
  let state = withStackEvent(beginProgress(T0), registration);
  state = withStackEvent(state, crop);
  assert.deepEqual(state.notes, [{ kind: "planCropped", percent: 50 }]);
  const view = progressFrameView(fr, {
    label: "Image du plan",
    state,
    now: T0 + 1000,
  });
  assert.deepEqual(view.notes, [
    "L’image est recadrée sur la zone que toutes les nuits couvrent\u00a0: 50\u00a0% du champ gardé.",
  ]);
  // Said once.
  assert.equal(withStackEvent(state, crop).notes.length, 1);
  // « Bords nets »: only the edges without data go; the whole-field note stays.
  const clean = withStackEvent(
    withStackEvent(beginProgress(T0), registration),
    {
      ...crop,
      mode: "clean",
      reason: "noEmptyPixel",
      keptShare: 0.9708,
    },
  );
  assert.deepEqual(clean.notes, [
    { kind: "fullField", degrees: 39, unit: "masters" },
    { kind: "planCleanEdges", percent: 97 },
  ]);
  assert.equal(
    progressFrameView(fr, {
      label: "Image du plan",
      state: clean,
      now: T0 + 1000,
    }).notes[1],
    "Bords sans donnée retirés\u00a0: 97\u00a0% du champ gardé.",
  );
  // Nothing cut (« Tout le champ »): nothing said and the whole field stays.
  const whole = withStackEvent(
    withStackEvent(beginProgress(T0), registration),
    {
      ...crop,
      mode: "whole",
      applied: false,
      reason: "wholeField",
      keptShare: 1,
    },
  );
  assert.deepEqual(whole.notes, [
    { kind: "fullField", degrees: 39, unit: "masters" },
  ]);
});

test("a field that hardly turned and lights all aligned give no note, and a new family forgets the last notes", () => {
  const quiet = withStackEvent(beginProgress(T0), {
    event: "registration",
    unit: "lights",
    frames: 90,
    registered: 90,
    unregistered: [],
    fieldRotationDegrees: 0.4,
    framing: "common",
  });
  assert.deepEqual(quiet.notes, []);
  const unmeasured = withStackEvent(beginProgress(T0), {
    event: "registration",
    unit: "lights",
    frames: 90,
    registered: 90,
    unregistered: [],
    fieldRotationDegrees: null,
    framing: "common",
  });
  assert.deepEqual(unmeasured.notes, []);
  const turned = withStackEvent(beginProgress(T0), {
    event: "registration",
    unit: "lights",
    frames: 30,
    registered: 30,
    unregistered: [],
    fieldRotationDegrees: 40,
    framing: "full",
  });
  assert.equal(turned.notes.length, 1);
  const next = withStackEvent(turned, {
    event: "family",
    family: "Ha",
    index: 2,
    total: 2,
  });
  assert.deepEqual(next.notes, []);
});
