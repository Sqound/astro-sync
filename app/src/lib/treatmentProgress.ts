// What a treatment that runs on this computer says about itself while it
// runs: the step in words, a counter when the treatment knows one, a
// percentage when Siril gives one, since when it runs, and the last useful
// line of its journal. Pure TypeScript (no React, no Tauri): the events of
// the core come in through the `with*` functions, `progressFrameView` turns a
// state into what the frame writes; `screens/night/SummaryStep.tsx`,
// `screens/detect/TreatmentsFlow.tsx` and `components/ProgressFrame.tsx` draw
// it.
//
// One bar for the whole run. Siril reports the percentage of its CURRENT
// COMMAND, and every step of the script, every family, every night starts it
// again at zero: a bar drawn from that value restarts all along a night. A
// run that knows its units of work (the nights of a target, the targets of a
// stacking: `beginProgress`' `units`, their weights being the frames they
// hold) therefore draws `overall`, the share of the whole run, computed from
// the unit, the family, the step and the command's percentage, and held
// monotonic: nothing that arrives can send it back. The command's own
// percentage stays, as the thin bar of the current step.

import type { Translate } from "../i18n/locale.ts";
import type { ProgressionEvent, StackEvent, WorkProgress } from "./bridge";
import { formatDuration } from "./duration.ts";

/** The treatments that run here, as the sending's items name them. */
export type TreatmentKey =
  | "stacking"
  | "progression"
  | "logs"
  | "sky"
  /** The image of a multi-night plan: the stacking's events. */
  | "planImage";

/** After this long without a new line of the journal, the frame says so. */
export const SILENT_AFTER_MS = 60_000;

export type Step =
  /** Before the first step is known. */
  | { kind: "preparing" }
  /** A step of the stacking script (`master_darks`, `integration`…). */
  | { kind: "stack"; name: string; index: number; total: number }
  | { kind: "progression"; stage: "registration" | "milestones" }
  | { kind: "logs" }
  | { kind: "sky" };

export interface Counter {
  kind: "file" | "milestone";
  index: number;
  total: number;
}

/** What a step found, said beside its duration once it is done. */
export type StepResult =
  | {
      kind: "registered";
      registered: number;
      frames: number;
      unit: RegisteredUnit;
    }
  | { kind: "file"; file: string };

/** A step that ended: kept in the list under the bar. */
export interface DoneStep {
  /** The unit it belonged to (« Nuit du 9 août »), when the run has units. */
  unit: string | null;
  step: Step;
  durationMs: number;
  result: StepResult | null;
}

/** A line the engine printed, kept in the run's log. */
export interface LogEntry {
  at: number;
  unit: string | null;
  line: string;
}

/** The log keeps this many lines (the oldest are dropped). */
export const MAX_LOG_LINES = 500;

/** What the stacking learned of the night that the person should know while
 * it runs: the field turned, so the whole field is kept; some lights could
 * not be aligned and are left out. The image of a multi-night plan says the
 * same of its nights: `unit` is what the frames are, the lights of a night
 * or the masters of a plan. */
export type Note =
  | { kind: "fullField"; degrees: number; unit: RegisteredUnit }
  | {
      kind: "unregistered";
      count: number;
      total: number;
      unit: RegisteredUnit;
    }
  /** The corners of the image of a plan no dual-band night reaches:
   * `percent` of the image, a whole number from 1 to 100. */
  | { kind: "noDualBand"; percent: number }
  /** The image of a plan cropped to the area every night covers
   * (`common`), or only its edges without data removed (`clean`):
   * `percent` of the field kept, a whole number from 1 to 100. */
  | { kind: "planCropped"; percent: number }
  | { kind: "planCleanEdges"; percent: number };

export type RegisteredUnit = Extract<
  StackEvent,
  { event: "registration" }
>["unit"];

