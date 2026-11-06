// How Astro Sync writes a date, an hour and a byte count -- pure TypeScript
// (no React, no Tauri), so it runs under `node --test` and every screen says
// the same thing the same way. An hour is « 23:40 » on 24 hours in every
// language, a date is written as people read it in their language and never
// as the ISO text the service sends (« 2026-09-16T20:05:00Z »).

import type { TranslateParams } from "../i18n/locale.ts";

type Translate = (key: string, params?: TranslateParams) => string;

/** « mer. 16 sept., 23:40 » -- the weekday, the day, the month and the hour of
 * the instant, in the computer's time zone (or `timeZone`) and in the
 * language of the window. An instant that cannot be read gives « — ». */
export function formatDateTime(
  instant: string | number | Date | null | undefined,
  locale: string,
  timeZone?: string,
): string {
  const date = toDate(instant);
  if (date === null) {
    return "—";
  }
  return new Intl.DateTimeFormat(locale, {
    weekday: "short",
    day: "numeric",
    month: "short",
    hour: "2-digit",
    minute: "2-digit",
    hourCycle: "h23",
    ...(timeZone ? { timeZone } : {}),
  }).format(date);
}

/** « mer. 16 sept. » -- the same, without the hour: how a night is named. */
export function formatDay(
  instant: string | number | Date | null | undefined,
  locale: string,
  timeZone?: string,
): string {
  const date = toDate(instant);
  if (date === null) {
    return "—";
  }
  return new Intl.DateTimeFormat(locale, {
    weekday: "short",
    day: "numeric",
    month: "short",
    ...(timeZone ? { timeZone } : {}),
  }).format(date);
}

function toDate(
  instant: string | number | Date | null | undefined,
): Date | null {
  if (instant === null || instant === undefined || instant === "") {
    return null;
  }
  const date = instant instanceof Date ? instant : new Date(instant);
  return Number.isNaN(date.getTime()) ? null : date;
}

/** A number with one decimal, in the decimal mark of the language (« 4,1 »,
 * « 4.1 »). */
export function oneDecimal(value: number, locale: string): string {
  return new Intl.NumberFormat(locale, {
    minimumFractionDigits: 1,
    maximumFractionDigits: 1,
  }).format(value);
}

/** Renders a byte count in the largest unit that keeps it readable, through
 * the locale's own `common.*` templates, with one decimal in the language's
 * own decimal mark (« 4,1 Go », « 15,6 Go »). */
export function formatBytes(
  t: Translate,
  bytes: number,
  locale: string,
): string {
  if (bytes >= 1_000_000_000) {
    return t("common.gigabytes", {
      value: oneDecimal(bytes / 1_000_000_000, locale),
    });
  }
  if (bytes >= 1_000_000) {
    return t("common.megabytes", {
      value: oneDecimal(bytes / 1_000_000, locale),
    });
  }
  if (bytes >= 1_000) {
    return t("common.kilobytes", { value: oneDecimal(bytes / 1_000, locale) });
  }
  return t("common.bytes", { value: bytes });
}

/** The sentence of `base` for `count`: `base.one` where the language says
 * « un » (« 1 fichier », « 0 fichier » in French), `base.other` everywhere
 * else. A language whose count needs no declension (Russian, Japanese)
 * writes the same sentence under both. */
export function pluralText(
  t: Translate,
  locale: string,
  base: string,
  count: number,
  params: TranslateParams = {},
): string {
  const one = new Intl.PluralRules(locale).select(count) === "one";
  return t(`${base}.${one ? "one" : "other"}`, { count, ...params });
}

/** A size as the screens write it: the largest readable unit, one decimal only
 * when it says something (« 412 Mo », « 1,3 Go », « 250 Go », « 38 ko »). */
export function formatSize(
  t: Translate,
  bytes: number,
  locale: string,
): string {
  // One decimal, never rounded up (« 230,9 Go » left, not « 231 »), and
  // none when it is zero (« 412 Mo »).
  const value = (amount: number) =>
    new Intl.NumberFormat(locale, {
      minimumFractionDigits: 0,
      maximumFractionDigits: 1,
    }).format(Math.floor(amount * 10 + 1e-6) / 10);
  if (bytes >= 1_000_000_000) {
    return t("common.gigabytes", { value: value(bytes / 1_000_000_000) });
  }
  if (bytes >= 1_000_000) {
    return t("common.megabytes", { value: value(bytes / 1_000_000) });
  }
  if (bytes >= 1_000) {
    return t("common.kilobytes", { value: value(bytes / 1_000) });
  }
  return t("common.bytes", { value: bytes });
}

/** « 2 h 20 », « 45 min », « 3 h »: an integration in minutes. */
export function formatMinutes(t: Translate, minutes: number): string {
  const total = Math.max(0, Math.round(minutes));
  const hours = Math.floor(total / 60);
  const rest = total % 60;
  if (hours === 0) return t("common.minutesShort", { count: rest });
  if (rest === 0) return t("common.hoursShort", { count: hours });
  return t("common.hoursMinutes", {
    hours,
    minutes: String(rest).padStart(2, "0"),
  });
}

/** « 3f9a…c41e »: the first and last four characters of a SHA-256. */
export function shortDigest(sha256: string): string {
  const hex = sha256.replace(/^sha256:/, "");
  return hex.length <= 8 ? hex : `${hex.slice(0, 4)}…${hex.slice(-4)}`;
}

/** « Siril et l'environnement Python »: a list in the words of `locale`
 * (`Intl.ListFormat`, known to every engine the window runs in). */
export function conjunction(locale: string, items: readonly string[]): string {
  const { ListFormat } = Intl as unknown as {
    ListFormat: new (
      locale: string,
      options: { type: "conjunction" },
    ) => { format(items: readonly string[]): string };
  };
  return new ListFormat(locale, { type: "conjunction" }).format(items);
}
