import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

// The shell of every screen.

async function css(): Promise<string> {
  return readFile(new URL("./shell.css", import.meta.url), "utf8");
}

function rule(source: string, selector: string): string {
  const start = source.indexOf(`${selector} {`);
  assert.ok(start >= 0, `shell.css has no rule for ${selector}`);
  return source.slice(start, source.indexOf("}", start));
}

test("shell.css never hardcodes a color, only var(--as-*) references", async () => {
  const source = await css();
  assert.ok(!/#[0-9a-fA-F]{3,8}\b/.test(source), "a hex colour in shell.css");
  assert.ok(
    !/\b(?:rgb|rgba|hsl|hsla)\(/i.test(source),
    "an rgb()/hsl() colour in shell.css",
  );
});

test("the content has one width, centred, and never shrinks", async () => {
  const content = rule(await css(), ".as-content > *");
  assert.match(content, /width: 47\.5rem/);
  assert.match(content, /min-width: 47\.5rem/);
  assert.match(content, /margin-right: auto/);
  assert.match(content, /margin-left: auto/);
});

test("the head stays on top and the foot of a step at the bottom", async () => {
  const source = await css();
  assert.match(rule(source, ".as-page__top"), /position: sticky;\s+top: 0/);
  assert.match(rule(source, ".as-step-foot"), /position: sticky;\s+bottom: 0/);
  // Both are opaque: the content scrolls under them, never through them.
  assert.match(rule(source, ".as-page__top"), /background-color: var\(--as-/);
  assert.match(rule(source, ".as-step-foot"), /background-color: var\(--as-/);
});

test("« Nuits » scrolls its rows, not the page", async () => {
  const source = await css();
  assert.match(rule(source, ".as-content > .as-page--list"), /height: 100%/);
  assert.match(
    rule(source, ".as-page--list .as-nights__rows"),
    /overflow-y: auto/,
  );
});

test("the window opens larger, holds on a 13-inch laptop and has a minimum", async () => {
  const conf = JSON.parse(
    await readFile(
      new URL("../../src-tauri/tauri.conf.json", import.meta.url),
      "utf8",
    ),
  ) as {
    app: {
      windows: {
        width: number;
        height: number;
        minWidth: number;
        minHeight: number;
      }[];
    };
  };
  const [window] = conf.app.windows;
  assert.ok(window.width > 800 && window.height > 600, "larger than 800 × 600");
  // A 13-inch laptop shows 1 280 × 800 points at the least, less the menu bar.
  assert.ok(
    window.width <= 1280 && window.height <= 775,
    "fits a 13-inch laptop",
  );
  // The sidebar (12 rem and its two margins), the content (47.5 rem) and
  // its two margins.
  assert.ok(window.minWidth >= (13.5 + 47.5 + 3) * 16, "nothing shrinks");
  assert.ok(window.minHeight >= 600);
});

test("a list to choose from has the look of the text field", async () => {
  const source = await css();
  const select = rule(source, "select.as-field");
  assert.match(select, /appearance: none/);
  assert.match(select, /width: 100%/);
  // The chevron is the theme's, from its tokens.
  assert.match(select, /var\(--as-color-muted-text\)/);
  // The field and the list share one height and one size of text.
  assert.match(rule(source, ".as-field"), /min-height: 2\.375rem/);
  assert.match(rule(source, ".as-field"), /font-size: 0\.875rem/);
});

test("the snackbars sit at the bottom centre, a thin edge of their tone, never a solid block", async () => {
  const source = await css();
  const stack = rule(source, ".as-snackbars");
  assert.match(stack, /position: fixed/);
  assert.match(stack, /bottom: /);
  assert.match(stack, /translateX\(-50%\)/);
  const card = rule(source, ".as-snackbar");
  assert.match(card, /border-left: 3px solid var\(--as-snack-tone\)/);
  assert.match(card, /background-color: var\(--as-color-modal-surface\)/);
  for (const tone of ["success", "info", "warning", "error"]) {
    assert.match(
      rule(source, `.as-snackbar--${tone}`),
      /--as-snack-tone: var\(--as-color-/,
    );
  }
});
