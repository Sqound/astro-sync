import assert from "node:assert/strict";
import test from "node:test";

import {
  itemStateText,
  milestoneCount,
  progressionRows,
  progressionSetAside,
  resumeNotice,
  sendingItems,
  sentLines,
  summaryView,
} from "./wizardSummary.ts";
import type { ImagePlan, ImageQuality } from "./bridge";
import type { Seg } from "./wizardFiles.ts";
import type { Treatment } from "./bridge";
import {
  CAPABILITIES_ADVANCED,
  CAPABILITIES_PLUS,
  M31,
  M31_LIGHTS,
  NIGHT_FOLDER,
  capture,
  detectionAllFound,
  detectionMixed,
  image,
  online,
  onlineMixed,
  plansMixed,
  precheck,
  resume,
  target,
} from "./homeFixtures.ts";
import { SUPPORTED_LOCALES, createTranslator } from "../i18n/locale.ts";
import { nightSentence, oneFinalStop } from "./nights.ts";

const fr = createTranslator("fr");
const plain = (s: string) => s;
const text = (segs: readonly Seg[]) =>
  plain(
    segs
      .map((s) =>
        typeof s === "string" ? s : "strong" in s ? s.strong : s.code,
      )
      .join(""),
  );
const ALL = new Set<Treatment>([
  "final_image",
  "sky_annotations",
  "logs",
  "progression",
]);

test("the summary: sent results with their SHA-256, kept online, never leaves; the check of the limits before sending", () => {
  const view = summaryView(fr, "fr", {
    precheck: precheck(plansMixed(), { toSend: 446_000_000 }),
    detection: detectionMixed(),
    online: onlineMixed(),
    treatments: ALL,
    stackTargets: new Set(),
    resume: false,
    privateServer: false,
    date: "07/10/2026",
  });
  assert.equal(
    plain(fr(view.leadKey, view.leadParams)),
    "Ce qui complétera la session du 07/10/2026.",
  );
  assert.deepEqual(
    view.groups.map((g) => fr(g.titleKey)),
    [
      "Ajouté à la session",
      "Conservé en ligne",
      "Ne quitte jamais cet ordinateur",
    ],
  );
  assert.deepEqual(
    view.groups[0].rows.map((r) => [text(r.segs), r.value]),
    [
      [
        "Image finale de NGC 7000 · NGC7000_final.tif · SHA-256 3f9a…c41e",
        "412 Mo",
      ],
      [
        "Variante de la capture de M31 · M31_final_v2.jpg · SHA-256 8c1f…e2a7",
        "15 Mo",
      ],
      ["Le rapport des journaux : paliers, qualité, guidage", "calculé ici"],
      ["Annotations du ciel de 3 images", "calculées ici"],
      // The milestones each target's light frames will give.
      ["Images de progression\u00a0: 5 pour NGC 7000", "calculées ici"],
      ["Images de progression\u00a0: 5 pour IC 1396", "calculées ici"],
      ["Images de progression\u00a0: 5 pour M31", "calculées ici"],
    ],
  );
  assert.deepEqual(
    view.groups[1].rows.map((r) => [text(r.segs), r.value]),
    [
      [
        "Image finale de IC 1396 · déjà en ligne, identique : non renvoyée",
        "96 Mo",
      ],
      [
        "Image finale de M31 · M31_final.jpg, envoyée depuis le téléphone (image principale)",
        "14 Mo",
      ],
    ],
  );
  assert.deepEqual(
    view.groups[2].rows.map((r) => [text(r.segs), r.value]),
    [
      ["Poses FITS\u00a0: 120 lights et 130 de calibration", "8,2 Go"],
      ["Journaux bruts\u00a0: NINA, PHD2", "2 fichiers"],
    ],
  );
  assert.deepEqual(
    view.checks.map((c) => plain(c.text)),
    [
      "À envoyer : 446 Mo · place restante : 231,4 Go sur 250 Go. Il en restera 230,9 Go.",
      "Plus gros fichier\u00a0: NGC7000_final.tif, 412 Mo · plafond par fichier\u00a0: 1 Go.",
      // A subscription has no cap on the variants: no « N sur M ».
      "Variantes de la capture de M31 : 1, sans limite de nombre.",
    ],
  );
  assert.equal(view.refusals.length, 0);
  assert.equal(fr(view.primaryKey), "Envoyer cette nuit");
});

