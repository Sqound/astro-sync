import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

import {
  COLOUR_SLIDER,
  COLOUR_STOPS,
  DEFAULT_COLOUR,
  DEFAULT_SOFT_STARS,
  SOFT_STARS_SLIDER,
  SOFT_STARS_STOPS,
  DEFAULT_FINISH,
  applyFinishesToAll,
  changedFinishes,
  colourOf,
  colourSnap,
  finishLine,
  finishOf,
  finishSummaries,
  hasFinish,
  loadTargetFinishes,
  pipelinesToRun,
  saveTargetFinishes,
  softStarsOf,
  softStarsSnap,
  withTargetFinishes,
  type PipelineFinish,
} from "./finish.ts";
import { createTranslator } from "../i18n/locale.ts";
import { SUPPORTED_LOCALES } from "../i18n/locale.ts";

test("the colour slider goes from 0 to +100 % with detents at +20 % and +40 %, the stars' from 0 to 100 % with 50 % and 85 %, and both start at nothing", () => {
  assert.deepEqual(
    COLOUR_STOPS.map((stop) => stop.value),
    [0, 20, 40],
  );
  assert.deepEqual(
    SOFT_STARS_STOPS.map((stop) => stop.value),
    [0, 50, 85],
  );
  for (const slider of [COLOUR_SLIDER, SOFT_STARS_SLIDER]) {
    assert.equal(slider.min, 0);
    assert.ok(slider.max >= 60, "up to +60 % at least, as asked");
  }
  assert.equal(DEFAULT_COLOUR, 0);
  assert.equal(DEFAULT_SOFT_STARS, 0);
});

test("the magnets: near a detent the slider holds on it, elsewhere any whole percent is kept", () => {
  assert.equal(colourSnap(22), 20);
  assert.equal(colourSnap(38), 40);
  assert.equal(colourSnap(30), 30);
  assert.equal(colourSnap(1), 0);
  assert.equal(colourSnap(150), 100);
  assert.equal(softStarsSnap(83), 85);
  assert.equal(softStarsSnap(70), 70);
  assert.equal(softStarsSnap(100), 100);
  assert.equal(softStarsSnap(-5), 0);
});

test("a record or a settings file is read as a percent: numbers, the words of the first version, or nothing", () => {
  assert.equal(colourOf(35), 35);
  assert.equal(colourOf("40"), 40);
  assert.equal(colourOf("enhanced"), 20);
  assert.equal(colourOf("strong"), 40);
  assert.equal(colourOf("vivid"), 0);
  assert.equal(colourOf(undefined), 0);
  assert.equal(colourOf(180), 100);
  assert.equal(softStarsOf("light"), 50);
  assert.equal(softStarsOf("strong"), 85);
  assert.equal(softStarsOf(70.4), 70);
  assert.equal(softStarsOf(undefined), 0);
});

test("an image that carries a finish is a variant, and says it in words", () => {
  const t = (key: string, params?: Record<string, string | number>) =>
    params ? `${key}:${params.value}` : key;
  assert.equal(hasFinish({}), false);
  assert.equal(hasFinish({ colour: 0, softStars: "none" }), false);
  assert.equal(hasFinish({ colour: 20 }), true);
  assert.equal(hasFinish({ softStars: 70 }), true);
  assert.deepEqual(finishSummaries({}, t), []);
  assert.deepEqual(finishSummaries({ colour: 40, softStars: 50 }, t), [
    "wizard.treatments.colour.summary.strong",
    "wizard.treatments.softStars.summary.light",
  ]);
  assert.deepEqual(finishSummaries({ colour: 33, softStars: 70 }, t), [
    "wizard.treatments.colour.summary.custom:33",
    "wizard.treatments.softStars.summary.custom:70",
  ]);
});

test("each detent has its label in the ten languages, the value in it, and the images say it", async () => {
  assert.equal(SUPPORTED_LOCALES.length, 10);
  for (const locale of SUPPORTED_LOCALES) {
    const dictionary = JSON.parse(
      await readFile(
        new URL(`../i18n/${locale}.json`, import.meta.url),
        "utf8",
      ),
    ) as Record<string, string>;
    const colours = COLOUR_STOPS.map((s) => dictionary[s.labelKey]);
    const stars = SOFT_STARS_STOPS.map((s) => dictionary[s.labelKey]);
    for (const labels of [colours, stars]) {
      assert.ok(labels.every(Boolean), `${locale}: a label is missing`);
      assert.equal(new Set(labels).size, 3, `${locale}: the labels differ`);
    }
    assert.match(colours[1], /20/, `${locale}: ${colours[1]}`);
    assert.match(colours[2], /40/, `${locale}: ${colours[2]}`);
    assert.match(stars[1], /50/, `${locale}: ${stars[1]}`);
    assert.match(stars[2], /85/, `${locale}: ${stars[2]}`);
    const summaries = [
      ...COLOUR_STOPS.map((s) => s.summaryKey),
      ...SOFT_STARS_STOPS.map((s) => s.summaryKey),
    ].filter((key): key is string => key !== null);
    assert.equal(summaries.length, 4);
    for (const key of summaries) {
      assert.ok(dictionary[key], `${locale}: ${key}`);
    }
    for (const key of [
      "wizard.treatments.colour.title",
      "wizard.finish.colour.help",
      "wizard.treatments.colour.custom",
      "wizard.treatments.colour.summary.custom",
      "wizard.treatments.softStars.title",
      "wizard.finish.stars.help",
      "wizard.treatments.softStars.custom",
      "wizard.treatments.softStars.summary.custom",
      "wizard.treatments.finish.note",
    ]) {
      assert.ok(dictionary[key], `${locale}: ${key}`);
    }
    for (const key of [
      "wizard.treatments.colour.custom",
      "wizard.treatments.colour.summary.custom",
      "wizard.treatments.softStars.custom",
      "wizard.treatments.softStars.summary.custom",
    ]) {
      assert.match(dictionary[key], /\{value\}/, `${locale}: ${key}`);
    }
  }
});

