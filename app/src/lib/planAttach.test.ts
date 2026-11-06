import assert from "node:assert/strict";
import test from "node:test";

import { createTranslator } from "../i18n/locale.ts";
import type {
  AttachGate,
  NightAttachment,
  PlanChoice,
  PlanChoices,
} from "./bridge";
import { nightTag, refusalWords } from "./multiNight.ts";
import {
  attachRefusalWords,
  blockView,
  choiceView,
  createRows,
  listed,
  loadAttachment,
  loadChoices,
  saveAttachment,
  saveChoices,
  searchNone,
  searchReady,
  windowIntro,
} from "./planAttach.ts";

const fr = createTranslator("fr");
const en = createTranslator("en");

const OPEN: AttachGate = { allowed: true, closed: null };
const RESERVED: AttachGate = {
  allowed: false,
  closed: "not_in_subscription",
};
// The service never heard (a private server never closes them).
const UNKNOWN: AttachGate = { allowed: false, closed: "unknown" };

const HEART = {
  projectId: "p-heart",
  name: "IC 1805",
  commonName: "Nébuleuse du Cœur",
  nightCount: 3,
  totalSeconds: 40_080,
};

const choice = (extra: Partial<PlanChoice> = {}): PlanChoice => ({
  projectId: "p-heart",
  name: "IC 1805",
  commonName: "Nébuleuse du Cœur",
  targetId: "ic-1805",
  goalMinutes: 720,
  nightCount: 3,
  totalSeconds: 40_080,
  state: "available",
  fromProjectId: null,
  fromName: null,
  ...extra,
});

test("a night in no plan offers the link; without the right the link gives way to the sentence of « Traitements » and « Voir les offres »", () => {
  const none = (gate: AttachGate): NightAttachment => ({
    plans: [],
    gate,
  });
  const open = blockView(fr, "fr", {
    attachment: none(OPEN),
    failed: false,
    cached: null,
  });
  assert.deepEqual(open, {
    kind: "none",
    text: "Cette session ne fait partie d’aucun plan multi-nuits.",
    attach: true,
    closed: null,
  });
  const reserved = blockView(fr, "fr", {
    attachment: none(RESERVED),
    failed: false,
    cached: null,
  });
  assert.equal(reserved.kind === "none" && reserved.attach, false);
  assert.equal(
    reserved.kind === "none" && reserved.closed?.text,
    "Le rattachement à un plan multi-nuits est réservé à Planification avancée.",
  );
  // The offer is named in each language as it already is elsewhere.
  assert.equal(
    en("planAttach.reserved"),
    "Attaching a night to a multi-night plan is reserved to Advanced Planning.",
  );
  assert.equal(reserved.kind === "none" && reserved.closed?.offers, true);
  // The service never heard: the same sentence of « Traitements », no offer.
  const priv = blockView(fr, "fr", {
    attachment: none(UNKNOWN),
    failed: false,
    cached: null,
  });
  assert.equal(priv.kind === "none" && priv.attach, false);
  assert.equal(priv.kind === "none" && priv.closed?.offers, false);
  assert.equal(
    priv.kind === "none" && priv.closed?.text,
    fr("multiNight.unknownService"),
  );
  // Reading: nothing is said yet.
  assert.deepEqual(
    blockView(fr, "fr", { attachment: null, failed: false, cached: null }),
    { kind: "loading" },
  );
});

test("a night in a plan says « Plan multi-nuits : … » and offers « Détacher de ce plan », unless the right is missing", () => {
  const attached = (gate: AttachGate): NightAttachment => ({
    plans: [HEART],
    gate,
  });
  const open = blockView(fr, "fr", {
    attachment: attached(OPEN),
    failed: false,
    cached: null,
  });
  assert.equal(open.kind, "attached");
  if (open.kind !== "attached") return;
  assert.equal(
    fr("planAttach.attachedTo", { title: open.plans[0].title }),
    "Plan multi-nuits : IC 1805 — Nébuleuse du Cœur",
  );
  assert.equal(open.plans[0].facts, "3 nuits · 11 h 08");
  assert.equal(open.detach, true);
  assert.equal(fr("planAttach.detach"), "Détacher de ce plan");
  // The plan is still read without the right, but cannot be left.
  const reserved = blockView(fr, "fr", {
    attachment: attached(RESERVED),
    failed: false,
    cached: null,
  });
  assert.equal(reserved.kind === "attached" && reserved.detach, false);
  assert.equal(reserved.kind === "attached" && reserved.closed?.offers, true);
});

