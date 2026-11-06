import assert from "node:assert/strict";
import test from "node:test";

import {
  filesView,
  findingSegs,
  type FileLine,
  type Seg,
  type TargetCard,
} from "./wizardFiles.ts";
import {
  IC,
  M31,
  M31_LIGHTS,
  NGC,
  detectionAllFound,
  detectionMixed,
  onlineMixed,
  online,
  plansMixed,
  capture,
  image,
  target,
} from "./homeFixtures.ts";
import { createTranslator } from "../i18n/locale.ts";

const fr = createTranslator("fr");
const text = (segs: readonly Seg[]) =>
  segs
    .map((s) => (typeof s === "string" ? s : "strong" in s ? s.strong : s.code))
    .join("");
/** The no-break spaces of the French text read as plain ones. */
const spaced = (line: string) => line.replace(/\s+/gu, " ");
const lines = (list: readonly FileLine[]) =>
  list.map((l) => `${l.glyph} ${text(l.segs)}`);
/** The final image, the frames, then the other findings. */
const all = (card: TargetCard): FileLine[] => [
  ...card.final.lines,
  ...card.frames.lines,
  ...card.lines,
];

test("everything found: per target its final image and its frames, then the calibration and the logs", () => {
  const view = filesView(fr, "fr", {
    detection: detectionAllFound(),
    online: online("night-7", [NGC, IC, M31]),
    plans: [],
    added: false,
    eveningDay: "7 oct.",
  });
  assert.equal(
    view.summary,
    "Lu dans le dossier de la nuit et dans les en-têtes FITS ; rien n’est déplacé.",
  );
  assert.deepEqual(
    view.targets.map((c) => [c.designation, c.badge.key]),
    [
      ["NGC 7000", "ready"],
      ["IC 1396", "ready"],
      ["M31", "ready"],
    ],
  );
  assert.deepEqual(lines(all(view.targets[0])), [
    "ok final/ · NGC7000_final.tif · 412 Mo",
    "ok lights/ · 40 poses · 180 s · Ha 20, OIII 20 · gain 100 · 1,3 Go",
  ]);
  assert.deepEqual(
    lines(all(view.targets[2]))[1],
    "ok lights/ · 42 poses · 120 s · sans filtre · gain 100 · 1,3 Go",
  );
  assert.deepEqual(lines(view.calibration!), [
    "ok darks/ · 30 poses · 180 s et 120 s · gain 100",
    "ok flats/ · 50 poses · Ha 25, OIII 25",
    "ok biases/ · 50 poses",
    "ok dark-flats/ · vide : les biases corrigent les flats",
  ]);
  assert.deepEqual(lines(view.logs.lines), [
    "ok logs/ · NINA · journal de séquence du 7 oct. : paliers, mises au point",
    "ok logs/ · PHD2 · PHD2_GuideLog_2026-10-07.txt\u00a0: guidage",
  ]);
  assert.equal(view.footer, null);
});

