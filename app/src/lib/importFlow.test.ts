import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

import { SUPPORTED_LOCALES, createTranslator } from "../i18n/locale.ts";
import type { BulkCreateOutcome, Proposal, SpaceStorage } from "./bridge";
import { summaryImages, type TreatmentResult } from "./detectTreatments.ts";
import {
  NO_IMPORT,
  canGoTo,
  footRule,
  importCardLine,
  importPresence,
  importReducer,
  importThread,
  preChosenTargets,
  preTickedImages,
  previousOf,
  reviewReadOnly,
  type ImportAction,
  type ImportState,
  type Made,
} from "./importFlow.ts";
import { MENU_GROUPS, groupEntries, menuEntryOf } from "./menu.ts";

// « Détecter les sessions » is one wizard of five steps whose state lives
// above the screens (`ImportProvider`). The reducer is the import: where
// each step may go, what each foot offers, what is ticked from the start,
// what the menu and « Sessions » say of it.

const fr = createTranslator("fr");
const plain = (text: string) => text.replace(/[  ]/g, " ");
const play = (actions: ImportAction[], from: ImportState = NO_IMPORT) =>
  actions.reduce(importReducer, from);

const PROPOSAL = {
  reading: {
    filesRead: 10,
    lights: 8,
    calibrationFrames: 2,
    logFiles: 0,
    skippedNoDate: 0,
    nights: 2,
    targets: 2,
  },
  sessions: [
    {
      id: "d1",
      eveningDate: "2026-08-03",
      targets: [
        {
          targetId: "ngc7000",
          designation: "NGC 7000",
          frames: 72,
          filters: [],
        },
        { targetId: "m31", designation: "M31", frames: 0, filters: [] },
      ],
    },
  ],
  lines: [],
  places: [],
} as unknown as Proposal;
const USER = { account: {} } as never;
const REVIEW = {
  ticked: {},
  answers: {},
  places: {},
  shifts: {},
  filters: {},
};
const OUTCOME = {
  sessions: [
    {
      detectedId: "d1",
      sessionId: "s1",
      eveningDate: "2026-08-03",
      action: "created",
      planAction: "none",
      planId: null,
      designations: ["NGC 7000", "M31"],
      frames: 72,
      shiftHours: 0,
    },
  ],
  multiNight: [],
  placesCreated: [],
} as unknown as BulkCreateOutcome;
const CREATED = {
  request: {
    proposal: PROPOSAL,
    ticked: [],
    places: [],
    answers: [],
    shifts: [],
  },
  outcome: OUTCOME,
};

/** What would be created, computed when step 2 is left: the same session
 * under its provisional identifier. */
const LOCAL = {
  ...CREATED,
  outcome: {
    ...OUTCOME,
    sessions: OUTCOME.sessions.map((s) => ({
      ...s,
      sessionId: "local-0a1b2c3d4e5f",
    })),
  },
};

const read: ImportAction[] = [
  { type: "open" },
  { type: "scanStart" },
  { type: "scanProgress", read: 482, total: 1423, current: "/a/b.fits" },
  { type: "scanDone", proposal: PROPOSAL, user: USER, review: REVIEW },
];
const planned: ImportAction[] = [
  ...read,
  { type: "createStart" },
  { type: "planned", local: LOCAL },
];
/** Step 4's primary went through its first stage: the space created them. */
const createdOnStep4: ImportAction[] = [
  ...planned,
  { type: "skip" },
  { type: "sendStart" },
  { type: "working", working: "sessions" },
  { type: "created", created: CREATED },
  { type: "sendFailed", error: "La connexion a été coupée." },
];

const made = (pipelineId: string, size = 1000): Made => ({
  targetId: "ngc7000",
  designation: "NGC 7000",
  pipelineId,
  nights: 1,
  path: `/w/NGC 7000/stacked/${pipelineId}.tif`,
  size,
  send: { kind: "plan", outcome: {} as never },
});

