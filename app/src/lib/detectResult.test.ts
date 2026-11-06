import assert from "node:assert/strict";
import test from "node:test";

import type { BulkCreateOutcome, Proposal } from "./bridge.ts";
import {
  attachedFrames,
  firstToTreat,
  resultRows,
  sessionPreparation,
  shiftText,
} from "./detectResult.ts";

const proposal = {
  places: [
    {
      id: "p1",
      chosen: "k1",
      candidates: [{ placeId: "k1", name: "Maison", distanceKm: 1.4 }],
    },
  ],
  sessions: [
    {
      id: "a",
      place: "p1",
      existing: { kind: "reuse-plan", planId: "x", planName: "Voile – OIII" },
    },
    { id: "b", place: "p1", existing: { kind: "create" } },
  ],
  multiNight: [],
} as unknown as Proposal;
const outcome: BulkCreateOutcome = {
  sessions: [
    {
      detectedId: "b",
      sessionId: "s2",
      eveningDate: "2026-08-03",
      action: "created",
      planAction: "created",
      planId: "n",
      designations: ["NGC 7000"],
      frames: 72,
      shiftHours: -9,
    },
    {
      detectedId: "a",
      sessionId: "s1",
      eveningDate: "2026-08-07",
      action: "updated",
      planAction: "reused",
      planId: "x",
      designations: ["Sh2-103"],
      frames: 40,
      shiftHours: 0,
    },
  ],
  multiNight: [],
  placesCreated: [],
};

test("rows come newest first with their place and reused plan", () => {
  const rows = resultRows(proposal, outcome);
  assert.deepEqual(
    rows.map((r) => r.sessionId),
    ["s1", "s2"],
  );
  assert.equal(rows[0].place, "Maison");
  assert.equal(rows[0].planName, "Voile – OIII");
  assert.equal(rows[1].planName, null);
});

test("the shift reads with a true minus, the attached poses are counted", () => {
  assert.equal(shiftText(-9), "−9 h");
  assert.equal(shiftText(2), "+2 h");
  const rows = resultRows(proposal, outcome);
  assert.deepEqual(attachedFrames(rows), { frames: 40, sessions: 1 });
  assert.equal(firstToTreat(rows), "s1");
});

test("the targets added to existing sessions, and what was not written, named", async () => {
  const { addedTargets, skippedRows } = await import("./detectResult.ts");
  const made = {
    sessions: [
      {
        detectedId: "s1",
        sessionId: "x",
        eveningDate: "2026-08-11",
        action: "updated",
        addedTargets: ["Sh2-103"],
      },
      {
        detectedId: "s2",
        sessionId: "y",
        eveningDate: "2026-08-10",
        action: "created",
        addedTargets: [],
      },
    ],
    multiNight: [],
    placesCreated: [],
    skipped: [
      { kind: "session", id: "s3", reason: "one_plan_per_night" },
      { kind: "multi-night", id: "g", reason: "teapot" },
    ],
  } as unknown as BulkCreateOutcome;
  assert.deepEqual(addedTargets(made), [
    { designation: "Sh2-103", eveningDate: "2026-08-11" },
  ]);
  const rows = skippedRows(
    {
      sessions: [{ id: "s3", eveningDate: "2026-08-12", targets: [] }],
      multiNight: [{ id: "g", designation: "IC 1396" }],
    } as unknown as Proposal,
    made,
  );
  assert.deepEqual(
    rows.map((r) => [r.label, r.reason]),
    [
      ["2026-08-12", "one_plan_per_night"],
      ["IC 1396", "refused"],
    ],
  );
});

test("a session opened from the result gets a night folder of its attached targets only", () => {
  const withTargets = {
    ...proposal,
    sessions: [
      {
        id: "a",
        place: "p1",
        existing: { kind: "existing", sessionId: "s1" },
        targets: [
          { designation: "Sh2-103", targetId: "t-sh2" },
          { designation: "NGC 7000", targetId: "t-ngc" },
          { designation: "Unknown", targetId: null },
        ],
      },
      { id: "b", place: "p1", existing: { kind: "create" }, targets: [] },
    ],
  } as unknown as Proposal;
  const preparation = sessionPreparation(withTargets, outcome, "s1");
  assert.deepEqual(preparation?.targetIds, ["t-sh2"]);
  assert.deepEqual(
    preparation?.outcome.sessions.map((s) => s.sessionId),
    ["s1"],
    "the other sessions of the import are not prepared",
  );
  // Nothing to link: no known target among those attached, or no session.
  assert.equal(sessionPreparation(withTargets, outcome, "s2"), null);
  assert.equal(sessionPreparation(withTargets, outcome, "missing"), null);
});
