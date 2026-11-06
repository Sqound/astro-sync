import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

import { createTranslator, SUPPORTED_LOCALES } from "../i18n/locale.ts";
import type {
  IncidentsApplied,
  IncidentsApply,
  MasterOutcome,
  NightPlanChange,
  PersonalTargetCreate,
  PlanCreated,
} from "./bridge";
import type { IncidentsPlan } from "./incidentsDraft.ts";
import {
  applyCibles,
  applyIncidents,
  applyMeasures,
  type ApplyContext,
  type ApplyHooks,
  type NightApplyDeps,
} from "./nightApply.ts";
import {
  NO_CHOICES,
  ciblesLine,
  ciblesPlan,
  ciblesWrite,
  incidentsLine,
  measuresLine,
  measuresPlan,
  measuresWrite,
  nightReducer,
  reportLines,
  targetsCount,
  type NightAction,
  type NightChoices,
} from "./nightRecap.ts";

// In the night's wizard the person chooses on a step and the
// step's primary applies it (it is applied when the person goes to the next
// step). The targets and the plan of « Cibles », the masters
// of « Traitements », the incidents of « Récapitulatif » are rows with a case;
// nothing is written before the primary, and the case is the undo.

const fr = createTranslator("fr");
const en = createTranslator("en");
const SESSION = "s-night";

const play = (actions: NightAction[], from: NightChoices = NO_CHOICES) =>
  actions.reduce(nightReducer, from);

const FORM: PersonalTargetCreate = {
  name: "Ma cible",
  label: "nebula",
  raHours: 20.9,
  decDegrees: 44.3,
};

const PICK_PLAN: Extract<NightAction, { type: "pickPlan" }>["pick"] = {
  kind: "attach",
  projectId: "p-heart",
  title: "IC 1805 — Nébuleuse du Cœur",
  facts: "3 nuits · 11 h 08",
  movesFromId: null,
  movesFromName: null,
};

const MASTER = {
  targetId: "t-m31",
  folderName: "M31",
  designation: "M31",
  source: "master" as const,
  file: "/home/astro/M31_master.xisf",
  fileName: "M31_master.xisf",
};

/* ------------------------------ The rows, the case ------------------------- */

test("a card of the catalogue adds a ticked row, once; its case is the undo, the card adds it again", () => {
  const added = play([
    { type: "pickCatalogue", targetId: "t-ngc7000", designation: "NGC 7000" },
    { type: "pickCatalogue", targetId: "t-ic1396", designation: "IC 1396" },
  ]);
  assert.deepEqual(
    added.targets.map((row) => [
      row.key,
      row.kind,
      row.designation,
      row.ticked,
    ]),
    [
      ["catalogue:t-ngc7000", "catalogue", "NGC 7000", true],
      ["catalogue:t-ic1396", "catalogue", "IC 1396", true],
    ],
  );
  const undone = play(
    [{ type: "tickTarget", key: "catalogue:t-ngc7000", on: false }],
    added,
  );
  assert.deepEqual(
    undone.targets.map((row) => row.ticked),
    [false, true],
  );
  // The same card again: no second row, the first one is ticked.
  const again = play(
    [
      {
        type: "pickCatalogue",
        targetId: "t-ngc7000",
        designation: "NGC 7000",
      },
    ],
    undone,
  );
  assert.equal(again.targets.length, 2);
  assert.equal(again.targets[0].ticked, true);
});

test("the form of « Nouvelle cible » is recorded as a row to create, numbered", () => {
  const state = play([
    { type: "pickNew", create: FORM },
    { type: "pickNew", create: { ...FORM, name: "Autre" } },
  ]);
  assert.deepEqual(
    state.targets.map((row) => [row.key, row.kind, row.designation]),
    [
      ["new:1", "create", "Ma cible"],
      ["new:2", "create", "Autre"],
    ],
  );
  assert.deepEqual(state.targets[0].create, FORM);
  assert.equal(state.targets[0].targetId, null);
});

