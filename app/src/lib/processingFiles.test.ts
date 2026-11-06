import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

import { SUPPORTED_LOCALES, createTranslator } from "../i18n/locale.ts";
import type { ProcessingScan } from "./bridge";
import {
  cleanBoxes,
  defaultTicks,
  filesView,
  holdsText,
  kindsOf,
  processingSize,
  producedBytes,
  stampHour,
  tickedBytes,
} from "./processingFiles.ts";

const fr = createTranslator("fr");
const en = createTranslator("en");
const plain = (text: string) =>
  text.replaceAll("\u00a0", " ").replaceAll("’", "'");
const same = (actual: string, expected: string) =>
  assert.equal(plain(actual), plain(expected));

const MB = 1_000_000;

/** A sample machine (figures are illustrative): 168 Mo of masters, 546 Mo of
 * night results (5 nights sent: 346 Mo, 2 not sent: 200 Mo), 225 Mo of plan
 * renders (1 plan, sent), 41 Mo of temporaries, the person's own 220 Mo. */
const scan: ProcessingScan = {
  reachable: true,
  root: "/data/astro/AstroSync",
  computedAt: "2026-10-23T18:52:00",
  folderBytes: 1_200 * MB,
  userBytes: 220 * MB,
  pythonEnvBytes: 308 * MB,
  categories: [
    {
      kind: "masters",
      bytes: 168 * MB,
      count: 9,
      sentBytes: 0,
      unsentBytes: 0,
      sentCount: 0,
      unsentCount: 0,
    },
    {
      kind: "nightResults",
      bytes: 546 * MB,
      count: 7,
      sentBytes: 346 * MB,
      unsentBytes: 200 * MB,
      sentCount: 5,
      unsentCount: 2,
    },
    {
      kind: "planRenders",
      bytes: 225 * MB,
      count: 1,
      sentBytes: 225 * MB,
      unsentBytes: 0,
      sentCount: 1,
      unsentCount: 0,
    },
    {
      kind: "temp",
      bytes: 41 * MB,
      count: 1,
      sentBytes: 0,
      unsentBytes: 0,
      sentCount: 0,
      unsentCount: 0,
    },
  ],
};

test("sizes: whole megabytes, one decimal for gigabytes, a small file is never « 0 Mo », nothing measured is « — »", () => {
  same(processingSize(fr, 168 * MB, "fr"), "168 Mo");
  same(processingSize(fr, 168_400_000, "fr"), "168 Mo");
  same(processingSize(fr, 1_000 * MB, "fr"), "1,0 Go");
  same(processingSize(en, 1_234 * MB, "en"), "1.2 GB");
  same(processingSize(fr, 0, "fr"), "0 Mo");
  same(processingSize(fr, 38_000, "fr"), "38 ko");
  assert.equal(processingSize(fr, null, "fr"), "—");
});

test("the hour of the calculation is on 24 hours", () => {
  assert.equal(stampHour("2026-10-23T18:52:00", "fr"), "18:52");
  assert.equal(stampHour("2026-10-23T08:05:00", "en"), "08:05");
  assert.equal(stampHour("nonsense", "fr"), "—");
});

test("a line per category, the total, the person's own files and the Python environment apart", () => {
  const view = filesView(fr, "fr", scan);
  assert.deepEqual(
    view.rows.map((row) => [row.kind, row.bytes]),
    [
      ["masters", 168 * MB],
      ["nightResults", 546 * MB],
      ["planRenders", 225 * MB],
      ["temp", 41 * MB],
    ],
  );
  assert.equal(producedBytes(scan), 980 * MB);
  assert.equal(view.total.bytes, 980 * MB);
  same(view.total.note ?? "", "sur 1,2 Go dans le dossier de travail");
  assert.equal(view.user.bytes, 220 * MB);
  assert.equal(view.python.bytes, 308 * MB);
  same(view.stamp ?? "", "Calculé à 18:52");
  assert.equal(view.cleanable, true);
  same(view.rows[1].note ?? "", "stacked/ de 7 nuits");
  same(view.rows[2].note ?? "", "1 plan");
});

