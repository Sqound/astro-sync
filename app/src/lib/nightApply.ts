// What the primary of a step of the night's wizard writes: « Cibles » the
// targets and the plan, « Traitements » the measures, « Récapitulatif » the
// incidents. The calls are handed in, so the order, what a failure leaves
// undone and what is reported are tested under `node --test` without the
// application (the same as `importApply.ts` for the import).
//
// Each function applies its plan in the order of `nightRecap.ts` and stops at
// the first thing that fails: what was done stays done (the hook is called
// for it, the choice leaves its list), the rest waits, so that a second try
// goes on from there and the person can untick what cannot be done.

import type { TranslateParams } from "../i18n/locale.ts";
import type {
  IncidentsApplied,
  IncidentsApply,
  MasterOutcome,
  NewPlanTarget,
  NightPlanChange,
  NightTargetChange,
  PersonalTargetCreate,
  PersonalTargetCreated,
  PlanCreated,
} from "./bridge";
import type { IncidentsPlan } from "./incidentsDraft.ts";
import { incidentsResultOf } from "./importApply.ts";
import { masterQualityText, masterRefusalText } from "./masterMeasure.ts";
import type { CiblesPlan, MeasuresPlan, ReportItem } from "./nightRecap.ts";
import { attachRefusalWords } from "./planAttach.ts";

export interface NightApplyDeps {
  personalTargetCreate: (
    target: PersonalTargetCreate,
    sessionId: string,
  ) => Promise<PersonalTargetCreated>;
  nightTargetsChange: (
    sessionId: string,
    change: NightTargetChange,
  ) => Promise<string[]>;
  nightPlanCreate: (
    sessionId: string,
    target: NewPlanTarget,
  ) => Promise<PlanCreated>;
  nightPlanAttach: (
    sessionId: string,
    projectId: string,
  ) => Promise<NightPlanChange>;
  nightPlanDetach: (
    sessionId: string,
    projectId: string,
  ) => Promise<NightPlanChange>;
  masterMeasure: (
    sessionId: string,
    folderName: string,
    file: string,
  ) => Promise<MasterOutcome>;
  masterMeasureForget: (sessionId: string, folderName: string) => Promise<void>;
  incidentsApply: (request: IncidentsApply) => Promise<IncidentsApplied>;
}

/** What the words of a failure need. */
export interface ApplyContext {
  t: (key: string, params?: TranslateParams) => string;
  locale: string;
  /** A thrown error, in words (`formatError`). */
  reason: (error: unknown) => string;
}

/** The part of a step that is being written, for the line above the foot. */
export type ApplyWork = "targets" | "plan" | "measure" | "incidents";

export interface ApplyHooks {
  work: (work: ApplyWork) => void;
  targetApplied: (key: string) => void;
  planApplied: () => void;
  detachApplied: (projectId: string, title: string) => void;
  measureApplied: (targetId: string, figures: string) => void;
  forgetApplied: (targetId: string) => void;
}

/** What could not be written: which choice, and why, in words. */
export interface ApplyFailure {
  kind: "target" | "plan" | "detach" | "measure" | "forget" | "incidents";
  /** The target, the plan or the night the choice was about. */
  name: string;
  reason: string;
}

export type ApplyEnd = { ok: true } | { ok: false; failure: ApplyFailure };

const done: ApplyEnd = { ok: true };
const failed = (
  kind: ApplyFailure["kind"],
  name: string,
  reason: string,
): ApplyEnd => ({ ok: false, failure: { kind, name, reason } });

/** The words of a plan gesture the service did not do, `null` when it did. */
function planRefusal(
  context: ApplyContext,
  change: NightPlanChange | PlanCreated,
): string | null {
  const { t } = context;
  switch (change.result) {
    case "attached":
    case "detached":
    case "created":
      return null;
    case "closed":
      return change.closed === "unknown"
        ? t("multiNight.unknownService")
        : t("planAttach.reserved");
    case "missing":
      return change.missing === "place"
        ? t("planAttach.missingPlace")
        : t("planAttach.missingTarget");
    case "refused":
      return attachRefusalWords(t, change.code, change.message);
  }
}

/**
 * « Continuer » on « Cibles »: the targets to create, then the targets of the
 * catalogue to add (both join the night), then the plan to make or to join,
 * then the plans to leave. The folder of each target is made by the next step.
 */
