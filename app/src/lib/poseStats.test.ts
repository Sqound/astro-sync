import assert from "node:assert/strict";
import test from "node:test";

import {
  NO_POSES,
  clockText,
  curvesOf,
  discardTick,
  discardView,
  discardedForRun,
  discardedPaths,
  estimateMinutes,
  exposureText,
  familiesOfPlan,
  flaggedOf,
  offsetText,
  placeOffsetMinutes,
  reasonCountsText,
  reasonText,
  sampleText,
  shownFamily,
  startRun,
  statsOf,
  withAllDiscarded,
  withPoseDiscarded,
  withPoseEvent,
  type FamiliesPlan,
  type FamilyStats,
  type FlaggedPose,
  type PoseEvent,
  type PoseMeasure,
  type PoseStats,
  type TargetPoses,
} from "./poseStats.ts";
import { createTranslator } from "../i18n/locale.ts";

const fr = createTranslator("fr");

const PLAN: FamiliesPlan = {
  sampleAllUpTo: 200,
  sampleStep: 4,
  families: [
    { label: "IRCUT_30s_g200", exposureSeconds: 30, total: 430, planned: 108 },
    { label: "IRCUT_10s_g200", exposureSeconds: 10, total: 36, planned: 36 },
  ],
};

const BASE = Date.UTC(2026, 9, 10, 14, 0, 0) / 1000;

function pose(index: number, extra: Partial<PoseMeasure> = {}): PoseMeasure {
  return {
    path: `/n/M45/lights/b_${String(index).padStart(4, "0")}.fit`,
    index,
    dateObs: null,
    unix: BASE + index * 30,
    fwhmPixels: 3.2,
    eccentricity: 0.31,
    starCount: 240,
    skyBackground: 400,
    unreadable: false,
    ...extra,
  };
}

const MEDIANS = {
  fwhmPixels: 3.2,
  eccentricity: 0.31,
  starCount: 240,
  skyBackground: 400,
};

function flag(index: number, kind: "clouds" | "dawn" = "clouds"): FlaggedPose {
  return {
    path: pose(index).path,
    index,
    reason: kind === "dawn" ? { kind: "dawn", factor: 7 } : { kind: "clouds" },
  };
}

function eventFor(
  index: number,
  done: number,
  flagged: FlaggedPose[] = [],
): PoseEvent {
  return {
    event: "pose",
    family: "IRCUT_30s_g200",
    measure: pose(index),
    done,
    planned: 144,
    judged: done >= 10,
    medians: MEDIANS,
    flagged,
  };
}

function finished(flagged: FlaggedPose[]): TargetPoses {
  const family: FamilyStats = {
    label: "IRCUT_30s_g200",
    exposureSeconds: 30,
    total: 430,
    measured: Array.from({ length: 40 }, (_, i) => pose(i)),
    judged: true,
    medians: MEDIANS,
    shapeMissing: null,
    flagged,
  };
  const stats: PoseStats = {
    families: [family],
    total: 430,
    planned: 108,
    stopped: false,
  };
  return { run: { status: "done", stats }, plan: PLAN, kept: [] };
}

test("the sample is said with the whole families first, then the one in four", () => {
  assert.equal(
    sampleText(fr, "fr", PLAN),
    "Mesure sur un échantillon : les 36 poses de 10 s, puis une pose sur 4 pour les 430 poses de 30 s (108 poses), et les voisines de toute pose qui s’écarte. Environ 2 min.",
  );
  const small: FamiliesPlan = {
    ...PLAN,
    families: [{ label: "Ha", exposureSeconds: 180, total: 40, planned: 40 }],
  };
  assert.equal(
    sampleText(fr, "fr", small),
    "Toutes les poses sont mesurées : les 40 poses de 180 s. Environ 1 min.",
  );
  assert.equal(estimateMinutes(1), 1);
  assert.equal(estimateMinutes(144), 2);
});

test("exposures are written in the language", () => {
  assert.equal(exposureText(fr, "fr", 10), "10 s");
  assert.equal(exposureText(fr, "fr", 1.5), "1,5 s");
  assert.equal(exposureText(fr, "fr", null), "—");
});

