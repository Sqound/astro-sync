import assert from "node:assert/strict";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import test, { before } from "node:test";
import { fileURLToPath } from "node:url";

import { rolldown } from "rolldown";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";

import { createTranslator } from "../i18n/locale.ts";
import type { ProgressionEvent, StackEvent } from "./bridge";
import {
  MAX_LOG_LINES,
  beginProgress,
  finishProgress,
  nightUnitLabel,
  overallShare,
  progressFrameView,
  startUnit,
  withProgressionEvent,
  withStackEvent,
  withUnits,
  type TreatmentProgress,
} from "./treatmentProgress.ts";

// One bar for the whole run. Siril reports the percentage of its CURRENT
// COMMAND, which the script starts again at every step; a bar drawn from it
// restarted all along a night. These tests replay the events of a three-night
// run, in the order the core emits them, and hold the bar of the run
// monotonic.

const fr = createTranslator("fr");
const T0 = 1_000_000;

/** One entry of a recorded run: an event of the stacking, or the start of a
 * unit (what the window does between two `stacking_run` calls). */
type Recorded =
  | { at: number; unit: { index: number; label: string } }
  | { at: number; event: StackEvent };

/** The events of one stacking of a night, as `stacking.rs` emits them: a
 * `family` per job, then for each step of the script a `step`, a few
 * `progress` events whose percentage runs 0 to 100 inside the step (and, in
 * the calibration, twice: two Siril commands), the registration's answer
 * and the output files. Returns the entries and the time it ends. */
function nightEvents(
  start: number,
  families: number,
  steps: string[],
): { entries: Recorded[]; end: number } {
  const entries: Recorded[] = [];
  let at = start;
  for (let family = 1; family <= families; family += 1) {
    at += 500;
    entries.push({
      at,
      event: {
        event: "family",
        family: `F${family}`,
        index: family,
        total: families,
      },
    });
    steps.forEach((step, position) => {
      at += 300;
      entries.push({
        at,
        event: {
          event: "step",
          step,
          index: position + 1,
          total: steps.length,
        },
      });
      // Two commands in the calibration: the percentage runs twice.
      const sweeps = step === "calibration" ? 2 : 1;
      for (let sweep = 0; sweep < sweeps; sweep += 1) {
        for (const percent of [0, 24.5, 51, 77.2, 100]) {
          at += 2_000;
          entries.push({
            at,
            event: {
              event: "progress",
              percent,
              line: `${step}: ${percent.toFixed(0)}%`,
              at,
            },
          });
        }
      }
      if (step === "registration") {
        entries.push({
          at,
          event: {
            event: "registration",
            unit: "lights",
            frames: 100,
            registered: 98,
            unregistered: ["a.fit", "b.fit"],
            fieldRotationDegrees: 3,
            framing: "common",
          },
        });
      }
      if (step === "render") {
        entries.push({
          at,
          event: {
            event: "output",
            pipeline: "raw",
            file: `render-F${family}.fit`,
          },
        });
      }
    });
  }
  return { entries, end: at };
}

const SCRIPT = [
  "strip_wcs",
  "master_darks",
  "master_flats",
  "calibration",
  "registration",
  "integration",
  "render",
];

/** Three nights of 100, 150 and 80 lights, two families each. */
function threeNights(): {
  entries: Recorded[];
  weights: number[];
  end: number;
} {
  const weights = [100, 150, 80];
  const entries: Recorded[] = [];
  let at = T0;
  weights.forEach((_, index) => {
    entries.push({
      at,
      unit: { index, label: `Nuit ${index + 1}` },
    });
    const night = nightEvents(at, 2, SCRIPT);
    entries.push(...night.entries);
    at = night.end + 1_000;
  });
  return { entries, weights, end: at };
}

function replay(
  entries: readonly Recorded[],
  weights: readonly number[],
  onState?: (state: TreatmentProgress) => void,
): TreatmentProgress {
  let state = beginProgress(T0, undefined, weights);
  for (const entry of entries) {
    state =
      "unit" in entry
        ? startUnit(state, entry.unit.index, entry.unit.label, entry.at)
        : withStackEvent(state, entry.event, entry.at);
    onState?.(state);
  }
  return state;
}