export interface TreatmentProgress {
  startedAt: number;
  finishedAt: number | null;
  step: Step;
  counter: Counter | null;
  /** The notes of the stacking in progress; a new family starts them again. */
  notes: readonly Note[];
  /** 0–100 as Siril gives it, or `null`: the bar does not say where it is. */
  percent: number | null;
  lastLine: string | null;
  lastLineAt: number | null;
  /** The weight of each unit of the run (the frames of a night, of a
   * target); empty for a treatment that does not know its units. */
  weights: readonly number[];
  /** The unit in progress (an index of `weights`). */
  unit: number;
  unitLabel: string | null;
  /** The family of the stacking in progress (a filter, a camera); `name` is
   * what the core calls it. */
  family: { index: number; total: number; name: string | null } | null;
  /** 0–1 along the unit in progress, held monotonic; `null` before any
   * signal. */
  within: number | null;
  /** 0–1 along the step in progress (Siril's percentage, held monotonic
   * inside the step). */
  stepFraction: number;
  /** 0–100 along the whole run, held monotonic; `null` when the run has no
   * units or has said nothing yet. */
  overall: number | null;
  stepStartedAt: number;
  /** What the step in progress found so far. */
  result: StepResult | null;
  /** The steps already done in this run. */
  done: readonly DoneStep[];
  /** The lines the engine printed in this run. */
  log: readonly LogEntry[];
}

/** The state of a treatment that starts at `now`. `units`: the weights of
 * the units of work the run is made of, when it knows them (see the header). */
export function beginProgress(
  now: number,
  step: Step = { kind: "preparing" },
  units: readonly number[] = [],
): TreatmentProgress {
  return {
    startedAt: now,
    finishedAt: null,
    step,
    counter: null,
    notes: [],
    percent: null,
    lastLine: null,
    lastLineAt: null,
    weights: units,
    unit: 0,
    unitLabel: null,
    family: null,
    within: null,
    stepFraction: 0,
    overall: null,
    stepStartedAt: now,
    result: null,
    done: [],
    log: [],
  };
}

/** Where the run stands, in words: the unit (« Nuit du 9 août »), and its
 * family (« Nuit du 9 août (Ha) ») when the target is stacked in several, so
 * that two steps of the same name are told apart in the list. */
function scopeLabel(state: TreatmentProgress): string | null {
  const family =
    state.family && state.family.total > 1 ? state.family.name : null;
  if (!family) return state.unitLabel;
  return state.unitLabel ? `${state.unitLabel} (${family})` : family;
}

/** The step in progress ends: it joins the list of the steps done. The
 * « preparing » step before the first one is not a step of the script. */
function closeStep(state: TreatmentProgress, now: number): TreatmentProgress {
  if (state.step.kind === "preparing") {
    return { ...state, stepStartedAt: now, result: null };
  }
  const record: DoneStep = {
    unit: scopeLabel(state),
    step: state.step,
    durationMs: Math.max(0, now - state.stepStartedAt),
    result: state.result,
  };
  return {
    ...state,
    done: [...state.done, record],
    stepStartedAt: now,
    result: null,
  };
}

const clamp01 = (value: number): number =>
  Number.isFinite(value) ? Math.min(1, Math.max(0, value)) : 0;

/** The share of the whole run (0–100) a position stands for. 100 is only
 * reached by `finishProgress`. */
export function overallShare(
  weights: readonly number[],
  unit: number,
  within: number,
): number | null {
  if (weights.length === 0) return null;
  const weight = (value: number | undefined) =>
    value !== undefined && value > 0 ? value : 1;
  const total = weights.reduce((sum, value) => sum + weight(value), 0);
  const at = Math.min(Math.max(unit, 0), weights.length - 1);
  let before = 0;
  for (let index = 0; index < at; index += 1) before += weight(weights[index]);
  const share =
    ((before + weight(weights[at]) * clamp01(within)) / total) * 100;
  return Math.min(99.9, Math.round(share * 10) / 10);
}

/** Puts `within` (never smaller than it was) and the whole run's share
 * (never smaller than it was) in the state. */
function advance(state: TreatmentProgress, within: number | null) {
  if (within === null) return state;
  const held = Math.max(state.within ?? 0, clamp01(within));
  const share = overallShare(state.weights, state.unit, held);
  const overall = share === null ? null : Math.max(state.overall ?? 0, share);
  return held === state.within && overall === state.overall
    ? state
    : { ...state, within: held, overall };
}

/** The run learns its units late (the nights of a target are only known once
 * their folders are prepared): their weights replace the ones it had. The
 * bar keeps what it already reached. */
export function withUnits(
  state: TreatmentProgress,
  units: readonly number[],
): TreatmentProgress {
  return { ...state, weights: units };
}

/** « Nuit du 8 oct. »: the evening of a night (`YYYY-MM-DD`, a calendar day,
 * not an instant) as the unit of a run. */
