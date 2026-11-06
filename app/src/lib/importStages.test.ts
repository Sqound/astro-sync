import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

import { createTranslator, SUPPORTED_LOCALES } from "../i18n/locale.ts";
import type {
  BulkCreateOutcome,
  ImageToSend,
  PlanImageOutcome,
  Proposal,
  Settled,
} from "./bridge";
import {
  applyRecap,
  createdIds,
  movedPath,
  PlanNotCreated,
  settledImage,
  type ApplyDeps,
  type ApplyHooks,
  type ApplyInput,
} from "./importApply.ts";
import {
  NO_IMPORT,
  importReducer,
  provisionalIds,
  recapPlan,
  recapProgress,
  type Created,
  type ImportAction,
  type ImportState,
  type Made,
} from "./importFlow.ts";

// Nothing is written to the space before
// step 4. Step 2's « Continuer » computes what would be created (provisional
// identifiers), the treatments of step 3 run on it, and the primary of step 4
// creates the sessions and the plan, adds the incidents, tidies the files and
// queues the images, in that order, each stage said; a failure stops there,
// what went through stays, the next try skips it.

const fr = createTranslator("fr");
const plain = (text: string) => text.replace(/[\u00a0\u202f]/g, " ");

const PROPOSAL = {
  reading: {
    filesRead: 10,
    lights: 8,
    calibrationFrames: 0,
    logFiles: 0,
    skippedNoDate: 0,
    nights: 2,
    targets: 2,
  },
  sessions: [
    {
      id: "d1",
      eveningDate: "2026-08-09",
      targets: [
        { targetId: "ic1805", designation: "IC 1805", frames: 98, filters: [] },
        { targetId: "m31", designation: "M31", frames: 40, filters: [] },
      ],
    },
    {
      id: "d2",
      eveningDate: "2026-08-12",
      targets: [
        {
          targetId: "ic1805",
          designation: "IC 1805",
          frames: 120,
          filters: [],
        },
      ],
    },
  ],
  lines: [],
  places: [],
} as unknown as Proposal;

const session = (detectedId: string, sessionId: string, date: string) => ({
  detectedId,
  sessionId,
  eveningDate: date,
  action: "created" as const,
  planAction: "created" as const,
  planId: null,
  designations: detectedId === "d1" ? ["IC 1805", "M31"] : ["IC 1805"],
  frames: 0,
  shiftHours: 0,
});

/** What « Continuer » computed: provisional sessions and plan. */
const LOCAL_OUTCOME: BulkCreateOutcome = {
  sessions: [
    session("d1", "local-aaaaaaaaaaaa", "2026-08-09"),
    session("d2", "local-bbbbbbbbbbbb", "2026-08-12"),
  ],
  multiNight: [
    {
      groupId: "g-1",
      planId: "local-pppppppppppp",
      designation: "IC 1805",
      sessionIds: ["local-aaaaaaaaaaaa", "local-bbbbbbbbbbbb"],
      goalSeconds: 3600,
    },
  ],
  placesCreated: [],
};
/** What the space created. */
const CREATED_OUTCOME: BulkCreateOutcome = {
  sessions: [
    session("d1", "sess-0809", "2026-08-09"),
    session("d2", "sess-0812", "2026-08-12"),
  ],
  multiNight: [{ ...LOCAL_OUTCOME.multiNight[0], planId: "prj-777" }],
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
    toId: "sess-0809",
    from: "/w/2026-08-09 · Cœur · local-aaaaaa",
    to: "/w/2026-08-09 · Cœur · sess-0",
  },
  {
    fromId: "local-bbbbbbbbbbbb",
    toId: "sess-0812",
    from: "/w/2026-08-12 · Cœur · local-bbbbbb",
    to: "/w/2026-08-12 · Cœur · sess-0 (2)",
  },
];
const PLANS: Settled[] = [
  {
    fromId: "local-pppppppppppp",
    toId: "prj-777",
    from: "/w/IC 1805 · local-pppppp",
    to: "/w/IC 1805 · prj-77",
  },
];