test("a finite cap on the variants still says « N sur M permises »; the unlimited sentinel never prints a number", () => {
  const input = {
    detection: detectionMixed(),
    online: onlineMixed(),
    treatments: ALL,
    stackTargets: new Set<string>(),
    resume: false,
    privateServer: false,
  };
  const checks = (limit: number) =>
    summaryView(fr, "fr", {
      ...input,
      precheck: precheck(plansMixed(), {
        capabilities: { ...CAPABILITIES_PLUS, "capture.variant_limit": limit },
      }),
      date: "07/10/2026",
    }).checks.map((c) => plain(c.text));
  assert.ok(
    checks(2).includes(
      "Variantes de la capture de M31\u00a0: 1 sur 2 permises.",
    ),
  );
  for (const line of checks(Number.MAX_SAFE_INTEGER))
    assert.doesNotMatch(line, /9007199254740991|permises/);
});

test("a server that keeps no quota says « sans limite », never a figure of 9 million gigabytes", () => {
  const view = summaryView(fr, "fr", {
    precheck: precheck(plansMixed(), {
      capabilities: {
        ...CAPABILITIES_PLUS,
        "storage.quota_bytes": Number.MAX_SAFE_INTEGER,
      },
      remaining: Number.MAX_SAFE_INTEGER,
      toSend: 446_000_000,
    }),
    detection: detectionMixed(),
    online: onlineMixed(),
    treatments: ALL,
    stackTargets: new Set(),
    resume: false,
    privateServer: true,
    date: "07/10/2026",
  });
  assert.equal(
    plain(view.checks[0].text),
    "À envoyer : 446 Mo · place restante : sans limite.",
  );
});

test("a limit passed: « Envoi impossible : rien n'est envoyé », which file and which limit, the variant set aside", () => {
  const view = summaryView(fr, "fr", {
    precheck: precheck(plansMixed(false), {
      capabilities: CAPABILITIES_ADVANCED,
      remaining: 380_000_000,
      allowed: false,
      toSend: 412_000_000,
      findings: [
        {
          limit: "file_too_large",
          path: `${NIGHT_FOLDER}/NGC 7000/final/NGC7000_final.tif`,
          size: 412_000_000,
          maxFileBytes: 100_000_000,
        },
        {
          limit: "storage_quota",
          toSendBytes: 412_000_000,
          remainingBytes: 380_000_000,
          quotaBytes: 50_000_000_000,
        },
      ],
    }),
    detection: detectionMixed(),
    online: onlineMixed(),
    treatments: new Set<Treatment>(["final_image", "sky_annotations"]),
    stackTargets: new Set(),
    resume: false,
    privateServer: false,
    date: "07/10/2026",
  });
  assert.equal(view.allowed, false);
  assert.equal(
    plain(fr(view.leadKey, view.leadParams)),
    "Ce qui compléterait la session du 07/10/2026 si vous l’envoyiez.",
  );
  assert.deepEqual(
    view.groups.map((g) => fr(g.titleKey)),
    [
      "À envoyer à votre espace",
      "Conservé en ligne",
      "Laissé de côté",
      "Ne quitte jamais cet ordinateur",
    ],
  );
  assert.equal(
    text(view.groups[0].rows[0].segs),
    "Image finale de NGC 7000 · NGC7000_final.tif",
  );
  assert.ok(view.groups[0].rows[0].over);
  assert.equal(
    text(view.groups[2].rows[0].segs),
    "M31_final_v2.jpg · votre espace ne garde qu’une image par capture",
  );
  assert.deepEqual(view.refusals.map(plain), [
    "NGC7000_final.tif pèse 412 Mo ; le plafond par fichier de votre abonnement est de 100 Mo.",
    "À envoyer : 412 Mo ; place restante : 380 Mo sur 50 Go.",
  ]);
  assert.deepEqual(view.checks, []);
  assert.equal(
    plain(fr("wizard.summary.impossible")),
    "Envoi impossible : rien n’est envoyé",
  );
  assert.equal(
    plain(fr("wizard.summary.nothingSent")),
    "Tant qu’une limite est dépassée, aucune partie de la nuit n’est envoyée.",
  );
});

