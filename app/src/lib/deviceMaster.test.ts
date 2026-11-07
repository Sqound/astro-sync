import assert from "node:assert/strict";
import test from "node:test";

import type {
  DeviceFamily,
  DeviceImage,
  IntegrationPreview,
  TargetDetection,
} from "./bridge";
import {
  BYTES_PER_POSE,
  deviceBox,
  deviceChoiceView,
  deviceUses,
  hasDeviceImage,
  loadDeviceUse,
  measuredImage,
  restackCost,
  restacksPoses,
  saveDeviceUse,
  sourceOf,
  sourcesOf,
  usedDeviceImages,
  withDeviceChoice,
  type DeviceChoices,
} from "./deviceMaster.ts";
import { DEFAULT_CHOICES, treatmentsView } from "./wizardTreatments.ts";
import { summaryView } from "./wizardSummary.ts";
import type { Seg } from "./wizardFiles.ts";
import { createTranslator } from "../i18n/locale.ts";
import {
  PIPELINES,
  detectionMixed,
  onlineMixed,
  plansMixed,
  precheck,
  tools,
  treatments,
} from "./homeFixtures.ts";

const fr = createTranslator("fr");
/** The French typography writes its colons and figures with no-break spaces:
 * the sentences are compared with plain ones. */
const n = (value: string | null | undefined) =>
  (value ?? "").replace(/[\u00a0\u202f]/g, " ");
const text = (segs: readonly Seg[]) =>
  n(
    segs
      .map((s) =>
        typeof s === "string" ? s : "strong" in s ? s.strong : s.code,
      )
      .join(""),
  );

/* The night of 10 to 11 October 2026, as the engine reads it: two images of
   10 s (28 and 8 poses) and one of 30 s (430), the 466 poses in lights/. */
const STACKED_430 = "Stacked_430_M 45_30.0s_IRCUT_20261011-061931.fit";
const STACKED_28 = "Stacked_28_M 45_10.0s_IRCUT_20261010-223400.fit";
const STACKED_8 = "Stacked_8_M 45_10.0s_IRCUT_20261010-233016.fit";

function image(
  fileName: string,
  poses: number,
  exposure: number,
  family: string,
): DeviceImage {
  return {
    relativePath: `M45/device/${fileName}`,
    fileName,
    bytes: 49_775_040,
    kind: "seestar",
    device: "Seestar S50 Pro",
    poses,
    exposureSeconds: exposure,
    totalSeconds: poses * exposure,
    filter: "IRCUT",
    gain: 200,
    width: 2160,
    height: 3840,
    solved: true,
    family,
  };
}

function family(
  label: string,
  exposure: number,
  lights: number,
  images: string[],
  overrides: Partial<DeviceFamily> = {},
): DeviceFamily {
  const covers = images.length === 1;
  return {
    label,
    filter: "IRCUT",
    exposureSeconds: exposure,
    gain: 200,
    lights,
    images,
    coverage: covers ? "covers" : "split",
    defaultSource: covers || lights === 0 ? "device" : "restack",
    useFile: covers || lights === 0 ? images[0] : null,
    asidePoses: 0,
    ...overrides,
  };
}

const M45 = "t-m45";

function target(
  families: DeviceFamily[],
  images: DeviceImage[],
  lightFrames: number,
  jpgs: string[] = [],
): TargetDetection {
  return {
    targetId: M45,
    designation: "M45",
    folderName: "M45",
    inNight: true,
    lightFrames,
    filters: ["IRCUT"],
    finalImage: null,
    otherFinalImages: [],
    minimum: "missing",
    finalBytes: null,
    lightSummary: {
      filters: [{ filter: "IRCUT", frames: lightFrames }],
      exposures: [30, 10],
      gains: [200],
      bytes: 7_200_000_000,
    },
    device: { images, families, jpgs },
  };
}

