import assert from "node:assert/strict";
import test from "node:test";

import type {
  BulkCreateOutcome,
  CreatedSession,
  DetectedIncident,
  Proposal,
} from "./bridge.ts";
import {
  commonDir,
  importedSessions,
  nightFolderOf,
  relativeTo,
  sessionSegment,
  shiftIncident,
} from "./incidentsImport.ts";

const HOME = "/home/marie/Astro";

test("the common folder of paths, written either way, and a root is never a night", () => {
  assert.equal(commonDir([`${HOME}/a/x.fit`, `${HOME}/a/y.fit`]), `${HOME}/a`);
  assert.equal(commonDir([`${HOME}/a/x.fit`, `${HOME}/b/y.fit`]), HOME);
  assert.equal(
    commonDir(["C:\\Astro\\M45\\a.fit", "C:\\Astro\\M45\\b.fit"]),
    "C:/Astro/M45",
  );
  assert.equal(commonDir(["/a/x.fit", "/b/y.fit"]), "");
  assert.equal(relativeTo(HOME, `${HOME}/ASIAIR/Light`), "ASIAIR/Light");
  assert.equal(relativeTo(HOME, HOME), "");
  assert.equal(relativeTo(HOME, "/home/marie/Other"), null);
  assert.equal(
    nightFolderOf(["/a/x.fit", "/b/y.fit"], []),
    null,
    "frames of two disks",
  );
});

test("the folder read widens to the logs, two levels at most, never to a root", () => {
  const frames = [
    `${HOME}/Autorun/Light/Sadr/a.fit`,
    `${HOME}/Autorun/Light/Sadr/b.fit`,
  ];
  assert.equal(nightFolderOf(frames, []), `${HOME}/Autorun/Light/Sadr`);
  assert.equal(
    nightFolderOf(frames, [`${HOME}/Autorun/Log/Autorun_Log.txt`]),
    `${HOME}/Autorun`,
  );
  // A log elsewhere on the disk: the frames' folder is widened twice and no more.
  assert.equal(
    nightFolderOf(frames, ["/mnt/backup/log.txt"]),
    `${HOME}/Autorun`,
  );
  // It stops below the root of the disk.
  assert.equal(
    nightFolderOf(["/Astro/Light/a.fit"], ["/Backup/log.txt"]),
    "/Astro",
  );
});

/* A proposal with two nights, a night with a clock shift, and sessions the
 * import only added frames to. */
const night = (
  id: string,
  date: string,
  designation: string,
  dir: string,
  extra: Record<string, unknown> = {},
) => ({
  id,
  eveningDate: date,
  writtenEveningDate: date,
  place: "p1",
  start: `${date}T20:00:00Z`,
  end: `${date}T23:00:00Z`,
  targets: [
    {
      key: `object:${designation}`,
      targetId: `t-${designation}`,
      designation,
      files: [`${dir}/a.fit`, `${dir}/b.fit`],
    },
  ],
  shiftHours: 0,
  logs: [],
  ...extra,
});
const proposal = {
  places: [
    {
      id: "p1",
      longitude: 139.99,
      chosen: "k1",
      candidates: [{ placeId: "k1", name: "Maison", distanceKm: 1 }],
    },
  ],
  logs: [
    { id: "l1", path: `${HOME}/ASIAIR/Autorun/Log/log.txt`, kind: "asiair" },
  ],
  sessions: [
    night("s-m45", "2026-10-10", "M45", `${HOME}/Seestar/M 45_sub`),
    night("s-sadr", "2026-08-14", "Sadr", `${HOME}/ASIAIR/Autorun/Light/Sadr`, {
      logs: [{ logId: "l1", targets: [], matchedFrames: 3 }],
    }),
    night("s-shift", "2026-08-03", "NGC 7000", `${HOME}/Shifted`, {
      eveningDate: "2026-08-02",
      shiftHours: -9,
      writtenEveningDate: "2026-08-03",
    }),
    night("s-old", "2026-08-01", "M31", `${HOME}/Old`),
    night("s-root", "2026-08-05", "M13", "/lost", {
      targets: [
        {
          key: "object:m13",
          targetId: "t-m13",
          designation: "M13",
          files: ["/lost/a.fit", "/other/b.fit"],
        },
      ],
    }),
  ],
  multiNight: [],
} as unknown as Proposal;

const made = (
  detectedId: string,
  sessionId: string,
  action: CreatedSession["action"],
  eveningDate: string,
  designations: string[],
): CreatedSession => ({
  detectedId,
  sessionId,
  eveningDate,
  action,
  planAction: "created",
  planId: null,
  designations,
  frames: 2,
  shiftHours: 0,
});
const outcome: BulkCreateOutcome = {
  sessions: [
    made("s-m45", "created-m45", "created", "2026-10-10", ["M45"]),
    made("s-sadr", "created-sadr", "created", "2026-08-14", ["Sadr"]),
    {
      ...made("s-shift", "created-shift", "created", "2026-08-02", [
        "NGC 7000",
      ]),
      shiftHours: -9,
    },
    made("s-old", "existing-old", "updated", "2026-08-01", ["M31"]),
    made("s-root", "created-root", "created", "2026-08-05", ["M13"]),
  ],
  multiNight: [],
  placesCreated: [],
};

