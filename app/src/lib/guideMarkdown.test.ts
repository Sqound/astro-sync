import assert from "node:assert/strict";
import test from "node:test";

import { SUPPORTED_LOCALES, createTranslator } from "../i18n/locale.ts";
import {
  EXAMPLE_NIGHT,
  NINA_PATTERN,
  STRUCTURE_NOTES,
  STRUCTURE_ROWS,
  linkCommand,
  ninaPath,
} from "./guide.ts";
import { guideMarkdown } from "./guideMarkdown.ts";

// The Guide page in Markdown, what the guide's file
// holds and « Copier le guide entier » copies -- the page's own text, in each
// of the ten languages.

const SHOWN = "~/Documents/Astro Sync";

test("the Guide in Markdown is the page's text, in each of the ten languages", () => {
  assert.equal(SUPPORTED_LOCALES.length, 10);
  for (const locale of SUPPORTED_LOCALES) {
    const t = createTranslator(locale);
    const text = guideMarkdown(t, SHOWN, "Astro Sync");
    // The title and the lead, the three sections.
    assert.ok(text.startsWith(`# ${t("guide.title")}\n\n${t("guide.lead")}\n`));
    for (const heading of [
      `## ${t("guide.anchor.structure")}`,
      `## ${t("guide.anchor.finder")}`,
      `## ${t("guide.tips")}`,
      `### ${t("guide.anchor.move")}`,
      `### ${t("guide.links.title")}`,
      `### ${t("guide.anchor.nina")}`,
    ]) {
      assert.ok(text.includes(heading), `${locale}: ${heading}`);
    }
    // Every sentence of the page is there.
    for (const key of [
      "guide.structure.lead",
      "guide.finder.lead",
      "guide.finder.windows",
      "guide.finder.finalTitle",
      "guide.finder.finalLine",
      "guide.finder.hiddenTitle",
      "guide.finder.hiddenLine",
      "guide.move.line",
      "guide.links.line",
      "guide.nina.line",
      "guide.ai.line",
      ...STRUCTURE_NOTES,
    ]) {
      assert.ok(text.includes(t(key)), `${locale}: ${key}`);
    }
    // The table: its head and one row per folder, in the language.
    assert.ok(
      text.includes(
        `| ${t("guide.table.folder")} | ${t("guide.table.contents")} | ${t("guide.table.use")} |`,
      ),
      locale,
    );
    for (const row of STRUCTURE_ROWS) {
      const path = typeof row.path === "string" ? row.path : t(row.path.key);
      assert.ok(
        text.includes(
          `| \`${path}\` | ${t(row.contentsKey)} | ${t(row.useKey)} |`,
        ),
        `${locale}: ${path}`,
      );
    }
    // The command, NINA's path and pattern are the page's, written once.
    assert.ok(text.includes(linkCommand(SHOWN, t("guide.links.disk"))), locale);
    assert.ok(text.includes(ninaPath(SHOWN)), locale);
    assert.ok(text.includes(NINA_PATTERN), locale);
    // The tree carries the example night and the root.
    assert.ok(text.includes("Astro Sync/\n"), locale);
    assert.ok(text.includes(`${EXAMPLE_NIGHT}/`), locale);
    // Nothing is left unsaid: no key, no placeholder, no « undefined ».
    assert.doesNotMatch(text, /\bguide\.[a-z]+\.[a-z]+/u, locale);
    assert.doesNotMatch(text, /\bwizard\.(tree|folder)\./u, locale);
    assert.doesNotMatch(text, /undefined|\[object/u, locale);
    assert.ok(text.endsWith("\n") && !text.endsWith("\n\n"), locale);
  }
});

test("the table cells never break their row, and the text is deterministic", () => {
  const t = createTranslator("fr");
  const text = guideMarkdown(t, SHOWN, "Astro Sync");
  const rows = text.split("\n").filter((line) => line.startsWith("| "));
  assert.equal(rows.length, 2 + STRUCTURE_ROWS.length);
  for (const row of rows) {
    // Three cells: four unescaped pipes.
    assert.equal(row.replace(/\\\|/g, "").split("|").length - 1, 4, row);
  }
  // The same text twice: the fingerprint of the file stays put.
  assert.equal(text, guideMarkdown(t, SHOWN, "Astro Sync"));
  // A folder elsewhere changes only the paths.
  const elsewhere = guideMarkdown(t, "/mnt/astro/Nuits", "Nuits");
  assert.ok(elsewhere.includes("/mnt/astro/Nuits/"));
  assert.ok(elsewhere.includes("Nuits/\n"));
  assert.ok(!elsewhere.includes(SHOWN));
});

test("a pipe in a cell is escaped", () => {
  const t = (key: string) =>
    key === "guide.row.logs.contents" ? "a | b" : key;
  const text = guideMarkdown(t, SHOWN, "Astro Sync");
  assert.ok(text.includes("| a \\| b |"));
});
