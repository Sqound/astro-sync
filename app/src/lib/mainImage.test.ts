import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

import { SUPPORTED_LOCALES } from "../i18n/locale.ts";

import type { ImageToSend, NightOnline, StackedRender } from "./bridge.ts";
import {
  summaryImages,
  type Made,
  type SummaryImage,
} from "./detectTreatments.ts";
import { NO_IMPORT, importReducer, mainPicksOf } from "./importFlow.ts";
import { importSends } from "./importSend.ts";
import {
  KEEP,
  groupKeyOfRow,
  keepChoices,
  mainGroups,
  mainPlan,
  sinceDate,
  type MainContext,
  type OnlineRead,
} from "./mainImage.ts";

// The main image of a capture, chosen on the cards of « Images à envoyer »:
// which image leads by default, what a choice changes, what the space holds
// changes (keep it, take its place, a public capture), and what is told to the
// queue. Pure: no window, no service.

const SESSION = "night-1";
const sha = (name: string) => name.padEnd(64, "0");
const PIPELINES = ["raw", "polished", "hoo", "dualband-correction"] as const;

function night(
  pipelineId: string,
  extra: Partial<Made> = {},
  targetId = "m45",
): Made {
  const render = {
    sha256: sha(`${targetId}-${pipelineId}`),
    path: `/stacked/${targetId}_${pipelineId}.png`,
    size: 12_400_000,
    pipelineId,
  } as unknown as StackedRender;
  const send: ImageToSend = {
    kind: "night",
    sessionId: SESSION,
    targetId,
    render,
  };
  return {
    targetId,
    designation: "M45",
    pipelineId,
    nights: 1,
    path: render.path,
    size: render.size,
    send,
    ...extra,
  };
}

const four = () => PIPELINES.map((id) => night(id));
const keyOf = (rows: readonly SummaryImage[], pipelineId: string) =>
  rows.find((row) => row.pipelineId === pipelineId && !row.fullResolution)!.key;

/** What the space holds for `SESSION`: one capture of M45 with `images`
 * (`[pipelineId, main]`, digests as the fixture makes them). */
function online(
  images: [string, boolean][],
  over: { visibility?: string; series?: object | null; id?: string } = {},
): NightOnline {
  const list = images.map(([pipelineId, main], index) => ({
    id: `img-${index}`,
    main,
    sha256: sha(`m45-${pipelineId}`),
    byteSize: 12_400_000,
    originalName: `${pipelineId}.png`,
    source: "stacking",
    createdAt: "2026-11-12T20:00:00Z",
    skySolution: "missing",
  }));
  return {
    sessionId: SESSION,
    targets: [
      {
        targetId: "m45",
        name: "M 45",
        planned: true,
        captures: [
          {
            id: over.id ?? "cap-1",
            targetIds: ["m45"],
            source: "stacking",
            stage: "draft",
            visibility: over.visibility ?? "private",
            uploaded: true,
            createdAt: "2026-11-12T20:00:00Z",
            mainImage: list.find((image) => image.main) ?? null,
            imageCount: list.length,
            images: list,
            ...(over.series === undefined ? {} : { series: over.series }),
          },
        ],
      },
    ],
    analysis: {
      present: false,
      sentAt: null,
      originDeviceId: null,
      deviceName: null,
      reportVersion: null,
      producer: null,
      stepCount: 0,
      editedStepCount: 0,
    },
  } as unknown as NightOnline;
}

function context(
  made: readonly Made[],
  reads: Record<string, OnlineRead> = {},
  over: Partial<MainContext> = {},
): MainContext {
  const rows = summaryImages(made, null);
  const sessionOf: Record<string, string | null> = {};
  const shaOf: Record<string, string> = {};
  for (const row of rows) {
    const item = made.find((m) => m.path === row.path)!;
    sessionOf[row.key] =
      item.send.kind === "night" ? item.send.sessionId : null;
    shaOf[row.key] =
      item.send.kind === "night" ? item.send.render.sha256 : "plan";
  }
  return {
    sessionOf,
    existing: new Set(Object.keys(reads)),
    reads,
    shaOf,
    variantsAllowed: true,
    ...over,
  };
}

