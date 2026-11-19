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
import { mainImageName, summaryImageView } from "./summaryImages.ts";
import { groupKeyOfRow, mainGroups, type MainContext } from "./mainImage.ts";

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
  assert.equal(flat(capture.meta), "3 nuits empilées · PNG · 44,4 Mo");
  assert.equal(variant.title, "Variante de la capture de IC 1805");
  assert.equal(variant.render, "Correction bibande");
  assert.equal(flat(variant.meta), "PNG · 44,6 Mo");
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
    "Trop lourde pour votre espace (44,4 Mo > 25,0 Mo)",
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

/** Nothing known of the space: every group is alone in the world. */
const NO_CONTEXT: MainContext = {
  sessionOf: {},
  existing: new Set(),
  reads: {},
  shaOf: {},
  variantsAllowed: true,
};

function draw(
  rows: readonly SummaryImage[],
  ticked: string[] = [],
  main: {
    choices?: Record<string, string>;
    context?: MainContext;
    open?: boolean;
  } = {},
): string {
  const context = main.context ?? NO_CONTEXT;
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
        groups: mainGroups(rows, ticked, main.choices ?? {}, context),
        mainContext: context,
        onChoose: () => undefined,
        choiceOpen: main.open ?? true,
      }),
    ),
  );
}

