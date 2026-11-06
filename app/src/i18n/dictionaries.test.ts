import assert from "node:assert/strict";
import { readFile, readdir } from "node:fs/promises";
import test from "node:test";

import {
  DEFAULT_LOCALE,
  DICTIONARIES,
  SUPPORTED_LOCALES,
  createTranslator,
  localeName,
  shippedLocaleFor,
  type Locale,
} from "./locale.ts";

// The dictionaries follow the locale registry (`locales.json`): one dictionary per shipped locale, none without
// one, every key of the default locale's dictionary translated everywhere.

// The canonical registry: the copy `contracts/` beside `app/`.
const canonicalRegistryUrl = new URL(
  "../../../contracts/locales/registry.json",
  import.meta.url,
);
const translated = SUPPORTED_LOCALES.filter(
  (locale) => locale !== DEFAULT_LOCALE,
);

async function loadDictionary(locale: Locale): Promise<Record<string, string>> {
  const raw = await readFile(
    new URL(`./${locale}.json`, import.meta.url),
    "utf8",
  );
  return JSON.parse(raw) as Record<string, string>;
}

function placeholdersOf(template: string): string[] {
  return [...template.matchAll(/\{(\w+)\}/g)].map((match) => match[1]).sort();
}

test("locales.json is the canonical locale registry", async () => {
  const canonical = JSON.parse(await readFile(canonicalRegistryUrl, "utf8"));
  const shipped = JSON.parse(
    await readFile(new URL("./locales.json", import.meta.url), "utf8"),
  );
  assert.deepEqual(
    shipped,
    canonical,
    "locales.json must stay equal to the canonical locale registry",
  );
  assert.deepEqual(
    SUPPORTED_LOCALES,
    canonical.locales.map((entry: { tag: string }) => entry.tag),
  );
  assert.equal(DEFAULT_LOCALE, canonical.defaultLocale);
});

test("one dictionary per registry locale, and none without one", async () => {
  for (const locale of SUPPORTED_LOCALES) {
    await assert.doesNotReject(
      loadDictionary(locale),
      `missing ${locale}.json`,
    );
    assert.ok(DICTIONARIES[locale], `${locale} is not in DICTIONARIES`);
  }
  assert.deepEqual(
    Object.keys(DICTIONARIES).sort(),
    [...SUPPORTED_LOCALES].sort(),
  );
  const files = (await readdir(new URL("./", import.meta.url)))
    .filter((name) =>
      /^[a-z]{2,3}(?:-[A-Z][a-z]{3})?(?:-[A-Z]{2})?\.json$/.test(name),
    )
    .map((name) => name.replace(/\.json$/, ""))
    .sort();
  assert.deepEqual(files, [...SUPPORTED_LOCALES].sort());
});

test("no translated dictionary defines a key the default one does not have", async () => {
  const base = await loadDictionary(DEFAULT_LOCALE);
  const baseKeys = new Set(Object.keys(base));
  for (const locale of translated) {
    const dictionary = await loadDictionary(locale);
    for (const key of Object.keys(dictionary)) {
      assert.ok(
        baseKeys.has(key),
        `${locale}.json has key "${key}" not present in ${DEFAULT_LOCALE}.json`,
      );
    }
  }
});

test("every default template's placeholders are preserved wherever a translation overrides it", async () => {
  const base = await loadDictionary(DEFAULT_LOCALE);
  for (const locale of translated) {
    const dictionary = await loadDictionary(locale);
    for (const [key, template] of Object.entries(dictionary)) {
      const expected = placeholdersOf(base[key] ?? "");
      const actual = placeholdersOf(template);
      assert.deepEqual(
        actual,
        expected,
        `${locale}.json's "${key}" has placeholders [${actual.join(", ")}] but ${DEFAULT_LOCALE}.json's has [${expected.join(", ")}]`,
      );
    }
  }
});

