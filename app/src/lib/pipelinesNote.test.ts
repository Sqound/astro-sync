import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

import { PIPELINES_DOC_URL, pipelinesNote } from "./pipelinesNote.ts";
import { SUPPORTED_LOCALES, createTranslator } from "../i18n/locale.ts";

test("the note has its two links, each opening the right target", async () => {
  const opened: string[] = [];
  const note = pipelinesNote({
    openUrl: (url) => void opened.push(`web:${url}`),
    openFolder: () => void opened.push("folder"),
  });
  assert.equal(note.textKey, "wizard.treatments.pipelinesNote.text");
  assert.deepEqual(
    note.links.map((link) => [link.kind, link.target]),
    [
      [
        "web",
        "https://github.com/sqound/astro-sync/blob/main/stacking/pipelines/SCHEMA.md",
      ],
      ["folder", null],
    ],
  );
  assert.equal(PIPELINES_DOC_URL, note.links[0].target);
  for (const link of note.links) await link.open();
  assert.deepEqual(opened, [`web:${PIPELINES_DOC_URL}`, "folder"]);
});

test("the note and its links are said in the ten languages", async () => {
  const note = pipelinesNote({
    openUrl: () => undefined,
    openFolder: () => undefined,
  });
  for (const locale of SUPPORTED_LOCALES) {
    const dictionary = JSON.parse(
      await readFile(
        new URL(`../i18n/${locale}.json`, import.meta.url),
        "utf8",
      ),
    ) as Record<string, string>;
    for (const key of [
      note.textKey,
      ...note.links.map((link) => link.labelKey),
    ]) {
      assert.ok(dictionary[key], `${locale}: ${key}`);
    }
  }
});

test("the note is one component, shown in the window of a target, under the render of a plan and in Réglages › Dépendances, and the folder command exists", async () => {
  const read = (path: string) =>
    readFile(new URL(path, import.meta.url), "utf8");
  const step = await read("../screens/night/TreatmentsStep.tsx");
  const extras = await read("../components/RenderExtras.tsx");
  const dialog = await read("../screens/night/PipelineDialog.tsx");
  const settings = await read("../screens/DependenciesTab.tsx");
  const component = await read("../components/PipelinesNote.tsx");
  // The single place that builds the note and its links.
  assert.match(
    component,
    /pipelinesNote\(\{\s*openUrl: openInBrowser,\s*openFolder: pipelinesFolderOpen/,
  );
  assert.match(component, /noticeClassName\("neutral"\)/);
  for (const source of [extras, dialog, settings]) {
    assert.match(source, /<PipelinesNote \/>/);
    assert.doesNotMatch(source, /pipelinesNote\(/, "no second copy");
    assert.doesNotMatch(source, /pipelinesNote\.(text|doc|folder)/);
  }
  // In the head of « Pipelines pour {cible} », and under the
  // render of a plan.
  assert.equal((step.match(/<RenderExtras\b/g) ?? []).length, 1);
  assert.ok(
    dialog.indexOf("<PipelinesNote") < dialog.indexOf("view.cards.map"),
    "the note before the cards",
  );
  const bridge = await read("./bridge.ts");
  assert.match(bridge, /invoke<void>\("pipelines_folder_open"\)/);
  const readme = await read("../../../README.md");
  assert.match(readme, /stacking\/pipelines\/SCHEMA\.md/);
});

test("Réglages › Dépendances: the note stands under the title of « Pipelines de traitement », before the list of the pipelines", async () => {
  const settings = await readFile(
    new URL("../screens/DependenciesTab.tsx", import.meta.url),
    "utf8",
  );
  const title = settings.indexOf('t("deps.pipelines.title")');
  const note = settings.indexOf("<PipelinesNote />");
  const list = settings.indexOf("views.map(");
  assert.ok(title > 0 && note > title && list > note);
  // Same sentence and same two links as « Traitements »: the existing keys,
  // the documentation of the format on the repository, then the folder.
  const fr = createTranslator("fr");
  const shown = pipelinesNote({
    openUrl: () => undefined,
    openFolder: () => undefined,
  });
  assert.ok(fr(shown.textKey).length > 0);
  assert.deepEqual(
    shown.links.map((link) => link.labelKey),
    [
      "wizard.treatments.pipelinesNote.doc",
      "wizard.treatments.pipelinesNote.folder",
    ],
  );
  assert.equal(
    PIPELINES_DOC_URL,
    "https://github.com/sqound/astro-sync/blob/main/stacking/pipelines/SCHEMA.md",
  );
});