test("a resume: the notice says what is online and what was added; only the complements leave", () => {
  const detection = detectionAllFound();
  detection.targets = [target(M31, 96, M31_LIGHTS, null, "online")];
  detection.calibration = { darks: 20, flats: 0, darkFlats: 0, biases: 0 };
  const night = online(
    "night-5",
    [M31],
    [
      capture(
        M31,
        image("M31_final.jpg", "aa", 14_000_000, {
          source: "phone",
          solved: true,
          createdAt: "2026-10-06T08:00:00Z",
        }),
      ),
    ],
  );
  const state = resume("to_complete", "fichiers", [{ key: "logsAdded" }]);
  assert.equal(
    plain(
      resumeNotice(fr, "fr", { resume: state, online: night, detection }) ?? "",
    ),
    "En ligne : l’image finale de M31, envoyée depuis le téléphone le 6 oct., et ses annotations du ciel ; aucune analyse. Ajouté au dossier depuis : 2 journaux et 96 poses.",
  );
  const view = summaryView(fr, "fr", {
    precheck: precheck([], { toSend: 38_000 }),
    detection,
    online: night,
    treatments: new Set<Treatment>(["logs", "sky_annotations", "final_image"]),
    stackTargets: new Set(),
    resume: true,
    privateServer: false,
    date: "07/10/2026",
  });
  assert.equal(
    plain(fr(view.leadKey)),
    "Ce qui sera fait quand vous enverrez les compléments.",
  );
  assert.deepEqual(
    view.groups[0].rows.map((r) => text(r.segs)),
    ["Le rapport des journaux : paliers, qualité, guidage"],
  );
  assert.deepEqual(
    view.groups[1].rows.map((r) => [text(r.segs), r.value]),
    [
      [
        "Image finale de M31 · M31_final.jpg, envoyée depuis le téléphone",
        "14 Mo",
      ],
      ["Annotations du ciel de M31", "en ligne"],
    ],
  );
  assert.deepEqual(
    view.checks.map((c) => plain(c.text)),
    ["À envoyer : 38 ko · place restante : 231,4 Go sur 250 Go."],
  );
  assert.ok(view.analysisNote);
  assert.equal(fr(view.primaryKey), "Envoyer les compléments");
});

test("the sending and the night sent: no primary, items in order; the account of what is new and what is a variant", () => {
  const items = sendingItems(fr, "fr", {
    plans: plansMixed(),
    detection: detectionMixed(),
    online: onlineMixed(),
    treatments: ALL,
    progress: {
      logs: { state: "done" },
      [`image:${plansMixed()[0].image.sha256}`]: {
        state: "sending",
        detail: "255 Mo sur 412 Mo",
        progress: 0.62,
      },
    },
  });
  assert.deepEqual(
    items.map((i) => [i.label, itemStateText(fr, i)]),
    [
      ["Le rapport des journaux", "envoyé"],
      ["Image finale de NGC 7000", "255 Mo sur 412 Mo"],
      ["Variante de la capture de M31", "en attente"],
      ["Annotations du ciel", "en attente"],
      ["Images de progression", "en attente"],
    ],
  );
  assert.equal(
    plain(fr("wizard.sending.resumes")),
    "L’envoi reprend tout seul après une coupure.",
  );
  assert.deepEqual(
    sentLines(fr, "fr", {
      plans: plansMixed(),
      detection: detectionMixed(),
      online: onlineMixed(),
      treatments: ALL,
      solved: 3,
      progressionImages: 9,
    }).map(plain),
    [
      "2 captures envoyées : NGC 7000 (nouvelle), M31 (variante) ; IC 1396 était déjà en ligne, identique",
      "Le rapport des journaux : paliers, qualité, guidage",
      "Annotations du ciel de 3 images",
      "9 images de progression",
    ],
  );
});

test("a treatment done here says « fait » with its duration, the report of the logs keeps « envoyé »", () => {
  const items = sendingItems(fr, "fr", {
    plans: [],
    detection: detectionMixed(),
    online: onlineMixed(),
    treatments: new Set<Treatment>([...ALL, "stacking"]),
    progress: {
      logs: { state: "done" },
      stacking: {
        state: "done",
        label: "Empilement de M31",
        localDone: true,
        durationMs: 362_000,
      },
      sky: { state: "done", localDone: true, durationMs: null },
      progression: { state: "failed" },
    },
  });
  assert.deepEqual(
    items.map((i) => [i.label, itemStateText(fr, i)]),
    [
      ["Le rapport des journaux", "envoyé"],
      ["Empilement de M31", "fait · 6 min 02 s"],
      // Ticked with no final image to send (plans: []): the line stays.
      ["Image finale", "rien à traiter"],
      ["Annotations du ciel", "fait"],
      ["Images de progression", "échec"],
    ],
  );
});

/** The summary says the real number of progression images per target -- the
 * milestones the core makes, then those that leave --, never « three per
 * target ». */
