import assert from "node:assert/strict";
import test from "node:test";

import type { DeclaredIncident, DetectedIncident } from "./bridge";
import {
  declaredLine,
  declaredText,
  detectedEvidence,
  detectedLine,
  detectedText,
  lengthText,
} from "./incidentsLines.ts";
import { createTranslator } from "../i18n/locale.ts";

const fr = createTranslator("fr");
const en = createTranslator("en");

const FLIP: DetectedIncident = {
  id: "pause:2026-08-14T22:13:38Z",
  eventType: "pause",
  start: "2026-08-14T22:13:38Z",
  end: "2026-08-14T22:24:58Z",
  code: "meridian_flip",
  params: {
    durationSeconds: 680,
    succeeded: true,
    afterPose: 13,
    beforePose: 14,
    gapSeconds: 707,
  },
  description: "Meridian flip",
  evidence: {
    source: "asiair",
    code: "meridian_flip",
    params: {
      durationSeconds: 680,
      succeeded: true,
      afterPose: 13,
      beforePose: 14,
      gapSeconds: 707,
    },
  },
  payload: { reason: "Meridian flip", cause: "meridian_flip" },
};

const LOST: DetectedIncident = {
  id: "equipment_issue:2026-08-15T00:52:23Z",
  eventType: "equipment_issue",
  start: "2026-08-15T00:52:23Z",
  end: "2026-08-15T01:09:29Z",
  code: "guide_star_lost",
  params: { count: 8, notFound: 1 },
  description: "Guide star lost 8 times, 1 with no star found again",
  evidence: {
    source: "phd2",
    code: "guide_star_lost",
    params: { count: 8, notFound: 1 },
  },
  payload: { description: "Guide star lost 8 times", severity: "minor" },
};

test("lengths are said as the window says them", () => {
  assert.equal(lengthText(fr, 47), "47 s");
  assert.equal(lengthText(fr, 707), "11 min 47 s");
  assert.equal(lengthText(fr, 720), "12 min");
});

test("a detection says its words from its code, in the language of the window", () => {
  assert.equal(detectedText(fr, "fr", FLIP), "Retournement au méridien");
  assert.equal(
    detectedText(fr, "fr", LOST),
    "Étoile de guidage perdue 8 fois, une fois sans étoile retrouvée",
  );
  assert.equal(
    detectedText(en, "en", LOST),
    "Guide star lost 8 times, once with no star found again",
  );
  assert.equal(
    detectedText(fr, "fr", {
      code: "guide_star_lost",
      params: { count: 1, notFound: 0 },
    }),
    "Étoile de guidage perdue une fois",
  );
  assert.equal(
    detectedText(fr, "fr", {
      code: "pose_gap",
      params: { afterPose: 13, beforePose: 14, gapSeconds: 707 },
    }),
    "Aucune pose pendant 11 min 47 s, entre la pose 13 et la pose 14",
  );
});

test("the grey line carries the source, the outcome and the poses around the gap", () => {
  const evidence = detectedEvidence(fr, "fr", FLIP);
  assert.equal(evidence.code, "meridian_flip");
  assert.equal(evidence.params.source, "ASIAIR");
  assert.equal(evidence.params.outcome, "réussi");
  assert.equal(evidence.params.duration, "11 min 20 s");
  assert.equal(evidence.params.gapPart, " · aucune pose entre la 13 et la 14");
  const lost = detectedEvidence(fr, "fr", LOST);
  assert.equal(lost.params.source, "PHD2");
  assert.equal(lost.params.notFoundPart, " · une fois sans étoile retrouvée");
});

test("a detected line is ready to add: payload in the person's language, genre, times", () => {
  const line = detectedLine(fr, "fr", FLIP);
  assert.equal(line.origin, "detected");
  assert.equal(line.genre, "pause");
  assert.equal(line.start, Date.parse("2026-08-14T22:13:38Z") / 1000);
  assert.equal(line.end, Date.parse("2026-08-14T22:24:58Z") / 1000);
  assert.deepEqual(line.payload, {
    cause: "meridian_flip",
    reason: "Retournement au méridien",
  });
  const lost = detectedLine(fr, "fr", LOST);
  assert.deepEqual(lost.payload, {
    description:
      "Étoile de guidage perdue 8 fois, une fois sans étoile retrouvée",
    severity: "minor",
  });
  // A newer genre keeps the payload the core gave, its description in words.
  const timeouts = detectedLine(fr, "fr", {
    id: "guiding_timeouts:2026-08-15T01:20:00Z",
    eventType: "guiding_timeouts",
    start: "2026-08-15T01:20:00Z",
    end: null,
    code: "guiding_timeouts",
    params: { count: 11 },
    description: "Guiding did not settle in time 11 times",
    evidence: {
      source: "asiair",
      code: "settle_timeout",
      params: { count: 11 },
    },
    payload: { count: 11, firstAt: "a", lastAt: "b", description: "x" },
  });
  assert.equal(timeouts.payload.count, 11);
  assert.equal(
    timeouts.payload.description,
    "11 attentes de mise en place du guidage dépassées",
  );
});

function declared(
  eventType: DeclaredIncident["eventType"],
  payload: Record<string, unknown>,
  extra: Partial<DeclaredIncident> = {},
): DeclaredIncident {
  return {
    eventId: "01J8Z3K9QZ6N3S6R3E6G8H1234",
    eventType,
    recordedAt: "2026-08-14T22:14:00Z",
    payload,
    source: "declared",
    timing: "observed",
    targetId: null,
    until: null,
    resumeEventId: null,
    ...extra,
  };
}

test("a declared signalement says its own words, else its sky or its cause", () => {
  assert.equal(
    declaredText(fr, declared("condition", { sky: "hazy", notes: "Voilé" })),
    "Voilé",
  );
  assert.equal(
    declaredText(fr, declared("condition", { sky: "hazy" })),
    "Voilé",
  );
  assert.equal(declaredText(fr, declared("pause", { cause: "wind" })), "Vent");
  assert.equal(declaredText(fr, declared("pause", {})), "Pause");
});

test("a declared line is the person's, a pause keeps its end, a cause or an app stop is no incident", () => {
  const pause = declaredLine(
    fr,
    declared("pause", { reason: "Flip" }, { until: "2026-08-14T22:26:00Z" }),
  );
  assert.ok(pause);
  assert.equal(pause.origin, "declared");
  assert.equal(pause.end, Date.parse("2026-08-14T22:26:00Z") / 1000);
  assert.equal(pause.stored, false);
  assert.equal(
    declaredLine(
      fr,
      declared("note", { pauseCause: { pauseEventId: "x", cause: "wind" } }),
    ),
    null,
  );
  assert.equal(
    declaredLine(fr, declared("note", { appStopped: { from: "a", to: "b" } })),
    null,
  );
  assert.ok(declaredLine(fr, declared("note", { text: "Vent" })));
});

test("an event Astro Sync wrote before stays, as a detected line with nothing to choose", () => {
  const line = declaredLine(
    fr,
    declared(
      "equipment_issue",
      { description: "Guidage" },
      { source: "astro-sync" },
    ),
  );
  assert.ok(line);
  assert.equal(line.origin, "detected");
  assert.equal(line.stored, true);
});