const storage = (over: Partial<SpaceStorage> = {}): SpaceStorage =>
  ({
    usedBytes: 0,
    quotaBytes: 1e12,
    remainingBytes: 1e12,
    maxFileBytes: 1_073_741_824,
    captureCount: 0,
    captureCountLimit: 9007199254740991,
    variantLimit: 9007199254740991,
    ...over,
  }) as SpaceStorage;

/* ------------------------------ The steps ------------------------------- */

test("the steps follow the import: the folders, the reading on the same step, the review once read, the treatments once left with « Continuer » -- nothing created", () => {
  const opened = play([{ type: "open" }]);
  assert.equal(opened.alive, true);
  assert.equal(opened.step, 1);
  const reading = play([{ type: "open" }, { type: "scanStart" }]);
  assert.equal(
    reading.step,
    1,
    "the reading is a state of step 1, not a window",
  );
  assert.ok(reading.scan);
  const review = play(read);
  assert.equal(review.step, 2);
  assert.equal(review.scan, null);
  assert.equal(review.reached, 2);
  const creating = play([...read, { type: "createStart" }]);
  assert.equal(creating.creating, true);
  assert.equal(creating.step, 2, "nothing moves before it is computed");
  const treatments = play(planned);
  assert.equal(treatments.step, 3);
  // Nothing in the space before step 4.
  assert.equal(treatments.created, null);
  assert.equal(treatments.local, LOCAL);
  assert.deepEqual(treatments.chosen, ["ngc7000"]);
});

test("« Arrêter la détection » goes back to the folders, writes nothing and says nothing; a failure says why", () => {
  const stopped = play([
    { type: "open" },
    { type: "scanStart" },
    { type: "scanEnd", error: null },
  ]);
  assert.equal(stopped.step, 1);
  assert.equal(stopped.scan, null);
  assert.equal(stopped.error, null);
  assert.equal(stopped.proposal, null);
  const failed = play([
    { type: "open" },
    { type: "scanStart" },
    { type: "scanEnd", error: "Dossier introuvable" },
  ]);
  assert.equal(failed.error, "Dossier introuvable");
});

test("going back: the folders and the review stay open until step 4 creates, read-only after it, step 3 from step 4 with nothing undone", () => {
  const review = play(read);
  assert.equal(previousOf(review), 1);
  const folders = play([{ type: "goTo", step: 1 }], review);
  assert.equal(folders.step, 1);
  assert.equal(folders.proposal, PROPOSAL, "the reading is kept");
  assert.equal(canGoTo(folders, 2), true, "the review opens again");
  // Left with « Continuer »: nothing is written, the review is still open.
  const three = play(planned);
  assert.equal(canGoTo(three, 1), true);
  assert.equal(previousOf(three), 2);
  const back = play([{ type: "goTo", step: 2 }], three);
  assert.equal(back.step, 2);
  assert.equal(reviewReadOnly(back), false);
  const changed = play(
    [{ type: "review", update: (r) => ({ ...r, shifts: { d1: 2 } }) }],
    back,
  );
  assert.deepEqual(changed.review, { ...REVIEW, shifts: { d1: 2 } });
  // Created by step 4: « Dossier » would be a new import, the review is
  // read-only, « Traitements » closed (its folders carry the sessions now).
  const made4 = play(createdOnStep4);
  assert.equal(made4.created, CREATED);
  assert.equal(canGoTo(made4, 1), false);
  assert.equal(canGoTo(made4, 3), false);
  assert.equal(previousOf(made4), null);
  const readOnly = play([{ type: "goTo", step: 2 }], made4);
  assert.equal(reviewReadOnly(readOnly), true);
  const unchanged = play(
    [{ type: "review", update: (r) => ({ ...r, shifts: { d1: 2 } }) }],
    readOnly,
  );
  assert.deepEqual(unchanged.review, REVIEW, "nothing created is changed");
  // Step 4 → 3: the images made stay, the summary stays reachable.
  const four = play(
    [
      { type: "treatStart", target: "NGC 7000" },
      { type: "treatDone", made: [made("raw")], errors: [] },
      { type: "goTo", step: 3 },
    ],
    three,
  );
  assert.equal(four.step, 3);
  assert.equal(four.made.length, 1);
  assert.equal(canGoTo(four, 4), true);
});