const NIGHT_IMAGE: ImageToSend = {
  kind: "night",
  sessionId: "local-aaaaaaaaaaaa",
  targetId: "m31",
  render: {
    pipelineId: "natural",
    path: "/w/2026-08-09 · Cœur · local-aaaaaa/M31/stacked/M31_natural.tif",
    sha256: "a".repeat(64),
    size: 10,
  },
};
const PLAN_IMAGE: ImageToSend = {
  kind: "plan",
  outcome: {
    projectId: "local-pppppppppppp",
    targetId: "ic1805",
    render: {
      pipelineId: "natural",
      path: "/w/IC 1805 · local-pppppp/stacked/IC_1805_plan_natural.tif",
      sha256: "b".repeat(64),
      size: 20,
    },
    otherFiles: ["/w/IC 1805 · local-pppppp/stacked/IC_1805_plan.json"],
    combinedNightIds: ["local-aaaaaaaaaaaa", "local-bbbbbbbbbbbb"],
    totalsByFilter: [],
    planRender: "natural",
  } as PlanImageOutcome,
};

function deps(trace: string[], over: Partial<ApplyDeps> = {}): ApplyDeps {
  return {
    create: async () => {
      trace.push("create");
      return { outcome: CREATED_OUTCOME, nights: NIGHTS };
    },
    settlePlans: async (nights) => {
      trace.push(`plans:${nights.length}`);
      return PLANS;
    },
    incidentsApply: async (request) => {
      trace.push(`incidents:${request.sessionId}`);
      return {
        added: request.adds.map(() => ({}) as never),
        duplicate: [],
        replaced: [],
        rejected: [],
        replacementsRefused: null,
      } as never;
    },
    workingFolder: async () => "/w",
    organisePlan: async (_destination, _mode, created) => {
      trace.push(`tidyPlan:${created.outcome.sessions[0].sessionId}`);
      return {
        mode: "symlink",
        destination: "/w",
        actions: [
          {
            source: "/raw/a.fit",
            destination: "/w/x/a.fit",
            kind: "link",
            bytes: 1,
          },
        ],
        conflicts: [],
        files: 1,
        bytes: 1,
        windowsNoLink: false,
        summary: "",
      };
    },
    organise: async () => {
      trace.push("organise");
      return {
        done: 1,
        alreadyThere: 0,
        failed: [],
        stopped: false,
      } as never;
    },
    enqueue: async (images) => {
      trace.push(
        `enqueue:${images
          .map((image) =>
            image.kind === "night" ? image.sessionId : image.outcome.projectId,
          )
          .join(",")}`,
      );
    },
    ...over,
  };
}

/** The hooks, as the provider dispatches them, kept in `held`. */
function hooks() {
  const log: string[] = [];
  const held: {
    created: Created | null;
    report: ApplyInput["report"];
  } = { created: null, report: { plans: null, incidents: {}, tidy: null } };
  const hooks: ApplyHooks = {
    phase: (stage) => log.push(stage),
    created: (created) => {
      held.created = created;
    },
    plansDone: (plans) => {
      held.report = { ...held.report, plans };
    },
    incidentsDone: (sessionId, result) => {
      held.report = {
        ...held.report,
        incidents: { ...held.report.incidents, [sessionId]: result },
      };
    },
    tidyDone: (result) => {
      held.report = { ...held.report, tidy: result };
    },
  };
  return { log, held, hooks };
}

const PLAN: ApplyInput["plan"] = {
  incidents: [
    {
      sessionId: "local-aaaaaaaaaaaa",
      plan: { adds: [{} as never], replacements: [], summary: {} as never },
    },
  ],
  tidy: { on: true, mode: "symlink", destination: null },
};

