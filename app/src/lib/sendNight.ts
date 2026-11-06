// « Récapitulatif » -> sending: the order in which the work of a sending is done. The three local treatments that have
// nothing to do with each other -- the stacking of the FITS, the progression images, the report of the logs -- run
// together; what follows needs all of them and keeps its order: the check with the service (it needs every image, stacked
// and progression), the queue, the upload, the sky solutions, the record of the sending. The progression images leave
// after the image of their target's capture, as its history: they are counted as such, never as captures or variants.
// After the upload come the lines of the night's multi-night plans and the image of each plan: a refusal or a failure
// there is said, never stops the night. Pure TypeScript (no React, no Tauri): the calls come in as functions, the order is
// tested under `node --test`; `screens/night/SummaryStep.tsx` draws the three states.

import type {
  EnqueueAnswer,
  ImageCandidate,
  ImagePlan,
  IntegrationsSent,
  NightPrecheck,
  PlanImageRun,
  QuotaCheck,
} from "./bridge";
import { isProgressionPlan } from "./wizardSummary.ts";

/** What the progression images give, even when a target failed on the way. */
export interface ProgressionResult {
  candidates: ImageCandidate[];
  /** The light frames behind the images, per target. */
  frames: Record<string, number>;
  /** The first target that failed, if one did: the others were still done. */
  failure: { error: unknown } | null;
}

export interface SendCalls {
  /** The stacking of every target chosen, one after the other (Siril keeps
   * the machine busy), or `null` when it is not ticked. */
  stacking: (() => Promise<ImageCandidate[]>) | null;
  progression: (() => Promise<ProgressionResult>) | null;
  logs: (() => Promise<unknown>) | null;
  precheck: (extra: ImageCandidate[]) => Promise<NightPrecheck>;
  /** The plans that leave (the final image only when it is ticked). */
  keep: (plan: ImagePlan) => boolean;
  enqueue: (plans: ImagePlan[], quota: QuotaCheck) => Promise<EnqueueAnswer>;
  upload: () => Promise<unknown>;
  /** The lines of the night's multi-night plans, or `null` when the night
   * works for none. */
  integrations: (() => Promise<IntegrationsSent[]>) | null;
  /** The image of each multi-night plan to make, by plan (made here, then
   * queued and sent), or `null`. */
  planImage: (() => Promise<Record<string, PlanImageRun>>) | null;
  /** The sky solutions, or `null` when they are not ticked: how many. */
  solve: (() => Promise<number>) | null;
  record: (progressionFrames: Record<string, number>) => Promise<unknown>;
}

export type SendResult =
  /** The service refused: nothing was queued. */
  | { kind: "refused"; precheck: NightPrecheck }
  | {
      kind: "sent";
      precheck: NightPrecheck;
      solved: number;
      /** The progression images that left: each the history of its
       * target's capture, never a capture nor a variant. */
      progressionImages: number;
      /** The same, per target. */
      progressionByTarget: Record<string, number>;
      /** The multi-night plans: the lines sent or refused, the images of
       * the plans. */
      multiNight: {
        integrations: IntegrationsSent[];
        planImage: Record<string, PlanImageRun>;
      };
      /** A treatment that failed without stopping the sending. */
      failures: {
        logs: { error: unknown } | null;
        progression: { error: unknown } | null;
        multiNight: { error: unknown } | null;
      };
    };

/**
 * Does the whole sending. A failure of the stacking, of the check, of the
 * queue, of the upload or of the sky solutions rejects, as it always did
 * (the screen says it and goes back to the summary) -- but the stacking is
 * let to finish its neighbours first. A failure of the report of the logs or
 * of the progression images does not stop the images: it is returned.
 */
export async function sendNight(calls: SendCalls): Promise<SendResult> {
  const [stacked, progressed, logged] = await Promise.allSettled([
    calls.stacking ? calls.stacking() : Promise.resolve([] as ImageCandidate[]),
    calls.progression
      ? calls.progression()
      : Promise.resolve<ProgressionResult>({
          candidates: [],
          frames: {},
          failure: null,
        }),
    calls.logs ? calls.logs() : Promise.resolve(null),
  ]);
  if (stacked.status === "rejected") throw stacked.reason;
  // A progression that could not even start fails as a whole.
  const progression: ProgressionResult =
    progressed.status === "fulfilled"
      ? progressed.value
      : {
          candidates: [],
          frames: {},
          failure: { error: progressed.reason },
        };
  const failures: {
    logs: { error: unknown } | null;
    progression: { error: unknown } | null;
    multiNight: { error: unknown } | null;
  } = {
    logs: logged.status === "rejected" ? { error: logged.reason } : null,
    progression: progression.failure,
    multiNight: null,
  };
  const checked = await calls.precheck([
    ...stacked.value,
    ...progression.candidates,
  ]);
  if (!checked.quota.allowed) return { kind: "refused", precheck: checked };
  const plans = checked.plans.filter(calls.keep);
  const answer = await calls.enqueue(plans, checked.quota);
  if (answer.result === "refused") {
    return {
      kind: "refused",
      precheck: {
        ...checked,
        quota: {
          ...checked.quota,
          allowed: false,
          findings: answer.findings,
        },
      },
    };
  }
  await calls.upload();
  // The multi-night plans: after the night's own images, so a plan's image
  // never delays them; their failure is said, the night is sent.
  const multiNight = {
    integrations: [] as IntegrationsSent[],
    planImage: {} as Record<string, PlanImageRun>,
  };
  try {
    if (calls.integrations)
      multiNight.integrations = await calls.integrations();
    if (calls.planImage) multiNight.planImage = await calls.planImage();
  } catch (error) {
    failures.multiNight = { error };
  }
  const solved = calls.solve ? await calls.solve() : 0;
  await calls.record(progression.frames).catch(() => undefined);
  const progressionByTarget: Record<string, number> = {};
  for (const plan of plans.filter(isProgressionPlan)) {
    progressionByTarget[plan.image.targetId] =
      (progressionByTarget[plan.image.targetId] ?? 0) + 1;
  }
  return {
    kind: "sent",
    precheck: checked,
    solved,
    progressionImages: Object.values(progressionByTarget).reduce(
      (sum, count) => sum + count,
      0,
    ),
    progressionByTarget,
    multiNight,
    failures,
  };
}