test("the plan chosen is a row with its case; « Détacher » is a case that keeps only what is ticked", () => {
  const picked = play([{ type: "pickPlan", pick: PICK_PLAN }]);
  assert.equal(picked.plan?.ticked, true);
  assert.equal(
    play([{ type: "tickPlan", on: false }], picked).plan?.ticked,
    false,
  );
  // Another pick replaces the first.
  const created = play(
    [
      {
        type: "pickPlan",
        pick: {
          kind: "create",
          target: { targetId: "t-ic1805", name: "IC 1805", commonName: null },
          title: "IC 1805",
        },
      },
    ],
    picked,
  );
  assert.equal(created.plan?.kind, "create");
  const leaving = play([
    { type: "tickDetach", projectId: "p-1", title: "IC 1848", on: true },
    { type: "tickDetach", projectId: "p-2", title: "IC 1805", on: true },
    { type: "tickDetach", projectId: "p-1", title: "IC 1848", on: false },
  ]);
  assert.deepEqual(leaving.detach, [{ projectId: "p-2", title: "IC 1805" }]);
});

test("a master is one choice per target (the figures kept are one set); « Retirer la mesure » is a case", () => {
  const state = play([
    { type: "pickMeasure", pick: MASTER },
    {
      type: "pickMeasure",
      pick: { ...MASTER, source: "device", file: "/n/M31/device/m31.fits" },
    },
  ]);
  assert.deepEqual(Object.keys(state.measures), ["t-m31"]);
  assert.equal(state.measures["t-m31"].source, "device");
  assert.equal(
    play([{ type: "tickMeasure", targetId: "t-m31", on: false }], state)
      .measures["t-m31"].ticked,
    false,
  );
  const forget = { targetId: "t-m31", folderName: "M31", designation: "M31" };
  assert.deepEqual(
    play([{ type: "tickForget", pick: forget, on: true }]).forget,
    [forget],
  );
  assert.deepEqual(
    play([
      { type: "tickForget", pick: forget, on: true },
      { type: "tickForget", pick: forget, on: false },
    ]).forget,
    [],
  );
});

/* ------------------------------ What a step applies ------------------------ */

test("« Cibles » applies the ticked rows only, in the order creations, targets of the catalogue, plan, plans left", () => {
  const state = play([
    { type: "pickCatalogue", targetId: "t-ngc7000", designation: "NGC 7000" },
    { type: "pickCatalogue", targetId: "t-ic1396", designation: "IC 1396" },
    { type: "pickNew", create: FORM },
    { type: "tickTarget", key: "catalogue:t-ic1396", on: false },
    { type: "pickPlan", pick: PICK_PLAN },
    { type: "tickDetach", projectId: "p-old", title: "IC 1848", on: true },
  ]);
  const plan = ciblesPlan(state);
  assert.deepEqual(
    plan.creates.map((row) => row.designation),
    ["Ma cible"],
  );
  assert.deepEqual(
    plan.adds.map((row) => row.designation),
    ["NGC 7000"],
  );
  assert.equal(plan.plan?.kind, "attach");
  assert.deepEqual(plan.detach, [{ projectId: "p-old", title: "IC 1848" }]);
  // « Continuer avec N cibles » counts the ones that will join.
  assert.equal(targetsCount(2, plan), 4);
});

test("nothing ticked, nothing applied: the primary only goes on", () => {
  const state = play([
    { type: "pickCatalogue", targetId: "t-ngc7000", designation: "NGC 7000" },
    { type: "pickNew", create: FORM },
    { type: "pickPlan", pick: PICK_PLAN },
    { type: "tickTarget", key: "catalogue:t-ngc7000", on: false },
    { type: "tickTarget", key: "new:1", on: false },
    { type: "tickPlan", on: false },
  ]);
  assert.equal(ciblesWrite(ciblesPlan(state)), false);
  assert.equal(ciblesWrite(ciblesPlan(NO_CHOICES)), false);
  assert.equal(ciblesLine(ciblesPlan(state), fr, "fr"), null);
  assert.equal(measuresWrite(measuresPlan(NO_CHOICES)), false);
  assert.equal(measuresLine(measuresPlan(NO_CHOICES), fr), null);
  // A master left unticked is not measured.
  const off = play([
    { type: "pickMeasure", pick: MASTER },
    { type: "tickMeasure", targetId: "t-m31", on: false },
  ]);
  assert.equal(measuresWrite(measuresPlan(off)), false);
});