test("a job running holds the thread and « Précédent »: the reading, the creation, the treatments, the sending", () => {
  const busy: ImportState[] = [
    play([{ type: "open" }, { type: "scanStart" }]),
    play([...read, { type: "createStart" }]),
    play([...planned, { type: "treatStart", target: "NGC 7000" }]),
    play([{ type: "skip" }, { type: "sendStart" }], play(planned)),
  ];
  for (const state of busy) {
    for (const step of [1, 2, 3, 4, 5] as const)
      assert.equal(canGoTo(state, step), false, `${state.step} → ${step}`);
    assert.equal(previousOf(state), null);
    assert.ok(importThread(state).every((item) => !item.reopens));
  }
});

test("a run that ends opens « Récapitulatif »; « Arrêter les traitements » keeps the import on « Traitements » with what was made", () => {
  const running = play([
    ...planned,
    { type: "treatStart", target: "NGC 7000" },
  ]);
  const ended = play(
    [{ type: "treatDone", made: [made("raw")], errors: [] }],
    running,
  );
  assert.equal(ended.step, 4);
  assert.equal(ended.running, null);
  assert.equal(ended.treated, true);
  const stopped = play(
    [
      { type: "treatStop" },
      { type: "treatDone", made: [made("raw")], errors: [] },
    ],
    running,
  );
  assert.equal(stopped.step, 3);
  assert.equal(stopped.made.length, 1);
  assert.equal(stopped.stopping, false);
  assert.equal(footRule(stopped).secondary, "keepImages");
});

test("the import outlives its page: a job's result lands in the state whoever shows it, and the page reopens at the same step", () => {
  // The page is a view of this state: the menu unmounts it, the provider
  // keeps the state and the jobs (`ImportProvider` is above `AppShell`).
  const running = play([
    ...planned,
    { type: "treatStart", target: "NGC 7000" },
  ]);
  const elsewhere = importPresence(running);
  assert.equal(elsewhere.shown, true);
  assert.equal(elsewhere.job, "treat");
  const ended = play(
    [{ type: "treatDone", made: [made("raw")], errors: [] }],
    running,
  );
  // Reopened (`open` on an import alive changes nothing): step 4.
  const reopened = play([{ type: "open" }], ended);
  assert.equal(reopened, ended);
  assert.equal(reopened.step, 4);
  assert.equal(importPresence(reopened).waiting, true);
  // « Terminé » left: forgotten, the folders kept for next time.
  const closed = play(
    [{ type: "sent", count: 1, variants: 0 }, { type: "close" }],
    reopened,
  );
  assert.equal(closed.alive, false);
  assert.equal(closed.proposal, null);
  assert.equal(importPresence(closed).shown, false);
});

/* --------------------------------- The foot -------------------------------- */

