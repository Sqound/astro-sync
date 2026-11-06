// What Astro Sync says when something is missing or not read: the words of the ten
// states, their levels, and the ten languages.

import assert from "node:assert/strict";
import test from "node:test";

import type {
  CalibrationNote,
  DetectedTarget,
  Finding,
  Proposal,
} from "./bridge";
import {
  SUPPORTED_LOCALES,
  createTranslator,
  type Locale,
} from "../i18n/locale.ts";
import {
  calibrationFinding,
  calibrationFindings,
  calibrationNoteLevel,
  calibrationNoteText,
  darksToSee,
  nothingReadable,
  nothingReadableText,
  sheetUnreadText,
  skyPositionText,
  solveTried,
  solveWhy,
  stackedWithoutText,
  unreadAsideParts,
  unreadCounts,
} from "./missingMessages.ts";
import { asideFiles } from "./detectReview.ts";

const fr = createTranslator("fr");
/** Every kind of space reads as one: the French dictionary keeps no-break
 * spaces where the typography wants them. */
const spaced = (value: unknown) =>
  typeof value === "string" ? value.replace(/\s+/gu, " ") : value;
const same = (actual: unknown, expected: unknown, message?: string) =>
  assert.equal(spaced(actual), spaced(expected), message);
/** The no-break spaces of the French text read as plain ones. */
const plain = (text: string | null) =>
  text === null ? null : text.replace(/\s+/gu, " ");

function proposal(
  patch: {
    aside?: Partial<Proposal["setAside"]>;
    lights?: number;
    sessions?: number;
  } = {},
): Proposal {
  return {
    reading: { lights: patch.lights ?? 0 },
    sessions: Array.from({ length: patch.sessions ?? 0 }),
    setAside: {
      withoutDate: [],
      unreadable: [],
      stacks: [],
      calibrationUnattached: [],
      logsUnattached: [],
      otherFiles: [],
      ...patch.aside,
    },
  } as unknown as Proposal;
}

const files = (count: number, extension: string) =>
  Array.from({ length: count }, (_, index) => `x/${index}.${extension}`);

// ------------------------------------------------------- darks and flats

const NO_DARK: CalibrationNote = {
  code: "noDark",
  exposureSeconds: 32,
  count: 10,
  foundSeconds: [30],
};

test("a dark that does not fit and a filter without flats are warnings, with the expected words", () => {
  same(
    plain(calibrationNoteText(fr, "fr", NO_DARK)),
    "Pas de dark de 32 s pour ces poses (10 darks de 30 s non rattachés).",
  );
  same(
    plain(
      calibrationNoteText(fr, "fr", {
        ...NO_DARK,
        count: 1,
        foundSeconds: [30],
      }),
    ),
    "Pas de dark de 32 s pour ces poses (1 dark de 30 s non rattaché).",
  );
  same(
    plain(
      calibrationNoteText(fr, "fr", { ...NO_DARK, foundSeconds: [30, 60] }),
    ),
    "Pas de dark de 32 s pour ces poses (10 darks de 30 s et 60 s non rattachés).",
  );
  const flat: CalibrationNote = { code: "noFlat", filter: "V" };
  same(calibrationNoteText(fr, "fr", flat), "Pas de flat pour le filtre V.");
  same(calibrationNoteLevel(NO_DARK), "warning");
  same(calibrationNoteLevel(flat), "warning");
  same(darksToSee([flat, NO_DARK]), 10, "« Voir les 10 darks »");
  same(darksToSee([flat]), 0);
});

test("No dark at all, and no flat at all, are informations", () => {
  const dark: CalibrationNote = { code: "noDarkAtAll", exposureSeconds: 32 };
  const flat: CalibrationNote = { code: "noFlatAtAll", filter: "V" };
  same(calibrationNoteLevel(dark), "information");
  same(calibrationNoteLevel(flat), "information");
  same(
    plain(calibrationNoteText(fr, "fr", dark)),
    "Aucun dark pour ces poses de 32 s.",
  );
  same(calibrationNoteText(fr, "fr", flat), "Aucun flat pour le filtre V.");
  same(darksToSee([dark, flat]), 0, "nothing to see for an information");
});

const finding = (
  key: string,
  severity: Finding["severity"],
  params: Finding["params"],
): Finding => ({ key, severity, targetId: "ic1805", params, files: [] });