export function nightUnitLabel(
  t: Translate,
  locale: string,
  eveningDate: string,
): string {
  const date = new Date(`${eveningDate}T12:00:00Z`);
  const day = Number.isNaN(date.getTime())
    ? eveningDate
    : new Intl.DateTimeFormat(locale, {
        day: "numeric",
        month: "short",
        timeZone: "UTC",
      }).format(date);
  return t("run.unitNight", { day });
}

/** A unit of work begins (the next night, the next target): the step and the
 * family of the last one are over, and the bar stays where it was. */
export function startUnit(
  state: TreatmentProgress,
  index: number,
  label: string | null,
  now: number,
): TreatmentProgress {
  const closed = closeStep(state, now);
  return advance(
    {
      ...closed,
      step: { kind: "preparing" },
      unit: index,
      unitLabel: label,
      family: null,
      within: null,
      stepFraction: 0,
      percent: null,
      counter: null,
      notes: [],
    },
    0,
  );
}

export function finishProgress(
  state: TreatmentProgress,
  now: number,
): TreatmentProgress {
  const closed = closeStep(state, now);
  return {
    ...closed,
    step: state.step,
    finishedAt: now,
    within: state.weights.length > 0 ? 1 : closed.within,
    overall: state.overall === null ? null : 100,
  };
}

/** How long the treatment ran (to its end, or to `now` while it runs). */
export function elapsedMs(state: TreatmentProgress, now: number): number {
  return Math.max(0, (state.finishedAt ?? now) - state.startedAt);
}

const clampPercent = (value: number | null): number | null =>
  value === null || !Number.isFinite(value)
    ? null
    : Math.min(100, Math.max(0, value));

function withLine(
  state: TreatmentProgress,
  line: string | null,
  at: number,
): TreatmentProgress {
  if (line === null) return state;
  const last = state.log[state.log.length - 1];
  // The same text again (a refresh of the journal) is not a new entry.
  const log =
    last && last.line === line && last.unit === scopeLabel(state)
      ? state.log
      : [...state.log, { at, unit: scopeLabel(state), line }].slice(
          -MAX_LOG_LINES,
        );
  return { ...state, lastLine: line, lastLineAt: at, log };
}

/** The notes the registration of a stacking calls for: nothing for a field
 * that hardly turned and lights that were all aligned. */
function registrationNotes(
  event: Extract<StackEvent, { event: "registration" }>,
): Note[] {
  const notes: Note[] = [];
  if (event.framing === "full" && event.fieldRotationDegrees !== null) {
    notes.push({
      kind: "fullField",
      degrees: Math.round(event.fieldRotationDegrees),
      unit: event.unit,
    });
  }
  if (event.unregistered.length > 0) {
    notes.push({
      kind: "unregistered",
      count: event.unregistered.length,
      total: event.frames,
      unit: event.unit,
    });
  }
  return notes;
}

/** Where the stacking stands along its unit (0–1): the family of the
 * target, the step of the script and Siril's percentage in that step. */
function stackWithin(state: TreatmentProgress): number | null {
  const { step } = state;
  if (step.kind !== "stack" || step.total <= 0) return null;
  const families = Math.max(1, state.family?.total ?? 1);
  const family = Math.max(1, state.family?.index ?? 1);
  const inFamily =
    (Math.min(Math.max(step.index, 1), step.total) - 1 + state.stepFraction) /
    step.total;
  return (family - 1 + inFamily) / families;
}

/** Where the progression images stand along their unit (0–1): the alignment
 * of the poses is the first half, the milestones the second. */
function progressionWithin(state: TreatmentProgress): number | null {
  if (state.step.kind !== "progression") return null;
  if (state.step.stage === "registration") return 0.5 * state.stepFraction;
  const counter = state.counter;
  const milestones = counter && counter.total > 0 ? counter.total : 1;
  const passed = counter ? Math.max(0, counter.index - 1) : 0;
  return 0.5 + (0.5 * (passed + state.stepFraction)) / milestones;
}

/** An event of the stacking. A new step starts Siril's percentage again (and
 * the step before it joins the list of the steps done, at `now`); the
 * registration gives the notes; a new family forgets the last one's; the
 * output files are what the step found. The share of the whole run only
 * moves forward. */