test("the milestones of a night: one frame, a quarter, half, three quarters, all", () => {
  assert.deepEqual(
    [0, 1, 2, 3, 4, 5, 9, 40, 90].map(milestoneCount),
    [0, 1, 2, 3, 4, 4, 5, 5, 5],
  );
});

test("after the run: « Images de progression : 5 pour IC 1805 », and a target with no capture set aside", () => {
  const t = createTranslator("fr");
  const names = (id: string) => (id === "ic1805" ? "IC 1805" : "M31");
  const plans = [
    {
      image: { kind: "stacked", targetId: "ic1805", size: 46_000_000 },
      decision: { decision: "new_capture" },
    },
    ...[1, 2, 3, 4, 5].map((n) => ({
      image: {
        kind: "progression",
        targetId: "ic1805",
        size: 2_000_000,
        path: `IC 1805/progression/milestone_0${n}.jpg`,
      },
      decision: { decision: "progression_of_new", firstSha256: "e" },
    })),
    {
      image: { kind: "progression", targetId: "m31", size: 1_000_000 },
      decision: { decision: "not_sent", reason: "no_capture_for_progression" },
    },
  ] as unknown as ImagePlan[];
  const rows = progressionRows(t, { plans, detection: null, names });
  assert.deepEqual(
    rows.map((row) => text(row.segs)),
    ["Images de progression\u00a0: 5 pour IC 1805"],
  );
  const aside = progressionSetAside(t, {
    plans,
    names,
    size: (bytes) => `${bytes / 1_000_000} Mo`,
  });
  assert.deepEqual(
    aside.map((row) => [text(row.segs), row.value]),
    [
      [
        "Images de progression de M31 · la cible n’a pas de capture, en ligne ou dans cet envoi",
        "1 Mo",
      ],
    ],
  );
  // No dictionary keeps « three per target ».
  for (const locale of [
    "fr",
    "en",
    "ja",
    "de",
    "es",
    "it",
    "pt-BR",
    "zh-Hans",
    "zh-Hant",
    "ru",
  ]) {
    const said = createTranslator(
      locale as Parameters<typeof createTranslator>[0],
    )("wizard.summary.progression", {
      count: 5,
      target: "IC 1805",
    });
    assert.match(said, /5/, locale);
    assert.match(said, /IC 1805/, locale);
  }
});

/** The night of a real capture -- `final/` empty,
 * `stacked/` holding the TIFF an earlier stacking made, the stacking ticked,
 * « Envoi de l'image finale déjà traitée » not. */
function leftover(
  decision: ImagePlan["decision"],
  sha256 = "32a2".padEnd(60, "0") + "9dcf",
): ImagePlan {
  return {
    image: {
      targetId: M31,
      path: `${NIGHT_FOLDER}/M31/stacked/M31_2026-08-10_raw.tif`,
      sha256,
      size: 47_900_000,
      kind: "stacked",
      render: null,
    },
    decision,
  };
}

const STACKING = new Set<Treatment>(["stacking", "logs"]);
const FINAL_IMAGE_ALSO = new Set<Treatment>(["stacking", "final_image"]);

test("a stacked image left by an earlier run is never listed as a final image to send again", () => {
  for (const treatments of [STACKING, FINAL_IMAGE_ALSO]) {
    const view = summaryView(fr, "fr", {
      precheck: precheck([leftover({ decision: "new_capture" })]),
      detection: detectionMixed(),
      online: onlineMixed(),
      treatments,
      stackTargets: new Set([M31]),
      resume: false,
      privateServer: false,
      date: "07/10/2026",
    });
    const sent = view.groups.find((g) => g.key === "sent")!;
    assert.deepEqual(
      sent.rows.map((r) => text(r.segs)).filter((row) => /^Image/.test(row)),
      ["Image empilée de M31"],
      "only the stacking of this sending, « calculé ici »",
    );
    assert.ok(!sent.rows.some((r) => /Image finale/.test(text(r.segs))));
    assert.equal(
      view.groups.find((g) => g.key === "setAside"),
      undefined,
    );
    // The image online stays, and the new one will be its variant.
    const kept = view.groups.find((g) => g.key === "keptOnline")!;
    assert.equal(kept.rows.length, 1);
    assert.match(text(kept.rows[0].segs), /M31_final\.jpg.*image principale/);
  }
});