test("the bar of a three-night run never goes back, whatever Siril's percentage does", () => {
  const { entries, weights, end } = threeNights();
  const seen: number[] = [];
  const raw: Array<number | null> = [];
  const final = replay(entries, weights, (state) => {
    if (state.overall !== null) seen.push(state.overall);
    raw.push(state.percent);
  });
  // The recorded run does what happened on a real run: the command's percentage falls back
  // to nothing at each step, so a bar drawn from it restarted again and again.
  let rawRestarts = 0;
  for (let index = 1; index < raw.length; index += 1) {
    const before = raw[index - 1];
    const after = raw[index];
    if (before !== null && (after === null || after < before)) rawRestarts += 1;
  }
  assert.ok(rawRestarts >= 3 * 2 * SCRIPT.length, `${rawRestarts} restarts`);
  // The bar of the run: monotonic, always a determined value after the first
  // signal, and never full before the run says it is finished.
  assert.ok(seen.length > 100);
  for (let index = 1; index < seen.length; index += 1) {
    assert.ok(
      seen[index] >= seen[index - 1],
      `the bar went back from ${seen[index - 1]} to ${seen[index]} at event ${index}`,
    );
  }
  assert.ok(seen.every((value) => value >= 0 && value < 100));
  assert.ok(seen[0] < 5, "it starts near zero");
  assert.ok(seen[seen.length - 1] > 90, "it ends near the top");
  // A step change never sends it back, in particular: the value right after
  // each `step` event is at least the one right before.
  const finished = finishProgress(final, end);
  assert.equal(finished.overall, 100);
  assert.equal(finished.finishedAt, end);
});

test("the bar follows the frames: a night of 150 lights moves it more than one of 80", () => {
  const { entries, weights } = threeNights();
  const atUnitStart: number[] = [];
  replay(entries, weights, (state) => {
    // The first state of each unit is the one `startUnit` returns.
    if (
      state.step.kind === "preparing" &&
      state.within === 0 &&
      state.family === null
    ) {
      atUnitStart.push(state.overall ?? -1);
    }
  });
  // 0, 100/330, 250/330 of the run.
  assert.deepEqual(
    atUnitStart.map((value) => Math.round(value)),
    [0, 30, 76],
  );
});

test("a step starts its percentage again, the bar of the run does not", () => {
  let state = beginProgress(T0, undefined, [1]);
  state = withStackEvent(
    state,
    { event: "step", step: "calibration", index: 4, total: 7 },
    T0,
  );
  state = withStackEvent(
    state,
    { event: "progress", percent: 90, line: null, at: T0 + 1 },
    T0 + 1,
  );
  const high = state.overall!;
  state = withStackEvent(
    state,
    { event: "step", step: "registration", index: 5, total: 7 },
    T0 + 2,
  );
  assert.equal(state.percent, null);
  assert.ok(state.overall! >= high);
  const view = progressFrameView(fr, { label: "M31", state, now: T0 + 3 });
  assert.equal(view.percent, state.overall);
  assert.equal(
    view.stepPercent,
    null,
    "no command percentage yet in this step",
  );
  state = withStackEvent(
    state,
    { event: "progress", percent: 5, line: null, at: T0 + 4 },
    T0 + 4,
  );
  const again = progressFrameView(fr, { label: "M31", state, now: T0 + 5 });
  assert.equal(again.stepPercent, 5, "the thin bar is the step's own");
  assert.ok(again.percent! >= view.percent!);
});

