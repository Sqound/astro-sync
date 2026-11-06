// « Mesure des poses »: what the window keeps of the
// measure of a target's poses -- the answers as they come, the families, the
// poses that depart from the rest of the night, the choice « Écarter les poses
// signalées » of « Traitements » -- and the figures the card and its curves
// show. Pure TypeScript, tested under `node --test`; `components/PoseStatsCard`
// and `screens/night/TreatmentsStep` draw it. The measure itself is the
// core's (`pose_stats`): the window only files what it reports.

import type { Translate } from "../i18n/locale.ts";
import { formatMinutes, pluralText } from "./written.ts";

/* ----------------------------------- The core's shapes ------------------- */

/** `pose_stats::PoseMeasure`. */
export interface PoseMeasure {
  path: string;
  index: number;
  dateObs: string | null;
  unix: number | null;
  fwhmPixels: number | null;
  eccentricity: number | null;
  starCount: number | null;
  skyBackground: number | null;
  /** A pose with too few stars for a width of its own hands over the width of
   * each star it read (frame pixels), for the figure of its family. */
  starWidths?: number[];
  starEccentricities?: number[];
  unreadable: boolean;
}

/** `pose_stats::Reason`. */
export type Reason =
  | { kind: "dawn"; factor: number }
  | { kind: "sky"; factor: number }
  | { kind: "clouds" }
  | { kind: "trailed" }
  | { kind: "wide" };

export interface FlaggedPose {
  path: string;
  index: number;
  reason: Reason;
}

export interface Medians {
  fwhmPixels: number | null;
  eccentricity: number | null;
  starCount: number | null;
  skyBackground: number | null;
}

/** `pose_stats::FamilyShape`: the width of the stars of a whole family, pooled
 * over its poses when none has a width of its own. The family's, not a pose's. */
export interface FamilyShape {
  fwhmPixels: number;
  eccentricity: number | null;
  stars: number;
  poses: number;
}

/** `pose_stats::FamilyStats`. */
export interface FamilyStats {
  label: string;
  exposureSeconds: number | null;
  total: number;
  measured: PoseMeasure[];
  judged: boolean;
  medians: Medians;
  shapeMissing: number | null;
  /** Absent while the measure goes (the core gives it with the finished stats). */
  familyShape?: FamilyShape | null;
  flagged: FlaggedPose[];
}

/** `pose_stats::PoseStats`. */
export interface PoseStats {
  families: FamilyStats[];
  total: number;
  planned: number;
  stopped: boolean;
}

/** `pose_stats::PoseEvent`, emitted on `astro-sync://pose-stats`. */
export type PoseEvent =
  | {
      event: "pose";
      family: string;
      measure: PoseMeasure;
      done: number;
      planned: number;
      judged: boolean;
      medians: Medians;
      flagged: FlaggedPose[];
    }
  | { event: "neighbours"; count: number; planned: number };

/** `commands_poses::FamiliesPlan`: what the measure will take, before it starts. */
export interface FamilyPlan {
  label: string;
  exposureSeconds: number | null;
  total: number;
  planned: number;
}

export interface FamiliesPlan {
  families: FamilyPlan[];
  sampleAllUpTo: number;
  sampleStep: number;
}

/* ------------------------------------ What the window keeps -------------- */

/** The measure of one target: not started, going, stopped or finished. */
export type PoseRun =
  | { status: "idle" }
  | {
      status: "running";
      startedAt: number;
      /** What is measured so far, by family. */
      families: FamilyStats[];
      /** The family of the pose measured last: the one drawn while it goes. */
      family: string | null;
      done: number;
      planned: number;
    }
  | { status: "stopped" | "done"; stats: PoseStats }
  | { status: "failed"; error: unknown; stats: PoseStats | null };

export interface TargetPoses {
  run: PoseRun;
  /** What the families are, said before the measure (`pose_stats_plan`). */
  plan: FamiliesPlan | null;
  /** The poses flagged that the person **keeps** (unticked in « Traitements »):
   * the others are left out of the stacking. */
  kept: string[];
}

export const NO_POSES: TargetPoses = {
  run: { status: "idle" },
  plan: null,
  kept: [],
};

/** The poses of every target of the night, by target. */
export type NightPoses = Readonly<Record<string, TargetPoses>>;

export function posesOf(night: NightPoses, targetId: string): TargetPoses {
  return night[targetId] ?? NO_POSES;
}