test("missing items: what and where in one line; the missing final image in amber, calibration in amber, no logs in grey", () => {
  const detection = detectionAllFound();
  detection.targets[1] = target(
    IC,
    38,
    detection.targets[1].lightSummary,
    null,
  );
  detection.calibration = { darks: 0, flats: 0, darkFlats: 0, biases: 0 };
  detection.logs = [];
  detection.findings = [
    {
      key: "detection.finalMissing",
      severity: "warning",
      targetId: IC,
      params: { target: "IC 1396", folder: "IC 1396/final/" },
      files: [],
    },
    {
      key: "detection.flatsMissing",
      severity: "warning",
      targetId: IC,
      params: {
        target: "IC 1396",
        filter: "OIII",
        none: false,
        stackedWithout: true,
      },
      files: [],
    },
    {
      key: "detection.darksMissing",
      severity: "warning",
      targetId: M31,
      params: {
        target: "M31",
        exposureSeconds: 120,
        gain: 100,
        none: false,
        count: 2,
        found: [{ exposureSeconds: 300, gain: 100 }],
      },
      files: [],
    },
    {
      key: "detection.noLogs",
      severity: "info",
      targetId: null,
      params: {},
      files: [],
    },
  ];
  const view = filesView(fr, "fr", {
    detection,
    online: null,
    plans: [],
    added: false,
    eveningDay: "7 oct.",
  });
  assert.equal(view.summary, "2 cibles prêtes sur 3 · 2 avertissements");
  assert.deepEqual(view.targets[1].badge, {
    tone: "warning",
    key: "finalMissing",
  });
  assert.equal(view.targets[1].final.state.tone, "warning");
  assert.equal(view.targets[1].frames.state.tone, "success");
  // The flats and the darks that are missing are said once, in the
  // Calibration card, not under each target.
  assert.deepEqual(lines(all(view.targets[1])), [
    "warning Aucune image finale : seules les poses FITS seront utilisées pour créer une image finale (empilement).",
    "ok lights/ · 38 poses · 180 s · Ha 19, OIII 19 · gain 100 · 1,2 Go",
  ]);
  assert.equal(lines(all(view.targets[2])).length, 2);
  assert.deepEqual(
    lines(view.calibration ?? []).map(spaced),
    [
      "warning Pas de dark de 120 s · gain 100 pour M31 : les 2 darks du dossier sont de 300 s.",
      "warning Pas de flat pour le filtre OIII.",
    ].map(spaced),
  );
  assert.deepEqual(
    view.calibration?.map((line) => [line.banner, line.action]),
    [
      [true, "openDarks"],
      [true, "openFlats"],
    ],
    "an amber block with a link that opens the folder",
  );
  assert.ok(view.logs.empty);
  assert.equal(
    view.footer,
    "IC 1396 partira sans capture tant que son image finale manque ; l’empilement, si vous le cochez, peut la produire.",
  );
});

test("misfiled frames from the headers: from where to where, the assistant offered, outside the structure", () => {
  const detection = detectionAllFound();
  detection.findings = [
    {
      key: "detection.framesMisfiled",
      severity: "warning",
      targetId: null,
      params: {
        count: 3,
        kind: "flat",
        header: "FLAT",
        from: "M31/lights/",
        to: "calibration/flats/",
        fromLights: true,
      },
      files: [],
    },
    {
      key: "detection.framesOtherTarget",
      severity: "warning",
      targetId: M31,
      params: { count: 2, object: "NGC 7000", folder: "M31/lights/" },
      files: [],
    },
  ];
  detection.outsideStructure = [
    "Light_NGC7000_0041.fits",
    "a.fits",
    "b.fits",
    "c.fits",
  ];
  const view = filesView(fr, "fr", {
    detection,
    online: null,
    plans: [],
    added: false,
    eveningDay: "7 oct.",
  });
  assert.equal(
    view.summary,
    "NGC 7000 et IC 1396 prêtes · 2 rangements à faire pour M31",
  );
  const m31 = view.targets[2];
  assert.deepEqual(m31.badge, { tone: "warning", key: "tidy", count: 2 });
  assert.ok(m31.askAssistant);
  assert.deepEqual(lines(m31.lines), [
    "warning 3 flats rangés dans M31/lights/ : leur en-tête dit FLAT. À déplacer dans calibration/flats/.",
    "warning 2 poses de NGC 7000 dans M31/lights/ (en-tête OBJECT). À déplacer dans NGC 7000/lights/.",
  ]);
  assert.equal(
    text(view.outside!),
    "4 fichiers à la racine de la nuit : Light_NGC7000_0041.fits et 3 autres. À ranger dans le dossier de leur cible.",
  );
});