test("offline: the last answer is shown, with its phrase and no gesture; with none, the phrase and « Réessayer », never nothing", () => {
  const cached: NightAttachment = { plans: [HEART], gate: OPEN };
  const offline = blockView(fr, "fr", {
    attachment: null,
    failed: true,
    cached,
  });
  assert.equal(offline.kind, "attached");
  if (offline.kind !== "attached") return;
  assert.equal(offline.detach, false);
  assert.equal(offline.closed, null);
  assert.ok(offline.offline?.startsWith("Hors ligne :"));
  // A night in no plan, offline: the link is not offered.
  const bare = blockView(fr, "fr", {
    attachment: null,
    failed: true,
    cached: { plans: [], gate: OPEN },
  });
  assert.equal(bare.kind === "none" && bare.attach, false);
  const nothing = blockView(fr, "fr", {
    attachment: null,
    failed: true,
    cached: null,
  });
  assert.deepEqual(nothing, {
    kind: "failed",
    text: "Impossible de lire vos plans multi-nuits pour l’instant.",
  });
});

test("a plan of the window: its facts, the night it takes from another plan, the plan the night is in", () => {
  const plain = choiceView(fr, "fr", choice());
  assert.equal(plain.title, "IC 1805 — Nébuleuse du Cœur");
  assert.equal(plain.facts, "3 nuits · 11 h 08 · objectif 12 h");
  assert.equal(plain.note, null);
  assert.equal(plain.disabled, false);

  // A night of another plan: attaching moves it, said under the plan.
  const moves = choiceView(
    fr,
    "fr",
    choice({
      state: "moves",
      fromProjectId: "p-soul",
      fromName: "IC 1848",
    }),
  );
  assert.equal(moves.note, "Déplace la session du plan IC 1848");
  assert.equal(
    choiceView(en, "en", choice({ state: "moves" })).note,
    "Moves the session out of another plan",
  );

  const current = choiceView(fr, "fr", choice({ state: "current" }));
  assert.equal(current.disabled, true);
  assert.deepEqual(current.tag, {
    tone: "success",
    text: "Cette session y est",
  });
  const other = choiceView(fr, "fr", choice({ state: "other_target" }));
  assert.equal(other.disabled, false);
  assert.equal(other.tag?.text, "Autre cible");
  // A plan with no time and no goal says its nights alone.
  assert.equal(
    choiceView(
      fr,
      "fr",
      choice({ nightCount: 1, totalSeconds: 0, goalMinutes: null }),
    ).facts,
    "1 nuit",
  );
});

test("what the window says above the list, and when nothing is found", () => {
  const proposed = (plans: PlanChoice[], extra: Partial<PlanChoices> = {}) => ({
    plans,
    noTarget: false,
    total: plans.length,
    ...extra,
  });
  assert.equal(
    windowIntro(fr, proposed([choice()]), ["IC 1805"]),
    "Plans multi-nuits de IC 1805. Un clic choisit le plan\u00a0: la session y est rattachée quand vous continuez.",
  );
  assert.equal(
    windowIntro(fr, proposed([], { total: 4 }), ["IC 1805", "M31"]),
    "Aucun plan multi-nuits pour IC 1805, M31. Cherchez un plan par le nom de sa cible, ou créez-en un.",
  );
  assert.equal(
    windowIntro(fr, proposed([], { total: 0 }), ["IC 1805"]),
    "Vous n’avez encore aucun plan multi-nuits. Créez-en un pour cette session.",
  );
  assert.equal(
    windowIntro(fr, proposed([choice()], { noTarget: true }), []),
    "Cette session n’a pas de cible renseignée : tous vos plans multi-nuits sont proposés.",
  );
  assert.equal(windowIntro(fr, null, ["IC 1805"]), null);
  assert.equal(searchNone(fr, " zzz "), "Aucun plan ne répond à « zzz ».");
  // The search replaces the proposed plans; without a search they stand.
  const all = proposed([choice()]);
  const found = [choice({ projectId: "p-m31", name: "M31" })];
  assert.deepEqual(listed(all, found), found);
  assert.deepEqual(listed(all, null), all.plans);
  assert.deepEqual(listed(null, null), []);
  assert.equal(searchReady("  "), false);
  assert.equal(searchReady("m"), true);
});

