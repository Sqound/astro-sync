import { test } from "node:test";
import assert from "node:assert/strict";

import { partOfNightPath, targetFolders, targetPath } from "./targetFolders.ts";

test("a target of the new form gives input/ what the person brings and output/ what Astro Sync makes", () => {
  assert.deepEqual(targetFolders("inputOutput"), {
    lights: "input/lights/",
    processed: "input/processed/",
    device: "input/device/",
    stacked: "output/stacked/",
    progression: "output/progression/",
  });
  // A detection of an earlier build names no form: the new one.
  assert.equal(targetFolders(undefined).processed, "input/processed/");
  assert.equal(targetPath("M31", "lights"), "M31/input/lights/");
  assert.equal(
    targetPath("M31/", "stacked", "inputOutput"),
    "M31/output/stacked/",
  );
});

test("a target made before Astro Sync 1.2.0 keeps its folders side by side", () => {
  assert.equal(targetPath("M31", "processed", "legacy"), "M31/final/");
  assert.equal(targetPath("M31", "lights", "legacy"), "M31/lights/");
  assert.equal(targetPath("M31", "stacked", "legacy"), "M31/stacked/");
});

test("a path of the night is read in both forms", () => {
  assert.deepEqual(
    partOfNightPath(["M31", "input", "lights", "Ha", "l0.fit"]),
    {
      target: "M31",
      part: "lights",
      under: ["Ha", "l0.fit"],
    },
  );
  assert.deepEqual(partOfNightPath(["M31", "lights", "l0.fit"]), {
    target: "M31",
    part: "lights",
    under: ["l0.fit"],
  });
  assert.deepEqual(partOfNightPath(["M31", "input", "device", "Stacked.fit"]), {
    target: "M31",
    part: "device",
    under: ["Stacked.fit"],
  });
  assert.equal(partOfNightPath(["M31", "input", "x.fit"]), null);
  assert.equal(partOfNightPath(["calibration", "darks"]), null);
});
