import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

import type { BulkCreateOutcome, Proposal, SpaceStorage } from "./bridge.ts";
import {
  DUALBAND_WORDS,
  bandOfFilter,
  bandOfFilters,
  folderToOpen,
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

test("the band of a filter is the core's decision: the import's step and the night's say the same (the smoke's OIII « Dual-band » then « Broadband »)", () => {
  const core = readFileSync(
    new URL(
      "../../../crates/astro-sync-core/src/plan_image.rs",
      import.meta.url,
    ),
    "utf8",
  );
  const list = /pub const DUALBAND_WORDS: \[&str; \d+\] = \[([^\]]*)\]/.exec(
    core,
  );
  assert.ok(list, "the core's DUALBAND_WORDS");
  const words = [...list[1].matchAll(/"([^"]+)"/g)].map((m) => m[1]);
  assert.deepEqual([...DUALBAND_WORDS], words);
  // The names of the core's own test, and a mono sensor's single lines.
  for (const name of [
    "L-eXtreme",
    "L-Ultimate",
    "ALP-T",
    "NBZ",
    "Dual band",
    "Bibande",
    "Ha/OIII",
    "L-eNhance",
  ])
    assert.equal(bandOfFilter(name), "dualband", name);
  for (const name of [
    "UV/IR-cut",
    "RGB",
    "L-Pro",
    "Lum",
    "Ha",
    "OIII",
    "SII",
    "Hα",
  ])
    assert.equal(bandOfFilter(name), "broadband", name);
  // The fixture of the smoke: one OIII target.
  assert.equal(bandOfFilters(["OIII"]), "broadband");
  // The bands a person answers in the review are written as these names.
  assert.equal(bandOfFilter("Dual-band"), "dualband");
  assert.equal(bandOfFilter("Broadband"), "broadband");
});