const THIRTY = image(STACKED_430, 430, 30, "IRCUT_30s_g200");
const TEN_A = image(STACKED_28, 28, 10, "IRCUT_10s_g200");
const TEN_B = image(STACKED_8, 8, 10, "IRCUT_10s_g200");

/** The common case: one image, the poses of its family here. */
const ONE = () =>
  target([family("IRCUT_30s_g200", 30, 430, [STACKED_430])], [THIRTY], 430);
/** The real night: three images, two families. */
const NIGHT = () =>
  target(
    [
      family("IRCUT_30s_g200", 30, 430, [STACKED_430]),
      family("IRCUT_10s_g200", 10, 36, [STACKED_28, STACKED_8]),
    ],
    [THIRTY, TEN_A, TEN_B],
    466,
  );

test("a family takes the choice of the person when it can be honoured, else the engine's default", () => {
  const night = NIGHT();
  const [thirty, ten] = night.device!.families;
  assert.equal(sourceOf(thirty, undefined), "device");
  assert.equal(sourceOf(thirty, "restack"), "restack");
  assert.equal(sourceOf(thirty, "device"), "device");
  // The split family has no image to take: « device » is not honoured.
  assert.equal(sourceOf(ten, undefined), "restack");
  assert.equal(sourceOf(ten, "device"), "restack");
  // A family whose poses are not here cannot restack.
  const alone = family("IRCUT_30s_g200", 30, 0, [STACKED_430]);
  assert.equal(sourceOf(alone, "restack"), "device");
});

test("what the stacking is asked to take: the file of each family that takes the device's image", () => {
  const night = NIGHT();
  assert.deepEqual(deviceUses(night, {}), [{ file: STACKED_430 }]);
  // Restacking the 30 s family too: nothing is taken.
  const restack: DeviceChoices = { [M45]: { IRCUT_30s_g200: "restack" } };
  assert.deepEqual(deviceUses(night, restack), []);
  assert.deepEqual(
    sourcesOf(night, restack).map(({ family: f, source }) => [f.label, source]),
    [
      ["IRCUT_30s_g200", "restack"],
      ["IRCUT_10s_g200", "restack"],
    ],
  );
  // A target without an image of a device asks for nothing.
  const plain: TargetDetection = { ...ONE(), device: undefined };
  assert.deepEqual(deviceUses(plain, {}), []);
  assert.equal(hasDeviceImage(plain), false);
});

test("the poses are stacked again only when some pose is outside the families that take the image", () => {
  const night = NIGHT();
  // 30 s takes the image, 10 s restacks its 36 poses: the poses count.
  assert.equal(restacksPoses(night, {}), true);
  // One image, one family, all the poses in it: nothing to measure or flag.
  assert.equal(restacksPoses(ONE(), {}), false);
  assert.equal(
    restacksPoses(ONE(), { [M45]: { IRCUT_30s_g200: "restack" } }),
    true,
  );
  // Poses of a family the device has no image of: stacked as ever.
  assert.equal(restacksPoses({ ...ONE(), lightFrames: 500 }, {}), true);
  // No device image at all: stacked as ever.
  assert.equal(restacksPoses({ ...ONE(), device: undefined }, {}), true);
});

test("the image measured is the one the first family that takes a device's image takes", () => {
  assert.equal(measuredImage(NIGHT(), {})?.fileName, STACKED_430);
  assert.equal(
    measuredImage(NIGHT(), { [M45]: { IRCUT_30s_g200: "restack" } }),
    null,
  );
  assert.deepEqual(
    usedDeviceImages(NIGHT(), {}).map((i) => i.fileName),
    [STACKED_430],
  );
});

test("the cost of stacking again is the one measured: 430 poses in 10 min and 90 Go, 36 in about a minute", () => {
  assert.deepEqual(restackCost(430), {
    minutes: 10,
    bytes: 430 * BYTES_PER_POSE,
  });
  assert.equal(restackCost(430).bytes, 90_300_000_000);
  assert.equal(restackCost(36).minutes, 1);
  assert.equal(restackCost(1).minutes, 1);
});

