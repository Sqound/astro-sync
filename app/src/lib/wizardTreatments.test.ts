import assert from "node:assert/strict";
import test from "node:test";

import { readFileSync } from "node:fs";

import {
  DEFAULT_CHOICES,
  tickedTreatmentsOf,
  treatmentsView,
} from "./wizardTreatments.ts";
import type { Seg } from "./wizardFiles.ts";
import type { PipelineFinish, TargetFinishes } from "./finish.ts";
import type { IntegrationPreview, ToolRow } from "./bridge";
import {
  IC,
  M31,
  NGC,
  PIPELINES,
  previews,
  detectionAllFound,
  detectionMixed,
  onlineMixed,
  plansMixed,
  tools,
  treatments,
} from "./homeFixtures.ts";
import { createTranslator } from "../i18n/locale.ts";

const fr = createTranslator("fr");
const text = (segs: readonly Seg[]) =>
  segs
    .map((s) => (typeof s === "string" ? s : "strong" in s ? s.strong : s.code))
    .join("");
function viewOf(options: {
  reserved?: boolean;
  missing?: { stacking?: boolean; astap?: boolean };
  variants?: boolean;
  privateServer?: string | null;
  stackTargets?: string[];
  previews?: Record<string, IntegrationPreview>;
  targetRenders?: Record<string, string[]>;
  tools?: ToolRow[];
  progress?: Record<string, number | null>;
  targetFinishes?: TargetFinishes;
  finishDefault?: PipelineFinish;
}) {
  return treatmentsView(fr, "fr", {
    treatments: treatments(options.reserved, options.missing),
    tools: options.tools ?? tools({ astap: options.missing?.astap }),
    detection: detectionMixed(),
    online: onlineMixed(),
    plans: plansMixed(options.variants ?? true),
    variantsAllowed: options.variants ?? true,
    privateServer: options.privateServer ?? null,
    pipelines: PIPELINES,
    choices: {
      ...DEFAULT_CHOICES,
      stackTargets: new Set(options.stackTargets ?? []),
      targetRenders: options.targetRenders ?? {},
      targetFinishes: options.targetFinishes,
    },
    previews: options.previews,
    eveningDate: "2026-10-07",
    progress: options.progress,
    finishDefault: options.finishDefault,
  });
}

test("Synchronisation avancée+: the five options ticked, each with its tool, no word about the offer", () => {
  const view = viewOf({});
  assert.equal(view.reservedLine, false);
  assert.deepEqual(
    view.options.map((o) => [fr(o.titleKey), o.state, o.ticked, text(o.tool)]),
    [
      [
        "Image empilée de la nuit (à partir des FITS)",
        "available",
        true,
        "Pipeline Python d’Astro Sync · outil présent",
      ],
      [
        "Envoi de l’image finale déjà traitée",
        "available",
        true,
        "Sans outil : le fichier de final/ part tel quel, avec son empreinte SHA-256.",
      ],
      [
        "Annotations du ciel",
        "available",
        true,
        "ASTAP 2026.09 · plate solving sur cet ordinateur, environ 20 s par image",
      ],
      [
        "Journaux",
        "available",
        true,
        "Lecteurs NINA, PHD2 et ASIAIR d’Astro Sync · aucun outil à installer",
      ],
      [
        "Progression",
        "available",
        true,
        "Siril 1.4.0 · empilement intégré, réglages fixes",
      ],
    ],
  );
  const final = view.options[1];
  assert.deepEqual(
    final.lines.map((l) => text(l.segs)),
    [
      "NGC 7000 · NGC7000_final.tif · 412 Mo · sera envoyée",
      "IC 1396 · déjà en ligne, identique : non renvoyée",
      "M31 · M31_final_v2.jpg · 15 Mo, différente de celle en ligne : sera envoyée comme variante de la capture",
    ],
  );
  assert.equal(
    text(view.options[3].lines[0].segs),
    "2 journaux trouvés dans logs/ : NINA, PHD2",
  );
});

