// The React half of the i18n engine: a context provider around
// `./locale.ts`'s pure lookup/interpolation logic. See that module's doc
// comment for why the two are split (JSX cannot be type-stripped by `node
// --test`, so the dictionary tests import `./locale.ts` directly).
import {
  createContext,
  useContext,
  useEffect,
  useMemo,
  useState,
  type ReactNode,
} from "react";

import { platformFontFaces, themeFontFamily } from "../theme/tokens";
import { createTranslator, type Locale, type Translate } from "./locale";

export {
  DEFAULT_LOCALE,
  SUPPORTED_LOCALES,
  detectSystemLocale,
  isLocale,
  localeName,
  type Dictionary,
  type Locale,
  type Translate,
  type TranslateParams,
} from "./locale";

interface I18nContextValue {
  locale: Locale;
  setLocale: (locale: Locale) => void;
  t: Translate;
}

const I18nContext = createContext<I18nContextValue | null>(null);

export function I18nProvider({
  locale,
  onLocaleChange,
  children,
}: {
  /** The locale to render with -- the caller (App.tsx) owns whether this
   * comes from saved settings or `detectSystemLocale()`, so this provider
   * never reads storage itself. */
  locale: Locale;
  onLocaleChange: (locale: Locale) => void;
  children: ReactNode;
}) {
  const [current, setCurrent] = useState(locale);

  useEffect(() => {
    setCurrent(locale);
  }, [locale]);

  // The document states its language, so the browser engine picks the right
  // script's faces and glyph forms (Simplified or Traditional Chinese).
  // Chinese reads in its own script's faces first: the base stack names
  // Japanese faces, which draw Han characters in their Japanese forms.
  useEffect(() => {
    const root = document.documentElement;
    root.lang = current;
    const chinese: Record<string, string> = {
      "zh-Hans": '"PingFang SC", "Microsoft YaHei"',
      "zh-Hant": '"PingFang TC", "Microsoft JhengHei"',
    };
    const base = platformFontFaces(navigator.userAgent) + themeFontFamily();
    root.style.setProperty(
      "--as-font-family",
      chinese[current] ? `${chinese[current]}, ${base}` : base,
    );
  }, [current]);

  const value = useMemo<I18nContextValue>(
    () => ({
      locale: current,
      setLocale: (next) => {
        setCurrent(next);
        onLocaleChange(next);
      },
      t: createTranslator(current),
    }),
    [current, onLocaleChange],
  );

  return <I18nContext.Provider value={value}>{children}</I18nContext.Provider>;
}

export function useI18n(): I18nContextValue {
  const context = useContext(I18nContext);
  if (!context) {
    throw new Error("useI18n must be used within an I18nProvider");
  }
  return context;
}
