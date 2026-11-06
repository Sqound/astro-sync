import assert from "node:assert/strict";
import test from "node:test";

import { createTranslator } from "../i18n/locale.ts";
import type { FramingNotch, PlanFraming } from "./bridge";
import {
  along,
  dimPath,
  frameToSend,
  framingView,
  hatchPath,
  percentOf,
  resolveFrame,
  sameFrame,
  summaryFrame,
} from "./planFraming.ts";

const fr = createTranslator("fr");
const en = createTranslator("en");
const ja = createTranslator("ja");
const plain = (text: string) => text.replace(/ /g, " ");

/** The notches measured on the real Seestar night of 10/11 October 2026 cut in
 * two, one half declared dual-band. */
const notch = (
  kind: FramingNotch["kind"],
  top: number,
  left: number,
  height: number,
  width: number,
  share: number,
  degraded: number,
  recommended = false,
): FramingNotch => ({
  top,
  left,
  height,
  width,
  shareOfFrame: share,
  degradedShare: degraded,
  lowestSeconds: degraded === 0 ? 12_900 : 6_450,
  lowestDualBandSeconds: degraded === 0 ? 6_450 : 0,
  kind,
  recommended,
  default: kind === "whole",
});

const REAL: PlanFraming = {
  record: "M_45_plan.json",
  frameHeight: 3840,
  frameWidth: 2160,
  degradedShare: 0.381,
  noDualBandShare: 0.381,
  hasDualBand: true,
  fullSeconds: 12_900,
  fullDualBandSeconds: 6_450,
  recommendedDegradedShare: 0.05,
  fullPolygon: [
    [2156, 1280],
    [2156, 3652],
    [0, 2652],
    [0, 276],
  ],
  notches: [
    notch("cropped", 1278, 8, 1378, 2152, 0.3575, 0),
    notch("mark", 958, 6, 1994, 2154, 0.5178, 0.0479, true),
    notch("between", 639, 4, 2609, 2156, 0.6782, 0.1458),
    notch("between", 320, 2, 3224, 2158, 0.8388, 0.265),
    notch("whole", 0, 0, 3840, 2160, 1, 0.381),
  ],
  offered: true,
  preview: null,
};

test("the frame chosen is the notch whose four integers were kept, else the whole image (the default and the last notch)", () => {
  assert.equal(resolveFrame(REAL, null)?.index, 4);
  const kept = { top: 958, left: 6, height: 1994, width: 2154 };
  const chosen = resolveFrame(REAL, kept);
  assert.equal(chosen?.index, 1);
  assert.equal(chosen?.notch.kind, "mark");
  // A frame that is not a notch of this record (the geometry changed): the default.
  assert.equal(resolveFrame(REAL, { ...kept, top: 959 })?.index, 4);
  assert.ok(sameFrame(kept, REAL.notches[1]));
});

test("nothing is offered before the first image, under one percent, or without a smaller frame", () => {
  const kept = { top: 958, left: 6, height: 1994, width: 2154 };
  assert.equal(resolveFrame(null, kept), null);
  assert.equal(resolveFrame({ ...REAL, offered: false }, kept), null);
  assert.equal(
    resolveFrame({ ...REAL, notches: [REAL.notches[4]] }, kept),
    null,
  );
  assert.equal(frameToSend({ ...REAL, offered: false }, kept), undefined);
  assert.equal(summaryFrame(null, kept), null);
});

test("the job carries the frame unless it is the whole image; the summary says its size only then", () => {
  const kept = { top: 958, left: 6, height: 1994, width: 2154 };
  assert.deepEqual(frameToSend(REAL, kept), kept);
  assert.deepEqual(summaryFrame(REAL, kept), { width: 2154, height: 1994 });
  // The whole image (nothing kept, or the last notch kept): no frame, no line.
  assert.equal(frameToSend(REAL, null), undefined);
  assert.equal(
    frameToSend(REAL, { top: 0, left: 0, height: 3840, width: 2160 }),
    undefined,
  );
  assert.equal(summaryFrame(REAL, null), null);
});

