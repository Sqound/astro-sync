import assert from "node:assert/strict";
import test from "node:test";

import type { OrganisePlan, Proposal } from "./bridge.ts";
import {
  buildRequest,
  buildScope,
  calibrationOf,
  framesFolder,
  hasLocation,
  inPlaceTargets,
  intoNightFolder,
  loadLocations,
  logsOf,
  modeSentence,
  NO_LOCATIONS,
  opensByItself,
  organiseProposal,
  relativeTo,
  saveLocations,
  sheetModel,
  treeFolders,
  utcTime,
} from "./wizardDetect.ts";

const ROOT = "/Astro/ASIAIR/Autorun/Light";

function detected(
  key: string,
  designation: string,
  files: string[],
  start: string,
  end: string,
) {
  return {
    key,
    targetId: null,
    designation,
    name: null,
    source: { by: "object" },
    objects: [designation],
    start,
    end,
    frames: files.length,
    exposureSeconds: 180,
    filters: [],
    acquisition: [
      {
        filter: null,
        exposureSeconds: 180,
        gain: 100,
        frames: files.length,
        totalSeconds: files.length * 180,
      },
    ],
    instrument: "ASI533MC Pro",
    files,
    existing: "new",
  };
}

const IC = [1, 2, 3].map((n) => `${ROOT}/IC 1805/Light_${n}.fit`);
const NGC = [1, 2].map((n) => `${ROOT}/NGC 7000/Light_${n}.fit`);
const LOOSE = [`${ROOT}/Light_loose.fit`];

function proposal(): Proposal {
  return {
    reading: {
      filesRead: 0,
      lights: 6,
      calibrationFrames: 0,
      logFiles: 0,
      nights: 1,
      targets: 2,
    },
    places: [],
    equipment: [],
    sessions: [
      {
        id: "s-2026-08-10",
        eveningDate: "2026-08-10",
        writtenEveningDate: "2026-08-10",
        place: "p",
        equipment: [],
        start: "2026-08-10T11:45:00Z",
        end: "2026-08-10T16:46:00Z",
        targets: [
          detected(
            "ic",
            "IC 1805",
            IC,
            "2026-08-10T11:45:00Z",
            "2026-08-10T14:00:00Z",
          ),
          detected(
            "ngc",
            "NGC 7000",
            NGC,
            "2026-08-10T16:10:00Z",
            "2026-08-10T16:46:00Z",
          ),
          detected(
            "none",
            "?",
            LOOSE,
            "2026-08-10T18:52:00Z",
            "2026-08-10T18:58:00Z",
          ),
        ],
        shiftHours: 0,
        calibration: [
          { setId: "c1", kind: "dark", daysAway: 0, matchedOn: [] },
          { setId: "c2", kind: "flat", daysAway: 0, matchedOn: [] },
        ],
        logs: [],
        existing: { kind: "create" },
      },
    ],
    multiNight: [],
    calibrationSets: [
      {
        id: "c1",
        kind: "dark",
        kindFromFolder: false,
        folder: "d",
        frames: 30,
        exposureSeconds: 180,
        gain: 100,
        temperatureC: -10,
        filter: null,
        instrument: null,
        start: null,
        end: null,
        files: ["/d/1.fit"],
      },
      {
        id: "c2",
        kind: "flat",
        kindFromFolder: false,
        folder: "f",
        frames: 25,
        exposureSeconds: 1,
        gain: 100,
        temperatureC: -10,
        filter: null,
        instrument: null,
        start: null,
        end: null,
        files: ["/f/1.fit"],
      },
      {
        id: "c3",
        kind: "bias",
        kindFromFolder: false,
        folder: "b",
        frames: 50,
        exposureSeconds: 0,
        gain: 100,
        temperatureC: -10,
        filter: null,
        instrument: null,
        start: null,
        end: null,
        files: [],
      },
    ],
    logs: [
      {
        id: "l1",
        path: "/L/Autorun_Log.txt",
        kind: "asiair",
        utcOffset: null,
        start: null,
        end: null,
        matchedFrames: 4,
        sessions: ["s-2026-08-10"],
      },
      {
        id: "l2",
        path: "/L/nina.log",
        kind: "nina",
        utcOffset: null,
        start: null,
        end: null,
        matchedFrames: 4,
        sessions: ["s-2026-08-10"],
      },
      {
        id: "l3",
        path: "/L/old.txt",
        kind: "phd2",
        utcOffset: null,
        start: null,
        end: null,
        matchedFrames: 0,
        sessions: [],
      },
    ],
    setAside: {
      withoutDate: [],
      unreadable: [],
      stacks: [],
      calibrationUnattached: [],
      logsUnattached: [],
      otherFiles: [],
    },
    scoped: {
      targets: [{ id: "t-ic", designation: "IC 1805", files: IC }],
      unassigned: [
        { key: "ngc", designation: "NGC 7000", targetId: "t-ngc", files: NGC },
      ],
      withoutObject: LOOSE,
      otherNights: 96,
    },
    lines: [],
    notes: [],
  } as unknown as Proposal;
}

