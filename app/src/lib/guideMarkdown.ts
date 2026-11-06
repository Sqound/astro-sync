// The text of the Guide page in Markdown (« Poser une
// question à une IA »): what Astro Sync writes at the root of the working
// folder as `GUIDE-ASTRO-SYNC.<language>.md` and what « Copier le guide
// entier » copies. It is the page's own text, read from the dictionary of the
// language asked for (`guide.*`, `wizard.tree.*`): no sentence is written for
// it alone. The parts that do not change from one language to another
// (folder names, the command, NINA's pattern) stay as the page shows them.
// Pure TypeScript, tested under `node --test`.

import type { Translate } from "../i18n/locale.ts";
import {
  EXAMPLE_NIGHT,
  EXAMPLE_TARGETS,
  NINA_PATTERN,
  STRUCTURE_NOTES,
  STRUCTURE_ROWS,
  linkCommand,
  ninaPath,
} from "./guide.ts";
import { folderTree } from "./wizardSteps.ts";

/** A table cell: a pipe or a line break would cut the row. */
function cell(text: string): string {
  return text.replace(/\|/g, "\\|").replace(/\n/g, " ");
}

/** A fenced block. */
function fence(language: string, body: string): string {
  return ["```" + language, body, "```"].join("\n");
}

/**
 * The Guide page of Astro Sync in Markdown, in the language of `t`.
 * `shown` is the working folder as the page writes it (`~/Documents/Astro
 * Sync`), `rootName` its folder name.
 */
export function guideMarkdown(
  t: Translate,
  shown: string,
  rootName: string,
): string {
  const tree = folderTree(rootName, EXAMPLE_NIGHT, EXAMPLE_TARGETS)
    .map((row) => {
      const note = row.noteKey ? t(row.noteKey) : (row.note ?? "");
      return `${"  ".repeat(row.depth)}${row.name}${note ? `  # ${note}` : ""}`;
    })
    .join("\n");
  const rows = STRUCTURE_ROWS.map((row) => {
    const path = typeof row.path === "string" ? row.path : t(row.path.key);
    return `| \`${cell(path)}\` | ${cell(t(row.contentsKey))} | ${cell(t(row.useKey))} |`;
  });
  const parts = [
    `# ${t("guide.title")}`,
    t("guide.lead"),
    `## ${t("guide.anchor.structure")}`,
    t("guide.structure.lead"),
    fence("text", tree),
    [
      `| ${cell(t("guide.table.folder"))} | ${cell(t("guide.table.contents"))} | ${cell(t("guide.table.use"))} |`,
      "| --- | --- | --- |",
      ...rows,
    ].join("\n"),
    STRUCTURE_NOTES.map((key) => `- ${t(key)}`).join("\n"),
    `## ${t("guide.anchor.finder")}`,
    t("guide.finder.lead"),
    t("guide.finder.windows"),
    `### ${t("guide.finder.finalTitle")}`,
    t("guide.finder.finalLine"),
    `### ${t("guide.finder.hiddenTitle")}`,
    t("guide.finder.hiddenLine"),
    `## ${t("guide.tips")}`,
    `### ${t("guide.anchor.move")}`,
    t("guide.move.line"),
    `### ${t("guide.links.title")}`,
    t("guide.links.line"),
    fence("sh", linkCommand(shown, t("guide.links.disk"))),
    `### ${t("guide.anchor.nina")}`,
    t("guide.nina.line"),
    `**${t("guide.nina.path")}**`,
    fence("text", ninaPath(shown)),
    `**${t("guide.nina.pattern")}**`,
    fence("text", NINA_PATTERN),
    `### ${t("wizard.folder.askAssistant")}`,
    t("guide.ai.line"),
  ];
  return parts.join("\n\n") + "\n";
}
