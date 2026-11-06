import assert from "node:assert/strict";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import test, { before } from "node:test";
import { fileURLToPath } from "node:url";

import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { rolldown } from "rolldown";

import { createTranslator, SUPPORTED_LOCALES } from "../i18n/locale.ts";
import type {
  IncidentsApplied,
  IncidentsApply,
  OrganiseAction,
  OrganiseOutcome,
  OrganisePlan,
} from "./bridge";
import {
  applyRecap,
  type ApplyDeps,
  type ApplyHooks,
  type ApplyInput,
} from "./importApply.ts";
import {
  NO_IMPORT,
  footRule,
  importReducer,
  recapLine,
  recapPlan,
  type ImportAction,
  type ImportState,
} from "./importFlow.ts";
import type { Genre, Line } from "./incidents.ts";
import {
  EMPTY_DRAFT,
  draftLines,
  draftPlan,
  draftReducer,
  type IncidentsDraft,
} from "./incidentsDraft.ts";
import {
  planToChange,
  tidyReportLines,
  tidyResultOf,
  tidyStateOf,
} from "./tidy.ts";
import type { UploadRow } from "./uploads.ts";

// On step 4 of the import, what is ticked goes with the step's
// primary (it is applied when the person goes to the next step): the incidents of
// every night, the tidying; no card writes by
// itself; « Terminé » reports what was done.

const fr = createTranslator("fr");
const T0 = Date.UTC(2026, 7, 10, 20, 0, 0) / 1000;

function line(id: string, minutes: number, genre: Genre = "note"): Line {
  return {
    id,
    origin: "detected",
    genre,
    start: T0 + minutes * 60,
    end: null,
    text: id,
    payload: { text: id },
    evidence: null,
    eventId: null,
    recordedAt: null,
  };
}

function declared(id: string, minutes: number): Line {
  return { ...line(id, minutes), origin: "declared", eventId: id };
}

const SESSIONS = [
  { sessionId: "s-aug10", detectedId: "d1" },
  { sessionId: "s-aug11", detectedId: "d2" },
];

/** Step 4 before its primary: what would be created is known, nothing
 * is created yet. */
const LOCAL = {
  request: {} as never,
  outcome: { sessions: SESSIONS, multiNight: [] } as never,
};

function base(): ImportState {
  return {
    ...NO_IMPORT,
    alive: true,
    step: 4,
    reached: 4,
    treated: true,
    local: LOCAL,
    ticked: ["img-1"],
  };
}

function play(actions: ImportAction[], from: ImportState = base()) {
  return actions.reduce(importReducer, from);
}

const load = (sessionId: string, lines: Line[]): ImportAction => ({
  type: "incidents",
  sessionId,
  action: { type: "loaded", base: lines, unproposed: [] },
});

const READ = [
  load("s-aug10", [line("a1", 10), line("a2", 60), line("a3", 120)]),
  load("s-aug11", [line("b1", 15), line("b2", 200)]),
];

/* --------------------------- The store, the plan --------------------------- */

test("the primary applies the incidents ticked of every night, not only the one shown", () => {
  const plan = recapPlan(play(READ), "; ");
  assert.deepEqual(
    plan.incidents.map((night) => night.sessionId),
    ["s-aug10", "s-aug11"],
  );
  assert.equal(plan.incidentCount, 5);
  assert.deepEqual(
    plan.incidents[0].plan.adds.map((add) => add.key),
    ["a1", "a2", "a3"],
  );
  assert.deepEqual(
    plan.incidents[1].plan.adds.map((add) => add.key),
    ["b1", "b2"],
  );
  assert.equal(plan.isolated, 5);
});