test("every translated dictionary translates every key (no English fallback left)", async () => {
  const base = await loadDictionary(DEFAULT_LOCALE);
  for (const locale of translated) {
    const dictionary = await loadDictionary(locale);
    const missing = Object.keys(base).filter((key) => !(key in dictionary));
    assert.deepEqual(
      missing,
      [],
      `${locale}.json is missing translations for: ${missing.join(", ")}`,
    );
  }
});

test("a host language finds its shipped locale; any other reads English, explicitly", () => {
  assert.equal(shippedLocaleFor("fr-CA"), "fr");
  assert.equal(shippedLocaleFor("ja"), "ja");
  assert.equal(shippedLocaleFor("de-DE"), "de");
  // The only Portuguese shipped is Brazilian: a bare or European tag reads it.
  assert.equal(shippedLocaleFor("pt-BR"), "pt-BR");
  assert.equal(shippedLocaleFor("pt-PT"), "pt-BR");
  assert.equal(shippedLocaleFor("pt"), "pt-BR");
  assert.equal(shippedLocaleFor("ru"), "ru");
  assert.equal(shippedLocaleFor("xx"), null);
  assert.equal(localeName("ja"), "日本語");
  const english = createTranslator(DEFAULT_LOCALE);
  const unknown = createTranslator("en-XA");
  assert.equal(unknown("settings.title"), english("settings.title"));
});

// The home (« Accueil »): every key it shows, and its French, character for
// character.
const HOME_KEYS = [
  "nav.home",
  "home.connection.connected",
  "home.how.title",
  "home.how.folded",
  "home.how.step1.title",
  "home.how.step1.line",
  "home.how.step2.title",
  "home.how.step2.line",
  "home.how.step3.title",
  "home.how.step3.line",
  "home.how.step4.title",
  "home.how.step4.line",
  "home.how.folder",
  "home.readGuide",
  "home.start.signInFirst",
  "home.resume.title",
  "home.resume.night",
  "home.resume.line",
  "home.missing.logsAdded",
  "space.connected.start",
  "space.intro.sendsAndFinds",
  "space.intro.withoutSpaceShort",
  "space.email.helpShort",
  "space.email.helpMore",
  "common.learnMore",
];

test("the keys of the home exist in the ten languages, and none says « démarrer » in French (« session » is the word used)", async () => {
  for (const locale of SUPPORTED_LOCALES) {
    const dictionary = await loadDictionary(locale);
    for (const key of HOME_KEYS) {
      assert.ok(dictionary[key], `${locale}.json has no "${key}"`);
    }
  }
  const fr = await loadDictionary("fr");
  for (const key of HOME_KEYS) {
    assert.ok(
      !/d[ée]marr/iu.test(fr[key]),
      `fr: ${key} says « démarrer »: ${fr[key]}`,
    );
  }
});