test("a plan the night leaves by joining another one is not left a second time", () => {
  const state = play([
    {
      type: "pickPlan",
      pick: { ...PICK_PLAN, movesFromId: "p-old", movesFromName: "IC 1848" },
    },
    { type: "tickDetach", projectId: "p-old", title: "IC 1848", on: true },
    { type: "tickDetach", projectId: "p-other", title: "M31", on: true },
  ]);
  assert.deepEqual(
    ciblesPlan(state).detach.map((item) => item.projectId),
    ["p-other"],
  );
});

test("a measure kept that is forgotten while a new one replaces it is not forgotten first", () => {
  const state = play([
    { type: "pickMeasure", pick: MASTER },
    {
      type: "tickForget",
      pick: { targetId: "t-m31", folderName: "M31", designation: "M31" },
      on: true,
    },
    {
      type: "tickForget",
      pick: { targetId: "t-m42", folderName: "M42", designation: "M42" },
      on: true,
    },
  ]);
  const plan = measuresPlan(state);
  assert.deepEqual(
    plan.forget.map((item) => item.targetId),
    ["t-m42"],
  );
  assert.deepEqual(
    plan.measures.map((item) => item.targetId),
    ["t-m31"],
  );
});

/* ------------------------------ The sentences above the foot --------------- */

test("the foot of « Cibles » says what « Continuer » will do, in the order it does it", () => {
  const state = play([
    { type: "pickCatalogue", targetId: "t-ngc7000", designation: "NGC 7000" },
    { type: "pickCatalogue", targetId: "t-ic1396", designation: "IC 1396" },
    { type: "pickNew", create: FORM },
    { type: "pickPlan", pick: PICK_PLAN },
    { type: "tickDetach", projectId: "p-old", title: "IC 1848", on: true },
  ]);
  assert.equal(
    ciblesLine(ciblesPlan(state), fr, "fr"),
    "En continuant : 1 cible créée et ajoutée à la nuit · 2 cibles ajoutées à la nuit · session rattachée au plan IC 1805 — Nébuleuse du Cœur · session détachée du plan IC 1848.",
  );
  assert.equal(
    ciblesLine(
      ciblesPlan(
        play([{ type: "pickCatalogue", targetId: "t", designation: "M31" }]),
      ),
      en,
      "en",
    ),
    "When you continue: 1 target added to the night.",
  );
  const create = play([
    {
      type: "pickPlan",
      pick: {
        kind: "create",
        target: { targetId: "t-ic1805", name: "IC 1805", commonName: null },
        title: "IC 1805",
      },
    },
  ]);
  assert.equal(
    ciblesLine(ciblesPlan(create), fr, "fr"),
    "En continuant : plan multi-nuits créé pour IC 1805, avec la session dedans.",
  );
});

test("the foot of « Traitements » names each master and each measure forgotten; « Récapitulatif » counts the incidents", () => {
  const state = play([
    { type: "pickMeasure", pick: MASTER },
    {
      type: "pickMeasure",
      pick: {
        ...MASTER,
        targetId: "t-ic1805",
        designation: "IC 1805",
        source: "device",
        folderName: "IC 1805",
      },
    },
    {
      type: "tickForget",
      pick: { targetId: "t-m42", folderName: "M42", designation: "M42" },
      on: true,
    },
  ]);
  assert.equal(
    measuresLine(measuresPlan(state), fr),
    "En continuant : mesure de M42 retirée · master de M31 mesuré · image de l’appareil de IC 1805 mesurée.",
  );
  assert.equal(
    incidentsLine(3, fr, "fr"),
    "En envoyant : 3 incidents ajoutés à la session.",
  );
  assert.equal(
    incidentsLine(1, en, "en"),
    "When you send: 1 incident added to the session.",
  );
  assert.equal(incidentsLine(0, fr, "fr"), null);
});

