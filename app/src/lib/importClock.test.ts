import assert from "node:assert/strict";
import test from "node:test";

import type { BulkCreateOutcome, DetectionInput, Proposal } from "./bridge.ts";
import {
  clocksOf,
  loadImportClock,
  saveImportClocks,
  withImportClock,
} from "./importClock.ts";

const proposal = {
  sessions: [
    { id: "a", eveningDate: "2026-08-02", writtenEveningDate: "2026-08-03" },
    { id: "b", eveningDate: "2026-08-07", writtenEveningDate: "2026-08-07" },
  ],
} as unknown as Proposal;
const outcome = {
  sessions: [
    {
      detectedId: "a",
      sessionId: "s-a",
      eveningDate: "2026-08-02",
      shiftHours: -9,
    },
    {
      detectedId: "b",
      sessionId: "s-b",
      eveningDate: "2026-08-07",
      shiftHours: 0,
    },
  ],
  multiNight: [],
  placesCreated: [],
} as unknown as BulkCreateOutcome;

function storage() {
  const held = new Map<string, string>();
  return {
    getItem: (k: string) => held.get(k) ?? null,
    setItem: (k: string, v: string) => void held.set(k, v),
  };
}

test("only the sessions whose clock was corrected keep a correction", () => {
  const clocks = clocksOf(proposal, outcome);
  assert.deepEqual(Object.keys(clocks), ["s-a"]);
  assert.deepEqual(clocks["s-a"], {
    writtenEvening: "2026-08-03",
    shiftHours: -9,
  });
});

test("the correction is kept per session on this computer and read back", () => {
  const store = storage();
  saveImportClocks(store, clocksOf(proposal, outcome));
  assert.equal(loadImportClock(store, "s-a")?.shiftHours, -9);
  assert.equal(loadImportClock(store, "s-b"), null);
  assert.equal(loadImportClock(null, "s-a"), null);
  store.setItem("astro-sync.importClock:s-bad", "{nope");
  assert.equal(loadImportClock(store, "s-bad"), null);
  store.setItem(
    "astro-sync.importClock:s-zero",
    JSON.stringify({ writtenEvening: "y", shiftHours: 0 }),
  );
  assert.equal(loadImportClock(store, "s-zero"), null);
});

test("the wizard draws the night's window on the evening the files were written", () => {
  const clock = clocksOf(proposal, outcome)["s-a"];
  const input = { eveningDate: "2026-08-02", targets: [] } as DetectionInput;
  assert.equal(withImportClock(input, clock).eveningDate, "2026-08-03");
  assert.equal(withImportClock(input, null), input);
});