test("« Créer un plan multi-nuits »: one row for the night's target, one per target when there are several", () => {
  const one = createRows(fr, [{ targetId: "ic-1805", name: "IC 1805" }]);
  assert.equal(one.length, 1);
  assert.equal(one[0].label, "Créer un plan multi-nuits");
  assert.equal(
    one[0].body.startsWith("Pour IC 1805, avec cette session dedans."),
    true,
  );
  assert.deepEqual(one[0].target, {
    targetId: "ic-1805",
    name: "IC 1805",
    commonName: null,
  });
  const several = createRows(fr, [
    { targetId: "ic-1805", name: "IC 1805" },
    { targetId: "m-31", name: null },
    { targetId: null, name: "Sans identifiant" },
  ]);
  assert.deepEqual(
    several.map((row) => row.label),
    [
      "Créer un plan multi-nuits pour IC 1805",
      "Créer un plan multi-nuits pour m-31",
    ],
  );
  assert.deepEqual(createRows(fr, []), []);
});

test("the words of a refusal of the service are said in clear, the plan's own right in « Planification avancée »", () => {
  assert.equal(
    attachRefusalWords(fr, "target_mismatch", "words"),
    "cette session n’est pas de la cible de ce plan.",
  );
  assert.equal(
    attachRefusalWords(fr, "multi_night_not_active", ""),
    "le rattachement à un plan multi-nuits est réservé à Planification avancée.",
  );
  // The integrations' refusal keeps « Synchronisation avancée+ ».
  assert.equal(
    refusalWords(fr, "multi_night_not_active", ""),
    "le plan multi-nuits dans Astro Sync est réservé à Synchronisation avancée+.",
  );
  assert.equal(
    refusalWords(en, "target_mismatch", ""),
    "this session is not of this plan’s target.",
  );
  assert.equal(
    refusalWords(fr, "night_not_found", ""),
    "cette session n’est plus dans votre espace.",
  );
  assert.equal(
    attachRefusalWords(fr, "capability_required", "m"),
    "votre offre ne permet pas de créer un plan multi-nuits.",
  );
});

test("the last answers are kept per night for the offline state, and a storage that refuses is no harm", () => {
  const memory = new Map<string, string>();
  const storage = {
    getItem: (key: string) => memory.get(key) ?? null,
    setItem: (key: string, value: string) => void memory.set(key, value),
  };
  const attachment: NightAttachment = { plans: [HEART], gate: OPEN };
  saveAttachment(storage, "n-10", attachment);
  assert.deepEqual(loadAttachment(storage, "n-10"), attachment);
  assert.equal(loadAttachment(storage, "n-11"), null);
  const choices: PlanChoices = { plans: [choice()], noTarget: false, total: 1 };
  saveChoices(storage, "n-10", choices);
  assert.deepEqual(loadChoices(storage, "n-10"), choices);
  // No storage, or one that throws: nothing is kept, nothing breaks.
  assert.equal(loadChoices(null, "n-10"), null);
  saveChoices(null, "n-10", choices);
  const broken = {
    getItem: () => {
      throw new Error("blocked");
    },
    setItem: () => {
      throw new Error("blocked");
    },
  };
  saveAttachment(broken, "n-10", attachment);
  assert.equal(loadAttachment(broken, "n-10"), null);
  memory.set("astro-sync.nightAttachment", "not json");
  assert.equal(loadAttachment(storage, "n-10"), null);
});

test("« Nuits »: a night attached by itself has no integration time to say (the tag keeps the plan and the nights)", () => {
  const tags = [
    { projectId: "p-heart", name: "IC 1805", nightCount: 2, totalSeconds: 0 },
  ];
  assert.equal(nightTag(fr, "fr", tags), "Plan multi-nuits IC 1805 · 2 nuits");
  assert.equal(nightTag(en, "en", tags), "Multi-night plan IC 1805 · 2 nights");
});