test("each notch says its name, its size and share of the whole image, and its exposure", () => {
  const first = framingView(fr, "fr", REAL, {
    index: 0,
    notch: REAL.notches[0],
  });
  assert.equal(first.name, "Zone commune");
  assert.equal(plain(first.size), "2 152 × 1 378 px · 36 % de l’image entière");
  assert.equal(plain(first.exposure), "3 h 35 partout, dont 1 h 48 en Hα");
  assert.equal(first.note.tone, "muted");
  assert.equal(
    plain(first.note.text),
    "Le plus petit cadre : toutes les nuits couvrent chaque point de l’image.",
  );
  assert.equal(first.showMark, false);
  const mark = framingView(fr, "fr", REAL, {
    index: 1,
    notch: REAL.notches[1],
  });
  assert.equal(mark.name, "Cadre conseillé");
  assert.equal(mark.mark, true);
  assert.equal(plain(mark.size), "2 154 × 1 994 px · 52 % de l’image entière");
  assert.equal(
    plain(mark.exposure),
    "5 % du cadre à 1 h 48 au lieu de 3 h 35, sans Hα",
  );
  assert.equal(mark.note.tone, "ok");
  assert.equal(
    plain(mark.note.text),
    "Repère : le plus grand cadre dont au plus 5 % de l’image sont à pose réduite.",
  );
  const whole = framingView(fr, "fr", REAL, {
    index: 4,
    notch: REAL.notches[4],
  });
  assert.equal(whole.name, "Tout le champ");
  assert.equal(whole.isDefault, true);
  assert.equal(plain(whole.size), "2 160 × 3 840 px");
  assert.equal(
    plain(whole.exposure),
    "38 % du cadre à 1 h 48 au lieu de 3 h 35, sans Hα",
  );
  assert.equal(whole.note.tone, "warn");
  assert.equal(
    plain(whole.note.text),
    "Au-delà du repère, la dégradation augmente : plus de l’image a moins de pose et pas d’Hα.",
  );
  // Beyond the mark, the mark is drawn dotted on the picture.
  assert.equal(whole.showMark, true);
  assert.equal(
    plain(whole.line),
    "Des coins de l’image n’ont pas toutes les nuits : 38 % de l’image n’a pas l’Hα / OIII. Choisissez la taille du cadre que vous gardez, de la zone commune à tout le champ ; vous pouvez la changer plus tard, sans empiler à nouveau.",
  );
});

test("under each notch the share of the image at reduced exposure; names on the first and the last, the mark and the default flagged", () => {
  const view = framingView(fr, "fr", REAL, {
    index: 4,
    notch: REAL.notches[4],
  });
  assert.deepEqual(
    view.stops.map((stop) => plain(stop.percent)),
    ["0 %", "5 %", "15 %", "27 %", "38 %"],
  );
  assert.deepEqual(
    view.stops.map((stop) => stop.name),
    ["Zone commune", null, null, null, "Tout le champ"],
  );
  assert.deepEqual(
    view.stops.map((stop) => [stop.mark, stop.isDefault]),
    [
      [false, false],
      [true, false],
      [false, false],
      [false, false],
      [false, true],
    ],
  );
});