function groupsOf(
  made: readonly Made[],
  ticked: string[] | "all",
  choices: Record<string, string> = {},
  reads: Record<string, OnlineRead> = {},
  over: Partial<MainContext> = {},
) {
  const rows = summaryImages(made, null);
  const keys = ticked === "all" ? rows.map((row) => row.key) : ticked;
  const ctx = context(made, reads, over);
  return { rows, keys, ctx, groups: mainGroups(rows, keys, choices, ctx) };
}

test("by default the image that leads today is the main image: nothing changes unless the person chooses", () => {
  const made = four();
  const { rows, groups } = groupsOf(made, "all");
  assert.equal(groups.length, 1);
  assert.equal(groups[0].cards.length, 4);
  assert.equal(groups[0].selected, keyOf(rows, "raw"));
  assert.equal(groups[0].keepOffered, false);
  const plan = mainPlan(groups);
  assert.deepEqual(plan[groups[0].key], {
    kind: "row",
    key: keyOf(rows, "raw"),
    wish: null,
  });
});

test("the main image is the first one ticked when the natural render is not", () => {
  const made = four();
  const rows = summaryImages(made, null);
  const { groups } = groupsOf(made, [
    keyOf(rows, "hoo"),
    keyOf(rows, "polished"),
  ]);
  assert.equal(groups[0].selected, keyOf(rows, "polished"));
});

test("a choice leads the group: it leaves first, the others are variants of it, whatever their order", () => {
  const made = four();
  const rows = summaryImages(made, null);
  const key = groupKeyOfRow(rows[0]);
  const { groups, keys } = groupsOf(made, "all", { [key]: keyOf(rows, "hoo") });
  assert.equal(groups[0].selected, keyOf(rows, "hoo"));
  const sends = importSends(rows, made, keys, mainPlan(groups));
  assert.deepEqual(
    sends.map((item) => [item.row.pipelineId, item.variant]),
    [
      ["hoo", false],
      ["raw", true],
      ["polished", true],
      ["dualband-correction", true],
    ],
  );
  for (const item of sends.slice(1))
    assert.equal(
      (item.send as { variantOfSha256?: string }).variantOfSha256,
      sha("m45-hoo"),
    );
});

test("the TIFF of the chosen image leaves right after it, the others after that", () => {
  const raw = night("raw");
  const hoo = night("hoo");
  const tiff = night("hoo", {
    path: "/stacked/m45_hoo.tif",
    fullResolution: true,
    twinOf: hoo.path,
    send: { ...(hoo.send as object), fullResolution: true } as ImageToSend,
  });
  const made = [raw, hoo, tiff];
  const rows = summaryImages(made, null);
  const keys = rows.map((row) => row.key);
  const key = groupKeyOfRow(rows[0]);
  const groups = mainGroups(
    rows,
    keys,
    { [key]: keyOf(rows, "hoo") },
    context(made),
  );
  assert.equal(
    groups[0].cards.length,
    2,
    "the TIFF is the second box of its PNG",
  );
  const sends = importSends(rows, made, keys, mainPlan(groups));
  assert.deepEqual(
    sends.map((item) => [item.row.pipelineId, !!item.row.fullResolution]),
    [
      ["hoo", false],
      ["hoo", true],
      ["raw", false],
    ],
  );
});

test("unticking the main image gives the place to the first image still ticked, and the screen shows it", () => {
  const made = four();
  const rows = summaryImages(made, null);
  const key = groupKeyOfRow(rows[0]);
  const choices = { [key]: keyOf(rows, "polished") };
  const ticked = rows
    .map((row) => row.key)
    .filter((k) => k !== keyOf(rows, "polished"));
  // The choice is dropped with the tick...
  assert.deepEqual(keepChoices(rows, ticked, choices), {});
  // ...and even one that were kept falls back to a ticked image.
  const groups = mainGroups(rows, ticked, choices, context(made));
  assert.equal(groups[0].selected, keyOf(rows, "raw"));
  assert.equal(groups[0].blocks[keyOf(rows, "polished")], "notTicked");
  // « Garder » is not an image: unticking never drops it.
  assert.deepEqual(keepChoices(rows, ticked, { [key]: KEEP }), { [key]: KEEP });
});

