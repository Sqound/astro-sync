import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

// « Régler l'image » leaves the fixed column of 47.5 rem the other screens
// have, and only this page does.

async function read(name: string): Promise<string> {
  return readFile(new URL(name, import.meta.url), "utf8");
}

function ruleBody(source: string, selector: string): string {
  const stripped = source.replace(/\/\*[\s\S]*?\*\//g, "");
  const start = stripped.indexOf(`${selector} {`);
  assert.ok(start >= 0, `no rule for ${selector}`);
  return stripped.slice(start, stripped.indexOf("}", start));
}

test("the adjust page is two columns, wider than the fixed column, and as high as the window", async () => {
  const body = ruleBody(await read("adjust.css"), ".as-content > .as-adjust");
  assert.match(body, /display: grid/);
  assert.match(body, /grid-template-columns: minmax\(0, 1fr\) clamp\(22rem/);
  assert.match(body, /width: auto/);
  assert.match(body, /min-width: 0/);
  assert.match(body, /height: 100%/);
});

test("the other screens keep the column of 47.5 rem", async () => {
  const shell = ruleBody(await read("shell.css"), ".as-content > *");
  assert.match(shell, /width: 47\.5rem/);
  assert.ok(!(await read("adjust.css")).includes(".as-content > * {"));
});

test("the panel scrolls by itself and its foot stays", async () => {
  const css = await read("adjust.css");
  assert.match(ruleBody(css, ".as-adjust__scroll"), /overflow-y: auto/);
  assert.match(ruleBody(css, ".as-adjust__scroll"), /min-height: 0/);
  assert.match(ruleBody(css, ".as-adjust__foot"), /flex: none/);
});

test("under 48 rem the panel goes under the image, which keeps 40 % of the height", async () => {
  const css = await read("adjust.css");
  const media = css.slice(css.indexOf("@media (max-width: 48rem)"));
  assert.match(media, /grid-template-columns: minmax\(0, 1fr\);/);
  assert.match(media, /height: 40vh/);
});

test("the sliders take two columns once the panel passes 30 rem", async () => {
  // 13 rem a column, a 1.5 rem gap, 1 rem of padding: 30 rem is the first width
  // that holds two.
  const body = ruleBody(await read("adjust.css"), ".as-adjust__sliders");
  assert.match(body, /repeat\(auto-fill, minmax\(13rem, 1fr\)\)/);
});