/** What is measured so far, or what the measure came to; `null` before. */
export function statsOf(entry: TargetPoses): PoseStats | null {
  switch (entry.run.status) {
    case "idle":
      return null;
    case "running": {
      const families = entry.run.families;
      return {
        families,
        total: families.reduce((sum, f) => sum + f.total, 0),
        planned: entry.run.planned,
        stopped: false,
      };
    }
    case "failed":
      return entry.run.stats;
    default:
      return entry.run.stats;
  }
}

/** The families of a plan as empty stats: the curves start from them. */
export function familiesOfPlan(plan: FamiliesPlan): FamilyStats[] {
  return plan.families.map((family) => ({
    label: family.label,
    exposureSeconds: family.exposureSeconds,
    total: family.total,
    measured: [],
    judged: false,
    medians: {
      fwhmPixels: null,
      eccentricity: null,
      starCount: null,
      skyBackground: null,
    },
    shapeMissing: null,
    flagged: [],
  }));
}

/** The run that starts: nothing measured, or what a stopped measure kept
 * (« Reprendre la mesure »). */
export function startRun(
  plan: FamiliesPlan,
  resume: PoseStats | null,
  now: number,
): PoseRun {
  const families = resume ? resume.families : familiesOfPlan(plan);
  const done = families.reduce((sum, f) => sum + f.measured.length, 0);
  return {
    status: "running",
    startedAt: now,
    families,
    family: null,
    done,
    planned: Math.max(
      done,
      plan.families.reduce((sum, f) => sum + f.planned, 0),
    ),
  };
}

/** The run after one event of the core. */
export function withPoseEvent(run: PoseRun, event: PoseEvent): PoseRun {
  if (run.status !== "running") return run;
  if (event.event === "neighbours") return { ...run, planned: event.planned };
  const families = run.families.map((family) => {
    if (family.label !== event.family) return family;
    const known = family.measured.some((m) => m.path === event.measure.path);
    const measured = known
      ? family.measured
      : [...family.measured, event.measure].sort(
          (a, b) =>
            (a.unix ?? Number.MAX_VALUE) - (b.unix ?? Number.MAX_VALUE) ||
            a.index - b.index,
        );
    return {
      ...family,
      measured,
      judged: event.judged,
      medians: event.medians,
      flagged: event.flagged,
    };
  });
  return {
    ...run,
    families,
    family: event.family,
    done: event.done,
    planned: event.planned,
  };
}

/** Every pose flagged, all families. */
export function flaggedOf(stats: PoseStats | null): FlaggedPose[] {
  return stats ? stats.families.flatMap((family) => family.flagged) : [];
}

/** The poses left out of the stacking: the flagged ones the person did not
 * keep. Nothing while the measure is going (a measure not finished decides
 * nothing). */
export function discardedPaths(entry: TargetPoses): string[] {
  if (entry.run.status === "running" || entry.run.status === "idle") return [];
  const kept = new Set(entry.kept);
  return flaggedOf(statsOf(entry))
    .map((pose) => pose.path)
    .filter((path) => !kept.has(path));
}

/** The state of the case « Écarter les poses signalées » of « Traitements ». */
export type DiscardTick = "all" | "some" | "none";

export function discardTick(entry: TargetPoses): DiscardTick {
  const flagged = flaggedOf(statsOf(entry)).length;
  if (flagged === 0) return "none";
  const left = discardedPaths(entry).length;
  return left === flagged ? "all" : left === 0 ? "none" : "some";
}

/** The master case: ticked while some are left out, unticked ticks them all. */
export function withAllDiscarded(
  entry: TargetPoses,
  discard: boolean,
): TargetPoses {
  const flagged = flaggedOf(statsOf(entry)).map((pose) => pose.path);
  return { ...entry, kept: discard ? [] : flagged };
}

/** One pose's case. */
export function withPoseDiscarded(
  entry: TargetPoses,
  path: string,
  discard: boolean,
): TargetPoses {
  const kept = new Set(entry.kept);
  if (discard) kept.delete(path);
  else kept.add(path);
  return { ...entry, kept: [...kept] };
}

/* ----------------------------------- What the card says ------------------ */

/** « 10 s », « 1,5 s », « 180 s »: an exposure. */
export function exposureText(
  t: Translate,
  locale: string,
  seconds: number | null,
): string {
  if (seconds === null) return "—";
  const value = new Intl.NumberFormat(locale, {
    maximumFractionDigits: 1,
  }).format(seconds);
  return t("common.secondsShort", { count: value });
}

/** « 2 min »: the measure's length before it starts, half a second per pose (0.25 s measured on the Seestar night),
 * rounded up to the minute (a first estimate: the count of poses says it). */
export const SECONDS_PER_POSE = 0.5;