test("the French of the home is the expected one, character for character", async () => {
  const fr = await loadDictionary("fr");
  const expected: Record<string, string> = {
    "app.tagline":
      "Le pont entre vos dossiers d’astrophotographie et vos sessions Astro Portal.",
    "home.how.title": "Comment ça marche",
    "home.how.folded": "quatre pas, de la nuit à l’envoi",
    "home.how.step1.title": "Choisissez une session",
    "home.how.step1.line":
      "Une session de votre espace, ou une nouvelle : Astro Sync crée son dossier.",
    "home.how.step2.title": "Rangez vos fichiers",
    "home.how.step2.line":
      "L’image finale, les poses, la calibration et les journaux, chacun à sa place.",
    "home.how.step3.title": "Vérifiez et choisissez",
    "home.how.step3.line":
      "Astro Sync lit les dossiers et les en-têtes FITS, dit ce qui manque, puis vous cochez les traitements.",
    "home.how.step4.title": "Envoyez",
    "home.how.step4.line":
      "Seuls des résultats partent, après la vérification de leur taille ; les FITS et les journaux restent ici.",
    "home.how.folder": "Tout se passe dans votre dossier de travail : {path}.",
    "home.readGuide": "Lire le guide",
    "home.start.signInFirst":
      "Connectez-vous d’abord : les sessions viennent de votre espace.",
    "home.resume.title": "À reprendre",
    "home.resume.line": "À compléter : {what}",
    "space.connected.start": "Commencer",
    "space.intro.createOrSignIn": "Créer votre espace ou vous connecter",
    "space.intro.sendsAndFinds":
      "Envoyez vos captures et vos journaux de nuit à votre espace, puis retrouvez-les sur le web et sur vos autres appareils.",
    "space.intro.withoutSpaceShort":
      "Sans espace, Astro Sync ne peut ni lister vos sessions ni les envoyer.",
    "space.email.helpShort": "Nous vous envoyons un code à six chiffres.",
    "space.email.send": "Recevoir mon code de connexion",
    "space.ownServer": "J’utilise mon propre serveur",
    "sessions.description":
      "Les sessions de votre espace, des plus récentes aux plus anciennes",
  };
  for (const [key, value] of Object.entries(expected)) {
    // The expected text writes the typographic apostrophe and plain spaces; the
    // dictionary may keep the straight one and writes the no-break space of
    // French typography.
    assert.equal(
      fr[key].replaceAll("'", "’").replaceAll("\u00a0", " "),
      value.replaceAll("'", "’").replaceAll("\u00a0", " "),
      key,
    );
  }
  assert.equal(fr["nav.sessions"], "Sessions");
  // What Astro Sync lists and sends is the session; the night stays the
  // time (« Intégration de la nuit »).
  assert.equal(fr["sessions.title"], "Sessions");
  assert.equal(fr["nights.create"], "Créer une session");
  assert.equal(fr["nights.createForm.submit"], "Créer cette session");
  assert.equal(fr["wizard.summary.title"], "Récapitulatif");
  assert.equal(
    fr["wizard.summary.lead"],
    "Ce qui complétera la session du {date}.",
  );
  assert.equal(fr["wizard.summary.sent"], "Ajouté à la session");
  assert.equal(fr["wizard.sent.backToNights"], "Revenir aux sessions");
  assert.equal(
    fr["wizard.treatments.integration.title"],
    "Intégration de la nuit",
  );
  assert.equal(fr["common.learnMore"], "En savoir plus");
});

test("the window says « vous » everywhere; only the prompt copied for an AI assistant says « tu », and it is built by the core", async () => {
  const fr = await loadDictionary("fr");
  for (const [key, value] of Object.entries(fr)) {
    assert.ok(
      !/(?<!\p{L})(tu|toi|ton|tes)(?!\p{L})/iu.test(value),
      `${key} says « tu »: ${value}`,
    );
  }
  // The Guide says how the prompt starts, in the person's language.
  assert.match(fr["guide.ai.line"], /Parle-moi en français\./u);
});

test("the French of the tabs, the Guide and Réglages is the expected one, and every new key is in the ten languages", async () => {
  const fr = await loadDictionary("fr");
  const expected: Record<string, string> = {
    "nav.logs": "Journaux",
    "nav.annotations": "Annotations",
    "nav.guide": "Guide",
    "progression.formTitle": "Une famille de poses",
    "progression.generateButton": "Générer la progression",
    "progression.skipButton": "Passer pour l’instant",
    "progression.availableTitle": "Progression générée",
    "logs.formTitle": "Une nuit",
    "logs.found": "Journaux trouvés",
    "logs.sent": "Rapport envoyé",
    "annotations.solveAll": "Tout résoudre",
    "guide.tips": "Astuces",
    "guide.table.use": "Ce qu’Astro Sync en fait",
    "settings.tab.general": "Général",
    "settings.tab.dependencies": "Dépendances",
    "deps.lead":
      "Astro Sync n’embarque aucun outil lourd : il vérifie ceux dont il a besoin, et peut les télécharger pour vous.",
    "deps.check": "Vérifier à nouveau",
    "deps.optional": "Facultatif",
    "space.connected.receivesCaptures":
      "Les captures et les mesures de vos sessions.",
  };
  for (const [key, value] of Object.entries(expected)) {
    assert.equal(
      fr[key].replaceAll("'", "’").replaceAll(" ", " "),
      value.replaceAll("'", "’").replaceAll(" ", " "),
      key,
    );
  }
  for (const locale of SUPPORTED_LOCALES) {
    const words = await loadDictionary(locale);
    for (const prefix of [
      "logs.",
      "annotations.",
      "guide.",
      "deps.",
      "tabs.",
      "settings.folder.",
      "settings.utc.",
    ]) {
      const keys = Object.keys(fr).filter((key) => key.startsWith(prefix));
      for (const key of keys) {
        assert.ok(words[key], `${locale}: ${key}`);
      }
    }
  }
});