test("the night folder's findings read in « Fichiers », « Traitements » and the summary", () => {
  const dark = finding("detection.darksMissing", "warning", {
    target: "IC 1805",
    exposureSeconds: 32,
    gain: null,
    none: false,
    count: 10,
    found: [{ exposureSeconds: 30, gain: null }],
  });
  const flat = finding("detection.flatsMissing", "warning", {
    target: "IC 1805",
    filter: "V",
    none: false,
    stackedWithout: true,
  });
  const darkItem = calibrationFinding(fr, "fr", dark);
  same(
    plain(darkItem?.files ?? null),
    "Pas de dark de 32 s pour IC 1805 : les 10 darks du dossier sont de 30 s.",
  );
  same(darkItem?.level, "warning");
  same(darkItem?.folder, "darks", "« Ouvrir le dossier des darks »");
  same(darkItem?.without, null, "the darks of the folder are used");
  same(
    plain(darkItem?.treatment ?? null),
    "Pas de dark de 32 s : les 10 darks de la nuit sont de 30 s.",
  );
  const flatItem = calibrationFinding(fr, "fr", flat);
  same(flatItem?.files, "Pas de flat pour le filtre V.");
  same(flatItem?.folder, "flats");
  same(
    plain(flatItem?.treatment ?? null),
    "Pas de flat pour le filtre V : empilement sans flats.",
  );
  assert.deepEqual(flatItem?.without, { kind: "flat", filter: "V" });

  // No dark at all: an information, nothing to open, the stacking goes
  // without.
  const none = finding("detection.darksMissing", "info", {
    target: "IC 1805",
    exposureSeconds: 32,
    gain: null,
    none: true,
    count: 0,
    found: [],
  });
  const noneItem = calibrationFinding(fr, "fr", none);
  same(noneItem?.level, "information");
  same(noneItem?.folder, null);
  same(
    plain(noneItem?.treatment ?? null),
    "Pas de dark de 32 s : cette cible sera empilée sans dark.",
  );

  // The summary says only what the stacking really went without.
  const both = calibrationFindings(fr, "fr", [none, flat], "ic1805");
  same(
    stackedWithoutText(fr, "fr", both),
    "Empilée sans dark et sans flats du filtre V.",
  );
  same(
    stackedWithoutText(
      fr,
      "fr",
      calibrationFindings(fr, "fr", [flat], "ic1805"),
    ),
    "Empilée sans flats du filtre V.",
  );
  same(
    stackedWithoutText(
      fr,
      "fr",
      calibrationFindings(fr, "fr", [none], "ic1805"),
    ),
    "Empilée sans dark.",
  );
  same(
    stackedWithoutText(
      fr,
      "fr",
      calibrationFindings(fr, "fr", [dark], "ic1805"),
    ),
    null,
    "darks that do not fit are still used: nothing to say here",
  );
  assert.deepEqual(calibrationFindings(fr, "fr", [dark], "other"), []);
  same(
    calibrationFinding(fr, "fr", {
      ...flat,
      key: "detection.noLogs",
    }),
    null,
  );
});

test("a gain is said only when two darks of the same exposure differ by it", () => {
  const dark = finding("detection.darksMissing", "warning", {
    target: "IC 1805",
    exposureSeconds: 32,
    gain: 100,
    none: false,
    count: 3,
    found: [{ exposureSeconds: 32, gain: 200 }],
  });
  same(
    plain(calibrationFinding(fr, "fr", dark)?.files ?? null),
    "Pas de dark de 32 s · gain 100 pour IC 1805 : les 3 darks du dossier sont de 32 s · gain 200.",
  );
});

// ------------------------------------------------------ files not read

