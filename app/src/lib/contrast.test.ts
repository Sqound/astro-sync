import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

import {
  CONTRAST_MAX,
  CONTRAST_SLIDER,
  CONTRAST_STOPS,
  DEFAULT_CONTRAST,
  contrastOf,
  contrastSnap,
  contrastToCore,
  contrastValue,
  hasContrast,
} from "./contrast.ts";
import { SUPPORTED_LOCALES } from "../i18n/locale.ts";

test("the contrast is a continuous gain from 0 to 5 held on three detents: « Sans », « Présentation » 2.0 (the default) and « Fort » 3.6", () => {
  assert.deepEqual(
    CONTRAST_STOPS.map((stop) => [stop.value, stop.gain]),
    [
      [0, null],
      [2, 2.0],
      [3.6, 3.6],
    ],
  );
  assert.equal(DEFAULT_CONTRAST, 2);
  assert.ok(CONTRAST_MAX >= 4, "at least 4, as asked");
  assert.equal(CONTRAST_SLIDER.min, 0);
  assert.equal(CONTRAST_SLIDER.max, CONTRAST_MAX);
});

test("the magnets: near a detent the slider holds on it, elsewhere any value of the grain is kept", () => {
  assert.equal(contrastSnap(2.07), 2);
  assert.equal(contrastSnap(1.93), 2);
  assert.equal(contrastSnap(3.55), 3.6);
  assert.equal(contrastSnap(0.08), 0);
  assert.equal(contrastSnap(2.4), 2.4);
  assert.equal(contrastSnap(2.25), 2.25);
  assert.equal(contrastSnap(4.7), 4.7);
  assert.equal(contrastSnap(9), CONTRAST_MAX);
  assert.equal(contrastSnap(-1), 0);
  assert.equal(contrastSnap(Number.NaN), 0);
});

test("what a record or a settings file holds is read as a gain: words of the core, numbers, or nothing", () => {
  assert.equal(contrastValue("strong"), 3.6);
  assert.equal(contrastValue("presentation"), 2);
  assert.equal(contrastValue("none"), 0);
  assert.equal(contrastValue(2.5), 2.5);
  assert.equal(contrastValue("2.5"), 2.5);
  assert.equal(contrastValue(undefined), 0);
  assert.equal(contrastValue("extreme"), 0);
  assert.equal(contrastValue(9), CONTRAST_MAX);
  assert.equal(hasContrast("none"), false);
  assert.equal(hasContrast(undefined), false);
  assert.equal(hasContrast(0.5), true);
  // A settings file with nothing or a word that is not one reads as the default.
  assert.equal(contrastOf("strong"), 3.6);
  assert.equal(contrastOf(0), 0);
  assert.equal(contrastOf(2.5), 2.5);
  assert.equal(contrastOf("extreme"), 2);
  assert.equal(contrastOf(undefined), 2);
});

test("the core is sent the words it always read for the two historical gains, the number otherwise", () => {
  assert.deepEqual([0, 2, 3.6, 2.5, 4].map(contrastToCore), [
    "none",
    "presentation",
    "strong",
    2.5,
    4,
  ]);
});

test("each detent has its label in the ten languages (the gain in it for the two curves), and the custom words say the value", async () => {
  assert.equal(SUPPORTED_LOCALES.length, 10);
  for (const locale of SUPPORTED_LOCALES) {
    const dictionary = JSON.parse(
      await readFile(
        new URL(`../i18n/${locale}.json`, import.meta.url),
        "utf8",
      ),
    ) as Record<string, string>;
    const labels = CONTRAST_STOPS.map((stop) => dictionary[stop.labelKey]);
    assert.ok(
      labels.every((label) => label),
      `${locale}: a label is missing`,
    );
    assert.equal(new Set(labels).size, 3, `${locale}: the labels differ`);
    assert.match(labels[1], /2[.,]0/, `${locale}: ${labels[1]}`);
    assert.match(labels[2], /3[.,]6/, `${locale}: ${labels[2]}`);
    for (const key of [
      "wizard.treatments.contrast.title",
      "wizard.finish.contrast.help",
      "wizard.treatments.contrast.custom",
      "wizard.treatments.contrast.summary.custom",
    ]) {
      assert.ok(dictionary[key], `${locale}: ${key}`);
    }
    for (const key of [
      "wizard.treatments.contrast.custom",
      "wizard.treatments.contrast.summary.custom",
    ]) {
      assert.match(dictionary[key], /\{value\}/, `${locale}: ${key}`);
    }
  }
});

test("the slider is wired: a continuous range with its detents, the setting kept, one choice for every render", async () => {
  const read = (path: string) =>
    readFile(new URL(path, import.meta.url), "utf8");
  const slider = await read("../components/SnapSlider.tsx");
  assert.match(slider, /type="range"/);
  assert.match(slider, /min=\{spec\.min\}/);
  assert.match(slider, /max=\{spec\.max\}/);
  assert.match(slider, /step=\{spec\.step\}/);
  assert.match(slider, /aria-valuetext/);
  assert.match(slider, /snapTo\(spec, Number\(event\.target\.value\)\)/);
  const contrast = await read("../components/ContrastSlider.tsx");
  assert.match(contrast, /spec=\{CONTRAST_SLIDER\}/);
  const step = await read("../screens/night/TreatmentsStep.tsx");
  // One slider, under the box « Empilement », shown only when a
  // target is ticked; under the render of a plan only when it is not.
  assert.equal((step.match(/<ContrastSlider\b/g) ?? []).length, 1);
  assert.match(step, /result\.contrastShown \?/);
  assert.match(step, /<RenderExtras slider=\{slider\} \/>/);
  assert.match(step, /slider=\{!result\.contrastShown\}/);
  assert.match(step, /localSettingsSetContrast/);
});
