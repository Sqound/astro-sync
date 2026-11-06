import assert from "node:assert/strict";
import test from "node:test";

import {
  CREATED,
  DETECT_USER,
  DETECT_USER_FREE,
  ORGANISE_OUTCOME,
  ORGANISE_PLAN,
  PROPOSAL,
  PROPOSAL_FREE,
} from "./detect-fixtures.mjs";

// The data shared by the scenarios: the keys the bridge's types
// require are there, and the ids point at each other.

const REASONS = new Set([
  "place.known",
  "place.nearest",
  "place.choose",
  "place.create",
  "equipment.known",
  "equipment.choose",
  "session.create",
  "session.existing",
  "plan.reuse",
  "plan.replace",
  "plan.freeRuleExisting",
  "target.toSolve",
  "target.inSession",
  "target.addToSession",
  "target.overlap",
  "target.createPersonal",
  "target.detected",
  "clock.shift",
  "clock.suspect",
  "clock.needsPlace",
  "calibration.attached",
  "log.attached",
  "multiNight.locked",
  "multiNight.create",
]);

for (const [name, proposal] of [
  ["PROPOSAL", PROPOSAL],
  ["PROPOSAL_FREE", PROPOSAL_FREE],
] as const) {
  test(`${name}: the keys of a Proposal`, () => {
    for (const key of [
      "reading",
      "places",
      "equipment",
      "sessions",
      "multiNight",
      "calibrationSets",
      "logs",
      "setAside",
      "lines",
      "notes",
    ]) {
      assert.ok(key in proposal, key);
    }
    for (const line of proposal.lines) {
      assert.ok(REASONS.has(line.reason), line.reason);
      assert.equal(line.id, `${line.reason}:${line.subject}`);
      assert.equal(typeof line.checked, "boolean");
      assert.equal(typeof line.locked, "boolean");
      assert.equal(typeof line.ask, "boolean");
    }
    assert.equal(
      new Set(proposal.lines.map((l) => l.id)).size,
      proposal.lines.length,
    );
  });

  test(`${name}: the ids point at each other`, () => {
    const sessions = new Set(proposal.sessions.map((s) => s.id));
    const sets = new Set(proposal.calibrationSets.map((s) => s.id));
    const logs = new Set(proposal.logs.map((l) => l.id));
    const places = new Set(proposal.places.map((p) => p.id));
    for (const place of proposal.places) {
      for (const id of place.sessions) assert.ok(sessions.has(id), id);
    }
    for (const session of proposal.sessions) {
      assert.ok(places.has(session.place), session.place);
      for (const a of session.calibration)
        assert.ok(sets.has(a.setId), a.setId);
      for (const a of session.logs) assert.ok(logs.has(a.logId), a.logId);
    }
    for (const group of proposal.multiNight) {
      for (const night of group.nights)
        assert.ok(sessions.has(night.sessionId));
    }
    for (const line of proposal.lines) {
      assert.ok(
        line.kind === "place"
          ? places.has(line.subject)
          : line.kind === "equipment" || line.kind === "multi-night"
            ? true
            : sessions.has(line.subject.split("/")[0]),
        line.id,
      );
    }
  });
}

test("PROPOSAL: the totals of the shared data", () => {
  assert.equal(PROPOSAL.sessions.length, 7);
  const ic = PROPOSAL.multiNight.find((g) => g.designation === "IC 1805")!;
  assert.equal(ic.goalSeconds, 40140);
  assert.equal(ic.frames, 223);
  assert.equal(PROPOSAL.places[1].candidates.length, 2);
  assert.equal(PROPOSAL.places[2].action, "create");
  const aside = PROPOSAL.setAside;
  assert.equal(
    aside.withoutDate.length +
      aside.calibrationUnattached.length +
      aside.logsUnattached.length,
    25,
  );
});

test("PROPOSAL_FREE: the groups are locked, the rest is the same", () => {
  assert.ok(PROPOSAL_FREE.multiNight.every((g) => g.locked));
  assert.ok(PROPOSAL.multiNight.every((g) => !g.locked));
  assert.equal(DETECT_USER_FREE.account.advancedPlanning, false);
  assert.equal(DETECT_USER.account.advancedPlanning, true);
});

test("CREATED matches PROPOSAL", () => {
  const known = new Set(PROPOSAL.sessions.map((s) => s.id));
  assert.equal(
    CREATED.sessions.filter((s) => s.action === "created").length,
    4,
  );
  for (const created of CREATED.sessions)
    assert.ok(known.has(created.detectedId));
  assert.equal(CREATED.multiNight[0].goalSeconds, 40140);
  const made = new Set(CREATED.sessions.map((s) => s.sessionId));
  for (const id of CREATED.multiNight[0].sessionIds)
    assert.ok(made.has(id), id);
});

test("the tidying: 978 files, three checks failed", () => {
  assert.equal(ORGANISE_PLAN.files, 978);
  assert.equal(ORGANISE_PLAN.actions.length, 978);
  assert.equal(ORGANISE_OUTCOME.failed.length, 3);
});
