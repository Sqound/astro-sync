// « Liste des incidents »: the signalements declared
// during the execution and what Astro Sync detected (the poses, the headers, the
// logs) in one list, by time; the pairs that seem to go together; the quick
// choice; what « Ajouter à la session » will write. Pure TypeScript, tested
// under `node --test`; `components/IncidentsCard` draws it. The detection
// itself and the writing are the core's (`incidents`): the window files what
// they say and decides what goes.

import type { Translate } from "../i18n/locale.ts";
import {
  discardedPaths,
  flaggedOf,
  statsOf,
  type FlaggedPose,
  type PoseMeasure,
  type TargetPoses,
} from "./poseStats.ts";

/* -------------------------------------------------- The shapes of the core -- */

/** The genres of the list: the four of the execution, then the four further
 * ones the service accepts (their payloads are the contract's). */
export type Genre =
  | "condition"
  | "equipment_issue"
  | "pause"
  | "note"
  | "exposure_change"
  | "guiding_timeouts"
  | "sensor_temperature_drift"
  | "guiding_spikes";

/** The four genres a person can choose or edit (the dialog). */
export const EDITABLE_GENRES: readonly Genre[] = [
  "condition",
  "equipment_issue",
  "pause",
  "note",
];

/** Whether a person can edit a line of this genre. The four others are
 * written by Astro Sync from the logs and the poses: the list shows a lock
 * (state, not a button) in place of the pencil, and the dialog never offers
 * them. */
export function isEditableGenre(genre: Genre): boolean {
  return EDITABLE_GENRES.includes(genre);
}

export type Origin = "declared" | "detected";

/** What backs a detected line: a short grey sentence, said from a code. */
export interface Evidence {
  source: "asiair" | "phd2" | "nina" | "fits" | "poses";
  code: string;
  params: Record<string, string | number>;
}

/** One line of the list. Times are Unix seconds (UTC). */
export interface Line {
  /** A declared line: its event's id; a detected one: its detection key. */
  id: string;
  origin: Origin;
  genre: Genre;
  start: number;
  /** The end of a pause (or of a stretch); `null` for an instant. */
  end: number | null;
  /** What the line says, in the person's language (a detected line's text is
   * made by the window from its code, then editable). */
  text: string;
  /** The payload the service takes for the genre (a detected line's, ready to
   * send; rebuilt from `text` when the person edits it). */
  payload: Record<string, unknown>;
  /** The grey line under a detected line. */
  evidence: Evidence | null;
  /** A declared line: the id of its `pause` (the `resume` belongs to it). */
  eventId: string | null;
  /** When a declared line was recorded (the device's clock), Unix seconds. */
  recordedAt: number | null;
  /** The person edited the line in the window. */
  edited?: boolean;
  /** The line was added by hand (« Ajouter un incident »). */
  manual?: boolean;
  /** An event Astro Sync wrote to the session before (a detection added
   * earlier): it stays, there is nothing to choose and it takes part in no
   * pair. Its badge is « Détecté ». */
  stored?: boolean;
}

/* ------------------------------------------------------------ Pairs --------- */

/** Two lines of one genre closer than this (or overlapping) seem to go
 * together (set on real nights). */
export const PAIR_GAP_SECONDS = 10 * 60;

export interface Pair {
  declared: string;
  detected: string;
}

const endOf = (line: Line) => line.end ?? line.start;

/** The distance between two lines, in seconds: zero when they overlap. */
export function gapBetween(a: Line, b: Line): number {
  if (a.start <= endOf(b) && b.start <= endOf(a)) return 0;
  return a.start > endOf(b) ? a.start - endOf(b) : b.start - endOf(a);
}

/** Pairs each declared line with the closest detected line of the same genre
 * within [`PAIR_GAP_SECONDS`], one to one (closest pairs first). */
export function pairsOf(lines: readonly Line[]): Pair[] {
  const declared = lines.filter((l) => l.origin === "declared" && !l.stored);
  const detected = lines.filter((l) => l.origin === "detected" && !l.stored);
  const edges: { d: Line; a: Line; gap: number }[] = [];
  for (const d of declared) {
    for (const a of detected) {
      if (d.genre !== a.genre) continue;
      const gap = gapBetween(d, a);
      if (gap <= PAIR_GAP_SECONDS) edges.push({ d, a, gap });
    }
  }
  edges.sort(
    (x, y) => x.gap - y.gap || x.d.start - y.d.start || x.a.start - y.a.start,
  );
  const usedD = new Set<string>();
  const usedA = new Set<string>();
  const pairs: Pair[] = [];
  for (const { d, a } of edges) {
    if (usedD.has(d.id) || usedA.has(a.id)) continue;
    usedD.add(d.id);
    usedA.add(a.id);
    pairs.push({ declared: d.id, detected: a.id });
  }
  return pairs;
}