/* ------------------------------ Applying, in order ------------------------- */

interface Fake {
  calls: string[];
  deps: NightApplyDeps;
}

function fake(options: {
  fail?: string;
  plan?: Partial<Record<"attach" | "create" | "detach", unknown>>;
  master?: MasterOutcome;
}): Fake {
  const calls: string[] = [];
  const log = async <T>(call: string, value: T): Promise<T> => {
    calls.push(call);
    if (options.fail === call) throw new Error("boom");
    return value;
  };
  const deps: NightApplyDeps = {
    personalTargetCreate: (target) =>
      log(`create:${target.name}`, {
        target: { id: "t-new", name: target.name },
        nightTargetIds: null,
      }),
    nightTargetsChange: (_session, change) =>
      log(`${change.kind}:${change.targetId}`, []),
    nightPlanCreate: (_session, target) =>
      log(
        `planCreate:${target.targetId}`,
        (options.plan?.create ?? {
          result: "created",
          projectId: "p-new",
          name: "IC 1805",
          goalMinutes: 600,
          nights: 1,
        }) as PlanCreated,
      ),
    nightPlanAttach: (_session, projectId) =>
      log(
        `attach:${projectId}`,
        (options.plan?.attach ?? {
          result: "attached",
          projectId,
          nights: 2,
        }) as NightPlanChange,
      ),
    nightPlanDetach: (_session, projectId) =>
      log(
        `detach:${projectId}`,
        (options.plan?.detach ?? {
          result: "detached",
          projectId,
          nights: 1,
        }) as NightPlanChange,
      ),
    masterMeasure: (_session, folder, file) =>
      log(
        `measure:${folder}:${file}`,
        options.master ??
          ({
            result: "measured",
            master: "M31_master.xisf",
            quality: {
              fwhmPixels: 3.8,
              fwhmArcsec: 2.1,
              eccentricity: 0.31,
              starCount: 212,
            },
          } as never),
      ),
    masterMeasureForget: (_session, folder) =>
      log(`forget:${folder}`, undefined),
    incidentsApply: (request: IncidentsApply) =>
      log(`incidents:${request.adds.length}+${request.replacements.length}`, {
        added: request.adds.map((add) => add.key),
        duplicate: [],
        replaced: request.replacements.map((r) => r.eventId),
        rejected: [],
        replacementsRefused: null,
      } as unknown as IncidentsApplied),
  };
  return { calls, deps };
}

const CONTEXT: ApplyContext = {
  t: fr,
  locale: "fr",
  reason: (error) => (error instanceof Error ? error.message : String(error)),
};

/** The choices, then the wizard's hooks acting on them. */
function session(actions: NightAction[]) {
  let state = play(actions);
  const works: string[] = [];
  const send = (action: NightAction) => {
    state = nightReducer(state, action);
  };
  const applyHooks: ApplyHooks = {
    work: (work) => works.push(work),
    targetApplied: (key) => send({ type: "targetApplied", key }),
    planApplied: () => send({ type: "planApplied" }),
    detachApplied: (projectId, name) =>
      send({ type: "detachApplied", projectId, name }),
    measureApplied: (targetId, figures) =>
      send({ type: "measureApplied", targetId, figures }),
    forgetApplied: (targetId) => send({ type: "forgetApplied", targetId }),
  };
  return { state: () => state, works, hooks: applyHooks };
}

const EVERYTHING: NightAction[] = [
  { type: "pickCatalogue", targetId: "t-ngc7000", designation: "NGC 7000" },
  { type: "pickNew", create: FORM },
  { type: "pickPlan", pick: PICK_PLAN },
  { type: "tickDetach", projectId: "p-old", title: "IC 1848", on: true },
];