test("videos are not processed, RAW files not read, and a folder of nothing else is a warning", () => {
  const withBoth = proposal({
    lights: 1186,
    sessions: 7,
    aside: {
      videos: files(2, "avi"),
      videoCaptures: files(2, "avi.txt"),
      rawFiles: files(7, "cr2"),
    },
  });
  const counts = unreadCounts(withBoth);
  assert.deepEqual(counts, { videos: 2, captures: 2, raw: 7 });
  assert.deepEqual(unreadAsideParts(fr, "fr", counts), [
    "2 vidéos non traitées, avec leurs 2 fichiers de capture",
    "7 fichiers RAW non lus",
  ]);
  same(nothingReadable(withBoth), false);
  assert.deepEqual(
    unreadAsideParts(fr, "fr", { videos: 1, captures: 0, raw: 1 }),
    ["1 vidéo non traitée", "1 fichier RAW non lu"],
  );
  assert.deepEqual(
    unreadAsideParts(fr, "fr", { videos: 1, captures: 1, raw: 0 }),
    ["1 vidéo non traitée, avec son fichier de capture"],
  );
  // « Voir les 36 fichiers »: the lists count them too.
  const sheet = proposal({
    aside: {
      withoutDate: files(12, "fit"),
      calibrationUnattached: files(11, "fit"),
      logsUnattached: files(2, "log"),
      videos: files(2, "avi"),
      videoCaptures: files(2, "txt"),
      rawFiles: files(7, "cr2"),
    },
  });
  same(asideFiles(sheet).all.length, 36);

  const lune = proposal({
    aside: { videos: files(2, "avi"), rawFiles: files(7, "cr2") },
  });
  same(nothingReadable(lune), true);
  same(
    nothingReadableText(fr, "fr", unreadCounts(lune)).replace(/\s+/gu, " "),
    "Aucune pose lisible dans ce dossier. Astro Sync lit les poses FITS et XISF : 7 fichiers RAW n’y sont pas lus, 2 vidéos n’y sont pas traitées.",
  );
  // A real empty folder, or one with poses, says nothing more than before.
  same(nothingReadable(proposal()), false);
  same(
    nothingReadable(
      proposal({
        lights: 3,
        sessions: 1,
        aside: { rawFiles: files(2, "nef") },
      }),
    ),
    false,
  );
  same(sheetUnreadText(fr, "fr", { videos: 0, captures: 0, raw: 0 }), null);
  same(
    plain(sheetUnreadText(fr, "fr", { videos: 2, captures: 2, raw: 7 })),
    plain(
      "2 vidéos de ce dossier ne sont pas traitées ; 7 fichiers RAW de ce dossier ne sont pas lus.",
    ),
  );
});

// --------------------------------------------------------- the solver

const target = (patch: Partial<DetectedTarget>): DetectedTarget =>
  patch as DetectedTarget;

test("the solver's reason in words, a link for what Réglages fixes, an information for the rest", () => {
  const cases: [string, "warning" | "information", boolean, string][] = [
    [
      "solver_missing",
      "warning",
      true,
      "Aucun solveur n’est installé : les solutions de ciel ne peuvent pas être calculées ici.",
    ],
    [
      "no_database",
      "warning",
      true,
      "La base d’étoiles d’ASTAP est introuvable.",
    ],
    ["no_index", "warning", true, "astrometry.net n’a aucun fichier d’index."],
    [
      "too_few_stars",
      "information",
      false,
      "Trop peu d’étoiles reconnues dans l’image.",
    ],
    [
      "unreadable_image",
      "information",
      false,
      "Le solveur ne peut pas lire cette image.",
    ],
    [
      "not_solved",
      "information",
      false,
      "Aucune correspondance entre les étoiles de l’image et le catalogue.",
    ],
    [
      "outside_database",
      "warning",
      true,
      "Champ hors des index installés : la base d’étoiles d’ASTAP ne couvre pas un champ de cette taille.",
    ],
    [
      "timeout",
      "information",
      false,
      "Pas de solution après 120 s : la résolution a été arrêtée.",
    ],
  ];
  same(cases.length, 8, "eight failures, and the position: nine");
  for (const [code, level, settings, words] of cases) {
    const why = solveWhy(fr, target({ solveFailure: { code, seconds: 120 } }));
    same(why?.level, level, code);
    same(why?.settings, settings, code);
    same(plain(why?.text ?? null), plain(words), code);
  }
  // The ninth: the field is found, outside the known targets: the position
  // and no link (a known limit).
  const placed = solveWhy(fr, target({ solvedPosition: [331.2875, 46.4789] }));
  same(placed?.level, "information");
  same(placed?.settings, false);
  same(
    placed?.text,
    "Champ centré sur 22 h 05 min 09 s +46° 28′ 44″, hors des cibles connues.",
  );
  same(skyPositionText(331.2875, 46.4789), "22 h 05 min 09 s +46° 28′ 44″");
  same(skyPositionText(10, -5.5), "00 h 40 min 00 s −5° 30′ 00″");
  // Nothing asked of a solver: « à faire », as before.
  same(solveWhy(fr, target({})), null);
  same(solveTried(target({})), false);
  same(solveTried(target({ solveFailure: { code: "not_solved" } })), true);
  same(
    plain(
      fr("detect.night.toSolveTried", { count: 65, frame: "Light_0001.fit" }),
    ),
    plain(
      "65 poses sans cible connue : plate solving essayé sur une pose (Light_0001.fit).",
    ),
  );
});