test("a variant announced: ticking M31's stacking says it in bold, in its card; one variant only", () => {
  const view = viewOf({ stackTargets: [M31] });
  const stacking = view.options[0];
  assert.deepEqual(
    stacking.cards.map((c) => [c.designation, c.ticked, c.line]),
    [
      ["NGC 7000", false, "40 poses"],
      ["IC 1396", false, "38 poses"],
      ["M31", true, "42 poses"],
    ],
  );
  const notes = (index: number) =>
    stacking.cards[index].notes.map((n) => [text(n.segs), n.tone]);
  assert.deepEqual(notes(0), [], "unticked, nothing said of its final image");
  assert.deepEqual(notes(1), [
    [
      "Une image finale est en ligne\u00a0: une variante serait créée.",
      "muted",
    ],
  ]);
  assert.deepEqual(notes(2), [
    [
      "M31 a déjà une image finale en ligne\u00a0: une variante de la capture sera créée.",
      "strong",
    ],
    ["L’image en ligne est conservée.", "muted"],
    [
      "L’image empilée remplace, pour M31, l’envoi de M31_final_v2.jpg\u00a0: une seule variante est créée.",
      "variant",
    ],
  ]);
  assert.equal(stacking.severalRenders, true);
  assert.equal(view.contrastShown, true);
  assert.equal(viewOf({}).contrastShown, false, "no target ticked: no slider");
  assert.equal(
    text(view.options[1].lines[2].segs),
    "M31 · M31_final_v2.jpg laissée de côté : l’image empilée est envoyée à la place",
  );
});
test("one card per target -- band, frames and duration, its pipelines (defaults by band), the button's level, its integration; unticked, greyed with its choice kept", () => {
  const view = viewOf({
    stackTargets: [NGC, IC],
    previews: previews(),
    targetRenders: { [IC]: ["polished", "hoo"], [M31]: ["polished"] },
  });
  const cards = view.options[0].cards;
  assert.deepEqual(
    cards.map((c) => [c.designation, c.bandLabel, c.line, c.ticked, c.button]),
    [
      [
        "NGC 7000",
        "Bibande",
        "40 poses · 2 h 00 · 1 famille",
        true,
        "tertiary",
      ],
      [
        "IC 1396",
        "Large bande",
        "38 poses · 1 h 16 · 1 famille",
        true,
        "tertiary",
      ],
      [
        "M31",
        "Large bande",
        "42 poses · 1 h 24 · 1 famille",
        false,
        "tertiary",
      ],
    ],
  );
  const chips = (index: number) =>
    cards[index].chips.map((chip) => [chip.name, chip.warning]);
  // NGC 7000 (dual-band) takes the natural render and the correction.
  assert.deepEqual(chips(0), [
    ["Rendu naturel", null],
    ["Correction bibande", null],
  ]);
  // A pipeline discouraged for the filter, chosen all the same: said, never
  // refused; GraXpert missing: the polished render says so.
  assert.deepEqual(chips(1), [
    ["Palette HOO", "déconseillé pour ce filtre"],
    ["Rendu soigné", "sans GraXpert"],
  ]);
  // Unticked: its choice is kept, and its integration is not said.
  assert.deepEqual(chips(2), [["Rendu soigné", "sans GraXpert"]]);
  assert.equal(cards[2].integration, null);
  assert.deepEqual(
    cards[1].integration?.families.map((f) => f.text),
    ["UV/IR-cut · 38 × 120 s · gain 100 · bin 1 · −10 °C"],
  );
  assert.match(cards[1].integration?.outside ?? "", /^1 pose hors de la nuit/u);
  assert.deepEqual(
    cards[0].notes.map((n) => text(n.segs)),
    ["L’image empilée sera envoyée à la place de NGC7000_final.tif."],
  );
  assert.deepEqual(cards[0].notes[0].segs[1], { strong: "NGC7000_final.tif" });
  // GraXpert downloading: the label follows its percent.
  const downloading = viewOf({
    stackTargets: [IC],
    previews: previews(),
    targetRenders: { [IC]: ["polished"] },
    progress: { graxpert: 48 },
  }).options[0].cards[1];
  assert.deepEqual(downloading.chips[0].warning, "GraXpert · 48\u00a0%");
  // A space that keeps one image: one pipeline, the correction on a
  // dual-band target.
  const one = viewOf({
    variants: false,
    stackTargets: [NGC],
    previews: previews(),
  });
  assert.deepEqual(one.options[0].cards[0].chosen, ["dualband-correction"]);
  assert.equal(one.options[0].severalRenders, false);
});

