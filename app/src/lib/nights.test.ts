import assert from "node:assert/strict";
import test from "node:test";

import {
  createFormReady,
  detectionInput,
  eveningDateAtLongitude,
  longitudeOf,
  isOffline,
  knownPlaces,
  loadSnapshot,
  missingText,
  nightHeader,
  nightRow,
  offlineWhen,
  saveSnapshot,
  treatmentHold,
} from "./nights.ts";
import {
  IC,
  M31,
  NGC,
  NIGHT_FOLDER,
  SESSIONS,
  capture,
  detail,
  detectionAllFound,
  image,
  online,
  resume,
  target,
  view,
  M31_LIGHTS,
} from "./homeFixtures.ts";
import { SUPPORTED_LOCALES, createTranslator } from "../i18n/locale.ts";
import type { SessionSummary } from "./bridge";

const fr = createTranslator("fr");
const plain = (text: string | null) => text ?? "";
const TZ = "Europe/Paris";
const byId = (id: string) => SESSIONS.find((s) => s.id === id)!;

function emptyFolder() {
  const detection = detectionAllFound();
  detection.targets = [
    target(IC, 0, { filters: [], exposures: [], gains: [], bytes: 0 }, null),
  ];
  detection.logs = [];
  detection.calibration = { darks: 0, flats: 0, darkFlats: 0, biases: 0 };
  return detection;
}

test("a night just open (Jeu. 8 oct.): « en cours », the folder created empty, nothing online, not started", () => {
  const row = nightRow(fr, "fr", {
    session: byId("night-8"),
    detail: detail("night-8", [IC]),
    view: view(
      emptyFolder(),
      online("night-8", [IC]),
      resume("not_started", "cibles"),
    ),
    timeZone: TZ,
  });
  assert.equal(row.day, "Jeu. 8 oct.");
  assert.equal(row.hours, "21:10 – en cours");
  assert.equal(row.open, true);
  assert.equal(row.placeAndTargets, "Col des Étoiles · IC 1396");
  assert.equal(row.local, "Dossier créé, vide");
  assert.equal(row.online, "Rien encore");
  assert.equal(row.state, "not_started");
  assert.equal(row.missing, null);
});

test("a night to complete (Mer. 7 oct.): its folder, the phone's image, and what it misses in the expected words", () => {
  const detection = detectionAllFound();
  detection.targets[1] = target(
    IC,
    38,
    detection.targets[1].lightSummary,
    null,
  );
  const row = nightRow(fr, "fr", {
    session: byId("night-7"),
    detail: detail("night-7", [NGC, IC, M31]),
    view: view(
      detection,
      online(
        "night-7",
        [NGC, IC, M31],
        [
          capture(
            M31,
            image("M31_final.jpg", "aa", 14_000_000, { source: "phone" }),
          ),
        ],
      ),
      resume("to_complete", "fichiers", [
        { key: "finalImage", targetId: NGC, name: "NGC 7000" },
        { key: "finalImage", targetId: IC, name: "IC 1396" },
        { key: "skyAnnotations", count: 3 },
        { key: "analysis" },
      ]),
    ),
    timeZone: TZ,
  });
  assert.equal(row.day, "Mer. 7 oct.");
  assert.equal(row.hours, "21:05 – 04:40");
  assert.equal(row.placeAndTargets, "Col des Étoiles · NGC 7000, IC 1396, M31");
  assert.equal(row.local, "3 cibles · 250 poses · 2 images finales");
  assert.equal(row.online, "1 image (M31, depuis le téléphone)");
  assert.equal(row.state, "to_complete");
  assert.equal(
    plain(row.missing),
    "2 images finales, annotations du ciel de 3 images, l’analyse",
  );
});