/** The merged list, by time (the declared line before the detected one when
 * they start together), a pair kept together: the declared line first. */
export function merge(lines: readonly Line[]): Line[] {
  const pairs = pairsOf(lines);
  const partner = new Map<string, string>();
  for (const pair of pairs) {
    partner.set(pair.declared, pair.detected);
    partner.set(pair.detected, pair.declared);
  }
  const byId = new Map(lines.map((l) => [l.id, l]));
  // A pair is placed at its earlier line's time.
  const key = (line: Line) => {
    const other = partner.get(line.id);
    const start = other
      ? Math.min(line.start, (byId.get(other) as Line).start)
      : line.start;
    return start;
  };
  return [...lines].sort((a, b) => {
    const by = key(a) - key(b);
    if (by !== 0) return by;
    const pa = partner.get(a.id);
    // Within one pair: the declared first.
    if (pa === b.id) return a.origin === "declared" ? -1 : 1;
    return a.start - b.start || a.id.localeCompare(b.id);
  });
}

/* ---------------------------------------------------------- The choices ----- */

/** Which lines are ticked: the detected ones, and the declared ones that
 * have a detected partner. An isolated declared line is not a choice: it stays
 * in the session. */
export type Ticks = Readonly<Record<string, boolean>>;

/** The choices the list starts from: a detected line alone is ticked (nothing
 * was noted then); in a pair, the declared line is ticked and the detected one
 * is not. */
export function initialTicks(lines: readonly Line[]): Ticks {
  const pairs = pairsOf(lines);
  const paired = new Set(pairs.flatMap((p) => [p.declared, p.detected]));
  const ticks: Record<string, boolean> = {};
  for (const line of lines) {
    if (line.stored) continue;
    if (line.origin === "detected") ticks[line.id] = !paired.has(line.id);
    else if (paired.has(line.id)) ticks[line.id] = true;
  }
  return ticks;
}

/** One case ticked or unticked. In a pair at least one case stays ticked
 * (ticking the other one gives « les deux »): a case that would leave both off
 * is refused. */
export function withTick(
  ticks: Ticks,
  lines: readonly Line[],
  id: string,
  value: boolean,
): Ticks {
  if (!(id in ticks)) return ticks;
  const pair = pairsOf(lines).find(
    (p) => p.declared === id || p.detected === id,
  );
  if (pair && !value) {
    const other = pair.declared === id ? pair.detected : pair.declared;
    if (!ticks[other]) return ticks;
  }
  return { ...ticks, [id]: value };
}

export type Quick = "mine" | "detections" | "both";

/** The quick choice that says what every pair is, or `null` when the pairs
 * differ (choices made line by line: no segment is active). */
export function quickOf(lines: readonly Line[], ticks: Ticks): Quick | null {
  const pairs = pairsOf(lines);
  if (pairs.length === 0) return null;
  const kinds = new Set(
    pairs.map((p) => {
      const d = ticks[p.declared];
      const a = ticks[p.detected];
      return d && a ? "both" : d ? "mine" : "detections";
    }),
  );
  return kinds.size === 1 ? ([...kinds][0] as Quick) : null;
}

/** The ticks with every pair set to `choice`; the lines outside a pair do not
 * move. `canReplace` false (a night not finished): « detections » is refused. */
export function withQuick(
  ticks: Ticks,
  lines: readonly Line[],
  choice: Quick,
  canReplace = true,
): Ticks {
  if (choice === "detections" && !canReplace) return ticks;
  const next = { ...ticks };
  for (const pair of pairsOf(lines)) {
    next[pair.declared] = choice !== "detections";
    next[pair.detected] = choice !== "mine";
  }
  return next;
}

/** What each line comes to once added. */
export type Fate =
  | "stays" // a declared line: stays in the session
  | "replaced" // a declared line a detection replaces
  | "add" // a detected line alone, ticked: will be added
  | "replaces" // a detected line that replaces a declared one
  | "alongside" // a detected line added beside its declared partner
  | "skipped"; // a detected line not added