test("Synchronisation avancée: two options greyed behind one line and « Voir les offres », the stacking open; the different M31 image is not sent", () => {
  const view = viewOf({ reserved: true, variants: false });
  assert.equal(view.reservedLine, true);
  assert.equal(
    fr("wizard.treatments.reserved"),
    "Journaux et progression sont réservés à Synchronisation avancée+.",
  );
  assert.deepEqual(
    view.options.map((o) => [o.state, o.ticked]),
    [
      ["available", true],
      ["available", true],
      ["available", true],
      ["reserved", false],
      ["reserved", false],
    ],
  );
  // The stacking is open: its tool is said present.
  assert.equal(
    text(view.options[0].tool),
    "Pipeline Python d’Astro Sync · outil présent",
  );
  assert.equal(
    text(view.options[3].tool),
    "Lecteurs NINA, PHD2 et ASIAIR d’Astro Sync",
  );
  const final = view.options[1];
  assert.ok(final.offersLink);
  assert.equal(final.lines[2].glyph, "warning");
  assert.match(
    text(final.lines[2].segs),
    /^M31 · M31_final_v2\.jpg est différente de l’image en ligne\. .* elle ne sera pas envoyée\.$/u,
  );
});

test("a private server: everything open, no offer named; a missing tool: greyed, « introuvable », Dépendances", () => {
  const priv = viewOf({
    privateServer: "https://astro.exemple.test",
    stackTargets: [NGC],
  });
  assert.equal(priv.reservedLine, false);
  assert.equal(priv.privateServer, "https://astro.exemple.test");
  assert.ok(
    priv.options.every((o) => o.state === "available" && !o.offersLink),
  );
  assert.equal(
    text(priv.options[1].lines[0].segs),
    "NGC 7000 · NGC7000_final.tif · 412 Mo · remplacée par l’image empilée",
  );

  const missing = viewOf({ missing: { stacking: true, astap: true } });
  assert.deepEqual(
    missing.options.map((o) => o.state),
    ["toolMissing", "available", "toolMissing", "available", "available"],
  );
  assert.equal(
    text(missing.options[0].tool),
    "Pipeline Python d’Astro Sync · introuvable",
  );
  assert.equal(
    missing.options[0].description,
    "L’outil n’est pas vérifié sur cet ordinateur : l’option est grisée.".replace(
      " :",
      " :",
    ),
  );
  assert.equal(text(missing.options[2].tool), "ASTAP · introuvable");
  assert.equal(fr("wizard.treatments.dependencies"), "Réglages › Dépendances");
  assert.equal(fr("wizard.treatments.toolMissing"), "À installer");
});