test("an incident unticked is not written; all unticked, nothing is", () => {
  const lines = (id: string) => draftLines(play(READ).incidents[id]);
  const one = play([
    ...READ,
    {
      type: "incidents",
      sessionId: "s-aug10",
      action: { type: "tick", lines: lines("s-aug10"), id: "a2", value: false },
    },
  ]);
  const plan = recapPlan(one, "; ");
  assert.deepEqual(
    plan.incidents[0].plan.adds.map((add) => add.key),
    ["a1", "a3"],
  );
  assert.equal(plan.incidentCount, 4);
  const none = play([
    ...READ,
    ...["s-aug10", "s-aug11"].flatMap((sessionId) =>
      lines(sessionId).map((l): ImportAction => ({
        type: "incidents",
        sessionId,
        action: {
          type: "tick",
          lines: lines(sessionId),
          id: l.id,
          value: false,
        },
      })),
    ),
  ]);
  const nothing = recapPlan(none, "; ");
  assert.deepEqual(nothing.incidents, []);
  assert.equal(nothing.incidentCount, 0);
  assert.equal(nothing.tidy, null);
});

test("a line added by hand is ticked and written with the others", () => {
  const manual = { ...line("manual:1", 30), manual: true };
  const state = play([
    ...READ,
    {
      type: "incidents",
      sessionId: "s-aug11",
      action: { type: "addManual", line: manual },
    },
  ]);
  assert.deepEqual(
    recapPlan(state, "; ").incidents[1].plan.adds.map((add) => add.key),
    ["b1", "manual:1", "b2"],
  );
});

test("the tidying is unticked by default, ticked it goes with the primary", () => {
  const todo = {
    kind: "todo" as const,
    inPlace: 0,
    toDo: 558,
    skipped: 0,
    inPlaceAs: null,
  };
  const state = play([{ type: "tidyOverview", overview: todo }]);
  assert.equal(state.tidy.on, false);
  assert.equal(recapPlan(state, "").tidy, null);
  const ticked = importReducer(state, { type: "tidyTick", on: true });
  const plan = recapPlan(ticked, "");
  assert.equal(plan.tidy?.on, true);
  assert.equal(plan.tidy?.mode, "move");
  assert.equal(plan.tidyFiles, 558);
  // The sheet records a choice, it moves nothing.
  const copied = importReducer(ticked, {
    type: "tidyChoice",
    choice: { on: true, mode: "copy", destination: "/Astro" },
  });
  assert.deepEqual(recapPlan(copied, "").tidy, {
    on: true,
    mode: "copy",
    destination: "/Astro",
  });
});

test("the confirmation line lists the images, the incidents and the tidying", () => {
  const todo = {
    kind: "todo" as const,
    inPlace: 0,
    toDo: 3,
    skipped: 0,
    inPlaceAs: null,
  };
  const state = play([
    ...READ,
    { type: "tidyOverview", overview: todo },
    { type: "tidyTick", on: true },
  ]);
  assert.equal(
    recapLine(recapPlan(state, ""), fr, "fr"),
    "En continuant\u00a0: 1 image envoyée · 5 incidents ajoutés · poses rangées dans le dossier de travail.",
  );
  const empty = { ...base(), ticked: [] };
  assert.equal(
    recapLine(recapPlan(empty, ""), fr, "fr"),
    "En continuant, rien n’est envoyé, ajouté ni rangé.",
  );
});

test("leaving step 4 with « Précédent » keeps the ticks and the option in the store", () => {
  const lines = draftLines(play(READ).incidents["s-aug10"]);
  const state = play([
    ...READ,
    {
      type: "incidents",
      sessionId: "s-aug10",
      action: { type: "tick", lines, id: "a1", value: false },
    },
    { type: "tidyTick", on: true },
    { type: "goTo", step: 3 },
  ]);
  assert.equal(state.step, 3);
  const back = importReducer(state, { type: "goTo", step: 4 });
  assert.equal(back.step, 4);
  assert.equal(back.tidy.on, true);
  assert.equal(back.incidents["s-aug10"].ticks.a1, false);
  assert.equal(back.incidents["s-aug10"].ticks.a2, true);
  assert.equal(back.incidents["s-aug10"].status, "ready");
});

test("the primary waits while a list of incidents is being read", () => {
  const loading = play([
    { type: "incidents", sessionId: "s-aug10", action: { type: "loading" } },
  ]);
  assert.equal(footRule(loading).primary?.enabled, false);
  assert.equal(footRule(play(READ)).primary?.enabled, true);
});

