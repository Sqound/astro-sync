import assert from "node:assert/strict";
import test from "node:test";

import type { BulkCreateOutcome, Proposal, SpaceStorage } from "./bridge.ts";
import {
  bandOfFilters,
  folderToOpen,
  formatOf,
  imageRoom,
  sendPrimary,
  treatmentCards,
  treatmentCounts,
  treatmentFolders,
} from "./detectTreatments.ts";

const target = (key: string, frames: number, filters: string[] = []) => ({
  key,
  targetId: key,
  designation: key.toUpperCase(),
  frames,
  filters,
});

const proposal = {
  sessions: [
    { id: "s9", targets: [target("ic1805", 98)] },
    {
      id: "s10",
      targets: [target("ic1805", 45), target("sh2-103", 36, ["L-eNhance"])],
    },
    { id: "s31", targets: [target("m31", 64)] },
  ],
} as unknown as Proposal;

const outcome = {
  sessions: [
    {
      detectedId: "s9",
      sessionId: "a9",
      eveningDate: "2026-08-09",
      designations: ["IC1805"],
    },
    {
      detectedId: "s10",
      sessionId: "a10",
      eveningDate: "2026-08-10",
      designations: ["IC1805", "SH2-103"],
    },
    {
      detectedId: "s31",
      sessionId: "a31",
      eveningDate: "2026-07-31",
      designations: ["M31"],
    },
  ],
  multiNight: [
    {
      groupId: "g",
      planId: "p",
      designation: "IC1805",
      sessionIds: ["a9", "a10"],
      goalSeconds: 1,
    },
  ],
  placesCreated: [],
} as unknown as BulkCreateOutcome;

test("one card per target over every night created; its multi-night plan; the band of its filters", () => {
  const cards = treatmentCards(proposal, outcome);
  assert.deepEqual(
    cards.map((c) => [
      c.targetId,
      c.nights.length,
      c.frames,
      c.band,
      c.multiNight?.planId ?? null,
    ]),
    [
      ["ic1805", 2, 143, "broadband", "p"],
      ["m31", 1, 64, "broadband", null],
      ["sh2-103", 1, 36, "dualband", null],
    ],
  );
  assert.deepEqual(treatmentCounts(cards), {
    nights: 3,
    targets: 3,
    frames: 243,
  });
  assert.equal(bandOfFilters(["UV/IR cut"]), "broadband");
});

test("« 2 images sur 3 »: the place left, none when nothing caps it", () => {
  const storage = (count: number, limit: number) =>
    ({ captureCount: count, captureCountLimit: limit }) as SpaceStorage;
  assert.deepEqual(imageRoom(storage(1, 3)), { remaining: 2, quota: 3 });
  assert.equal(imageRoom(storage(10, 9007199254740991)), null);
  assert.equal(imageRoom(null), null);
  assert.deepEqual(sendPrimary([]), { kind: "done" });
  assert.deepEqual(sendPrimary(["a", "b"]), { kind: "send", count: 2 });
  assert.equal(formatOf("/x/M31_natural.tif"), "TIFF");
  assert.equal(formatOf("/x/M31.png"), "PNG");
});

test("the folder « Ouvrir le dossier de travail » opens is the target's, else the working folder", () => {
  const night = (nightFolder: string, targetId: string, folderName: string) =>
    ({
      detectedId: "d",
      sessionId: "s",
      eveningDate: "2026-10-01",
      nightFolder,
      longitude: null,
      targets: [{ targetId, designation: "x", folderName, frames: 3 }],
      linked: 0,
      alreadyThere: 0,
      conflicts: 0,
    }) as const;
  const folders = treatmentFolders([
    night("/w/2026-10-01 · Home · local-aa", "sh2-103", "Sh2-103"),
    night("/w/2026-10-02 · Home · local-bb", "sh2-103", "Sh2-103"),
    night("C:\\w\\2026-10-01 · Home · local-cc\\", "m31", "M 31"),
  ]);
  assert.deepEqual(folders, {
    "sh2-103": "/w/2026-10-01 · Home · local-aa/Sh2-103",
    m31: "C:\\w\\2026-10-01 · Home · local-cc\\M 31",
  });
  assert.equal(
    folderToOpen(folders, "sh2-103", "/w"),
    "/w/2026-10-01 · Home · local-aa/Sh2-103",
  );
  // The target has no folder yet, or none is being treated.
  assert.equal(folderToOpen(folders, "ngc-7000", "/w"), "/w");
  assert.equal(folderToOpen({}, null, "/w"), "/w");
  assert.equal(folderToOpen({}, null, null), null);
});
