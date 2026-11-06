import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

import { createTranslator } from "../i18n/locale.ts";
import type { BuildInfo } from "./bridge";
import {
  buildDateText,
  menuVersionLine,
  menuVersionTitle,
  settingsVersionLine,
  shownSha,
} from "./buildInfo.ts";

// The version of the build, in the menu and in Réglages.

const info: BuildInfo = {
  version: "1.0.0",
  gitSha: "6aa28f767",
  buildDate: "2026-10-13T23:30:00Z",
};
const unknown: BuildInfo = {
  version: "1.0.0",
  gitSha: "unknown",
  buildDate: "",
};

test("the menu says the version then the 9-character commit, a middle dot between", () => {
  assert.equal(
    menuVersionLine("Astro Sync", info),
    "Astro Sync 1.0.0 · 6aa28f767",
  );
  assert.equal(info.gitSha.length, 9);
});

test("a build outside a repository says the version alone", () => {
  assert.equal(shownSha(unknown), null);
  assert.equal(menuVersionLine("Astro Sync", unknown), "Astro Sync 1.0.0");
});

test("the date is written as people read it in their language, in UTC", () => {
  assert.equal(buildDateText(info, "fr"), "13 oct. 2026");
  assert.equal(buildDateText(info, "en"), "Oct 13, 2026");
  assert.equal(buildDateText(unknown, "fr"), null);
});

test("Réglages › Général writes « Version 1.0.0 · 6aa28f767 · construite le 13 oct. 2026 »", () => {
  const fr = createTranslator("fr");
  assert.equal(
    settingsVersionLine(fr, info, "fr"),
    "Version 1.0.0 · 6aa28f767 · construite le 13 oct. 2026",
  );
  const en = createTranslator("en");
  assert.equal(
    settingsVersionLine(en, info, "en"),
    "Version 1.0.0 · 6aa28f767 · built on Oct 13, 2026",
  );
  assert.equal(settingsVersionLine(fr, unknown, "fr"), "Version 1.0.0");
});

test("the menu line carries the date on hover, and nothing when it is unknown", () => {
  const fr = createTranslator("fr");
  assert.equal(menuVersionTitle(fr, info, "fr"), "Construite le 13 oct. 2026");
  assert.equal(menuVersionTitle(fr, unknown, "fr"), undefined);
});

test("the line sits at the foot of the menu in the muted text colour, no opacity", async () => {
  const css = await readFile(
    new URL("../theme/primitives.css", import.meta.url),
    "utf8",
  );
  const start = css.indexOf(".as-sidebar__version {");
  assert.ok(start >= 0, "primitives.css has no .as-sidebar__version rule");
  const rule = css.slice(start, css.indexOf("}", start));
  assert.match(rule, /margin: auto 0 0/);
  assert.match(rule, /color: var\(--as-color-muted-text\)/);
  assert.ok(!/opacity/.test(rule), "an opacity on the version line");
});