test("SHA-256: identical online is not sent again; a different image is a variant of the capture, never a « révision »", () => {
  const view = filesView(fr, "fr", {
    detection: detectionMixed(),
    online: onlineMixed(),
    plans: plansMixed(),
    added: false,
    eveningDay: "7 oct.",
  });
  assert.equal(
    view.summary,
    "NGC 7000 prête · IC 1396 déjà en ligne · M31 : une variante à envoyer",
  );
  assert.equal(
    lines(all(view.targets[0]))[0],
    "ok final/ · NGC7000_final.tif · 412 Mo · pas encore en ligne",
  );
  assert.equal(
    lines(all(view.targets[1]))[0],
    "ok IC1396_final.png · 96 Mo : déjà en ligne, identique (empreinte SHA-256 5d20…b9c3). Elle ne sera pas renvoyée.",
  );
  const m31 = view.targets[2];
  assert.deepEqual(m31.badge, { tone: "success", key: "online", phone: true });
  assert.deepEqual(lines(all(m31)).slice(0, 2), [
    "phone Image finale déjà en ligne : M31_final.jpg, envoyée depuis le téléphone le 8 oct. · 14 Mo. Elle est conservée.",
    "variant M31_final_v2.jpg · 15 Mo, différente de celle en ligne (empreinte SHA-256 8c1f…e2a7) : elle sera envoyée comme variante de la capture.",
  ]);
  assert.ok(m31.final.lines[1].framed);
  for (const card of view.targets) {
    for (const line of lines(all(card)))
      assert.ok(!/révision|version/u.test(line), line);
  }
});

test("a resumed night: the image online kept, frames and logs « Nouveau »", () => {
  const detection = detectionAllFound();
  detection.targets = [
    target(
      M31,
      96,
      { ...M31_LIGHTS, filters: [{ filter: null, frames: 96 }], bytes: 0 },
      null,
      "online",
    ),
  ];
  const view = filesView(fr, "fr", {
    detection,
    online: online(
      "night-5",
      [M31],
      [
        capture(
          M31,
          image("M31_final.jpg", "aa", 14_000_000, {
            source: "phone",
            solved: true,
          }),
        ),
      ],
    ),
    plans: [],
    added: true,
    eveningDay: "5 oct.",
  });
  assert.equal(
    view.summary,
    "Les journaux et les poses donnent le rapport de la nuit : paliers, qualité, guidage.",
  );
  assert.deepEqual(lines(all(view.targets[0])), [
    "phone Image finale en ligne : M31_final.jpg, envoyée depuis le téléphone · annotations du ciel en ligne. Conservées.",
    "ok lights/ · 96 poses · 120 s · sans filtre · gain 100",
  ]);
  assert.ok(view.targets[0].frames.lines[0].isNew);
  assert.ok(view.logs.lines.every((l) => l.isNew));
});

test("the four cases of a target: FITS only, final image only, both, neither", () => {
  const detection = detectionAllFound();
  const fits = detection.targets[0].lightSummary;
  detection.targets = [
    target(NGC, 40, fits, null), // FITS only
    target(IC, 0, fits, { name: "IC1396_final.png", bytes: 96_000_000 }), // final only
    target(M31, 42, fits, { name: "M31_final.tif", bytes: 50_000_000 }), // both
    {
      ...target(NGC, 0, fits, null),
      targetId: "t-empty",
      designation: "M33",
      folderName: "M33",
    }, // neither
  ];
  detection.findings = [];
  const view = filesView(fr, "fr", {
    detection,
    online: null,
    plans: [],
    added: false,
    eveningDay: "7 oct.",
  });
  const [only, final, both, neither] = view.targets;

  // FITS only: the final image is an amber warning, not an error.
  assert.deepEqual(only.badge, { tone: "warning", key: "finalMissing" });
  assert.deepEqual(only.final.state, { tone: "warning", key: "missing" });
  assert.deepEqual(only.frames.state, { tone: "success", key: "found" });
  assert.deepEqual(lines(only.final.lines), [
    "warning Aucune image finale : seules les poses FITS seront utilisées pour créer une image finale (empilement).",
  ]);

  // Final image only: the frames are an amber warning.
  assert.deepEqual(final.badge, { tone: "warning", key: "framesMissing" });
  assert.deepEqual(final.frames.state, { tone: "warning", key: "missing" });
  assert.deepEqual(final.final.state, { tone: "success", key: "found" });
  assert.deepEqual(lines(final.frames.lines), [
    "warning Aucune pose FITS : seule l’image finale sera envoyée.",
  ]);

  // Both: two green frames.
  assert.deepEqual(both.badge, { tone: "success", key: "ready" });
  assert.equal(both.final.state.tone, "success");
  assert.equal(both.frames.state.tone, "success");

  // Neither: the red error, in each frame and for the target.
  assert.deepEqual(neither.badge, { tone: "error", key: "empty" });
  assert.deepEqual(neither.final.state, { tone: "error", key: "missing" });
  assert.deepEqual(neither.frames.state, { tone: "error", key: "missing" });
  assert.deepEqual(lines(neither.lines), [
    "missing Ni pose FITS ni image finale : déposez les poses dans M33/lights/ ou une image finale (TIFF, JPEG ou PNG) dans M33/final/.",
  ]);
  // Only the FITS-only target is announced as leaving without a capture.
  assert.match(view.footer ?? "", /^NGC 7000 partira sans capture/u);
  assert.match(view.summary, /2 cibles prêtes sur 4/u);
});