export function estimateMinutes(planned: number): number {
  return Math.max(1, Math.ceil((planned * SECONDS_PER_POSE) / 60));
}

/** The sentence under the title before the measure: what the sample is. */
export function sampleText(
  t: Translate,
  locale: string,
  plan: FamiliesPlan,
): string {
  const sampled = plan.families.some((f) => f.total > plan.sampleAllUpTo);
  const parts = [...plan.families]
    // Whole families first, the sampled ones after (« les 36 poses de 10 s,
    // puis une pose sur quatre pour… »).
    .sort(
      (a, b) =>
        Number(a.total > plan.sampleAllUpTo) -
        Number(b.total > plan.sampleAllUpTo),
    )
    .map((family) => {
      const exposure = exposureText(t, locale, family.exposureSeconds);
      return family.total > plan.sampleAllUpTo
        ? t("poseStats.part.every", {
            step: plan.sampleStep,
            count: family.total,
            exposure,
            planned: family.planned,
          })
        : t("poseStats.part.whole", { count: family.total, exposure });
    });
  const planned = plan.families.reduce((sum, f) => sum + f.planned, 0);
  return t(sampled ? "poseStats.sample.sampled" : "poseStats.sample.whole", {
    parts: parts.join(t("poseStats.sample.then")),
    duration: formatMinutes(t, estimateMinutes(planned)),
  });
}

/** How the pose ranks: its place in the segment choice « 30 s · 430 poses ». */
export function familySegment(
  t: Translate,
  locale: string,
  family: Pick<FamilyStats, "exposureSeconds" | "total">,
): string {
  return t("poseStats.segment", {
    exposure: exposureText(t, locale, family.exposureSeconds),
    poses: pluralText(t, locale, "poseStats.poses", family.total),
  });
}

/** The reason of a flagged pose in words. */
export function reasonText(t: Translate, reason: Reason): string {
  switch (reason.kind) {
    case "dawn":
      return t("poseStats.reason.dawn", { factor: Math.round(reason.factor) });
    case "sky":
      return t("poseStats.reason.sky", { factor: Math.round(reason.factor) });
    case "clouds":
      return t("poseStats.reason.clouds");
    case "trailed":
      return t("poseStats.reason.trailed");
    case "wide":
      return t("poseStats.reason.wide");
  }
}

const REASON_ORDER = ["dawn", "sky", "clouds", "trailed", "wide"] as const;

/** « 7 à l’aube, 3 sous un nuage probable, 2 aux étoiles allongées »: how the
 * poses flagged divide by reason. `withFactor` adds the sky's top factor to
 * the dawn (« Traitements »). */
export function reasonCountsText(
  t: Translate,
  flagged: readonly FlaggedPose[],
  withFactor = false,
  separator = t("poseStats.counts.separator"),
): string {
  const parts: string[] = [];
  for (const kind of REASON_ORDER) {
    const mine = flagged.filter((pose) => pose.reason.kind === kind);
    if (mine.length === 0) continue;
    let text = t(`poseStats.counts.${kind}`, { count: mine.length });
    if (withFactor && (kind === "dawn" || kind === "sky")) {
      const top = Math.max(
        ...mine.map((pose) =>
          "factor" in pose.reason ? Math.round(pose.reason.factor) : 0,
        ),
      );
      text += ` ${t("poseStats.counts.upTo", { factor: top })}`;
    }
    parts.push(text);
  }
  return parts.join(separator);
}

/* ------------------------------------- The hours of the place ------------ */

/** Minutes east of UTC at `unix` for the place: its time zone when the
 * person's data names one, else the mean solar offset of its longitude, else
 * UTC. */
export function placeOffsetMinutes(
  timeZone: string | null | undefined,
  longitude: number | null | undefined,
  unix: number,
  zoneOffset: (timeZone: string, at: Date) => number | null,
): number {
  if (timeZone) {
    const offset = zoneOffset(timeZone, new Date(unix * 1000));
    if (offset !== null) return offset;
  }
  if (typeof longitude === "number" && Number.isFinite(longitude)) {
    return Math.round(longitude / 15) * 60;
  }
  return 0;
}

/** « 05:00 »: the clock of the place at `unix`, on 24 hours in every
 * language (the same as `written.ts`). */
export function clockText(unix: number, offsetMinutes: number): string {
  const shifted = new Date((unix + offsetMinutes * 60) * 1000);
  const two = (value: number) => String(value).padStart(2, "0");
  return `${two(shifted.getUTCHours())}:${two(shifted.getUTCMinutes())}`;
}