test("the scope is the session's range and its targets with what the catalogue knows", () => {
  const scope = buildScope(
    { startedAt: "2026-08-10T11:40:00Z", finishedAt: "2026-08-10T19:10:00Z" },
    {
      sessionId: "s",
      targets: [
        { targetId: "t-ic", name: "IC 1805", planned: true, captures: [] },
        { targetId: null, name: "Orphan", planned: true, captures: [] },
      ],
    } as never,
    { "t-ic": { aliases: ["Heart Nebula"], raHours: 2.5, decDegrees: 61.4 } },
  );
  assert.equal(scope.from, "2026-08-10T11:40:00.000Z");
  assert.equal(scope.to, "2026-08-10T19:10:00.000Z");
  assert.equal(scope.targets.length, 1);
  assert.deepEqual(scope.targets[0], {
    id: "t-ic",
    designation: "IC 1805",
    aliases: ["Heart Nebula"],
    raHours: 2.5,
    decDegrees: 61.4,
  });
});

test("a session without a finish is twelve hours long", () => {
  const scope = buildScope(
    { startedAt: "2026-08-10T11:40:00Z", finishedAt: null },
    null,
  );
  assert.equal(scope.to, "2026-08-10T23:40:00.000Z");
  assert.deepEqual(scope.targets, []);
});

test("the request carries the locations, no calibration folder, the scope", () => {
  const scope = buildScope(
    { startedAt: "2026-08-10T11:40:00Z", finishedAt: null },
    null,
  );
  const request = buildRequest(
    { fits: "/F", logs: null, subfolders: false },
    scope,
  );
  assert.deepEqual(request.fits, ["/F"]);
  assert.deepEqual(request.logs, []);
  assert.deepEqual(request.calibration, []);
  assert.equal(request.subfolders, false);
  assert.equal(request.scope, scope);
});

test("a location asks the sheet to open once, until it is answered", () => {
  assert.equal(hasLocation(NO_LOCATIONS), false);
  assert.equal(opensByItself(NO_LOCATIONS, false), false);
  assert.equal(opensByItself({ ...NO_LOCATIONS, logs: "/L" }, false), true);
  assert.equal(opensByItself({ ...NO_LOCATIONS, fits: "/F" }, true), false);
});

test("the locations are kept by session on this computer", () => {
  const store = new Map<string, string>();
  const storage = {
    getItem: (key: string) => store.get(key) ?? null,
    setItem: (key: string, value: string) => void store.set(key, value),
  };
  saveLocations(storage, "a", { fits: "/F", logs: "/L", subfolders: false });
  assert.deepEqual(loadLocations(storage, "a"), {
    fits: "/F",
    logs: "/L",
    subfolders: false,
  });
  assert.deepEqual(loadLocations(storage, "b"), NO_LOCATIONS);
  store.set("astro-sync.sendLocations:c", "not json");
  assert.deepEqual(loadLocations(storage, "c"), NO_LOCATIONS);
  assert.deepEqual(loadLocations(null, "a"), NO_LOCATIONS);
});

test("the sheet lists the session's targets with their frames and sets the rest apart", () => {
  const model = sheetModel(proposal(), ROOT);
  assert.ok(model);
  assert.equal(model.targets.length, 1);
  const ic = model.targets[0];
  assert.equal(ic.frames, 3);
  assert.deepEqual(ic.exposures, [180]);
  assert.equal(ic.totalSeconds, 540);
  assert.equal(ic.files[0].name, "IC 1805/Light_1.fit");
  assert.equal(model.frames, 3);
  assert.equal(model.unassigned.length, 1);
  assert.equal(model.unassigned[0].designation, "NGC 7000");
  assert.deepEqual(model.unassigned[0].span, {
    start: "2026-08-10T16:10:00Z",
    end: "2026-08-10T16:46:00Z",
  });
  assert.equal(model.withoutObject?.frames, 1);
  assert.equal(model.otherNights, 96);
});