const start = (over: Partial<ApplyInput> = {}): ApplyInput => ({
  local: LOCAL,
  created: null,
  plan: PLAN,
  images: [NIGHT_IMAGE, PLAN_IMAGE],
  report: { plans: null, incidents: {}, tidy: null },
  sent: false,
  ...over,
});

test("the primary creates, then the plan, the incidents, the tidying, the images -- in that order, each stage said", async () => {
  const trace: string[] = [];
  const run = hooks();
  const end = await applyRecap(deps(trace), start(), run.hooks);
  assert.equal(
    end.ok,
    true,
    String(end.ok === false && end.stage + " " + end.error),
  );
  assert.deepEqual(trace, [
    "create",
    "plans:2",
    "incidents:sess-0809",
    "tidyPlan:sess-0809",
    "organise",
    "enqueue:sess-0809,prj-777",
  ]);
  assert.deepEqual(run.log, ["sessions", "plan", "incidents", "tidy", "send"]);
  // The incidents are reported by the session created.
  assert.deepEqual(Object.keys(run.held.report.incidents), ["sess-0809"]);
  assert.equal(run.held.created?.outcome, CREATED_OUTCOME);
  assert.deepEqual(run.held.created?.nights, NIGHTS);
});

test("the images leave as the space created them: its ids, their files in the folders renamed", async () => {
  const end = await applyRecap(deps([]), start(), hooks().hooks);
  assert.ok(end.ok);
  const [night, plan] = end.images;
  assert.equal(night.kind, "night");
  assert.equal(night.kind === "night" && night.sessionId, "sess-0809");
  assert.equal(
    night.kind === "night" && night.render.path,
    "/w/2026-08-09 · Cœur · sess-0/M31/stacked/M31_natural.tif",
  );
  assert.equal(plan.kind, "plan");
  if (plan.kind !== "plan") return;
  assert.equal(plan.outcome.projectId, "prj-777");
  assert.deepEqual(plan.outcome.combinedNightIds, ["sess-0809", "sess-0812"]);
  assert.equal(
    plan.outcome.render.path,
    "/w/IC 1805 · prj-77/stacked/IC_1805_plan_natural.tif",
  );
  assert.deepEqual(plan.outcome.otherFiles, [
    "/w/IC 1805 · prj-77/stacked/IC_1805_plan.json",
  ]);
  // The digest is the file's: it does not change with the folder.
  assert.equal(plan.outcome.render.sha256, "b".repeat(64));
});

test("a failure stops the primary at its stage; the next try skips what went through and the sessions are never created twice", async () => {
  // The space cannot be reached: nothing went through.
  const trace: string[] = [];
  const first = hooks();
  const cut = await applyRecap(
    deps(trace, {
      create: async () => {
        trace.push("create");
        throw new Error("offline");
      },
    }),
    start(),
    first.hooks,
  );
  assert.equal(cut.ok === false && cut.stage, "sessions");
  assert.deepEqual(trace, ["create"]);
  assert.equal(first.held.created, null);
  assert.deepEqual(first.log, ["sessions"]);

  // Created, then the plan folder fails: the sessions stay created.
  const second: string[] = [];
  const run = hooks();
  const stopped = await applyRecap(
    deps(second, {
      settlePlans: async () => {
        second.push("plans");
        throw new Error("disk full");
      },
    }),
    start(),
    run.hooks,
  );
  assert.equal(stopped.ok === false && stopped.stage, "plan");
  assert.deepEqual(second, ["create", "plans"]);
  assert.ok(run.held.created);

  // The next try: no second creation, the plan, then the rest.
  const third: string[] = [];
  const resumed = await applyRecap(
    deps(third),
    start({ created: run.held.created, report: run.held.report }),
    hooks().hooks,
  );
  assert.equal(resumed.ok, true);
  assert.deepEqual(third, [
    "plans:2",
    "incidents:sess-0809",
    "tidyPlan:sess-0809",
    "organise",
    "enqueue:sess-0809,prj-777",
  ]);

  // The images could not be queued: the next try queues them only.
  const fourth: string[] = [];
  const last = hooks();
  const queueing = await applyRecap(
    deps(fourth, {
      enqueue: async () => {
        fourth.push("enqueue");
        throw new Error("queue locked");
      },
    }),
    start(),
    last.hooks,
  );
  assert.equal(queueing.ok === false && queueing.stage, "send");
  const fifth: string[] = [];
  await applyRecap(
    deps(fifth),
    start({ created: last.held.created, report: last.held.report }),
    hooks().hooks,
  );
  assert.deepEqual(fifth, ["enqueue:sess-0809,prj-777"]);
});

