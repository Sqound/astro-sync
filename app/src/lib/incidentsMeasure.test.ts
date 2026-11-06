import assert from "node:assert/strict";
import test from "node:test";

import { measureState } from "./incidentsMeasure.ts";
import type { NightPoses } from "./poseStats";

const targets = [
  { targetId: "a", folderName: "A" },
  { targetId: "b", folderName: "B" },
];
const done = { run: { status: "done" } } as unknown as NightPoses[string];
const idle = { run: { status: "idle" } } as unknown as NightPoses[string];

test("a session whose poses were never measured says the lines of the measure are not in the list", () => {
  assert.deepEqual(measureState({ targets }, {}), {
    noneMeasured: true,
    unmeasured: true,
  });
  assert.deepEqual(measureState({ targets }, { a: idle }), {
    noneMeasured: true,
    unmeasured: true,
  });
});

test("one target measured is enough: the measure was made", () => {
  assert.deepEqual(measureState({ targets }, { b: done }), {
    noneMeasured: false,
    unmeasured: false,
  });
});

test("a night without a target to measure keeps the way to the measure but no note", () => {
  assert.deepEqual(measureState({ targets: [] }, {}), {
    noneMeasured: true,
    unmeasured: false,
  });
});

test("at the result of « Détecter les sessions » the card has its own note, nothing here", () => {
  assert.deepEqual(measureState({ noMeasure: true, targets }, {}), {
    noneMeasured: false,
    unmeasured: false,
  });
  assert.deepEqual(measureState(null, {}), {
    noneMeasured: false,
    unmeasured: false,
  });
});