test("the target's messages for the assistant keep the new findings readable", () => {
  const detection = detectionAllFound();
  detection.findings = [
    {
      key: "detection.framesMissing",
      severity: "warning",
      targetId: IC,
      params: { target: "IC 1396" },
      files: [],
    },
    {
      key: "detection.targetEmpty",
      severity: "error",
      targetId: M31,
      params: { lightsFolder: "M31/lights/", folder: "M31/final/" },
      files: [],
    },
  ];
  const view = filesView(fr, "fr", {
    detection,
    online: null,
    plans: [],
    added: false,
    eveningDay: "7 oct.",
  });
  // Said in the frames, never twice among the other lines.
  for (const card of view.targets) assert.deepEqual(card.lines, []);
});

test("frames outside the night's window: a warning line, the frames still found, nothing to tidy", () => {
  const detection = detectionAllFound();
  detection.findings = [
    {
      key: "detection.framesOtherNight",
      severity: "warning",
      targetId: NGC,
      params: { count: 40, date: "2026-10-06", folder: "NGC 7000/lights/" },
      files: [],
    },
  ];
  const view = filesView(fr, "fr", {
    detection,
    online: null,
    plans: [],
    added: false,
    eveningDay: "7 oct.",
  });
  const card = view.targets[0];
  assert.deepEqual(card.frames.state, { tone: "success", key: "found" });
  assert.notEqual(card.badge.tone, "error");
  assert.notEqual(card.badge.key, "tidy");
  assert.equal(card.askAssistant, false);
  assert.deepEqual(lines(card.lines), [
    "warning 40 poses hors de la nuit (nuit du 6 oct.), écartées de l’intégration\u00a0; elles restent comptées dans NGC 7000/lights/.",
  ]);
});

// Darks taken in the ASIAIR's « Light » mode sit in
// calibration/darks/ with a LIGHT header. The folder is right: they are
// counted as darks, and one warning asks to check. Nothing is « à ranger ».
test("a calibration folder whose headers say another type: counted by the folder, a warning in the calibration zone, no tidying", () => {
  const detection = detectionAllFound();
  detection.calibration = { darks: 20, flats: 0, darkFlats: 0, biases: 0 };
  detection.calibrationSummary = {
    darks: {
      filters: [{ filter: null, frames: 20 }],
      exposures: [90],
      gains: [100],
      bytes: 0,
    },
    flats: { filters: [], exposures: [], gains: [], bytes: 0 },
  };
  detection.findings = [
    {
      key: "detection.calibrationHeaderDiffers",
      severity: "warning",
      targetId: null,
      params: {
        count: 20,
        folder: "calibration/darks/",
        header: "LIGHT",
        kind: "dark",
      },
      files: [],
    },
  ];
  const view = filesView(fr, "fr", {
    detection,
    online: null,
    plans: [],
    added: false,
    eveningDay: "7 oct.",
  });
  assert.deepEqual(lines(view.calibration!), [
    "ok darks/ · 20 poses · 90 s · gain 100",
    "none Aucun flat\u00a0: la qualité se mesure quand même.",
    "warning 20 poses de calibration/darks/ ont un en-tête LIGHT : elles sont lues comme des darks. Vérifiez qu’il ne s’agit pas d’une erreur de rangement.",
  ]);
  // The warning is in the calibration zone only: no target card carries it,
  // and no target is « à ranger ».
  for (const card of view.targets) {
    assert.notEqual(card.badge.key, "tidy");
    assert.ok(!card.askAssistant);
    assert.ok(!lines(card.lines).some((l) => l.includes("LIGHT")));
  }
  const [warning] = view.calibration!.slice(-1);
  assert.deepEqual(warning.segs[2], { code: "calibration/darks/" });
});