test("the stacking option follows its tool: Siril or Python missing, the cards stay greyed with the reason and each tool offers its download", () => {
  const missingTools: ToolRow[] = [
    ...tools({}).map((row) =>
      row.id === "siril"
        ? { ...row, state: "missing" as const, downloadBytes: 75_765_778 }
        : row,
    ),
    {
      id: "python-env",
      name: { fr: "Environnement Python", en: "Python environment" },
      kind: "python-packages",
      required: true,
      state: "missing",
      version: null,
      minVersion: null,
      path: null,
      page: "https://www.python.org/downloads/",
      licence: null,
      progressionCompatible: null,
      downloadBytes: 81_841_919,
      installable: true,
    },
  ];
  const option = treatmentsView(fr, "fr", {
    treatments: treatments(false, { stacking: true }),
    tools: missingTools,
    detection: detectionMixed(),
    online: onlineMixed(),
    plans: plansMixed(true),
    variantsAllowed: true,
    privateServer: null,
    pipelines: PIPELINES,
    choices: { ...DEFAULT_CHOICES, stackTargets: new Set([NGC]) },
  }).options[0];
  assert.equal(option.state, "toolMissing");
  assert.equal(option.cards.length, 3, "the cards stay");
  for (const card of option.cards) {
    assert.equal(card.enabled, false);
    assert.equal(card.ticked, false);
    assert.equal(card.button, null);
    assert.deepEqual(card.chips, []);
    assert.equal(
      card.noPipeline?.replaceAll("\u00a0", " "),
      "Aucun pipeline disponible tant que Siril et l’environnement Python ne sont pas installés.",
    );
  }
  assert.deepEqual(
    option.toInstall.map((pill) => [
      pill.name,
      pill.stateLabel,
      pill.action?.label,
    ]),
    [
      ["Siril", "Absent", "Télécharger (75,7 Mo)"],
      ["Python", "Absent", "Installer (81,8 Mo)"],
    ],
  );
});
test("a target without FITS frames has no card; one with a final image only still has its final image treated; no frame at all says so", () => {
  const detection = detectionMixed();
  detection.targets[0] = { ...detection.targets[0], lightFrames: 0 };
  const view = treatmentsView(fr, "fr", {
    treatments: treatments(),
    tools: tools({}),
    detection,
    online: onlineMixed(),
    plans: plansMixed(true),
    variantsAllowed: true,
    privateServer: null,
    pipelines: PIPELINES,
    choices: { ...DEFAULT_CHOICES, stackTargets: new Set() },
  });
  const stacking = view.options.find((o) => o.treatment === "stacking")!;
  assert.equal(stacking.state, "available");
  assert.deepEqual(
    stacking.cards.map((c) => c.designation),
    ["IC 1396", "M31"],
  );
  assert.equal(stacking.noFrames, false);
  detection.targets = detection.targets.map((target) => ({
    ...target,
    lightFrames: 0,
  }));
  const none = treatmentsView(fr, "fr", {
    treatments: treatments(),
    tools: tools({}),
    detection,
    online: onlineMixed(),
    plans: plansMixed(true),
    variantsAllowed: true,
    privateServer: null,
    pipelines: PIPELINES,
    choices: DEFAULT_CHOICES,
  }).options[0];
  assert.equal(none.noFrames, true);
  assert.deepEqual(none.cards, []);
  assert.equal(
    fr("wizard.stacking.noFrames"),
    "Aucune pose FITS dans lights/\u00a0: il n’y a rien à empiler.",
  );
});
test("« Sans traitement »: the core's ten-language texts (variant title and description) are the wizard's words", () => {
  const rust = readFileSync(
    new URL(
      "../../../crates/astro-sync-core/src/pipelines.rs",
      import.meta.url,
    ),
    "utf8",
  );
  const table = rust.slice(rust.indexOf("const LINEAR_TEXTS"));
  const rows = [
    ...table
      .slice(0, table.indexOf("];"))
      .matchAll(/\(\s*"([^"]+)",\s*"([^"]+)",\s*"([^"]+)",?\s*\)/gu),
  ].map(([, code, name, help]) => [
    code,
    name,
    help.replaceAll("\\u{a0}", "\u00a0"),
  ]);
  assert.equal(rows.length, 10);
  for (const [code, name, help] of rows) {
    const words = createTranslator(code);
    assert.equal(words("wizard.treatments.noProcessing"), name, code);
    assert.equal(words("wizard.treatments.noProcessingHelp"), help, code);
  }
});

// ---- The sky annotations after the stacking, and the badge that says why an
// option cannot be ticked.

const NO_IMAGE_NIGHT = () => {
  const detection = detectionMixed();
  detection.targets = detection.targets.map((item) => ({
    ...item,
    finalImage: null,
  }));
  return { detection, online: { ...onlineMixed(), targets: [] } };
};

function noImageView(choices: Partial<typeof DEFAULT_CHOICES>) {
  const { detection, online } = NO_IMAGE_NIGHT();
  return treatmentsView(fr, "fr", {
    treatments: treatments(),
    tools: tools({}),
    detection,
    online,
    plans: [],
    variantsAllowed: true,
    privateServer: null,
    pipelines: PIPELINES,
    choices: { ...DEFAULT_CHOICES, ...choices },
  });
}

const optionOf = (view: ReturnType<typeof noImageView>, name: string) =>
  view.options.find((option) => option.treatment === name)!;

test("no final image but frames to stack -- the sky annotations hold on the stacking, with its badge and its line", () => {
  const view = noImageView({ stackTargets: new Set([M31]) });
  const sky = optionOf(view, "sky_annotations");
  assert.equal(sky.state, "available");
  assert.equal(sky.ticked, true);
  assert.deepEqual(sky.badge, {
    tone: "neutral",
    label: "Après l’empilement",
    icon: null,
  });
  assert.match(text(sky.lines[0].segs), /^Après l’empilement : la solution/);
});

test("the stacking unticked, or no target ticked for it, leaves nothing to annotate", () => {
  for (const choices of [
    { stackTargets: new Set<string>() },
    { stackTargets: new Set([M31]), unticked: new Set(["stacking" as const]) },
  ]) {
    const sky = optionOf(noImageView(choices), "sky_annotations");
    assert.equal(sky.state, "nothing");
    assert.equal(sky.ticked, false);
    assert.equal(sky.badge?.label, "Rien à annoter");
    assert.equal(sky.lines.length, 0);
  }
});