test("the draft reducer: a pair keeps one case ticked, « Recharger » drops the edits and keeps the ticks", () => {
  const mine = declared("pause-d", 14);
  const found = line("pause-a", 13);
  let draft: IncidentsDraft = draftReducer(EMPTY_DRAFT, {
    type: "loaded",
    base: [mine, found],
    unproposed: [],
  });
  assert.deepEqual(draft.ticks, { "pause-d": true, "pause-a": false });
  const lines = draftLines(draft);
  // Unticking the only ticked case of a pair is refused.
  draft = draftReducer(draft, {
    type: "tick",
    lines,
    id: "pause-d",
    value: false,
  });
  assert.equal(draft.ticks["pause-d"], true);
  draft = draftReducer(draft, { type: "edit", line: { ...found, text: "x" } });
  assert.equal(draftLines(draft).find((l) => l.id === "pause-a")?.text, "x");
  draft = draftReducer(draft, { type: "reload" });
  assert.equal(draft.status, "idle");
  assert.deepEqual(draft.edits, {});
  assert.equal(draft.ticks["pause-d"], true);
  // Replaced by its detection: the plan names the signalement it replaces.
  const both = draftReducer(
    draftReducer(EMPTY_DRAFT, {
      type: "loaded",
      base: [mine, found],
      unproposed: [],
    }),
    {
      type: "quick",
      lines,
      choice: "detections",
      canReplace: true,
    },
  );
  const plan = draftPlan(both, " · détecté : ");
  assert.equal(plan.replacements.length, 1);
  assert.equal(plan.adds.length, 0);
  assert.equal(plan.replacements[0].eventId, "pause-d");
});

/* ------------------------------- The writing ------------------------------- */

function applied(added: number): IncidentsApplied {
  return {
    added: Array.from({ length: added }, (_, i) => ({
      key: `k${i}`,
      eventId: `e${i}`,
      resumeEventId: null,
    })),
    duplicate: [],
    rejected: [],
    replaced: [],
    replacementsRefused: null,
  };
}

function outcome(done: number, alreadyThere: number): OrganiseOutcome {
  return {
    done,
    verified: done,
    removedSources: 0,
    alreadyThere,
    failed: [],
    stopped: false,
  };
}

function linkAction(index: number, how?: "link" | "file"): OrganiseAction {
  return {
    source: `/raw/l${index}.fit`,
    destination: `/work/2026-08-10/IC 1805/lights/l${index}.fit`,
    kind: "already-there",
    bytes: 10,
    inPlaceAs: how,
  };
}

function planOf558(kind: "already-there" | "link"): OrganisePlan {
  const actions: OrganiseAction[] = Array.from({ length: 558 }, (_, i) => ({
    ...linkAction(i, "link"),
    kind,
    inPlaceAs: kind === "already-there" ? "link" : undefined,
  }));
  return {
    mode: "symlink",
    destination: "/work",
    actions,
    conflicts: [],
    files: actions.length,
    bytes: actions.length * 10,
    windowsNoLink: false,
    summary: "",
  };
}

function deps(trace: string[], overrides: Partial<ApplyDeps> = {}): ApplyDeps {
  return {
    create: async () => {
      trace.push("create");
      return { outcome: LOCAL.outcome, nights: [] };
    },
    settlePlans: async () => {
      trace.push("plans");
      return [];
    },
    enqueue: async () => {
      trace.push("enqueue");
    },
    incidentsApply: async (request: IncidentsApply) => {
      trace.push(`incidents:${request.sessionId}:${request.adds.length}`);
      return applied(request.adds.length);
    },
    workingFolder: async () => "/work",
    organisePlan: async () => planOf558("already-there"),
    organise: async () => {
      trace.push("organise");
      return outcome(0, 558);
    },
    ...overrides,
  };
}