test("the foot of each step: « Précédent » a link at the far left (none on step 1, while a job runs, on step 5), one tertiary secondary, one primary", () => {
  const rows: [string, ImportState, ReturnType<typeof footRule>][] = [
    [
      "1, folders",
      play([{ type: "open" }]),
      {
        previous: null,
        secondary: null,
        primary: { kind: "startScan", enabled: false },
      },
    ],
    [
      "1, reading",
      play([{ type: "open" }, { type: "scanStart" }]),
      { previous: null, secondary: "stopScan", primary: null },
    ],
    [
      "2, review",
      play(read),
      {
        previous: 1,
        secondary: null,
        primary: { kind: "create", enabled: false },
      },
    ],
    [
      "2, creating",
      play([...read, { type: "createStart" }]),
      {
        previous: null,
        secondary: null,
        primary: { kind: "create", enabled: false },
      },
    ],
    [
      "2, back from step 3: still open, nothing written",
      play([{ type: "goTo", step: 2 }], play(planned)),
      {
        previous: 1,
        secondary: null,
        primary: { kind: "create", enabled: false },
      },
    ],
    [
      "2, read-only once step 4 created",
      play([{ type: "goTo", step: 2 }], play(createdOnStep4)),
      {
        previous: null,
        secondary: null,
        primary: { kind: "continue", enabled: true },
      },
    ],
    [
      "3, targets ticked",
      play(planned),
      {
        previous: 2,
        secondary: "skipTreatments",
        primary: { kind: "treat", enabled: true },
      },
    ],
    [
      "3, none ticked",
      play([{ type: "toggleTarget", targetId: "ngc7000" }], play(planned)),
      {
        previous: 2,
        secondary: "skipTreatments",
        primary: { kind: "treat", enabled: false },
      },
    ],
    [
      "3, running",
      play([{ type: "treatStart", target: "NGC 7000" }], play(planned)),
      { previous: null, secondary: "stopTreatments", primary: null },
    ],
    [
      "4, images ticked",
      play(
        [
          { type: "treatStart", target: "NGC 7000" },
          { type: "treatDone", made: [made("raw")], errors: [] },
        ],
        play(planned),
      ),
      {
        previous: 3,
        secondary: "createWithoutSending",
        primary: { kind: "createSend", enabled: true },
      },
    ],
    [
      "4, nothing to send",
      play([{ type: "skip" }], play(planned)),
      {
        previous: 3,
        secondary: null,
        primary: { kind: "createOnly", enabled: true },
      },
    ],
    [
      "4, created, a later stage failed: the next try",
      play(createdOnStep4),
      {
        previous: null,
        secondary: null,
        primary: { kind: "finish", enabled: true },
      },
    ],
    [
      "5",
      play(
        [{ type: "skip" }, { type: "sent", count: 0, variants: 0 }],
        play(planned),
      ),
      {
        previous: null,
        secondary: null,
        primary: { kind: "backToSessions", enabled: true },
      },
    ],
  ];
  for (const [name, state, rule] of rows)
    assert.deepEqual(footRule(state), rule, name);
  // The read-only review of step 2 goes back to nothing before it: the
  // folders would be a new import.
  assert.deepEqual(footRule(play(read), { canCreate: true }).primary, {
    kind: "create",
    enabled: true,
  });
  assert.deepEqual(
    footRule(play([{ type: "open" }]), { ready: true }).primary,
    {
      kind: "startScan",
      enabled: true,
    },
  );
});

test("the foot every wizard shares draws « Précédent » as a link at the left, the secondary as a tertiary, then the primary", () => {
  const frame = readFileSync(
    new URL("../components/WizardFrame.tsx", import.meta.url),
    "utf8",
  );
  const foot = frame.slice(frame.indexOf("export function WizardFoot"));
  const previous = foot.indexOf("as-wizard-foot__previous");
  const secondary = foot.indexOf('buttonClassName("tertiary")');
  const primary = foot.indexOf('buttonClassName("primary")');
  assert.ok(previous > 0 && secondary > previous && primary > secondary);
  assert.match(foot, /buttonClassName\("link"\)\} as-wizard-foot__previous/);
  assert.match(foot, /t\("wizard\.previous"\)/);
  // The message is above the row.
  assert.ok(
    foot.indexOf("as-wizard-foot__message") <
      foot.indexOf("as-wizard-foot__row"),
  );
  // Never two links: the secondary is a tertiary button.
  assert.equal((foot.match(/buttonClassName\("link"\)/g) ?? []).length, 1);
});

/* ------------------------------- Pre-ticking ------------------------------- */

test("« Traitements » opens with every target that has frames ticked", () => {
  assert.deepEqual(
    preChosenTargets([
      { targetId: "ngc7000", frames: 72 },
      { targetId: "m31", frames: 0 },
      { targetId: "sh2-103", frames: 40 },
    ]),
    ["ngc7000", "sh2-103"],
  );
  assert.deepEqual(preChosenTargets([]), []);
});

