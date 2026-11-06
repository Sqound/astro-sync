import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

import { createTranslator, SUPPORTED_LOCALES } from "../i18n/locale.ts";
import type { BulkCreateOutcome, Proposal, Settled } from "./bridge";
import { treatmentNightFolders } from "./detectTreatments.ts";
import {
  applyRecap,
  type ApplyDeps,
  type ApplyHooks,
  type ApplyInput,
} from "./importApply.ts";
import {
  NO_IMPORT,
  importReducer,
  nightsWithLogs,
  recapPlan,
  recapStages,
  type Created,
  type ImportState,
  type LogsResult,
} from "./importFlow.ts";

// ASTRO-SYNC-SEND-01. A night sent by the import (« Détecter les sessions »)
// arrived without its logs: no guiding curve, no chronology of the poses, no
// acquisition step on its page, and « Les journaux de cette nuit ne sont pas
// encore envoyés ». The import created the sessions, wrote the incidents and
// queued the images, but never sent the report of the logs that « Journaux »
// (ticked by default) sends from the night's wizard. Step 4's primary now
// sends it, last, for each night that holds logs; a report that fails is
// said and stops nothing.

const session = (detectedId: string, sessionId: string, date: string) => ({
  detectedId,
  sessionId,
  eveningDate: date,
  action: "created" as const,
  planAction: "created" as const,
  planId: null,
  designations: ["Sh2-103"],
  frames: 0,
  shiftHours: 0,
});

const PROPOSAL = {
  sessions: [
    { id: "d1", eveningDate: "2026-08-07", targets: [], logs: [{}, {}] },
    { id: "d2", eveningDate: "2026-08-10", targets: [], logs: [] },
    { id: "d3", eveningDate: "2026-08-12", targets: [], logs: [{}] },
  ],
  lines: [],
  places: [],
} as unknown as Proposal;

const LOCAL_OUTCOME: BulkCreateOutcome = {
  sessions: [
    session("d1", "local-aaaaaaaaaaaa", "2026-08-07"),
    session("d2", "local-bbbbbbbbbbbb", "2026-08-10"),
    session("d3", "local-cccccccccccc", "2026-08-12"),
  ],
  multiNight: [],
  placesCreated: [],
};
const CREATED_OUTCOME: BulkCreateOutcome = {
  sessions: [
    session("d1", "sess-0807", "2026-08-07"),
    session("d2", "sess-0810", "2026-08-10"),
    session("d3", "sess-0812", "2026-08-12"),
  ],
  multiNight: [],
  placesCreated: [],
};
const REQUEST = {
  proposal: PROPOSAL,
  ticked: [],
  places: [],
  answers: [],
  shifts: [],
};
const LOCAL: Created = { request: REQUEST, outcome: LOCAL_OUTCOME };
const NIGHTS: Settled[] = [
  {
    fromId: "local-aaaaaaaaaaaa",
    toId: "sess-0807",
    from: "/w/2026-08-07 · Hilltop · local-aaaaaa",
    to: "/w/2026-08-07 · Hilltop · sess-0",
  },
  {
    fromId: "local-cccccccccccc",
    toId: "sess-0812",
    from: "/w/2026-08-12 · Hilltop · local-cccccc",
    to: "/w/2026-08-12 · Hilltop · sess-1",
  },
];
const LOGS = [
  {
    sessionId: "local-aaaaaaaaaaaa",
    folder: "/w/2026-08-07 · Hilltop · local-aaaaaa",
  },
  {
    sessionId: "local-cccccccccccc",
    folder: "/w/2026-08-12 · Hilltop · local-cccccc",
  },
];

function deps(trace: string[], over: Partial<ApplyDeps> = {}): ApplyDeps {
  return {
    create: async () => {
      trace.push("create");
      return { outcome: CREATED_OUTCOME, nights: NIGHTS };
    },
    settlePlans: async () => [],
    incidentsApply: async () => ({}) as never,
    workingFolder: async () => "/w",
    organisePlan: async () => ({}) as never,
    organise: async () => ({}) as never,
    enqueue: async (images) => {
      trace.push(`enqueue:${images.length}`);
    },
    sendLogs: async (sessionId, folder) => {
      trace.push(`logs:${sessionId}:${folder}`);
    },
    ...over,
  };
}

function hooks() {
  const log: string[] = [];
  const logs: Record<string, LogsResult> = {};
  const run: ApplyHooks = {
    phase: (stage) => log.push(stage),
    created: () => undefined,
    plansDone: () => undefined,
    incidentsDone: () => undefined,
    tidyDone: () => undefined,
    logsDone: (sessionId, result) => {
      logs[sessionId] = result;
    },
  };
  return { log, logs, run };
}

