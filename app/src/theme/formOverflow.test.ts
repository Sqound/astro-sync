import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

// A field never goes past the box that holds it. The boxes themselves are
// measured in a real page by `tests/astro-sync-form-overflow .test.mjs`;
// this file holds the rules of the common style that make it so, so a field
// added to any window inherits them.

const SHEETS = [
  "primitives.css",
  "shell.css",
  "targets.css",
  "multinight.css",
  "stacking.css",
  "device.css",
  "detect.css",
  "organise.css",
  "send-detect.css",
  "detect-review.css",
  "detect-result.css",
  "processing.css",
];

async function sheet(name: string): Promise<string> {
  return readFile(new URL(`./${name}`, import.meta.url), "utf8");
}

/** The declarations of the rule whose selector is exactly `selector`. */
function rule(source: string, selector: string): string {
  const pattern = new RegExp(
    `(?:^|\\})\\s*${selector.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}\\s*\\{([^}]*)\\}`,
    "m",
  );
  const found = pattern.exec(source);
  assert.ok(found, `no rule for ${selector}`);
  return found[1];
}

test("a text field counts its border in its width, can shrink to its column and stops at its container", async () => {
  const field = rule(await sheet("primitives.css"), ".as-field");
  assert.match(field, /box-sizing: border-box/);
  assert.match(field, /min-width: 0/);
  assert.match(field, /max-width: 100%/);
});

test("the cell of a dialog's grid and its tracks can shrink below the width of what they hold", async () => {
  const source = await sheet("primitives.css");
  assert.match(rule(source, ".as-dialog__field"), /min-width: 0/);
  assert.match(
    rule(source, ".as-dialog__grid--three"),
    /minmax\(0, 1fr\) minmax\(0, 1fr\) minmax\(0, 2fr\)/,
  );
  assert.match(
    rule(await sheet("targets.css"), ".as-dialog__grid--coordinates"),
    /minmax\(0, 1fr\) minmax\(0, 1fr\) auto/,
  );
});

test("no grid track of the style is a bare `fr`: it would grow with the widest field in it (minmax(0, …fr))", async () => {
  for (const name of SHEETS) {
    const source = await sheet(name);
    for (const declaration of source.matchAll(
      /grid-template-columns:\s*([^;]+);/g,
    )) {
      const value = declaration[1].replace(/minmax\([^)]*\)/g, "");
      assert.ok(
        !/\d(?:\.\d+)?fr\b/.test(value),
        `${name}: grid-template-columns: ${declaration[1]}`,
      );
    }
  }
});

test("a badge wraps inside the cell that holds it", async () => {
  const badge = rule(await sheet("primitives.css"), ".as-badge");
  assert.match(badge, /max-width: 100%/);
  assert.match(badge, /box-sizing: border-box/);
});