test("nothing produced is « rien pour l’instant », « Nettoyer… » is inactive, the person's files stay said", () => {
  const empty: ProcessingScan = {
    ...scan,
    folderBytes: 220 * MB,
    categories: scan.categories.map((category) => ({
      ...category,
      bytes: 0,
      count: 0,
      sentBytes: 0,
      unsentBytes: 0,
      sentCount: 0,
      unsentCount: 0,
    })),
  };
  const view = filesView(fr, "fr", empty);
  assert.equal(view.cleanable, false);
  same(view.rows[0].note ?? "", "rien pour l’instant");
  same(view.rows[3].note ?? "", "aucun empilement interrompu");
  assert.equal(view.user.bytes, 220 * MB);
});

test("the folder unreachable gives « — » everywhere, no zero, the Python environment stays", () => {
  const out: ProcessingScan = {
    ...scan,
    reachable: false,
    folderBytes: 0,
    userBytes: 0,
    categories: scan.categories.map((category) => ({
      ...category,
      bytes: 0,
      count: 0,
    })),
  };
  const view = filesView(fr, "fr", out);
  assert.equal(view.reachable, false);
  assert.ok(view.rows.every((row) => row.bytes === null && row.note === null));
  assert.equal(view.total.bytes, null);
  assert.equal(view.user.bytes, null);
  assert.equal(view.stamp, null);
  assert.equal(view.python.bytes, 308 * MB);
  // What an older build left in Astro Sync's own data is still said.
  const legacy = filesView(fr, "fr", {
    ...out,
    categories: out.categories.map((category) =>
      category.kind === "temp" ? { ...category, bytes: 5 * MB } : category,
    ),
  });
  assert.equal(legacy.rows[3].bytes, 5 * MB);
});

test("the sheet's boxes, the ones ticked at the opening, the button's sum", () => {
  const boxes = cleanBoxes(fr, "fr", scan);
  assert.deepEqual(
    boxes.map((box) => [box.id, box.kind, box.bytes]),
    [
      ["masters", "masters", 168 * MB],
      ["sent", "sentResults", 571 * MB],
      ["unsent", "unsentResults", 200 * MB],
      ["temp", "temp", 41 * MB],
    ],
  );
  same(boxes[1].note, "Déjà dans Astro Portal : 5 nuits et 1 plan.");
  same(
    boxes[2].note,
    "Pas encore dans Astro Portal : 2 nuits. Ce sont les seules copies ; il faudra relancer l’empilement.",
  );
  same(boxes[2].warning ?? "", "Supprimés, ils ne sont nulle part ailleurs.");
  assert.ok(
    boxes.filter((box) => box.id !== "unsent").every((b) => !b.warning),
  );
  // The results not sent are never ticked by themselves.
  const ticked = defaultTicks(boxes);
  assert.deepEqual(ticked, ["masters", "sent", "temp"]);
  assert.equal(tickedBytes(boxes, ticked), 780 * MB);
  assert.deepEqual(kindsOf(boxes, ticked), ["masters", "sentResults", "temp"]);
  // Ticking the not sent: 168 + 571 + 200 + 41 = 980 Mo, as the snackbar of B1.
  assert.equal(tickedBytes(boxes, [...ticked, "unsent"]), 980 * MB);
  assert.deepEqual(kindsOf(boxes, []), []);
});

test("a box with nothing in it is left out of the sheet", () => {
  const onlyTemp: ProcessingScan = {
    ...scan,
    categories: scan.categories.map((category) =>
      category.kind === "temp"
        ? category
        : { ...category, bytes: 0, sentBytes: 0, unsentBytes: 0 },
    ),
  };
  assert.deepEqual(
    cleanBoxes(fr, "fr", onlyTemp).map((box) => box.id),
    ["temp"],
  );
});

test("the nights and the plans a box holds are said in words, a part at zero left out", () => {
  same(holdsText(fr, "fr", 1, 0), "1 nuit");
  same(holdsText(fr, "fr", 0, 2), "2 plans");
  same(holdsText(en, "en", 5, 1), "5 nights and 1 plan");
});