const start = (over: Partial<ApplyInput> = {}): ApplyInput => ({
  local: LOCAL,
  created: null,
  plan: { incidents: [], tidy: null, logs: LOGS },
  images: [],
  report: { plans: null, incidents: {}, tidy: null, logs: {} },
  sent: false,
  ...over,
});

test("the primary sends the report of each night's logs, last, to the session created, from its folder renamed", async () => {
  const trace: string[] = [];
  const run = hooks();
  const end = await applyRecap(deps(trace), start(), run.run);
  assert.equal(end.ok, true);
  assert.deepEqual(trace, [
    "create",
    "logs:sess-0807:/w/2026-08-07 · Hilltop · sess-0",
    "logs:sess-0812:/w/2026-08-12 · Hilltop · sess-1",
  ]);
  assert.deepEqual(run.log, ["sessions", "logs"]);
  assert.deepEqual(run.logs, {
    "sess-0807": { sent: true },
    "sess-0812": { sent: true },
  });
});

test("a report of the logs that fails is said and stops nothing: the other nights leave, the import ends", async () => {
  const trace: string[] = [];
  const run = hooks();
  const end = await applyRecap(
    deps(trace, {
      sendLogs: async (sessionId) => {
        if (sessionId === "sess-0807")
          throw { code: "analysis_too_large", message: "Too large." };
        trace.push(`logs:${sessionId}`);
      },
    }),
    start(),
    run.run,
  );
  assert.equal(end.ok, true);
  assert.deepEqual(trace, ["create", "logs:sess-0812"]);
  assert.deepEqual(run.logs["sess-0807"], {
    sent: false,
    reason: "Too large.",
  });
  assert.deepEqual(run.logs["sess-0812"], { sent: true });
});

test("a second try sends only the reports that did not leave; a space that keeps no report gets none", async () => {
  const trace: string[] = [];
  await applyRecap(
    deps(trace),
    start({
      created: { ...LOCAL, outcome: CREATED_OUTCOME, nights: NIGHTS },
      report: {
        plans: null,
        incidents: {},
        tidy: null,
        logs: {
          "sess-0807": { sent: true },
          "sess-0812": { sent: false, reason: "cut" },
        },
      },
    }),
    hooks().run,
  );
  assert.deepEqual(trace, ["logs:sess-0812:/w/2026-08-12 · Hilltop · sess-1"]);
  const held: string[] = [];
  const run = hooks();
  await applyRecap(deps(held, { sendLogs: undefined }), start(), run.run);
  assert.deepEqual(held, ["create"]);
  assert.deepEqual(run.log, ["sessions"]);
});

test("the nights whose logs leave: each night of the proposal with logs, once the treatments prepared its folder", () => {
  const folders = treatmentNightFolders([
    { detectedId: "d1", nightFolder: LOGS[0].folder } as never,
    {
      detectedId: "d2",
      nightFolder: "/w/2026-08-10 · Hilltop · local-bbbbbb",
    } as never,
  ]);
  let state: ImportState = {
    ...NO_IMPORT,
    alive: true,
    proposal: PROPOSAL,
    local: LOCAL,
  };
  state = importReducer(state, {
    type: "treatFolders",
    folders: {},
    nights: folders,
  });
  // d2 has no logs; d3 was not prepared (no treatment): neither leaves.
  assert.deepEqual(nightsWithLogs(state), [LOGS[0]]);
  const plan = recapPlan(state, " · ");
  assert.deepEqual(plan.logs, [LOGS[0]]);
  assert.ok(recapStages(state, plan).includes("logs"));
  // « Terminé » keeps what each report came to.
  state = importReducer(state, {
    type: "logsSent",
    sessionId: "sess-0807",
    result: { sent: true },
  });
  assert.deepEqual(state.report.logs, { "sess-0807": { sent: true } });
});

test("the provider sends the logs from step 4's primary, held only when the space keeps no report, and says it in the ten languages", () => {
  const provider = readFileSync(
    new URL("../components/ImportProvider.tsx", import.meta.url),
    "utf8",
  );
  const apply = provider.slice(
    provider.indexOf("const apply = useCallback"),
    provider.indexOf("const send = useCallback"),
  );
  assert.match(apply, /nightSendLogs\(sessionId, folder\)/);
  assert.match(apply, /isAnalysisSendingHeld\(rights\.current\)/);
  assert.match(apply, /logsDone: \(sessionId, result\) =>/);
  assert.match(provider, /nights: treatmentNightFolders\(nights\)/);
  const english = createTranslator("en");
  for (const locale of SUPPORTED_LOCALES) {
    const t = createTranslator(locale);
    for (const key of [
      "import.recap.working.logs",
      "import.recap.done.logs",
      "import.done.logsSent",
      "import.done.logsFailed",
    ]) {
      assert.notEqual(t(key), key, `${locale} lacks ${key}`);
      if (locale !== "en")
        assert.notEqual(t(key), english(key), `${locale} ${key} is English`);
    }
  }
});