export function withStackEvent(
  state: TreatmentProgress,
  event: StackEvent,
  now: number = Date.now(),
): TreatmentProgress {
  switch (event.event) {
    case "family":
      return advance(
        {
          ...closeStep(state, now),
          step: { kind: "preparing" },
          family: {
            index: event.index,
            total: event.total,
            name: event.family,
          },
          stepFraction: 0,
          percent: null,
          notes: [],
        },
        // The family changes the place along the unit: the next one starts
        // where the last one ended.
        Math.max(0, (event.index - 1) / Math.max(1, event.total)),
      );
    case "registration":
      return {
        ...state,
        notes: registrationNotes(event),
        result: {
          kind: "registered",
          registered: event.registered,
          frames: event.frames,
          unit: event.unit,
        },
      };
    case "output":
      return { ...state, result: { kind: "file", file: event.file } };
    case "crop": {
      if (!event.applied) return state;
      const percent = Math.min(
        100,
        Math.max(1, Math.round(event.keptShare * 100)),
      );
      const others = state.notes.filter(
        (note) => note.kind !== "planCropped" && note.kind !== "planCleanEdges",
      );
      // Only the edges without data go: the edges fewer nights cover stay,
      // as the registration said.
      if (event.mode === "clean")
        return {
          ...state,
          notes: [...others, { kind: "planCleanEdges", percent }],
        };
      // Cut to the area every night covers: the field is no longer the
      // whole one, whatever the registration said of it.
      return {
        ...state,
        notes: [
          ...others.filter(
            (note) => !(note.kind === "fullField" && note.unit === "masters"),
          ),
          { kind: "planCropped", percent },
        ],
      };
    }
    case "coverage": {
      // Under one percent the corners are not worth a sentence.
      const percent = Math.round(event.noDualBandShare * 100);
      return percent < 1
        ? state
        : {
            ...state,
            notes: [
              ...state.notes.filter((note) => note.kind !== "noDualBand"),
              { kind: "noDualBand", percent: Math.min(100, percent) },
            ],
          };
    }
    case "step": {
      const next: TreatmentProgress = {
        ...closeStep(state, now),
        step: {
          kind: "stack",
          name: event.step,
          index: event.index,
          total: event.total,
        },
        percent: null,
        stepFraction: 0,
      };
      return advance(next, stackWithin(next));
    }
    case "progress": {
      const percent = clampPercent(event.percent);
      const next = withLine(
        {
          ...state,
          percent,
          stepFraction:
            percent === null
              ? state.stepFraction
              : Math.max(state.stepFraction, percent / 100),
        },
        event.line,
        event.at,
      );
      return advance(next, stackWithin(next));
    }
    default:
      return state;
  }
}

/** An event of the progression images: which part, which milestone, what
 * Siril prints. */
export function withProgressionEvent(
  state: TreatmentProgress,
  event: ProgressionEvent,
  now: number = Date.now(),
): TreatmentProgress {
  switch (event.event) {
    case "stage": {
      const next: TreatmentProgress = {
        ...closeStep(state, now),
        step: { kind: "progression", stage: event.stage },
        percent: null,
        stepFraction: 0,
        counter: event.stage === "registration" ? null : state.counter,
      };
      return advance(next, progressionWithin(next));
    }
    case "milestone": {
      const next: TreatmentProgress = {
        ...state,
        counter: { kind: "milestone", index: event.index, total: event.total },
        stepFraction: 0,
      };
      return advance(next, progressionWithin(next));
    }
    case "progress": {
      const percent = clampPercent(event.percent);
      const next = withLine(
        {
          ...state,
          percent,
          stepFraction:
            percent === null
              ? state.stepFraction
              : Math.max(state.stepFraction, percent / 100),
        },
        event.line,
        event.at,
      );
      return advance(next, progressionWithin(next));
    }
  }
}

/** The report of the logs says which file it reads (« fichier 3 sur 41 »):
 * the file is its last line. Other kinds of local work are not its. */
export function withWorkProgress(
  state: TreatmentProgress,
  progress: WorkProgress,
  now: number,
): TreatmentProgress {
  if (progress.kind !== "analysis" || progress.total <= 0) return state;
  return withLine(
    {
      ...state,
      counter: {
        kind: "file",
        index: Math.min(progress.done, progress.total),
        total: progress.total,
      },
    },
    progress.current,
    now,
  );
}

/** The stacking was stopped by the person (« Arrêter l'empilement »): the
 * core answers `stacking_failed` with the reason `cancelled`. */
export function isStackingCancelled(error: unknown): boolean {
  if (typeof error !== "object" || error === null) return false;
  const { code, details } = error as { code?: unknown; details?: unknown };
  return (
    code === "stacking_failed" &&
    typeof details === "object" &&
    details !== null &&
    (details as { why?: unknown }).why === "cancelled"
  );
}

