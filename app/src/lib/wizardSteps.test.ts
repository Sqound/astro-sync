import assert from "node:assert/strict";
import test from "node:test";

import {
  continueWithTargets,
  folderLine,
  folderStepAction,
  folderWasKnown,
  folderTree,
  openingState,
  openingStep,
  stepReading,
  targetChoices,
  targetFolderName,
  thread,
} from "./wizardSteps.ts";
import { IC, M31, NGC, detail, online } from "./homeFixtures.ts";
import { createTranslator } from "../i18n/locale.ts";

const fr = createTranslator("fr");
const plain = (text: string) => text;

test("the thread: five steps, those before ticked, the current one numbered; the sending keeps « Récapitulatif », the night sent ticks them all", () => {
  assert.deepEqual(
    thread("cibles").map((s) => [fr(s.labelKey), s.status]),
    [
      ["Cibles", "current"],
      ["Dossier", "waiting"],
      ["Fichiers", "waiting"],
      ["Traitements", "waiting"],
      ["Récapitulatif", "waiting"],
    ],
  );
  assert.deepEqual(
    thread("fichiers").map((s) => s.status),
    ["done", "done", "current", "waiting", "waiting"],
  );
  assert.equal(thread("sending")[4].status, "current");
  assert.ok(thread("sent").every((s) => s.status === "done"));
  assert.equal(openingStep(null), "cibles");
  assert.equal(openingStep("fichiers"), "fichiers");
});

test("« Quelles cibles ? »: each target with its debriefing and its folder's name in grey; « Continuer avec 3 cibles »", () => {
  const choices = targetChoices(
    fr,
    online("night-7", [NGC, IC, M31]),
    detail(
      "night-7",
      [NGC, IC, M31],
      [
        { targetId: NGC, acquiredMinutes: 140 },
        { targetId: IC, acquiredMinutes: 130 },
        { targetId: M31, acquiredMinutes: 100 },
      ],
    ),
    new Set(),
  );
  assert.deepEqual(
    choices.map((c) => [c.designation, c.line, c.folderName, c.ticked]),
    [
      ["NGC 7000", "Faite · Intégration 2 h 20", "NGC 7000/", true],
      ["IC 1396", "Faite · Intégration 2 h 10", "IC 1396/", true],
      ["M31", "Faite · Intégration 1 h 40", "M31/", true],
    ],
  );
  assert.equal(continueWithTargets(fr, "fr", 3), "Continuer avec 3 cibles");
  assert.equal(continueWithTargets(fr, "fr", 1), "Continuer avec 1 cible");
  const unticked = targetChoices(
    fr,
    online("night-7", [NGC, IC]),
    null,
    new Set([IC]),
  );
  assert.deepEqual(
    unticked.map((c) => [c.ticked, c.line]),
    [
      [true, null],
      [false, null],
    ],
  );
  assert.equal(targetFolderName('Sh2-155: "Cave"'), "Sh2-155- -Cave-");
});

test("« Dossier »: the folder created with the ticked targets only, and its tree", () => {
  const outcome = {
    created: true,
    targets: [
      {
        targetId: NGC,
        designation: "NGC 7000",
        folderName: "NGC 7000",
        state: "created" as const,
      },
      {
        targetId: IC,
        designation: "IC 1396",
        folderName: "IC 1396",
        state: "created" as const,
      },
      {
        targetId: M31,
        designation: "M31",
        folderName: "M31",
        state: "created" as const,
      },
    ],
  };
  assert.equal(
    plain(folderLine(fr, "fr", outcome)),
    "Dossier créé dans votre dossier de travail, avec un dossier pour chacune des 3 cibles cochées.",
  );
  const tree = folderTree(
    "Astro Sync",
    "2026-10-07 · Col des Étoiles · 7f3a2c",
    ["NGC 7000", "IC 1396", "M31"],
  );
  assert.deepEqual(
    tree.map((r) => `${"  ".repeat(r.depth)}${r.name}`),
    [
      "Astro Sync/",
      "  ASSISTANT-IA.md",
      "  2026-10-07 · Col des Étoiles · 7f3a2c/",
      "    logs/",
      "    calibration/",
      "      darks/",
      "      flats/",
      "      dark-flats/",
      "      biases/",
      "    NGC 7000/",
      "      lights/",
      "      final/",
      "    IC 1396/",
      "    M31/",
    ],
  );
  assert.equal(tree[12].note, "lights/, final/");
  assert.equal(
    fr(tree[1].noteKey!),
    "explique cette structure à un assistant IA",
  );
});

