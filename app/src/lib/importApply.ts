// What the primary of step 4 applies, in its order: the sessions and plans
// created (nothing was written to the space before), the plan folders
// renamed after the plans, the incidents ticked of every night, the tidying
// when it is ticked, the images queued, then the report of each night's logs
// (the guiding curve, the poses, the steps of the night: what « Journaux »,
// ticked by default, sends from the night's wizard; ASTRO-SYNC-SEND-01: the
// import sent none, so its nights arrived without them). The calls are handed in, so the
// order, what is skipped on a second try and what is reported are tested
// under `node --test` without the application.

import type {
  ImageToSend,
  ImportCreated,
  IncidentsApplied,
  IncidentsApply,
  OrganiseMode,
  OrganiseOutcome,
  OrganisePlan,
  Settled,
} from "./bridge";
import type {
  Created,
  IncidentsResult,
  LogsResult,
  NightLogs,
  RecapPlan,
  RecapReport,
  RecapWork,
} from "./importFlow.ts";
import { tidyResultOf, tidyStateOf, type TidyResult } from "./tidy.ts";

export interface ApplyDeps {
  /** The sessions and plans created, the night folders renamed. */
  create: () => Promise<ImportCreated>;
  /** The plan folders renamed after the plans created. */
  settlePlans: (nights: Settled[]) => Promise<Settled[]>;
  incidentsApply: (request: IncidentsApply) => Promise<IncidentsApplied>;
  /** The working folder, where « Ranger » puts the poses. */
  workingFolder: () => Promise<string>;
  /** The plan of the tidying, for the sessions created. */
  organisePlan: (
    destination: string,
    mode: Exclude<OrganiseMode, "leave-in-place">,
    created: Created,
  ) => Promise<OrganisePlan>;
  organise: (plan: OrganisePlan) => Promise<OrganiseOutcome>;
  /** The images queued (and their uploads started), as the space created
   * them. */
  enqueue: (images: ImageToSend[], created: Created) => Promise<void>;
  /** The report of a night's logs sent to its session (the session created,
   * its folder renamed); absent when the space keeps no report
   * (`analysis.logs` false): nothing is sent. */
  sendLogs?: (sessionId: string, folder: string) => Promise<unknown>;
}

export interface ApplyHooks {
  phase: (work: RecapWork) => void;
  created: (created: Created) => void;
  plansDone: (plans: Settled[]) => void;
  incidentsDone: (sessionId: string, result: IncidentsResult) => void;
  tidyDone: (result: TidyResult) => void;
  logsDone?: (sessionId: string, result: LogsResult) => void;
}

/** What the primary starts from: what would be created (step 3's), what was
 * created already by an earlier try, the plan of the page, the images
 * ticked (as made, under their provisional identifiers), what an earlier try
 * reported. */
export interface ApplyInput {
  local: Created;
  created: Created | null;
  plan: Pick<RecapPlan, "incidents" | "tidy"> & { logs?: NightLogs[] };
  images: ImageToSend[];
  report: RecapReport;
  /** The images were queued already (a second try does not queue twice). */
  sent: boolean;
}

/** The words of an error, whatever was thrown (a command's error carries a
 * `message`). */
function reasonOf(error: unknown): string {
  if (error && typeof error === "object" && "message" in error)
    return String((error as { message: unknown }).message);
  return String(error);
}

/** What a night's incidents came to, from the service's answer. */
export function incidentsResultOf(outcome: IncidentsApplied): IncidentsResult {
  return {
    added: outcome.added.length + outcome.duplicate.length,
    replaced: outcome.replaced.length,
    rejected: outcome.rejected.length,
    refused: outcome.replacementsRefused !== null,
  };
}

export type ApplyEnd =
  | { ok: true; created: Created; images: ImageToSend[] }
  /** A stage could not go through: the primary stops there, what went
   * through stays (a second try skips it). `sessionId` is the night whose
   * incidents failed (the session created), `null` otherwise. */
  | {
      ok: false;
      stage: RecapWork;
      sessionId: string | null;
      error: unknown;
    };

/** A multi-night plan whose image is ticked was not created: its image cannot
 * leave. */
export class PlanNotCreated extends Error {
  readonly designation: string;
  constructor(designation: string) {
    super(`The multi-night plan ${designation} was not created.`);
    this.designation = designation;
  }
}

/** The identifiers the space gave, by the provisional ones: each session by
 * the night it was proposed for, each plan by its group of nights. */
export function createdIds(
  local: Created,
  created: Created,
): Record<string, string> {
  const ids: Record<string, string> = {};
  for (const session of local.outcome.sessions) {
    const made = created.outcome.sessions.find(
      (s) => s.detectedId === session.detectedId,
    );
    if (made && made.sessionId !== session.sessionId)
      ids[session.sessionId] = made.sessionId;
  }
  for (const group of local.outcome.multiNight) {
    const made = created.outcome.multiNight.find(
      (m) => m.groupId === group.groupId,
    );
    if (made) ids[group.planId] = made.planId;
  }
  return ids;
}

/** `path` in the folder it was moved to, when a provisional folder holding
 * it was renamed. */
export function movedPath(path: string, moves: readonly Settled[]): string {
  for (const move of moves) {
    if (path === move.from) return move.to;
    for (const sep of ["/", "\\"]) {
      if (path.startsWith(move.from + sep))
        return move.to + path.slice(move.from.length);
    }
  }
  return path;
}

