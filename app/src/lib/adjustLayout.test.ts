import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

import { NARROW_REM, PANEL_TWO_COLUMNS_REM, fitSize } from "./adjustLayout.ts";

test("a portrait image in a wide box is fitted to the height, never cropped", () => {
  const size = fitSize(1200, 800, 1080, 1920);
  assert.equal(size.height, 800);
  assert.ok(size.width < 1200);
  assert.ok(Math.abs(size.width / size.height - 1080 / 1920) < 0.01);
});

test("a landscape image in a tall box is fitted to the width", () => {
  const size = fitSize(600, 900, 1080, 720);
  assert.equal(size.width, 600);
  assert.equal(size.height, 400);
});

test("the fitted image is always inside the box", () => {
  for (const [bw, bh, w, h] of [
    [500, 500, 1080, 1080],
    [333, 777, 1080, 604],
    [1500, 200, 720, 1080],
  ]) {
    const size = fitSize(bw, bh, w, h);
    assert.ok(size.width <= bw && size.height <= bh);
  }
});

test("a box or an image without size gives no size", () => {
  assert.deepEqual(fitSize(0, 800, 10, 10), { width: 0, height: 0 });
  assert.deepEqual(fitSize(800, 600, 0, 10), { width: 0, height: 0 });
});

test("the layout thresholds are those of the validated board", () => {
  assert.equal(NARROW_REM, 48);
  assert.equal(PANEL_TWO_COLUMNS_REM, 30);
});

test("the texts of the editor join two ideas with a full stop, never a semicolon or a dash", () => {
  const keys = [
    "adjust.previewNote",
    "adjust.brightness.help",
    "adjust.smoothing.help",
    "adjust.foot.kept",
    "adjust.saved.listHelp",
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
    for (const key of keys) {
      assert.ok(!/[;；—]/.test(dict[key]), `${locale}: ${key}`);
    }
  }
});
