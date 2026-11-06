import { matchDeviceLocaleTag } from "../vendor/device-locale.ts";
// The non-JSX half of the i18n engine: locale detection, the flat
// dictionaries, and `{placeholder}` interpolation. Kept separate from
// `index.tsx`'s `I18nProvider`/`useI18n` (which do touch JSX) so
// `dictionaries.test.ts` can import this module directly under `node
// --test` -- Node's built-in TypeScript type-stripping handles a plain
// `.ts` file's type annotations, but cannot parse JSX, so a test must
// never import anything from a `.tsx` file.
import en from "./en.json" with { type: "json" };
import fr from "./fr.json" with { type: "json" };
import ja from "./ja.json" with { type: "json" };
import de from "./de.json" with { type: "json" };
import es from "./es.json" with { type: "json" };
import it from "./it.json" with { type: "json" };
import ptBR from "./pt-BR.json" with { type: "json" };
import zhHans from "./zh-Hans.json" with { type: "json" };
import zhHant from "./zh-Hant.json" with { type: "json" };
import ru from "./ru.json" with { type: "json" };
// The locale registry: the one list of the shipped languages, their
// own-language names and the default one.
import registry from "./locales.json" with { type: "json" };

/** A shipped locale's tag (`en`, `zh-Hans`): one of {@link SUPPORTED_LOCALES}. */
export type Locale = string;

/** The shipped locales, in the registry's order. */
export const SUPPORTED_LOCALES: readonly Locale[] = registry.locales.map(
  (entry) => entry.tag,
);

/** The locale every missing word falls back to. */
export const DEFAULT_LOCALE: Locale = registry.defaultLocale;

export type Dictionary = Record<string, string>;

/**
 * One dictionary per catalogue file (`<tag>.json`). `dictionaries.test.ts`
 * fails when a registry locale has no dictionary here, or the reverse.
 */
export const DICTIONARIES: Readonly<Record<Locale, Dictionary>> = {
  en,
  fr,
  ja,
  de,
  es,
  it,
  "pt-BR": ptBR,
  "zh-Hans": zhHans,
  "zh-Hant": zhHant,
  ru,
};

export function isLocale(value: string): value is Locale {
  return (SUPPORTED_LOCALES as readonly string[]).includes(value);
}

/** The language's own name (`Français`), as the registry writes it. */
export function localeName(locale: Locale): string {
  return registry.locales.find((entry) => entry.tag === locale)?.name ?? locale;
}

/** The registry's entries as {@link shippedLocaleFor} reads them. */

/**
 * The shipped locale a host language tag (`fr-CA`, `zh-TW`, `pt-PT`) asks
 * for, by the registry's rule: the same tag; else, among the
 * locales of its language, the one of its script (`zh-Hant-TW`), the one that
 * lists its region (`zh-TW` reads `zh-Hant`), the bare language's own locale
 * (`fr-CA` reads `fr`), the one flagged `deviceDefault` (`zh` reads
 * `zh-Hans`), else the language's first locale; else none.
 */
export function shippedLocaleFor(candidate: string): Locale | null {
  return matchDeviceLocaleTag(candidate, registry.locales);
}

/** Reads the host OS/browser language list and returns the first supported
 * locale it names, or the default locale (English) if none matches. Used when Settings has no
 * saved `locale` override yet. */
export function detectSystemLocale(): Locale {
  const candidates =
    typeof navigator !== "undefined" && navigator.languages?.length
      ? navigator.languages
      : [typeof navigator !== "undefined" ? navigator.language : "en"];
  for (const candidate of candidates) {
    const locale = candidate ? shippedLocaleFor(candidate) : null;
    if (locale) {
      return locale;
    }
  }
  return DEFAULT_LOCALE;
}

export type TranslateParams = Record<string, string | number>;

export function interpolate(
  template: string,
  params?: TranslateParams,
): string {
  if (!params) {
    return template;
  }
  return template.replace(/\{(\w+)\}/g, (placeholder, name: string) =>
    name in params ? String(params[name]) : placeholder,
  );
}

export type Translate = (key: string, params?: TranslateParams) => string;

/** Builds a `t()` function for `locale`, falling back to the default
 * locale (English) for a locale without a dictionary, and for any key that
 * locale's dictionary does not (yet) define. */
export function createTranslator(locale: Locale): Translate {
  const fallback = DICTIONARIES[DEFAULT_LOCALE] ?? {};
  const dictionary = DICTIONARIES[locale] ?? fallback;
  return (key, params) =>
    interpolate(dictionary[key] ?? fallback[key] ?? key, params);
}
