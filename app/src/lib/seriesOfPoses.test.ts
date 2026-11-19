import assert from "node:assert/strict";
import test from "node:test";

import { SUPPORTED_LOCALES, createTranslator } from "../i18n/locale.ts";
import { sendOrder, type SummaryImage } from "./detectTreatments.ts";
import {
  defaultUntickedSeries,
  seriesBanner,
  seriesBoxKey,
  seriesKey,
  seriesRowText,
  targetsWithSeries,
  type SeriesOfPoses,
} from "./seriesOfPoses.ts";
import { isSeriesUnticked } from "./wizardSummary.ts";
import type { ImagePlan } from "./bridge";

// One capture per series of poses (CAPTURES-SERIES-1): what « Images à
// envoyer » and the « Récapitulatif » say of a target with two series (M45:
// 3 h 34 of 30 s frames, 6 min of 10 s ones, gain 200), the box of each
// series, the minor one unticked, and the order of the sending: the first
// image of each series is its capture.

const fr = createTranslator("fr");
const flat = (text: string) => text.replaceAll(" ", " ");
const LONG: SeriesOfPoses = {
  filter: "IRCUT",
  exposureSeconds: 30,
  gain: 200,
  binning: 1,
  frames: 429,
  totalSeconds: 12870,
};
const SHORT: SeriesOfPoses = {
  filter: "IRCUT",
  exposureSeconds: 10,
  gain: 200,
  binning: 1,
  frames: 36,
  totalSeconds: 360,
};

test("the key of a series is the service's", () => {
  assert.equal(seriesKey(LONG), "IRCUT|30|200|1");
  assert.equal(
    seriesKey({ ...LONG, filter: "ircut", frames: 430 }),
    seriesKey(LONG),
  );
  assert.equal(seriesKey({ ...SHORT, filter: null, gain: null }), "-|10|-|1");
});

test("a target with two series: the longest is the main one, the short one is minor and unticked", () => {
  const targets = targetsWithSeries([
    { targetId: "m45", series: SHORT },
    { targetId: "m45", series: LONG },
    { targetId: "m45", series: LONG },
    { targetId: "m31", series: LONG },
  ]);
  assert.deepEqual([...targets.keys()], ["m45"]);
  const list = targets.get("m45")!;
  assert.deepEqual(
    list.map((entry) => [entry.key, entry.main, entry.minor]),
    [
      ["IRCUT|30|200|1", true, false],
      ["IRCUT|10|200|1", false, true],
    ],
  );
  assert.deepEqual(
    [...defaultUntickedSeries(targets)],
    [seriesBoxKey("m45", "IRCUT|10|200|1")],
  );
  // Another filter is never minor beside it.
  const ha = targetsWithSeries([
    { targetId: "m45", series: LONG },
    { targetId: "m45", series: { ...SHORT, filter: "Ha" } },
  ]);
  assert.equal(ha.get("m45")![1].minor, false);
});

test("the words of the banner and of a series line", () => {
  const list = targetsWithSeries([
    { targetId: "m45", series: LONG },
    { targetId: "m45", series: SHORT },
  ]).get("m45")!;
  const banner = seriesBanner(fr, "fr", "M45", list);
  assert.equal(flat(banner.title), "M45 · deux séries de poses");
  assert.equal(
    flat(banner.body),
    "IRCUT 30 s × 429 (3 h 34) et IRCUT 10 s × 36 (6 min). Elles ne sont pas assemblées\u00a0: leurs temps de pose diffèrent.".replaceAll(
      "\u00a0",
      " ",
    ),
  );
  assert.equal(
    flat(banner.main),
    "La série de 3 h 34, la plus longue, représente la cible.",
  );
  assert.equal(
    flat(seriesRowText(fr, "fr", LONG)),
    "Série 30 s · gain 200 · 429 poses · 3 h 34",
  );
  assert.equal(
    flat(seriesRowText(fr, "fr", SHORT)),
    "Série 10 s · gain 200 · 36 poses · 6 min",
  );
});

