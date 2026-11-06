import assert from "node:assert/strict";
import test from "node:test";

import { createTranslator } from "../i18n/locale.ts";
import { afterCopy, closesWindow, promptNames } from "./promptWindow.ts";

// « Demander à un assistant IA » and the prompt's window.

const fr = createTranslator("fr");

test("copied: a success snackbar with « Voir le prompt »; refused: a warning and the window at once", () => {
  assert.deepEqual(afterCopy(true), {
    tone: "success",
    textKey: "wizard.folder.promptCopied",
    openAtOnce: false,
  });
  assert.deepEqual(afterCopy(false), {
    tone: "warning",
    textKey: "wizard.folder.copyRefused",
    openAtOnce: true,
  });
  assert.equal(fr("wizard.folder.viewPrompt"), "Voir le prompt");
  assert.equal(fr("wizard.folder.copyPrompt"), "Copier le prompt");
  assert.equal(
    fr("wizard.folder.promptTitle"),
    "Le prompt pour votre assistant IA",
  );
  // The refusal no longer points under the step: the prompt is in the window.
  assert.doesNotMatch(fr("wizard.folder.copyRefused"), /ci-dessous/);
});

test("the window closes with Échap", () => {
  assert.ok(closesWindow("Escape"));
  assert.ok(!closesWindow("Enter"));
});

test("the prompt is given the common names the night knows, by target folder", () => {
  const online = {
    targets: [
      {
        targetId: "t-m31",
        name: "Galaxie d’Andromède",
        planned: true,
        captures: [],
      },
      { targetId: "t-ic1396", name: "IC 1396", planned: true, captures: [] },
      { targetId: "t-ngc7000", name: null, planned: true, captures: [] },
    ],
  };
  assert.deepEqual(
    promptNames(
      [
        { targetId: "t-m31", folderName: "M31" },
        { targetId: "t-ic1396", folderName: "IC 1396" },
        { targetId: "t-ngc7000", folderName: "NGC 7000" },
      ],
      online,
    ),
    { M31: "Galaxie d’Andromède" },
  );
  assert.deepEqual(
    promptNames([{ targetId: "t-m31", folderName: "M31" }], null),
    {},
  );
});

// `ASSISTANT-IA.md` is made ready at startup and before the copy.

import {
  askAssistant,
  assistantFileSnack,
  ensureAssistantFile,
  replaceAssistantFile,
  shouldPrepareAtStartup,
} from "./promptWindow.ts";

const prepared = (outcome: string) => async () => ({
  assistantFile: { outcome },
});

test("startup: prepares once the folder is known, or once connected", () => {
  assert.ok(shouldPrepareAtStartup({ exists: true, chosen: false }, false));
  assert.ok(shouldPrepareAtStartup({ exists: false, chosen: true }, false));
  assert.ok(shouldPrepareAtStartup({ exists: false, chosen: false }, true));
  assert.ok(!shouldPrepareAtStartup({ exists: false, chosen: false }, false));
});

test("the call at startup goes through the bridge's prepare, once per call", async () => {
  let calls = 0;
  const state = await ensureAssistantFile(async () => {
    calls += 1;
    return { assistantFile: { outcome: "rewritten" } };
  });
  assert.equal(calls, 1);
  assert.deepEqual(state, { kind: "ready" });
});

test("the prompt is copied after the file is prepared, never before", async () => {
  const order: string[] = [];
  await askAssistant({
    prepare: async () => {
      order.push("prepare");
      return { assistantFile: { outcome: "created" } };
    },
    build: async () => {
      order.push("build");
      return "PROMPT";
    },
    copy: async (text) => {
      order.push(`copy:${text}`);
      return true;
    },
    onFile: () => order.push("file"),
    onCopied: (_, copied) => order.push(`copied:${copied}`),
  });
  assert.deepEqual(order, [
    "prepare",
    "file",
    "build",
    "copy:PROMPT",
    "copied:true",
  ]);
});

test("a file the person changed: a warning that says it was not replaced", async () => {
  const state = await ensureAssistantFile(prepared("kept_as_changed"));
  assert.deepEqual(state, { kind: "kept" });
  const snack = assistantFileSnack(state, String, fr);
  assert.equal(snack?.tone, "warning");
  assert.match(snack?.text ?? "", /ASSISTANT-IA\.md a été modifié/);
  assert.match(snack?.text ?? "", /pas été remplacé/);
  assert.equal(snack?.mode, "persistent");
  assert.equal(snack?.replaceable, true);
  assert.equal(
    fr("assistantFile.replace"),
    "Remplacer par la version d’Astro Sync",
  );
  assert.equal(assistantFileSnack({ kind: "ready" }, String, fr), null);
});

test("a write failure: an error in words, and the prompt is still copied", async () => {
  const copied: boolean[] = [];
  const files: unknown[] = [];
  await askAssistant({
    prepare: async () => {
      throw new Error("disque plein");
    },
    build: async () => "PROMPT",
    copy: async () => true,
    onFile: (state) => files.push(state),
    onCopied: (_, ok) => copied.push(ok),
  });
  assert.deepEqual(copied, [true]);
  const state = files[0] as Parameters<typeof assistantFileSnack>[0];
  assert.equal(state.kind, "failed");
  const snack = assistantFileSnack(state, () => "disque plein", fr);
  assert.equal(snack?.tone, "error");
  assert.match(snack?.text ?? "", /n’a pas pu être écrit/);
  assert.match(snack?.text ?? "", /disque plein/);
  assert.match(snack?.text ?? "", /copié quand même/);
});

test("« Remplacer par la version d'Astro Sync »: the copy's name is said, a failure too", async () => {
  const done = await replaceAssistantFile(
    async () => ({
      keptCopy: "/dossier/ASSISTANT-IA.modifie-2026-10-10.md",
    }),
    String,
    fr,
  );
  assert.equal(done.tone, "success");
  assert.match(done.text, /ASSISTANT-IA\.modifie-2026-10-10\.md/);
  const failed = await replaceAssistantFile(
    async () => {
      throw new Error("refusé");
    },
    () => "refusé",
    fr,
  );
  assert.equal(failed.tone, "error");
  assert.match(failed.text, /n’a pas pu être remplacé/);
});