test("the reducer: a choice is stored, and unticking the chosen image drops it", () => {
  const made = four();
  const rows = summaryImages(made, null);
  const key = groupKeyOfRow(rows[0]);
  let state = { ...NO_IMPORT, made, ticked: rows.map((row) => row.key) };
  state = importReducer(state, {
    type: "chooseMain",
    group: key,
    value: keyOf(rows, "hoo"),
  });
  assert.equal(state.mainChoice[key], keyOf(rows, "hoo"));
  state = importReducer(state, {
    type: "toggleImage",
    key: keyOf(rows, "hoo"),
  });
  assert.deepEqual(state.mainChoice, {});
  assert.equal(mainPicksOf(state)[key]?.kind, "row");
  assert.equal(
    (mainPicksOf(state)[key] as { key: string }).key,
    keyOf(rows, "raw"),
  );
});

test("one image alone is the main image: a sentence, no choice", () => {
  const made = [night("raw")];
  const { groups } = groupsOf(made, "all");
  assert.equal(groups[0].cards.length, 1);
  assert.equal(groups[0].keepOffered, false);
  assert.equal(groups[0].locked, false);
});

test("a capture already online and private: the main image of the space is kept by default, the images leave as variants of it", () => {
  const made = four();
  const reads = { [SESSION]: online([["raw", true]]) };
  const { rows, groups, keys } = groupsOf(made, "all", {}, reads);
  const group = groups[0];
  assert.equal(group.keepOffered, true);
  assert.equal(group.selected, KEEP);
  assert.equal(
    group.online.kind === "capture" && group.online.main?.name,
    "raw.png",
  );
  const plan = mainPlan(groups);
  assert.deepEqual(plan[group.key], { kind: "capture", captureId: "cap-1" });
  const sends = importSends(rows, made, keys, plan);
  for (const item of sends) {
    assert.equal(
      (item.send as { variantOfCapture?: string }).variantOfCapture,
      "cap-1",
    );
    assert.equal(item.variant, true);
    assert.equal(
      (item.send as { variantOfSha256?: string }).variantOfSha256,
      undefined,
    );
  }
});

test("the deliberate gesture: another image is ticked as the main one, the screen says it, the head asks the space to confirm", () => {
  const made = four();
  const reads = { [SESSION]: online([["raw", true]]) };
  const rows = summaryImages(made, null);
  const key = groupKeyOfRow(rows[0]);
  const { groups, keys } = groupsOf(
    made,
    "all",
    { [key]: keyOf(rows, "polished") },
    reads,
  );
  assert.equal(groups[0].selected, keyOf(rows, "polished"));
  const plan = mainPlan(groups, { [key]: keyOf(rows, "polished") });
  assert.deepEqual(plan[key], {
    kind: "row",
    key: keyOf(rows, "polished"),
    wish: "over",
  });
  const sends = importSends(rows, made, keys, plan);
  assert.equal(sends[0].row.pipelineId, "polished");
  assert.equal((sends[0].send as { mainWish?: string }).mainWish, "over");
  assert.equal(sends[0].variant, false);
  assert.equal(
    (sends[1].send as { variantOfSha256?: string }).variantOfSha256,
    sha("m45-polished"),
  );
});

test("a public capture: the choice is closed and every image is a variant of it", () => {
  const made = four();
  const reads = {
    [SESSION]: online([["raw", true]], { visibility: "public" }),
  };
  const rows = summaryImages(made, null);
  const key = groupKeyOfRow(rows[0]);
  // Whatever was chosen before, nothing changes the main image of a public capture.
  const { groups, keys } = groupsOf(
    made,
    "all",
    { [key]: keyOf(rows, "hoo") },
    reads,
  );
  assert.equal(groups[0].locked, true);
  assert.equal(groups[0].keepOffered, false);
  assert.equal(groups[0].selected, null);
  const sends = importSends(rows, made, keys, mainPlan(groups));
  assert.ok(
    sends.every(
      (item) =>
        (item.send as { variantOfCapture?: string }).variantOfCapture ===
        "cap-1",
    ),
  );
});