function hooks(state: ImportState) {
  const log: string[] = [];
  const next = { report: state.report, created: state.created };
  const hooks: ApplyHooks = {
    phase: (w: string) => log.push(`phase:${w}`),
    created: (created) => {
      next.created = created;
    },
    plansDone: (plans) => {
      next.report = { ...next.report, plans };
    },
    incidentsDone: (sessionId, result) => {
      next.report = {
        ...next.report,
        incidents: { ...next.report.incidents, [sessionId]: result },
      };
    },
    tidyDone: (result) => {
      next.report = { ...next.report, tidy: result };
    },
  };
  return {
    log,
    hooks,
    report: () => next.report,
    created: () => next.created,
  };
}

/** What the primary starts from: the state, its plan, no image. */
function input(
  state: ImportState,
  plan: ApplyInput["plan"],
  over: Partial<ApplyInput> = {},
): ApplyInput {
  return {
    local: state.local!,
    created: state.created,
    plan,
    images: [],
    report: state.report,
    sent: false,
    ...over,
  };
}

test("the primary writes the incidents of every night, then the tidying, and reports both", async () => {
  const state = play([...READ, { type: "tidyTick", on: true }]);
  const plan = recapPlan(state, "; ");
  const trace: string[] = [];
  const run = hooks(state);
  const end = await applyRecap(
    deps(trace, {
      organisePlan: async () => ({
        ...planOf558("link"),
        actions: planOf558("link").actions.slice(0, 3),
        files: 3,
      }),
      organise: async () => {
        trace.push("organise");
        return outcome(3, 0);
      },
    }),
    input(state, plan),
    run.hooks,
  );
  assert.equal(end.ok, true);
  // The sessions first: nothing was written before step 4.
  assert.deepEqual(trace, [
    "create",
    "incidents:s-aug10:3",
    "incidents:s-aug11:2",
    "organise",
  ]);
  assert.equal(run.report().incidents["s-aug10"].added, 3);
  assert.equal(run.report().incidents["s-aug11"].added, 2);
  assert.equal(run.report().tidy?.created, 3);
  assert.deepEqual(run.log, [
    "phase:sessions",
    "phase:incidents",
    "phase:tidy",
  ]);
});

test("nothing ticked: the sessions are created, nothing else is written", async () => {
  const state = base();
  const trace: string[] = [];
  const run = hooks(state);
  const end = await applyRecap(
    deps(trace),
    input(state, recapPlan(state, "")),
    run.hooks,
  );
  assert.equal(end.ok, true);
  assert.deepEqual(trace, ["create"]);
  assert.equal(run.report().tidy, null);
  assert.deepEqual(run.report().incidents, {});
});

test("a night whose lights are already linked: the engine is not asked, 558 are reported in place and 0 created", async () => {
  const state = play([{ type: "tidyTick", on: true }]);
  const trace: string[] = [];
  const run = hooks(state);
  await applyRecap(deps(trace), input(state, recapPlan(state, "")), run.hooks);
  assert.deepEqual(trace, ["create"], "organise is not run");
  const result = run.report().tidy;
  assert.equal(result?.created, 0);
  assert.equal(result?.inPlace, 558);
  assert.equal(result?.inPlaceAs, "link");
  const lines = tidyReportLines(fr, "fr", result!);
  assert.equal(lines.length, 1);
  assert.match(lines[0], /558/);
  assert.match(lines[0], /déjà rangées/);
  assert.match(lines[0], /liens symboliques/);
  assert.doesNotMatch(lines.join(" "), /\b0 lien/);
});

test("the option reports « déjà rangé » when every file is in place, and offers nothing to tick", () => {
  const state = tidyStateOf(planOf558("already-there"));
  assert.equal(state.kind, "done");
  assert.equal(state.inPlace, 558);
  assert.equal(state.toDo, 0);
  assert.equal(state.inPlaceAs, "link");
  // Even unticked, the check is reported in « Terminé ».
  const overview = play([{ type: "tidyOverview", overview: state }]);
  assert.notEqual(recapPlan(overview, "").tidy, null);
  assert.equal(recapPlan(overview, "").tidyFiles, 0);
  assert.equal(
    recapLine(recapPlan(overview, ""), fr, "fr"),
    "En continuant\u00a0: 1 image envoyée.",
  );
  assert.equal(
    fr("import.tidy.done", { how: fr("import.tidy.how.link") }),
    "Vos poses sont déjà rangées dans le dossier de travail (liens symboliques).",
  );
});