test("the events file the poses by family, in order of time, and the count grows", () => {
  let run = startRun(PLAN, null, 1000);
  assert.equal(run.status, "running");
  assert.equal(run.status === "running" && run.planned, 144);
  run = withPoseEvent(run, eventFor(8, 1));
  run = withPoseEvent(run, eventFor(4, 2));
  run = withPoseEvent(run, eventFor(4, 2)); // the same pose twice: once
  assert.ok(run.status === "running");
  const thirty = run.families.find((f) => f.label === "IRCUT_30s_g200")!;
  assert.deepEqual(
    thirty.measured.map((m) => m.index),
    [4, 8],
  );
  assert.equal(run.done, 2);
  run = withPoseEvent(run, { event: "neighbours", count: 12, planned: 156 });
  assert.ok(run.status === "running" && run.planned === 156);
  // An event outside a run changes nothing.
  assert.deepEqual(withPoseEvent({ status: "idle" }, eventFor(1, 1)), {
    status: "idle",
  });
});

test("a resumed run starts from what was kept", () => {
  const before = finished([]);
  const stats = statsOf(before)!;
  const run = startRun(PLAN, stats, 5);
  assert.ok(run.status === "running");
  assert.equal(run.done, 40);
  assert.equal(statsOf({ ...NO_POSES, run })!.families[0].measured.length, 40);
  assert.equal(familiesOfPlan(PLAN)[1].total, 36);
});

test("the poses flagged are left out unless the person keeps them", () => {
  const entry = finished([flag(10), flag(11), flag(12, "dawn")]);
  assert.equal(discardedPaths(entry).length, 3);
  assert.equal(discardTick(entry), "all");
  const some = withPoseDiscarded(entry, flag(11).path, false);
  assert.equal(discardedPaths(some).length, 2);
  assert.equal(discardTick(some), "some");
  const none = withAllDiscarded(some, false);
  assert.equal(discardedPaths(none).length, 0);
  assert.equal(discardTick(none), "none");
  const back = withAllDiscarded(none, true);
  assert.equal(discardTick(back), "all");
  assert.deepEqual(discardedForRun(none), null);
  assert.equal(discardedForRun(entry)?.length, 3);
  // A measure that is not finished decides nothing.
  const running: TargetPoses = {
    ...entry,
    run: startRun(PLAN, null, 0),
  };
  assert.deepEqual(discardedPaths(running), []);
  assert.deepEqual(discardedPaths(NO_POSES), []);
});

test("what « Traitements » says in each case", () => {
  assert.deepEqual(discardView(NO_POSES, 430), {
    kind: "unmeasured",
    poses: 430,
  });
  const flagged = finished([flag(10)]);
  assert.equal(discardView(flagged, 430).kind, "flagged");
  const quiet = finished([]);
  assert.equal(discardView(quiet, 430).kind, "none");
  const stopped: TargetPoses = {
    ...quiet,
    run: {
      status: "stopped",
      stats: { ...statsOf(quiet)!, stopped: true },
    },
  };
  assert.deepEqual(discardView(stopped, 430), {
    kind: "partial",
    measured: 40,
    planned: 108,
  });
  const shapeless = finished([]);
  const stats = statsOf(shapeless)!;
  stats.families[0].shapeMissing = 31;
  assert.equal(discardView(shapeless, 40).kind, "few");
});

test("the reasons are said, and counted by kind", () => {
  assert.equal(
    reasonText(fr, { kind: "dawn", factor: 7.2 }),
    "fond de ciel 7 fois plus haut : aube",
  );
  assert.equal(
    reasonText(fr, { kind: "clouds" }),
    "peu d’étoiles : nuage probable",
  );
  const flagged: FlaggedPose[] = [
    flag(1, "dawn"),
    { ...flag(2, "dawn"), reason: { kind: "dawn", factor: 10 } },
    flag(3),
    flag(4),
    flag(5),
    { path: "x", index: 6, reason: { kind: "trailed" } },
  ];
  assert.equal(
    reasonCountsText(fr, flagged),
    "2 à l’aube, 3 sous un nuage probable, 1 aux étoiles allongées",
  );
  assert.equal(
    reasonCountsText(fr, flagged, true, " · "),
    "2 à l’aube (fond de ciel jusqu’à 10 fois plus haut) · 3 sous un nuage probable · 1 aux étoiles allongées",
  );
  assert.equal(flaggedOf(null).length, 0);
});