test("the progression images of three targets share one bar too", () => {
  const events: Array<{ at: number; unit?: number; event?: ProgressionEvent }> =
    [];
  let at = T0;
  for (let target = 0; target < 3; target += 1) {
    events.push({ at, unit: target });
    events.push({ at, event: { event: "stage", stage: "registration" } });
    for (const percent of [0, 40, 100]) {
      at += 1000;
      events.push({
        at,
        event: { event: "progress", percent, line: `reg ${percent}`, at },
      });
    }
    events.push({ at, event: { event: "stage", stage: "milestones" } });
    for (let milestone = 1; milestone <= 3; milestone += 1) {
      events.push({
        at,
        event: { event: "milestone", index: milestone, total: 3 },
      });
      for (const percent of [0, 60, 100]) {
        at += 1000;
        events.push({
          at,
          event: { event: "progress", percent, line: null, at },
        });
      }
    }
  }
  let state = beginProgress(T0, undefined, [30, 30, 60]);
  const seen: number[] = [];
  for (const item of events) {
    state =
      item.unit !== undefined
        ? startUnit(state, item.unit, `Cible ${item.unit + 1}`, item.at)
        : withProgressionEvent(state, item.event!, item.at);
    if (state.overall !== null) seen.push(state.overall);
  }
  for (let index = 1; index < seen.length; index += 1) {
    assert.ok(seen[index] >= seen[index - 1], `went back at ${index}`);
  }
  assert.ok(seen[seen.length - 1] > 90);
});

test("the steps done are listed with their unit, result and duration, and the engine's lines are kept", () => {
  const { entries, weights } = threeNights();
  const state = replay(entries, weights);
  // Every step of every family of every night but the last (still open).
  assert.equal(state.done.length, 3 * 2 * SCRIPT.length - 1);
  const first = state.done[0];
  assert.equal(
    first.unit,
    "Nuit 1 (F1)",
    "the family tells two steps of one name apart",
  );
  assert.deepEqual(first.step, {
    kind: "stack",
    name: "strip_wcs",
    index: 1,
    total: 7,
  });
  assert.equal(
    first.durationMs,
    5 * 2000 + 300,
    "from its step event to the next step",
  );
  const registration = state.done.find(
    (record) =>
      record.step.kind === "stack" && record.step.name === "registration",
  )!;
  assert.deepEqual(registration.result, {
    kind: "registered",
    registered: 98,
    frames: 100,
    unit: "lights",
  });
  const render = state.done.find(
    (record) => record.step.kind === "stack" && record.step.name === "render",
  )!;
  assert.deepEqual(render.result, { kind: "file", file: "render-F1.fit" });
  // Durations are measured between the events of the run.
  assert.ok(state.done.every((record) => record.durationMs > 0));
  // The log keeps the lines in order and the unit they came from.
  assert.equal(state.log[0].line, "strip_wcs: 0%");
  assert.equal(state.log[0].unit, "Nuit 1 (F1)");
  assert.equal(state.log[state.log.length - 1].unit, "Nuit 3 (F2)");
  const view = progressFrameView(fr, {
    label: "M31",
    state,
    now: T0 + 999_999,
  });
  assert.equal(
    view.done[0].startsWith("Nuit 1 (F1) · préparation des poses · "),
    true,
  );
  assert.ok(
    view.done.some((row) =>
      row.includes("alignement · poses alignées : 98 sur 100 · "),
    ),
  );
  assert.ok(view.done.some((row) => row.includes("rendu · render-F1.fit · ")));
  assert.deepEqual(
    view.log.filter((row) => row.kind === "heading").map((row) => row.text),
    [
      "Nuit 1 (F1)",
      "Nuit 1 (F2)",
      "Nuit 2 (F1)",
      "Nuit 2 (F2)",
      "Nuit 3 (F1)",
      "Nuit 3 (F2)",
    ],
  );
});

test("the live line says the unit, the step, the item and the time of the step", () => {
  let state = beginProgress(T0, undefined, [100, 100]);
  state = startUnit(state, 0, nightUnitLabel(fr, "fr", "2026-08-09"), T0);
  state = withStackEvent(
    state,
    { event: "step", step: "registration", index: 5, total: 7 },
    T0 + 1_000,
  );
  const view = progressFrameView(fr, {
    label: "SH2-190",
    state,
    now: T0 + 13_000,
  });
  assert.equal(view.live, "Nuit du 9 août · alignement · 5 / 7 · 12 s");
  // The run's own time stays where it was.
  assert.equal(view.elapsed, "en cours depuis 13 s");
  // A run that does not know its units keeps its counter and no live line.
  const plain = progressFrameView(fr, {
    label: "x",
    state: beginProgress(T0),
    now: T0,
  });
  assert.equal(plain.live, null);
});

