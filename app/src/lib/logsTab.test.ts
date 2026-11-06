import assert from "node:assert/strict";
import test from "node:test";

import { createTranslator } from "../i18n/locale.ts";
import type {
  CapabilitiesState,
  DetectedLog,
  NightAnalysis,
  NightAnalysisSent,
  SessionReport,
} from "./bridge";
import {
  logLine,
  logsState,
  nightGuidingRms,
  orderedLogs,
  replacesLine,
  logsClockLine,
  sentLines,
  sentNightLine,
} from "./logsTab.ts";
import { isReserved, nightChoices } from "./tabs.ts";
import {
  CAPABILITIES_ADVANCED,
  CAPABILITIES_PLUS,
  ROOT,
} from "./homeFixtures.ts";

const fr = createTranslator("fr");
const plain = (text: string) => text.replaceAll(" ", " ").replaceAll("'", "’");
const same = (actual: string | null, expected: string) =>
  assert.equal(plain(actual ?? ""), plain(expected));

/** The night of Monday 5 October, NINA and PHD2. */
const LOGS: DetectedLog[] = [
  {
    relativePath: "logs/PHD2_GuideLog_2026-10-05.txt",
    kind: "phd2",
    bytes: 4_000_000,
  },
  { relativePath: "logs/NINA_2026-10-05.log", kind: "nina", bytes: 1_400_000 },
];

function report(): SessionReport {
  return {
    reportVersion: 3,
    producer: "astro-sync",
    generatedAtUtc: "2026-10-06T08:00:00Z",
    source: {
      phd2FilesParsed: 1,
      asiairFilesParsed: 0,
      fitsFilesParsed: 96,
      ninaFilesParsed: 1,
    },
    pixelScaleArcsec: null,
    timeline: {
      startedAt: null,
      endedAt: null,
      events: [
        { type: "autofocus", timestampUtc: "", source: "nina", data: {} },
        { type: "meridian-flip", timestampUtc: "", source: "nina", data: {} },
      ],
    },
    acquisitionSteps: [1, 2, 3].map((n) => ({
      key: `step-${n}`,
      targetHint: "M31",
      filter: null,
      frameCount: 32,
      exposureSeconds: 120,
      gain: 100,
      binning: null,
      sensorTempC: null,
    })),
    quality: Array.from({ length: 96 }, () => ({
      timestampUtc: "",
      stepKey: null,
      filter: null,
      exposureSeconds: 120,
      gain: 100,
      iso: null,
      binning: null,
      sensorTempC: null,
      rmsTotalArcsec: null,
      rmsRaArcsec: null,
      rmsDecArcsec: null,
      guidingSampleCount: null,
      skyBackgroundAdu: null,
      fwhmPixels: null,
      eccentricity: null,
      starCount: null,
      rejected: false,
      rejectReason: null,
      sourcePath: "",
    })),
    guidingSeries: [
      {
        start: "",
        end: "",
        rmsRa: null,
        rmsDec: null,
        rmsTotal: 0.82,
        samples: 300,
      },
      {
        start: "",
        end: "",
        rmsRa: null,
        rmsDec: null,
        rmsTotal: 0.82,
        samples: 100,
      },
    ],
  };
}

const SENT: NightAnalysisSent = {
  analysis: {
    analysis: {
      id: "analysis-1",
      sessionId: "night-5",
      reportVersion: 2,
      createdAt: "2026-10-08T10:00:00Z",
      originDeviceId: null,
      deviceName: null,
    },
    replaced: true,
    steps: { proposed: 3, keptEdited: 0 },
  },
  skyMeasure: { kind: "not_measured" },
  logsUtcOffset: null,
};

test("the night selector reads the night folders, the most recent first, in the expected words", () => {
  const choices = nightChoices(
    [
      {
        nightId: "night-5",
        folder: `${ROOT}/2026-10-05 · Col des Étoiles · 3b91e0`,
        name: "2026-10-05 · Col des Étoiles · 3b91e0",
      },
      {
        nightId: "night-7",
        folder: `${ROOT}/2026-10-07 · Col des Étoiles · 7f3a2c`,
        name: "2026-10-07 · Col des Étoiles · 7f3a2c",
      },
      { nightId: "renamed", folder: `${ROOT}/Ma nuit`, name: "Ma nuit" },
    ],
    "fr",
  );
  assert.deepEqual(
    choices.map((choice) => choice.label),
    [
      "Mer. 7 oct. · Col des Étoiles",
      "Lun. 5 oct. · Col des Étoiles",
      "Ma nuit",
    ],
  );
  assert.equal(choices[1].eveningDate, "2026-10-05");
  assert.equal(choices[1].place, "Col des Étoiles");
});

test("Journaux: the logs found with their reader and size, the analysis it would replace, one primary", () => {
  const ordered = orderedLogs(LOGS);
  assert.deepEqual(
    ordered.map((log) => log.kind),
    ["nina", "phd2"],
  );
  same(
    logLine(fr, "fr", ordered[0], "2026-10-05"),
    "NINA · journal de séquence du 5 oct. : paliers, mises au point · 1,4 Mo",
  );
  same(
    logLine(fr, "fr", ordered[1], "2026-10-05"),
    "PHD2 · PHD2_GuideLog_2026-10-05.txt : guidage · 4 Mo",
  );
  same(
    replacesLine(fr, "fr", "2026-10-03T09:00:00Z"),
    "Une analyse du 3 oct. est en ligne pour cette nuit : le nouveau rapport la remplacerait.",
  );
  assert.equal(replacesLine(fr, "fr", null), null);
  assert.equal(
    logsState({
      reserved: false,
      hasNight: true,
      logs: LOGS,
      sending: false,
      sent: false,
    }),
    "found",
  );
});

