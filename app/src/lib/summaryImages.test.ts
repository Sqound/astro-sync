import assert from "node:assert/strict";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import test, { before } from "node:test";
import { fileURLToPath } from "node:url";

import { rolldown } from "rolldown";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";

import { SUPPORTED_LOCALES, createTranslator } from "../i18n/locale.ts";
import type { SpaceStorage } from "./bridge";
import {
  canTickImage,
  sendOrder,
  summaryImages,
  type SummaryImage,
  type TreatmentResult,
} from "./detectTreatments.ts";
import { summaryImageView } from "./summaryImages.ts";

// « Images à envoyer » of the detection listed one image per target -- the
// first pipeline's -- although every pipeline ticked had made its own. The
// Récapitulatif lists them all: the first as the capture, the others as its
// variants beneath it, each with its box, its thumbnail, its path and the
// words of the space's limits.

const fr = createTranslator("fr");
const flat = (text: string) => text.replaceAll(" ", " ").replaceAll(" ", " ");

const FOLDER = "/captures/AstroSync/IC 1805 · 3976f1/stacked";
const result = (
  pipelineId: string,
  size: number,
  extra: Partial<TreatmentResult> = {},
): TreatmentResult => ({
  targetId: "ic1805",
  designation: "IC 1805",
  pipelineId,
  nights: 3,
  path: `${FOLDER}/IC_1805_plan_${pipelineId}.png`,
  size,
  ...extra,
});
/** A real run: three nights, two pipelines ticked. */
const TWO = [
  result("raw", 46_519_544),
  result("dualband-correction", 46_782_247),
];

/** The profile of a subscription (no cap) or of the free space. */
const storage = (over: Partial<SpaceStorage> = {}): SpaceStorage =>
  ({
    usedBytes: 0,
    quotaBytes: 1_000_000_000_000,
    remainingBytes: 1_000_000_000_000,
    maxFileBytes: 1_073_741_824,
    captureCount: 0,
    captureCountLimit: 9007199254740991,
    variantLimit: 9007199254740991,
    ...over,
  }) as SpaceStorage;
const FREE = storage({
  maxFileBytes: 26_214_400,
  captureCountLimit: 3,
  variantLimit: 0,
});

const names: Record<string, string> = {
  raw: "Rendu naturel",
  "dualband-correction": "Correction bibande",
};
const pipelineName = (id: string) => names[id] ?? id;

test("every pipeline's image is listed: the first as the capture, the others as its variants beneath it", () => {
  const rows = summaryImages(TWO, storage());
  assert.deepEqual(
    rows.map((row) => [row.pipelineId, row.role, row.blocked]),
    [
      ["raw", "capture", null],
      ["dualband-correction", "variant", null],
    ],
  );
  const [capture, variant] = rows.map((row) =>
    summaryImageView(fr, "fr", row, pipelineName),
  );
  assert.equal(capture.title, "IC 1805");
  assert.equal(capture.render, "Rendu naturel");
  assert.equal(flat(capture.meta), "3 nuits empilées · PNG · 46,5 Mo");
  assert.equal(variant.title, "Variante de la capture de IC 1805");
  assert.equal(variant.render, "Correction bibande");
  assert.equal(flat(variant.meta), "PNG · 46,8 Mo");
});

test("the natural render is the capture whatever the order of the pipelines; a render with the curve of presentation is never one", () => {
  const rows = summaryImages(
    [
      result("dualband-correction", 2),
      result("raw", 3, { contrast: 2, path: `${FOLDER}/a.tif` }),
      result("raw", 3, { contrast: 2.5, path: `${FOLDER}/b.tif` }),
      result("raw", 3, {
        colour: 40,
        softStars: 85,
        path: `${FOLDER}/c.tif`,
      }),
      result("raw", 1),
    ],
    null,
  );
  assert.deepEqual(
    rows.map((row) => [row.pipelineId, row.contrast ?? 0, row.role]),
    [
      ["raw", 0, "capture"],
      ["dualband-correction", 0, "variant"],
      ["raw", 2, "variant"],
      ["raw", 2.5, "variant"],
      ["raw", 0, "variant"],
    ],
  );
  const curve = summaryImageView(fr, "fr", rows[2], pipelineName);
  assert.equal(flat(curve.render), "Rendu naturel · Présentation · 2,0");
  // A value between the detents says its number; a finish says its settings.
  const custom = summaryImageView(fr, "fr", rows[3], pipelineName);
  assert.equal(flat(custom.render), "Rendu naturel · Contraste 2,5");
  const finish = summaryImageView(fr, "fr", rows[4], pipelineName);
  assert.equal(
    flat(finish.render),
    "Rendu naturel · Couleurs +40 % · Étoiles adoucies 85 %",
  );
});

