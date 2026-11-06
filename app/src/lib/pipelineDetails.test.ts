import assert from "node:assert/strict";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import test, { before } from "node:test";
import { fileURLToPath } from "node:url";

import { rolldown } from "rolldown";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";

import { createTranslator } from "../i18n/locale.ts";
import type { PipelineSections } from "./bridge";
import { pipelineSectionViews } from "./settingsTab.ts";
import { dependencyTools } from "./homeFixtures.ts";

// « Détails » under each pipeline of « Pipelines de traitement »:
// the steps the definition gives, in the person's language, folded by
// default. The component is bundled on the fly (the test runner reads no
// JSX) and drawn to a string.

const fr = createTranslator("fr");

type Details =
  typeof import("../components/PipelineDetails.tsx").PipelineDetails;
let PipelineDetails: Details;

before(async () => {
  // The primitives import their style sheets: none is needed for the markup.
  const stubCss = {
    name: "stub-css",
    load(id: string) {
      return id.endsWith(".css")
        ? { code: "export default '';", moduleType: "js" as const }
        : null;
    },
  };
  const bundle = await rolldown({
    input: fileURLToPath(
      new URL("../components/PipelineDetails.tsx", import.meta.url),
    ),
    platform: "node",
    plugins: [stubCss],
    external: ["react", "react/jsx-runtime", "react-dom"],
    logLevel: "silent",
  });
  const { output } = await bundle.generate({ format: "esm" });
  const folder = new URL(
    "../../node_modules/.cache/astro-sync-tests/",
    import.meta.url,
  );
  await mkdir(folder, { recursive: true });
  const file = new URL("pipeline-details.mjs", folder);
  await writeFile(file, output[0].code);
  PipelineDetails = (await import(file.href)).PipelineDetails;
});

const steps = [
  {
    title: "Fond de ciel par GraXpert",
    text: "Le modèle d’IA de GraXpert estime le fond de ciel.",
  },
  { title: "Débruitage léger", text: "GraXpert retire une partie du bruit." },
];

const answer: PipelineSections = {
  sections: [
    {
      id: "polished",
      name: "Rendu soigné",
      description: "Le rendu naturel poussé plus loin.",
      source: "shipped",
      author: "Astro Sync",
      licence: "MIT",
      trust: "not_needed",
      scriptSha256: null,
      dependencies: [],
      details: steps,
    },
    {
      id: "mine",
      name: "Le mien",
      description: "Sans étapes.",
      source: "user",
      author: "Camille",
      licence: "MIT",
      trust: "not_needed",
      scriptSha256: null,
      dependencies: [],
    },
  ],
  ignored: [],
};

test("the view of each pipeline carries its steps, none for a definition that gives none", () => {
  const [polished, mine] = pipelineSectionViews(
    fr,
    "fr",
    answer,
    dependencyTools({}),
    {},
  );
  assert.deepEqual(polished.details, steps);
  assert.deepEqual(mine.details, []);
  assert.equal(fr("deps.pipelines.details"), "Détails");
  assert.equal(createTranslator("ja")("deps.pipelines.details"), "詳細");
});

test("« Détails » is folded by default: one link-level button, no step shown", () => {
  const html = renderToStaticMarkup(
    createElement(PipelineDetails, { label: "Détails", steps }),
  );
  assert.match(
    html,
    /<button type="button" class="as-button as-button--link" aria-expanded="false"/,
  );
  assert.match(html, />Détails<\/button>/);
  assert.ok(!html.includes("Fond de ciel par GraXpert"));
});

test("unfolded, the steps are listed in order with what each does", () => {
  const html = renderToStaticMarkup(
    createElement(PipelineDetails, {
      label: "Détails",
      steps,
      initiallyOpen: true,
    }),
  );
  assert.match(html, /aria-expanded="true"/);
  assert.match(html, /<ol id="[^"]+" class="as-pipeline__steps">/);
  const first = html.indexOf("<strong>Fond de ciel par GraXpert</strong>");
  const second = html.indexOf("<strong>Débruitage léger</strong>");
  assert.ok(first > 0 && second > first, html);
  assert.match(html, /Le modèle d’IA de GraXpert estime le fond de ciel\./);
});

test("nothing is drawn for a pipeline without steps", () => {
  assert.equal(
    renderToStaticMarkup(
      createElement(PipelineDetails, { label: "Détails", steps: [] }),
    ),
    "",
  );
});

test("the tab draws the details under the description of each pipeline", async () => {
  const source = await readFile(
    new URL("../screens/DependenciesTab.tsx", import.meta.url),
    "utf8",
  );
  const description = source.indexOf("{view.origin} · {view.description}");
  const details = source.indexOf("<PipelineDetails");
  const dependencies = source.indexOf("{view.dependencies.map(");
  assert.ok(description > 0 && details > description && dependencies > details);
  assert.match(source, /label=\{t\("deps\.pipelines\.details"\)\}/);
});
