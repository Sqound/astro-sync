import assert from "node:assert/strict";
import test from "node:test";

import { DETECT_USER, PROPOSAL, PROPOSAL_FREE } from "./detect-fixtures.mjs";
import type { Proposal } from "./bridge.ts";
import {
  answerLine,
  borrowsCalibration,
  buildRequest,
  calibrationItems,
  chooseFilter,
  choosePlace,
  chooseShift,
  counts,
  createLabel,
  datesText,
  eveningLabel,
  followRights,
  formatCoordinates,
  formatHours,
  formatKm,
  formatShift,
  groupStatus,
  initialState,
  logItems,
  matchCriteria,
  nightLines,
  nightsWaitingBand,
  nightStatus,
  placesWithoutName,
  isBlankName,
  placeOf,
  shiftInstant,
  singleNightTargets,
  tickedIds,
  toggleLine,
  utcHour,
} from "./detectReview.ts";

const proposal = PROPOSAL as unknown as Proposal;
const free = PROPOSAL_FREE as unknown as Proposal;
const session = (date: string) =>
  proposal.sessions.find((s) => s.eveningDate === date)!;
const sessionLine = (date: string) =>
  nightLines(proposal)[session(date).id].session!;

test("the initial state ticks what the engine ticked, nothing that asks", () => {
  const state = initialState(proposal);
  assert.equal(state.ticked[sessionLine("2026-07-31").id], true);
  const overlap = Object.values(
    nightLines(proposal)[session("2026-08-09").id].targets,
  ).find((line) => line.ask)!;
  assert.equal(state.ticked[overlap.id], false);
  assert.equal(state.answers[overlap.id], undefined);
  // The nearest of the two places is the answer from the start.
  assert.deepEqual(state.places["place-2"], {
    kind: "known",
    knownPlaceId: "place-maison",
  });
  // The place to create is asked.
  assert.equal(state.places["place-3"], undefined);
  assert.equal(state.shifts[session("2026-08-03").id], -9);
});

test("a night without its place waits, ticked or not", () => {
  const state = initialState(proposal);
  const waiting = nightStatus(proposal, state, session("2026-08-15"));
  assert.equal(waiting.waitsPlace, true);
  assert.equal(waiting.on, false);
  assert.equal(nightStatus(proposal, state, session("2026-07-31")).on, true);
});

test("answering the place wakes the nights that wait for it", () => {
  const place = proposal.places.find((p) => p.id === "place-3")!;
  const answered = choosePlace(proposal, initialState(proposal), place, {
    kind: "create",
    name: "Fuji",
  });
  assert.equal(nightStatus(proposal, answered, session("2026-08-15")).on, true);
  const withdrawn = choosePlace(proposal, answered, place, null);
  assert.equal(
    nightStatus(proposal, withdrawn, session("2026-08-15")).on,
    false,
  );
});

test("unticking a place puts its nights back to waiting", () => {
  const state = toggleLine(
    initialState(proposal),
    proposal.lines.find((l) => l.id === "place.known:place-1")!.id,
  );
  assert.equal(placeOf(proposal, state, proposal.places[0]), null);
  for (const date of ["2026-07-31", "2026-08-03", "2026-08-09"]) {
    assert.equal(nightStatus(proposal, state, session(date)).waitsPlace, true);
  }
  assert.equal(nightStatus(proposal, state, session("2026-08-07")).on, true);
  // What depends on a night that waits is not ticked.
  const ids = tickedIds(proposal, state);
  assert.ok(!ids.some((id) => id.includes("s-2026-08-03-1")));
});

test("unticking a night drops its targets, clock, calibration and logs", () => {
  const night = session("2026-08-03");
  const state = toggleLine(
    initialState(proposal),
    sessionLine("2026-08-03").id,
  );
  const ids = tickedIds(proposal, state);
  assert.ok(!ids.some((id) => id.includes(night.id)));
  assert.ok(ids.some((id) => id.includes(session("2026-07-31").id)));
  assert.deepEqual(
    buildRequest(proposal, state).shifts,
    [],
    "the shift of a night that is off is not asked",
  );
});