test("the log keeps the last lines only, and the same line twice once", () => {
  let state = beginProgress(T0, undefined, [1]);
  state = withStackEvent(
    state,
    { event: "progress", percent: 1, line: "a", at: T0 },
    T0,
  );
  state = withStackEvent(
    state,
    { event: "progress", percent: 2, line: "a", at: T0 + 1 },
    T0 + 1,
  );
  assert.equal(state.log.length, 1);
  for (let index = 0; index < MAX_LOG_LINES + 20; index += 1) {
    state = withStackEvent(
      state,
      {
        event: "progress",
        percent: null,
        line: `line ${index}`,
        at: T0 + 10 + index,
      },
      T0 + 10 + index,
    );
  }
  assert.equal(state.log.length, MAX_LOG_LINES);
  assert.equal(
    state.log[state.log.length - 1].line,
    `line ${MAX_LOG_LINES + 19}`,
  );
});

test("the share of the run is weighted by the units, never full before the end, and learns its units late", () => {
  assert.equal(overallShare([], 0, 0.5), null);
  assert.equal(overallShare([1, 1], 1, 0.5), 75);
  assert.equal(overallShare([10, 30], 0, 1), 25);
  assert.equal(overallShare([1], 0, 1), 99.9);
  assert.equal(
    overallShare([0, 0], 1, 0),
    50,
    "a unit without weight counts as one",
  );
  // The nights of a target are known once prepared: the bar keeps what it had.
  let state = beginProgress(T0);
  state = withStackEvent(
    state,
    { event: "step", step: "x", index: 1, total: 2 },
    T0,
  );
  assert.equal(state.overall, null);
  state = withUnits(state, [1, 1]);
  state = startUnit(state, 0, "Nuit 1", T0 + 1);
  assert.equal(state.overall, 0);
});

test("an event of the stacking that arrives out of order cannot send the bar back", () => {
  let state = beginProgress(T0, undefined, [1]);
  state = withStackEvent(
    state,
    { event: "family", family: null, index: 2, total: 2 },
    T0,
  );
  state = withStackEvent(
    state,
    { event: "step", step: "integration", index: 6, total: 7 },
    T0,
  );
  const held = state.overall!;
  state = withStackEvent(
    state,
    { event: "family", family: null, index: 1, total: 2 },
    T0 + 1,
  );
  state = withStackEvent(
    state,
    { event: "step", step: "strip_wcs", index: 1, total: 7 },
    T0 + 2,
  );
  assert.equal(state.overall, held);
});

/* ------------------------------ The frame drawn ----------------------------- */

type Frame = typeof import("../components/ProgressFrame.tsx").ProgressFrame;
let ProgressFrame: Frame;

before(async () => {
  const stubCss = {
    name: "stub-css",
    load(id: string) {
      return id.endsWith(".css")
        ? { code: "export default '';", moduleType: "js" as const }
        : null;
    },
  };
  const bundle = await rolldown({
    input: fileURLToPath(
      new URL("../components/ProgressFrame.tsx", import.meta.url),
    ),
    platform: "node",
    plugins: [stubCss],
    external: ["react", "react/jsx-runtime", "react-dom"],
    logLevel: "silent",
  });
  const { output } = await bundle.generate({ format: "esm" });
  const folder = new URL(
    "../../node_modules/.cache/astro-sync-tests/",
    import.meta.url,
  );
  await mkdir(folder, { recursive: true });
  const file = new URL("run-progress-frame.mjs", folder);
  await writeFile(file, output[0].code);
  ProgressFrame = (await import(file.href)).ProgressFrame;
});

