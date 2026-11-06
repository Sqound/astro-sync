import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import type { IncidentsView } from "../components/IncidentsCard";
import { useI18n } from "../i18n";
import {
  incidentsDeclared,
  incidentsDetect,
  incidentsEventIds,
  type DeclaredIncidents,
} from "../lib/bridge";
import { errorReason } from "../lib/errorReason";
import { isProvisionalId } from "../lib/importFlow";
import {
  poseLine,
  poseRuns,
  quickOf,
  type Line,
  type Quick,
} from "../lib/incidents";
import {
  declaredLine,
  detectedLine,
  unproposedText,
} from "../lib/incidentsLines";
import {
  draftLines,
  EMPTY_DRAFT,
  isoOf,
  planOf,
  reconcileTicks,
  type DraftAction,
  type IncidentsDraft,
} from "../lib/incidentsDraft";
import { shiftIncident } from "../lib/incidentsImport";
import { measureState } from "../lib/incidentsMeasure";
import { statsOf, type NightPoses } from "../lib/poseStats";

/** What the list needs to know of the night. */
export interface IncidentsNight {
  sessionId: string;
  /** The night is finished: a rework can replace a signalement. */
  finished: boolean;
  nightFolder: string;
  /** The targets of the night with lights: id and folder. */
  targets: { targetId: string; folderName: string }[];
  eveningDate: string;
  longitude: number | null;
  /** The evening's day, for « la session du … ». */
  startedAt: string;
  /** Seconds the session's clock is ahead of the files' (the clock
   * correction « Détecter les sessions » applied): what is found in the
   * files is moved by it. 0 or absent: none. */
  shiftSeconds?: number;
  /** The session was made by « Détecter les sessions »:
   * nothing was declared on it, and the card says so. */
  imported?: boolean;
  /** The poses are not measured on this screen (the result of « Détecter
   * les sessions », for the sessions it made and the ones it added poses
   * to): the card says so, with the way to measure them. */
  noMeasure?: boolean;
}

/** Where the choices on the list are kept: the import holds them in its store
 * (they survive a « Précédent »; its primary writes them), a night's own
 * « Récapitulatif » keeps them in the card. */
export interface DraftStore {
  draft: IncidentsDraft | undefined;
  dispatch: (action: DraftAction) => void;
}

/**
 * « Liste des incidents » of a night: the signalements the session holds,
 * what the core detects in the files, what the measure of the poses flagged
 * -- one list, with the choices the person makes. Nothing is written before
 * `apply` (at the result of « Détecter les sessions », the step's primary
 * writes it: `store` keeps the choices there and `apply` is not called).
 */