test("the family's width reads as expected", () => {
  same(
    plain(
      fr("poseStats.familyWidth", {
        exposure: "32 s",
        value: "5,9",
        stars: 95,
      }),
    ),
    "Largeur des étoiles de la famille de 32 s : 5,9 px, médiane de 95 étoiles lues sur l’ensemble de ses poses. C’est la largeur de la famille, pas celle d’une pose.",
  );
});

// ------------------------------------------------------ ten languages

test("every sentence reads in the ten languages, with no placeholder left", () => {
  const sentences = (
    t: ReturnType<typeof createTranslator>,
    locale: Locale,
  ) => {
    const plural = (base: string, params: Record<string, string | number>) =>
      [1, 2].map((count) =>
        t(`${base}.${count === 1 ? "one" : "other"}`, { count, ...params }),
      );
    return [
      ...plural("calibration.noDark", { exposure: "32 s", found: "30 s" }),
      t("calibration.noDarkAtAll", { exposure: "32 s" }),
      t("calibration.noFlat", { filter: "V" }),
      t("calibration.noFlatUnfiltered"),
      t("calibration.noFlatAtAll", { filter: "V" }),
      t("calibration.noFlatAtAllUnfiltered"),
      ...plural("calibration.seeDarks", {}),
      ...plural("detect.aside.videos", {}),
      ...plural("detect.aside.videosCaptured", { captures: 2 }),
      ...plural("detect.aside.raw", {}),
      t("detect.nothingRead", { clauses: "7 RAW" }),
      ...plural("detect.nothingRead.raw", {}),
      ...plural("detect.nothingRead.videos", {}),
      t("wizard.detect.unread", { parts: "x" }),
      ...plural("wizard.detect.unread.videos", {}),
      ...plural("wizard.detect.unread.raw", {}),
      t("detect.night.toSolveTried", { count: 65, frame: "Light_0001.fit" }),
      t("detect.night.solveWhy.no_index"),
      t("detect.night.solvedAt", { position: "22 h" }),
      t("wizard.files.noDark", { settings: "32 s", target: "IC 1805" }),
      ...plural("wizard.files.noDarkOther", {
        settings: "32 s",
        target: "IC 1805",
        found: "30 s",
      }),
      t("wizard.files.noFlat", { filter: "V" }),
      t("wizard.files.noFlatUnfiltered"),
      t("wizard.files.noFlatAtAll", { filter: "V" }),
      t("wizard.files.noFlatAtAllUnfiltered"),
      t("wizard.files.openDarks"),
      t("wizard.files.openFlats"),
      t("wizard.treatments.noDark", { settings: "32 s" }),
      ...plural("wizard.treatments.noDarkOther", {
        settings: "32 s",
        found: "30 s",
      }),
      t("wizard.treatments.noFlat", { filter: "V" }),
      t("wizard.treatments.noFlatWithout", { filter: "V" }),
      t("wizard.treatments.noFlatUnfiltered"),
      t("wizard.treatments.noFlatWithoutUnfiltered"),
      t("wizard.summary.stackedWithout.dark"),
      t("wizard.summary.stackedWithout.flats", { filter: "V" }),
      t("wizard.summary.stackedWithout.flatsUnfiltered"),
      t("wizard.summary.stackedWithout.both", { filter: "V" }),
      t("wizard.summary.stackedWithout.bothUnfiltered"),
    ].map((sentence) => `${locale}: ${sentence}`);
  };
  const english = sentences(createTranslator("en"), "en");
  for (const locale of SUPPORTED_LOCALES) {
    const t = createTranslator(locale);
    const found = sentences(t, locale);
    same(found.length, english.length);
    for (const sentence of found) {
      assert.ok(
        !/\{\w+\}/.test(sentence),
        `a placeholder is left: ${sentence}`,
      );
      assert.ok(!/\b[a-z]+(\.[a-zA-Z_]+){2,}\b/.test(sentence), sentence);
    }
    if (locale !== "en") {
      // Translated, not left in English (« Aucun » is not « No »).
      const same = found.filter(
        (sentence, index) =>
          sentence.slice(locale.length + 2) ===
          english[index].slice("en: ".length),
      );
      assert.ok(same.length < 8, `${locale}: ${same.length} untranslated`);
    }
  }
});
