import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

import {
  appShellClassName,
  badgeClassName,
  buttonClassName,
  cardClassName,
  chipClassName,
  confirmClassName,
  contentClassName,
  fieldClassName,
  fieldGroupClassName,
  fieldLabelClassName,
  helpTextClassName,
  homeClassName,
  inactiveButtonClassName,
  infoButtonClassName,
  infoLineClassName,
  linkClassName,
  listClassName,
  noticeClassName,
  panelClassName,
  progressClassName,
  rowClassName,
  sidebarClassName,
  stepNumberClassName,
  tabClassName,
  tabListClassName,
  tableClassName,
  tableWrapperClassName,
  toolbarClassName,
} from "./primitives.ts";

test("class-name helpers cover panels, buttons, fields, badges, progress, and confirmation", () => {
  assert.equal(panelClassName, "as-panel");
  assert.equal(fieldClassName, "as-field");
  assert.equal(buttonClassName(), "as-button as-button--primary");
  assert.equal(
    buttonClassName("tertiary", { destructive: true }),
    "as-button as-button--tertiary is-destructive",
  );
  assert.equal(
    buttonClassName("primary", { block: true }),
    "as-button as-button--primary as-button--block",
  );
  assert.equal(badgeClassName("success"), "as-badge as-badge--success");
  assert.equal(badgeClassName("neutral"), "as-badge as-badge--neutral");
  assert.deepEqual(progressClassName, {
    track: "as-progress",
    fill: "as-progress__fill",
  });
  assert.equal(confirmClassName(false), "as-confirm");
  assert.equal(confirmClassName(true), "as-confirm as-confirm--destructive");
});

test("7 layout/table/tab primitives are named as expected", () => {
  assert.equal(appShellClassName, "as-app-shell");
  assert.equal(sidebarClassName, "as-sidebar");
  assert.equal(contentClassName, "as-content");
  assert.equal(tabListClassName, "as-tabs");
  assert.equal(tabClassName(false), "as-tab");
  assert.equal(tabClassName(true), "as-tab as-tab--active");
  assert.equal(tabClassName(false, true), "as-tab as-tab--nested");
  assert.equal(tableClassName, "as-table");
  assert.equal(tableWrapperClassName, "as-table-wrapper");
  assert.equal(toolbarClassName, "as-toolbar");
  assert.equal(fieldGroupClassName, "as-field-group");
  assert.equal(fieldLabelClassName, "as-field-label");
  assert.equal(helpTextClassName, "as-help-text");
  assert.equal(linkClassName, "as-link");
});

test("primitives.css never hardcodes a color, only var(--as-*) references", async () => {
  const css = await readFile(
    new URL("./primitives.css", import.meta.url),
    "utf8",
  );
  const hexColor = /#[0-9a-fA-F]{3,8}\b/;
  const rgbOrHsl = /\b(?:rgb|rgba|hsl|hsla)\(/i;
  assert.ok(
    !hexColor.test(css),
    "found a hex color literal in primitives.css -- map it from tokens.ts instead",
  );
  assert.ok(
    !rgbOrHsl.test(css),
    "found an rgb()/hsl() color literal in primitives.css -- map it from tokens.ts instead",
  );
});

test("every primitive class referenced by the TS helpers has a rule in primitives.css", async () => {
  const css = await readFile(
    new URL("./primitives.css", import.meta.url),
    "utf8",
  );
  const classNames = [
    panelClassName,
    fieldClassName,
    buttonClassName("primary"),
    buttonClassName("secondary"),
    buttonClassName("tertiary"),
    buttonClassName("link"),
    buttonClassName("primary", { destructive: true, block: true }),
    inactiveButtonClassName,
    noticeClassName("success"),
    noticeClassName("warning"),
    noticeClassName("error"),
    noticeClassName("neutral"),
    cardClassName("connected"),
    cardClassName("current"),
    cardClassName("waiting"),
    stepNumberClassName("done"),
    stepNumberClassName("current"),
    stepNumberClassName("waiting"),
    homeClassName,
    listClassName,
    rowClassName,
    chipClassName,
    infoLineClassName,
    infoButtonClassName,
    badgeClassName("success"),
    badgeClassName("warning"),
    badgeClassName("error"),
    badgeClassName("neutral"),
    progressClassName.track,
    progressClassName.fill,
    confirmClassName(false),
    confirmClassName(true),
    appShellClassName,
    sidebarClassName,
    contentClassName,
    tabListClassName,
    tabClassName(false),
    tabClassName(true),
    tableClassName,
    tableWrapperClassName,
    toolbarClassName,
    fieldGroupClassName,
    fieldLabelClassName,
    helpTextClassName,
    linkClassName,
  ]
    .join(" ")
    .split(" ");

  for (const className of new Set(classNames)) {
    assert.ok(
      css.includes(`.${className}`),
      `primitives.css has no rule for .${className}`,
    );
  }
});

test("the seven looks of the buttons each have their rule, from the theme's roles", async () => {
  const css = await readFile(
    new URL("./primitives.css", import.meta.url),
    "utf8",
  );
  const rule = (selector: string): string => {
    const start = css.indexOf(`${selector} {`);
    assert.ok(start >= 0, `primitives.css has no rule for ${selector}`);
    return css.slice(start, css.indexOf("}", start));
  };
  // Primary, secondary, tertiary and link.
  assert.match(rule(".as-button--primary"), /--as-color-gold-accent/);
  assert.match(rule(".as-button--primary"), /--as-color-primary-edge/);
  assert.match(rule(".as-button--primary"), /--as-color-primary-ink/);
  assert.match(rule(".as-button--primary"), /font-weight: 700/);
  assert.match(rule(".as-button--secondary"), /--as-color-active-surface/);
  assert.match(rule(".as-button--secondary"), /--as-color-active-gold-text/);
  assert.match(rule(".as-button--tertiary"), /--as-color-tertiary-border/);
  assert.match(rule(".as-button--link"), /--as-radius-sm/);
  // The destructive tint at three levels, and the destructive primary.
  assert.match(
    rule(".as-button--tertiary.is-destructive"),
    /--as-color-destructive-outline/,
  );
  assert.match(
    rule(".as-button--tertiary.is-destructive"),
    /--as-color-destructive-text/,
  );
  assert.match(
    rule(".as-button--link.is-destructive"),
    /--as-color-destructive-text/,
  );
  assert.match(
    rule(".as-button--primary.is-destructive"),
    /--as-color-destructive-surface/,
  );
  assert.match(
    rule(".as-button--primary.is-destructive"),
    /--as-color-destructive-ink/,
  );
  // 12 px corners (8 for the link), 44 px at least, 48 for a full-width
  // primary, never a pill.
  assert.match(rule(".as-button"), /--as-radius-md/);
  assert.match(rule(".as-button"), /min-height: 2\.75rem/);
  assert.match(
    rule(".as-button--primary.as-button--block"),
    /min-height: 3rem/,
  );
  assert.ok(
    !css.includes("--as-radius-pill"),
    "no shape of the window is a pill",
  );
});
