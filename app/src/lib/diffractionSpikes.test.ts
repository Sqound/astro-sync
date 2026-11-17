import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

import {
  fluxThreshold,
  screenSpikes,
  spikeLayer,
  starCount,
  type SpikeStar,
} from "./diffractionSpikes.ts";
import { NEUTRAL, adjustPixels, type AdjustValues } from "./imageAdjust.ts";

interface Fixture {
  width: number;
  height: number;
  thresholds: Record<string, number>;
  stars: SpikeStar[];
  cases: {
    strength: number;
    reach: number;
    scale: number;
    starFactor: number;
    layer: number[];
  }[];
}

const fixture = JSON.parse(
  readFileSync(
    new URL("./diffractionSpikes.fixture.json", import.meta.url),
    "utf8",
  ),
) as Fixture;

test("the window draws the spikes the script draws on the same stars", () => {
  for (const item of fixture.cases) {
    const layer = spikeLayer(
      fixture.width,
      fixture.height,
      fixture.stars,
      item.strength,
      item.reach,
      item.scale,
      item.starFactor,
    );
    assert.ok(layer, "something is drawn");
    assert.equal(layer.length, item.layer.length);
    let worst = 0;
    for (let i = 0; i < layer.length; i += 1) {
      worst = Math.max(worst, Math.abs(layer[i] - item.layer[i]));
    }
    assert.ok(worst < 1e-4, `largest difference ${worst}`);
  }
});

test("the brightness thresholds are the script's", () => {
  for (const [reach, flux] of Object.entries(fixture.thresholds)) {
    assert.ok(Math.abs(fluxThreshold(Number(reach)) - flux) < 1e-9);
  }
});

test("nothing is drawn with no strength, no star or no star let through", () => {
  const { stars } = fixture;
  assert.equal(spikeLayer(64, 48, stars, 0, 50, 0.25, 1), null);
  assert.equal(spikeLayer(64, 48, stars, 60, 50, 0.25, 0), null);
  assert.equal(spikeLayer(64, 48, [], 60, 50, 0.25, 1), null);
  const faint = [{ ...stars[3] }];
  assert.equal(spikeLayer(64, 48, faint, 100, 100, 0.25, 1), null);
});

test("a longer reach lets more stars through", () => {
  const counts = [0, 25, 50, 75, 100].map((r) => starCount(fixture.stars, r));
  assert.deepEqual(
    counts,
    [...counts].sort((a, b) => a - b),
  );
  assert.ok(counts[0] < counts[4]);
});

test("a pixel the light does not touch keeps its exact value", () => {
  const p = [0.123456789, 0.5, 0.987654321];
  screenSpikes(p, new Float32Array([0, 0, 0]), 0);
  assert.deepEqual(p, [0.123456789, 0.5, 0.987654321]);
  screenSpikes(p, new Float32Array([0, 0.5, 0]), 0);
  assert.equal(p[0], 0.123456789);
  assert.ok(p[1] > 0.5 && p[1] < 1);
  assert.equal(p[2], 0.987654321);
});

test("with the spikes off the preview is unchanged to the bit, on it only gets lighter", () => {
  const width = 64;
  const height = 48;
  const base = new Uint8ClampedArray(width * height * 4);
  for (let i = 0; i < base.length; i += 4) base.set([20, 25, 40, 255], i);
  const mask = new Uint8ClampedArray(base.length).fill(255);
  const off = new Uint8ClampedArray(base.length);
  adjustPixels(base, null, mask, { ...NEUTRAL }, off, null);
  assert.deepEqual([...off], [...base]);

  const values: AdjustValues = { ...NEUTRAL, spikes: 100, spikesReach: 100 };
  const layer = spikeLayer(width, height, fixture.stars, 100, 100, 0.25, 1);
  assert.ok(layer);
  const on = new Uint8ClampedArray(base.length);
  adjustPixels(base, null, mask, values, on, layer);
  let lighter = 0;
  for (let i = 0; i < base.length; i += 1) {
    assert.ok(on[i] >= base[i]);
    if (on[i] > base[i]) lighter += 1;
  }
  assert.ok(lighter > 20);
});

test("the drawing of the preview is quick", () => {
  const stars: SpikeStar[] = [];
  for (let i = 0; i < 220; i += 1) {
    stars.push({
      x: (i * 37) % 4000,
      y: (i * 91) % 5500,
      flux: 3 + ((i * 7) % 60),
      radius: 2,
      colour: [1, 1, 1],
    });
  }
  const started = performance.now();
  const layer = spikeLayer(780, 1080, stars, 100, 100, 1080 / 5500, 1);
  const ms = performance.now() - started;
  assert.ok(layer);
  assert.ok(ms < 400, `${ms} ms`);
  console.log(`spike layer of the preview, 220 stars: ${ms.toFixed(1)} ms`);
});