test("a final image needs no stacking for the sky annotations", () => {
  const view = viewOf({});
  const sky = optionOf(view, "sky_annotations");
  assert.equal(sky.state, "available");
  assert.equal(sky.badge, null);
});

test("one badge for each state of the core -- reserved, tool to install, nothing to treat, after the stacking", () => {
  const reserved = optionOf(viewOf({ reserved: true }), "progression");
  assert.deepEqual(reserved.badge, {
    tone: "neutral",
    label: "Synchronisation avancée+",
    icon: "lock",
  });
  const missing = optionOf(
    viewOf({ missing: { astap: true } }),
    "sky_annotations",
  );
  assert.deepEqual(missing.badge, {
    tone: "warning",
    label: "À installer",
    icon: "warning",
  });
  const nothing = (name: string) =>
    treatmentsView(fr, "fr", {
      treatments: treatments().map((item) =>
        item.treatment === name
          ? {
              ...item,
              available: false,
              unavailable: { why: "nothing_to_treat" as const },
            }
          : item,
      ),
      tools: tools({}),
      detection: detectionMixed(),
      online: onlineMixed(),
      plans: [],
      variantsAllowed: true,
      privateServer: null,
      pipelines: PIPELINES,
      choices: DEFAULT_CHOICES,
    }).options.find((option) => option.treatment === name)!.badge;
  assert.deepEqual(nothing("final_image"), {
    tone: "neutral",
    label: "Absent du dossier",
    icon: null,
  });
  assert.equal(nothing("logs")?.label, "Aucun journal");
  assert.equal(nothing("stacking")?.label, "Aucune pose");
  assert.equal(nothing("progression")?.label, "Aucune pose");
  // An option that can be ticked carries no badge, and none is red.
  assert.ok(viewOf({}).options.every((option) => option.badge === null));
});

test("« Journaux » is ticked by default when the night holds PHD2 logs, and can be unticked", () => {
  const detection = detectionAllFound();
  assert.ok(detection.logs.some((log) => log.kind === "phd2"));
  const ticked = tickedTreatmentsOf(
    treatments(),
    detection,
    null,
    DEFAULT_CHOICES,
  );
  assert.ok(ticked.has("logs"));
  const unticked = tickedTreatmentsOf(treatments(), detection, null, {
    ...DEFAULT_CHOICES,
    unticked: new Set(["logs"]),
  });
  assert.ok(!unticked.has("logs"));
  // Reserved (the space may not send a report): never ticked.
  assert.ok(
    !tickedTreatmentsOf(treatments(true), detection, null, DEFAULT_CHOICES).has(
      "logs",
    ),
  );
});

test("a target's card says each finish that differs from the default, one line per pipeline, nothing when all follow it", () => {
  const strong = { contrast: 3.6, colour: 20, softStars: 50 };
  const base = {
    stackTargets: [NGC],
    previews: previews(),
  };
  // Everything at the default: nothing more on the card.
  assert.deepEqual(viewOf(base).options[0].cards[0].finishes, []);
  // A finish set by hand to the default's own values says nothing either.
  assert.deepEqual(
    viewOf({
      ...base,
      targetFinishes: {
        [NGC]: { raw: { contrast: 2, colour: 0, softStars: 0 } },
      },
    }).options[0].cards[0].finishes,
    [],
  );
  const card = viewOf({
    ...base,
    targetFinishes: {
      [NGC]: {
        "dualband-correction": strong,
        raw: { contrast: 2, colour: 0, softStars: 50 },
        // Not chosen for this target: never said.
        hoo: strong,
      },
    },
  }).options[0].cards[0];
  assert.deepEqual(card.finishes, [
    {
      pipeline: "Rendu naturel",
      finish: "Présentation · Couleurs naturelles · Étoiles 50 %",
    },
    {
      pipeline: "Correction bibande",
      finish: "Fort · Couleurs +20 % · Étoiles 50 %",
    },
  ]);
  // A default that moves: the pipeline that follows it is not « modifiée ».
  const moved = viewOf({
    ...base,
    finishDefault: strong,
    targetFinishes: { [NGC]: { "dualband-correction": strong } },
  }).options[0].cards[0];
  assert.deepEqual(moved.finishes, []);
});