test("« Récapitulatif » opens with every image made ticked, unless the space cannot take it", () => {
  const results: TreatmentResult[] = [
    made("raw"),
    made("dualband-correction"),
    { ...made("raw"), targetId: "m31", designation: "M31", path: "/w/M31.tif" },
    {
      ...made("raw"),
      targetId: "sh2",
      designation: "Sh2-103",
      path: "/w/sh2.tif",
      size: 5e9,
    },
  ];
  // A subscription: everything but the image above the ceiling of a file.
  const open = summaryImages(results, storage());
  assert.deepEqual(
    preTickedImages(open, null).map((key) => key.split("\u0000")[1]),
    [
      "/w/NGC 7000/stacked/raw.tif",
      "/w/NGC 7000/stacked/dualband-correction.tif",
      "/w/M31.tif",
    ],
  );
  // The free space: two captures left, no variants.
  const free = storage({
    captureCount: 1,
    captureCountLimit: 3,
    variantLimit: 0,
  });
  const rows = summaryImages(results, free);
  assert.deepEqual(
    preTickedImages(rows, { remaining: 2 }).map(
      (key) => key.split("\u0000")[1],
    ),
    ["/w/NGC 7000/stacked/raw.tif", "/w/M31.tif"],
  );
  // The reducer pre-ticks with what it knows of the space when the run ends.
  const state = play(
    [
      { type: "storage", storage: free },
      { type: "treatStart", target: "NGC 7000" },
      {
        type: "treatDone",
        made: [made("raw"), made("dualband-correction")],
        errors: [],
      },
    ],
    play(planned),
  );
  assert.equal(
    state.ticked.length,
    1,
    "the variant needs a space that keeps one",
  );
});

test("the step-3 help says the targets are ticked, in the ten languages", () => {
  assert.equal(
    plain(fr("import.treat.cardsLead")),
    plain(
      "Les cibles qui ont des poses sont cochées ; décochez celles à ne pas traiter.",
    ),
  );
  for (const locale of SUPPORTED_LOCALES) {
    const t = createTranslator(locale);
    assert.notEqual(t("import.treat.cardsLead"), "import.treat.cardsLead");
    // The images are ticked too now: no « none ticked by default » left.
    assert.ok(t("detect.treat.headRoomAfter").length <= 1, locale);
  }
});

test("the information above « Traitements » says how long it takes and where the results stay, with a link to the working folder, in the ten languages", () => {
  const t = createTranslator("fr");
  assert.equal(
    plain(t("detect.treat.slow")),
    plain(
      "Le traitement peut prendre du temps selon les performances de votre ordinateur.",
    ),
  );
  assert.equal(
    plain(t("detect.treat.kept")),
    plain(
      "Tous les résultats finaux, même ceux qui ne sont pas envoyés, restent dans votre dossier de travail.",
    ),
  );
  assert.equal(t("detect.treat.openFolder"), "Ouvrir le dossier de travail");
  for (const locale of SUPPORTED_LOCALES) {
    const words = createTranslator(locale);
    for (const key of [
      "detect.treat.head",
      "detect.treat.slow",
      "detect.treat.kept",
      "detect.treat.openFolder",
    ])
      assert.ok(
        words(key).length > 3 && words(key) !== key,
        `${locale} ${key}`,
      );
  }
  // The block is the app's information notice: its icon and its tint.
  const screen = readFileSync(
    new URL("../screens/detect/ImportTreatments.tsx", import.meta.url),
    "utf8",
  );
  assert.match(screen, /<Notice tone="neutral" icon="info">/);
  assert.match(screen, /detect\.treat\.openFolder/);
});

test("the folders the treatments prepared are kept for the link to the working folder", () => {
  const state = importReducer(NO_IMPORT, {
    type: "treatFolders",
    folders: { m31: "/w/n/M 31" },
  });
  assert.deepEqual(state.folders, { m31: "/w/n/M 31" });
  assert.deepEqual(
    importReducer(state, {
      type: "treatFolders",
      folders: { m45: "/w/n/M 45" },
    }).folders,
    { m31: "/w/n/M 31", m45: "/w/n/M 45" },
  );
});

