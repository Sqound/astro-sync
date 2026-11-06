// « Liste des incidents » of a night, as data: what the window read for the
// night (the signalements it holds, what the files say), what the person did
// to the list (ticks, edits, lines removed or added by hand) and what writing
// it comes to. The draft is a plain value with a reducer, so the import can
// keep one per session in its store: the person ticks on step 4, goes back,
// returns, and finds the ticks; « Envoyer N images » / « Terminer » writes
// them all (it is applied when the person goes to the next step). The night's
// own « Récapitulatif » uses the same reducer locally. Pure TypeScript,
// tested under `node --test`.

import type { AddedIncident, IncidentReplacement } from "./bridge";
import {
  fatesOf,
  initialTicks,
  mergedText,
  merge,
  pairsOf,
  payloadFor,
  summaryOf,
  TEXT_LIMITS,
  withQuick,
  withTick,
  type Line,
  type Quick,
  type Summary,
  type Ticks,
} from "./incidents.ts";

export type DraftStatus = "idle" | "loading" | "ready" | "failed";

export interface IncidentsDraft {
  /** `idle`: not read yet; `loading`: being read; `ready`; `failed`. */
  status: DraftStatus;
  reason: string | null;
  /** The signalements of the session and the detections of the files. */
  base: Line[];
  /** The detections the contract has no signalement for. */
  unproposed: { text: string }[];
  /** The person's edits of a line that was read, by line id. */
  edits: Record<string, Line>;
  /** The lines the person removed. */
  removed: string[];
  /** The lines the person added (« + Ajouter un incident »). */
  manual: Line[];
  ticks: Ticks;
}

export const EMPTY_DRAFT: IncidentsDraft = {
  status: "idle",
  reason: null,
  base: [],
  unproposed: [],
  edits: {},
  removed: [],
  manual: [],
  ticks: {},
};

export type DraftAction =
  | { type: "loading" }
  | { type: "loaded"; base: Line[]; unproposed: { text: string }[] }
  | { type: "failed"; reason: string }
  /** « Réessayer » / « Recharger »: the list is read again, the person's
   * edits and removals are dropped; the ticks stay for what comes back. */
  | { type: "reload" }
  | { type: "tick"; lines: readonly Line[]; id: string; value: boolean }
  | {
      type: "quick";
      lines: readonly Line[];
      choice: Quick;
      canReplace: boolean;
    }
  | { type: "edit"; line: Line }
  | { type: "addManual"; line: Line }
  | { type: "remove"; id: string }
  /** The ticks follow the lines (the measure of the poses adds some). */
  | { type: "reconcile"; lines: readonly Line[] };

/** The lines of the list: what was read and what the person added, minus
 * what they removed, as they edited it, in time order. `extra` are the lines
 * the measure of the poses makes (the night's own screen). */
export function draftLines(
  draft: IncidentsDraft,
  extra: readonly Line[] = [],
): Line[] {
  const removed = new Set(draft.removed);
  return merge(
    [...draft.base, ...extra, ...draft.manual]
      .filter((line) => !removed.has(line.id))
      .map((line) => draft.edits[line.id] ?? line),
  );
}

/** The ticks that fit the lines: a line that is new takes its starting
 * choice, the others keep the person's, a pair keeps one case ticked. A
 * choice for a line that is gone is dropped unless `keepUnknown`. */
export function reconcileTicks(
  previous: Ticks,
  lines: readonly Line[],
  keepUnknown = false,
): Ticks {
  const start = initialTicks(lines);
  const next: Record<string, boolean> = keepUnknown ? { ...previous } : {};
  for (const id of Object.keys(start)) {
    next[id] = id in previous ? previous[id] : start[id];
  }
  for (const pair of pairsOf(lines)) {
    if (!next[pair.declared] && !next[pair.detected])
      next[pair.declared] = true;
  }
  const same =
    Object.keys(next).length === Object.keys(previous).length &&
    Object.keys(next).every((key) => previous[key] === next[key]);
  return same ? previous : next;
}

function withReconciled(draft: IncidentsDraft): IncidentsDraft {
  const ticks = reconcileTicks(draft.ticks, draftLines(draft), true);
  return ticks === draft.ticks ? draft : { ...draft, ticks };
}