test("the multi-night plan needs two nights on, and its goal follows them", () => {
  const group = proposal.multiNight.find((g) => g.id === "mn-ic1805")!;
  let state = initialState(proposal);
  let status = groupStatus(proposal, state, group);
  assert.equal(status.on, true);
  assert.equal(status.seconds, 40140);
  state = toggleLine(state, sessionLine("2026-08-10").id);
  status = groupStatus(proposal, state, group);
  assert.equal(status.on, true);
  assert.equal(status.seconds, 17640 + 14400);
  state = toggleLine(state, sessionLine("2026-08-11").id);
  status = groupStatus(proposal, state, group);
  assert.equal(status.on, false);
  assert.equal(status.short, true);
});

test("a group whose night waits for its place waits too", () => {
  const group = proposal.multiNight.find((g) => g.id === "mn-ngc7000")!;
  const status = groupStatus(proposal, initialState(proposal), group);
  assert.equal(status.on, false);
  assert.equal(status.waitsPlace, true);
  assert.equal(status.waitingDate, "2026-08-15");
});

test("a locked group (no advanced planning) is never on", () => {
  const state = initialState(free);
  for (const group of free.multiNight) {
    assert.equal(groupStatus(free, state, group).on, false);
  }
  const ids = tickedIds(free, state);
  assert.ok(!ids.some((id) => id.startsWith("multiNight")));
});

test("the primary counts the sessions made and the plans, live", () => {
  const state = initialState(proposal);
  assert.deepEqual(createLabel(proposal, state), {
    kind: "both",
    sessions: 4,
    plans: 1,
  });
  const fewer = toggleLine(state, sessionLine("2026-08-10").id);
  assert.deepEqual(createLabel(proposal, fewer), {
    kind: "both",
    sessions: 3,
    plans: 1,
  });
  assert.deepEqual(createLabel(free, initialState(free)), {
    kind: "sessions",
    sessions: 4,
  });
  let none = state;
  for (const line of proposal.lines.filter((l) => l.kind === "session")) {
    if (none.ticked[line.id]) none = toggleLine(none, line.id);
  }
  assert.deepEqual(createLabel(proposal, none), { kind: "none" });
});

test("only existing sessions left: the primary updates them", () => {
  let state = initialState(proposal);
  for (const date of ["2026-07-31", "2026-08-03", "2026-08-07", "2026-08-10"]) {
    state = toggleLine(state, sessionLine(date).id);
  }
  state = toggleLine(state, "multiNight.create:mn-ic1805");
  assert.deepEqual(createLabel(proposal, state), {
    kind: "update",
    sessions: 2,
  });
});

test("the footer's counts: reuse, replacement, poses attached, target added", () => {
  const found = counts(proposal, initialState(proposal));
  assert.equal(found.created, 4);
  assert.equal(found.updated, 2);
  assert.equal(found.attachedFrames, 178);
  assert.equal(found.attachedSessions, 2);
  assert.deepEqual(
    found.reused.map((r) => [r.planName, r.date]),
    [["Voile – OIII", "2026-08-07"]],
  );
  assert.deepEqual(
    found.replaced.map((r) => [r.planName, r.targets]),
    [["M33 – RVB", ["M31"]]],
  );
  assert.deepEqual(found.added, [
    { designation: "Sh2-103", date: "2026-08-11" },
  ]);
  assert.deepEqual(found.unanswered, [
    { designation: "Sh2-103", date: "2026-08-09" },
  ]);
  assert.deepEqual(found.multiGoals, [40140]);
  assert.equal(found.hasCalibration, true);
  assert.equal(found.hasLogs, true);
});