test("the choice is kept on this computer, and whatever cannot be read is the default", () => {
  const memory = new Map<string, string>();
  const storage = {
    getItem: (key: string) => memory.get(key) ?? null,
    setItem: (key: string, value: string) => void memory.set(key, value),
  };
  const choices = withDeviceChoice({}, M45, "IRCUT_30s_g200", "restack");
  saveDeviceUse(storage, "night-1", choices);
  assert.deepEqual(loadDeviceUse(storage, "night-1"), choices);
  assert.deepEqual(loadDeviceUse(storage, "night-2"), {});
  memory.set("astro-sync.deviceUse:night-3", "not json");
  assert.deepEqual(loadDeviceUse(storage, "night-3"), {});
  memory.set(
    "astro-sync.deviceUse:night-4",
    JSON.stringify({ [M45]: { a: "device", b: "nonsense" }, x: 3 }),
  );
  assert.deepEqual(loadDeviceUse(storage, "night-4"), {
    [M45]: { a: "device" },
  });
  assert.deepEqual(loadDeviceUse(null, "night-1"), {});
  // A change keeps the other families.
  assert.deepEqual(withDeviceChoice(choices, M45, "other", "device")[M45], {
    IRCUT_30s_g200: "restack",
    other: "device",
  });
});

/* ------------------------------- « Fichiers » ------------------------------ */

test("one image found, the way its header says it, and where the choice is", () => {
  const box = deviceBox(fr, "fr", ONE())!;
  assert.equal(n(box.state.text), "Trouvée");
  assert.equal(box.state.tone, "success");
  assert.equal(box.groups.length, 1);
  assert.equal(box.groups[0].heading, null, "one family: no family title");
  const lines = box.groups[0].lines.map((line) => text(line.segs));
  assert.equal(lines[0], `device/${STACKED_430} · 49,7 Mo`);
  assert.equal(
    lines[1],
    "Seestar S50 Pro · 430 poses de 30 s · 3 h 35 min · RVB linéaire, 16 bits · solution de champ dans l’en-tête",
  );
  const notes = box.notes.map((line) => text(line.segs));
  assert.ok(n(notes[0]).startsWith("Astro Sync peut la prendre pour master"));
  assert.ok(n(notes[0]).includes("« Traitements »"));
  assert.ok(
    notes.includes(
      "Le .jpg du même nom, étiré, n’est jamais pris pour un master.",
    ),
  );
});

test("the real night, three images and two families, the split one said why", () => {
  const box = deviceBox(fr, "fr", NIGHT())!;
  assert.equal(n(box.state.text), "3 trouvées");
  const [thirty, ten] = box.groups;
  assert.equal(n(thirty.heading), "Famille de 30 s · 430 poses");
  assert.deepEqual(thirty.status, {
    tone: "success",
    text: "Une image la couvre",
  });
  assert.equal(thirty.note, null);
  assert.equal(n(ten.heading), "Famille de 10 s · 36 poses");
  assert.deepEqual(ten.status, {
    tone: "warning",
    text: "Aucune image ne la couvre",
  });
  assert.deepEqual(
    ten.lines.map((line) => text(line.segs)),
    [
      `device/${STACKED_28} · 49,7 Mo`,
      "28 poses · 4 min 40 s",
      `device/${STACKED_8} · 49,7 Mo`,
      "8 poses · 1 min 20 s",
    ],
  );
  assert.equal(
    n(ten.note),
    n(
      "L’appareil a coupé cette famille en 2 images (28 et 8 poses) : aucune ne contient les 36 poses. Astro Sync ne combine pas deux images d’appareil : pour cette famille, un nouvel empilement des poses est proposé.",
    ),
  );
  const notes = box.notes.map((line) => text(line.segs));
  assert.ok(
    notes.some((line) =>
      line.startsWith(
        "Des familles de poses différentes (30 s et 10 s) ne se mêlent jamais",
      ),
    ),
  );
});