export async function applyCibles(
  deps: NightApplyDeps,
  sessionId: string,
  plan: CiblesPlan,
  context: ApplyContext,
  hooks: ApplyHooks,
): Promise<ApplyEnd> {
  if (plan.creates.length + plan.adds.length > 0) hooks.work("targets");
  for (const target of plan.creates) {
    try {
      await deps.personalTargetCreate(
        target.create as PersonalTargetCreate,
        sessionId,
      );
    } catch (error) {
      return failed("target", target.designation, context.reason(error));
    }
    hooks.targetApplied(target.key);
  }
  for (const target of plan.adds) {
    try {
      await deps.nightTargetsChange(sessionId, {
        kind: "add",
        targetId: target.targetId as string,
      });
    } catch (error) {
      return failed("target", target.designation, context.reason(error));
    }
    hooks.targetApplied(target.key);
  }
  if (plan.plan || plan.detach.length > 0) hooks.work("plan");
  if (plan.plan) {
    const pick = plan.plan;
    try {
      const outcome =
        pick.kind === "attach"
          ? await deps.nightPlanAttach(sessionId, pick.projectId)
          : await deps.nightPlanCreate(sessionId, pick.target);
      const refusal = planRefusal(context, outcome);
      if (refusal !== null) return failed("plan", pick.title, refusal);
    } catch (error) {
      return failed("plan", pick.title, context.reason(error));
    }
    hooks.planApplied();
  }
  for (const leaving of plan.detach) {
    try {
      const outcome = await deps.nightPlanDetach(sessionId, leaving.projectId);
      const refusal = planRefusal(context, outcome);
      if (refusal !== null) return failed("detach", leaving.title, refusal);
    } catch (error) {
      return failed("detach", leaving.title, context.reason(error));
    }
    hooks.detachApplied(leaving.projectId, leaving.title);
  }
  return done;
}

/**
 * « Continuer » on « Traitements »: the measures kept that the person asked to
 * forget, then the masters to measure (the figures stay in the target's
 * folder and leave with its images).
 */
export async function applyMeasures(
  deps: NightApplyDeps,
  sessionId: string,
  plan: MeasuresPlan,
  context: ApplyContext,
  hooks: ApplyHooks,
): Promise<ApplyEnd> {
  if (plan.forget.length + plan.measures.length > 0) hooks.work("measure");
  for (const item of plan.forget) {
    try {
      await deps.masterMeasureForget(sessionId, item.folderName);
    } catch (error) {
      return failed("forget", item.designation, context.reason(error));
    }
    hooks.forgetApplied(item.targetId);
  }
  for (const item of plan.measures) {
    try {
      const outcome = await deps.masterMeasure(
        sessionId,
        item.folderName,
        item.file,
      );
      if (outcome.result !== "measured") {
        return failed(
          "measure",
          item.designation,
          masterRefusalText(context.t, outcome.code),
        );
      }
      hooks.measureApplied(
        item.targetId,
        masterQualityText(
          context.t,
          context.locale,
          outcome.quality,
          item.source,
        ),
      );
    } catch (error) {
      return failed("measure", item.designation, context.reason(error));
    }
  }
  return done;
}

export type IncidentsEnd =
  | { ok: true; item: Extract<ReportItem, { kind: "incidents" }> | null }
  | { ok: false; failure: ApplyFailure };

/** « Envoyer cette nuit »: the incidents ticked are added to the session
 * before anything leaves. Nothing to add: nothing is asked of the service. */
export async function applyIncidents(
  deps: NightApplyDeps,
  sessionId: string,
  plan: IncidentsPlan,
  night: string,
  context: ApplyContext,
  hooks: Pick<ApplyHooks, "work">,
): Promise<IncidentsEnd> {
  if (plan.adds.length === 0 && plan.replacements.length === 0) {
    return { ok: true, item: null };
  }
  hooks.work("incidents");
  try {
    const outcome = await deps.incidentsApply({
      sessionId,
      adds: plan.adds,
      replacements: plan.replacements,
    });
    const result = incidentsResultOf(outcome);
    return {
      ok: true,
      item: {
        kind: "incidents",
        added: result.added + result.replaced,
        rejected: result.rejected,
        refused: result.refused,
      },
    };
  } catch (error) {
    return {
      ok: false,
      failure: {
        kind: "incidents",
        name: night,
        reason: context.reason(error),
      },
    };
  }
}