test("Journaux: « Rapport envoyé » says what was read, the quality and guiding, what it replaces", () => {
  const analysis: NightAnalysis = {
    report: report(),
    skyMeasure: null,
    skyMissingReason: "no_reference_stars",
    logsUtcOffset: null,
  };
  assert.ok(Math.abs((nightGuidingRms(report()) ?? 0) - 0.82) < 1e-9);
  const lines = sentLines(fr, "fr", analysis, SENT, "2026-10-03T09:00:00Z");
  assert.deepEqual(
    lines.map(plain),
    [
      "3 paliers, mises au point et retournement lus dans le journal de NINA",
      "Qualité de 96 poses ; guidage : erreur quadratique moyenne 0,82″ sur toute la nuit",
      "Remplace l’analyse du 3 oct. ; aucun palier corrigé à la main n’a été perdu",
    ].map(plain),
  );
  same(
    sentNightLine(fr, "fr", "2026-10-05", "Col des Étoiles"),
    "Nuit du 5 oct. · Col des Étoiles",
  );
  same(
    fr("logs.stay", { path: "logs/" }),
    "Les journaux restent dans logs/ sur cet ordinateur.",
  );
  assert.equal(
    logsState({
      reserved: false,
      hasNight: true,
      logs: LOGS,
      sending: false,
      sent: true,
    }),
    "sent",
  );
});

test("Journaux: the clock of the logs, found from the frames or given, is said once the report is sent", () => {
  const clock = {
    detected: true,
    phd2: "+02:00",
    asiair: "+02:00",
    nina: null,
    frames: 100,
    phd2MatchedFrames: 100,
    asiairMatchedFrames: 90,
    ninaMatchedFrames: null,
  };
  const analysis: NightAnalysis = {
    report: report(),
    skyMeasure: null,
    skyMissingReason: "no_reference_stars",
    logsUtcOffset: clock,
  };
  const lines = sentLines(fr, "fr", analysis, SENT, "2026-10-03T09:00:00Z");
  assert.equal(
    plain(lines[3] ?? ""),
    plain(
      "Heure des journaux : PHD2 +02:00, ASIAIR +02:00, trouvée d’après l’heure de vos poses",
    ),
  );
  assert.equal(
    plain(logsClockLine(fr, { ...clock, detected: false }) ?? ""),
    plain(
      "Heure des journaux : PHD2 +02:00, ASIAIR +02:00, comme indiqué dans les Réglages",
    ),
  );
  assert.equal(logsClockLine(fr, null), null, "no log, no clock");
  assert.equal(logsClockLine(fr, { ...clock, phd2: null, asiair: null }), null);
});

test("Journaux: no log in logs/ (an unknown file does not count), the primary greyed", () => {
  const unknown: DetectedLog[] = [
    { relativePath: "logs/notes.txt", kind: "unknown", bytes: 10 },
  ];
  assert.equal(
    logsState({
      reserved: false,
      hasNight: true,
      logs: unknown,
      sending: false,
      sent: false,
    }),
    "none",
  );
  assert.equal(
    logsState({
      reserved: false,
      hasNight: true,
      logs: [],
      sending: false,
      sent: false,
    }),
    "none",
  );
  same(
    fr("logs.none", { path: "logs/" }),
    "Aucun journal dans logs/ : déposez-y vos journaux NINA, PHD2 ou ASIAIR.",
  );
});

test("Journaux: reserved without analysis.logs, never on a private server nor before the space answers", () => {
  const advanced: CapabilitiesState = {
    paired: true,
    capabilities: CAPABILITIES_ADVANCED,
  };
  const plus: CapabilitiesState = {
    paired: true,
    capabilities: CAPABILITIES_PLUS,
  };
  assert.equal(isReserved(advanced, "analysis.logs"), true);
  assert.equal(isReserved(plus, "analysis.logs"), false);
  assert.equal(
    isReserved({ paired: false, capabilities: null }, "analysis.logs"),
    false,
  );
  assert.equal(isReserved(null, "analysis.logs"), false);
  assert.equal(
    logsState({
      reserved: true,
      hasNight: true,
      logs: LOGS,
      sending: false,
      sent: false,
    }),
    "reserved",
  );
  same(
    fr("logs.reserved"),
    "L’envoi des journaux est réservé à Synchronisation avancée+.",
  );
});

test("the head of Journaux reads word for word", () => {
  same(fr("logs.headTitle"), "Faites analyser les journaux de votre nuit");
  same(
    fr("logs.headLine"),
    "Astro Sync lit vos journaux NINA, PHD2 et ASIAIR sur cet ordinateur : paliers, mises au point, guidage. Seul le rapport part ; les journaux bruts restent ici.",
  );
  same(
    fr("logs.headNote"),
    "Une analyse par nuit : la nouvelle remplace la précédente.",
  );
  same(fr("logs.send"), "Analyser et envoyer le rapport");
  same(fr("logs.openFolder"), "Ouvrir le dossier logs");
});