test("the frame draws the live line, the steps done, the thin bar and the collapsed details", () => {
  const { entries, weights } = threeNights();
  const state = replay(entries.slice(0, 120), weights);
  const html = renderToStaticMarkup(
    createElement(ProgressFrame, {
      view: progressFrameView(fr, {
        label: "SH2-190",
        state,
        now: T0 + 400_000,
      }),
      location: fr("wizard.sending.local"),
      lastLineLabel: fr("wizard.sending.lastLineLabel"),
    }),
  );
  // Two bars: the run's, and the thin one of the step.
  assert.equal((html.match(/role="progressbar"/g) ?? []).length, 2);
  assert.match(html, /aria-label="Progression de l’étape en cours"/);
  assert.match(html, /class="as-frame__live"/);
  assert.match(html, /Nuit 1 \(F1\) · /);
  // The list of the steps done, under the bar.
  assert.match(
    html,
    /<ul class="as-frame__done" aria-label="Étapes déjà faites">/,
  );
  assert.match(html, /préparation des poses/);
  assert.match(html, /master des darks/);
  // The log, collapsed, under « Détails ».
  assert.match(
    html,
    /<details class="as-frame__details"><summary>Détails<\/summary>/,
  );
  assert.match(html, /<p class="as-frame__log-heading">Nuit 1 \(F1\)<\/p>/);
  assert.match(html, /<code class="as-frame__log-line">strip_wcs: 25%<\/code>/);
  assert.ok(!/<details[^>]* open/.test(html), "collapsed until opened");
});

test("an untracked frame draws no details and no list: the frames of before are unchanged", () => {
  const html = renderToStaticMarkup(
    createElement(ProgressFrame, {
      view: progressFrameView(fr, {
        label: "Le rapport des journaux",
        state: beginProgress(T0),
        now: T0 + 1000,
      }),
      location: fr("wizard.sending.local"),
      lastLineLabel: fr("wizard.sending.lastLineLabel"),
    }),
  );
  assert.ok(!html.includes("as-frame__details"));
  assert.ok(!html.includes("as-frame__done"));
  assert.ok(!html.includes("as-frame__step-bar"));
});

/* --------------------------------- The wiring -------------------------------- */

const source = (path: string) =>
  readFile(new URL(path, import.meta.url), "utf8");

test("the treatments of the sessions created feed one frame from the stacking AND the plan image, and start a unit per night", async () => {
  // The run belongs to the import; its page draws the frame.
  const flow = await source("../components/ImportProvider.tsx");
  const page = await source("../screens/detect/ImportTreatments.tsx");
  assert.match(page, /<ProgressFrame/);
  assert.match(flow, /for \(const listen of \[onStacking, onPlanImage\]\)/);
  assert.match(flow, /withStackEvent\(frame, event\)/);
  assert.match(flow, /startUnit\(frame, at, label, Date\.now\(\)\)/);
  assert.match(flow, /withUnits\(frame, weights\)/);
  assert.ok(!page.includes("progressClassName"));
  // The bar is no longer the percentage of the last Siril command.
  assert.ok(!flow.includes("running.percent"));
  assert.ok(!flow.includes("progressClassName"));
});

test("the night wizard's stacking and progression images start a unit per target", async () => {
  const summary = await source("../screens/night/SummaryStep.tsx");
  assert.match(summary, /beginFrame\("stacking", undefined, units\)/);
  assert.match(summary, /startUnit\(state, unitIndex, target\.designation/);
  assert.match(summary, /startUnit\(state, index, target\.designation/);
  assert.ok(
    !summary.includes("...beginProgress(state.startedAt)"),
    "a new target no longer resets the frame",
  );
});

test("every language says the new words of the frame", async () => {
  const keys = [
    "run.unitNight",
    "run.stepBarLabel",
    "run.doneLabel",
    "run.details",
    "run.emptyLog",
    "run.resultAlignedLights",
    "run.resultAlignedMasters",
  ];
  for (const tag of [
    "en",
    "fr",
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
      await source(`../i18n/${tag}.json`),
    ) as Record<string, string>;
    for (const key of keys) assert.ok(dictionary[key], `${tag}: ${key}`);
  }
});
