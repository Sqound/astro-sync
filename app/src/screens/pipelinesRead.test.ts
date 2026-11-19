import assert from "node:assert/strict";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import test, { before } from "node:test";
import { fileURLToPath } from "node:url";

import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { rolldown } from "rolldown";

import { pipelinesActions } from "../lib/pipelinesRead.ts";

// « Traitements » of the import while the pipelines are read: the star and its
// sentence in each card, the way to read again when it failed, its own words
// when there is nothing to offer. The zones are bundled on the fly and drawn
// to a string.

type Harness = typeof import("./pipelinesReadHarness.tsx");
let harness: Harness;
let fr: Record<string, string>;

before(async () => {
  const stubCss = {
    name: "stub-css",
    load(id: string) {
      return /\.(css|jpg|png|svg)$/.test(id)
        ? { code: "export default '';", moduleType: "js" as const }
        : null;
    },
  };
  const bundle = await rolldown({
    input: fileURLToPath(
      new URL("./pipelinesReadHarness.tsx", import.meta.url),
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
  const file = new URL("pipelines-read.mjs", folder);
  await writeFile(file, output[0].code);
  harness = await import(file.href);
  fr = JSON.parse(
    await readFile(new URL("../i18n/fr.json", import.meta.url), "utf8"),
  ) as Record<string, string>;
});

const inFrench = (child: unknown) =>
  renderToStaticMarkup(
    createElement(
      harness.I18nProvider,
      { locale: "fr", onLocaleChange: () => {} },
      child as never,
    ),
  );

const card = (status: "loading" | "ready" | "failed") =>
  inFrench(
    createElement(harness.CardPipelines, {
      status,
      names: "Rendu naturel · Correction bibande",
    }),
  );
const notice = (zone: "waiting" | "failed" | "none" | "listed") =>
  inFrench(
    createElement(harness.PipelinesReadNotice, { zone, onRetry: () => {} }),
  );

test("while the pipelines are read, the card shows the star and its sentence, no names", () => {
  const html = card("loading");
  assert.ok(html.includes("as-waiting"));
  assert.ok(html.includes('role="status"'));
  assert.ok(html.includes(fr["import.treat.wait"]));
  assert.ok(!html.includes("Rendu naturel"));
});

test("once read, the star is gone and the names are there", () => {
  const html = card("ready");
  assert.ok(!html.includes("as-waiting"));
  assert.ok(html.includes("Rendu naturel · Correction bibande"));
});

test("a card with no render shows no empty line", () => {
  assert.equal(
    inFrench(
      createElement(harness.CardPipelines, { status: "ready", names: "" }),
    ),
    "",
  );
});

test("a reading that failed shows no star and no names in the card", () => {
  const html = card("failed");
  assert.equal(html, "");
});

test("a reading that failed says so with « Réessayer », an empty list says its own words", () => {
  const failed = notice("failed");
  assert.ok(failed.includes(fr["import.treat.pipelinesFailed"]));
  assert.ok(failed.includes(fr["wizard.retry"]));
  assert.ok(failed.includes('role="alert"'));
  assert.ok(!failed.includes(fr["import.treat.pipelinesNone"]));
  const none = notice("none");
  assert.ok(none.includes(fr["import.treat.pipelinesNone"]));
  assert.ok(!none.includes(fr["wizard.retry"]));
  assert.equal(notice("waiting"), "");
  assert.equal(notice("listed"), "");
});

test("the three states are three different zones, and nothing starts or opens before the list is read", () => {
  const zones = [
    pipelinesActions("loading", 0),
    pipelinesActions("failed", 0),
    pipelinesActions("ready", 0),
    pipelinesActions("ready", 3),
  ];
  assert.deepEqual(
    zones.map((zone) => zone.zone),
    ["waiting", "failed", "none", "listed"],
  );
  for (const zone of zones.slice(0, 3)) {
    assert.equal(zone.launch, false);
    assert.equal(zone.edit, false);
  }
  assert.equal(zones[3].launch, true);
  assert.equal(zones[3].edit, true);
});

test("once the pipelines are listed, « Modifier » and the launch are both usable, whatever the tools", () => {
  // The state after `pipelines_list` alone (the tools are asked apart): the
  // names are shown, so both buttons are usable, as the card says.
  const listed = pipelinesActions("ready", 3);
  assert.equal(listed.edit, true);
  assert.equal(listed.launch, true);
  // Before it, neither is, and the card shows the star.
  const waiting = pipelinesActions("loading", 0);
  assert.equal(waiting.edit, false);
  assert.equal(waiting.launch, false);
});

const dialogOf = (toolsRead: boolean) =>
  inFrench(
    createElement(harness.PipelineDialog, {
      target: {
        targetId: "t1",
        designation: "IC 1396",
        frames: 38,
        duration: null,
        filter: null,
        band: "broadband",
      },
      pipelines: harness.PIPELINES,
      tools: toolsRead ? harness.tools() : [],
      toolsRead,
      initial: ["raw"],
      finishDefault: { contrast: 0, colour: 0, softStars: 0 } as never,
      initialFinishes: {},
      several: true,
      engine: "siril",
      progress: {},
      onInstall: () => {},
      onTrust: () => {},
      onCancel: () => {},
      onValidate: () => {},
      onApplyAll: () => {},
    } as never),
  );

test("the window opened before the tools are read lists the pipelines, says the tools are checked, shows no tool absent and holds « Valider »", () => {
  const html = dialogOf(false);
  assert.ok(html.includes("Rendu naturel"));
  assert.ok(html.includes("Palette HOO"));
  assert.ok(html.includes(fr["import.treat.toolsWait"]));
  assert.ok(html.includes("as-waiting"));
  assert.ok(!html.includes(fr["wizard.pipelines.tool.absent"]));
  assert.ok(!html.includes(fr["wizard.pipelines.tool.notInstalled"]));
  assert.ok(!html.includes("as-pill"));
  assert.ok(!html.includes("as-button--primary"), "no primary « Valider »");
  assert.ok(html.includes("disabled"));
});

test("once the tools are read the same window shows its pills and a usable « Valider »", () => {
  const html = dialogOf(true);
  assert.ok(!html.includes(fr["import.treat.toolsWait"]));
  assert.ok(html.includes("as-pill"));
  assert.ok(html.includes("as-button--primary"));
});

test("the ten languages say the four new sentences, with the sentence of waiting ending on an ellipsis", async () => {
  const folder = new URL("../i18n/", import.meta.url);
  for (const locale of [
    "fr",
    "en",
    "de",
    "es",
    "it",
    "ja",
    "pt-BR",
    "ru",
    "zh-Hans",
    "zh-Hant",
  ]) {
    const words = JSON.parse(
      await readFile(new URL(`${locale}.json`, folder), "utf8"),
    ) as Record<string, string>;
    for (const key of [
      "import.treat.wait",
      "import.treat.toolsWait",
      "import.treat.pipelinesFailed",
      "import.treat.pipelinesNone",
    ]) {
      assert.ok(words[key], `${locale} ${key}`);
      assert.ok(!/[;—]/.test(words[key]), `${locale} ${key} punctuation`);
    }
    assert.ok(words["import.treat.wait"].endsWith("…"), locale);
  }
});
