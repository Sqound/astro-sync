import assert from "node:assert/strict";
import { readdir, readFile } from "node:fs/promises";
import test from "node:test";

// In « Traitements » of the detection flow, the hint, the target cards and
// the frame of the run went past the right edge of their card (by the length
// of the engine's last line). The card is a `display: grid` with no column
// declared: its one implicit track is `auto`, as wide as the widest content
// it holds, and the frame's last line is `white-space: nowrap`. A track
// written `minmax(0, 1fr)` is the width of the card, whatever the line. This
// file holds the rules of the style that make it so.

async function sheets(): Promise<Array<[string, string]>> {
  const folder = new URL("./", import.meta.url);
  const names = (await readdir(folder)).filter((name) => name.endsWith(".css"));
  return Promise.all(
    names.map(
      async (name) =>
        [name, await readFile(new URL(name, folder), "utf8")] as [
          string,
          string,
        ],
    ),
  );
}

/** Every `{selector, body}` of a style sheet (no nesting in this style). */
function rules(source: string): Array<{ selector: string; body: string }> {
  const stripped = source.replace(/\/\*[\s\S]*?\*\//g, "");
  return [...stripped.matchAll(/([^{}]+)\{([^{}]*)\}/g)].map((found) => ({
    selector: found[1].trim().replace(/\s+/g, " "),
    body: found[2],
  }));
}

function body(source: string, selector: string): string {
  const found = rules(source).find((item) => item.selector === selector);
  assert.ok(found, `no rule for ${selector}`);
  return found.body;
}

test("every grid of the style names its columns: an implicit `auto` track grows with its widest content and pushes the others out of the card", async () => {
  const bare: string[] = [];
  for (const [name, source] of await sheets()) {
    for (const { selector, body: declarations } of rules(source)) {
      if (
        /display:\s*grid\b/.test(declarations) &&
        !/grid-template(?:-columns)?:/.test(declarations)
      )
        bare.push(`${name}: ${selector}`);
    }
  }
  assert.deepEqual(bare, []);
});

test("the card of « Traitements », its hint, its lists and the run take the width of the card (minmax(0, 1fr))", async () => {
  const source = await readFile(
    new URL("./detect-result.css", import.meta.url),
    "utf8",
  );
  for (const selector of [".as-treat__running", ".as-treat__links"])
    assert.match(
      body(source, selector),
      /grid-template-columns:\s*minmax\(0, 1fr\)/,
      selector,
    );
  const lists = rules(source).find(
    (item) => item.selector === ".as-treat__cards, .as-treat__images",
  );
  assert.ok(lists, "the lists of the treatments");
  assert.match(lists.body, /grid-template-columns:\s*minmax\(0, 1fr\)/);
});

test("the frame of a run shrinks to its container and cuts its last line with an ellipsis instead of widening", async () => {
  const source = await readFile(
    new URL("./primitives.css", import.meta.url),
    "utf8",
  );
  const frame = body(source, ".as-frame");
  assert.match(frame, /min-width:\s*0/);
  assert.match(frame, /max-width:\s*100%/);
  const line = body(source, ".as-frame__line");
  assert.match(line, /min-width:\s*0/);
  assert.match(line, /overflow:\s*hidden/);
  assert.match(line, /text-overflow:\s*ellipsis/);
  assert.match(line, /white-space:\s*nowrap/);
  for (const selector of [".as-frame__done", ".as-frame__log"])
    assert.match(
      body(source, selector),
      /grid-template-columns:\s*minmax\(0, 1fr\)/,
      selector,
    );
});