test("a stacked image of an earlier run, the target not stacked again: said as a stacked image; identical online it is kept, not sent again", () => {
  const view = (plans: ImagePlan[]) =>
    summaryView(fr, "fr", {
      precheck: precheck(plans),
      detection: detectionMixed(),
      online: onlineMixed(),
      treatments: new Set<Treatment>(["logs"]),
      stackTargets: new Set(),
      resume: false,
      privateServer: false,
      date: "07/10/2026",
    });
  const sent = view([leftover({ decision: "variant", captureId: "c" })]).groups;
  assert.ok(
    text(sent[0].rows[0].segs).startsWith("Variante de la capture de M31"),
  );
  const fresh = view([leftover({ decision: "new_capture" })]).groups;
  assert.equal(
    text(fresh[0].rows[0].segs),
    "Image empilée de M31 · M31_2026-08-10_raw.tif · SHA-256 32a2…9dcf",
  );
  const same = view([
    leftover({ decision: "already_online", captureId: "c", imageId: "i" }),
  ]).groups.find((g) => g.key === "keptOnline")!;
  assert.equal(
    text(same.rows[0].segs),
    "Image empilée de M31 · déjà en ligne, identique\u00a0: non renvoyée",
  );
  assert.equal(
    view([
      leftover({ decision: "already_online", captureId: "c", imageId: "i" }),
    ]).groups.find((g) => g.key === "sent")?.rows.length ?? 0,
    1, // the report of the logs only
  );
});

test("the sending says the truth: identical bytes already online are « déjà en ligne, identique », not « envoyé »; a treatment with nothing to do is « rien à traiter »", () => {
  const plans = [leftover({ decision: "variant", captureId: "c" })];
  const key = `image:${plans[0].image.sha256}`;
  const items = sendingItems(fr, "fr", {
    plans,
    detection: detectionMixed(),
    online: onlineMixed(),
    treatments: new Set<Treatment>([
      "stacking",
      "final_image",
      "sky_annotations",
    ]),
    progress: {
      [key]: { state: "done", alreadyOnline: true },
      sky: { state: "none" },
    },
  });
  assert.deepEqual(
    items.map((i) => [i.label, i.state, itemStateText(fr, i)]),
    [
      ["Variante de la capture de M31", "done", "déjà en ligne, identique"],
      // A stacked image is not a final image: the ticked treatment has none.
      ["Image finale", "none", "rien à traiter"],
      ["Annotations du ciel", "none", "rien à traiter"],
    ],
  );
  const sent = sendingItems(fr, "fr", {
    plans,
    detection: detectionMixed(),
    online: onlineMixed(),
    treatments: new Set<Treatment>(["stacking"]),
    progress: { [key]: { state: "done" } },
  });
  assert.equal(
    itemStateText(
      fr,
      sent.find((i) => i.key === key)!,
    ),
    "envoyé",
  );
});

const MEASURED: ImageQuality = {
  fwhmPixels: 3.8,
  fwhmArcsec: 2.14,
  eccentricity: 0.31,
  starCount: 212,
  measuredOn: "linear",
  measuredAt: "2026-08-10T23:40:00Z",
  tool: { name: "Astro Sync", version: "0.1.0" },
};

/** A render of this sending's stacking, with the quality Astro Sync read on
 * its master. */
function render(name: string, quality?: ImageQuality): ImagePlan {
  return {
    image: {
      targetId: M31,
      path: `${NIGHT_FOLDER}/M31/stacked/M31_2026-08-10_${name}.tif`,
      sha256: name.padEnd(64, "0"),
      size: 47_900_000,
      kind: "stacked",
      render: name,
    },
    decision: { decision: "new_capture" },
    ...(quality ? { metadata: { imageQuality: quality } } : {}),
  };
}