test("an overlap answered « add » is counted, « skip » is not, no answer is said", () => {
  const overlap = Object.values(
    nightLines(proposal)[session("2026-08-09").id].targets,
  ).find((line) => line.ask)!;
  const base = initialState(proposal);
  const add = counts(proposal, answerLine(base, overlap.id, "add"));
  assert.equal(add.added.length, 2);
  assert.equal(add.unanswered.length, 0);
  const skip = counts(proposal, answerLine(base, overlap.id, "skip"));
  assert.equal(skip.added.length, 1);
  assert.equal(skip.unanswered.length, 0);
});

test("the request carries the ticked ids, the places, the answers, the shifts", () => {
  const overlap = Object.values(
    nightLines(proposal)[session("2026-08-09").id].targets,
  ).find((line) => line.ask)!;
  const place = proposal.places.find((p) => p.id === "place-3")!;
  let state = initialState(proposal);
  state = answerLine(state, overlap.id, "skip");
  state = choosePlace(proposal, state, place, { kind: "create", name: "Fuji" });
  const request = buildRequest(proposal, state);
  // The sessions are the proposal's own; the place to create carries the name
  // the person typed (it names the night's folder), nothing else changes.
  assert.equal(request.proposal.sessions, proposal.sessions);
  const named = request.proposal.places.find((p) => p.id === "place-3")!;
  assert.equal(named.chosen, "created:place-3");
  assert.deepEqual(
    named.candidates.map((c) => c.name),
    ["Fuji"],
  );
  assert.deepEqual(
    request.proposal.places.filter((p) => p.id !== "place-3"),
    proposal.places.filter((p) => p.id !== "place-3"),
  );
  assert.ok(request.ticked.includes(sessionLine("2026-07-31").id));
  assert.ok(request.ticked.includes("multiNight.create:mn-ic1805"));
  assert.ok(request.ticked.includes("multiNight.create:mn-ngc7000"));
  assert.ok(
    !request.ticked.includes(overlap.id),
    "a line that asks is answered, not ticked",
  );
  assert.deepEqual(request.answers, [{ lineId: overlap.id, answer: "skip" }]);
  assert.deepEqual(request.shifts, [
    { sessionId: session("2026-08-03").id, hours: -9 },
  ]);
  assert.deepEqual(
    request.places.map((p) => [p.placeId, p.kind]),
    [
      ["place-1", "known"],
      ["place-2", "known"],
      ["place-3", "create"],
    ],
  );
  const created = request.places.find((p) => p.kind === "create")!;
  assert.equal(created.kind === "create" && created.name, "Fuji");
  assert.equal(created.kind === "create" && created.latitude, 35.6582);
});

test("a place no night uses is not asked for", () => {
  const request = buildRequest(proposal, initialState(proposal));
  assert.ok(!request.places.some((p) => p.placeId === "place-3"));
  assert.ok(!request.ticked.some((id) => id.includes("s-2026-08-15-1")));
});

test("the clock: « Aucun décalage » unticks it, a shift ticks it again", () => {
  const id = session("2026-08-03").id;
  const none = chooseShift(proposal, initialState(proposal), id, 0);
  assert.equal(
    nightStatus(proposal, none, session("2026-08-03")).shiftHours,
    0,
  );
  assert.deepEqual(buildRequest(proposal, none).shifts, []);
  const three = chooseShift(proposal, none, id, 3);
  assert.equal(
    nightStatus(proposal, three, session("2026-08-03")).shiftHours,
    3,
  );
  assert.deepEqual(buildRequest(proposal, three).shifts, [
    { sessionId: id, hours: 3 },
  ]);
});

test("the durations: 11 h 09, 3 h 00, 6 min", () => {
  assert.equal(formatHours(40140), "11 h 09");
  assert.equal(formatHours(19200), "5 h 20");
  assert.equal(formatHours(10800), "3 h 00");
  assert.equal(formatHours(360), "6 min");
});