export function useIncidents(
  night: IncidentsNight | null,
  poses: NightPoses,
  store: DraftStore,
) {
  const { t, locale } = useI18n();
  const draft = store.draft ?? EMPTY_DRAFT;
  const dispatch = store.dispatch;
  const draftRef = useRef(draft);
  draftRef.current = draft;
  const [attempt, setAttempt] = useState(0);
  const session = night?.sessionId ?? null;
  const targetsKey = night?.targets.map((x) => x.folderName).join("|") ?? "";

  // The signalements and the detections of the files. A list the store
  // already holds (the person came back to the step) is not read again.
  useEffect(() => {
    if (!night) return;
    if (draftRef.current.status === "ready") return;
    let cancelled = false;
    dispatch({ type: "loading" });
    void (async () => {
      try {
        const folders = night.targets.length
          ? night.targets.map((x) => x.folderName)
          : [null];
        // A session the import has not created yet has nothing
        // declared and is not asked for it.
        const provisional = isProvisionalId(night.sessionId);
        const [declared, ...detections] = await Promise.all([
          provisional
            ? Promise.resolve<DeclaredIncidents>({
                events: [],
                truncated: false,
              })
            : incidentsDeclared(night.sessionId),
          ...folders.map((folder) =>
            incidentsDetect({
              nightFolder: night.nightFolder,
              targetFolder: folder,
              eveningDate: night.eveningDate,
              longitude: night.longitude,
            }),
          ),
        ]);
        const found = new Map<
          string,
          (typeof detections)[number]["incidents"][number]
        >();
        for (const detection of detections) {
          for (const incident of detection.incidents)
            found.set(incident.id, incident);
        }
        // A detection already in the session (an earlier « Ajouter ») is
        // not proposed again: the stored event stands for it.
        const keys = [...found.keys()];
        const ids =
          keys.length && !provisional
            ? await incidentsEventIds(night.sessionId, keys)
            : [];
        const stored = new Set(declared.events.map((e) => e.eventId));
        keys.forEach((key, index) => {
          if (stored.has(ids[index])) found.delete(key);
        });
        if (cancelled) return;
        const declaredLines = declared.events
          .map((event) => declaredLine(t, event))
          .filter((line): line is Line => line !== null);
        const detectedLines = [...found.values()].map((incident) =>
          detectedLine(
            t,
            locale,
            shiftIncident(incident, night.shiftSeconds ?? 0),
          ),
        );
        const seen = new Map<string, { text: string }>();
        for (const detection of detections) {
          for (const item of detection.unproposed) {
            seen.set(item.code, { text: unproposedText(t, item) });
          }
        }
        dispatch({
          type: "loaded",
          base: [...declaredLines, ...detectedLines],
          unproposed: [...seen.values()],
        });
      } catch (error) {
        if (cancelled) return;
        // A list that could not be read says so (and offers « Réessayer »).
        dispatch({
          type: "failed",
          reason: errorReason(t, error),
        });
      }
    })();
    return () => {
      cancelled = true;
    };
    // The files and the session decide the list; the language only says it.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [session, night?.nightFolder, targetsKey, night?.eveningDate, attempt]);

  // The lines the measure of the poses adds.
  const poseLines = useMemo(() => {
    if (!night) return [];
    const format = (value: number, digits: number) =>
      new Intl.NumberFormat(locale, {
        minimumFractionDigits: digits,
        maximumFractionDigits: digits,
      }).format(value);
    const lines: Line[] = [];
    for (const target of night.targets) {
      const entry = poses[target.targetId];
      if (!entry) continue;
      const stats = statsOf(entry);
      for (const run of poseRuns(entry)) {
        const family = stats?.families.find((f) =>
          f.measured.some((m) => m.path === run.poses[0].path),
        );
        const id = `poses:${target.targetId}:${run.kind}:${isoOf(run.poses[0].unix as number)}`;
        lines.push(
          poseLine(
            t,
            locale,
            run,
            id,
            {
              starCount: family?.medians.starCount ?? null,
              eccentricity: family?.medians.eccentricity ?? null,
              sky: family?.medians.skyBackground ?? null,
            },
            format,
          ),
        );
      }
    }
    return lines;
  }, [night, poses, t, locale]);

  const lines = useMemo(() => draftLines(draft, poseLines), [draft, poseLines]);
  const ticks = draft.ticks;

  // The choices follow the lines: a line that is new takes its starting
  // choice, the others keep the person's, a pair keeps one case ticked.
  useEffect(() => {
    if (
      reconcileTicks(draftRef.current.ticks, lines) !== draftRef.current.ticks
    )
      dispatch({ type: "reconcile", lines });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [lines]);

  const canReplace = night?.finished ?? false;
  const tick = useCallback(
    (id: string, value: boolean) =>
      dispatch({ type: "tick", lines, id, value }),
    [lines, dispatch],
  );
  const quick = useCallback(
    (choice: Quick) => dispatch({ type: "quick", lines, choice, canReplace }),
    [lines, canReplace, dispatch],
  );
  const edit = useCallback(
    (line: Line) => dispatch({ type: "edit", line }),
    [dispatch],
  );
  const addManual = useCallback(
    (line: Line) => dispatch({ type: "addManual", line }),
    [dispatch],
  );
  const remove = useCallback(
    (id: string) => dispatch({ type: "remove", id }),
    [dispatch],
  );
  const reload = useCallback(() => {
    dispatch({ type: "reload" });
    setAttempt((n) => n + 1);
  }, [dispatch]);

  const view: IncidentsView = useMemo(() => {
    if (draft.status === "failed")
      return { kind: "failed", reason: draft.reason ?? "" };
    if (draft.status !== "ready") return { kind: "loading" };
    // No pose of the night is measured yet (the card of the session's
    // processing says so as the result of « Détecter les sessions » does,
    // with the same way to measure them).
    const { noneMeasured, unmeasured } = measureState(night, poses);
    if (lines.length === 0) {
      return {
        kind: "empty",
        // The result of « Détecter les sessions » has no « Fichiers » step
        // to measure in: its card says where the measure is made instead.
        canMeasure: noneMeasured,
      };
    }
    return {
      kind: "list",
      lines,
      ticks,
      canReplace,
      note: null,
      unproposed: draft.unproposed,
      unmeasured,
    };
  }, [draft, lines, ticks, canReplace, night, poses]);

  const stableQuick = useMemo(() => quickOf(lines, ticks), [lines, ticks]);
  // What the primary of the step writes for this night (the lines of the
  // measure of the poses included).
  const separator = t("incidents.mergeSeparator");
  const plan = useMemo(
    () => planOf(lines, ticks, separator),
    [lines, ticks, separator],
  );
  return {
    view,
    plan,
    /** The list is being read: what is ticked is not known yet. */
    loading:
      night !== null && draft.status !== "ready" && draft.status !== "failed",
    quick: stableQuick,
    tick,
    setQuick: quick,
    edit,
    addManual,
    remove,
    reload,
  };
}