/** « UTC+9 », « UTC+5:30 », « UTC−3 ». */
export function offsetText(offsetMinutes: number): string {
  if (offsetMinutes === 0) return "UTC";
  const sign = offsetMinutes > 0 ? "+" : "−";
  const abs = Math.abs(offsetMinutes);
  const hours = Math.floor(abs / 60);
  const minutes = abs % 60;
  return `UTC${sign}${hours}${minutes ? `:${String(minutes).padStart(2, "0")}` : ""}`;
}

/* ---------------------------------------------- The curves --------------- */

export type Metric = "fwhm" | "eccentricity" | "stars" | "sky";

export interface CurvePoint {
  /** 0..1 along the time axis of the family. */
  x: number;
  /** 0..1 up the axis of the curve. */
  y: number;
  flagged: boolean;
  path: string;
}

export interface Curve {
  metric: Metric;
  /** The label of the axis ends. */
  top: number;
  bottom: number;
  /** 0..1 */
  median: number | null;
  medianValue: number | null;
  /** The line of twenty stars (0..1) on the curve of the stars of a family
   * whose shape could not be read: under it, no width is given. */
  twenty: number | null;
  points: CurvePoint[];
}

export interface Tick {
  x: number;
  label: string;
}

export interface Curves {
  curves: Curve[];
  ticks: Tick[];
  /** The part of the time axis that was not measured (a stopped measure),
   * 0..1, or `null`. */
  unmeasured: { from: number; to: number } | null;
  /** The marks above the flagged runs (« nuage ? », « aube »), by x. */
  marks: { x: number; kind: Reason["kind"] }[];
}

const NICE = [1, 1.2, 1.5, 2, 2.5, 3, 4, 5, 6, 8, 10];

/** A round number at or above `value` (an axis end). */
function roundUp(value: number): number {
  if (value <= 0) return 1;
  const magnitude = 10 ** Math.floor(Math.log10(value));
  const scaled = value / magnitude;
  const step = NICE.find((n) => n >= scaled - 1e-9) ?? 10;
  return step * magnitude;
}

function axisOf(
  metric: Metric,
  values: readonly number[],
  median: number | null,
): [number, number] {
  if (values.length === 0) return [0, 1];
  const max = Math.max(...values, median ?? 0);
  const min = Math.min(...values, median ?? Number.MAX_VALUE);
  if (metric === "stars" || metric === "sky") return [0, roundUp(max * 1.05)];
  // A width and an eccentricity: an axis around the figures, rounded to a
  // tenth (or a twentieth) so its ends read.
  const unit = metric === "eccentricity" ? 0.05 : 0.5;
  const lower = Math.floor((min * 0.9) / unit) * unit;
  const upper = Math.ceil((max * 1.08) / unit) * unit;
  return [Math.max(0, lower), Math.max(lower + unit, upper)];
}

function valueOf(pose: PoseMeasure, metric: Metric): number | null {
  switch (metric) {
    case "fwhm":
      return pose.fwhmPixels;
    case "eccentricity":
      return pose.eccentricity;
    case "stars":
      return pose.starCount;
    case "sky":
      return pose.skyBackground;
  }
}

function medianOf(medians: Medians, metric: Metric): number | null {
  switch (metric) {
    case "fwhm":
      return medians.fwhmPixels;
    case "eccentricity":
      return medians.eccentricity;
    case "stars":
      return medians.starCount;
    case "sky":
      return medians.skyBackground;
  }
}

/** The metrics a family draws: the shape curves only when the family has a
 * width median (twenty stars or more on enough poses). */
export function metricsOf(family: FamilyStats): Metric[] {
  return family.medians.fwhmPixels === null
    ? ["stars", "sky"]
    : ["fwhm", "eccentricity", "stars", "sky"];
}

/** The curves of a family and the hours under them. The time axis spans the
 * family's first to last pose (the whole family when its poses are known);
 * ticks are whole hours of the place (half hours under three hours, quarter
 * hours under one). */
