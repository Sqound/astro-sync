import assert from "node:assert/strict";
import test from "node:test";

import { createTranslator } from "../i18n/locale.ts";
import { formatDuration } from "./duration.ts";

const fr = createTranslator("fr");
const sec = (value: number) => value * 1000;

test("a length of time reads in seconds, then minutes and seconds, then hours and minutes", () => {
  assert.equal(formatDuration(fr, 0), "0 s");
  assert.equal(formatDuration(fr, sec(47)), "47 s");
  assert.equal(formatDuration(fr, sec(59) + 999), "59 s");
  assert.equal(formatDuration(fr, sec(60)), "1 min 00 s");
  assert.equal(formatDuration(fr, sec(134)), "2 min 14 s");
  assert.equal(formatDuration(fr, sec(362)), "6 min 02 s");
  assert.equal(formatDuration(fr, sec(3599)), "59 min 59 s");
  assert.equal(formatDuration(fr, sec(3720)), "1 h 02 min");
  assert.equal(formatDuration(fr, sec(7200 + 59)), "2 h 00 min");
});

test("a negative or a broken length is zero, never a sentence with NaN", () => {
  assert.equal(formatDuration(fr, -5000), "0 s");
  assert.equal(formatDuration(fr, Number.NaN), "0 s");
  assert.equal(formatDuration(fr, Number.POSITIVE_INFINITY), "0 s");
});

test("each language writes the units of its own dictionary", () => {
  assert.equal(formatDuration(createTranslator("ja"), sec(134)), "2 分 14 秒");
  assert.equal(formatDuration(createTranslator("en"), sec(3720)), "1 h 02 min");
  assert.equal(formatDuration(createTranslator("ru"), sec(47)), "47 с");
});