test("the image of a plan the space did not create cannot leave: the primary stops at the plan and says which", async () => {
  const trace: string[] = [];
  const end = await applyRecap(
    deps(trace, {
      create: async () => {
        trace.push("create");
        return {
          outcome: { ...CREATED_OUTCOME, multiNight: [] },
          nights: NIGHTS,
        };
      },
    }),
    start(),
    hooks().hooks,
  );
  assert.equal(end.ok, false);
  assert.ok(end.ok === false && end.error instanceof PlanNotCreated);
  assert.equal(
    end.ok === false && (end.error as PlanNotCreated).designation,
    "IC 1805",
  );
  assert.deepEqual(trace, ["create"], "nothing is queued");
  assert.equal(
    fr("import.recap.planNotCreated", { plan: "IC 1805" }),
    "Le plan multi-nuits IC 1805 n’a pas été créé : son image ne peut pas partir. Décochez-la pour continuer.",
  );
  // Without its image, the plan that was not created stops nothing.
  const without = await applyRecap(
    deps([], {
      create: async () => ({
        outcome: { ...CREATED_OUTCOME, multiNight: [] },
        nights: NIGHTS,
      }),
    }),
    start({ images: [NIGHT_IMAGE] }),
    hooks().hooks,
  );
  assert.equal(without.ok, true);
});

test("the identifiers and the paths of what was made follow what the space created", () => {
  assert.deepEqual(
    createdIds(LOCAL, { request: REQUEST, outcome: CREATED_OUTCOME }),
    {
      "local-aaaaaaaaaaaa": "sess-0809",
      "local-bbbbbbbbbbbb": "sess-0812",
      "local-pppppppppppp": "prj-777",
    },
  );
  // A session that existed keeps its id: nothing to map.
  const existing = {
    ...LOCAL,
    outcome: {
      ...LOCAL_OUTCOME,
      sessions: [{ ...LOCAL_OUTCOME.sessions[0], sessionId: "sess-0809" }],
      multiNight: [],
    },
  };
  assert.deepEqual(
    createdIds(existing, { request: REQUEST, outcome: CREATED_OUTCOME }),
    {},
  );
  assert.equal(
    movedPath("/w/2026-08-09 · Cœur · local-aaaaaa/M31/x.tif", NIGHTS),
    "/w/2026-08-09 · Cœur · sess-0/M31/x.tif",
  );
  // A folder whose name starts like another is not moved with it.
  assert.equal(
    movedPath("/w/2026-08-09 · Cœur · local-aaaaaa (2)/x.tif", NIGHTS),
    "/w/2026-08-09 · Cœur · local-aaaaaa (2)/x.tif",
  );
  assert.equal(
    movedPath("C:\\w\\n · local-a\\x.tif", [
      {
        fromId: "a",
        toId: "b",
        from: "C:\\w\\n · local-a",
        to: "C:\\w\\n · s",
      },
    ]),
    "C:\\w\\n · s\\x.tif",
  );
  const same = settledImage(NIGHT_IMAGE, {}, []);
  assert.deepEqual(same, NIGHT_IMAGE);
});