test("logs added after the sending, a complete night, a night with no folder here", () => {
  const added = view(
    {
      ...emptyFolder(),
      targets: [target(M31, 96, M31_LIGHTS, null, "online")],
      logs: [
        { relativePath: "logs/a.log", kind: "nina", bytes: 10 },
        {
          relativePath: "logs/PHD2_GuideLog_2026-10-05.txt",
          kind: "phd2",
          bytes: 4_000_000,
        },
      ],
    },
    online(
      "night-5",
      [M31],
      [
        capture(
          M31,
          image("M31_final.jpg", "aa", 14_000_000, {
            source: "phone",
            solved: true,
          }),
        ),
      ],
    ),
    resume("to_complete", "fichiers", [{ key: "logsAdded" }]),
  );
  const five = nightRow(fr, "fr", {
    session: byId("night-5"),
    detail: detail("night-5", [M31]),
    view: added,
    timeZone: TZ,
  });
  assert.equal(five.hours, "20:50 – 03:15");
  assert.equal(five.local, "1 cible · 2 journaux ajoutés");
  assert.equal(five.online, "1 image avec annotations");
  assert.equal(
    plain(five.missing),
    "journaux ajoutés au dossier après l’envoi",
  );
  assert.equal(
    plain(missingText(fr, "fr", added.resume.missing, new Map(), "home")),
    "journaux ajoutés au dossier depuis l’envoi",
  );

  const complete = view(
    {
      ...emptyFolder(),
      targets: [target(NGC, 180, M31_LIGHTS, null, "online")],
    },
    online(
      "night-3",
      [NGC],
      [capture(NGC, image("NGC.tif", "bb", 1, { solved: true }))],
      { sentAt: "2026-10-04T09:00:00Z" },
    ),
    resume("complete", null),
  );
  const three = nightRow(fr, "fr", {
    session: byId("night-3"),
    view: complete,
    timeZone: TZ,
  });
  assert.equal(three.local, "1 cible · 180 poses");
  assert.equal(three.online, "1 image · annotations · analyse");
  assert.equal(three.state, "complete");

  const away = view(
    null,
    online(
      "night-2",
      [M31, NGC],
      [
        capture(M31, image("a.jpg", "cc", 1, { solved: true })),
        capture(NGC, image("b.jpg", "dd", 1, { solved: true })),
      ],
    ),
    resume("complete", null),
  );
  const two = nightRow(fr, "fr", {
    session: byId("night-2"),
    view: away,
    timeZone: TZ,
  });
  assert.equal(two.local, "Pas de dossier sur cet ordinateur");
  assert.equal(two.online, "2 images · annotations");
  assert.equal(two.hours, "20:45 – 01:30");
});

test("a row not read yet keeps its date and place; the empty and offline sentences read as expected", () => {
  const row = nightRow(fr, "fr", {
    session: byId("night-7"),
    detail: detail("night-7", []),
    timeZone: TZ,
  });
  assert.equal(row.placeAndTargets, "Col des Étoiles");
  assert.equal(row.local, null);
  assert.equal(row.state, null);
  assert.equal(
    plain(fr("nights.empty")),
    "Aucune session dans votre espace pour l’instant.",
  );
  assert.equal(
    plain(fr("nights.offline", { date: "8 oct. à 10:20" })),
    "Hors ligne : l’état en ligne est celui du 8 oct. à 10:20.",
  );
  assert.equal(
    plain(
      fr("nights.offline", {
        date: offlineWhen(fr, "fr", "2026-10-08T08:20:00Z", TZ),
      }),
    ),
    "Hors ligne : l’état en ligne est celui du 8 oct. à 10:20.",
  );
  assert.ok(isOffline("network_error"));
  assert.ok(!isOffline("not_paired"));
  const store = new Map<string, string>();
  const storage = {
    getItem: (k: string) => store.get(k) ?? null,
    setItem: (k: string, v: string) => void store.set(k, v),
  };
  saveSnapshot(storage, { savedAt: "2026-10-08T08:20:00Z", rows: [row] });
  assert.equal(loadSnapshot(storage)?.rows[0].sessionId, "night-7");
  assert.equal(loadSnapshot({ getItem: () => "{oops", setItem() {} }), null);
});

test("the creation window: a date and a place are needed, hours read « 21:00 », places come from the space's nights", () => {
  const form = {
    date: "2026-10-07",
    place: "Col des Étoiles",
    start: "21:00",
    end: "04:00",
    title: "",
  };
  assert.ok(createFormReady(form));
  assert.ok(!createFormReady({ ...form, place: " " }));
  assert.ok(!createFormReady({ ...form, start: "9h" }));
  assert.ok(createFormReady({ ...form, start: "", end: "" }));
  assert.deepEqual(
    knownPlaces([detail("night-7", []), detail("night-5", []), null]),
    ["Col des Étoiles"],
  );
  assert.equal(
    plain(
      fr("nights.createForm.folderLater", { path: "~/Documents/Astro Sync" }),
    ),
    "Le dossier de la nuit sera créé à l’étape « Dossier », dans ~/Documents/Astro Sync.",
  );
});

