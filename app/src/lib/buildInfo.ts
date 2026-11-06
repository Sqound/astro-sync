// The version of the build: every review `.dmg` carries the same version
// (`1.0.0`), so the window says which commit of `main` it was built from --
// the one in the name of the file -- in the menu (« Astro Sync 1.0.0 ·
// 6aa28f767 », with the date on hover) and in Réglages › Général (« Version
// 1.0.0 · 6aa28f767 · construite le 13 oct. 2026 »). The commit and the date
// are stamped at build time (`app_build_info`), never read from git at run
// time. Pure TypeScript, tested under `node --test`.

import type { TranslateParams } from "../i18n/locale.ts";
import type { BuildInfo } from "./bridge";

type Translate = (key: string, params?: TranslateParams) => string;

/** The separator of the parts of a line: a middle dot between spaces. */
const SEPARATOR = " · ";

/** The commit of the build, or `null` when the build was made outside a
 * repository (then only the version is said). */
export function shownSha(info: BuildInfo): string | null {
  return info.gitSha && info.gitSha !== "unknown" ? info.gitSha : null;
}

/** The build date as people write it in `locale` (« 13 oct. 2026 »), read in
 * UTC like the day `astro-sync --version` prints; `null` when unknown. */
export function buildDateText(info: BuildInfo, locale: string): string | null {
  const instant = Date.parse(info.buildDate);
  if (Number.isNaN(instant)) return null;
  return new Intl.DateTimeFormat(locale, {
    day: "numeric",
    month: "short",
    year: "numeric",
    timeZone: "UTC",
  }).format(new Date(instant));
}

/** The line at the foot of the menu: « Astro Sync 1.0.0 · 6aa28f767 ». */
export function menuVersionLine(appTitle: string, info: BuildInfo): string {
  const sha = shownSha(info);
  return `${appTitle} ${info.version}${sha ? `${SEPARATOR}${sha}` : ""}`;
}

/** What the line of the menu says on hover: « Construite le 13 oct. 2026 »;
 * `undefined` (no tooltip) when the date is unknown. */
export function menuVersionTitle(
  t: Translate,
  info: BuildInfo,
  locale: string,
): string | undefined {
  const date = buildDateText(info, locale);
  return date === null ? undefined : t("nav.buildTitle", { date });
}

/** The line of Réglages › Général: « Version 1.0.0 · 6aa28f767 · construite
 * le 13 oct. 2026 ». */
export function settingsVersionLine(
  t: Translate,
  info: BuildInfo,
  locale: string,
): string {
  const parts = [t("settings.version.version", { version: info.version })];
  const sha = shownSha(info);
  if (sha) parts.push(sha);
  const date = buildDateText(info, locale);
  if (date !== null) parts.push(t("settings.version.builtOn", { date }));
  return parts.join(SEPARATOR);
}