test("a plan without a dual-band integration never mentions the Hα", () => {
  const broadband: PlanFraming = {
    ...REAL,
    hasDualBand: false,
    noDualBandShare: 0,
    notches: REAL.notches.map((item) => ({
      ...item,
      lowestDualBandSeconds: 0,
    })),
  };
  const first = framingView(fr, "fr", broadband, {
    index: 0,
    notch: broadband.notches[0],
  });
  assert.equal(plain(first.exposure), "3 h 35 partout");
  const whole = framingView(fr, "fr", broadband, {
    index: 4,
    notch: broadband.notches[4],
  });
  assert.equal(
    plain(whole.exposure),
    "38 % du cadre à 1 h 48 au lieu de 3 h 35",
  );
  assert.equal(
    plain(whole.note.text),
    "Au-delà du repère, la dégradation augmente : plus de l’image a moins de pose.",
  );
  assert.equal(
    plain(whole.line),
    "Des coins de l’image n’ont pas toutes les nuits : 38 % de l’image n’est couverte que par une partie des nuits. Choisissez la taille du cadre que vous gardez, de la zone commune à tout le champ ; vous pouvez la changer plus tard, sans empiler à nouveau.",
  );
});

test("some Hα left at the lowest exposure is said", () => {
  const some = { ...REAL.notches[4], lowestDualBandSeconds: 3_600 };
  const view = framingView(fr, "fr", REAL, { index: 4, notch: some });
  assert.equal(
    plain(view.exposure),
    "38 % du cadre à 1 h 48 au lieu de 3 h 35, dont 1 h 00 en Hα",
  );
});

test("when the whole image is already inside the mark the slider has two notches and the whole image carries the mark and the default", () => {
  const near: PlanFraming = {
    ...REAL,
    degradedShare: 0.03,
    noDualBandShare: 0.03,
    notches: [
      notch("cropped", 130, 0, 3710, 2160, 0.966, 0),
      notch("whole", 0, 0, 3840, 2160, 1, 0.03, true),
    ],
  };
  const view = framingView(fr, "fr", near, {
    index: 1,
    notch: near.notches[1],
  });
  assert.equal(view.mark, true);
  assert.equal(view.isDefault, true);
  assert.equal(view.note.tone, "ok");
  assert.equal(view.showMark, false);
  assert.deepEqual(
    view.stops.map((stop) => [stop.name, stop.mark, stop.isDefault]),
    [
      ["Zone commune", false, false],
      ["Tout le champ", true, true],
    ],
  );
  assert.equal(
    plain(view.exposure),
    "3 % du cadre à 1 h 48 au lieu de 3 h 35, sans Hα",
  );
});

test("the words are the language's: English and Japanese", () => {
  const whole = { index: 4, notch: REAL.notches[4] };
  const english = framingView(en, "en", REAL, whole);
  assert.equal(english.name, "Whole field");
  assert.equal(
    english.exposure,
    "38% of the frame at 1 h 48 instead of 3 h 35, no Hα",
  );
  assert.equal(english.stops[0].name, "Common area");
  const japanese = framingView(ja, "ja", REAL, whole);
  assert.equal(japanese.name, "視野全体");
  assert.equal(japanese.stops[0].name, "共通範囲");
});

test("a share is a whole percent, at least 1 for what is not nil and at most 99 for what is not all", () => {
  assert.equal(percentOf(0), 0);
  assert.equal(percentOf(0.004), 1);
  assert.equal(percentOf(0.3575), 36);
  assert.equal(percentOf(0.9962), 99);
  assert.equal(percentOf(1), 99);
  assert.equal(percentOf(1, true), 100);
  assert.equal(percentOf(0.0479, true), 5);
});

test("the outline of the picture: the notches sit evenly on the track, the hatch is the picture without the area every night covers, the dim is the picture without the frame", () => {
  assert.deepEqual(
    [0, 1, 2, 3, 4].map((i) => along(i, 5)),
    [0, 0.25, 0.5, 0.75, 1],
  );
  assert.equal(along(0, 1), 0);
  assert.equal(
    hatchPath(REAL),
    "M0 0H2160V3840H0ZM2156 1280L2156 3652L0 2652L0 276Z",
  );
  assert.equal(hatchPath({ ...REAL, fullPolygon: [] }), "");
  assert.equal(
    dimPath(REAL, REAL.notches[1]),
    "M0 0H2160V3840H0ZM6 958H2160V2952H6Z",
  );
});
