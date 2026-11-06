import assert from "node:assert/strict";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import test, { before } from "node:test";
import { fileURLToPath } from "node:url";

import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { rolldown } from "rolldown";

import type { NightRow } from "../lib/nights.ts";

// In « Nuits », the chip « Plan multi-nuits IC 1805 · 3 nuits · 14 h 09 » of
// the « Cibles » column ran over the text of « Sur cet ordinateur » (a badge
// is `nowrap`, its column 7.5 rem). The row is bundled on the fly (the test
// runner reads no JSX) and drawn to a string; the rules of the grid are read
// from the style sheets.

type Harness = typeof import("./nightRowHarness.tsx");
let harness: Harness;

before(async () => {
  const stubCss = {
    name: "stub-css",
    load(id: string) {
      return id.endsWith(".css")
        ? { code: "export default '';", moduleType: "js" as const }
        : null;
    },
  };
  const bundle = await rolldown({
    input: fileURLToPath(new URL("./nightRowHarness.tsx", import.meta.url)),
    platform: "node",
    plugins: [stubCss],
    external: ["react", "react/jsx-runtime", "react-dom"],
    logLevel: "silent",
  });
  const { output } = await bundle.generate({ format: "esm" });
  const folder = new URL(
    "../../node_modules/.cache/astro-sync-tests/",
    import.meta.url,
  );
  await mkdir(folder, { recursive: true });
  const file = new URL("night-row.mjs", folder);
  await writeFile(file, output[0].code);
  harness = await import(file.href);
});

const PLAN_TAG = "Plan multi-nuits IC 1805 · 3 nuits · 14 h 09";
const SUMMARY = "1 cible · 205 poses · 1 intégration";

function row(partial: Partial<NightRow>): NightRow {
  return {
    sessionId: "night-1",
    day: "Jeu. 8 oct.",
    hours: "21:04 – 05:40",
    open: false,
    placeAndTargets: "Col des Étoiles · IC 1805",
    local: SUMMARY,
    online: "—",
    state: "complete",
    missing: null,
    reading: false,
    held: null,
    heldMessage: null,
    planTag: PLAN_TAG,
    ...partial,
  };
}

function drawn(value: NightRow): string {
  return renderToStaticMarkup(
    createElement(
      harness.I18nProvider,
      { locale: "fr", onLocaleChange: () => {} },
      createElement(harness.NightRowButton, {
        row: value,
        serviceDown: false,
        onOpen: () => {},
      }),
    ),
  );
}

/** The inner HTML of each `role="cell"` box of a row, in order. A cell holds
 * no other cell, so the next `role="cell"` (or the end) closes it. */
function cells(html: string): string[] {
  return html
    .split('role="cell"')
    .slice(1)
    .map((part) => part.replace(/^[^>]*>/, ""));
}

async function sheet(name: string): Promise<string> {
  return readFile(new URL(`../theme/${name}`, import.meta.url), "utf8");
}

function rule(source: string, selector: string): string {
  const pattern = new RegExp(
    `(?:^|\\})\\s*${selector.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}\\s*\\{([^}]*)\\}`,
    "m",
  );
  const found = pattern.exec(source);
  assert.ok(found, `no rule for ${selector}`);
  return found[1];
}

test("the plan chip is inside the « Cibles » cell, the summary inside « Sur cet ordinateur », never the one in the other", () => {
  const found = cells(drawn(row({})));
  assert.equal(found.length, 5);
  const [night, targets, local, online, state] = found;

  assert.ok(targets.includes(PLAN_TAG), "the chip is in the Cibles cell");
  assert.ok(targets.includes("as-nights__plan-tag"));
  assert.ok(
    targets.includes(`title="${PLAN_TAG}"`),
    "the full text is its title",
  );
  assert.ok(local.includes(SUMMARY), "the summary is in its own cell");
  for (const other of [night, local, online, state]) {
    assert.ok(!other.includes(PLAN_TAG), "the chip is in no other cell");
  }
  for (const other of [night, targets, online, state]) {
    assert.ok(!other.includes(SUMMARY), "the summary is in no other cell");
  }
});

test("a row of one, two or three lines per cell keeps each text in its cell", () => {
  const one = cells(drawn(row({ planTag: null, local: "—" })));
  const two = cells(drawn(row({ open: true })));
  const three = cells(
    drawn(row({ open: true, missing: "Dossier de la nuit à choisir" })),
  );
  assert.ok(!one[1].includes("as-badge"));
  assert.ok(two[1].includes(PLAN_TAG) && two[1].includes("En cours"));
  assert.ok(three[1].includes(PLAN_TAG) && !three[1].includes("à choisir"));
  assert.ok(three[4].includes("Dossier de la nuit à choisir"));
});

test("the row and its cells are grid and flow boxes, nothing is drawn with a position or an inline style", () => {
  const html = drawn(row({ open: true }));
  assert.ok(html.includes('class="as-nights__row"'));
  assert.equal(html.split('class="as-nights__cell"').length - 1, 5);
  assert.ok(!/\sstyle=/.test(html), "no inline style");
});

test("the style sheets lay the row out in grid tracks that can shrink, cells in flow, the chip wrapping in its cell", async () => {
  const primitives = await sheet("primitives.css");
  const shell = await sheet("shell.css");
  const multinight = await sheet("multinight.css");

  const grid = rule(primitives, ".as-nights__head,\n.as-nights__row");
  assert.match(grid, /display: grid/);
  assert.match(grid, /grid-template-columns:/);
  assert.match(
    primitives,
    /\n\.as-nights__row \{\s*box-sizing: border-box;\s*width: 100%/,
  );

  const cell = rule(primitives, ".as-nights__cell");
  assert.match(cell, /display: flex/);
  assert.match(cell, /flex-direction: column/);
  assert.match(rule(shell, ".as-nights__cell"), /min-width: 0/);
  assert.match(rule(shell, ".as-nights__cell"), /overflow-wrap: anywhere/);
  assert.match(rule(shell, ".as-nights__cell > *"), /max-width: 100%/);

  const chip = rule(multinight, ".as-nights__plan-tag");
  assert.match(chip, /white-space: normal/);
  assert.match(chip, /max-width: 100%/);
  assert.match(chip, /min-width: 0/);
  assert.match(chip, /overflow-wrap: anywhere/);

  for (const [name, source] of [
    ["primitives.css", primitives],
    ["shell.css", shell],
    ["multinight.css", multinight],
  ] as const) {
    for (const [, selector, body] of source.matchAll(
      /([^{}]*\.as-nights[^{}]*)\{([^}]*)\}/g,
    )) {
      assert.ok(
        !/position:\s*(absolute|fixed)/.test(body),
        `${name}: ${selector.trim()} is positioned out of the flow`,
      );
    }
  }
});