test("a target keeps its own rows; the targets stay in order", () => {
  const rows = summaryImages(
    [
      result("raw", 1),
      {
        ...result("raw", 1),
        targetId: "m31",
        designation: "M31",
        path: "/m31.tif",
      },
      result("dualband-correction", 1),
    ],
    null,
  );
  assert.deepEqual(
    rows.map((row) => [row.targetId, row.role]),
    [
      ["ic1805", "capture"],
      ["ic1805", "variant"],
      ["m31", "capture"],
    ],
  );
});

test("size guard: an image above the space's per-file ceiling cannot be ticked, and says why", () => {
  const rows = summaryImages(
    [result("raw", 46_519_544)],
    storage({ maxFileBytes: 26_214_400 }),
  );
  assert.deepEqual(rows[0].blocked, {
    kind: "tooHeavy",
    size: 46_519_544,
    max: 26_214_400,
  });
  assert.equal(
    flat(summaryImageView(fr, "fr", rows[0], pipelineName).blocked ?? ""),
    "Trop lourde pour votre espace (46,5 Mo > 26,2 Mo)",
  );
  assert.equal(canTickImage(rows[0], rows, [], null), false);
  // At the ceiling exactly, it still goes.
  const edge = summaryImages(
    [result("raw", 26_214_400)],
    storage({ maxFileBytes: 26_214_400 }),
  );
  assert.equal(edge[0].blocked, null);
});

test("size guard: the profile unknown, no guard", () => {
  const rows = summaryImages(TWO, null);
  assert.deepEqual(
    rows.map((row) => row.blocked),
    [null, null],
  );
  assert.equal(canTickImage(rows[1], rows, [], null), true);
});

test("a space that keeps no variant does not tick the variants; the place left is counted by target", () => {
  const rows = summaryImages(TWO, FREE);
  // Both are too heavy for the free space too (46 MB > 25 MB): that says first.
  assert.deepEqual(
    rows.map((row) => row.blocked?.kind),
    ["tooHeavy", "tooHeavy"],
  );
  const light = summaryImages(
    [result("raw", 1_000_000), result("dualband-correction", 1_000_000)],
    FREE,
  );
  assert.equal(light[0].blocked, null);
  assert.equal(light[1].blocked?.kind, "noVariants");
  // With a subscription the variants of a target ticked take no place.
  const several = [
    ...summaryImages(TWO, storage()),
    ...summaryImages(
      [
        {
          ...result("raw", 1),
          targetId: "m31",
          designation: "M31",
          path: "/m.tif",
        },
      ],
      storage(),
    ),
  ];
  const room = { remaining: 1 };
  const [capture, variant, other] = several;
  assert.equal(canTickImage(capture, several, [], room), true);
  assert.equal(canTickImage(variant, several, [capture.key], room), true);
  assert.equal(canTickImage(other, several, [capture.key], room), false);
  assert.equal(
    canTickImage(other, several, [capture.key, variant.key], room),
    false,
  );
});

test("what leaves: the first row ticked of a target is its capture, the others its variants; nothing ticked, nothing leaves", () => {
  const rows = summaryImages(TWO, storage());
  const [capture, variant] = rows;
  assert.deepEqual(sendOrder(rows, []), []);
  assert.deepEqual(
    sendOrder(rows, [variant.key, capture.key]).map((s) => [
      s.row.pipelineId,
      s.variantOf?.pipelineId ?? null,
    ]),
    [
      ["raw", null],
      ["dualband-correction", "raw"],
    ],
  );
  // The capture left unticked: the variant ticked goes as the capture.
  assert.deepEqual(
    sendOrder(rows, [variant.key]).map((s) => s.variantOf),
    [null],
  );
});