test("the hours are the place's, from its zone, else from its longitude", () => {
  const zone = (name: string): number | null =>
    name === "Asia/Tokyo" ? 540 : null;
  assert.equal(placeOffsetMinutes("Asia/Tokyo", 139.7, BASE, zone), 540);
  assert.equal(placeOffsetMinutes("Nowhere/Zone", 139.7, BASE, zone), 540);
  assert.equal(placeOffsetMinutes(null, 7.4, BASE, zone), 0);
  assert.equal(placeOffsetMinutes(null, -46.6, BASE, zone), -180);
  assert.equal(placeOffsetMinutes(null, null, BASE, zone), 0);
  assert.equal(clockText(BASE + 3 * 3600, 540), "02:00");
  assert.equal(offsetText(540), "UTC+9");
  assert.equal(offsetText(330), "UTC+5:30");
  assert.equal(offsetText(-180), "UTC−3");
  assert.equal(offsetText(0), "UTC");
});

test("the curves: the time axis, the ticks in local hours, the flagged points", () => {
  const poses = Array.from({ length: 60 }, (_, i) =>
    pose(i, i === 30 ? { starCount: 40 } : {}),
  );
  const family: FamilyStats = {
    label: "x",
    exposureSeconds: 30,
    total: 60,
    measured: poses,
    judged: true,
    medians: MEDIANS,
    shapeMissing: null,
    flagged: [flag(30)],
  };
  const curves = curvesOf(family, 540);
  assert.deepEqual(
    curves.curves.map((c) => c.metric),
    ["fwhm", "eccentricity", "stars", "sky"],
  );
  const stars = curves.curves.find((c) => c.metric === "stars")!;
  assert.equal(stars.bottom, 0);
  assert.ok(stars.top >= 240 && stars.top <= 300, `top ${stars.top}`);
  assert.equal(stars.points.filter((p) => p.flagged).length, 1);
  assert.equal(stars.points[0].x, 0);
  assert.ok(Math.abs(stars.points[59].x - 1) < 1e-9);
  // 30 minutes of poses: a tick every quarter hour, in the place's hours
  // (14:00 UTC is 23:00 at UTC+9).
  assert.deepEqual(
    curves.ticks.map((t) => t.label),
    ["23:00", "23:15"],
  );
  assert.equal(curves.unmeasured, null);
  // A family without a width median draws the stars and the sky only.
  const shapeless = curvesOf(
    {
      ...family,
      medians: { ...MEDIANS, fwhmPixels: null, eccentricity: null },
    },
    0,
  );
  assert.deepEqual(
    shapeless.curves.map((c) => c.metric),
    ["stars", "sky"],
  );
  // A stopped measure leaves the end of the family hatched.
  const partial = curvesOf(
    { ...family, measured: poses.slice(0, 20), total: 60, flagged: [] },
    0,
    { start: BASE, end: BASE + 59 * 30 },
  );
  assert.ok(partial.unmeasured !== null);
  assert.ok(partial.unmeasured!.from > 0.3 && partial.unmeasured!.from < 0.4);
});

test("the family shown is the chosen one, else the first with poses flagged", () => {
  const entry = finished([flag(3)]);
  const stats = statsOf(entry)!;
  const second: FamilyStats = {
    ...stats.families[0],
    label: "other",
    flagged: [],
  };
  const both: PoseStats = { ...stats, families: [second, stats.families[0]] };
  assert.equal(shownFamily(both, null)?.label, "IRCUT_30s_g200");
  assert.equal(shownFamily(both, "other")?.label, "other");
  assert.equal(shownFamily({ ...both, families: [] }, null), null);
});
