// The plans of the « Récapitulatif » as the core writes them and as the
// window reads them. The core flattens a plan's decision into the plan
// (`precheck::ImagePlan`, `#[serde(flatten)]`):
// `{ image, decision: "variant", captureId, metadata }`; the window reads
// `plan.decision.decision` (`bridge.ts`'s `ImagePlan`). Without this the
// window read every decision as missing: the summary called a variant a
// final image and counted no progression image. Pure TypeScript, tested
// under `node --test`.

import type { Decision, ImagePlan } from "./bridge";

/** A plan of the core, its decision flattened. */
export type CorePlan = Record<string, unknown> & {
  image: ImagePlan["image"];
};

/** A plan of the core as the window reads it (a plan already in that shape
 * is kept as it is). */
export function planFromCore(raw: CorePlan): ImagePlan {
  const { image, metadata, decision, ...rest } = raw as CorePlan & {
    metadata?: ImagePlan["metadata"];
    decision: unknown;
  };
  const read: Decision =
    typeof decision === "object" && decision !== null
      ? (decision as Decision)
      : ({ decision, ...rest } as unknown as Decision);
  return metadata === undefined
    ? { image, decision: read }
    : { image, decision: read, metadata };
}

/** A plan of the window as the core reads it back (`night_enqueue`). */
export function planToCore(plan: ImagePlan): CorePlan {
  return {
    image: plan.image,
    ...plan.decision,
    ...(plan.metadata === undefined ? {} : { metadata: plan.metadata }),
  };
}