test("the primary of « Cibles » creates, then adds, then makes the plan, then leaves the old one; the report keeps it all", async () => {
  const run = fake({});
  const held = session(EVERYTHING);
  const end = await applyCibles(
    run.deps,
    SESSION,
    ciblesPlan(held.state()),
    CONTEXT,
    held.hooks,
  );
  assert.deepEqual(end, { ok: true });
  assert.deepEqual(run.calls, [
    "create:Ma cible",
    "add:t-ngc7000",
    "attach:p-heart",
    "detach:p-old",
  ]);
  assert.deepEqual(held.works, ["targets", "plan"]);
  const after = held.state();
  assert.equal(after.targets.length, 0);
  assert.equal(after.plan, null);
  assert.deepEqual(after.detach, []);
  assert.deepEqual(reportLines(after.report, fr, "fr"), [
    "Cible Ma cible créée et ajoutée à la nuit",
    "Cible NGC 7000 ajoutée à la nuit",
    "Session rattachée au plan IC 1805 — Nébuleuse du Cœur",
    "Session détachée du plan IC 1848",
  ]);
});

test("a plan to create is made after the targets, with the night in it", async () => {
  const run = fake({});
  const held = session([
    { type: "pickCatalogue", targetId: "t-ngc7000", designation: "NGC 7000" },
    {
      type: "pickPlan",
      pick: {
        kind: "create",
        target: { targetId: "t-ic1805", name: "IC 1805", commonName: null },
        title: "IC 1805",
      },
    },
  ]);
  await applyCibles(
    run.deps,
    SESSION,
    ciblesPlan(held.state()),
    CONTEXT,
    held.hooks,
  );
  assert.deepEqual(run.calls, ["add:t-ngc7000", "planCreate:t-ic1805"]);
  assert.deepEqual(reportLines(held.state().report, fr, "fr"), [
    "Cible NGC 7000 ajoutée à la nuit",
    "Plan multi-nuits créé pour IC 1805, avec la session dedans",
  ]);
});

test("nothing ticked: the service is not called at all", async () => {
  const run = fake({});
  const held = session([
    ...EVERYTHING,
    { type: "tickTarget", key: "catalogue:t-ngc7000", on: false },
    { type: "tickTarget", key: "new:1", on: false },
    { type: "tickPlan", on: false },
    { type: "tickDetach", projectId: "p-old", title: "IC 1848", on: false },
  ]);
  const end = await applyCibles(
    run.deps,
    SESSION,
    ciblesPlan(held.state()),
    CONTEXT,
    held.hooks,
  );
  assert.deepEqual(end, { ok: true });
  assert.deepEqual(run.calls, []);
  assert.deepEqual(held.works, []);
  assert.deepEqual(held.state().report, []);
  // The unticked rows are still there, to tick again.
  assert.equal(held.state().targets.length, 2);
});

test("a failure stops the primary; what was done stays done and a second try goes on from there", async () => {
  const failing = fake({ fail: "add:t-ngc7000" });
  const held = session(EVERYTHING);
  const end = await applyCibles(
    failing.deps,
    SESSION,
    ciblesPlan(held.state()),
    CONTEXT,
    held.hooks,
  );
  assert.deepEqual(end, {
    ok: false,
    failure: { kind: "target", name: "NGC 7000", reason: "boom" },
  });
  // The creation went through and left the list; nothing after the failure.
  assert.deepEqual(failing.calls, ["create:Ma cible", "add:t-ngc7000"]);
  assert.deepEqual(
    held.state().targets.map((row) => row.designation),
    ["NGC 7000"],
  );
  assert.equal(held.state().plan?.kind, "attach");
  // Unticked, it lets the person go on without it.
  const second = fake({});
  const without = session([
    ...EVERYTHING,
    { type: "targetApplied", key: "new:1" },
    { type: "tickTarget", key: "catalogue:t-ngc7000", on: false },
  ]);
  await applyCibles(
    second.deps,
    SESSION,
    ciblesPlan(without.state()),
    CONTEXT,
    without.hooks,
  );
  assert.deepEqual(second.calls, ["attach:p-heart", "detach:p-old"]);
});

