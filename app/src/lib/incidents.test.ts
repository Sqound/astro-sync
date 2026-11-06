import assert from "node:assert/strict";
import test from "node:test";

import {
  PAIR_GAP_SECONDS,
  cut,
  edited,
  fatesOf,
  gapBetween,
  initialTicks,
  merge,
  mergedText,
  pairsOf,
  payloadFor,
  poseRuns,
  quickOf,
  summaryOf,
  withQuick,
  withTick,
  type Genre,
  type Line,
} from "./incidents.ts";
import {
  startRun,
  type FamilyStats,
  type FlaggedPose,
  type PoseMeasure,
  type TargetPoses,
} from "./poseStats.ts";

const T0 = Date.UTC(2026, 7, 14, 15, 0, 0) / 1000; // 00:00 at UTC+9

function line(
  id: string,
  origin: "declared" | "detected",
  genre: Genre,
  minutes: number,
  endMinutes: number | null = null,
  text = id,
): Line {
  return {
    id,
    origin,
    genre,
    start: T0 + minutes * 60,
    end: endMinutes === null ? null : T0 + endMinutes * 60,
    text,
    payload: {},
    evidence: null,
    eventId: origin === "declared" ? id : null,
    recordedAt: null,
  };
}

// The sample night: two pairs, one detected alone, one declared alone.
const LINES: Line[] = [
  line("pause-d", "declared", "pause", 14, 26),
  line("pause-a", "detected", "pause", 13, 25),
  line("lost-2", "detected", "equipment_issue", 117, 118),
  line("issue-d", "declared", "equipment_issue", 182),
  line("issue-a", "detected", "equipment_issue", 172, 190),
  line("sky-d", "declared", "condition", 270),
];

test("two lines of one genre that overlap or lie under ten minutes apart are a pair", () => {
  assert.equal(PAIR_GAP_SECONDS, 600);
  assert.deepEqual(pairsOf(LINES), [
    { declared: "pause-d", detected: "pause-a" },
    { declared: "issue-d", detected: "issue-a" },
  ]);
  // Another genre never pairs, even at the same time.
  assert.deepEqual(
    pairsOf([
      line("a", "declared", "note", 1),
      line("b", "detected", "pause", 1),
    ]),
    [],
  );
  // More than ten minutes apart: no pair.
  assert.deepEqual(
    pairsOf([
      line("a", "declared", "pause", 0),
      line("b", "detected", "pause", 11),
    ]),
    [],
  );
  assert.equal(
    gapBetween(
      line("a", "declared", "pause", 0, 5),
      line("b", "detected", "pause", 4),
    ),
    0,
  );
});

test("each declared line takes the closest detected line, one to one", () => {
  const lines = [
    line("d1", "declared", "pause", 0),
    line("d2", "declared", "pause", 8),
    line("a1", "detected", "pause", 7),
  ];
  assert.deepEqual(pairsOf(lines), [{ declared: "d2", detected: "a1" }]);
});

test("the list is by time, a pair together with its declared line first", () => {
  assert.deepEqual(
    merge([...LINES].reverse()).map((l) => l.id),
    ["pause-d", "pause-a", "lost-2", "issue-d", "issue-a", "sky-d"],
  );
});

test("a detected line alone starts ticked, a pair starts on the declared line", () => {
  const ticks = initialTicks(LINES);
  assert.deepEqual(ticks, {
    "pause-d": true,
    "pause-a": false,
    "lost-2": true,
    "issue-d": true,
    "issue-a": false,
  });
  assert.equal("sky-d" in ticks, false, "a declared line alone is no choice");
  assert.equal(quickOf(LINES, ticks), "mine");
});

test("in a pair one case at least stays ticked, ticking the other gives both", () => {
  let ticks = initialTicks(LINES);
  // Unticking the only ticked case is refused.
  assert.equal(withTick(ticks, LINES, "pause-d", false), ticks);
  ticks = withTick(ticks, LINES, "pause-a", true);
  assert.equal(fatesOf(LINES, ticks)["pause-a"], "alongside");
  ticks = withTick(ticks, LINES, "pause-d", false);
  assert.equal(fatesOf(LINES, ticks)["pause-a"], "replaces");
  assert.equal(fatesOf(LINES, ticks)["pause-d"], "replaced");
  // A line alone can be unticked: it is not added.
  ticks = withTick(ticks, LINES, "lost-2", false);
  assert.equal(fatesOf(LINES, ticks)["lost-2"], "skipped");
  // Not a choice: nothing moves.
  assert.equal(withTick(ticks, LINES, "sky-d", false), ticks);
});

test("the quick choice sets every pair, and goes out when the pairs differ", () => {
  const base = initialTicks(LINES);
  const detections = withQuick(base, LINES, "detections");
  assert.equal(quickOf(LINES, detections), "detections");
  assert.equal(detections["lost-2"], true, "a line alone does not move");
  const both = withQuick(base, LINES, "both");
  assert.equal(quickOf(LINES, both), "both");
  const mixed = withTick(
    withTick(detections, LINES, "issue-d", true),
    LINES,
    "issue-a",
    false,
  );
  assert.equal(quickOf(LINES, mixed), null);
  // The pairs uniform again: the segment lights again.
  assert.equal(quickOf(LINES, withQuick(mixed, LINES, "mine")), "mine");
  // A night not finished cannot replace: « Garder les détections » is refused.
  assert.equal(withQuick(base, LINES, "detections", false), base);
  assert.equal(
    quickOf([line("a", "declared", "note", 0)], {}),
    null,
    "no pair, no command",
  );
});

