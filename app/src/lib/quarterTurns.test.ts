import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

import {
  cleanTurns,
  turnBy,
  turnMatrix,
  turnPixels,
  turnStateKey,
  turnedSize,
  type Turns,
} from "./quarterTurns.ts";
import { NEUTRAL, adjustedLine, adjustedList } from "./imageAdjust.ts";

test("turns are brought into 0..3, a strange value is 0", () => {
  assert.equal(cleanTurns(4), 0);
  assert.equal(cleanTurns(-1), 3);
  assert.equal(cleanTurns(5), 1);
  assert.equal(cleanTurns("1"), 0);
  assert.equal(cleanTurns(Number.NaN), 0);
  assert.equal(cleanTurns(undefined), 0);
});

test("right adds a quarter turn clockwise, left takes one away, four make none", () => {
  assert.equal(turnBy(0, "right"), 1);
  assert.equal(turnBy(0, "left"), 3);
  assert.equal(turnBy(3, "right"), 0);
  let turns: Turns = 0;
  for (let i = 0; i < 4; i++) turns = turnBy(turns, "right");
  assert.equal(turns, 0);
});

test("the sides swap on an odd count only", () => {
  assert.deepEqual(turnedSize(0, 300, 200), { width: 300, height: 200 });
  assert.deepEqual(turnedSize(1, 300, 200), { width: 200, height: 300 });
  assert.deepEqual(turnedSize(2, 300, 200), { width: 300, height: 200 });
  assert.deepEqual(turnedSize(3, 300, 200), { width: 200, height: 300 });
});

test("the drawing matrix sends the corners where a clockwise turn puts them", () => {
  const land = (turns: Turns, x: number, y: number, w: number, h: number) => {
    const [a, b, c, d, e, f] = turnMatrix(turns, w, h);
    return [a * x + c * y + e, b * x + d * y + f];
  };
  // A 4 × 2 picture: its top-left corner (0, 0).
  assert.deepEqual(land(1, 0, 0, 4, 2), [2, 0]); // top-right of the 2 × 4 picture
  assert.deepEqual(land(1, 4, 0, 4, 2), [2, 4]);
  assert.deepEqual(land(2, 0, 0, 4, 2), [4, 2]);
  assert.deepEqual(land(3, 0, 0, 4, 2), [0, 4]);
  assert.deepEqual(land(3, 4, 0, 4, 2), [0, 0]);
});

test("the pixels land where the script's numpy.rot90 clockwise puts them", () => {
  // 3 columns × 2 rows, each pixel its own red level.
  const data = new Uint8ClampedArray(3 * 2 * 4);
  for (let i = 0; i < 6; i++) data[i * 4] = i + 1;
  const red = (array: Uint8ClampedArray) =>
    [...array].filter((_, index) => index % 4 === 0);
  // Rows: [1 2 3] / [4 5 6]. Turned right (2 columns × 3 rows): [4 1] / [5 2] / [6 3].
  assert.deepEqual(red(turnPixels(data, 3, 2, 1)), [4, 1, 5, 2, 6, 3]);
  // Half a turn: [6 5 4] / [3 2 1].
  assert.deepEqual(red(turnPixels(data, 3, 2, 2)), [6, 5, 4, 3, 2, 1]);
  // Turned left: [3 6] / [2 5] / [1 4].
  assert.deepEqual(red(turnPixels(data, 3, 2, 3)), [3, 6, 2, 5, 1, 4]);
  assert.equal(turnPixels(data, 3, 2, 0), data);
});

test("the state of a turned image has its words, none when not turned", () => {
  assert.equal(turnStateKey(0), null);
  assert.equal(turnStateKey(1), "adjust.rotate.state.right");
  assert.equal(turnStateKey(2), "adjust.rotate.state.half");
  assert.equal(turnStateKey(3), "adjust.rotate.state.left");
});

test("the words of the turns are in the ten languages and match the portal's", () => {
  const keys = [
    "adjust.rotate.left",
    "adjust.rotate.right",
    "adjust.rotate.reset",
    "adjust.rotate.state.right",
    "adjust.rotate.state.half",
    "adjust.rotate.state.left",
  ];
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
    const dict = JSON.parse(
      readFileSync(new URL(`../i18n/${locale}.json`, import.meta.url), "utf8"),
    ) as Record<string, string>;
    for (const key of keys) assert.ok(dict[key], `${locale}: ${key}`);
  }
  const fr = JSON.parse(
    readFileSync(new URL("../i18n/fr.json", import.meta.url), "utf8"),
  ) as Record<string, string>;
  assert.equal(fr["adjust.rotate.left"], "Pivoter à gauche");
  assert.equal(fr["adjust.rotate.right"], "Pivoter à droite");
  assert.equal(fr["adjust.rotate.reset"], "Revenir à l’orientation d’origine");
});

test("the line of a turned image set says the turn, even when no setting moved", () => {
  const fr = JSON.parse(
    readFileSync(new URL("../i18n/fr.json", import.meta.url), "utf8"),
  ) as Record<string, string>;
  const t = (key: string, params: Record<string, string | number> = {}) =>
    (fr[key] ?? key).replace(/\{(\w+)\}/g, (_, name) => String(params[name]));
  assert.equal(
    adjustedLine(t, "fr", NEUTRAL, 1),
    "Réglée\u00a0: tournée de 90° à droite",
  );
  assert.equal(
    adjustedList(t, "fr", { ...NEUTRAL, blackPoint: 7 }, 3),
    "tournée de 90° à gauche, point noir 7 %",
  );
  // Not turned: the line is the settings', as before.
  assert.equal(adjustedList(t, "fr", NEUTRAL), "Comme le rendu");
  assert.equal(adjustedList(t, "fr", NEUTRAL, 4), "Comme le rendu");
});

test("the screen turns with the portal's two buttons, writes the turn with the image, and lets a turn alone be saved", () => {
  const screen = readFileSync(
    new URL("../screens/adjust/AdjustScreen.tsx", import.meta.url),
    "utf8",
  );
  assert.match(screen, /adjust\.rotate\.left/);
  assert.match(screen, /adjust\.rotate\.right/);
  assert.match(screen, /adjust\.rotate\.reset/);
  assert.match(screen, /imageAdjustSave\(path, values, turns\)/);
  // « Rien n'a changé » only when no setting moved AND the image is not turned.
  assert.match(screen, /\(neutral && turns === 0\)/);
  // The image set already saved reopens with its turn.
  assert.match(screen, /cleanTurns\(answer\.state\.quarterTurns\)/);
});

test("the saved settings carry five values, never a turn", () => {
  const bridge = readFileSync(new URL("./bridge.ts", import.meta.url), "utf8");
  const start = bridge.indexOf("export interface SavedAdjustment");
  const body = bridge.slice(start, bridge.indexOf("}", start));
  assert.doesNotMatch(body, /quarterTurns|turns/i);
  assert.match(
    bridge,
    /imageAdjustSavedSave\(\s*name: string,\s*values: AdjustValues,/,
  );
});
