import assert from "node:assert/strict";
import test from "node:test";

import type { OrganiseAction, OrganisePlan } from "./bridge.ts";
import {
  emphasise,
  isDestructive,
  modeAvailable,
  organiseCounts,
  organiseEnd,
  organisePercent,
  organisePrimary,
  organiseTree,
  organiseTreeText,
  reasonKey,
  sourceFolder,
} from "./organise.ts";

const ROOT = "/home/person/Documents/Astro Sync";

function action(source: string, under: string): OrganiseAction {
  return { source, destination: `${ROOT}/${under}`, kind: "copy", bytes: 10 };
}

const PLAN: OrganisePlan = {
  mode: "copy",
  destination: ROOT,
  actions: [
    action("/data/ASIAIR/a1.fit", "2026-08-11/IC 1805/lights/a1.fit"),
    action("/data/ASIAIR/a2.fit", "2026-08-11/IC 1805/lights/a2.fit"),
    action("/data/ASIAIR/b1.fit", "2026-08-11/Sh2-103/lights/b1.fit"),
    action("/data/ASIAIR/d1.fit", "2026-08-11/calibration/flats/d1.fit"),
    action("/data/ASIAIR/d2.fit", "2026-08-11/calibration/darks/d2.fit"),
    action("/data/ASIAIR/l1.log", "2026-08-11/logs/l1.log"),
    action("/data/ASIAIR/m1.fit", "2026-07-31/M31/lights/m1.fit"),
    action("/data/ASIAIR/h1.fit", "2026-09-02/IC 1805/lights/Ha/h1.fit"),
    action("/data/ASIAIR/o1.fit", "2026-09-02/IC 1805/lights/OIII/o1.fit"),
    action("/data/ASIAIR/o2.fit", "2026-09-02/IC 1805/lights/OIII/o2.fit"),
  ],
  conflicts: [],
  files: 10,
  bytes: 100,
  windowsNoLink: false,
  summary: "",
};

test("the primary follows the mode and only « Déplacer » is destructive", () => {
  assert.deepEqual(organisePrimary("leave-in-place"), {
    key: "organise.done",
    counted: false,
    destructive: false,
  });
  assert.equal(organisePrimary("copy").key, "organise.copy");
  assert.equal(organisePrimary("symlink").key, "organise.link");
  assert.equal(organisePrimary("move").destructive, true);
  assert.equal(isDestructive("move"), true);
  assert.equal(isDestructive("copy"), false);
  assert.equal(isDestructive("symlink"), false);
});

test("only the link is held, and only without developer mode on Windows", () => {
  assert.equal(modeAvailable("symlink", true), false);
  assert.equal(modeAvailable("symlink", false), true);
  assert.equal(modeAvailable("move", true), true);
});

test("the counts come from where the plan puts the files", () => {
  assert.deepEqual(organiseCounts(PLAN), {
    frames: 7,
    calibration: 2,
    logs: 1,
  });
});

test("the tree has a filter level only where the plan has one, latest night first", () => {
  const nights = organiseTree(PLAN);
  assert.deepEqual(
    nights.map((night) => night.name),
    ["2026-09-02", "2026-08-11", "2026-07-31"],
  );
  const mono = nights[0].targets[0];
  assert.deepEqual(mono.filters, [
    { name: "Ha", frames: 1 },
    { name: "OIII", frames: 2 },
  ]);
  assert.equal(mono.frames, 3);
  const colour = nights[1];
  assert.deepEqual(
    colour.targets.map((t) => t.filters.length),
    [0, 0],
  );
  assert.deepEqual(colour.calibration, ["darks", "flats"]);
  assert.equal(colour.logs, 1);
});

