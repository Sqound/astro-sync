import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

import { createTranslator, SUPPORTED_LOCALES } from "../i18n/locale.ts";
import { applyRecap, type ApplyDeps, type ApplyInput } from "./importApply.ts";
import {
  NO_IMPORT,
  importReducer,
  recapProgress,
  recapStepText,
  stepStage,
  type Created,
  type ImportAction,
  type ImportState,
  type RecapPlan,
} from "./importFlow.ts";

// The primary of step 4 says what it is doing inside each of its parts, as
// the engine tells it (`import-progress`): the sessions being created and how
// many, the image being prepared and of how many. It never stays on one
// sentence while the engine works, a part that fails gives the button back
// with what was ticked and the main image chosen, and a second try does not
// do twice what went through.

const fr = createTranslator("fr");
const plain = (text: string) => text.replace(/[  ]/g, " ");
const t = (key: string, params?: Record<string, string | number>) =>
  plain(fr(key, params));

const PLAN = {
  sessions: 3,
  updated: 0,
  plans: 0,
  images: 2,
  incidents: [],
  incidentCount: 0,
  tidy: null,
  logs: [],
} as unknown as RecapPlan;

const sending: ImportState = {
  ...NO_IMPORT,
  alive: true,
  step: 4,
  reached: 4,
  local: { request: {}, outcome: { sessions: [], multiNight: [] } } as never,
  ticked: ["a", "b"],
  mainChoice: { "m45\u0000series": "b" },
  sending: true,
};

const play = (state: ImportState, actions: ImportAction[]) =>
  actions.reduce(importReducer, state);

test("each word of the engine belongs to one part of the primary", () => {
  assert.equal(stepStage({ step: "service" }), "sessions");
  assert.equal(stepStage({ step: "folders" }), "sessions");
  assert.equal(stepStage({ step: "prepare" }), "send");
  assert.equal(stepStage({ step: "check" }), "send");
  assert.equal(stepStage({ step: "queue" }), "send");
  // A word a later engine would add is left unsaid, never shown raw.
  assert.equal(stepStage({ step: "something-new" }), null);
});

test("« Création des sessions… » says how many sessions the space is creating, then the folders", () => {
  let state = play(sending, [{ type: "working", working: "sessions" }]);
  // Before the engine said anything: the sentence of the part.
  assert.equal(recapStepText(state, t, "fr"), null);
  assert.equal(recapProgress(state, PLAN, t, "fr"), "Création des sessions…");
  state = play(state, [
    { type: "workingStep", step: { step: "service", total: 3 } },
  ]);
  assert.equal(
    recapProgress(state, PLAN, t, "fr"),
    "Création de 3 sessions dans votre espace…",
  );
  state = play(state, [
    { type: "workingStep", step: { step: "service", total: 1 } },
  ]);
  assert.equal(
    recapProgress(state, PLAN, t, "fr"),
    "Création de 1 session dans votre espace…",
  );
  state = play(state, [{ type: "workingStep", step: { step: "folders" } }]);
  assert.equal(
    recapProgress(state, PLAN, t, "fr"),
    "Mise à jour des dossiers des nuits…",
  );
});

test("« Mise en file des images… » says which image is being prepared, of how many", () => {
  const created = {
    request: {},
    outcome: { sessions: [], multiNight: [] },
  } as unknown as Created;
  let state = play({ ...sending, created }, [
    { type: "working", working: "send" },
    { type: "workingStep", step: { step: "prepare", done: 1, total: 2 } },
  ]);
  assert.equal(
    recapProgress(state, PLAN, t, "fr"),
    "Sessions créées · Préparation de l’image 1 sur 2…",
  );
  state = play(state, [
    { type: "workingStep", step: { step: "prepare", done: 2, total: 2 } },
  ]);
  assert.match(recapProgress(state, PLAN, t, "fr") ?? "", /image 2 sur 2…$/);
  state = play(state, [
    { type: "workingStep", step: { step: "check", total: 2 } },
  ]);
  assert.match(
    recapProgress(state, PLAN, t, "fr") ?? "",
    /Vérification de ce que votre espace contient déjà…$/,
  );
  // The rows written: the sentence of the part again.
  state = play(state, [
    { type: "workingStep", step: { step: "queue", total: 2 } },
  ]);
  assert.match(
    recapProgress(state, PLAN, t, "fr") ?? "",
    /Mise en file des images…$/,
  );
});

test("a word of the engine is kept only while its part runs", () => {
  // Not sending: nothing is kept.
  const idle = play({ ...sending, sending: false }, [
    { type: "workingStep", step: { step: "prepare", done: 1, total: 2 } },
  ]);
  assert.equal(idle.workingStep, null);
  // A word of another part (late, or early) is not said under this one.
  let state = play(sending, [
    { type: "working", working: "sessions" },
    { type: "workingStep", step: { step: "prepare", done: 1, total: 2 } },
  ]);
  assert.equal(state.workingStep, null);
  // The next part starts on its own sentence.
  state = play(state, [
    { type: "workingStep", step: { step: "service", total: 3 } },
    { type: "working", working: "send" },
  ]);
  assert.equal(state.workingStep, null);
  assert.match(recapProgress(state, PLAN, t, "fr") ?? "", /Mise en file/);
});