test("a split family whose poses are not here takes the biggest image and names what it leaves aside", () => {
  const bigger = family("IRCUT_10s_g200", 10, 0, [STACKED_28, STACKED_8], {
    defaultSource: "device",
    useFile: STACKED_28,
    asidePoses: 8,
  });
  const box = deviceBox(fr, "fr", target([bigger], [TEN_A, TEN_B], 0))!;
  assert.ok(
    n(box.groups[0].note).includes(
      n(
        "les poses ne sont pas sur cet ordinateur : Astro Sync propose la plus grosse, qui laisse de côté 8 poses.",
      ),
    ),
  );
});

test("the .jpg alone is never a master, with the poses and without anything else", () => {
  const jpg = ["M45/device/Stacked_430_M 45_30.0s_IRCUT_20261011-061931.jpg"];
  const withPoses = deviceBox(fr, "fr", target([], [], 430, jpg))!;
  assert.deepEqual(withPoses.state, { tone: "warning", text: "Pas un master" });
  assert.equal(withPoses.groups.length, 0);
  const lines = withPoses.notes.map((line) => text(line.segs));
  assert.equal(
    lines[0],
    "device/Stacked_430_M 45_30.0s_IRCUT_20261011-061931.jpg",
  );
  assert.ok(lines[1].startsWith(n("Cette image est un .jpg : étirée")));
  assert.ok(
    lines[1].endsWith("Astro Sync empile les poses de lights/, comme avant."),
  );
  assert.equal(
    lines[2],
    "Pour utiliser l’image de l’appareil, copiez aussi son fichier .fit (même nom).",
  );
  const alone = deviceBox(fr, "fr", target([], [], 0, jpg))!;
  const aloneLines = alone.notes.map((line) => text(line.segs));
  assert.ok(aloneLines[1].startsWith("Ni poses, ni fichier .fit"));
  assert.ok(aloneLines[2].startsWith("Déposé dans final/"));
  // A target with nothing in device/ has no card.
  assert.equal(deviceBox(fr, "fr", { ...ONE(), device: undefined }), null);
});

/* ----------------------------- « Traitements » ----------------------------- */

test("the device's image is the default, the stacking of the poses the side line, the cost said", () => {
  const view = deviceChoiceView(fr, "fr", {
    target: ONE(),
    choices: {},
    detection: { folder: "/nuits/2026-10-10 · Maison · 3e7a91" },
    day: "10 oct.",
  })!;
  assert.equal(view.families.length, 1);
  const [row] = view.families;
  assert.equal(row.heading, null);
  assert.equal(row.source, "device");
  assert.equal(row.use.available, true);
  assert.equal(row.use.isDefault, true);
  assert.equal(
    text(row.use.segs),
    n(
      `${STACKED_430} · 430 poses · prête tout de suite : ni attente, ni place disque.`,
    ),
  );
  assert.equal(row.restack.available, true);
  assert.equal(row.restack.isDefault, false);
  assert.equal(
    text(row.restack.segs),
    "Les 430 poses de 30 s de lights/ · environ 10 min et 90,3 Go de place libre pendant le travail. Utile pour plusieurs nuits, une nuit coupée en plusieurs images, des poses ratées à écarter, des étoiles plus fines.",
  );
  assert.equal(
    n(view.pipelinesNote),
    n(
      "Les pipelines choisis partent d’elle comme d’un empilement d’Astro Sync : étirement, étoiles, contraste.",
    ),
  );
  // The image is the only source: the poses of the night mean nothing.
  assert.ok(n(view.unused).startsWith("Sans objet pour l’image de l’appareil"));
  assert.deepEqual(view.measure, {
    fileName: STACKED_430,
    path: `/nuits/2026-10-10 · Maison · 3e7a91/M45/device/${STACKED_430}`,
  });
});