test("the tree text shows lights/ alone, calibration by type, logs, and the nights left", () => {
  const text = organiseTreeText(
    PLAN,
    {
      poses: (n) => `${n} poses`,
      moreNights: (n) => `${n} autres nuits`,
    },
    2,
  );
  const lines = text.split("\n");
  assert.equal(lines[0], `${ROOT}/`);
  assert.equal(lines[1], "├─ 2026-09-02/");
  assert.match(lines[2], /IC 1805\/lights\/Ha\/ {2}lights\/OIII\/ +3 poses$/);
  assert.equal(lines[3], "├─ 2026-08-11/");
  assert.match(lines[4], /^│ {2}├─ IC 1805\/lights\/ +2 poses$/);
  assert.match(lines[5], /^│ {2}├─ Sh2-103\/lights\/ +1 poses$/);
  assert.equal(lines[6], "│  ├─ calibration/darks/ flats/");
  assert.equal(lines[7], "│  └─ logs/");
  assert.equal(lines[8], "└─ 1 autres nuits");
  assert.equal(lines.length, 9);
});

test("the source folder is the deepest one all the sources share", () => {
  assert.equal(sourceFolder(PLAN), "/data/ASIAIR");
  assert.equal(sourceFolder({ ...PLAN, actions: [] }), "");
});

test("a sentence is cut around the folders it names", () => {
  assert.deepEqual(
    emphasise("dans calibration/ et logs/ de la nuit", [
      "calibration/",
      "logs/",
    ]),
    [
      { text: "dans ", code: false },
      { text: "calibration/", code: true },
      { text: " et ", code: false },
      { text: "logs/", code: true },
      { text: " de la nuit", code: false },
    ],
  );
  assert.deepEqual(emphasise("rien", []), [{ text: "rien", code: false }]);
});

test("a failure says why by a known reason, io for the others", () => {
  assert.equal(
    reasonKey("checksum-differs"),
    "organise.reason.checksum-differs",
  );
  assert.equal(reasonKey("same-name"), "organise.reason.same-name");
  assert.equal(reasonKey("whatever"), "organise.reason.io");
});

test("the end names what was done by the mode, or that it stopped", () => {
  const outcome = {
    done: 998,
    verified: 998,
    removedSources: 998,
    alreadyThere: 0,
    failed: [{ source: "a", destination: "b", reason: "checksum-differs" }],
    stopped: false,
  };
  assert.deepEqual(organiseEnd("move", outcome), {
    kind: "moved",
    count: 998,
    failed: 1,
  });
  assert.equal(organiseEnd("copy", outcome).kind, "copied");
  assert.equal(organiseEnd("symlink", outcome).kind, "linked");
  assert.equal(
    organiseEnd("move", { ...outcome, stopped: true }).kind,
    "stopped",
  );
});

test("the bar follows the bytes, then the files", () => {
  assert.equal(
    organisePercent({
      done: 312,
      total: 1001,
      bytesDone: 5.6,
      bytesTotal: 17.6,
    }),
    32,
  );
  assert.equal(
    organisePercent({ done: 1, total: 4, bytesDone: 0, bytesTotal: 0 }),
    25,
  );
  assert.equal(
    organisePercent({ done: 0, total: 0, bytesDone: 0, bytesTotal: 0 }),
    0,
  );
});

test("the image a device stacked goes to device/ of its target, shown in the tree, never counted as a pose", () => {
  const plan: OrganisePlan = {
    ...PLAN,
    actions: [
      action("/data/S/l1.fit", "2026-10-10/M45/lights/IRCUT/l1.fit"),
      action("/data/S/l2.fit", "2026-10-10/M45/lights/IRCUT/l2.fit"),
      action("/data/S/Stacked_2.fit", "2026-10-10/M45/device/Stacked_2.fit"),
    ],
  };
  assert.deepEqual(organiseCounts(plan), {
    frames: 2,
    calibration: 0,
    logs: 0,
  });
  const [night] = organiseTree(plan);
  assert.deepEqual(
    night.targets.map((t) => [t.name, t.frames, t.device]),
    [["M45", 2, 1]],
  );
  const text = organiseTreeText(plan, {
    poses: (n) => `${n} poses`,
    moreNights: (n) => `${n} nuits`,
  });
  assert.ok(text.includes("M45/lights/IRCUT/"), text);
  assert.ok(text.includes("M45/device/"), text);
  // A target that has only the image: its device/ line alone.
  const alone = organiseTreeText(
    { ...plan, actions: [plan.actions[2]] },
    { poses: (n) => `${n} poses`, moreNights: (n) => `${n}` },
  );
  assert.ok(alone.includes("M45/device/") && !alone.includes("lights/"), alone);
});