test("the service refusing the plan is a failure said in clear, not a plan attached", async () => {
  const run = fake({
    plan: {
      attach: {
        result: "refused",
        projectId: "p-heart",
        code: "target_mismatch",
        message: "m",
      },
    },
  });
  const held = session([{ type: "pickPlan", pick: PICK_PLAN }]);
  const end = await applyCibles(
    run.deps,
    SESSION,
    ciblesPlan(held.state()),
    CONTEXT,
    held.hooks,
  );
  assert.deepEqual(end, {
    ok: false,
    failure: {
      kind: "plan",
      name: "IC 1805 — Nébuleuse du Cœur",
      reason: "cette session n’est pas de la cible de ce plan.",
    },
  });
  assert.equal(held.state().plan?.projectId, "p-heart");
  assert.deepEqual(held.state().report, []);
  const closed = fake({
    plan: { create: { result: "missing", missing: "place" } },
  });
  const made = session([
    {
      type: "pickPlan",
      pick: {
        kind: "create",
        target: { targetId: "t", name: "IC 1805", commonName: null },
        title: "IC 1805",
      },
    },
  ]);
  const second = await applyCibles(
    closed.deps,
    SESSION,
    ciblesPlan(made.state()),
    CONTEXT,
    made.hooks,
  );
  assert.equal(second.ok, false);
  assert.equal(
    !second.ok && second.failure.reason,
    fr("planAttach.missingPlace"),
  );
});

test("the primary of « Traitements » forgets the measures asked, then measures the masters; refused is a failure", async () => {
  const run = fake({});
  const held = session([
    { type: "pickMeasure", pick: MASTER },
    {
      type: "tickForget",
      pick: { targetId: "t-m42", folderName: "M42", designation: "M42" },
      on: true,
    },
  ]);
  const end = await applyMeasures(
    run.deps,
    SESSION,
    measuresPlan(held.state()),
    CONTEXT,
    held.hooks,
  );
  assert.deepEqual(end, { ok: true });
  assert.deepEqual(run.calls, [
    "forget:M42",
    "measure:M31:/home/astro/M31_master.xisf",
  ]);
  assert.deepEqual(held.works, ["measure"]);
  assert.deepEqual(held.state().measures, {});
  assert.deepEqual(held.state().forget, []);
  const lines = reportLines(held.state().report, fr, "fr");
  assert.equal(lines[0], "Mesure de M42 retirée");
  assert.match(
    lines[1],
    /^Master de M31 mesuré · Qualité mesurée sur votre master/,
  );
  assert.match(lines[1], /FWHM 2,1″/);

  const refused = fake({ master: { result: "refused", code: "stretched" } });
  const again = session([{ type: "pickMeasure", pick: MASTER }]);
  const failed = await applyMeasures(
    refused.deps,
    SESSION,
    measuresPlan(again.state()),
    CONTEXT,
    again.hooks,
  );
  assert.deepEqual(failed, {
    ok: false,
    failure: {
      kind: "measure",
      name: "M31",
      reason: fr("master.refusal.stretched"),
    },
  });
  // Still there, to untick.
  assert.equal(again.state().measures["t-m31"].ticked, true);
  const none = fake({});
  const untouched = session([
    { type: "pickMeasure", pick: MASTER },
    { type: "tickMeasure", targetId: "t-m31", on: false },
  ]);
  await applyMeasures(
    none.deps,
    SESSION,
    measuresPlan(untouched.state()),
    CONTEXT,
    untouched.hooks,
  );
  assert.deepEqual(none.calls, []);
});

const INCIDENTS_PLAN: IncidentsPlan = {
  adds: [
    {
      key: "a1",
      eventType: "note",
      at: "2026-08-10T20:10:00Z",
      until: null,
      payload: {},
    },
    {
      key: "a2",
      eventType: "note",
      at: "2026-08-10T21:00:00Z",
      until: null,
      payload: {},
    },
  ],
  replacements: [
    {
      eventId: "e1",
      eventType: "note",
      at: "2026-08-10T22:00:00Z",
      until: null,
      payload: {},
    },
  ],
  replacedLines: [],
  keptLines: [],
  summary: {} as never,
};