test("two families, two choices, the split one restacks and says why the image is unavailable", () => {
  const view = deviceChoiceView(fr, "fr", {
    target: NIGHT(),
    choices: {},
    detection: { folder: "/n" },
    day: "10 oct.",
  })!;
  const [thirty, ten] = view.families;
  assert.equal(n(thirty.heading), "Famille de 30 s · 430 poses");
  assert.equal(thirty.source, "device");
  assert.equal(n(ten.heading), "Famille de 10 s · 36 poses");
  assert.equal(ten.source, "restack");
  assert.equal(ten.use.available, false);
  assert.equal(ten.use.isDefault, false);
  assert.equal(
    n(ten.use.why),
    n(
      "2 images (28 et 8 poses) : aucune ne contient les 36 poses de la famille.",
    ),
  );
  assert.equal(ten.restack.isDefault, true);
  assert.equal(
    text(ten.restack.segs),
    "Les 36 poses de 10 s de lights/ · environ 1 min et 7,5 Go de place libre pendant le travail.",
  );
  // The poses of the 10 s family are stacked again: their measure counts.
  assert.equal(view.unused, null);
  // A choice that cannot be honoured changes nothing.
  const forced = deviceChoiceView(fr, "fr", {
    target: NIGHT(),
    choices: { [M45]: { IRCUT_10s_g200: "device" } },
    detection: null,
    day: null,
  })!;
  assert.equal(forced.families[1].source, "restack");
  assert.equal(forced.measure, null, "no night folder, no path");
});

test("the family whose poses are not here offers no new stacking and says why", () => {
  const only = target(
    [family("IRCUT_30s_g200", 30, 0, [STACKED_430])],
    [THIRTY],
    0,
  );
  const view = deviceChoiceView(fr, "fr", {
    target: only,
    choices: { [M45]: { IRCUT_30s_g200: "restack" } },
    detection: null,
    day: null,
  })!;
  const [row] = view.families;
  assert.equal(row.source, "device");
  assert.equal(row.restack.available, false);
  assert.equal(n(row.restack.why), "Les poses ne sont pas sur cet ordinateur.");
});

test("four reasons to stack again, with the night's own split, and the comparison measured", () => {
  const view = deviceChoiceView(fr, "fr", {
    target: NIGHT(),
    choices: {},
    detection: null,
    day: "10 oct.",
  })!;
  const why = view.why;
  assert.deepEqual(
    why.cases.map((item) => item.title),
    [
      "Plusieurs nuits à combiner.",
      "Une nuit coupée en plusieurs images.",
      "Des poses ratées à écarter.",
      "Des étoiles plus fines.",
    ],
  );
  assert.equal(
    n(why.cases[1].text),
    n(
      "Le 10 oct., les 36 poses de 10 s sont dans 2 images de l’appareil (28 et 8 poses) ; empilées à nouveau, elles font une image de 36 poses.",
    ),
  );
  assert.ok(n(why.cases[3].text).includes("12 % plus fines"));
  assert.deepEqual(why.table.head.map(n), [
    "Image de l’appareil",
    "Empilée ici",
  ]);
  assert.deepEqual(
    why.table.rows.map(([label, a, b]) => [
      n(label),
      n(a),
      b === null ? null : n(b),
    ]),
    [
      ["Poses empilées", "430", "429 (1 non alignée, nommée)"],
      ["Largeur des étoiles (FWHM)", "4,14 px", "3,65 px (12 % plus fines)"],
      ["Excentricité · étoiles trouvées", "0,45 · 382", "0,41 · 371"],
      ["Rapport signal sur bruit des étoiles", "égal à ± 5 %", null],
      ["Temps de travail", "aucun", "10 min 26 s"],
      [
        "Place libre pendant le travail",
        "aucune",
        "environ 90,3 Go, rendue ensuite",
      ],
    ],
  );
  // Without a split family the case says it in general.
  const one = deviceChoiceView(fr, "fr", {
    target: ONE(),
    choices: {},
    detection: null,
    day: "10 oct.",
  })!;
  assert.ok(n(one.why.cases[1].text).startsWith("Quand l’appareil coupe"));
});