test("bytes already online: as a variant the choice is impossible from Astro Sync, as the main image there is nothing to take", () => {
  const made = four();
  const reads = {
    [SESSION]: online([
      ["raw", true],
      ["polished", false],
    ]),
  };
  const { rows, groups } = groupsOf(made, "all", {}, reads);
  assert.equal(groups[0].blocks[keyOf(rows, "polished")], "onlineVariant");
  assert.equal(groups[0].blocks[keyOf(rows, "raw")], "onlineMain");
  assert.equal(groups[0].blocks[keyOf(rows, "hoo")], undefined);
  // A choice of such an image falls back to « Garder ».
  const key = groupKeyOfRow(rows[0]);
  const again = mainGroups(
    rows,
    rows.map((row) => row.key),
    { [key]: keyOf(rows, "polished") },
    context(made, reads),
  );
  assert.equal(again[0].selected, KEEP);
});

test("the space cannot be read (offline, or being read): today's behaviour, and the decision is the space's, when the images leave", () => {
  const made = four();
  const rows = summaryImages(made, null);
  const key = groupKeyOfRow(rows[0]);
  for (const reads of [{ [SESSION]: "failed" as const }]) {
    const { groups } = groupsOf(made, "all", {}, reads);
    assert.equal(groups[0].online.kind, "unread");
    assert.equal(groups[0].selected, keyOf(rows, "raw"));
    assert.equal(groups[0].keepOffered, false);
    // Nothing was chosen: the main image of the space is kept at send time.
    assert.deepEqual(mainPlan(groups)[key], {
      kind: "row",
      key: keyOf(rows, "raw"),
      wish: "keep",
    });
    // A choice made: it takes the place only if the space confirms.
    const chosen = groupsOf(made, "all", { [key]: keyOf(rows, "hoo") }, reads);
    assert.deepEqual(
      mainPlan(chosen.groups, { [key]: keyOf(rows, "hoo") })[key],
      {
        kind: "row",
        key: keyOf(rows, "hoo"),
        wish: "over",
      },
    );
  }
  // Being read: the same, without a line saying it could not.
  const pending = context(made, {}, { existing: new Set([SESSION]) });
  const groups = mainGroups(
    rows,
    rows.map((row) => row.key),
    {},
    pending,
  );
  assert.equal(groups[0].online.kind, "pending");
  assert.equal(mainPlan(groups)[key]?.kind, "row");
});

test("a night the import creates holds nothing: no line, no wish", () => {
  const made = four();
  const rows = summaryImages(made, null);
  const key = groupKeyOfRow(rows[0]);
  const ctx = context(made, {}, { existing: new Set() });
  const groups = mainGroups(
    rows,
    rows.map((row) => row.key),
    {},
    ctx,
  );
  assert.equal(groups[0].online.kind, "none");
  assert.equal((mainPlan(groups)[key] as { wish: unknown }).wish, null);
});

test("a capture is the one of its series: another series online is no capture of this one", () => {
  const withSeries = (exposureSeconds: number): Made[] =>
    PIPELINES.map((id) =>
      night(id, {
        series: {
          filter: "IRCUT",
          totalSeconds: 429 * 30,
          exposureSeconds,
          gain: 200,
          binning: 1,
          frames: 429,
        },
      }),
    );
  const held = (exposureSeconds: number) =>
    online([["raw", true]], {
      series: { filter: "IRCUT", exposureSeconds, gain: 200, binning: 1 },
    });
  const same = groupsOf(withSeries(30), "all", {}, { [SESSION]: held(30) });
  assert.equal(same.groups[0].keepOffered, true);
  const other = groupsOf(withSeries(10), "all", {}, { [SESSION]: held(30) });
  assert.equal(other.groups[0].online.kind, "none");
  // A capture without a series may take this one: the service decides by the
  // name of its image, so the screen does not say.
  const unnamed = groupsOf(
    withSeries(30),
    "all",
    {},
    { [SESSION]: online([["raw", true]]) },
  );
  assert.equal(unnamed.groups[0].online.kind, "unread");
});