/** The stacking was refused because the offer does not include it (the core
 * answers `stacking_failed` with the reason `not_in_subscription`): not a
 * failure of the person or of the application, and the same for every
 * target. */
export function isOfferRefusal(error: unknown): boolean {
  if (typeof error !== "object" || error === null) return false;
  const { code, details } = error as { code?: unknown; details?: unknown };
  return (
    code === "stacking_failed" &&
    typeof details === "object" &&
    details !== null &&
    (details as { why?: unknown }).why === "not_in_subscription"
  );
}

/** The error of a stacking that did not start because the person had
 * already stopped it (same shape as the core's `stacking_failed`). */
export function stackingCancelledError(): {
  code: string;
  message: string;
  details: { why: string };
} {
  return {
    code: "stacking_failed",
    message: "Cancelled",
    details: { why: "cancelled" },
  };
}

/* --------------------------------- The view -------------------------------- */

const STACK_STEPS: Readonly<Record<string, string>> = {
  strip_wcs: "wizard.sending.stepStripWcs",
  "master_dark-flats": "wizard.sending.stepMasterDarkFlats",
  master_biases: "wizard.sending.stepMasterBiases",
  master_flats: "wizard.sending.stepMasterFlats",
  master_darks: "wizard.sending.stepMasterDarks",
  calibration: "wizard.sending.stepCalibration",
  registration: "wizard.sending.stepRegistration",
  integration: "wizard.sending.stepIntegration",
  render: "wizard.sending.stepRender",
  linear_export: "wizard.sending.stepLinearExport",
  // The image of a multi-night plan (`plan_stack.py`).
  alignment: "wizard.sending.stepRegistration",
  combination: "wizard.sending.stepCombination",
};

/** The name of the step alone (« intégration »); a step of the script this
 * window does not know is written as the script names it. */
export function stepName(t: Translate, step: Step): string {
  switch (step.kind) {
    case "preparing":
      return t("wizard.sending.stepPreparing");
    case "stack": {
      const key = STACK_STEPS[step.name];
      return key ? t(key) : step.name;
    }
    case "progression":
      return t(
        step.stage === "registration"
          ? "wizard.sending.stepProgressionRegistration"
          : "wizard.sending.stepProgressionMilestones",
      );
    case "logs":
      return t("wizard.sending.stepLogs");
    case "sky":
      return t("wizard.sending.stepSky");
  }
}

/** The step in words, with its place in the script. */
export function stepText(t: Translate, step: Step): string {
  return step.kind === "stack"
    ? t("wizard.sending.stepOf", {
        step: stepName(t, step),
        index: step.index,
        total: step.total,
      })
    : stepName(t, step);
}

/** What a step found, in words. */
export function resultText(t: Translate, result: StepResult): string {
  return result.kind === "registered"
    ? t(
        result.unit === "masters"
          ? "run.resultAlignedMasters"
          : "run.resultAlignedLights",
        { registered: result.registered, frames: result.frames },
      )
    : result.file;
}

/** A note of the stacking in words. */
export function noteText(t: Translate, note: Note): string {
  switch (note.kind) {
    case "fullField":
      return t(
        note.unit === "masters"
          ? "wizard.sending.notePlanFullField"
          : "wizard.sending.noteFullField",
        { degrees: note.degrees },
      );
    case "unregistered":
      return t(
        note.unit === "masters"
          ? "wizard.sending.notePlanUnregistered"
          : "wizard.sending.noteUnregistered",
        { count: note.count, total: note.total },
      );
    case "noDualBand":
      return t("wizard.sending.notePlanNoDualBand", { percent: note.percent });
    case "planCropped":
      return t("wizard.sending.notePlanCropped", { percent: note.percent });
    case "planCleanEdges":
      return t("wizard.sending.notePlanCleanEdges", { percent: note.percent });
  }
}

export function counterText(t: Translate, counter: Counter): string {
  return t(
    counter.kind === "file"
      ? "wizard.sending.counterFile"
      : "wizard.sending.counterMilestone",
    { index: counter.index, total: counter.total },
  );
}

/** A row of the log under « Détails »: the unit it belongs to, or a line. */
export interface LogRow {
  kind: "heading" | "line";
  text: string;
}