test("one card per session the import made or added poses to; the ones with nothing to read are not there", () => {
  const sessions = importedSessions(proposal, outcome.sessions);
  assert.deepEqual(
    sessions.map((s) => s.sessionId),
    ["created-m45", "created-sadr", "created-shift", "existing-old"],
  );
  assert.deepEqual(
    sessions.map((s) => s.action),
    ["created", "created", "created", "updated"],
  );
});

test("a session the import only added poses to is read like the others, without the words of a session it made", () => {
  const old = importedSessions(proposal, outcome.sessions)[3];
  assert.equal(old.night.nightFolder, `${HOME}/Old`);
  assert.equal(
    old.night.imported,
    false,
    "something may have been declared on it",
  );
  assert.equal(old.night.noMeasure, true, "the poses are not measured here");
  assert.equal(old.night.finished, false, "unknown until the space says");
  const finished = importedSessions(
    proposal,
    outcome.sessions,
    new Set(["existing-old"]),
  )[3];
  assert.equal(
    finished.night.finished,
    true,
    "a finished session can have a signalement replaced",
  );
  // The targets the import did not attach to the session are not read.
  const two = {
    ...proposal,
    sessions: proposal.sessions.map((s) =>
      s.id !== "s-old"
        ? s
        : {
            ...s,
            targets: [
              ...s.targets,
              {
                key: "object:ngc",
                targetId: "t-ngc",
                designation: "NGC 7000",
                files: [`${HOME}/Old/other/c.fit`],
              },
            ],
          },
    ),
  } as unknown as Proposal;
  const only = importedSessions(two, outcome.sessions)[3];
  assert.deepEqual(only.night.targets, [{ targetId: "t-M31", folderName: "" }]);
  assert.equal(only.targetName, "M31");
});

test("each session is read from its own folders, on its own evening, as a night already carried out", () => {
  const [m45, sadr] = importedSessions(proposal, outcome.sessions);
  assert.deepEqual(m45.night, {
    sessionId: "created-m45",
    finished: true,
    nightFolder: `${HOME}/Seestar/M 45_sub`,
    targets: [{ targetId: "t-M45", folderName: "" }],
    eveningDate: "2026-10-10",
    longitude: 139.99,
    startedAt: "2026-10-10T20:00:00Z",
    shiftSeconds: 0,
    imported: true,
    noMeasure: true,
  });
  assert.equal(m45.place, "Maison");
  assert.equal(m45.targetName, "M45");
  // The logs of the session widen the folder; the target is then a subfolder.
  assert.equal(sadr.night.nightFolder, `${HOME}/ASIAIR/Autorun`);
  assert.deepEqual(sadr.night.targets, [
    { targetId: "t-Sadr", folderName: "Light/Sadr" },
  ]);
  assert.notEqual(m45.night.sessionId, sadr.night.sessionId);
});

test("a session with a clock shift is read on the evening its files were written, and its incidents move with the clock", () => {
  const shifted = importedSessions(proposal, outcome.sessions)[2];
  assert.equal(shifted.night.eveningDate, "2026-08-03", "the written evening");
  assert.equal(shifted.eveningDate, "2026-08-02", "the session's own evening");
  assert.equal(shifted.night.shiftSeconds, -9 * 3600);
  // The shift in force is the one the person left ticked, whatever the
  // proposal started with.
  const unticked = importedSessions(
    proposal,
    outcome.sessions.map((m) =>
      m.sessionId === "created-shift" ? { ...m, shiftHours: 0 } : m,
    ),
  )[2];
  assert.equal(unticked.night.shiftSeconds, 0);
  const incident = {
    id: "guiding_timeouts:2026-08-03T21:00:00Z",
    eventType: "guiding_timeouts",
    start: "2026-08-03T21:00:00Z",
    end: "2026-08-03T23:30:00Z",
    code: "guiding_timeouts",
    params: {
      count: 4,
      firstAt: "2026-08-03T21:00:00Z",
      lastAt: "2026-08-03T23:30:00Z",
    },
    description: "x",
    evidence: {
      source: "asiair",
      code: "settle_timeout",
      params: { count: 4 },
    },
    payload: {
      count: 4,
      firstAt: "2026-08-03T21:00:00Z",
      lastAt: "2026-08-03T23:30:00Z",
    },
  } as unknown as DetectedIncident;
  const moved = shiftIncident(incident, -9 * 3600);
  assert.equal(moved.start, "2026-08-03T12:00:00Z");
  assert.equal(moved.end, "2026-08-03T14:30:00Z");
  assert.equal(moved.payload.firstAt, "2026-08-03T12:00:00Z");
  assert.equal(moved.payload.lastAt, "2026-08-03T14:30:00Z");
  assert.equal(moved.params.firstAt, "2026-08-03T12:00:00Z");
  assert.equal(moved.id, incident.id, "the key is an identity, not a time");
  assert.equal(
    shiftIncident(incident, 0),
    incident,
    "no shift, nothing copied",
  );
});

test("the selector says each session with its name, its day and how many lines it holds", () => {
  const session = { designations: "M45", eveningDate: "2026-10-10" };
  assert.equal(sessionSegment("fr", session, 3, " · "), "M45 · 10 oct. · 3");
  assert.equal(sessionSegment("fr", session, null, " · "), "M45 · 10 oct.");
  assert.equal(
    sessionSegment(
      "fr",
      { designations: "Sadr", eveningDate: "2026-08-14" },
      5,
      " · ",
    ),
    "Sadr · 14 août · 5",
  );
  assert.match(sessionSegment("ja", session, 3, " · "), /^M45 · 10月10日 · 3$/);
});