test("a part that fails gives the button back, keeps the ticks and the main image, and says why", () => {
  for (const [code, words] of [
    ["not_paired", /pas encore connecté à un espace/],
    ["network_error", /Impossible de joindre le service.*réessayez/],
    ["service_start_timeout", /n’a pas fini de démarrer.*réessayez/],
  ] as const) {
    // What `formatError` says of a command's error: the words of its code.
    const error = fr(`errors.${code}`, { message: "" });
    const state = play(sending, [
      { type: "working", working: "sessions" },
      { type: "workingStep", step: { step: "service", total: 3 } },
      { type: "sendFailed", error },
    ]);
    assert.equal(state.sending, false, code);
    assert.equal(state.working, null);
    assert.equal(state.workingStep, null);
    assert.match(plain(state.error ?? ""), words);
    assert.deepEqual(state.ticked, ["a", "b"]);
    assert.deepEqual(state.mainChoice, { "m45\u0000series": "b" });
    assert.equal(state.step, 4);
  }
});

test("a second try after a failure of the queuing creates no session again and queues once", async () => {
  const calls: string[] = [];
  const created = {
    request: {},
    outcome: { sessions: [], multiNight: [] },
  } as unknown as Created;
  let fail = true;
  const deps = {
    create: async () => {
      calls.push("create");
      return { outcome: created.outcome, nights: [] };
    },
    settlePlans: async () => [],
    incidentsApply: async () => {
      throw new Error("not called");
    },
    workingFolder: async () => "/w",
    organisePlan: async () => {
      throw new Error("not called");
    },
    organise: async () => {
      throw new Error("not called");
    },
    enqueue: async () => {
      calls.push("enqueue");
      if (fail) throw { code: "network_error", message: "" };
    },
  } as unknown as ApplyDeps;
  const image = {
    kind: "night",
    sessionId: "s1",
    targetId: "m45",
    render: { path: "/w/n/M45/stacked/a.png", sha256: "aa", size: 8 },
  } as never;
  const input: ApplyInput = {
    local: created,
    created: null,
    plan: { incidents: [], tidy: null, logs: [] },
    images: [image],
    report: { plans: null, incidents: {}, tidy: null } as never,
    sent: false,
  };
  let kept: Created | null = null;
  const phases: string[] = [];
  const hooks = {
    phase: (work: string) => phases.push(work),
    created: (made: Created) => {
      kept = made;
    },
    plansDone: () => undefined,
    incidentsDone: () => undefined,
    tidyDone: () => undefined,
  } as never;
  const first = await applyRecap(deps, input, hooks);
  assert.equal(first.ok, false);
  assert.equal(!first.ok && first.stage, "send");
  fail = false;
  const second = await applyRecap(deps, { ...input, created: kept }, hooks);
  assert.equal(second.ok, true);
  assert.deepEqual(calls, ["create", "enqueue", "enqueue"]);
  assert.deepEqual(phases, ["sessions", "send", "send"]);
});

test("the foot shows the star beside the sentence and in the greyed button while the primary runs", () => {
  const summary = readFileSync(
    new URL("../screens/detect/ImportSummary.tsx", import.meta.url),
    "utf8",
  );
  const foot = summary.slice(summary.indexOf("<WizardFoot"));
  assert.match(
    foot,
    /className="as-recap-progress" aria-busy=\{state\.sending\}/,
  );
  assert.match(foot, /state\.sending \? \(\s*<Waiting\s+size="inline"/);
  assert.match(foot, /busy: state\.sending/);
  const frame = readFileSync(
    new URL("../components/WizardFrame.tsx", import.meta.url),
    "utf8",
  );
  assert.match(frame, /aria-busy=\{primary\.busy \? true : undefined\}/);
  assert.match(frame, /primary\.busy \? \(\s*<BrandLoader/);
  // One height for the line, with or without the star: nothing moves.
  const css = readFileSync(
    new URL("../theme/shell.css", import.meta.url),
    "utf8",
  );
  assert.match(css, /\.as-recap-progress \{[^}]*min-height: 1\.5rem/);
  // The star stands still when the person asks for less motion.
  const mark = readFileSync(
    new URL("../vendor/brand-mark.js", import.meta.url),
    "utf8",
  );
  assert.match(mark, /prefers-reduced-motion/);
});

test("the words of the steps exist in the ten languages, with their figures", () => {
  assert.equal(SUPPORTED_LOCALES.length, 10);
  for (const locale of SUPPORTED_LOCALES) {
    const words = createTranslator(locale);
    const texts = [
      words("import.recap.step.service.one", { count: 1 }),
      words("import.recap.step.service.other", { count: 3 }),
      words("import.recap.step.folders"),
      words("import.recap.step.prepare", { done: 1, total: 2 }),
      words("import.recap.step.check"),
    ];
    for (const text of texts) {
      assert.doesNotMatch(text, /import\.recap\.step/, locale);
      assert.doesNotMatch(text, /[;；—{}]/, `${locale}: ${text}`);
      assert.match(text, /…$/, `${locale}: ${text}`);
    }
    assert.match(texts[1], /3/, locale);
    assert.match(texts[3], /1.*2/, locale);
    // « espace », never « compte ».
    assert.doesNotMatch(texts.join(" "), /compte|account|Konto|cuenta/i);
  }
});