test("partly in place: the counts are said and the sheet lists only what would change", () => {
  const plan = planOf558("already-there");
  plan.actions[0] = { ...plan.actions[0], kind: "link", inPlaceAs: undefined };
  plan.actions[1] = { ...plan.actions[1], kind: "link", inPlaceAs: undefined };
  const state = tidyStateOf(plan);
  assert.deepEqual([state.kind, state.inPlace, state.toDo], ["partly", 556, 2]);
  const change = planToChange(plan);
  assert.equal(change.files, 2);
  assert.ok(change.actions.every((a) => a.kind !== "already-there"));
  assert.equal(
    fr("import.tidy.partly", {
      inPlace: 556,
      how: "liens symboliques",
      todo: 2,
    }),
    "Déjà rangés : 556 (liens symboliques) · à ranger : 2.",
  );
});

test("the report tells created, in place and skipped apart", () => {
  const plan = planOf558("already-there");
  plan.actions[0] = { ...plan.actions[0], kind: "link", inPlaceAs: undefined };
  plan.conflicts = [
    {
      source: "/raw/x.fit",
      destination: "/work/x.fit",
      reason: "destination-exists",
    },
  ];
  const result = tidyResultOf("symlink", plan, outcome(1, 557));
  const lines = tidyReportLines(fr, "fr", result);
  assert.equal(lines.length, 3);
  assert.match(lines[0], /^1 lien créé$/);
  assert.match(lines[1], /557/);
  assert.match(lines[2], /1 fichier laissé à sa place/);
  assert.match(lines[2], /existe déjà/);
});

test("a night that cannot be written stops the primary before the tidying and the images; the nights written are not written twice", async () => {
  const state = play([...READ, { type: "tidyTick", on: true }]);
  const plan = recapPlan(state, "");
  const trace: string[] = [];
  const run = hooks(state);
  const failing = deps(trace, {
    incidentsApply: async (request) => {
      trace.push(`incidents:${request.sessionId}`);
      if (request.sessionId === "s-aug11") throw new Error("offline");
      return applied(request.adds.length);
    },
  });
  const end = await applyRecap(failing, input(state, plan), run.hooks);
  assert.equal(end.ok, false);
  assert.equal(end.ok === false && end.stage, "incidents");
  assert.equal(end.ok === false && end.sessionId, "s-aug11");
  assert.deepEqual(trace, ["create", "incidents:s-aug10", "incidents:s-aug11"]);
  assert.deepEqual(Object.keys(run.report().incidents), ["s-aug10"]);
  // The second try: the sessions are not created again, the first night is
  // skipped.
  const again: string[] = [];
  const next = { ...state, report: run.report(), created: run.created() };
  const second = hooks(next);
  const done = await applyRecap(deps(again), input(next, plan), second.hooks);
  assert.equal(done.ok, true);
  assert.deepEqual(again, ["incidents:s-aug11:2"]);
});

/* ------------------------------ What is drawn ------------------------------ */

type Entry = {
  IncidentsCard: typeof import("../components/IncidentsCard.tsx").IncidentsCard;
  ImportDoneView: typeof import("../screens/detect/ImportDone.tsx").ImportDoneView;
  I18nProvider: typeof import("../i18n/index.tsx").I18nProvider;
};
let entry: Entry;

before(async () => {
  const stub = {
    name: "stub-css-and-entry",
    resolveId(id: string) {
      return id === "virtual:entry" ? id : null;
    },
    load(id: string) {
      if (id === "virtual:entry") {
        const base = fileURLToPath(new URL("../", import.meta.url));
        return {
          code: `export { IncidentsCard } from ${JSON.stringify(base + "components/IncidentsCard.tsx")};
export { ImportDoneView } from ${JSON.stringify(base + "screens/detect/ImportDone.tsx")};
export { I18nProvider } from ${JSON.stringify(base + "i18n/index.tsx")};`,
          moduleType: "js" as const,
        };
      }
      return id.endsWith(".css")
        ? { code: "export default '';", moduleType: "js" as const }
        : null;
    },
  };
  const bundle = await rolldown({
    input: "virtual:entry",
    platform: "node",
    plugins: [stub],
    external: ["react", "react/jsx-runtime", "react-dom"],
    logLevel: "silent",
  });
  const { output } = await bundle.generate({ format: "esm" });
  const folder = new URL(
    "../../node_modules/.cache/astro-sync-tests/",
    import.meta.url,
  );
  await mkdir(folder, { recursive: true });
  const file = new URL("import-recap.mjs", folder);
  await writeFile(file, output[0].code);
  entry = await import(file.href);
});