test("the primary of « Récapitulatif » adds the incidents ticked before it sends; none ticked, nothing is asked", async () => {
  const run = fake({});
  const end = await applyIncidents(
    run.deps,
    SESSION,
    INCIDENTS_PLAN,
    "10 août",
    CONTEXT,
    { work: () => undefined },
  );
  assert.deepEqual(run.calls, ["incidents:2+1"]);
  assert.ok(end.ok);
  assert.deepEqual(end.ok && end.item, {
    kind: "incidents",
    added: 3,
    rejected: 0,
    refused: false,
  });
  assert.deepEqual(
    reportLines(end.ok && end.item ? [end.item] : [], fr, "fr"),
    ["3 incidents ajoutés à la session"],
  );
  const empty = fake({});
  const nothing = await applyIncidents(
    empty.deps,
    SESSION,
    { ...INCIDENTS_PLAN, adds: [], replacements: [] },
    "10 août",
    CONTEXT,
    { work: () => undefined },
  );
  assert.deepEqual(nothing, { ok: true, item: null });
  assert.deepEqual(empty.calls, []);
  const failing = fake({ fail: "incidents:2+1" });
  const stopped = await applyIncidents(
    failing.deps,
    SESSION,
    INCIDENTS_PLAN,
    "10 août",
    CONTEXT,
    { work: () => undefined },
  );
  assert.deepEqual(stopped, {
    ok: false,
    failure: { kind: "incidents", name: "10 août", reason: "boom" },
  });
});

/* ------------------------------ The report --------------------------------- */

test("« Nuit envoyée » reports what was done on the way, in the language of the window", () => {
  const report = [
    { kind: "target", how: "added", name: "NGC 7000" },
    { kind: "plan", how: "create", name: "IC 1805" },
    { kind: "forget", target: "M42" },
    { kind: "incidents", added: 2, rejected: 1, refused: true },
  ] as const;
  assert.deepEqual(reportLines(report, en, "en"), [
    "Target NGC 7000 added to the night",
    "Multi-night plan created for IC 1805, with the session in it",
    "Measure of M42 removed",
    "2 incidents added to the session (1 refused by the space)",
    "The replacements were not accepted (the session has changed).",
  ]);
  assert.deepEqual(reportLines([], fr, "fr"), []);
});

/* ------------------------------ The wiring --------------------------------- */

const source = (relative: string) =>
  readFile(new URL(`../${relative}`, import.meta.url), "utf8");