test("a detection without a scope has no sheet", () => {
  const bare = { ...proposal(), scoped: null };
  assert.equal(sheetModel(bare, ROOT), null);
});

test("the calibration of the night counts the sets attached, by kind, in order", () => {
  assert.deepEqual(calibrationOf(proposal()), [
    { kind: "dark", frames: 30 },
    { kind: "flat", frames: 25 },
  ]);
  const unattached = proposal();
  unattached.sessions[0].calibration = [];
  assert.deepEqual(
    calibrationOf(unattached).map((item) => item.kind),
    ["dark", "flat", "bias"],
  );
});

test("the logs are the ones tied to the night, with their readers in order", () => {
  const logs = logsOf(proposal());
  assert.equal(logs.files.length, 2);
  assert.deepEqual(logs.readers, ["asiair", "nina"]);
});

test("the tidying keeps the session's targets only", () => {
  const kept = organiseProposal(proposal());
  const names = kept.sessions[0].targets.map((target) => target.designation);
  assert.deepEqual(names, ["IC 1805"]);
  assert.equal(kept.sessions[0].targets[0].frames, 3);
  assert.equal(kept.calibrationSets.length, 3);
});

test("the plan goes into the night's folder, its first level dropped", () => {
  const plan: OrganisePlan = {
    mode: "copy",
    destination: "/N",
    actions: [
      {
        source: "/a/1.fit",
        destination: "/N/2026-08-10/IC 1805/lights/1.fit",
        kind: "copy",
        bytes: 1,
      },
      {
        source: "/a/2.fit",
        destination: "/N/2026-08-10/IC 1805/lights/Ha/2.fit",
        kind: "copy",
        bytes: 1,
      },
      {
        source: "/d/1.fit",
        destination: "/N/2026-08-10/calibration/darks/1.fit",
        kind: "copy",
        bytes: 1,
      },
      {
        source: "/L/x.log",
        destination: "/N/2026-08-10/logs/x.log",
        kind: "copy",
        bytes: 1,
      },
    ],
    conflicts: [],
    files: 4,
    bytes: 4,
    windowsNoLink: false,
    summary: "",
  };
  const moved = intoNightFolder(plan, "/Docs/2026-08-10 · Maison · 9b02f1");
  assert.equal(moved.destination, "/Docs/2026-08-10 · Maison · 9b02f1");
  assert.deepEqual(
    moved.actions.map((action) => action.destination),
    [
      "/Docs/2026-08-10 · Maison · 9b02f1/IC 1805/lights/1.fit",
      "/Docs/2026-08-10 · Maison · 9b02f1/IC 1805/lights/Ha/2.fit",
      "/Docs/2026-08-10 · Maison · 9b02f1/calibration/darks/1.fit",
      "/Docs/2026-08-10 · Maison · 9b02f1/logs/x.log",
    ],
  );
  assert.deepEqual(
    moved.actions.map((action) => action.source),
    plan.actions.map((action) => action.source),
  );
  assert.deepEqual(treeFolders(plan), [
    "IC 1805/lights/",
    "IC 1805/lights/Ha/",
    "calibration/",
    "logs/",
  ]);
});

test("the sentence under the modes follows the choice, the move warns", () => {
  assert.equal(modeSentence("leave-in-place").key, "wizard.detect.mode.leave");
  assert.equal(modeSentence("copy").tone, "success");
  assert.equal(modeSentence("move").tone, "warning");
  assert.equal(modeSentence("symlink").key, "wizard.detect.mode.link");
});

test("the frames read in place come from the folder they share", () => {
  assert.equal(framesFolder(IC), `${ROOT}/IC 1805`);
  assert.equal(framesFolder([]), "");
  const read = inPlaceTargets(proposal());
  assert.deepEqual(read["t-ic"], {
    folder: `${ROOT}/IC 1805`,
    frames: 3,
    exposures: [180],
    gains: [100],
  });
  assert.equal(read["t-ngc"], undefined);
});

test("paths are shown under the folder read, the name alone elsewhere", () => {
  assert.equal(relativeTo(`${ROOT}/a/b.fit`, ROOT), "a/b.fit");
  assert.equal(relativeTo("/x/y/b.fit", ROOT), "b.fit");
  assert.equal(utcTime("2026-08-10T11:45:00Z", "fr"), "11:45");
});