test("the summary counts what is written and what stays", () => {
  const detections = withQuick(initialTicks(LINES), LINES, "detections");
  assert.deepEqual(summaryOf(LINES, detections), {
    count: 3,
    isolated: 1,
    replace: 2,
    alongside: 0,
    kept: 1,
    declared: 3,
  });
  const both = withQuick(initialTicks(LINES), LINES, "both");
  const summary = summaryOf(LINES, both);
  assert.equal(summary.count, 3);
  assert.equal(summary.alongside, 2);
  assert.equal(summary.kept, 3);
  const mine = summaryOf(LINES, initialTicks(LINES));
  assert.equal(mine.count, 1);
  assert.equal(mine.kept, 3);
});

test("the text is cut on a character, and the person's words come first", () => {
  assert.equal(cut("abc", 5), "abc");
  assert.equal(cut("a😀b", 2), "a😀");
  assert.equal(
    mergedText("Retournement", "Pause détectée", " · détecté : ", 200),
    "Retournement · détecté : Pause détectée",
  );
  assert.equal(mergedText("", "Détecté", " · ", 200), "Détecté");
  assert.equal(mergedText("Même", "Même", " · ", 200), "Même");
  assert.equal(mergedText("x".repeat(250), "y", " · ", 200).length, 200);
});

test("the payload follows the genre and its limit", () => {
  assert.deepEqual(payloadFor("note", "  Vent  "), { text: "Vent" });
  assert.deepEqual(payloadFor("equipment_issue", "Guidage perdu"), {
    description: "Guidage perdu",
    severity: "minor",
  });
  assert.deepEqual(payloadFor("pause", "x".repeat(300)), {
    reason: "x".repeat(200),
  });
  assert.deepEqual(payloadFor("pause", ""), {});
  assert.deepEqual(payloadFor("condition", "Voilé", { sky: "hazy" }), {
    sky: "hazy",
    notes: "Voilé",
  });
  assert.deepEqual(payloadFor("condition", "Aube"), { notes: "Aube" });
});

test("editing a line changes its genre, time, end (a pause only) and payload", () => {
  const original = {
    ...line("a", "detected", "equipment_issue", 172, 190, "Étoile perdue"),
    payload: { description: "Étoile perdue", severity: "minor" },
  };
  const asPause = edited(original, {
    genre: "pause",
    start: original.start,
    end: original.start + 600,
    text: "Guidage",
  });
  assert.equal(asPause.genre, "pause");
  assert.equal(asPause.end, original.start + 600);
  assert.deepEqual(asPause.payload, { reason: "Guidage" });
  assert.equal(asPause.edited, true);
  const asNote = edited(asPause, {
    genre: "note",
    start: original.start,
    end: original.start + 600,
    text: "N",
  });
  assert.equal(asNote.end, null, "only a pause has an end");
  assert.deepEqual(asNote.payload, { text: "N" });
});

function measure(i: number, unix: number): PoseMeasure {
  return {
    path: `/n/p${i}.fit`,
    index: i,
    dateObs: null,
    unix,
    fwhmPixels: 3,
    eccentricity: 0.3,
    starCount: 200,
    skyBackground: 400,
    unreadable: false,
  };
}

test("the poses flagged become runs: one per kind, poses less than ten minutes apart", () => {
  const poses = [0, 1, 2, 3, 4, 5].map((i) =>
    measure(i, T0 + i * 46 + (i >= 3 ? 7200 : 0)),
  );
  const flagged: FlaggedPose[] = [
    { path: poses[0].path, index: 0, reason: { kind: "clouds" } },
    { path: poses[1].path, index: 1, reason: { kind: "clouds" } },
    { path: poses[3].path, index: 3, reason: { kind: "clouds" } },
    { path: poses[4].path, index: 4, reason: { kind: "dawn", factor: 4 } },
    { path: poses[5].path, index: 5, reason: { kind: "dawn", factor: 9.6 } },
  ];
  const family: FamilyStats = {
    label: "f",
    exposureSeconds: 30,
    total: 6,
    measured: poses,
    judged: true,
    medians: {
      fwhmPixels: 3,
      eccentricity: 0.3,
      starCount: 200,
      skyBackground: 400,
    },
    shapeMissing: null,
    flagged,
  };
  const entry: TargetPoses = {
    run: {
      status: "done",
      stats: { families: [family], total: 6, planned: 6, stopped: false },
    },
    plan: null,
    kept: [],
  };
  const runs = poseRuns(entry);
  assert.deepEqual(
    runs.map((r) => [r.kind, r.poses.length]),
    [
      ["clouds", 2],
      ["clouds", 1],
      ["dawn", 2],
    ],
  );
  // A measure still going decides nothing.
  const running: TargetPoses = {
    ...entry,
    run: startRun({ sampleAllUpTo: 200, sampleStep: 4, families: [] }, null, 0),
  };
  assert.deepEqual(poseRuns(running), []);
});