test("the detection of a night reads its targets online, the ticks kept here and the stacking planned", () => {
  const input = detectionInput(
    byId("night-7"),
    online("night-7", [NGC, IC, M31], [capture(M31, image("m.jpg", "x", 1))]),
    new Set([IC]),
    new Set([NGC]),
    TZ,
  );
  assert.equal(input.eveningDate, "2026-10-07");
  assert.deepEqual(
    input.targets.map((t) => [
      t.designation,
      t.ticked,
      t.onlineFinal,
      t.stackingPlanned,
    ]),
    [
      ["NGC 7000", true, false, true],
      ["IC 1396", false, false, false],
      ["M31", true, true, false],
    ],
  );
  assert.equal(
    NIGHT_FOLDER.endsWith("2026-10-07 · Col des Étoiles · 7f3a2c"),
    true,
  );
});

test("the window of a night is drawn from its place's longitude, never from a time zone", () => {
  // 21:48 UTC on the 10th at 2° E: the evening of the 10th.
  assert.equal(eveningDateAtLongitude("2026-08-10T21:48:00Z", 2), "2026-08-10");
  // 01:00 UTC on the 11th at 10° E: still the evening of the 10th.
  assert.equal(
    eveningDateAtLongitude("2026-08-11T01:00:00Z", 10),
    "2026-08-10",
  );
  // 12:00 UTC on the 7th at 135° E is 21:00 local: the evening of the 7th.
  assert.equal(
    eveningDateAtLongitude("2026-10-07T12:00:00Z", 135),
    "2026-10-07",
  );
  const withPlace = detail("night-7", [NGC]);
  withPlace.night!.place = {
    value: { name: "Col des Mélèzes", latitude: 45, longitude: 10 },
  };
  assert.equal(longitudeOf(withPlace), 10);
  assert.equal(longitudeOf(detail("night-7", [NGC])), null);
  const input = detectionInput(
    { startedAt: "2026-08-11T01:00:00Z" },
    online("night-7", [NGC]),
    new Set(),
    new Set(),
    undefined,
    10,
  );
  assert.equal(input.eveningDate, "2026-08-10");
  assert.equal(input.longitude, 10);
});

test("the header of a night, as the night's page on the web draws it: the eyebrow, « Nuit du … », the line with the hours in UTC and the number of targets", () => {
  const seven = byId("night-7");
  const withPlan = detail("night-7", [NGC, IC, M31]);
  withPlan.night!.title = { value: "Triangle d’été" };
  withPlan.session = { ...seven, planId: "plan-1" };
  const header = nightHeader(fr, "fr", {
    session: { ...seven, planId: "plan-1" },
    detail: withPlan,
    online: online("night-7", [NGC, IC, M31]),
  });
  // The name the night carries (the plan's) is the small gold line above,
  // never the title: the title is the night's date.
  assert.equal(header.eyebrow, "Triangle d’été");
  assert.equal(header.title, "Nuit du 7 oct.");
  assert.equal(header.line, "Col des Étoiles · 19:05 → 02:40 UTC · 3 cibles");
  // The names of the targets are not in the header any more.
  assert.ok(!header.line.includes("NGC 7000"));
});

test("a night without a plan says so above its date; a night of a plan that carries no name has no eyebrow; one night in progress and one target", () => {
  const eight = byId("night-8");
  const bare = nightHeader(fr, "fr", {
    session: { ...eight, planId: null },
    detail: detail("night-8", [IC]),
    online: online("night-8", [IC]),
  });
  assert.equal(bare.eyebrow, "Nuit sans plan");
  assert.equal(bare.title, "Nuit du 8 oct.");
  // Open: the start only, in UTC; one target is « 1 cible ».
  assert.equal(bare.line, "Col des Étoiles · 19:10 UTC · 1 cible");
  const named = nightHeader(fr, "fr", {
    session: { ...eight, planId: "plan-2" },
    detail: detail("night-8", [IC]),
    online: null,
  });
  assert.equal(named.eyebrow, null);
  // Nothing online yet: no count, and no empty separator.
  assert.equal(named.line, "Col des Étoiles · 19:10 UTC");
  // English and Japanese say the same thing without a word-for-word copy.
  const en = nightHeader(createTranslator("en"), "en", {
    session: { ...eight, planId: null },
    detail: detail("night-8", [IC]),
    online: online("night-8", [IC, NGC]),
  });
  assert.equal(en.eyebrow, "Night without a plan");
  assert.equal(en.line, "Col des Étoiles · 19:10 UTC · 2 targets");
});