/* ------------------------------ The list drawn ------------------------------ */

type Entry = {
  SummaryImages: typeof import("../screens/detect/SummaryImages.tsx").SummaryImages;
  I18nProvider: typeof import("../i18n/index.tsx").I18nProvider;
};
let entry: Entry;

before(async () => {
  const stub = {
    name: "stub-css-and-entry",
    resolveId(id: string) {
      return id === "virtual:entry" ? id : null;
    },
    load(id: string) {
      if (id === "virtual:entry") {
        const base = fileURLToPath(new URL("../", import.meta.url));
        return {
          code: `export { SummaryImages } from ${JSON.stringify(base + "screens/detect/SummaryImages.tsx")};
export { I18nProvider } from ${JSON.stringify(base + "i18n/index.tsx")};`,
          moduleType: "js" as const,
        };
      }
      return id.endsWith(".css")
        ? { code: "export default '';", moduleType: "js" as const }
        : null;
    },
  };
  const bundle = await rolldown({
    input: "virtual:entry",
    platform: "node",
    plugins: [stub],
    external: ["react", "react/jsx-runtime", "react-dom"],
    logLevel: "silent",
  });
  const { output } = await bundle.generate({ format: "esm" });
  const folder = new URL(
    "../../node_modules/.cache/astro-sync-tests/",
    import.meta.url,
  );
  await mkdir(folder, { recursive: true });
  const file = new URL("summary-images.mjs", folder);
  await writeFile(file, output[0].code);
  entry = await import(file.href);
});

function draw(rows: readonly SummaryImage[], ticked: string[] = []): string {
  return renderToStaticMarkup(
    createElement(
      entry.I18nProvider,
      { locale: "fr", onLocaleChange: () => undefined },
      createElement(entry.SummaryImages, {
        rows,
        ticked,
        room: null,
        pipelineName,
        onToggle: () => undefined,
      }),
    ),
  );
}

test("the Récapitulatif with two pipelines: two rows, each with its box, its path as a link and « Afficher dans le Finder »", () => {
  const html = flat(draw(summaryImages(TWO, storage())));
  assert.equal((html.match(/<li /g) ?? []).length, 2);
  assert.equal((html.match(/type="checkbox"/g) ?? []).length, 2);
  assert.ok(!html.includes("disabled"), "both boxes open");
  assert.ok(!/checked=""/.test(html), "nothing ticked by default");
  assert.match(html, /<strong>IC 1805<\/strong> · Rendu naturel/);
  assert.match(html, /3 nuits empilées · PNG · 46,5 Mo/);
  assert.match(
    html,
    /<strong>Variante de la capture de IC 1805<\/strong> · Correction bibande/,
  );
  assert.match(html, /PNG · 46,8 Mo/);
  // The path is the link that opens the file.
  for (const row of TWO) {
    assert.ok(html.includes(`>${row.path}</button>`), row.path);
  }
  assert.equal(
    (html.match(/Ouvrir avec l’application par défaut/g) ?? []).length,
    2,
  );
  assert.equal(
    (html.match(/Afficher dans (le Finder|l’Explorateur|le dossier)/g) ?? [])
      .length,
    2,
  );
  // The variant stands beneath its capture.
  assert.match(html, /data-role="capture"[\s\S]*data-role="variant"/);
});

test("the Récapitulatif with a size guard: the box is disabled, with the reason", () => {
  const html = flat(
    draw(summaryImages(TWO, storage({ maxFileBytes: 26_214_400 }))),
  );
  assert.equal((html.match(/disabled/g) ?? []).length, 2);
  assert.match(html, /Trop lourde pour votre espace \(46,5 Mo &gt; 26,2 Mo\)/);
  assert.match(html, /Trop lourde pour votre espace \(46,8 Mo &gt; 26,2 Mo\)/);
});