/* ------------------------- The menu and « Sessions » ---------------------- */

test("« Import en cours » is in the menu only while an import has read something, under « Sessions », with its percentage or « à vous »", () => {
  const nights = MENU_GROUPS.find((group) => group.section === "nights")!;
  assert.deepEqual(
    groupEntries(nights, false).map((entry) => entry.id),
    ["sessions", "progression", "logs", "annotations"],
  );
  assert.deepEqual(
    groupEntries(nights, true).map((entry) => entry.id),
    ["sessions", "import", "progression", "logs", "annotations"],
  );
  const app = MENU_GROUPS.find((group) => group.section === "app")!;
  assert.equal(groupEntries(app, true), app.entries);
  // Choosing the folders is still « Sessions »; then the entry is chosen.
  assert.equal(menuEntryOf("import", false), "sessions");
  assert.equal(menuEntryOf("import", true), "import");
  assert.equal(importPresence(play([{ type: "open" }])).shown, false);
  const reading = play([
    { type: "open" },
    { type: "scanStart" },
    { type: "scanProgress", read: 482, total: 1423 },
  ]);
  assert.deepEqual(
    { ...importPresence(reading) },
    {
      shown: true,
      waiting: false,
      done: false,
      percent: 34,
      target: null,
      step: 1,
      job: "scan",
      written: false,
      confirmAbandon: false,
    },
  );
  // Until step 4 creates, nothing is written; « Abandonner » asks first.
  assert.equal(importPresence(play(planned)).written, false);
  assert.equal(importPresence(play(planned)).confirmAbandon, true);
  assert.equal(importPresence(play(createdOnStep4)).written, true);
  const waiting = importPresence(play(read));
  assert.equal(waiting.waiting, true);
  assert.equal(waiting.percent, null);
  const treating = play([
    ...planned,
    { type: "treatStart", target: "NGC 7000" },
    {
      type: "frameStart",
      frame: { overall: 12.7 } as never,
    },
  ]);
  assert.equal(importPresence(treating).percent, 12);
  for (const locale of SUPPORTED_LOCALES) {
    const t = createTranslator(locale);
    for (const key of [
      "nav.import",
      "import.badge.waiting",
      "import.card.running",
      "import.card.waiting",
      "import.card.resume",
      "import.card.abandon",
      "wizard.previous",
      "import.back",
    ])
      assert.notEqual(t(key), key, `${locale} lacks ${key}`);
  }
  assert.equal(fr("nav.import"), "Import en cours");
});

test("the card of « Sessions » says the job, the step and how far it is, or that the import waits", () => {
  const t = (key: string, params?: Record<string, string | number>) =>
    plain(fr(key, params));
  const treating = play([
    ...planned,
    { type: "treatStart", target: "NGC 7000" },
    { type: "frameStart", frame: { overall: 12.4 } as never },
  ]);
  assert.equal(
    importCardLine(importPresence(treating), t),
    "NGC 7000 · Étape 3 · Traitements · 12 %",
  );
  assert.equal(
    importCardLine(importPresence(play([{ type: "skip" }], play(planned))), t),
    "Étape 4 · Récapitulatif",
  );
  const reading = play([
    { type: "open" },
    { type: "scanStart" },
    { type: "scanProgress", read: 40, total: 100 },
  ]);
  assert.equal(
    importCardLine(importPresence(reading), t),
    "Étape 1 · Dossier · lecture des fichiers · 40 %",
  );
});

test("the finish of a pipeline chosen in « Traitements » of the import is held, and left as it is while the treatments run", () => {
  const finishes = {
    M31: { hoo: { contrast: 3.6, colour: 20, softStars: 50 } },
  };
  const held = play([{ type: "finishes", finishes }]);
  assert.deepEqual(held.finishes, finishes);
  const running = { ...held, running: { target: "M31" } };
  assert.equal(
    importReducer(running, { type: "finishes", finishes: {} }).finishes,
    finishes,
  );
});