/* A target card of the real night, ticked, in the box « Traitements ». */
function viewOf(
  target: TargetDetection,
  deviceUse: DeviceChoices = {},
  families = 2,
) {
  const detection = detectionMixed();
  detection.targets = [target, ...detection.targets.slice(1)];
  const previews: Record<string, IntegrationPreview> = {
    [M45]: {
      targetFolder: "M45",
      outside: 0,
      families: [
        {
          label: "IRCUT_30s_g200",
          filter: "IRCUT",
          band: "broadband",
          exposureSeconds: 30,
          gain: 200,
          binning: "1",
          temperatureC: 17,
          frameCount: 430,
          totalSeconds: 12900,
          master: null,
        },
        {
          label: "IRCUT_10s_g200",
          filter: "IRCUT",
          band: "broadband",
          exposureSeconds: 10,
          gain: 200,
          binning: "1",
          temperatureC: 17,
          frameCount: 36,
          totalSeconds: 360,
          master: null,
        },
      ],
    },
  };
  previews[M45].families = previews[M45].families.slice(0, families);
  return treatmentsView(fr, "fr", {
    treatments: treatments(),
    tools: tools(),
    detection: { ...detection, folder: "/n" },
    online: onlineMixed(),
    plans: plansMixed(true),
    variantsAllowed: true,
    privateServer: null,
    pipelines: PIPELINES,
    choices: {
      ...DEFAULT_CHOICES,
      stackTargets: new Set([M45]),
      deviceUse,
    },
    previews,
    eveningDate: "2026-10-10",
  });
}

test("the card of a target that has the device's image says the device, offers the choice and leaves out what is not integrated", () => {
  const card = viewOf(NIGHT()).options[0].cards[0];
  assert.equal(card.designation, "M45");
  assert.equal(card.line, "Seestar S50 Pro · 466 poses · 3 h 41 · 2 familles");
  assert.ok(card.device, "the choice is in the card");
  assert.equal(card.device?.families.length, 2);
  // The 30 s family takes the image: only the 10 s family is integrated.
  assert.deepEqual(
    card.integration?.families.map((f) => f.text.split(" · ")[0]),
    ["IRCUT 10 s"].map(
      (x) => card.integration!.families[0].text.split(" · ")[0],
    ),
  );
  assert.equal(card.integration?.families.length, 1);
  assert.equal(card.restacksPoses, true);
});

test("a target with one image and all its poses in it integrates nothing and has no pose to measure", () => {
  const card = viewOf(ONE(), {}, 1).options[0].cards[0];
  assert.equal(card.integration, null, "nothing is integrated");
  assert.equal(card.restacksPoses, false);
  // Restacking brings the integration and the poses back.
  const again = viewOf(ONE(), { [M45]: { IRCUT_30s_g200: "restack" } }, 1)
    .options[0].cards[0];
  assert.ok(again.integration);
  assert.equal(again.restacksPoses, true);
});

test("a target with only the device's image is something to stack", () => {
  const only = target(
    [family("IRCUT_30s_g200", 30, 0, [STACKED_430])],
    [THIRTY],
    0,
  );
  const view = viewOf(only, {}, 0);
  const stacking = view.options[0];
  assert.equal(stacking.noFrames, false);
  const card = stacking.cards.find((item) => item.targetId === M45)!;
  assert.equal(card.frames, 430);
  assert.equal(card.line, "Seestar S50 Pro · 430 poses");
});