test("the numbers follow the language", () => {
  assert.equal(formatKm(1.4, "fr"), "1,4");
  assert.equal(formatKm(8.1, "en"), "8.1");
  assert.equal(
    formatCoordinates(43.8731, 7.4312, "fr"),
    "43,8731 N · 7,4312 E",
  );
  assert.equal(formatCoordinates(-33.5, -70.25, "en"), "33.5000 S · 70.2500 W");
  assert.equal(formatShift(-9), "−9 h");
  assert.equal(formatShift(3), "+3 h");
});

test("the dates of a night and of a group", () => {
  assert.equal(eveningLabel("2026-07-31", "fr"), "Ven. 31 juil.");
  assert.equal(
    datesText(["2026-08-09", "2026-08-10", "2026-08-11"], "fr"),
    "9, 10 et 11 août",
  );
  assert.equal(datesText(["2026-08-03", "2026-08-15"], "fr"), "3 et 15 août");
  assert.equal(datesText(["2026-08-07"], "fr"), "7 août");
  assert.equal(
    datesText(["2026-07-31", "2026-08-03"], "fr"),
    "31 juillet et 3 août",
  );
});

test("the hours: UTC, a shift moves them", () => {
  assert.equal(utcHour("2026-08-03T12:20:00Z"), "12:20");
  assert.equal(shiftInstant("2026-08-03T12:20:00Z", 9), "2026-08-03T21:20:00Z");
  assert.equal(shiftInstant("2026-08-03T15:56:00Z", 9), "2026-08-04T00:56:00Z");
});

test("the calibration of the 10 August borrows the darks of the 9th", () => {
  const items = calibrationItems(proposal, session("2026-08-10"));
  assert.deepEqual(
    items.map((i) => [i.kind, i.frames, i.date]),
    [
      ["dark", 30, "2026-08-09"],
      ["flat", 25, "2026-08-10"],
    ],
  );
  assert.equal(borrowsCalibration(items), true);
  assert.deepEqual(matchCriteria(items), [
    "date",
    "exposure",
    "temperature",
    "gain",
  ]);
  assert.equal(
    borrowsCalibration(calibrationItems(proposal, session("2026-08-09"))),
    false,
  );
});

test("the logs of a night, one entry per kind", () => {
  assert.deepEqual(
    logItems(proposal, session("2026-08-09")).map((i) => [i.kind, i.count]),
    [
      ["asiair", 1],
      ["phd2", 1],
    ],
  );
});

test("the targets without a multi-night plan", () => {
  assert.deepEqual(singleNightTargets(proposal), ["M31", "Sh2-103"]);
});

test("the engine's own user data match the fixture's places and plans", () => {
  assert.equal(DETECT_USER.places.length, 2);
  assert.equal(proposal.places[1].candidates.length, 2);
});

test("a night whose poses name no filter waits for its band, and nothing is supposed", () => {
  const asked = structuredClone(proposal);
  const night = asked.sessions.find((s) => s.eveningDate === "2026-07-31")!;
  night.filterAsk = true;
  for (const target of night.targets) target.filters = [];
  const state = initialState(asked);
  assert.equal(state.filters[night.id], undefined);
  const waiting = nightStatus(asked, state, night);
  assert.equal(waiting.waitsFilter, true);
  assert.equal(waiting.on, false);
  assert.equal(
    tickedIds(asked, state).includes(nightLines(asked)[night.id].session!.id),
    false,
  );

  const answered = chooseFilter(state, night.id, "dualband");
  assert.equal(nightStatus(asked, answered, night).on, true);
  const request = buildRequest(asked, answered);
  const sent = request.proposal.sessions.find((s) => s.id === night.id)!;
  assert.deepEqual(
    sent.targets.map((target) => target.filters),
    night.targets.map(() => ["Dual-band"]),
  );
  // The proposal itself is left as read.
  assert.deepEqual(night.targets[0].filters, []);
  // A night that asks nothing is untouched.
  assert.equal(
    nightStatus(proposal, initialState(proposal), session("2026-07-31"))
      .waitsFilter,
    false,
  );
});