export function fatesOf(
  lines: readonly Line[],
  ticks: Ticks,
): Record<string, Fate> {
  const partner = new Map<string, string>();
  for (const pair of pairsOf(lines)) {
    partner.set(pair.declared, pair.detected);
    partner.set(pair.detected, pair.declared);
  }
  const fates: Record<string, Fate> = {};
  for (const line of lines) {
    const other = partner.get(line.id);
    if (line.stored) {
      fates[line.id] = "stays";
    } else if (line.origin === "declared") {
      fates[line.id] =
        other && !ticks[line.id] && ticks[other] ? "replaced" : "stays";
    } else if (!ticks[line.id]) {
      fates[line.id] = "skipped";
    } else if (!other) {
      fates[line.id] = "add";
    } else {
      fates[line.id] = ticks[other] ? "alongside" : "replaces";
    }
  }
  return fates;
}

export interface Summary {
  /** The detected lines that will be written (added or replacing). */
  count: number;
  isolated: number;
  replace: number;
  alongside: number;
  /** The declared lines that stay, and all the declared lines. */
  kept: number;
  declared: number;
}

export function summaryOf(lines: readonly Line[], ticks: Ticks): Summary {
  const fates = fatesOf(lines, ticks);
  const count = (fate: Fate) =>
    Object.values(fates).filter((value) => value === fate).length;
  const isolated = count("add");
  const replace = count("replaces");
  const alongside = count("alongside");
  const declared = lines.filter(
    (l) => l.origin === "declared" && !l.stored,
  ).length;
  return {
    count: isolated + replace + alongside,
    isolated,
    replace,
    alongside,
    kept: declared - replace,
    declared,
  };
}

/* -------------------------------------------------------------- Editing ----- */

export const TEXT_LIMITS: Record<Genre, number> = {
  condition: 300,
  equipment_issue: 300,
  pause: 200,
  note: 500,
  exposure_change: 300,
  guiding_timeouts: 300,
  sensor_temperature_drift: 300,
  guiding_spikes: 300,
};

/** `text` cut to `max` characters, on a character boundary (never in the
 * middle of an emoji or of a letter with its mark). */
export function cut(text: string, max: number): string {
  const chars = [...text];
  return chars.length <= max ? text : chars.slice(0, max).join("");
}

/** The payload a genre takes for a text (and, for a pause, nothing more). */
export function payloadFor(
  genre: Genre,
  text: string,
  previous: Record<string, unknown> = {},
): Record<string, unknown> {
  const trimmed = cut(text.trim(), TEXT_LIMITS[genre]);
  switch (genre) {
    case "condition": {
      const { sky } = previous as { sky?: string };
      return {
        ...(sky ? { sky } : {}),
        ...(trimmed ? { notes: trimmed } : {}),
      };
    }
    case "equipment_issue": {
      const { severity } = previous as { severity?: string };
      return { description: trimmed, severity: severity ?? "minor" };
    }
    case "pause": {
      const { cause } = previous as { cause?: string };
      return {
        ...(cause ? { cause } : {}),
        ...(trimmed ? { reason: trimmed } : {}),
      };
    }
    case "note":
      return { text: trimmed };
    default:
      // The service's newer genres keep the payload the detection gave.
      return previous;
  }
}

/** The line after the person edited it: another genre, time, end or text. */
export function edited(
  line: Line,
  change: { genre: Genre; start: number; end: number | null; text: string },
): Line {
  const genre = change.genre;
  const end = genre === "pause" ? change.end : null;
  return {
    ...line,
    genre,
    start: change.start,
    end,
    text: change.text,
    payload: payloadFor(
      genre,
      change.text,
      genre === line.genre ? line.payload : {},
    ),
    edited: true,
  };
}

/** What a replacement says when the person's own words were there: their text
 * first, then what was detected, `max` characters at most. */
export function mergedText(
  mine: string,
  detected: string,
  separator: string,
  max: number,
): string {
  const own = mine.trim();
  const found = detected.trim();
  if (!own) return cut(found, max);
  if (!found || own === found) return cut(own, max);
  return cut(`${own}${separator}${found}`, max);
}

/* ------------------------------------------- Lines from the poses ----- */

/** Poses flagged by the measure, grouped in runs (same reason kind, less than
 * [`PAIR_GAP_SECONDS`] apart) into detected lines. */
export interface PoseRun {
  kind: FlaggedPose["reason"]["kind"];
  poses: PoseMeasure[];
  flagged: FlaggedPose[];
}