test("under a stacked image, one line says the quality Astro Sync measured, once for the renders of one stacking, absent without a measure", () => {
  const rows = (plans: ImagePlan[], locale: "fr" | "en" | "ja" = "fr") =>
    summaryView(createTranslator(locale), locale, {
      precheck: precheck(plans),
      detection: detectionMixed(),
      online: onlineMixed(),
      treatments: new Set<Treatment>(["logs"]),
      stackTargets: new Set(),
      resume: false,
      privateServer: false,
      date: "07/10/2026",
    }).groups.find((g) => g.key === "sent")!.rows;
  const measured = rows([render("raw", MEASURED), render("linear", MEASURED)]);
  // The first image, its quality, the second image: the line is said once.
  assert.deepEqual(
    measured.slice(0, 3).map((r) => r.note === true),
    [false, true, false],
  );
  assert.equal(measured.filter((r) => r.note).length, 1);
  assert.equal(
    text(measured[1].segs),
    "Qualité de l’image empilée\u00a0: FWHM 2,1″ · excentricité 0,31 · 212 étoiles",
  );
  assert.equal(measured[1].value, "");
  // Without the arc seconds the pixels; without an eccentricity, without it.
  const pixels = rows([
    render("raw", { ...MEASURED, fwhmArcsec: null, eccentricity: null }),
  ]);
  assert.equal(
    text(pixels[1].segs),
    "Qualité de l’image empilée\u00a0: FWHM 3,8\u00a0px · 212 étoiles",
  );
  // The decimal mark follows the language; no key is left bare.
  for (const locale of ["en", "ja"] as const) {
    const said = text(rows([render("raw", MEASURED)], locale)[1].segs);
    assert.match(said, /FWHM 2\.1″/u, locale);
    assert.ok(!said.includes("wizard."), said);
  }
  // No measure (a space without the right, an image processed elsewhere): no line.
  assert.equal(rows([render("raw")]).filter((r) => r.note).length, 0);
});

// The lead of « Récapitulatif » says which session the night belongs to, by
// its date, short and in the order of the language, in the ten
// dictionaries.
test("the lead of the summary carries the date of the night in its sentence form (« 7 oct. »), in each of the ten languages", () => {
  const expected: Record<string, string> = {
    en: "Oct 7",
    fr: "7 oct.",
    ja: "10月7日",
    "zh-Hans": "10月7日",
    "zh-Hant": "10月7日",
  };
  assert.equal(SUPPORTED_LOCALES.length, 10);
  for (const locale of SUPPORTED_LOCALES) {
    // One date per night: the evening, whatever the computer's time zone.
    const date = nightSentence("2026-10-07", locale);
    if (expected[locale]) assert.equal(date, expected[locale], locale);
    assert.ok(date.includes("7"), `${locale}: ${date}`);
    assert.ok(!/\d{2}[/.]\d{2}/.test(date), `${locale}: no numeric date`);
    for (const allowed of [true, false]) {
      const view = summaryView(createTranslator(locale), locale, {
        precheck: precheck(plansMixed(), allowed ? {} : { allowed: false }),
        detection: detectionMixed(),
        online: onlineMixed(),
        treatments: ALL,
        stackTargets: new Set(),
        resume: false,
        privateServer: false,
        date,
      });
      const lead = oneFinalStop(
        createTranslator(locale)(view.leadKey, view.leadParams),
      );
      assert.ok(lead.includes(date), `${locale}: ${lead}`);
      assert.ok(!lead.includes("{"), `${locale}: ${lead}`);
      assert.ok(!lead.endsWith(".."), `${locale}: ${lead}`);
    }
  }
  assert.equal(nightSentence("not a date", "fr"), "—");
});

// The quality measured on a master the person gave leaves with the images
// of its target that Astro Sync did not stack (their final images), and
// the summary says where the figures were read.
test("under a final image the quality is said as measured on the person's master, under a stacked image as Astro Sync's own", () => {
  const final: ImagePlan = {
    ...render("raw"),
    image: {
      targetId: M31,
      path: `${NIGHT_FOLDER}/M31/final/M31_final.tif`,
      sha256: "f".repeat(64),
      size: 47_900_000,
      kind: "final",
      render: null,
    },
    metadata: { imageQuality: MEASURED },
  };
  const rows = (plans: ImagePlan[], locale: "fr" | "en" = "fr") =>
    summaryView(createTranslator(locale), locale, {
      precheck: precheck(plans),
      detection: detectionMixed(),
      online: onlineMixed(),
      treatments: new Set<Treatment>(["logs", "final_image"]),
      stackTargets: new Set(),
      resume: false,
      privateServer: false,
      date: "07/10/2026",
    }).groups.find((g) => g.key === "sent")!.rows;
  const measured = rows([final]);
  assert.equal(
    text(measured[1].segs),
    "Qualité mesurée sur votre master : FWHM 2,1″ · excentricité 0,31 · 212 étoiles",
  );
  assert.equal(measured[1].note, true);
  assert.match(
    text(rows([final], "en")[1].segs),
    /^Quality measured on your master: FWHM 2\.1″/,
  );
  // A stacked image keeps the stacking's sentence.
  assert.match(
    text(rows([render("raw", MEASURED)])[1].segs),
    /^Qualité de l’image empilée/,
  );
  // No measure, no line.
  const bare = { ...final, metadata: undefined };
  assert.equal(rows([bare]).filter((r) => r.note).length, 0);
});
