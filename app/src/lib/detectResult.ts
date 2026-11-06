// « Détecter les sessions », the result: one row per session created or
// updated, newest first, and the sentence under the title. Pure: the
// window writes the words.

import type { BulkCreateOutcome, CreatedSession, Proposal } from "./bridge";

export interface ResultRow {
  sessionId: string;
  eveningDate: string;
  place: string;
  designations: string[];
  action: CreatedSession["action"];
  planAction: CreatedSession["planAction"];
  /** The planned plan reused or replaced, by name. */
  planName: string | null;
  shiftHours: number;
  frames: number;
  /** The multi-night plan this session belongs to, if any. */
  multiNight: {
    designation: string;
    nights: number;
    goalSeconds: number;
  } | null;
}

/** « −9 h », « +2 h »: the shift, with the true minus sign. */
export function shiftText(hours: number): string {
  return `${hours < 0 ? "−" : "+"}${Math.abs(hours)} h`;
}

/** The rows, newest evening first. */
export function resultRows(
  proposal: Proposal,
  outcome: BulkCreateOutcome,
): ResultRow[] {
  const detected = new Map(proposal.sessions.map((s) => [s.id, s]));
  const places = new Map(proposal.places.map((p) => [p.id, p]));
  const group = new Map<string, ResultRow["multiNight"]>();
  for (const made of outcome.multiNight) {
    const source = proposal.multiNight.find((g) => g.id === made.groupId);
    for (const id of made.sessionIds) {
      group.set(id, {
        designation: made.designation,
        nights: made.sessionIds.length,
        goalSeconds: made.goalSeconds,
      });
    }
    void source;
  }
  return outcome.sessions
    .map((made) => {
      const found = detected.get(made.detectedId);
      const place = found ? places.get(found.place) : undefined;
      const name =
        place?.candidates.find((c) => c.placeId === place.chosen)?.name ?? "";
      const match = found?.existing;
      return {
        sessionId: made.sessionId,
        eveningDate: made.eveningDate,
        place: name,
        designations: made.designations,
        action: made.action,
        planAction: made.planAction,
        planName:
          match && "planName" in match && made.planAction !== "created"
            ? match.planName
            : null,
        shiftHours: made.shiftHours,
        frames: made.frames,
        multiNight:
          group.get(made.sessionId) ?? group.get(made.detectedId) ?? null,
      };
    })
    .sort((a, b) => b.eveningDate.localeCompare(a.eveningDate));
}

/** The poses attached to the sessions that already existed. */
export function attachedFrames(rows: ResultRow[]): {
  frames: number;
  sessions: number;
} {
  const updated = rows.filter((row) => row.action === "updated");
  return {
    frames: updated.reduce((sum, row) => sum + row.frames, 0),
    sessions: updated.length,
  };
}

/** The first session to open for « Choisir les traitements… »: the newest
 * one that holds poses. */
export function firstToTreat(rows: ResultRow[]): string | null {
  return rows.find((row) => row.frames > 0)?.sessionId ?? null;
}

/** « Sh2-103 ajoutée à la session du 11 août »: the targets added to the
 * sessions that existed. */
export function addedTargets(
  outcome: BulkCreateOutcome,
): { designation: string; eveningDate: string }[] {
  return outcome.sessions.flatMap((made) =>
    made.action === "updated"
      ? (made.addedTargets ?? []).map((designation) => ({
          designation,
          eveningDate: made.eveningDate,
        }))
      : [],
  );
}

/** What the service or Astro Sync did not write, named for the person: the
 * night's date, the target and its night, the multi-night plan's target, and
 * the reason's key (`detect.skip.<reason>`, an unknown one reads
 * `refused`). */
export const SKIP_REASONS = [
  "one_plan_per_night",
  "plan_limit_reached",
  "plan_not_pending",
  "target_not_found",
  "target_unknown",
  "capability_required",
  "nights_missing",
  "no_target",
] as const;

export function skippedRows(
  proposal: Proposal,
  outcome: BulkCreateOutcome,
): { kind: string; id: string; label: string; reason: string }[] {
  return (outcome.skipped ?? []).map((item) => {
    const [sessionId, targetKey] = item.id.split("/");
    const session = proposal.sessions.find((s) => s.id === sessionId);
    const target = session?.targets.find((t) => t.key === targetKey);
    const group = proposal.multiNight.find((g) => g.id === item.id);
    const label =
      item.kind === "multi-night"
        ? (group?.designation ?? item.id)
        : item.kind === "target"
          ? `${target?.designation ?? targetKey} · ${session?.eveningDate ?? ""}`
          : (session?.eveningDate ?? item.id);
    const reason = (SKIP_REASONS as readonly string[]).includes(item.reason)
      ? item.reason
      : "refused";
    return { kind: item.kind, id: item.id, label, reason };
  });
}

/** What « open this session » asks of the engine before its wizard opens:
 * the night's folder of the working folder, with the frames, the calibration
 * and the logs of the session linked into it, as the treatments prepare it.
 * The outcome narrowed to the session, and the ids of the targets the import
 * attached to it (the ones in it, the ones added); `null` when it has
 * nothing to link (no frames, no known target). */
export function sessionPreparation(
  proposal: Proposal,
  outcome: BulkCreateOutcome,
  sessionId: string,
): { outcome: BulkCreateOutcome; targetIds: string[] } | null {
  const made = outcome.sessions.find((item) => item.sessionId === sessionId);
  const found = proposal.sessions.find((item) => item.id === made?.detectedId);
  if (!made || !found || made.frames === 0) return null;
  const targetIds = found.targets
    .filter(
      (target) =>
        target.targetId !== null &&
        made.designations.includes(target.designation),
    )
    .map((target) => target.targetId as string);
  if (targetIds.length === 0) return null;
  return { outcome: { ...outcome, sessions: [made] }, targetIds };
}
