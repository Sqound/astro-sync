// The Windows installer (NSIS, made by the Tauri bundler) speaks the
// application's languages: it opens in the language of Windows when it is one
// of them, offers the choice in a first dialog, and its uninstaller reuses
// the choice (kept by NSIS under the product's key).
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const conf = JSON.parse(
  readFileSync(join(here, "../../src-tauri/tauri.conf.json"), "utf8"),
);
const locales = JSON.parse(
  readFileSync(join(here, "../i18n/locales.json"), "utf8"),
) as { locales: { tag: string }[] };

/** The NSIS language of each locale of the application. */
const NSIS_LANGUAGE: Record<string, string> = {
  en: "English",
  fr: "French",
  de: "German",
  it: "Italian",
  es: "Spanish",
  "pt-BR": "PortugueseBR",
  ru: "Russian",
  ja: "Japanese",
  "zh-Hans": "SimpChinese",
  "zh-Hant": "TradChinese",
};

const nsis = conf.bundle.windows.nsis as {
  languages: string[];
  displayLanguageSelector: boolean;
};

test("the installer carries every language of the application, English first", () => {
  const expected = locales.locales.map((locale) => NSIS_LANGUAGE[locale.tag]);
  assert.ok(!expected.includes(undefined as unknown as string));
  assert.deepEqual([...nsis.languages].sort(), [...expected].sort());
  // The fallback when Windows speaks none of them.
  assert.equal(nsis.languages[0], "English");
});

test("the installer offers the choice of its language", () => {
  // The dialog preselects the language of Windows (MUI_LANGDLL_DISPLAY).
  assert.equal(nsis.displayLanguageSelector, true);
});

const custom = conf.bundle.windows.nsis.customLanguageFiles as Record<
  string,
  string
>;
const OWN_KEYS = ["appRunning", "appRunningOkKill", "failedToKillApp"];

function langStrings(file: string): Map<string, string> {
  const source = readFileSync(join(here, "../../src-tauri", file), "utf8");
  const strings = new Map<string, string>();
  for (const [, key, text] of source.matchAll(
    /^LangString (\w+) \$\{LANG_\w+\} "(.*)"$/gm,
  )) {
    strings.set(key, text);
  }
  return strings;
}

test("every language of the installer has its own file of the bundler's messages", () => {
  // A custom file replaces the bundler's whole file: it must hold every key
  // the English one holds, or NSIS shows an empty message.
  assert.deepEqual(Object.keys(custom).sort(), [...nsis.languages].sort());
  const english = [...langStrings(custom.English).keys()].sort();
  for (const key of OWN_KEYS) assert.ok(english.includes(key), key);
  for (const [language, file] of Object.entries(custom)) {
    assert.equal(file, `nsis/${language}.nsh`);
    assert.deepEqual(
      [...langStrings(file).keys()].sort(),
      english,
      `${language}: the same keys as English`,
    );
  }
});

test("an open Astro Sync is closed with the application's words, never « Click OK to kill it »", () => {
  for (const [language, file] of Object.entries(custom)) {
    const strings = langStrings(file);
    for (const key of OWN_KEYS) {
      const text = strings.get(key) ?? "";
      // The bundler replaces the placeholder by the product name.
      assert.ok(text.includes("{{product_name}}"), `${language} ${key}`);
      assert.ok(!/kill|!/i.test(text), `${language} ${key}: ${text}`);
    }
  }
  assert.equal(
    langStrings(custom.French).get("appRunningOkKill"),
    "{{product_name}} est ouvert : il sera fermé pour continuer.",
  );
  assert.equal(
    langStrings(custom.English).get("appRunningOkKill"),
    "{{product_name}} is open: it will be closed to continue.",
  );
});