test("an unticked card shows no device choice", () => {
  const detection = detectionMixed();
  detection.targets = [NIGHT(), ...detection.targets.slice(1)];
  const view = treatmentsView(fr, "fr", {
    treatments: treatments(),
    tools: tools(),
    detection,
    online: onlineMixed(),
    plans: plansMixed(true),
    variantsAllowed: true,
    privateServer: null,
    pipelines: PIPELINES,
    choices: { ...DEFAULT_CHOICES },
  });
  assert.equal(view.options[0].cards[0].device, null);
});

test("the option is named « Image empilée de la nuit (à partir des FITS) » in ten languages", () => {
  const titles = [
    "fr",
    "en",
    "de",
    "es",
    "it",
    "ja",
    "pt-BR",
    "ru",
    "zh-Hans",
    "zh-Hant",
  ].map((locale) =>
    createTranslator(locale as "fr")("wizard.treatments.stacking.title"),
  );
  assert.equal(new Set(titles).size, 10, "ten different titles");
  for (const title of titles) {
    assert.ok(title.includes("FITS"), title);
    assert.ok(!title.includes("wizard."), `missing: ${title}`);
  }
  assert.equal(titles[0], "Image empilée de la nuit (à partir des FITS)");
});

/* ------------------------------ « Récapitulatif » --------------------------- */

test("the render says where it starts from, the image never leaves, the figures say whose they are", () => {
  const detection = detectionMixed();
  detection.targets = [ONE(), ...detection.targets.slice(1)];
  const summary = summaryView(fr, "fr", {
    precheck: precheck([]),
    detection,
    online: onlineMixed(),
    treatments: new Set(["stacking"] as const),
    stackTargets: new Set([M45]),
    deviceUse: {},
    deviceQuality: {
      [M45]: {
        fwhmPixels: 4.144,
        fwhmArcsec: null,
        eccentricity: 0.448,
        starCount: 382,
      } as never,
    },
    resume: false,
    privateServer: false,
    date: "10 oct.",
  });
  const sent = summary.groups.find((group) => group.key === "sent")!;
  const rows = sent.rows.map((row) => text(row.segs));
  const at = rows.indexOf("Image empilée de M45 · PNG");
  assert.ok(at >= 0, rows.join(" | "));
  assert.equal(
    rows[at + 1],
    "Faite ici à partir de l’image empilée par le Seestar S50 Pro (430 poses de 30 s).",
  );
  assert.equal(
    rows[at + 2],
    n(
      "Qualité mesurée sur le master de l’appareil : FWHM 4,1 px · excentricité 0,45 · 382 étoiles",
    ),
  );
  const never = summary.groups.find((group) => group.key === "neverLeaves")!;
  const neverRows = never.rows.map((row) => [text(row.segs), row.value]);
  assert.ok(
    neverRows.some(
      ([line, value]) =>
        line === `Image empilée par l’appareil · ${STACKED_430}` &&
        value === "49,7 Mo",
    ),
    JSON.stringify(neverRows),
  );
  assert.ok(
    neverRows.some(
      ([line]) => line === "lue sur place, jamais copiée ni envoyée",
    ),
  );
  // Without a measure, no figure line; without the stacking, nothing of it.
  const unmeasured = summaryView(fr, "fr", {
    precheck: precheck([]),
    detection,
    online: onlineMixed(),
    treatments: new Set(["stacking"] as const),
    stackTargets: new Set([M45]),
    resume: false,
    privateServer: false,
    date: "10 oct.",
  });
  assert.ok(
    !unmeasured.groups
      .flatMap((group) => group.rows)
      .some((row) =>
        text(row.segs).startsWith("Qualité mesurée sur le master"),
      ),
  );
  const none = summaryView(fr, "fr", {
    precheck: precheck([]),
    detection,
    online: onlineMixed(),
    treatments: new Set(),
    stackTargets: new Set(),
    resume: false,
    privateServer: false,
    date: "10 oct.",
  });
  assert.ok(
    !none.groups
      .flatMap((group) => group.rows)
      .some((row) => text(row.segs).includes(STACKED_430)),
  );
});