const flat = (html: string) =>
  html.replace(/<[^>]+>/g, "").replace(/\s+/g, " ");

function drawCard(withStep: "send" | "finish" | "create", ticked: boolean) {
  const lines = [line("a1", 10), line("a2", 60), line("a3", 120)];
  const ticks = Object.fromEntries(lines.map((l) => [l.id, ticked]));
  const noop = () => undefined;
  return renderToStaticMarkup(
    createElement(
      entry.I18nProvider,
      { locale: "fr", onLocaleChange: noop },
      createElement(entry.IncidentsCard, {
        view: {
          kind: "list",
          lines,
          ticks,
          canReplace: true,
          note: null,
          unproposed: [],
        },
        offset: 0,
        targetName: null,
        onTick: noop,
        onQuick: noop,
        onEdit: noop,
        onRemove: noop,
        onAdd: noop,
        onRetry: noop,
        onMeasure: noop,
        withStep,
      }),
    ),
  );
}

test("the card of the import has no « Ajouter N incidents à la session », keeps « + Ajouter un incident », and its foot says what the primary will do", () => {
  const html = drawCard("send", true);
  const text = flat(html);
  assert.doesNotMatch(text, /Ajouter \d+ incidents? à la session/);
  assert.doesNotMatch(html, /as-incidents__actions/);
  assert.doesNotMatch(html, /btn-secondary|as-button--secondary/);
  assert.match(text, /Ajouter un incident/);
  assert.match(text, /3 incidents seront ajoutés avec l’envoi \(3 isolés\)/);
  assert.equal((html.match(/type="checkbox"/g) ?? []).length, 3);
  assert.match(flat(drawCard("finish", true)), /seront ajoutés en terminant/);
  assert.match(flat(drawCard("create", true)), /seront ajoutés à la création/);
  assert.match(flat(drawCard("send", false)), /Aucun incident ne sera ajouté/);
});

function uploading(extra: Partial<UploadRow> = {}): UploadRow {
  return {
    sha256: "a".repeat(64),
    origin: "detect",
    batch: 1,
    fileName: "ic1805.tif",
    size: 100_000_000,
    target: "IC 1805",
    rendering: "Rendu naturel",
    status: "uploading",
    sent: 40_000_000,
    total: 100_000_000,
    speed: 5_000_000,
    lastAt: 1,
    captureId: null,
    deduplicated: false,
    code: null,
    message: null,
    retryInSeconds: null,
    retryable: false,
    limit: null,
    notice: false,
    ...extra,
  };
}

function drawDone(
  row: UploadRow,
  tidy: string[] | null = null,
  counts = { sent: 1, variants: 0 },
) {
  const noop = () => undefined;
  return renderToStaticMarkup(
    createElement(
      entry.I18nProvider,
      { locale: "fr", onLocaleChange: noop },
      createElement(entry.ImportDoneView, {
        view: {
          made: 2,
          updated: 0,
          plans: 0,
          sent: counts.sent,
          variants: counts.variants,
          incidents: [
            {
              night: "IC 1805 · 10 août",
              added: 3,
              rejected: 0,
              refused: false,
            },
            {
              night: "IC 1805 · 11 août",
              added: 2,
              rejected: 0,
              refused: false,
            },
          ],
          tidy,
          batch: [row],
          sky: {},
          web: null,
        },
        onRetrySky: noop,
        onOpenUploads: noop,
        onLeave: noop,
      }),
    ),
  );
}