test("no card of the night's wizard writes by itself: the primaries apply what is chosen", async () => {
  const targets = await source("screens/night/TargetsStep.tsx");
  assert.doesNotMatch(targets, /nightTargetsChange|personalTargetCreate/);
  assert.doesNotMatch(targets, /undoAdd|wizard\.targets\.list\.undo/);
  assert.match(targets, /type: "pickCatalogue"/);
  assert.match(targets, /type: "pickNew"/);
  assert.match(targets, /ciblesLine\(plan, t, locale\)/);
  const plans = await source("screens/night/PlanAttach.tsx");
  assert.doesNotMatch(
    plans,
    /nightPlanAttach|nightPlanDetach|nightPlanCreate|useSnackbar/,
  );
  assert.match(plans, /type: "pickPlan"/);
  assert.match(plans, /type: "tickDetach"/);
  const master = await source("screens/night/MasterMeasure.tsx");
  const device = await source("screens/night/DeviceMaster.tsx");
  for (const text of [master, device]) {
    assert.doesNotMatch(text, /masterMeasure\(|masterMeasureForget/);
    assert.match(text, /type: "pickMeasure"|type: "tickForget"/);
  }
  const panel = await source("screens/night/IncidentsPanel.tsx");
  assert.doesNotMatch(panel, /\.apply\(|onApply|incidentsApply/);
  assert.match(panel, /withStep="send"/);
  const hook = await source("hooks/useIncidents.ts");
  assert.doesNotMatch(hook, /incidentsApply|const apply\b/);
  const card = await source("components/IncidentsCard.tsx");
  assert.doesNotMatch(card, /onApply|"incidents\.add"|as-incidents__actions/);
  const wizard = await source("screens/night/NightWizard.tsx");
  assert.match(wizard, /applyCibles\(/);
  assert.match(wizard, /applyMeasures\(/);
  assert.match(wizard, /void continueFromTargets\(\)/);
  assert.match(wizard, /void continueFromTreatments\(\)/);
  assert.doesNotMatch(wizard, /withFresh|onRemoved|onAdded/);
  const summary = await source("screens/night/SummaryStep.tsx");
  assert.match(summary, /applyIncidents\(/);
  assert.match(summary, /reportLines\(report, t, locale\)/);
  assert.match(summary, /incidentsLine\(/);
});

test("the words exist in the ten languages", async () => {
  const keys = [
    "wizard.recap.lead",
    "wizard.recap.sendLead",
    "wizard.recap.working.targets",
    "wizard.recap.working.plan",
    "wizard.recap.working.measure",
    "wizard.recap.working.incidents",
    "wizard.recap.targets.added.one",
    "wizard.recap.targets.created.other",
    "wizard.recap.plan.attach",
    "wizard.recap.plan.create",
    "wizard.recap.plan.detach.one",
    "wizard.recap.plan.detach.other",
    "wizard.recap.measure.master",
    "wizard.recap.measure.device",
    "wizard.recap.measure.forget",
    "wizard.recap.incidents.one",
    "wizard.recap.report.title",
    "wizard.recap.report.added",
    "wizard.recap.report.created",
    "wizard.recap.failed.target",
    "wizard.recap.failed.plan",
    "wizard.recap.failed.detach",
    "wizard.recap.failed.measure",
    "wizard.recap.failed.forget",
    "wizard.recap.failed.incidents",
    "wizard.targets.card.inList",
    "wizard.targets.pending.add",
    "wizard.targets.pending.create",
    "wizard.targets.pending.skipped",
    "planAttach.pending.attach",
    "planAttach.pending.detach",
    "planAttach.pending.skipped",
    "master.pending.measure",
    "master.pending.skipped",
    "master.pending.forget",
    "deviceMaster.pending",
  ];
  const dictionaries = await Promise.all(
    SUPPORTED_LOCALES.map(async (locale) => ({
      locale,
      dictionary: JSON.parse(
        await readFile(
          new URL(`../i18n/${locale}.json`, import.meta.url),
          "utf8",
        ),
      ) as Record<string, string>,
    })),
  );
  for (const { locale, dictionary } of dictionaries) {
    for (const key of keys) {
      assert.ok(dictionary[key], `${locale} ${key}`);
    }
    // The placeholders of the sentences that carry a value.
    for (const [key, placeholder] of [
      ["wizard.recap.lead", "{list}"],
      ["wizard.recap.sendLead", "{list}"],
      ["wizard.recap.plan.attach", "{name}"],
      ["wizard.recap.plan.create", "{target}"],
      ["wizard.recap.plan.detach.one", "{list}"],
      ["wizard.recap.measure.master", "{target}"],
      ["wizard.recap.report.added", "{name}"],
      ["wizard.recap.failed.target", "{reason}"],
      ["wizard.recap.failed.incidents", "{reason}"],
      ["master.pending.measure", "{file}"],
    ]) {
      assert.ok(dictionary[key].includes(placeholder), `${locale} ${key}`);
    }
    // The words of the buttons that wrote by themselves are gone.
    for (const key of [
      "wizard.targets.list.undo",
      "incidents.add.one",
      "planAttach.attached",
      "master.forgotten",
    ]) {
      assert.equal(dictionary[key], undefined, `${locale} still has ${key}`);
    }
  }
});