test("the finishes of a plan's image follow its folder when the space renames it", () => {
  const render = (path: string) => ({
    pipelineId: "raw",
    path,
    sha256: "ab",
    size: 10,
  });
  const image: ImageToSend = {
    kind: "plan",
    outcome: {
      projectId: "local-pppppppppppp",
      targetId: "ic1805",
      render: render("/w/IC 1805 · local-aaaaaa/stacked/P_plan_raw.tif"),
      finishes: [
        {
          render: render(
            "/w/IC 1805 · local-aaaaaa/stacked/P_plan_raw_strong_colour-40.tif",
          ),
        },
      ],
      otherFiles: [],
      combinedNightIds: ["local-aaaaaaaaaaaa"],
      totalsByFilter: [],
      planRender: "natural",
    },
  };
  const moves = [
    {
      fromId: "local-aaaaaa",
      toId: "sess-0",
      from: "/w/IC 1805 · local-aaaaaa",
      to: "/w/IC 1805 · sess-0",
    },
  ];
  const settled = settledImage(image, {}, moves);
  assert.equal(settled.kind, "plan");
  if (settled.kind !== "plan") return;
  assert.equal(
    settled.outcome.render.path,
    "/w/IC 1805 · sess-0/stacked/P_plan_raw.tif",
  );
  assert.equal(
    settled.outcome.finishes?.[0].render.path,
    "/w/IC 1805 · sess-0/stacked/P_plan_raw_strong_colour-40.tif",
  );
});

test("the progression images of a plan follow its folder when the space renames it", () => {
  const step = (nights: number) => ({
    nightId: `sess-${nights}`,
    eveningDate: `2026-10-0${nights}`,
    nights,
    frames: 100 * nights,
    seconds: 3000 * nights,
    jpeg: `/w/Sh2-103 · local-fea0a6/progression/Sh2-103_plan-2_night-0${nights}_${100 * nights}-frames.jpg`,
    sha256: `ab${nights}`,
    size: 10,
  });
  const image: ImageToSend = {
    kind: "plan",
    outcome: {
      projectId: "local-pppppppppppp",
      targetId: "sh2-103",
      render: {
        pipelineId: "raw",
        path: "/w/Sh2-103 · local-fea0a6/stacked/P_plan_raw.tif",
        sha256: "ab",
        size: 10,
      },
      otherFiles: [],
      combinedNightIds: [],
      totalsByFilter: [],
      planRender: "natural",
    },
    progression: [step(1), step(2), step(3)],
  };
  const moves = [
    {
      fromId: "local-fea0a6",
      toId: "80340a",
      from: "/w/Sh2-103 · local-fea0a6",
      to: "/w/Sh2-103 · 80340a",
    },
  ];
  const settled = settledImage(image, {}, moves);
  assert.equal(settled.kind, "plan");
  if (settled.kind !== "plan") return;
  assert.deepEqual(
    settled.progression?.map((item) => item.jpeg),
    [
      "/w/Sh2-103 · 80340a/progression/Sh2-103_plan-2_night-01_100-frames.jpg",
      "/w/Sh2-103 · 80340a/progression/Sh2-103_plan-2_night-02_200-frames.jpg",
      "/w/Sh2-103 · 80340a/progression/Sh2-103_plan-2_night-03_300-frames.jpg",
    ],
  );
  // Its other fields are the same.
  assert.equal(settled.progression?.[1].sha256, "ab2");
});

/* ------------------- Nothing written before step 4 ------------------------ */

const play = (actions: ImportAction[], from: ImportState = NO_IMPORT) =>
  actions.reduce(importReducer, from);

const made = (
  targetId: string,
  designation: string,
  send: ImageToSend,
): Made => ({
  targetId,
  designation,
  pipelineId: "natural",
  nights: 1,
  path: send.kind === "night" ? send.render.path : send.outcome.render.path,
  size: 10,
  send,
});