test("« Demander à un assistant IA » the words around the copied prompt, in « vous »", () => {
  assert.equal(
    plain(fr("wizard.folder.assistNote")),
    "Vous ne savez pas comment déposer vos fichiers ? Votre assistant IA, s’il a accès à cet ordinateur, peut vous aider grâce à ce prompt : il lui explique la structure et ce qui est attendu.",
  );
  assert.equal(
    plain(fr("wizard.folder.promptCopied")),
    "Prompt copié. Collez-le dans un assistant IA qui a accès à cet ordinateur et à vos dossiers, par exemple Claude en mode Cowork ou ChatGPT en mode agent.",
  );
  assert.equal(
    fr("wizard.folder.promptGiveTo"),
    "Donnez ce texte à un assistant IA qui a accès à cet ordinateur et à vos dossiers, par exemple Claude en mode Cowork ou ChatGPT en mode agent.",
  );
  assert.equal(fr("wizard.folder.askAssistant"), "Demander à un assistant IA");
  assert.equal(fr("wizard.folder.continue"), "Continuer vers les fichiers");
});

test("« Dossier »: the step creates the folder at each opening without a question; only two copies are asked about", () => {
  // A folder found: the core is idempotent, it keeps what is there and makes
  // what is missing (a target ticked since).
  assert.equal(folderStepAction({ state: "found", folder: "/n" }), "prepare");
  // No folder: made straight away, whether this computer knew one or not
  // (Astro Sync creates the folder itself, without asking anything).
  assert.equal(folderStepAction({ state: "missing" }), "prepare");
  // Two copies: the person chooses.
  assert.equal(
    folderStepAction({ state: "duplicated", folders: ["/a", "/b"] }),
    "choose",
  );
  // A folder known before and gone: said beside the new one, never asked.
  assert.equal(folderWasKnown({ state: "missing" }, true), true);
  assert.equal(folderWasKnown({ state: "missing" }, false), false);
  assert.equal(folderWasKnown({ state: "found", folder: "/n" }, true), false);
});

test("the opening of the wizard never waits without an end", () => {
  const base = {
    contextRead: false,
    hasSession: false,
    phase: null,
    viewFailed: false,
  } as const;
  // Reading: the star.
  assert.equal(openingState(base), "loading");
  assert.equal(
    openingState({ ...base, contextRead: true, hasSession: true }),
    "loading",
  );
  // A step known and the night known: drawn.
  assert.equal(
    openingState({
      ...base,
      contextRead: true,
      hasSession: true,
      phase: "dossier",
    }),
    "ready",
  );
  // A step named by « Nuits » but a night that is nowhere: said, not drawn
  // as an empty frame.
  assert.equal(
    openingState({ ...base, contextRead: true, phase: "fichiers" }),
    "not-found",
  );
  // The night is neither in the list nor on the service: said, not awaited.
  assert.equal(openingState({ ...base, contextRead: true }), "not-found");
  // The night's page failed to read before a step was known: the error.
  assert.equal(
    openingState({
      ...base,
      contextRead: true,
      hasSession: true,
      viewFailed: true,
    }),
    "failed",
  );
});

test("a step that needs the night's page: drawn, waiting, or stuck with its error (« Fichiers », « Traitements »)", () => {
  assert.equal(stepReading({}, false), "ready");
  assert.equal(stepReading({}, true), "ready");
  assert.equal(stepReading(null, false), "loading");
  assert.equal(stepReading(null, true), "failed");
});
