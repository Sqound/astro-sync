import assert from "node:assert/strict";
import test from "node:test";

import type { ImagePlan } from "./bridge";
import { planFromCore, planToCore, type CorePlan } from "./imagePlans.ts";

const image = {
  targetId: "ic1805",
  path: "IC 1805/progression/milestone_05.jpg",
  sha256: "5".repeat(64),
  size: 2_196_123,
  kind: "progression",
  render: null,
  frameCount: 90,
} as ImagePlan["image"];

test("a plan of the core, its decision flattened, is read as the window reads it, and written back the same", () => {
  const core: CorePlan = {
    image,
    decision: "progression_of_new",
    firstSha256: "e".repeat(64),
  };
  const read = planFromCore(core);
  assert.deepEqual(read, {
    image,
    decision: { decision: "progression_of_new", firstSha256: "e".repeat(64) },
  });
  assert.deepEqual(planToCore(read), core);
  // With the proposals, and a refusal with its reason.
  const variant: CorePlan = {
    image,
    decision: "variant",
    captureId: "capture-1",
    metadata: { variantGeneratedTitle: { en: "Raw render", fr: "Rendu brut" } },
  };
  assert.equal(planFromCore(variant).decision.decision, "variant");
  assert.deepEqual(planFromCore(variant).metadata, {
    variantGeneratedTitle: { en: "Raw render", fr: "Rendu brut" },
  });
  assert.deepEqual(planToCore(planFromCore(variant)), variant);
  const aside: CorePlan = {
    image,
    decision: "not_sent",
    reason: "no_capture_for_progression",
  };
  assert.deepEqual(planFromCore(aside).decision, {
    decision: "not_sent",
    reason: "no_capture_for_progression",
  });
});

test("a plan already in the window's shape is kept", () => {
  const plan: ImagePlan = { image, decision: { decision: "new_capture" } };
  assert.deepEqual(planFromCore(plan as unknown as CorePlan), plan);
});