export function draftReducer(
  draft: IncidentsDraft,
  action: DraftAction,
): IncidentsDraft {
  switch (action.type) {
    case "loading":
      return { ...draft, status: "loading", reason: null };
    case "loaded":
      return withReconciled({
        ...draft,
        status: "ready",
        reason: null,
        base: action.base,
        unproposed: action.unproposed,
      });
    case "failed":
      return { ...draft, status: "failed", reason: action.reason };
    case "reload":
      return { ...draft, status: "idle", edits: {}, removed: [] };
    case "tick": {
      const ticks = withTick(
        draft.ticks,
        action.lines,
        action.id,
        action.value,
      );
      return ticks === draft.ticks ? draft : { ...draft, ticks };
    }
    case "quick":
      return {
        ...draft,
        ticks: withQuick(
          draft.ticks,
          action.lines,
          action.choice,
          action.canReplace,
        ),
      };
    case "edit":
      return action.line.manual
        ? withReconciled({
            ...draft,
            manual: draft.manual.map((m) =>
              m.id === action.line.id ? action.line : m,
            ),
          })
        : withReconciled({
            ...draft,
            edits: { ...draft.edits, [action.line.id]: action.line },
          });
    case "addManual": {
      const next = withReconciled({
        ...draft,
        manual: [...draft.manual, action.line],
      });
      // A line added by hand is ticked: the person wrote it to be added.
      return { ...next, ticks: { ...next.ticks, [action.line.id]: true } };
    }
    case "remove":
      return withReconciled({
        ...draft,
        removed: draft.removed.includes(action.id)
          ? draft.removed
          : [...draft.removed, action.id],
        manual: draft.manual.filter((m) => m.id !== action.id),
      });
    case "reconcile": {
      const ticks = reconcileTicks(draft.ticks, action.lines);
      return ticks === draft.ticks ? draft : { ...draft, ticks };
    }
  }
}

/* ------------------------------ What writing it comes to ------------------ */

export const isoOf = (unix: number): string =>
  new Date(Math.round(unix) * 1000).toISOString().replace(/\.\d{3}Z$/, "Z");

export interface IncidentsPlan {
  adds: AddedIncident[];
  replacements: IncidentReplacement[];
  /** The signalements a detection replaces, then the ones the person edited
   * (their words are what the done sheet names). */
  replacedLines: Line[];
  /** The signalements that stay as they are. */
  keptLines: Line[];
  summary: Summary;
}

/** What the ticked lines write: the detections added (alone or beside their
 * partner), the signalements a detection replaces (the person's words first,
 * then what was detected), the signalements the person edited. `separator`
 * joins the two texts of a replacement. */
export function planOf(
  lines: readonly Line[],
  ticks: Ticks,
  separator: string,
): IncidentsPlan {
  const fates = fatesOf(lines, ticks);
  const partnerOf = new Map<string, Line>();
  const byId = new Map(lines.map((l) => [l.id, l]));
  for (const pair of pairsOf(lines)) {
    partnerOf.set(pair.detected, byId.get(pair.declared) as Line);
  }
  const adds: AddedIncident[] = [];
  const replacements: IncidentReplacement[] = [];
  const replacedLines: Line[] = [];
  const replacedIds = new Set<string>();
  const until = (line: Line) =>
    line.genre === "pause" && line.end ? isoOf(line.end) : null;
  for (const line of lines) {
    const fate = fates[line.id];
    if (fate === "add" || fate === "alongside") {
      adds.push({
        key: line.id,
        eventType: line.genre,
        at: isoOf(line.start),
        until: until(line),
        payload: line.payload,
      });
    } else if (fate === "replaces") {
      const mine = partnerOf.get(line.id) as Line;
      const text = mergedText(
        mine.text,
        line.text,
        separator,
        TEXT_LIMITS[line.genre],
      );
      replacements.push({
        eventId: mine.eventId as string,
        eventType: line.genre,
        at: isoOf(line.start),
        until: until(line),
        payload: payloadFor(line.genre, text, line.payload),
      });
      replacedIds.add(mine.id);
      replacedLines.push(mine);
    }
  }
  // A signalement the person edited is replaced by its new form.
  for (const line of lines) {
    if (
      line.origin === "declared" &&
      line.edited &&
      !replacedIds.has(line.id)
    ) {
      replacements.push({
        eventId: line.eventId as string,
        eventType: line.genre,
        at: isoOf(line.start),
        until: until(line),
        payload: line.payload,
      });
      replacedLines.push(line);
    }
  }
  const keptLines = lines.filter(
    (l) => l.origin === "declared" && !replacedIds.has(l.id) && !l.edited,
  );
  return {
    adds,
    replacements,
    replacedLines,
    keptLines,
    summary: summaryOf(lines, ticks),
  };
}

/** What the draft writes (the lines the draft holds, its ticks). */
export function draftPlan(
  draft: IncidentsDraft,
  separator: string,
  extra: readonly Line[] = [],
): IncidentsPlan {
  return planOf(draftLines(draft, extra), draft.ticks, separator);
}

/** Whether the plan writes anything. */
export function writesSomething(plan: IncidentsPlan): boolean {
  return plan.adds.length > 0 || plan.replacements.length > 0;
}