test("from the folders to step 4, nothing is created: « Continuer » computes, the treatments run on provisional sessions", () => {
  const state = play([
    { type: "open" },
    { type: "scanStart" },
    {
      type: "scanDone",
      proposal: PROPOSAL,
      user: { account: {} } as never,
      review: {} as never,
    },
    { type: "createStart" },
    { type: "planned", local: LOCAL },
    { type: "treatStart", target: "IC 1805" },
    {
      type: "treatDone",
      made: [
        made("m31", "M31", NIGHT_IMAGE),
        made("ic1805", "IC 1805", PLAN_IMAGE),
      ],
      errors: [],
    },
  ]);
  assert.equal(state.step, 4);
  assert.equal(state.created, null, "nothing in the space before step 4");
  assert.equal(state.local, LOCAL);
  assert.deepEqual(state.chosen, ["ic1805", "m31"]);
  assert.equal(state.ticked.length, 2);
  // « Abandonner l'import » removes the provisional folders, sessions and plan.
  assert.deepEqual(provisionalIds(state), [
    "local-aaaaaaaaaaaa",
    "local-bbbbbbbbbbbb",
    "local-pppppppppppp",
  ]);
  // The line above the primary says what it will create.
  const plan = recapPlan(state, "");
  assert.equal(plan.sessions, 2);
  assert.equal(plan.plans, 1);

  // Back to step 2, M31 unticked, « Continuer » again: the images of what
  // left go with it, the others stay with their ticks.
  const withoutM31: Created = {
    request: REQUEST,
    outcome: {
      ...LOCAL_OUTCOME,
      sessions: LOCAL_OUTCOME.sessions.map((s) => ({
        ...s,
        designations: ["IC 1805"],
      })),
    },
  };
  const again = play(
    [
      { type: "goTo", step: 2 },
      { type: "createStart" },
      { type: "planned", local: withoutM31 },
    ],
    state,
  );
  assert.equal(again.step, 3);
  assert.equal(again.reached, 4, "the summary stays reachable");
  assert.deepEqual(
    again.made.map((item) => item.targetId),
    ["ic1805"],
  );
  assert.equal(again.ticked.length, 1);
  assert.deepEqual(again.chosen, ["ic1805"]);
});

test("the progress line of the primary says each stage gone through and the one running", () => {
  const four: ImportState = {
    ...NO_IMPORT,
    alive: true,
    step: 4,
    reached: 4,
    local: LOCAL,
    ticked: ["x"],
    sending: true,
  };
  const plan = {
    ...recapPlan(four, ""),
    incidents: PLAN.incidents as never,
    tidy: PLAN.tidy,
  };
  const t = (key: string, params?: Record<string, string | number>) =>
    plain(fr(key, params));
  assert.equal(
    recapProgress({ ...four, working: "sessions" }, plan, t),
    "Création des sessions…",
  );
  const created = {
    ...four,
    created: { request: REQUEST, outcome: CREATED_OUTCOME },
    report: { plans: PLANS, incidents: {}, tidy: null },
    working: "incidents" as const,
  };
  assert.equal(
    recapProgress(created, plan, t),
    "Sessions créées · Plan multi-nuits en place · Ajout des incidents…",
  );
  // Stopped at the tidying: what went through stays said.
  assert.equal(
    recapProgress(
      { ...created, working: null, sending: false, error: "x" },
      plan,
      t,
    ),
    "Sessions créées · Plan multi-nuits en place",
  );
});