const row = (
  path: string,
  series: SeriesOfPoses | null,
  role: SummaryImage["role"] = "capture",
): SummaryImage => ({
  key: `m45\u0000${path}`,
  targetId: "m45",
  designation: "M45",
  pipelineId: "natural",
  nights: 1,
  path,
  size: 10,
  role,
  blocked: null,
  ...(series ? { series } : {}),
});

test("the sending: the first image of each series is its capture, the others its variants", () => {
  const rows = [
    row("IRCUT_30s_g200_raw.png", LONG),
    row("IRCUT_30s_g200_hoo.png", LONG, "variant"),
    row("IRCUT_10s_g200_raw.png", SHORT, "variant"),
    row("IRCUT_10s_g200_hoo.png", SHORT, "variant"),
  ];
  const order = sendOrder(
    rows,
    rows.map((item) => item.key),
  );
  assert.deepEqual(
    order.map(({ row: item, variantOf }) => [
      item.path,
      variantOf?.path ?? null,
    ]),
    [
      ["IRCUT_30s_g200_raw.png", null],
      ["IRCUT_30s_g200_hoo.png", "IRCUT_30s_g200_raw.png"],
      ["IRCUT_10s_g200_raw.png", null],
      ["IRCUT_10s_g200_hoo.png", "IRCUT_10s_g200_raw.png"],
    ],
  );
  // Without a series known: one capture for the target, as before.
  const before = rows.map((item) => ({ ...item, series: undefined }));
  assert.equal(
    sendOrder(
      before,
      before.map((item) => item.key),
    ).filter((item) => item.variantOf === null).length,
    1,
  );
});

test("a series left unticked in the « Récapitulatif » stays on this computer", () => {
  const plan = (series: SeriesOfPoses | null) =>
    ({
      image: {
        targetId: "m45",
        path: "/n/M45/stacked/x.png",
        sha256: "a".repeat(64),
        size: 10,
        kind: "stacked",
        render: "natural",
        series,
      },
      decision: { decision: "new_capture" },
    }) as unknown as ImagePlan;
  const unticked = new Set([seriesBoxKey("m45", seriesKey(SHORT))]);
  assert.equal(isSeriesUnticked(plan(SHORT), unticked), true);
  assert.equal(isSeriesUnticked(plan(LONG), unticked), false);
  assert.equal(isSeriesUnticked(plan(null), unticked), false);
});

// CAPTURES-SERIES-2 (the screens in real render): in « Images à envoyer »
// each series of a target has its own capture — the first image of the
// 6-minute series read « Variante de la capture de M45 » though it is sent
// as a capture of its own.
test("« Images à envoyer »: the first image of each series is a capture, not a variant", async () => {
  const { summaryImages } = await import("./detectTreatments.ts");
  const render = (series: SeriesOfPoses, id: string) => ({
    targetId: "m45",
    designation: "M45",
    pipelineId: id,
    nights: 1,
    series,
    path: `/n/M45/stacked/IRCUT_${series.exposureSeconds}s_g200_${id}.png`,
    size: 13_000_000,
  });
  const rows = summaryImages(
    [
      render(SHORT, "natural"),
      render(SHORT, "hoo"),
      render(LONG, "natural"),
      render(LONG, "hoo"),
    ],
    null,
  );
  assert.deepEqual(
    rows.map((row) => [row.series?.exposureSeconds, row.role]),
    [
      [30, "capture"],
      [30, "variant"],
      [10, "capture"],
      [10, "variant"],
    ],
  );
});

test("two levels of choice never use the same words: the series stands for the target, the card for its series", () => {
  for (const locale of SUPPORTED_LOCALES) {
    const t = createTranslator(locale);
    const mainImage = t("detect.send.main.label").toLowerCase();
    const series = [
      t("series.banner.main", { duration: "3 h" }),
      t("series.mainBadge"),
    ];
    for (const text of series) {
      // The series says it represents the target; « Image principale » is
      // the choice made on the cards, inside one series.
      assert.ok(!text.toLowerCase().includes(mainImage), `${locale}: ${text}`);
      assert.ok(!/[;\u2014]/.test(text), `${locale}: ${text}`);
    }
  }
});
