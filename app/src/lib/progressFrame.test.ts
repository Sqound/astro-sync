import assert from "node:assert/strict";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import test, { before } from "node:test";
import { fileURLToPath } from "node:url";

import { rolldown } from "rolldown";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";

import { brandLoaderCss } from "../vendor/brand-mark.js";
import { createTranslator } from "../i18n/locale.ts";
import {
  beginProgress,
  progressFrameView,
  withProgressionEvent,
  withStackEvent,
  withWorkProgress,
} from "./treatmentProgress.ts";

// The frame of a treatment that runs, rendered for real: the component is
// bundled on the fly (the test runner reads no JSX) and drawn to a string.

const fr = createTranslator("fr");
const T0 = 1_000_000;

type Frame = typeof import("../components/ProgressFrame.tsx").ProgressFrame;
let ProgressFrame: Frame;

before(async () => {
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
      new URL("../components/ProgressFrame.tsx", import.meta.url),
    ),
    platform: "node",
    plugins: [stubCss],
    external: ["react", "react/jsx-runtime", "react-dom"],
    logLevel: "silent",
  });
  const { output } = await bundle.generate({ format: "esm" });
  // Beside `node_modules`, so `react` resolves from it.
  const folder = new URL(
    "../../node_modules/.cache/astro-sync-tests/",
    import.meta.url,
  );
  await mkdir(folder, { recursive: true });
  const file = new URL("progress-frame.mjs", folder);
  await writeFile(file, output[0].code);
  ProgressFrame = (await import(file.href)).ProgressFrame;
});

const draw = (
  state: ReturnType<typeof beginProgress>,
  now: number,
  stop = false,
  label = "Empilement de M31",
) =>
  renderToStaticMarkup(
    createElement(ProgressFrame, {
      view: progressFrameView(fr, { label, state, now }),
      location: fr("wizard.sending.local"),
      lastLineLabel: fr("wizard.sending.lastLineLabel"),
      stop: stop
        ? { label: fr("wizard.sending.stopStacking"), onStop: () => undefined }
        : undefined,
    }),
  );

test("state A: the stacking at step 6 of 7 draws the star beside a determined bar, the time and the last line", () => {
  let state = beginProgress(T0);
  state = withStackEvent(state, {
    event: "step",
    step: "integration",
    index: 6,
    total: 7,
  });
  state = withStackEvent(state, {
    event: "progress",
    percent: 28.18,
    line: "Integrating 90 images…",
    at: T0 + 130_000,
  });
  const html = draw(state, T0 + 134_000, true);
  assert.match(
    html,
    /<strong>Empilement de M31<\/strong> · intégration, étape 6 sur 7/,
  );
  assert.match(html, /sur cet ordinateur/);
  // The star is the application's loader, before the bar.
  assert.ok(
    html.indexOf("data-brand-loader") < html.indexOf('role="progressbar"'),
  );
  assert.match(html, /role="progressbar"[^>]*aria-valuenow="28.2"/);
  assert.match(html, /width:28.2%/);
  assert.match(html, /28&nbsp;%|28 %/);
  assert.match(html, /en cours depuis 2 min 14 s/);
  assert.match(html, /<code>Integrating 90 images…<\/code>/);
  assert.match(html, /aria-label="Dernière ligne du journal"/);
  // The link that stops this one work, at the level of a link.
  assert.match(
    html,
    /<button type="button" class="as-button as-button--link is-destructive">Arrêter l’empilement<\/button>/,
  );
  assert.ok(!html.includes("dernière ligne il y a"));
});

test("state B: the report of the logs has a sliding bar, no percentage, its counter and the file it reads, and no stop", () => {
  let state = beginProgress(T0);
  state = withWorkProgress(
    state,
    {
      kind: "analysis",
      done: 1,
      total: 2,
      bytesDone: 0,
      bytesTotal: 0,
      current: "NINA_20261007_190512.log",
    },
    T0 + 1000,
  );
  const html = draw(state, T0 + 12_000, false, "Le rapport des journaux");
  assert.match(html, /as-frame__fill as-frame__fill--sliding/);
  assert.ok(!/aria-valuenow/.test(html), "an undetermined bar has no value");
  assert.ok(!html.includes("as-frame__percent"));
  assert.match(html, /fichier 1 sur 2/);
  assert.match(html, /en cours depuis 12 s/);
  assert.match(html, /<code>NINA_20261007_190512.log<\/code>/);
  assert.ok(!html.includes("Arrêter"));
});

test("the progression frame says its milestone as its counter", () => {
  let state = beginProgress(T0);
  for (const event of [
    { event: "stage", stage: "milestones" },
    { event: "milestone", index: 2, total: 3 },
    { event: "progress", percent: 62, line: null, at: T0 },
  ] as const) {
    state = withProgressionEvent(state, event);
  }
  const html = draw(
    state,
    T0 + 68_000,
    false,
    "Images de progression de NGC 7000",
  );
  assert.match(html, /jalon 2 sur 3/);
  assert.match(html, /width:62%/);
  assert.match(html, /en cours depuis 1 min 08 s/);
});

test("a journal silent for 100 seconds says so after its last line", () => {
  const state = withStackEvent(beginProgress(T0), {
    event: "progress",
    percent: null,
    line: "Integrating 90 images…",
    at: T0,
  });
  const html = draw(state, T0 + 100_000);
  assert.match(html, /dernière ligne il y a 1 min 40 s/);
});

test("« Réduire les animations »: the sliding segment becomes a hatched bar, the star is still", async () => {
  const css = await readFile(
    new URL("../theme/primitives.css", import.meta.url),
    "utf8",
  );
  const reduced = css.slice(
    css.indexOf("@media (prefers-reduced-motion: reduce)"),
  );
  assert.match(reduced, /\.as-frame__fill--sliding\s*\{[^}]*animation:\s*none/);
  assert.match(reduced, /repeating-linear-gradient/);
  // The star's own CSS (the portal's figure) stops with the same query, and
  // leaves the figure still, at rest, tilted 22°.
  const loaderCss = brandLoaderCss();
  assert.match(loaderCss, /prefers-reduced-motion:\s*reduce/);
  assert.match(loaderCss, /animation:\s*none/);
});