test("the provider writes nothing before step 4's primary: « Continuer » and the treatments only compute and work on this computer", () => {
  const provider = readFileSync(
    new URL("../components/ImportProvider.tsx", import.meta.url),
    "utf8",
  );
  const body = (from: string, to: string) =>
    provider.slice(provider.indexOf(from), provider.indexOf(to));
  const writes = [
    "sessionsCreateBulk(",
    "detectImportCreate(",
    "detectImportPlans(",
    "incidentsApply(",
    "sessionsOrganise(",
    "detectImagesSend(",
    "planImageRun(",
    "nightMultiNight(",
  ];
  const create = body("const create = useCallback", "const runTreatments");
  assert.match(create, /sessionsProvisional\(request\)/);
  const treat = body("const runTreatments", "const stopTreatments");
  assert.match(treat, /planImageRunLocal\(/);
  assert.match(
    treat,
    /detectTreatmentsPrepare\(\s*local\.request\.proposal,\s*local\.outcome/,
  );
  for (const write of writes) {
    assert.ok(!create.includes(write), `« Continuer » calls ${write}`);
    assert.ok(!treat.includes(write), `the treatments call ${write}`);
  }
  // Step 4's primary is the one place that writes.
  const apply = body("const apply = useCallback", "const send = useCallback");
  for (const write of ["detectImportCreate(", "detectImagesSend("])
    assert.ok(apply.includes(write), write);
  // The incidents of a session not created yet are not asked of the space.
  const hook = readFileSync(
    new URL("../hooks/useIncidents.ts", import.meta.url),
    "utf8",
  );
  assert.match(hook, /isProvisionalId\(night\.sessionId\)/);
  // The Tauri side: the plan image of the import reads no plan of the space.
  const rust = readFileSync(
    new URL("../../src-tauri/src/commands_multi_night.rs", import.meta.url),
    "utf8",
  );
  const local = rust.slice(
    rust.indexOf("pub async fn plan_image_run_local"),
    rust.indexOf("fn make_plan_images"),
  );
  assert.match(local, /local_plan_lines\(/);
  assert.doesNotMatch(local, /night_side|require_portal_and_token/);
});

test("step 2, the card of « Sessions » and the primary of step 4 say it, in the ten languages", () => {
  assert.equal(plain(fr("import.create.send")), "Créer et envoyer");
  assert.equal(plain(fr("import.create.only")), "Créer");
  assert.match(
    plain(fr("detect.review.nothingYet")),
    /Rien n’est écrit dans votre espace avant le Récapitulatif\./,
  );
  assert.match(
    plain(fr("import.card.nothingWritten")),
    /Rien n’est encore écrit/,
  );
  const keys = [
    "import.create.send",
    "import.create.only",
    "import.create.withoutSending",
    "import.recap.sessions.one",
    "import.recap.plans.other",
    "import.recap.working.sessions",
    "import.recap.working.plan",
    "import.recap.done.sessions",
    "import.recap.done.plan",
    "import.recap.done.incidents",
    "import.recap.done.tidy",
    "import.recap.done.send",
    "import.recap.createFailed",
    "import.recap.foldersFailed",
    "import.recap.planNotCreated",
    "import.recap.planFailed",
    "import.summary.toCreate.one",
    "import.summary.toImport",
    "import.summary.toUpdate",
    "import.treat.title",
    "import.card.nothingWritten",
    "import.abandon.title",
    "import.abandon.body",
    "import.abandon.nothingCreated",
    "import.abandon.created",
    "incidents.willAdd.create.other",
  ];
  const english = createTranslator("en");
  for (const locale of SUPPORTED_LOCALES) {
    const t = createTranslator(locale);
    for (const key of keys) {
      assert.notEqual(t(key), key, `${locale} lacks ${key}`);
      if (locale !== "en" && locale !== "fr")
        assert.notEqual(t(key), english(key), `${locale} ${key} is English`);
    }
  }
  // The card of « Sessions » says it while nothing is written.
  const card = readFileSync(
    new URL("../screens/detect/ImportCard.tsx", import.meta.url),
    "utf8",
  );
  assert.match(card, /!presence\.written && \(/);
  assert.match(card, /t\("import\.card\.nothingWritten"\)/);
  // « Abandonner l'import » asks first, a destructive primary beside a link.
  assert.match(
    card,
    /presence\.confirmAbandon \? setAsking\(true\) : imp\.abandon\(\)/,
  );
  assert.match(card, /buttonClassName\("primary", \{ destructive: true \}\)/);
  assert.match(card, /role="dialog"/);
});