test("a greyed primary names the nights waiting for a band, the primary wakes on the answer", () => {
  const asked = structuredClone(proposal);
  asked.multiNight = [];
  const [first, second] = [...asked.sessions].sort((a, b) =>
    a.eveningDate.localeCompare(b.eveningDate),
  );
  asked.sessions = [second, first];
  for (const night of asked.sessions) {
    night.filterAsk = true;
    for (const target of night.targets) target.filters = [];
  }
  let state = initialState(asked);
  assert.equal(createLabel(asked, state).kind, "none");
  // Oldest first, whatever the order of the proposal.
  assert.deepEqual(
    nightsWaitingBand(asked, state).map((night) => night.id),
    [first.id, second.id],
  );
  state = chooseFilter(state, first.id, "broadband");
  assert.deepEqual(
    nightsWaitingBand(asked, state).map((night) => night.id),
    [second.id],
  );
  assert.notEqual(createLabel(asked, state).kind, "none");
  state = chooseFilter(state, second.id, "dualband");
  assert.equal(nightsWaitingBand(asked, state).length, 0);
  // Taking the answer back puts the night on the list again.
  state = chooseFilter(state, second.id, null);
  assert.equal(nightsWaitingBand(asked, state).length, 1);
});

test("the review follows an offer granted after the detection", () => {
  // Detected on the free account: the plans are locked and nothing multi-night
  // is ticked; the person unticked a night meanwhile.
  let state = initialState(free);
  state = toggleLine(state, sessionLine("2026-08-03").id);
  // The engine followed the account: the same nights, the plans open.
  const unlocked = proposal.multiNight.map((g) => `multiNight.create:${g.id}`);
  const followed = followRights(proposal, state, unlocked);
  for (const id of unlocked) {
    assert.equal(followed.ticked[id], true, `${id} is ticked by default`);
  }
  assert.ok(tickedIds(proposal, followed).includes(unlocked[0]));
  // What the person decided stays.
  assert.equal(
    followed.ticked[sessionLine("2026-08-03").id],
    state.ticked[sessionLine("2026-08-03").id],
  );
  // The offer ends: the plans close and are no longer ticked.
  const back = followRights(free, followed, []);
  assert.ok(!tickedIds(free, back).some((id) => id.startsWith("multiNight")));
  for (const group of free.multiNight) {
    assert.equal(back.ticked[`multiNight.locked:${group.id}`], false);
    assert.equal(back.ticked[`multiNight.create:${group.id}`], undefined);
  }
});

test("a place to create with its name emptied waits for a name: « Continuer » is blocked, and the place is not named in the request", () => {
  const overlap = Object.values(
    nightLines(proposal)[session("2026-08-09").id].targets,
  ).find((line) => line.ask)!;
  const place = proposal.places.find((p) => p.id === "place-3")!;
  let state = initialState(proposal);
  state = answerLine(state, overlap.id, "skip");
  assert.deepEqual(placesWithoutName(proposal, state), []);
  state = choosePlace(proposal, state, place, { kind: "create", name: "Fuji" });
  assert.deepEqual(placesWithoutName(proposal, state), []);
  // Emptied, or only spaces: the place waits for a name.
  for (const name of ["", "   "]) {
    const blank = choosePlace(proposal, state, place, { kind: "create", name });
    assert.deepEqual(
      placesWithoutName(proposal, blank).map((p) => p.id),
      ["place-3"],
    );
    assert.equal(isBlankName(name), true);
    // Never written as a name in the proposal the treatments read.
    const request = buildRequest(proposal, blank);
    assert.equal(
      request.proposal.places.find((p) => p.id === "place-3")!.chosen,
      proposal.places.find((p) => p.id === "place-3")!.chosen,
    );
  }
  // A place whose nights are not used (the choice taken back) never blocks.
  assert.deepEqual(
    placesWithoutName(proposal, choosePlace(proposal, state, place, null)),
    [],
  );
  assert.equal(isBlankName(" Col de l'arrière-pays "), false);
});