test("the sliders are wired: a continuous range each, kept in the settings, under the contrast's, in both places", async () => {
  const read = (path: string) =>
    readFile(new URL(path, import.meta.url), "utf8");
  const sliders = await read("../components/FinishSliders.tsx");
  assert.match(sliders, /spec=\{COLOUR_SLIDER\}/);
  assert.match(sliders, /spec=\{SOFT_STARS_SLIDER\}/);
  const step = await read("../screens/night/TreatmentsStep.tsx");
  assert.equal((step.match(/<FinishSliders\b/g) ?? []).length, 1);
  assert.match(step, /localSettingsSetColour/);
  assert.match(step, /localSettingsSetSoftStars/);
  const extras = await read("../components/RenderExtras.tsx");
  assert.match(extras, /\{slider && \(\s*<FinishSliders/);
});

/* ------------------- The finish of each pipeline -------------------------- */

const fr = createTranslator("fr");
const STRONG: PipelineFinish = { contrast: 3.6, colour: 20, softStars: 50 };
const NONE: PipelineFinish = { contrast: 0, colour: 0, softStars: 0 };

test("the folded line names each of the three settings, at zero too", () => {
  assert.equal(
    finishLine(DEFAULT_FINISH, fr, "fr"),
    "Présentation · Couleurs naturelles · Étoiles intactes",
  );
  assert.equal(
    finishLine(STRONG, fr, "fr"),
    "Fort · Couleurs +20 % · Étoiles 50 %",
  );
  assert.equal(
    finishLine(NONE, fr, "fr"),
    "Sans contraste · Couleurs naturelles · Étoiles intactes",
  );
  assert.equal(
    finishLine({ contrast: 2.4, colour: 30, softStars: 85 }, fr, "fr"),
    "Contraste 2,4 · Couleurs +30 % · Étoiles 85 %",
  );
});

test("a pipeline follows the default until it has its own, which stays when the default moves", () => {
  const finishes = { M31: { hoo: STRONG } };
  assert.deepEqual(finishOf(finishes, "M31", "raw", DEFAULT_FINISH), {
    finish: DEFAULT_FINISH,
    own: false,
  });
  assert.deepEqual(finishOf(finishes, "M31", "hoo", NONE), {
    finish: STRONG,
    own: true,
  });
  assert.deepEqual(finishOf(undefined, "M31", "hoo", NONE), {
    finish: NONE,
    own: false,
  });
  // The lines of the card: the pipelines chosen whose finish differs, « Sans
  // traitement » never.
  assert.deepEqual(
    changedFinishes(
      { M31: { hoo: STRONG, raw: DEFAULT_FINISH, linear: STRONG } },
      "M31",
      ["raw", "hoo", "linear"],
      DEFAULT_FINISH,
      "linear",
    ),
    [{ pipelineId: "hoo", finish: STRONG }],
  );
});

test("the stacking is asked each pipeline with its own finish, and the others with nothing (the default)", () => {
  assert.deepEqual(
    pipelinesToRun(
      ["raw", "hoo", "linear"],
      { M31: { hoo: STRONG, linear: STRONG } },
      "M31",
      "linear",
    ),
    [
      { pipelineId: "raw", trustedSha256: null },
      {
        pipelineId: "hoo",
        trustedSha256: null,
        finish: { contrast: "strong", colour: 20, softStars: 50 },
      },
      { pipelineId: "linear", trustedSha256: null },
    ],
  );
  assert.deepEqual(
    pipelinesToRun(["raw"], { M31: { raw: NONE } }, "M31", "linear")[0].finish,
    { contrast: "none", colour: 0, softStars: 0 },
  );
});

test("« Valider » keeps the finishes of the pipelines ticked; « Appliquer à toutes les cibles » copies them by pipeline id, the default included", () => {
  const before = { IC: { raw: NONE }, NGC: { hoo: NONE } };
  const validated = withTargetFinishes(
    before,
    "M31",
    { hoo: STRONG, polished: NONE, linear: STRONG },
    ["raw", "hoo", "linear"],
    "linear",
  );
  assert.deepEqual(validated.M31, { hoo: STRONG });
  assert.deepEqual(validated.IC, { raw: NONE });
  // Everything back to the default: the target is no longer named.
  assert.equal(
    "M31" in withTargetFinishes(validated, "M31", {}, ["raw"], "linear"),
    false,
  );
  const copied = applyFinishesToAll(
    before,
    ["M31", "NGC"],
    { hoo: STRONG },
    ["raw", "hoo"],
    "linear",
  );
  assert.deepEqual(copied.M31, { hoo: STRONG });
  // NGC had its own finish of the HOO palette: it takes the one copied.
  assert.deepEqual(copied.NGC, { hoo: STRONG });
  // IC was not named: left as it was.
  assert.deepEqual(copied.IC, { raw: NONE });
  // The source follows the default: the targets follow it too.
  assert.equal(
    "NGC" in applyFinishesToAll(before, ["NGC"], {}, ["hoo"], "linear"),
    false,
  );
  assert.deepEqual(
    before.NGC,
    { hoo: NONE },
    "the finishes given are never changed",
  );
});

test("the finishes are kept on this computer beside the pipelines: closed and opened the next day, they are there", () => {
  const held = new Map<string, string>();
  const storage = {
    getItem: (key: string) => held.get(key) ?? null,
    setItem: (key: string, value: string) => void held.set(key, value),
  };
  saveTargetFinishes(storage, "night-1", { M31: { hoo: STRONG } });
  assert.ok(held.has("astro-sync.finishes:night-1"));
  assert.deepEqual(loadTargetFinishes(storage, "night-1"), {
    M31: { hoo: STRONG },
  });
  assert.deepEqual(loadTargetFinishes(storage, "night-2"), {});
  // What cannot be read follows the default; words of the core are read.
  held.set("astro-sync.finishes:night-3", "not json");
  assert.deepEqual(loadTargetFinishes(storage, "night-3"), {});
  held.set(
    "astro-sync.finishes:night-4",
    JSON.stringify({
      M31: { hoo: { contrast: "strong", colour: "enhanced", softStars: 900 } },
      IC: "nothing",
    }),
  );
  assert.deepEqual(loadTargetFinishes(storage, "night-4"), {
    M31: { hoo: { contrast: 3.6, colour: 20, softStars: 100 } },
  });
  assert.deepEqual(loadTargetFinishes(null, "night-1"), {});
});

test("the words of « Finition » are in the ten languages, with their blanks", async () => {
  const keys = [
    "wizard.finish.label",
    "wizard.finish.default",
    "wizard.finish.modified",
    "wizard.finish.edit",
    "wizard.finish.fold",
    "wizard.finish.reset",
    "wizard.finish.contrast.none",
    "wizard.finish.contrast.presentation",
    "wizard.finish.contrast.strong",
    "wizard.finish.colour.none",
    "wizard.finish.stars.none",
    "wizard.finish.contrast.help",
    "wizard.finish.colour.help",
    "wizard.finish.colour.helpHoo",
    "wizard.finish.stars.help",
    "wizard.finish.note",
    "wizard.finish.defaultTitle",
    "wizard.finish.defaultLead",
    "wizard.finish.targetModified",
  ];
  const blanks: Record<string, string[]> = {
    "wizard.finish.contrast.custom": ["{value}"],
    "wizard.finish.colour.value": ["{value}"],
    "wizard.finish.stars.value": ["{value}"],
    "wizard.finish.targetLine": ["{pipeline}", "{finish}"],
  };
  for (const locale of SUPPORTED_LOCALES) {
    const dictionary = JSON.parse(
      await readFile(
        new URL(`../i18n/${locale}.json`, import.meta.url),
        "utf8",
      ),
    ) as Record<string, string>;
    for (const key of keys) assert.ok(dictionary[key], `${locale}: ${key}`);
    for (const [key, wanted] of Object.entries(blanks)) {
      for (const blank of wanted) {
        assert.ok(dictionary[key]?.includes(blank), `${locale}: ${key}`);
      }
    }
    assert.notEqual(
      dictionary["wizard.finish.default"],
      dictionary["wizard.finish.modified"],
      locale,
    );
  }
  const french = JSON.parse(
    await readFile(new URL("../i18n/fr.json", import.meta.url), "utf8"),
  ) as Record<string, string>;
  // « Appliquer à toutes les cibles » says it copies the finish too.
  assert.match(french["wizard.pipelines.applyAllHelp"], /finition/);
  // The default no longer says « the same setting for every target ».
  assert.doesNotMatch(french["wizard.finish.defaultLead"], /même réglage/);
});