test("« Terminé »: a running upload draws the bar of the page « Envois », then « Envoyée »; the page says the uploads go on", () => {
  const running = drawDone(uploading());
  assert.match(running, /role="progressbar"/);
  assert.match(running, /aria-valuenow="40"/);
  assert.match(flat(running), /40,0 Mo sur 100,0 Mo · 5,0 Mo\/s/);
  assert.doesNotMatch(flat(running), /Envoi en cours/);
  assert.match(
    flat(running),
    /Vous pouvez quitter cette page sans fermer l’application\s: les envois continuent et se retrouvent dans la page Envois\./,
  );
  assert.match(flat(running), /Voir les envois/);
  const done = drawDone(uploading({ status: "done", sent: 100_000_000 }));
  assert.doesNotMatch(done, /role="progressbar"/);
  assert.match(flat(done), /Envoyée/);
});

test("« Terminé » says how many of the images that left are variants", () => {
  const split = flat(drawDone(uploading(), null, { sent: 2, variants: 1 }));
  assert.match(split, /1 capture \+ 1 variante mises en file d’envoi/);
  const many = flat(drawDone(uploading(), null, { sent: 5, variants: 3 }));
  assert.match(many, /2 captures \+ 3 variantes mises en file d’envoi/);
  // No variant: the line is the one it always was.
  const plain = flat(drawDone(uploading(), null, { sent: 2, variants: 0 }));
  assert.match(plain, /2 images mises en file d’envoi/);
});

test("« Terminé » reports the incidents added per night and the files tidied", () => {
  const text = flat(
    drawDone(uploading({ status: "done" }), [
      "Poses déjà rangées : 558 (liens symboliques) · rien à créer",
    ]),
  );
  assert.match(text, /3 incidents ajoutés · IC 1805 · 10 août/);
  assert.match(text, /2 incidents ajoutés · IC 1805 · 11 août/);
  assert.match(text, /558 \(liens symboliques\) · rien à créer/);
  assert.match(
    flat(drawDone(uploading(), null)),
    /Poses laissées dans leur dossier/,
  );
});

/* ------------------------------- The wiring -------------------------------- */

const source = (relative: string) =>
  readFile(new URL(`../${relative}`, import.meta.url), "utf8");

test("no card of the recap writes by itself, and the primary applies the choices", async () => {
  const summary = await source("screens/detect/ImportSummary.tsx");
  assert.doesNotMatch(summary, /incidents\.add\b|incidents\.apply|onApply/);
  assert.doesNotMatch(summary, /detect\.files\.organise/);
  assert.doesNotMatch(summary, /sessionsOrganise\(/);
  assert.match(summary, /import\.tidy\.label/);
  assert.match(summary, /import\.tidy\.see/);
  assert.match(summary, /recapLine\(plan, t, locale\)/);
  const results = await source("screens/detect/ResultIncidents.tsx");
  assert.match(results, /withStep=\{withStep\}/);
  assert.doesNotMatch(results, /incidentsApply/);
  const dialog = await source("screens/detect/OrganiseDialog.tsx");
  assert.doesNotMatch(dialog, /sessionsOrganise\(|primary\(mode\)/);
  const provider = await source("components/ImportProvider.tsx");
  assert.match(provider, /applyRecap\(/);
  // « Créer et envoyer » / « Envoyer N images » and « Créer » / « Terminer »
  // go through it: the second one sends no image.
  assert.match(
    provider,
    /\(rendering: \(row: SummaryImage\) => string\) => apply\(rendering\)/,
  );
  assert.match(
    provider,
    /const finish = useCallback\(\(\) => apply\(null\), \[apply\]\)/,
  );
});

test("the words exist in the ten languages", () => {
  const keys = [
    "incidents.willAdd.send.one",
    "incidents.willAdd.none",
    "import.tidy.label",
    "import.tidy.see",
    "import.tidy.done",
    "import.recap.lead",
    "import.recap.none",
    "import.done.leave",
    "import.done.tidy.allInPlace",
  ];
  for (const locale of SUPPORTED_LOCALES) {
    const t = createTranslator(locale);
    for (const key of keys) {
      assert.notEqual(t(key), key, `${locale} ${key}`);
    }
  }
});