test("the calibration header warning reads in the ten languages, singular and plural, with no placeholder left", () => {
  for (const locale of [
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
  ] as const) {
    const t = createTranslator(locale);
    for (const [count, kind] of [
      [1, "flat"],
      [20, "dark-flat"],
      [3, "bias"],
    ] as const) {
      const segs = findingSegs(t, locale, {
        key: "detection.calibrationHeaderDiffers",
        severity: "warning",
        targetId: null,
        params: {
          count,
          folder: "calibration/x/",
          header: "LIGHT",
          kind,
        },
        files: [],
      });
      const sentence = text(segs);
      assert.ok(!/[{}]/.test(sentence), `${locale}: ${sentence}`);
      assert.ok(sentence.includes("LIGHT"), `${locale}: ${sentence}`);
      assert.ok(sentence.includes(String(count)), `${locale}: ${sentence}`);
      assert.ok(sentence.includes("calibration/x/"), `${locale}: ${sentence}`);
    }
  }
});

test("a target whose device stacked an image does not promise « seules les poses FITS »: the device's image is the source, and only the poses it does not cover are stacked", () => {
  const detection = detectionAllFound();
  const fits = detection.targets[0].lightSummary;
  const seestar = target(M31, 466, fits, null);
  seestar.device = {
    images: [
      {
        relativePath: "M31/device/Stacked_430.fit",
        fileName: "Stacked_430.fit",
        bytes: 1,
        device: "Seestar S50 Pro",
        poses: 430,
        exposureSeconds: 30,
        totalSeconds: 12900,
        filter: "IRCUT",
        gain: 200,
        width: 2160,
        height: 3840,
        solved: false,
        family: "IRCUT_30s_g200",
      },
    ],
    families: [
      {
        label: "IRCUT_30s_g200",
        filter: "IRCUT",
        exposureSeconds: 30,
        gain: 200,
        lights: 430,
        images: ["Stacked_430.fit"],
        coverage: "covers",
        defaultSource: "device",
        useFile: "Stacked_430.fit",
        asidePoses: 0,
      },
    ],
    jpgs: [],
  };
  detection.targets = [seestar];
  detection.findings = [];
  const view = filesView(fr, "fr", {
    detection,
    online: null,
    plans: [],
    added: false,
    eveningDay: "10 oct.",
  });
  const [card] = view.targets;
  const said = spaced(lines(card.final.lines).join("\n"));
  assert.match(said, /l’image de l’appareil sera utilisée comme source/u);
  assert.match(said, /les 36 autres poses FITS seront empilées/u);
  assert.doesNotMatch(said, /seules les poses FITS/u);
  // The finding of the detection says the same.
  const finding = {
    key: "detection.finalFromDevice",
    severity: "warning" as const,
    targetId: M31,
    params: { restackFrames: 0 },
    files: [],
  };
  assert.equal(
    text(findingSegs(fr, "fr", finding)),
    "Aucune image finale\u00a0: l’image de l’appareil sera utilisée comme source.",
  );
});
