// Typed class-name helpers over `primitives.css`. Keeping these names in one
// typed place (rather than inline string literals scattered across
// components) is what makes the primitive set "shared": every consumer gets
// the same panel/button/field/badge/progress/confirm markup.

export const panelClassName = "as-panel";
export const fieldClassName = "as-field";

/** The four levels of an action, from the weakest to the strongest.
 * A level is chosen by the role of the action, not by the look wanted. */
export type ButtonLevel = "link" | "tertiary" | "secondary" | "primary";

export interface ButtonOptions {
  /** The destructive tint: a colour, not a level -- the action keeps the
   * level its role gives it. */
  destructive?: boolean;
  /** On a line of its own, the whole width of its card. */
  block?: boolean;
}

export function buttonClassName(
  level: ButtonLevel = "primary",
  options: ButtonOptions = {},
): string {
  return [
    "as-button",
    `as-button--${level}`,
    options.destructive ? "is-destructive" : "",
    options.block ? "as-button--block" : "",
  ]
    .filter(Boolean)
    .join(" ");
}

/** The step that waits: a button that is not an action yet. */
export const inactiveButtonClassName = "as-button as-button--inactive";

export type BadgeTone =
  | "success"
  | "warning"
  | "error"
  | "neutral"
  // A small label with a contour and no fill, white bold
  // text (« Plan prévu », « Existe déjà », « Exécutée ») or gold (the tag of
  // a multi-night plan).
  | "outline"
  | "outline-gold";

export function badgeClassName(tone: BadgeTone): string {
  return `as-badge as-badge--${tone}`;
}

/** A tinted block that says something (never a solid one). */
export function noticeClassName(tone: BadgeTone): string {
  return `as-notice as-notice--${tone}`;
}

export const progressClassName = {
  track: "as-progress",
  fill: "as-progress__fill",
};

export function confirmClassName(destructive: boolean): string {
  return destructive ? "as-confirm as-confirm--destructive" : "as-confirm";
}

// The application screens add a handful of primitives the first scaffold never
// needed: an app shell (sidebar + content), a tab list, a data table, a labeled
// field group, and a toolbar row. Same rule as every primitive above:
// `primitives.css` only ever references `var(--as-*)`.

export const appShellClassName = "as-app-shell";
export const sidebarClassName = "as-sidebar";
export const contentClassName = "as-content";

export const tabListClassName = "as-tabs";

/** A group of the menu (« Accueil » alone, or a titled section), its title
 * (not interactive: the gold of the theme at 70 % on the panel) and an entry
 * (`nested`: indented under its title). */
export const tabGroupClassName = "as-tabs__group";
export const tabSectionTitleClassName =
  "as-tabs__title as-tabs__title--gold-70";

export function tabClassName(active: boolean, nested = false): string {
  const classes = ["as-tab"];
  if (nested) classes.push("as-tab--nested");
  if (active) classes.push("as-tab--active");
  return classes.join(" ");
}

export const tableClassName = "as-table";
/** Wrap every `<table className={tableClassName}>` in an element with this
 * class: a compact window narrower than the table's natural width scrolls
 * the table horizontally instead of silently clipping its trailing
 * columns off-screen with no way to reach them. */
export const tableWrapperClassName = "as-table-wrapper";
export const toolbarClassName = "as-toolbar";
export const fieldGroupClassName = "as-field-group";
export const fieldLabelClassName = "as-field-label";
export const helpTextClassName = "as-help-text";

/** A link inside a sentence (the terms, the privacy policy): a button that
 * reads as text, since it opens the system browser rather than navigating. */
export const linkClassName = "as-link";

// The home (« Accueil »): a title, the connection card, the cards of the
// three steps, and the list of the other places.
export const homeClassName = "as-home";
export const homeHeaderClassName = "as-home__header";
export const homeTitleClassName = "as-home__title";
export const homeMarkClassName = "as-home__mark";
export const leadClassName = "as-lead";
export const sectionTitleClassName = "as-section-title";
export const pageTitleClassName = "as-page-title";
export const stackClassName = "as-stack";
export const iconClassName = "as-icon";

export type CardTone = "plain" | "connected" | "current" | "waiting";

export function cardClassName(tone: CardTone = "plain"): string {
  return tone === "plain" ? "as-card" : `as-card as-card--${tone}`;
}

export const cardAvatarClassName = "as-card__avatar";
export const cardBodyClassName = "as-card__body";
export const cardTitleClassName = "as-card__title";
export const cardHeadingClassName = "as-card__heading";
export const cardTextClassName = "as-card__text";
export const cardSubClassName = "as-card__sub";

export type StepNumberTone = "done" | "current" | "waiting";

export function stepNumberClassName(tone: StepNumberTone): string {
  return `as-step__number as-step__number--${tone}`;
}

export const stepHeadClassName = "as-step__head";
export const stepTextClassName = "as-step__text";
export const stepTitleClassName = "as-step__title";
export const stepLineClassName = "as-step__line";
export const stepProgressClassName = "as-step__progress";
export const stepActionsClassName = "as-step__actions";

export const listClassName = "as-list";
export const rowClassName = "as-row";
export const rowIconClassName = "as-row__icon";
export const rowTextClassName = "as-row__text";
export const rowTitleClassName = "as-row__title";
export const rowLineClassName = "as-row__line";
export const rowChevronClassName = "as-row__chevron";

export const chipClassName = "as-chip";
export const infoLineClassName = "as-info-line";
export const infoButtonClassName = "as-info-button";