test("the texts exist in the ten languages, with the same placeholders, and none keeps the old promise", async () => {
  const keys = [
    "settings.processing.title",
    "settings.processing.line",
    "settings.processing.details",
    "settings.processing.sheet.kept",
    "settings.processing.sheet.unsent.warning",
    "settings.processing.stamp",
    "settings.processing.freed",
    "settings.processing.unreachable",
    "errors.processing_files_busy",
  ];
  const dictionaries: Record<string, Record<string, string>> = {};
  for (const locale of SUPPORTED_LOCALES) {
    dictionaries[locale] = JSON.parse(
      await readFile(
        new URL(`../i18n/${locale}.json`, import.meta.url),
        "utf8",
      ),
    );
  }
  const placeholders = (text: string) =>
    [...text.matchAll(/\{(\w+)\}/g)].map((match) => match[1]).sort();
  for (const locale of SUPPORTED_LOCALES) {
    for (const key of keys) {
      assert.ok(dictionaries[locale][key], `${locale}: ${key}`);
      assert.deepEqual(
        placeholders(dictionaries[locale][key]),
        placeholders(dictionaries.fr[key]),
        `${locale}: ${key}`,
      );
    }
  }
  // The titles, one per language.
  const titles: Record<string, string> = {
    fr: "Fichiers de traitement",
    en: "Processing files",
    de: "Verarbeitungsdateien",
    es: "Archivos de procesamiento",
    it: "File di elaborazione",
    "pt-BR": "Arquivos de processamento",
    ru: "Файлы обработки",
    ja: "処理ファイル",
    "zh-Hans": "处理文件",
    "zh-Hant": "處理檔案",
  };
  for (const [locale, title] of Object.entries(titles)) {
    assert.equal(dictionaries[locale]["settings.processing.title"], title);
  }
  // The working folder is no longer said to
  // be untouched in everything; the person's own files are.
  same(
    dictionaries.fr["settings.folder.line"],
    "Astro Sync y crée un dossier par nuit ; il ne touche jamais à vos propres fichiers. Ses fichiers de traitement se nettoient plus bas.",
  );
  for (const locale of SUPPORTED_LOCALES) {
    for (const key of [
      "settings.folder.line",
      "home.how.folderDetails",
      "wizard.folder.neverMovesDetails",
      "guide.note.never",
    ]) {
      const text = dictionaries[locale][key];
      assert.doesNotMatch(
        text,
        /supprime jamais rien|never moves or deletes anything|nie etwas|nunca mueve ni elimina nada|non vi sposta né vi elimina mai nulla|nunca move nem exclui nada|ничего там не перемещает|削除したりすることはありません。作業|绝不会移动或删除其中|絕不會在其中移動或刪除/,
        `${locale}: ${key} keeps the old promise`,
      );
    }
  }
});

test("the section is wired in the settings: after the working folder, levels of buttons, snackbar, destructive primary", async () => {
  const settings = await readFile(
    new URL("../screens/SettingsScreen.tsx", import.meta.url),
    "utf8",
  );
  assert.ok(
    settings.indexOf('t("settings.folder.title")') <
      settings.indexOf("<ProcessingFiles />") &&
      settings.indexOf("<ProcessingFiles />") <
        settings.indexOf('t("settings.languageLabel")'),
    "between « Dossier de travail » and « Langue »",
  );
  const screen = await readFile(
    new URL("../screens/ProcessingFiles.tsx", import.meta.url),
    "utf8",
  );
  // « Nettoyer… » secondary, « Recalculer » tertiary, « Réinstaller… » link.
  assert.match(
    screen,
    /buttonClassName\("secondary"\)[\s\S]*?settings\.processing\.clean/,
  );
  assert.match(
    screen,
    /buttonClassName\("tertiary"\)[\s\S]*?settings\.processing\.recompute/,
  );
  assert.match(
    screen,
    /buttonClassName\("link"\)[\s\S]*?settings\.processing\.python\.reinstall/,
  );
  // The primary of the sheets is destructive; « Nettoyer… » never is.
  assert.equal(screen.match(/destructive: true/g)?.length, 2);
  assert.doesNotMatch(screen, /buttonClassName\("secondary", \{/);
  // « N Mo libérés » through the shared snackbar.
  assert.match(screen, /useSnackbar\(\)/);
  assert.match(screen, /settings\.processing\.freed/);
  // The sizes are read when the tab opens and at « Recalculer », nowhere else.
  assert.equal(screen.match(/processingFilesScan\(\)/g)?.length, 1);
  assert.match(screen, /void compute\(\)/);
});