test("the ten dictionaries name the list of what is sent with the word of the session, in the language's own glossary", async () => {
  const word: Record<string, RegExp> = {
    fr: /^Sessions$/u,
    en: /^Sessions$/u,
    ja: /^セッション$/u,
    de: /^Sitzungen$/u,
    es: /^Sesiones$/u,
    it: /^Sessioni$/u,
    "pt-BR": /^Sessões$/u,
    ru: /^Сеансы$/u,
    "zh-Hans": /^观测记录$/u,
    "zh-Hant": /^觀測記錄$/u,
  };
  for (const locale of SUPPORTED_LOCALES) {
    const dictionary = await loadDictionary(locale);
    for (const key of ["nav.sessions", "sessions.title"]) {
      assert.match(dictionary[key], word[locale], `${locale}: ${key}`);
    }
    assert.ok(
      dictionary["wizard.summary.sent"],
      `${locale}: wizard.summary.sent`,
    );
    assert.notEqual(
      dictionary["wizard.summary.sent"],
      dictionary["wizard.summary.toSend"],
      `${locale}: « added to the session » is not « to send to your space »`,
    );
  }
});

test("the two refusals of a night's links (Windows without developer mode, F1) are said in the ten languages, and the French is the reference", async () => {
  const fr = await loadDictionary("fr");
  assert.equal(
    fr["errors.symlink_unavailable"],
    "Sous Windows, les liens demandent le mode développeur. Astro Sync n’a pas pu préparer le dossier de la nuit.",
  );
  assert.equal(
    fr["errors.link_across_volumes"],
    "Les poses et le dossier de travail sont sur deux lecteurs différents\u00a0: Astro Sync ne peut pas les relier sans mode développeur. Placez le dossier de travail sur le même lecteur que les poses, ou activez le mode développeur de Windows.",
  );
  for (const locale of SUPPORTED_LOCALES) {
    const dictionary = await loadDictionary(locale);
    for (const key of [
      "errors.symlink_unavailable",
      "errors.link_across_volumes",
    ]) {
      assert.ok(dictionary[key]?.trim(), `${locale}: ${key}`);
    }
  }
});

test("a dictionary writes its apostrophes one way: the typographic one, never the straight one", async () => {
  for (const locale of SUPPORTED_LOCALES) {
    const dictionary = await loadDictionary(locale);
    const straight = Object.entries(dictionary)
      .filter(([, text]) => text.includes("'"))
      .map(([key]) => key);
    assert.deepEqual(straight, [], `${locale}: straight apostrophe`);
  }
});

test("the English dictionary names the offer in English, as the phone does", async () => {
  const en = await loadDictionary("en");
  assert.equal(
    en["logs.reserved"],
    "Sending the logs is reserved to Advanced Sync+.",
  );
  const french = Object.entries(en)
    .filter(([, text]) => text.includes("Synchronisation avancée"))
    .map(([key]) => key);
  assert.deepEqual(french, []);
});

test("what the command line says in each language (tables, states, waits) is in the ten dictionaries with the same blanks", async () => {
  const en = await loadDictionary("en");
  const keys = Object.keys(en).filter((key) => key.startsWith("cli."));
  assert.ok(keys.length >= 25, `${keys.length} command-line sentences`);
  for (const locale of translated) {
    const dictionary = await loadDictionary(locale);
    for (const key of keys) {
      assert.ok(dictionary[key]?.trim(), `${locale}: ${key}`);
      assert.deepEqual(
        placeholdersOf(dictionary[key]),
        placeholdersOf(en[key]),
        `${locale}: ${key}`,
      );
    }
  }
});