test("a space that keeps no variant cannot keep the main image beside new ones: today's behaviour", () => {
  const made = four();
  const reads = { [SESSION]: online([["raw", true]]) };
  const { groups } = groupsOf(made, "all", {}, reads, {
    variantsAllowed: false,
  });
  assert.equal(groups[0].keepOffered, false);
  assert.equal(groups[0].selected !== KEEP, true);
  assert.equal(
    (mainPlan(groups)[groups[0].key] as { wish: unknown }).wish,
    null,
  );
});

test("the linear file is attached to the capture a series is kept in", () => {
  const raw = night("raw");
  const linear = night("none", { file: "linear" });
  const made = [raw, linear];
  const reads = { [SESSION]: online([["raw", true]]) };
  const rows = summaryImages(made, null);
  const ctx = context(made, reads);
  const keys = rows.map((row) => row.key);
  const groups = mainGroups(rows, keys, {}, ctx);
  const sends = importSends(rows, made, keys, mainPlan(groups));
  assert.equal(sends.length, 2);
  const file = sends.find((item) => item.row.role === "file")!;
  assert.equal(
    (file.send as { variantOfCapture?: string }).variantOfCapture,
    "cap-1",
  );
  assert.equal(file.variant, false);
});

test("each target and each series has its own group and its own choice", () => {
  const made = [
    ...PIPELINES.map((id) => night(id)),
    ...PIPELINES.map((id) => night(id, {}, "ic1805")),
  ];
  const rows = summaryImages(made, null);
  const groups = mainGroups(
    rows,
    rows.map((row) => row.key),
    {},
    context(made),
  );
  assert.equal(groups.length, 2);
  assert.notEqual(groups[0].key, groups[1].key);
});

test("the date the main image is online since is written as the window writes dates", () => {
  const now = new Date("2026-11-18T10:00:00Z");
  assert.equal(sinceDate("fr", "2026-11-12T20:00:00Z", now), "12 nov.");
  assert.equal(sinceDate("en", "2026-11-12T20:00:00Z", now), "Nov 12");
  assert.equal(sinceDate("fr", "2025-03-02T20:00:00Z", now), "2 mars 2025");
  assert.equal(sinceDate("fr", "not a date", now), "");
});

const WORDS = [
  "label",
  "groupTitle.one",
  "groupTitle.other",
  "current",
  "only",
  "keep",
  "keepSince",
  "swap",
  "public",
  "unread",
  "onlineVariant",
  "onlineMain",
  "notTicked",
].map((key) => `detect.send.main.${key}`);

test("the words of the choice and the two sentences of the subtitle are in the ten languages, with no semicolon and no long dash", async () => {
  const portal: Record<string, string> = {
    fr: "image principale",
    en: "Main image",
    de: "Hauptbild",
    es: "imagen principal",
    it: "immagine principale",
    ja: "メイン画像",
    "pt-BR": "imagem principal",
    ru: "основное изображение",
    "zh-Hans": "主图",
    "zh-Hant": "主圖",
  };
  assert.equal(SUPPORTED_LOCALES.length, 10);
  for (const locale of SUPPORTED_LOCALES) {
    const dictionary = JSON.parse(
      await readFile(
        new URL(`../i18n/${locale}.json`, import.meta.url),
        "utf8",
      ),
    ) as Record<string, string>;
    for (const key of [...WORDS, "detect.send.room", "detect.send.noRoom"]) {
      const text = dictionary[key];
      assert.ok(text && text.trim().length > 0, `${locale} ${key}`);
      assert.ok(!/[;；\u2014]/u.test(text), `${locale} ${key}: ${text}`);
    }
    // The word of the portal, in the label of the choice.
    assert.equal(
      dictionary["detect.send.main.label"].toLowerCase(),
      portal[locale].toLowerCase(),
      locale,
    );
    // The blanks of a sentence are the same in every language.
    assert.match(
      dictionary["detect.send.main.groupTitle.other"],
      /\{target\}.*\{count\}/u,
    );
    assert.match(
      dictionary["detect.send.main.keepSince"],
      /\{render\}.*\{date\}/u,
    );
    assert.match(dictionary["detect.send.main.swap"], /\{old\}.*\{new\}/u);
    assert.match(dictionary["detect.send.main.current"], /\{render\}/u);
  }
});