/* --------------------------------- The wiring -------------------------------- */

const source = (path: string) =>
  readFile(new URL(path, import.meta.url), "utf8");

test("the flow keeps every render and every plan image it made, and sends the capture first", async () => {
  // The run and the sending belong to the import; its
  // « Récapitulatif » draws the list.
  const flow = await source("../components/ImportProvider.tsx");
  const summary = await source("../screens/detect/ImportSummary.tsx");
  assert.ok(
    !flow.includes("outcomes[0]"),
    "the plan's other pipelines are no longer dropped",
  );
  assert.ok(
    !/renders\.find\(/.test(flow),
    "the first render only is no longer kept",
  );
  assert.match(flow, /for \(const plan of plans\)/);
  assert.match(flow, /for \(const render of first\.renders\)/);
  assert.match(flow, /summaryImages\(made, held\.storage\)/);
  assert.match(flow, /importSends\(rows, made, held\.ticked\)/);
  // The capture of a target goes first and the others name it, whatever the
  // kind of image (a night's render or a plan's).
  const sends = await source("../lib/importSend.ts");
  assert.match(sends, /sendOrder\(rows, ticked\)/);
  assert.match(sends, /variantOfSha256/);
  assert.match(summary, /<SummaryImages/);
});

test("the open, reveal and preview commands are registered and only touch the results in `stacked/`", async () => {
  const lib = await source("../../src-tauri/src/lib.rs");
  for (const name of [
    "result_file_open",
    "result_file_reveal",
    "result_file_preview",
  ]) {
    assert.ok(lib.includes(`commands_detect::${name}`), name);
  }
  const commands = await source("../../src-tauri/src/commands_detect.rs");
  assert.match(commands, /STACKED_DIR/);
  assert.match(commands, /variant_of_sha256/);
});

test("every language says the new words of the list", async () => {
  const keys = [
    "detect.send.tooHeavy",
    "detect.send.noVariants",
    "detect.send.openTitle",
    "detect.send.revealFinder",
    "detect.send.revealExplorer",
    "detect.send.revealFolder",
    "detect.send.previewAlt",
    "detect.send.noPreview",
    "detect.send.noRoom",
    "detect.send.room",
  ];
  for (const locale of SUPPORTED_LOCALES) {
    const dictionary = JSON.parse(
      await source(`../i18n/${locale}.json`),
    ) as Record<string, string>;
    for (const key of keys) {
      assert.ok((dictionary[key] ?? "").trim().length > 0, `${locale} ${key}`);
    }
    assert.match(dictionary["detect.send.tooHeavy"], /\{size\}.*\{max\}/u);
    assert.match(dictionary["detect.send.previewAlt"], /\{file\}/u);
  }
  const french = JSON.parse(await source("../i18n/fr.json")) as Record<
    string,
    string
  >;
  assert.equal(
    flat(french["detect.send.noRoom"]),
    flat(
      "Une capture par cible, ses variantes en dessous ; rien n’est envoyé sans votre case.",
    ),
  );
});

test("the row of a render's TIFF says « TIFF pleine résolution », a TIFF alone « TIFF seul »", () => {
  const png = result("raw", 13_000_000);
  const rows = summaryImages(
    [
      png,
      {
        ...result("raw", 47_000_000),
        path: `${FOLDER}/IC_1805_plan_raw.tif`,
        fullResolution: true,
        twinOf: png.path,
      },
      {
        ...result("linear", 20_000_000),
        path: `${FOLDER}/IC_1805_plan_linear.tif`,
      },
    ],
    storage(),
  );
  assert.deepEqual(
    rows.map((row) => flat(summaryImageView(fr, "fr", row, pipelineName).meta)),
    [
      // The nights stacked are said on the capture's row only.
      "3 nuits empilées · PNG · 13,0 Mo",
      "TIFF pleine résolution · 47,0 Mo",
      "TIFF seul · 20,0 Mo",
    ],
  );
});