test("each night says it is being read until its read is back, then shows what was read", () => {
  const waiting = nightRow(fr, "fr", {
    session: byId("night-7"),
    reading: true,
    timeZone: TZ,
  });
  assert.equal(waiting.reading, true);
  // Its date stays readable meanwhile.
  assert.match(waiting.day, /7 oct\./);
  assert.equal(plain(fr("wizard.wait.night")), "Lecture de la session…");
  const read = nightRow(fr, "fr", {
    session: byId("night-7"),
    detail: detail("night-7", []),
    timeZone: TZ,
  });
  assert.equal(read.reading, false, "a row read is no longer waiting");
});

// A session still to debrief, or abandoned, is held back
// from the treatment. The rule is the contract's -- `journal.schema.json`,
// `state` -- told by the fields the service sends on a session.

const planned = (
  overrides: Partial<SessionSummary> = {},
): Pick<SessionSummary, "planId" | "status" | "outcome"> => ({
  planId: "plan-1",
  status: "finished",
  outcome: null,
  ...overrides,
});

test("a finished session of a plan that is not debriefed is « à débriefer »: held", () => {
  assert.equal(treatmentHold(planned()), "to_debrief");
  // An earlier service does not send `outcome`: the contract reads `null`.
  assert.equal(treatmentHold(planned({ outcome: undefined })), "to_debrief");
});

test("an abandoned session is held, whatever else it says", () => {
  assert.equal(treatmentHold(planned({ outcome: "abandoned" })), "abandoned");
  assert.equal(
    treatmentHold(planned({ outcome: "abandoned", planId: null })),
    "abandoned",
  );
});

test("a session the service gives debriefed is selectable again", () => {
  assert.equal(treatmentHold(planned({ outcome: "executed" })), null);
  assert.equal(treatmentHold(planned({ outcome: "partial" })), null);
  // The very same session, before and after its debriefing.
  const before = planned();
  assert.equal(treatmentHold(before), "to_debrief");
  assert.equal(treatmentHold({ ...before, outcome: "executed" }), null);
});

test("a session without a plan has no promise to debrief against, and an open night is not over: neither is held", () => {
  assert.equal(treatmentHold(planned({ planId: null })), null);
  assert.equal(treatmentHold(planned({ status: "open" })), null);
  assert.equal(treatmentHold(planned({ status: "paused" })), null);
});

test("the row of a held session carries the message, no state of its own, and the row of a debriefed one nothing of it", () => {
  const session = { ...byId("night-7"), planId: "plan-1", outcome: null };
  const view7 = view(
    detectionAllFound(),
    online("night-7", [NGC]),
    resume("to_complete", "cibles"),
  );
  const held = nightRow(fr, "fr", {
    session,
    detail: detail("night-7", [NGC]),
    view: view7,
    timeZone: TZ,
  });
  assert.equal(held.held, "to_debrief");
  assert.equal(
    held.heldMessage,
    "Débriefez cette session dans l’app pour commencer le traitement.",
  );
  assert.equal(held.state, null);
  assert.equal(held.missing, null);
  // While its detail is being read the row already says it.
  const reading = nightRow(fr, "fr", { session, reading: true, timeZone: TZ });
  assert.equal(reading.held, "to_debrief");
  const done = nightRow(fr, "fr", {
    session: { ...session, outcome: "executed" },
    detail: detail("night-7", [NGC]),
    view: view7,
    timeZone: TZ,
  });
  assert.equal(done.held, null);
  assert.equal(done.heldMessage, null);
  assert.equal(done.state, "to_complete");
});

test("the message of a held session is said in every shipped language", () => {
  for (const locale of SUPPORTED_LOCALES) {
    const text = createTranslator(locale)("nights.held.debrief");
    assert.notEqual(text, "nights.held.debrief", locale);
    assert.ok(text.length > 20, locale);
  }
});