export function curvesOf(
  family: FamilyStats,
  offsetMinutes: number,
  spanOf?: { start: number; end: number },
): Curves {
  const measured = family.measured.filter((m) => m.unix !== null);
  const times = measured.map((m) => m.unix as number);
  const start = spanOf?.start ?? (times.length ? Math.min(...times) : 0);
  const end = Math.max(
    spanOf?.end ?? (times.length ? Math.max(...times) : 1),
    start + 1,
  );
  const x = (unix: number) => (unix - start) / (end - start);
  const flagged = new Set(family.flagged.map((f) => f.path));
  const curves: Curve[] = metricsOf(family).map((metric) => {
    const median = medianOf(family.medians, metric);
    const values = measured
      .map((m) => valueOf(m, metric))
      .filter((v): v is number => v !== null);
    const [bottom, top] = axisOf(metric, values, median);
    const y = (value: number) => (value - bottom) / (top - bottom);
    const shapeless = family.medians.fwhmPixels === null;
    return {
      metric,
      top,
      bottom,
      median: median === null ? null : y(median),
      medianValue: median,
      twenty: metric === "stars" && shapeless && top >= 20 ? y(20) : null,
      points: measured.flatMap((m) => {
        const value = valueOf(m, metric);
        return value === null
          ? []
          : [
              {
                x: x(m.unix as number),
                y: y(value),
                flagged: flagged.has(m.path),
                path: m.path,
              },
            ];
      }),
    };
  });
  const span = end - start;
  const step = span >= 3 * 3600 ? 3600 : span >= 3600 ? 1800 : 900;
  const ticks: Tick[] = [];
  const first = Math.ceil((start + offsetMinutes * 60) / step) * step;
  for (let local = first; local <= end + offsetMinutes * 60; local += step) {
    const unix = local - offsetMinutes * 60;
    ticks.push({ x: x(unix), label: clockText(unix, offsetMinutes) });
  }
  const last = times.length ? Math.max(...times) : start;
  const complete = measured.length >= family.total || last >= end - 1;
  return {
    curves,
    ticks,
    unmeasured: complete ? null : { from: x(last), to: 1 },
    marks: family.flagged.flatMap((f) => {
      const pose = family.measured.find((m) => m.path === f.path);
      return pose?.unix == null
        ? []
        : [{ x: x(pose.unix), kind: f.reason.kind }];
    }),
  };
}

/** The time span of a family: first to last measured pose; when the end of
 * the family is not measured yet (a measure going, or stopped), the span is
 * carried to the family's last pose at the pace of the poses measured, so that
 * the axis is the whole night and what is not measured shows as such. */
export function familySpan(
  family: FamilyStats,
): { start: number; end: number } | undefined {
  const timed = family.measured.filter(
    (m): m is PoseMeasure & { unix: number } => m.unix !== null,
  );
  if (timed.length === 0) return undefined;
  const first = timed.reduce((a, b) => (b.index < a.index ? b : a));
  const last = timed.reduce((a, b) => (b.index > a.index ? b : a));
  if (last.index >= family.total - 1 || last.index === first.index) {
    return {
      start: Math.min(...timed.map((m) => m.unix)),
      end: Math.max(...timed.map((m) => m.unix)),
    };
  }
  const pace = (last.unix - first.unix) / (last.index - first.index);
  return {
    start: first.unix,
    end: last.unix + pace * (family.total - 1 - last.index),
  };
}

/** The family the card shows: the one chosen if it exists, else the first
 * with a pose flagged, else the largest. */
export function shownFamily(
  stats: PoseStats,
  chosen: string | null,
): FamilyStats | null {
  return (
    stats.families.find((f) => f.label === chosen) ??
    stats.families.find((f) => f.flagged.length > 0) ??
    stats.families[0] ??
    null
  );
}

/** The poses a stacking will leave out, `null` when none: for
 * `stackingRun`'s `discarded`. */
export function discardedForRun(entry: TargetPoses): string[] | null {
  const paths = discardedPaths(entry);
  return paths.length > 0 ? paths : null;
}

/** What « Traitements » says about the target's measure. */
export type DiscardView =
  | { kind: "none" }
  | { kind: "unmeasured"; poses: number }
  | { kind: "partial"; measured: number; planned: number }
  | { kind: "few"; total: number }
  | { kind: "flagged"; flagged: FlaggedPose[]; total: number };

/** The kind of block the card of a target in « Traitements » shows. */
export function discardView(entry: TargetPoses, total: number): DiscardView {
  const stats = statsOf(entry);
  if (entry.run.status === "idle" || stats === null) {
    return { kind: "unmeasured", poses: total };
  }
  if (entry.run.status === "running") return { kind: "none" };
  const flagged = flaggedOf(stats);
  if (flagged.length > 0) return { kind: "flagged", flagged, total };
  if (entry.run.status === "stopped" || entry.run.status === "failed") {
    return {
      kind: "partial",
      measured: stats.families.reduce((sum, f) => sum + f.measured.length, 0),
      planned: stats.planned,
    };
  }
  if (stats.families.every((f) => f.shapeMissing !== null)) {
    return { kind: "few", total };
  }
  return { kind: "none" };
}