test("the Récapitulatif with two pipelines: two rows, each with its box, its path as a link and « Afficher dans le Finder »", () => {
  const html = flat(draw(summaryImages(TWO, storage())));
  assert.equal((html.match(/<li class="as-treat__image/g) ?? []).length, 2);
  assert.equal((html.match(/type="checkbox"/g) ?? []).length, 2);
  assert.ok(!/type="checkbox"[^>]*disabled/.test(html), "both boxes open");
  assert.ok(!/checked=""/.test(html), "nothing ticked by default");
  assert.match(html, /<strong>IC 1805<\/strong> · Rendu naturel/);
  assert.match(html, /3 nuits empilées · PNG · 44,4 Mo/);
  assert.match(
    html,
    /<strong>Variante de la capture de IC 1805<\/strong> · Correction bibande/,
  );
  assert.match(html, /PNG · 44,6 Mo/);
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
  assert.equal((html.match(/type="checkbox"[^>]*disabled/g) ?? []).length, 2);
  assert.match(html, /Trop lourde pour votre espace \(44,4 Mo &gt; 25,0 Mo\)/);
  assert.match(html, /Trop lourde pour votre espace \(44,6 Mo &gt; 25,0 Mo\)/);
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
  assert.match(
    flow,
    /importSends\(rows, made, held\.ticked, mainPicksOf\(held\)\)/,
  );
  // The capture of a target goes first and the others name it, whatever the
  // kind of image (a night's render or a plan's).
  const sends = await source("../lib/importSend.ts");
  assert.match(sends, /sendOrder\(rows, ticked, picks\)/);
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
      "Une capture par cible, ses variantes en dessous. Rien n’est envoyé sans votre case.",
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
      "3 nuits empilées · PNG · 12,4 Mo",
      "TIFF pleine résolution · 44,8 Mo",
      "TIFF seul · 19,1 Mo",
    ],
  );
});

/* ----------------------- The main image of a capture ----------------------- */

// The screen of the choice, drawn as the window draws it (the markup is the
// one the captures show): the group, its radios, its lines and the titles that
// follow the choice.

const FOUR = [
  result("raw", 12_400_000),
  result("polished", 12_400_000),
  result("hoo", 12_400_000),
  result("dualband-correction", 12_400_000),
];
names.polished = "Rendu soigné";
names.hoo = "Palette HOO";
const SESSION = "night-1";
const digest = (pipelineId: string) => `d-${pipelineId}`.padEnd(64, "0");

/** The four renders, as the import of an existing night draws them: `held`
 * is what the space holds for the night (`images`: `[pipelineId, main]`). */
function scene(
  held: null | {
    images: [string, boolean][];
    visibility?: "private" | "public";
    failed?: boolean;
  },
  choices: Record<string, string> = {},
  open = true,
  made: TreatmentResult[] = FOUR,
) {
  const rows = summaryImages(made, storage());
  const ticked = rows.map((row) => row.key);
  const context: MainContext = {
    sessionOf: Object.fromEntries(rows.map((row) => [row.key, SESSION])),
    existing: new Set(held ? [SESSION] : []),
    reads: held
      ? {
          [SESSION]: held.failed
            ? "failed"
            : ({
                sessionId: SESSION,
                targets: [
                  {
                    targetId: "ic1805",
                    name: "IC 1805",
                    planned: true,
                    captures: [
                      {
                        id: "cap-1",
                        targetIds: ["ic1805"],
                        source: "stacking",
                        stage: "draft",
                        visibility: held.visibility ?? "private",
                        uploaded: true,
                        createdAt: "2026-11-12T20:00:00Z",
                        mainImage: held.images
                          .filter(([, main]) => main)
                          .map(([id]) => ({
                            id,
                            main: true,
                            sha256: digest(id),
                            byteSize: 1,
                            originalName: `${id}.png`,
                            source: "stacking",
                            createdAt: "2026-11-12T20:00:00Z",
                            skySolution: "missing",
                          }))[0],
                        imageCount: held.images.length,
                        images: held.images.map(([id, main]) => ({
                          id,
                          main,
                          sha256: digest(id),
                          byteSize: 1,
                          originalName: `${id}.png`,
                          source: "stacking",
                          createdAt: "2026-11-12T20:00:00Z",
                          skySolution: "missing",
                        })),
                      },
                    ],
                  },
                ],
              } as never),
        }
      : {},
    shaOf: Object.fromEntries(
      rows.map((row) => [row.key, digest(row.pipelineId)]),
    ),
    variantsAllowed: true,
  };
  return { rows, html: flat(draw(rows, ticked, { choices, context, open })) };
}

const radios = (html: string) => html.match(/<input type="radio"[^>]*>/g) ?? [];
const groupKey = (rows: readonly SummaryImage[]) => groupKeyOfRow(rows[0]);

test("the default: one group named « Image principale », a radio on each card, the natural render main, nothing to read about the space", () => {
  const { html } = scene(null);
  assert.match(html, /IC 1805 · une capture, 4 images/);
  assert.match(html, /Image principale : Rendu naturel/);
  assert.match(html, /role="radiogroup" aria-labelledby="[^"]+:title"/);
  const found = radios(html);
  assert.equal(found.length, 4);
  assert.equal(found.filter((input) => input.includes("checked")).length, 1);
  assert.equal(found.filter((input) => input.includes("disabled")).length, 0);
  assert.equal((html.match(/>Image principale<\/span>/g) ?? []).length, 4);
  // One name for the four radios: a group.
  assert.equal(new Set(found.map((i) => /name="([^"]+)"/.exec(i)![1])).size, 1);
  assert.ok(!html.includes("Garder l’image principale"));
  // The titles: the capture, then the variants of it.
  assert.match(html, /<strong>IC 1805<\/strong> · Rendu naturel/);
  assert.equal(
    (html.match(/Variante de la capture de IC 1805/g) ?? []).length,
    3,
  );
  // The subtitle of the list is two sentences.
  assert.ok(!html.includes(" ; "));
});

test("a choice: the titles follow it, the order of the cards does not move", () => {
  const rows = summaryImages(FOUR, storage());
  const { html } = scene(null, { [groupKey(rows)]: rows[2].key });
  assert.match(html, /Image principale : Palette HOO/);
  assert.match(html, /<strong>IC 1805<\/strong> · Palette HOO/);
  assert.match(
    html,
    /<strong>Variante de la capture de IC 1805<\/strong> · Rendu naturel/,
  );
  // Raw, polished, HOO, dual-band: the same order as without a choice.
  const order = [
    ...html.matchAll(
      / · (Rendu naturel|Rendu soigné|Palette HOO|Correction bibande)</g,
    ),
  ].map((m) => m[1]);
  assert.deepEqual(order, [
    "Rendu naturel",
    "Rendu soigné",
    "Palette HOO",
    "Correction bibande",
  ]);
});

test("one image alone: a sentence and no choice", () => {
  const { html } = scene(null, {}, true, [result("raw", 12_400_000)]);
  assert.match(html, /IC 1805 · une capture, 1 image/);
  assert.ok(
    html.includes(flat("Une seule image : elle sera l’image principale.")),
    "Une seule image : elle sera l’image principale.",
  );
  assert.equal(radios(html).length, 0);
  assert.ok(!html.includes("radiogroup"));
});

test("the capture is online and private: « Garder » is ticked, with its render and its date, and the images are variants", () => {
  const { html } = scene({ images: [["raw", true]] });
  assert.match(html, /Garder l’image principale de votre espace/);
  assert.match(html, /Rendu naturel · en ligne depuis le 12 nov\./);
  assert.match(html, /Image principale : Rendu naturel/);
  const found = radios(html);
  assert.equal(found.length, 5, "the four cards and « Garder »");
  assert.equal(found.filter((input) => input.includes("checked")).length, 1);
  assert.equal(
    (html.match(/Variante de la capture de IC 1805/g) ?? []).length,
    4,
  );
  assert.ok(!html.includes("devient une variante"));
  // The image that is the main one of the space already says so.
  assert.match(html, /C’est déjà l’image principale de votre espace\./);
});

test("the deliberate gesture says what happens: the old image becomes a variant, the new one is the main image", () => {
  const rows = summaryImages(FOUR, storage());
  const { html } = scene(
    { images: [["raw", true]] },
    { [groupKey(rows)]: rows[1].key },
  );
  assert.ok(
    html.includes(
      flat(
        "« Rendu naturel » devient une variante. « Rendu soigné » est l’image principale.",
      ),
    ),
    "« Rendu naturel » devient une variante. « Rendu soigné » est l’image principale.",
  );
  assert.ok(
    html.includes(flat("Image principale : Rendu soigné")),
    "Image principale : Rendu soigné",
  );
  assert.match(html, /<strong>IC 1805<\/strong> · Rendu soigné/);
});

test("a public capture: the choice is closed and the sentence explains it", () => {
  const { html } = scene({ images: [["raw", true]], visibility: "public" });
  assert.ok(
    html.includes(
      flat(
        "Capture publique : son image principale ne change pas depuis Astro Sync. Vos images arrivent en variantes. Changez l’image principale depuis votre espace.",
      ),
    ),
    "Capture publique : son image principale ne change pas depuis Astro Sync. Vos images arrivent en variantes. Changez l’image principale depuis votre espace.",
  );
  const found = radios(html);
  assert.equal(found.length, 4);
  assert.ok(found.every((input) => input.includes("disabled")));
  assert.ok(!found.some((input) => input.includes("checked")));
  assert.ok(!html.includes("Garder l’image principale"));
  assert.equal(
    (html.match(/Variante de la capture de IC 1805/g) ?? []).length,
    4,
  );
});

test("the space could not be read: a grey line, today's choice", () => {
  const { html } = scene({ images: [], failed: true });
  assert.match(html, /État de votre espace non lu\./);
  assert.ok(!html.includes("Garder l’image principale"));
  assert.equal(radios(html).length, 4);
});

test("bytes online as a variant: the choice of that image is closed, and the card says why", () => {
  const { html } = scene({
    images: [
      ["raw", true],
      ["polished", false],
    ],
  });
  assert.match(
    html,
    /Déjà en ligne en variante\. Changez l’image principale depuis votre espace\./,
  );
});

test("an image not ticked cannot be chosen, and the card says so; nothing can be chosen while the images are sent", () => {
  const rows = summaryImages(FOUR, storage());
  const some = flat(
    draw(rows, [rows[0].key, rows[1].key], { context: NO_CONTEXT }),
  );
  assert.equal(
    (some.match(/Cochez cette image pour la choisir\./g) ?? []).length,
    2,
  );
  const sending = scene(null, {}, false).html;
  assert.ok(radios(sending).every((input) => input.includes("disabled")));
});

test("a sentence about the main image names a render by its name, and by the whole of it only when two renders of the group share the name", () => {
  const plain = summaryImages(
    [result("raw", 1_000_000), result("hoo", 1_000_000)],
    storage(),
  );
  assert.equal(
    mainImageName(fr, "fr", plain[1], plain, pipelineName),
    "Palette HOO",
  );
  const twins = summaryImages(
    [
      result("raw", 1_000_000, {
        path: `${FOLDER}/IC_1805_plan_raw_soft.png`,
        contrast: 2,
      }),
      result("raw", 1_000_000, {
        path: `${FOLDER}/IC_1805_plan_raw_strong.png`,
        contrast: 3.6,
      }),
    ],
    storage(),
  );
  assert.equal(twins.length, 2);
  const both = twins.map((row) =>
    mainImageName(fr, "fr", row, twins, pipelineName),
  );
  assert.notEqual(both[0], both[1]);
  assert.match(both[1], /^Rendu naturel · /);
});
