import assert from "node:assert/strict";
import test from "node:test";

import { MENU, MENU_GROUPS, MENU_TOP, menuEntryOf } from "./menu.ts";
import { createTranslator } from "../i18n/locale.ts";

test("the menu is « Accueil » alone, then « Vos nuits » and « Application », in the validated order and words", () => {
  const fr = createTranslator("fr");
  assert.deepEqual(
    MENU_TOP.map((entry) => fr(entry.labelKey)),
    ["Accueil"],
  );
  assert.ok(MENU_TOP.every((entry) => entry.section === undefined));
  assert.deepEqual(
    MENU_GROUPS.map((group) => [
      fr(group.titleKey),
      group.entries.map((entry) => fr(entry.labelKey)),
    ]),
    [
      ["Vos nuits", ["Sessions", "Progression", "Journaux", "Annotations"]],
      ["Application", ["Envois", "Espace Astro Portal", "Réglages", "Guide"]],
    ],
  );
  // The flat list is the same entries in the same order, nothing lost.
  assert.deepEqual(
    MENU.map((entry) => entry.id),
    [
      "home",
      "sessions",
      "progression",
      "logs",
      "annotations",
      "queue",
      "space",
      "settings",
      "guide",
    ],
  );
  assert.deepEqual(
    [...MENU_TOP, ...MENU_GROUPS.flatMap((group) => group.entries)],
    [...MENU],
  );
  // « Simulation et paquet » is not in the menu: opened from a night.
  assert.ok(!MENU.some((entry) => entry.id === "dryRun"));
});

test("the section titles exist in the ten languages", async () => {
  const { SUPPORTED_LOCALES } = await import("../i18n/locale.ts");
  assert.equal(SUPPORTED_LOCALES.length, 10);
  for (const locale of SUPPORTED_LOCALES) {
    const t = createTranslator(locale);
    for (const group of MENU_GROUPS) {
      assert.notEqual(t(group.titleKey), group.titleKey, `${locale}`);
    }
  }
  const expected: Record<string, [string, string]> = {
    fr: ["Vos nuits", "Application"],
    en: ["Your nights", "Application"],
    de: ["Deine Nächte", "Anwendung"],
    ja: ["夜", "アプリ"],
  };
  for (const [locale, [nights, app]] of Object.entries(expected)) {
    const t = createTranslator(locale as never);
    assert.equal(t("nav.section.nights"), nights);
    assert.equal(t("nav.section.app"), app);
  }
});

test("the wizard of a night and the simulation keep « Sessions » chosen, the queue is « Envois », the space is its own entry", () => {
  for (const screen of ["night", "dryRun", "sessions"] as const) {
    assert.equal(menuEntryOf(screen), "sessions", screen);
  }
  assert.equal(menuEntryOf("queue"), "queue");
  assert.equal(menuEntryOf("space"), "space");
  for (const entry of MENU) assert.equal(menuEntryOf(entry.id), entry.id);
});