/** What a frame writes, ready to draw. */
export interface ProgressFrameView {
  /** « Empilement de M31 », written in bold. */
  label: string;
  /** What follows it in the first row (« · intégration, étape 6 sur 7 »). */
  headRest: string;
  /** 0–100 for a determined bar, `null` for a sliding one. For a run that
   * knows its units it is the share of the WHOLE run (it never goes back). */
  percent: number | null;
  percentText: string | null;
  /** The thin bar of the step in progress (Siril's percentage), only under
   * the bar of a whole run; it may start again at each step. */
  stepPercent: number | null;
  stepBarLabel: string;
  /** « Nuit du 9 août · intégration · 6 / 7 · 12 s »: the live line of a run
   * that knows its units; `counter` and the elapsed time stay for the
   * others. */
  live: string | null;
  counter: string | null;
  elapsed: string;
  /** What the stacking learned of the night (the field turned, lights left out). */
  notes: string[];
  /** The steps already done: « Nuit du 9 août · alignement · … · 12 s ». */
  done: string[];
  doneLabel: string;
  /** The log of the engine, collapsed under « Détails ». */
  log: LogRow[];
  detailsLabel: string;
  emptyLog: string;
  lastLine: string | null;
  /** « dernière ligne il y a 1 min 40 s », after a minute without news. */
  silent: string | null;
  ariaLabel: string;
}

/** The item « n / N » the live line shows: the file or the milestone being
 * done, else the place of the step in its script. */
function itemOf(state: TreatmentProgress): string | null {
  if (state.counter) return `${state.counter.index} / ${state.counter.total}`;
  return state.step.kind === "stack" && state.step.total > 0
    ? `${state.step.index} / ${state.step.total}`
    : null;
}

export function progressFrameView(
  t: Translate,
  input: { label: string; state: TreatmentProgress; now: number },
): ProgressFrameView {
  const { label, state, now } = input;
  const tracked = state.weights.length > 0;
  const whole = tracked && state.overall !== null;
  const shown = whole ? state.overall : state.percent;
  const percent =
    shown === null ? null : whole ? Math.floor(shown) : Math.round(shown);
  const sinceLine = now - (state.lastLineAt ?? state.startedAt);
  // A journal that has said nothing yet is not « silent »: only one that
  // said something and then stopped.
  const quiet = state.lastLineAt !== null && sinceLine >= SILENT_AFTER_MS;
  const head = t("wizard.sending.frameHead", {
    label,
    step: stepText(t, state.step),
  });
  const join = t("detect.join");
  const log: LogRow[] = [];
  let heading: string | null = null;
  for (const entry of state.log) {
    if (entry.unit !== null && entry.unit !== heading) {
      heading = entry.unit;
      log.push({ kind: "heading", text: entry.unit });
    }
    log.push({ kind: "line", text: entry.line });
  }
  return {
    label: head.startsWith(label) ? label : "",
    headRest: head.startsWith(label) ? head.slice(label.length) : head,
    percent: shown === null ? null : Math.round(shown * 10) / 10,
    percentText:
      percent === null ? null : t("wizard.sending.percent", { percent }),
    stepPercent:
      whole && state.percent !== null ? Math.round(state.percent) : null,
    stepBarLabel: t("run.stepBarLabel"),
    live: tracked
      ? [
          scopeLabel(state),
          stepName(t, state.step),
          itemOf(state),
          formatDuration(
            t,
            Math.max(0, (state.finishedAt ?? now) - state.stepStartedAt),
          ),
        ]
          .filter((part): part is string => part !== null)
          .join(join)
      : null,
    counter: state.counter ? counterText(t, state.counter) : null,
    elapsed: t("wizard.sending.elapsed", {
      duration: formatDuration(t, elapsedMs(state, now)),
    }),
    notes: state.notes.map((note) => noteText(t, note)),
    done: state.done.map((record) =>
      [
        record.unit,
        stepName(t, record.step),
        record.result ? resultText(t, record.result) : null,
        formatDuration(t, record.durationMs),
      ]
        .filter((part): part is string => part !== null)
        .join(join),
    ),
    doneLabel: t("run.doneLabel"),
    log,
    detailsLabel: t("run.details"),
    emptyLog: t("run.emptyLog"),
    lastLine: state.lastLine,
    silent: quiet
      ? t("wizard.sending.silent", { duration: formatDuration(t, sinceLine) })
      : null,
    ariaLabel: t("wizard.sending.frameLabel", { label }),
  };
}
