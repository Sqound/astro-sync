import assert from "node:assert/strict";
import test from "node:test";

import type { UserData } from "./bridge.ts";
import { utcOffsetMinutes, withUtcOffsets } from "./detectPlaces.ts";

test("a place's offset follows its time zone and the date (summer time)", () => {
  assert.equal(
    utcOffsetMinutes("Europe/Paris", new Date("2026-08-10T12:00:00Z")),
    120,
  );
  assert.equal(
    utcOffsetMinutes("Europe/Paris", new Date("2026-01-10T12:00:00Z")),
    60,
  );
  assert.equal(
    utcOffsetMinutes("Asia/Tokyo", new Date("2026-08-10T12:00:00Z")),
    540,
  );
  assert.equal(
    utcOffsetMinutes("America/St_Johns", new Date("2026-01-10T12:00:00Z")),
    -210,
  );
  assert.equal(utcOffsetMinutes("UTC", new Date("2026-08-10T12:00:00Z")), 0);
});

test("a time zone the system does not know gives no offset", () => {
  assert.equal(utcOffsetMinutes("Mars/Olympus", new Date()), null);
});

test("every place of the person's data gets its offset, none without a time zone", () => {
  const user = {
    places: [
      {
        id: "a",
        name: "Maison",
        latitude: 43.9,
        longitude: 7.4,
        timeZone: "Europe/Paris",
      },
      {
        id: "b",
        name: "Site noir",
        latitude: 44,
        longitude: 7,
        timeZone: null,
      },
    ],
    equipment: [],
    sessions: [],
    plans: [],
    account: { advancedPlanning: false, severalPlansPerNight: false },
  } as UserData;
  const out = withUtcOffsets(user, new Date("2026-08-10T12:00:00Z"));
  assert.deepEqual(
    out.places.map((place) => place.utcOffsetMinutes),
    [120, null],
  );
});