/** An image made under provisional identifiers, as it leaves once the space
 * created its session (its plan): the identifiers it names, and its file in
 * the folder renamed. */
export function settledImage(
  image: ImageToSend,
  ids: Record<string, string>,
  moves: readonly Settled[],
): ImageToSend {
  const id = (value: string) => ids[value] ?? value;
  if (image.kind === "night")
    return {
      ...image,
      sessionId: id(image.sessionId),
      render: { ...image.render, path: movedPath(image.render.path, moves) },
    };
  const outcome = image.outcome;
  return {
    ...image,
    outcome: {
      ...outcome,
      projectId: id(outcome.projectId),
      combinedNightIds: outcome.combinedNightIds.map(id),
      render: {
        ...outcome.render,
        path: movedPath(outcome.render.path, moves),
      },
      ...(outcome.finishes
        ? {
            finishes: outcome.finishes.map((finish) => ({
              ...finish,
              render: {
                ...finish.render,
                path: movedPath(finish.render.path, moves),
              },
            })),
          }
        : {}),
      otherFiles: outcome.otherFiles.map((file) => movedPath(file, moves)),
    },
    // The milestones of the progression are files of the plan's folder too.
    ...(image.progression
      ? {
          progression: image.progression.map((step) => ({
            ...step,
            jpeg: movedPath(step.jpeg, moves),
          })),
        }
      : {}),
  };
}

/**
 * Applies the primary of step 4, stage after stage: the sessions (and the
 * plans, in the same call of the service) created and the night folders
 * renamed; the plan folders renamed; the incidents of each night (a night
 * written by an earlier try is skipped); the tidying; the images queued.
 * Stops at the first failure; what went through stays and is skipped by the
 * next try.
 */
export async function applyRecap(
  deps: ApplyDeps,
  input: ApplyInput,
  hooks: ApplyHooks,
): Promise<ApplyEnd> {
  const { local, plan, report } = input;
  let created = input.created;
  if (!created) {
    hooks.phase("sessions");
    try {
      const made = await deps.create();
      created = {
        request: local.request,
        outcome: made.outcome,
        nights: made.nights,
      };
      hooks.created(created);
    } catch (error) {
      return { ok: false, stage: "sessions", sessionId: null, error };
    }
  }
  const ids = createdIds(local, created);
  let plans = report.plans;
  if (local.outcome.multiNight.length > 0 && plans === null) {
    hooks.phase("plan");
    try {
      // An image of a plan the space did not create cannot leave.
      for (const image of input.images) {
        if (image.kind !== "plan") continue;
        const group = local.outcome.multiNight.find(
          (m) => m.planId === image.outcome.projectId,
        );
        if (group && !ids[group.planId])
          throw new PlanNotCreated(group.designation);
      }
      plans = await deps.settlePlans(created.nights ?? []);
      hooks.plansDone(plans);
    } catch (error) {
      return { ok: false, stage: "plan", sessionId: null, error };
    }
  }
  const todo = plan.incidents
    .map((night) => ({
      ...night,
      sessionId: ids[night.sessionId] ?? night.sessionId,
    }))
    .filter((night) => !(night.sessionId in report.incidents));
  if (todo.length > 0) hooks.phase("incidents");
  for (const night of todo) {
    try {
      const outcome = await deps.incidentsApply({
        sessionId: night.sessionId,
        adds: night.plan.adds,
        replacements: night.plan.replacements,
      });
      hooks.incidentsDone(night.sessionId, incidentsResultOf(outcome));
    } catch (error) {
      return {
        ok: false,
        stage: "incidents",
        sessionId: night.sessionId,
        error,
      };
    }
  }
  if (plan.tidy && report.tidy === null) {
    hooks.phase("tidy");
    try {
      const choice = plan.tidy;
      const destination = choice.destination ?? (await deps.workingFolder());
      const tidyPlan = await deps.organisePlan(
        destination,
        choice.mode,
        created,
      );
      // Everything in place already (the treatments linked every frame):
      // the engine is not asked, the result says how many are there.
      const outcome =
        tidyStateOf(tidyPlan).toDo > 0 ? await deps.organise(tidyPlan) : null;
      hooks.tidyDone(tidyResultOf(choice.mode, tidyPlan, outcome));
    } catch (error) {
      return { ok: false, stage: "tidy", sessionId: null, error };
    }
  }
  const moves = [...(created.nights ?? []), ...(plans ?? [])];
  const images = input.images.map((image) => settledImage(image, ids, moves));
  if (images.length > 0 && !input.sent) {
    hooks.phase("send");
    try {
      await deps.enqueue(images, created);
    } catch (error) {
      return { ok: false, stage: "send", sessionId: null, error };
    }
  }
  // The report of each night's logs, last: a night already reported by an
  // earlier try is skipped; a report that fails is said, never stops the
  // import (the sessions and the images went through).
  const logs = (plan.logs ?? [])
    .map((night) => ({
      sessionId: ids[night.sessionId] ?? night.sessionId,
      folder: movedPath(night.folder, moves),
    }))
    .filter((night) => report.logs?.[night.sessionId]?.sent !== true);
  if (deps.sendLogs && logs.length > 0) {
    hooks.phase("logs");
    for (const night of logs) {
      let result: LogsResult;
      try {
        await deps.sendLogs(night.sessionId, night.folder);
        result = { sent: true };
      } catch (error) {
        result = { sent: false, reason: reasonOf(error) };
      }
      hooks.logsDone?.(night.sessionId, result);
    }
  }
  return { ok: true, created, images };
}