test("the sidebar renders « Accueil » alone, one labelled list per titled group, the indent, and the badge on « Envois »", async () => {
  const { readFile } = await import("node:fs/promises");
  const app = await readFile(new URL("../App.tsx", import.meta.url), "utf8");
  // Built from the menu, not written out by hand.
  assert.ok(app.includes("MENU_TOP.map"));
  assert.ok(app.includes("MENU_GROUPS.map"));
  // A title is a plain paragraph (not a button), the list it names says so.
  assert.match(
    app,
    /<p\s+id=\{`as-menu-\$\{group\.section\}`\}\s+className=\{tabSectionTitleClassName\}/,
  );
  assert.ok(app.includes("<ul aria-labelledby={`as-menu-${group.section}`}>"));
  assert.ok(app.includes("renderEntry(entry, true)"));
  // The active entry stays marked; the badge stays on « Envois » only.
  assert.ok(app.includes('aria-current={active ? "page" : undefined}'));
  assert.ok(app.includes('tab.id === "queue" && pending > 0'));
  assert.ok(app.includes('className="as-tab__badge"'));
  // « Espace Astro Portal » goes through the same route as the home's link.
  assert.ok(app.includes('screen === "space"'));

  const primitives = await import("../theme/primitives.ts");
  assert.equal(
    primitives.tabSectionTitleClassName,
    "as-tabs__title as-tabs__title--gold-70",
  );
  assert.equal(
    primitives.tabClassName(true, true),
    "as-tab as-tab--nested as-tab--active",
  );
  const css = await readFile(
    new URL("../theme/primitives.css", import.meta.url),
    "utf8",
  );
  assert.match(
    css,
    /\.as-tabs__title--gold-70 \{\s*color: color-mix\(\s*in srgb,\s*var\(--as-color-gold-accent\) 70%,\s*var\(--as-color-panel\)/,
  );
  assert.match(
    css,
    /\.as-tab--nested \{\s*padding-left: calc\(var\(--as-space-md\) \+ 0\.875rem\)/,
  );
  // The hover: light, never gold text.
  const hover = css.slice(css.indexOf(".as-tab:not(.as-tab--active):hover"));
  const rule = hover.slice(0, hover.indexOf("}"));
  assert.match(rule, /background-color: color-mix\(/);
  assert.ok(!/gold/.test(rule), "the hover must not use gold text");
});

test("a menu entry keeps its badge in line: at the right edge of the label when both fit, the import's under the label at the start of the line, never floated", async () => {
  const { readFile } = await import("node:fs/promises");
  const read = (relative: string) =>
    readFile(new URL(relative, import.meta.url), "utf8");
  const app = await read("../App.tsx");
  // The label is its own box, stacked above the import's badge only.
  assert.ok(app.includes('"as-tab__label as-tab__label--stacked"'));
  assert.match(app, /tab\.id === "import" && !presence\.done/);
  const primitives = await read("../theme/primitives.css");
  const rule = (source: string, selector: string) => {
    const start = source.indexOf(`${selector} {`);
    assert.ok(start >= 0, `no rule for ${selector}`);
    return source.slice(start, source.indexOf("}", start));
  };
  const tab = rule(primitives, ".as-tab");
  assert.match(tab, /display: flex/);
  assert.match(tab, /flex-wrap: wrap/);
  // A lone item of a line sits at its start; two share the line, the badge
  // at the right edge.
  assert.match(tab, /justify-content: space-between/);
  assert.match(rule(primitives, ".as-tab__label--stacked"), /flex-basis: 100%/);
  const uploads = await read("../theme/uploads.css");
  const badge = rule(uploads, ".as-tab__badge");
  assert.ok(!/float:/.test(badge), "a badge never floats");
  assert.match(badge, /flex: 0 0 auto/);
});

test("« Associer un dossier » and « Analyse locale » left the journey: no screen, no route, no command", async () => {
  const { readFile, readdir } = await import("node:fs/promises");
  const read = (relative: string) =>
    readFile(new URL(relative, import.meta.url), "utf8");
  const app = await read("../App.tsx");
  const screens = await readdir(new URL("../screens/", import.meta.url));
  for (const gone of [
    "AssociateScreen",
    "AnalysisScreen",
    "PlaceholderScreen",
  ]) {
    assert.ok(!app.includes(gone), `App.tsx still opens ${gone}`);
    assert.ok(!screens.includes(`${gone}.tsx`), `${gone}.tsx is still there`);
  }
  const lib = await read("../../src-tauri/src/lib.rs");
  for (const command of [
    "list_associations",
    "associate_existing",
    "associate_new_folder",
    "analysis_report",
    "upload_analysis",
    "sky_measure,",
    "upload_sky_measure",
    "siril_detect",
  ]) {
    assert.ok(!lib.includes(command), `the window still registers ${command}`);
  }
  // The queue (« Envois ») has its menu entry; the simulation stays, opened from a night's summary.
  assert.ok(app.includes('{screen === "queue" && <QueueScreen />}'));
  assert.ok(app.includes('{screen === "dryRun" && <DryRunScreen />}'));
  // Réglages › « Dépendances » keeps « Réglages » chosen.
  assert.equal(menuEntryOf("dependencies"), "settings");
});
