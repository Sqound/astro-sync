// Typed loader for the platform-neutral design tokens in `./tokens.json` --
// never an independent list of raw colors.
//
// Every CSS custom property Astro Sync uses is derived from these values at
// runtime by `applyTheme`/`themeCssVariables` below; `primitives.css` only
// ever references `var(--as-*)`, never a color literal.

import tokensJson from "./tokens.json" with { type: "json" };

// `type` aliases (not `interface`s) on purpose: the color/radii/spacing
// shapes below are passed to `prefixedVariables`'s `Record<string, string>`
// parameter, and only object-literal type aliases -- not interfaces -- are
// structurally assignable to an index-signature type in strict mode.
export type ThemeColorTokens = {
  pageBackground: string;
  primaryText: string;
  mutedText: string;
  panel: string;
  border: string;
  goldAccent: string;
  activeGoldText: string;
  activeSurface: string;
  focus: string;
  success: string;
  warning: string;
  error: string;
  modalSurface: string;
  modalBorder: string;
  modalScrim: string;
  // The four button levels and the inactive look: `primitives.css` reads them as
  // `--as-color-primary-edge`, `--as-color-tertiary-border`, ...
  primaryEdge: string;
  primaryInk: string;
  tertiaryBorder: string;
  destructiveSurface: string;
  destructiveEdge: string;
  destructiveInk: string;
  destructiveOutline: string;
  destructiveText: string;
  inactiveSurface: string;
  inactiveText: string;
};

export type ThemeTypographyTokens = {
  fontFamily: string[];
};

export type ThemeRadiiTokens = {
  sm: string;
  md: string;
  lg: string;
  pill: string;
};

export type ThemeSpacingTokens = {
  xs: string;
  sm: string;
  md: string;
  lg: string;
  xl: string;
};

export type ThemeTokens = {
  colors: ThemeColorTokens;
  typography: ThemeTypographyTokens;
  radii: ThemeRadiiTokens;
  spacing: ThemeSpacingTokens;
};

export const tokens: ThemeTokens = tokensJson as ThemeTokens;

/** `pageBackground` -> `page-background`, `goldAccent` -> `gold-accent`. */
function camelToKebab(value: string): string {
  return value.replace(/([a-z0-9])([A-Z])/g, "$1-$2").toLowerCase();
}

function prefixedVariables(
  prefix: string,
  values: Record<string, string>,
): Record<string, string> {
  const variables: Record<string, string> = {};
  for (const [role, value] of Object.entries(values)) {
    variables[`--as-${prefix}-${camelToKebab(role)}`] = value;
  }
  return variables;
}

export function themeColorVariables(
  colors: ThemeColorTokens = tokens.colors,
): Record<string, string> {
  return prefixedVariables("color", colors);
}

export function themeRadiusVariables(
  radii: ThemeRadiiTokens = tokens.radii,
): Record<string, string> {
  return prefixedVariables("radius", radii);
}

export function themeSpacingVariables(
  spacing: ThemeSpacingTokens = tokens.spacing,
): Record<string, string> {
  return prefixedVariables("space", spacing);
}

/**
 * The faces named first on Windows. The shared stack (`tokens.json`) names
 * macOS faces, then Yu Gothic: on Windows that draws every Latin and Cyrillic
 * text in a Japanese face (Russian letters spread out and overflow their
 * boxes, and a backslash shows as a yen sign in a folder path). Segoe UI
 * covers Latin, Greek and Cyrillic; the Japanese and Chinese faces that
 * follow still draw what it lacks. Nothing on the other systems.
 */
export function platformFontFaces(userAgent: string): string {
  return userAgent.includes("Windows")
    ? '"Segoe UI Variable Text", "Segoe UI", '
    : "";
}

export function themeFontFamily(
  typography: ThemeTypographyTokens = tokens.typography,
): string {
  return typography.fontFamily
    .map((name) => (name.includes(" ") ? `"${name}"` : name))
    .join(", ");
}

/** Every CSS custom property `primitives.css` relies on, in one map. */
export function themeCssVariables(
  source: ThemeTokens = tokens,
): Record<string, string> {
  return {
    ...themeColorVariables(source.colors),
    ...themeRadiusVariables(source.radii),
    ...themeSpacingVariables(source.spacing),
    "--as-font-family": themeFontFamily(source.typography),
  };
}

/** The minimal shape `applyTheme` needs -- satisfied by any DOM element. */
export interface ThemeVariableTarget {
  style: { setProperty(name: string, value: string): void };
}

/**
 * Sets every derived CSS custom property on `target` (typically
 * `document.documentElement`). Takes the target explicitly, and never reads
 * `document` itself, so this module stays importable and testable outside a
 * browser (see `tokens.test.ts`).
 */
export function applyTheme(
  target: ThemeVariableTarget,
  source: ThemeTokens = tokens,
): void {
  for (const [name, value] of Object.entries(themeCssVariables(source))) {
    target.style.setProperty(name, value);
  }
}
