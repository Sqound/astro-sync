import assert from "node:assert/strict";
import test from "node:test";

import {
  detentOf,
  formatNumber,
  fractionOf,
  sliderLabel,
  snapTo,
  type SliderSpec,
} from "./snapSlider.ts";

const SPEC: SliderSpec = {
  min: 0,
  max: 10,
  step: 0.5,
  snap: 0.5,
  detents: [
    { value: 0, labelKey: "zero" },
    { value: 4, labelKey: "four" },
    { value: 6, labelKey: "six" },
  ],
  customKey: "custom",
  minDecimals: 1,
};

test("a value is clamped, set on the grain of the step, and held on a detent within the magnet's reach", () => {
  assert.equal(snapTo(SPEC, 4.4), 4);
  assert.equal(snapTo(SPEC, 3.5), 4);
  assert.equal(snapTo(SPEC, 3.0), 3);
  assert.equal(snapTo(SPEC, 7.3), 7.5);
  assert.equal(snapTo(SPEC, 12), 10);
  assert.equal(snapTo(SPEC, -3), 0);
  assert.equal(snapTo(SPEC, Number.NaN), 0);
  assert.equal(snapTo(SPEC, Number.POSITIVE_INFINITY), 0);
});

test("two detents closer than twice the reach: the nearer wins", () => {
  const close: SliderSpec = {
    ...SPEC,
    snap: 1.5,
    detents: [
      { value: 4, labelKey: "four" },
      { value: 6, labelKey: "six" },
    ],
  };
  assert.equal(snapTo(close, 4.5), 4);
  assert.equal(snapTo(close, 5.5), 6);
});

test("the detent a value sits on, where it stands along the track, and its words", () => {
  assert.equal(detentOf(SPEC, 4)?.labelKey, "four");
  assert.equal(detentOf(SPEC, 4.5), null);
  assert.equal(fractionOf(SPEC, 4), 0.4);
  const t = (key: string, params?: Record<string, string | number>) =>
    params ? `${key}:${params.value}` : key;
  assert.equal(sliderLabel(SPEC, 6, t, "6,0"), "six");
  assert.equal(sliderLabel(SPEC, 4.5, t, "4,5"), "custom:4,5");
});

test("a number is written as the language writes it", () => {
  assert.equal(formatNumber("fr", 2.4, 1), "2,4");
  assert.equal(formatNumber("en", 2.4, 1), "2.4");
  assert.equal(formatNumber("fr", 3, 1), "3,0");
  assert.equal(formatNumber("fr", 30), "30");
  assert.equal(formatNumber("fr", 2.25, 1), "2,25");
});