export function poseRuns(entry: TargetPoses): PoseRun[] {
  const stats = statsOf(entry);
  if (!stats || entry.run.status === "running") return [];
  const runs: PoseRun[] = [];
  for (const family of stats.families) {
    const timed = family.flagged
      .map((flag) => ({
        flag,
        pose: family.measured.find((m) => m.path === flag.path),
      }))
      .filter(
        (item): item is { flag: FlaggedPose; pose: PoseMeasure } =>
          item.pose !== undefined && item.pose.unix !== null,
      )
      .sort((a, b) => (a.pose.unix as number) - (b.pose.unix as number));
    for (const item of timed) {
      const last = runs[runs.length - 1];
      const lastPose = last?.poses[last.poses.length - 1];
      if (
        last &&
        last.kind === item.flag.reason.kind &&
        lastPose &&
        (item.pose.unix as number) - (lastPose.unix as number) <=
          PAIR_GAP_SECONDS
      ) {
        last.poses.push(item.pose);
        last.flagged.push(item.flag);
      } else {
        runs.push({
          kind: item.flag.reason.kind,
          poses: [item.pose],
          flagged: [item.flag],
        });
      }
    }
  }
  return runs.sort(
    (a, b) => (a.poses[0].unix as number) - (b.poses[0].unix as number),
  );
}

/** Which poses of a flagged run the person left in (kept) are not an
 * incident: a pose the person keeps in « Traitements » is still a departing
 * pose; the incident is about the night, not about the choice. */
export function keptOut(entry: TargetPoses): Set<string> {
  return new Set(discardedPaths(entry));
}

export { flaggedOf };

/* -------------------------------------------------------- The text of a line - */

/** A line of the pose runs says its words from its kind and its figures. The
 * keys are `incidents.pose.<kind>` (the text) and `incidents.poseEvidence.<kind>`
 * (the grey line). */
const PLURAL_KINDS = new Set(["dawn", "sky", "trailed", "wide"]);

export function poseLine(
  t: Translate,
  locale: string,
  run: PoseRun,
  id: string,
  medians: {
    starCount: number | null;
    eccentricity: number | null;
    sky: number | null;
  },
  formatNumber: (value: number, digits: number) => string,
): Line {
  const first = run.poses[0];
  const last = run.poses[run.poses.length - 1];
  const count = run.poses.length;
  const params: Record<string, string | number> = { count };
  let genre: Genre = "equipment_issue";
  let payload: Record<string, unknown> = {};
  let key: string = run.kind;
  switch (run.kind) {
    case "dawn":
    case "sky": {
      genre = "condition";
      const factors = run.flagged.map((f) =>
        "factor" in f.reason ? Math.round(f.reason.factor) : 0,
      );
      params.factor = Math.max(...factors);
      params.low = Math.min(...factors);
      break;
    }
    case "clouds": {
      genre = "condition";
      payload = { sky: "cloudy" };
      const stars = run.poses
        .map((p) => p.starCount ?? 0)
        .sort((a, b) => a - b);
      params.stars = stars.map((n) => formatNumber(n, 0)).join(" / ");
      params.median =
        medians.starCount === null ? "—" : formatNumber(medians.starCount, 0);
      key = "clouds";
      break;
    }
    case "trailed": {
      const values = run.poses
        .map((p) => p.eccentricity)
        .filter((v): v is number => v !== null)
        .sort((a, b) => a - b);
      params.values = values.map((v) => formatNumber(v, 2)).join(" / ");
      params.median =
        medians.eccentricity === null
          ? "—"
          : formatNumber(medians.eccentricity, 2);
      break;
    }
    case "wide": {
      const values = run.poses
        .map((p) => p.fwhmPixels)
        .filter((v): v is number => v !== null)
        .sort((a, b) => a - b);
      params.values = values.map((v) => formatNumber(v, 1)).join(" / ");
      break;
    }
  }
  const one = new Intl.PluralRules(locale).select(count) === "one";
  const text = t(
    `incidents.pose.${key}${PLURAL_KINDS.has(key) ? (one ? ".one" : ".other") : ""}`,
    params,
  );
  const payloadText =
    genre === "condition"
      ? { ...payload, notes: cut(text, TEXT_LIMITS.condition) }
      : genre === "equipment_issue"
        ? {
            description: cut(text, TEXT_LIMITS.equipment_issue),
            severity: "minor",
          }
        : payload;
  return {
    id,
    origin: "detected",
    genre,
    start: first.unix as number,
    end: count > 1 || genre === "condition" ? (last.unix as number) : null,
    text,
    payload: payloadText,
    evidence: { source: "poses", code: `pose_${key}`, params },
    eventId: null,
    recordedAt: null,
  };
}
