import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

import {
  CURTAIN_START,
  clampCurtain,
  curtainFromKey,
  curtainFromPointer,
  curtainToggle,
  labelVisible,
} from "./compareCurtain.ts";

test("the screen opens on all « Après » (the bar at the start)", () => {
  assert.equal(CURTAIN_START, 0);
});

test("arrows move 2 %, Shift and an arrow 10 %, never past the edges", () => {
  assert.equal(curtainFromKey(50, "ArrowRight", false), 52);
  assert.equal(curtainFromKey(50, "ArrowLeft", false), 48);
  assert.equal(curtainFromKey(50, "ArrowRight", true), 60);
  assert.equal(curtainFromKey(50, "ArrowLeft", true), 40);
  assert.equal(curtainFromKey(99, "ArrowRight", true), 100);
  assert.equal(curtainFromKey(1, "ArrowLeft", false), 0);
});

test("Home is all « Après », End is all « Avant », another key is not the handle's", () => {
  assert.equal(curtainFromKey(40, "Home", false), 0);
  assert.equal(curtainFromKey(40, "End", false), 100);
  assert.equal(curtainFromKey(40, "a", false), null);
  assert.equal(curtainFromKey(40, "Tab", false), null);
});

test("a double click goes to all « Avant » from the left half, all « Après » from the right", () => {
  assert.equal(curtainToggle(0), 100);
  assert.equal(curtainToggle(49), 100);
  assert.equal(curtainToggle(50), 0);
  assert.equal(curtainToggle(100), 0);
});

test("the pointer gives the share of the image to its left, clamped", () => {
  assert.equal(curtainFromPointer(300, 100, 400), 50);
  assert.equal(curtainFromPointer(50, 100, 400), 0);
  assert.equal(curtainFromPointer(900, 100, 400), 100);
  assert.equal(curtainFromPointer(300, 100, 0), 0);
});

test("a value that is not a number is the start", () => {
  assert.equal(clampCurtain(Number.NaN), 0);
  assert.equal(clampCurtain(-5), 0);
  assert.equal(clampCurtain(150), 100);
});

test("a label fades within 96 px of the bar", () => {
  // 600 px wide: the bar at 10 % is 60 px from the left edge.
  assert.equal(labelVisible("before", 10, 600), false);
  assert.equal(labelVisible("after", 10, 600), true);
  assert.equal(labelVisible("before", 50, 600), true);
  assert.equal(labelVisible("after", 50, 600), true);
  assert.equal(labelVisible("after", 90, 600), false);
  assert.equal(labelVisible("before", 90, 600), true);
});

test("the screen has the curtain's handle as a slider, reached by Tab, and no Avant | Après selector", () => {
  const screen = readFileSync(
    new URL("../screens/adjust/AdjustScreen.tsx", import.meta.url),
    "utf8",
  );
  assert.match(screen, /role="slider"/);
  assert.match(screen, /tabIndex=\{0\}/);
  assert.match(screen, /aria-valuetext=/);
  assert.match(screen, /onDoubleClick=/);
  assert.match(screen, /curtainFromKey\(/);
  assert.doesNotMatch(screen, /as-segments/);
  assert.doesNotMatch(screen, /aria-pressed/);
});

test("the dictionaries say the curtain in the ten languages, with the same blank", () => {
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
    assert.ok(dict["adjust.compare"], `${locale}: adjust.compare`);
    assert.match(dict["adjust.compare.value"], /\{percent\}/, locale);
  }
});
