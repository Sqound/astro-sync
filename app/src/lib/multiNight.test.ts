import assert from "node:assert/strict";
import test from "node:test";

import { createTranslator } from "../i18n/locale.ts";
import type {
  IntegrationMaster,
  MultiNightGate,
  MultiNightView,
  NightProject,
  Pipeline,
  PlanLine,
} from "./bridge";
import {
  DEFAULT_PLAN_IMAGE,
  chosenKeys,
  chosenSources,
  combinedLines,
  familyText,
  multiNightSummary,
  nightTag,
  planImageWanted,
  planChoicesOf,
  planOptionView,
  planPipelines,
  referenceLine,
  sentMultiNightLines,
  sourceOf,
  targetIntegrationView,
} from "./multiNight.ts";
import { savePlanKept } from "./planKept.ts";
import type { Seg } from "./wizardFiles.ts";

const fr = createTranslator("fr");
const en = createTranslator("en");
const plain = (segs: Seg[]) =>
  segs
    .map((seg) =>
      typeof seg === "string" ? seg : "strong" in seg ? seg.strong : seg.code,
    )
    .join("");

const OPEN: MultiNightGate = { allowed: true, closed: null };
const RESERVED: MultiNightGate = {
  allowed: false,
  closed: "not_in_subscription",
};
// The service never heard (a private server never closes the
// multi-night plans; only an unanswered service does, without an offer).
const UNKNOWN: MultiNightGate = { allowed: false, closed: "unknown" };

const PIPELINES = [
  { id: "raw", displayName: "Rendu naturel", name: { fr: "Rendu naturel" } },
  {
    id: "dualband-correction",
    displayName: "Correction bibande",
    name: { fr: "Correction bibande" },
  },
] as unknown as Pipeline[];

/** The Heart: the 9th dual-band, the 10th RGB, the 12th
 * UV/IR-cut (the night being sent). */
const HEART: NightProject = {
  projectId: "p-heart",
  targetId: "ic-1805",
  name: "IC 1805",
  commonName: "Nébuleuse du Cœur",
  integrations: [],
  totalSeconds: 40_080,
  nightCount: 3,
  planCapture: null,
};

function line(
  night: string,
  date: string,
  filter: string,
  band: PlanLine["band"],
  frames: number,
  exposure: number,
  place: PlanLine["place"],
  computer: string | null = null,
): PlanLine {
  return {
    key: `${night}|${filter}`,
    nightId: night,
    eveningDate: date,
    filter,
    band,
    frameCount: frames,
    exposureSeconds: exposure,
    totalSeconds: frames * exposure,
    place,
    computerLabel: computer,
    master: place === "otherComputer" ? null : `/w/${night}/master.fit`,
    bytes: place === "otherComputer" ? 0 : 312_000_000,
    source: "master",
  };
}

const LINES = [
  line("n-09", "2026-08-09", "L-eXtreme", "dualband", 98, 180, "thisComputer"),
  line("n-10", "2026-08-10", "RGB", "broadband", 89, 90, "thisComputer"),
  line("n-12", "2026-08-12", "UV/IR-cut", "broadband", 120, 120, "thisNight"),
];

test("the night's integration is said family by family, with the frames left out and the master kept", () => {
  assert.equal(
    familyText(fr, "fr", {
      filter: "UV/IR-cut",
      exposureSeconds: 120,
      gain: 100,
      binning: "1x1",
      temperatureC: -10,
      frameCount: 120,
    }),
    "UV/IR-cut · 120 × 120 s · gain 100 · bin 1 · −10 °C",
  );
  const view = targetIntegrationView(fr, "fr", {
    targetId: "ic-1805",
    targetFolder: "IC 1805",
    eveningDate: "2026-08-12",
    preview: {
      targetFolder: "IC 1805",
      outside: 1,
      families: [
        {
          label: "UV_IR-cut_120s_g100",
          filter: "UV/IR-cut",
          exposureSeconds: 120,
          gain: 100,
          binning: "1",
          temperatureC: -10,
          frameCount: 120,
          totalSeconds: 14_400,
          master: { bytes: 312_000_000 } as IntegrationMaster,
        },
      ],
    },
  });
  assert.equal(view.families[0].duration, "4 h 00");
  assert.equal(
    view.families[0].master ?? "",
    "Master gardé : IC 1805/stacked/UV_IR-cut_120s_g100_integration.fit · 312 Mo, le seul intermédiaire conservé.",
  );
  assert.equal(
    view.outside ?? "",
    "1 pose hors de la nuit du 12 août, écartée de l’intégration (elle reste comptée dans le dossier).",
  );
});

test("with the right, every integration here is ticked, the reference is the longest broadband one when the plan mixes bands, a dual-band one proposes its render", () => {
  const choices = {
    ...DEFAULT_PLAN_IMAGE,
    unticked: new Set(["n-12|UV/IR-cut"]),
  };
  const view = planOptionView(fr, "fr", {
    project: HEART,
    lines: LINES,
    gate: OPEN,
    choices,
    pipelines: PIPELINES,
    targetRenders: {},
  });
  assert.equal(view.state, "available");
  assert.equal(
    view.line,
    "Plan multi-nuits IC 1805 — Nébuleuse du Cœur · 3 nuits. Combine les intégrations cochées, filtre par filtre, alignées sur la nuit de référence et pondérées par leur durée.",
  );
  assert.deepEqual(
    view.groups.map((group) => group.label),
    ["Bibande (Hα + OIII)", "RGB", "UV/IR-cut"],
  );
  // The plan mixes a dual-band night (4 h 54) and a broadband one (2 h 14): the reference is the broadband
  // night, whose frame is the plan's and has colour everywhere (`plan_image::reference_of`).
  assert.equal(
    plain(view.groups[0].lines[0].segs),
    "Dim. 9 août · 98 × 180 s sur cet ordinateur",
  );
  assert.match(plain(view.groups[1].lines[0].segs), /nuit de référence/);
  assert.equal(view.groups[0].lines[0].duration, "4 h 54");
  assert.equal(
    plain(view.groups[2].lines[0].segs),
    "Mer. 12 août · 120 × 120 s cette nuit",
  );
  assert.equal(view.groups[2].lines[0].ticked, false);
  assert.equal(
    view.combined ?? "",
    "Combinées : Bibande 4 h 54 · RGB 2 h 14 · 7 h 08 sur 2 nuits",
  );
  assert.equal(
    view.notCombined ?? "",
    "Non combinée : UV/IR-cut 4 h 00 (décochée).",
  );
  // The image takes the pipelines of the plan's target; none was
  // chosen, so its defaults, « Correction bibande » with the dual-band night.
  assert.equal(
    view.pipelinesLine ?? "",
    "L’image est faite avec les pipelines choisis pour IC 1805 : Rendu naturel et Correction bibande.",
  );
  const chosenOnly = planOptionView(fr, "fr", {
    project: HEART,
    lines: LINES,
    gate: OPEN,
    choices,
    pipelines: PIPELINES,
    targetRenders: { "ic-1805": ["dualband-correction"] },
  });
  assert.equal(
    chosenOnly.pipelinesLine ?? "",
    "L’image est faite avec les pipelines choisis pour IC 1805 : Correction bibande.",
  );
  assert.deepEqual(chosenKeys(LINES, choices), ["n-09|L-eXtreme", "n-10|RGB"]);
  assert.equal(planImageWanted(OPEN, LINES, choices), true);
  // Without the dual-band night: the natural render alone.
  const broadband = {
    ...choices,
    unticked: new Set(["n-09|L-eXtreme"]),
  };
  assert.deepEqual(planPipelines(LINES, broadband, {}, "ic-1805", PIPELINES), [
    "raw",
  ]);
  assert.deepEqual(planPipelines(LINES, choices, {}, "ic-1805", PIPELINES), [
    "raw",
    "dualband-correction",
  ]);
  // The choice of the target wins, « Sans traitement » last.
  assert.deepEqual(
    planPipelines(
      LINES,
      choices,
      { "ic-1805": ["linear", "raw"] },
      "ic-1805",
      PIPELINES,
    ),
    ["raw", "linear"],
  );
});

test("an integration on another computer is listed, never combined; one night left is the image of that night", () => {
  const lines = [
    LINES[0],
    line(
      "n-10",
      "2026-08-10",
      "RGB",
      "broadband",
      89,
      90,
      "otherComputer",
      "Portable",
    ),
    LINES[2],
  ];
  const choices = {
    ...DEFAULT_PLAN_IMAGE,
    unticked: new Set(["n-12|UV/IR-cut"]),
  };
  const view = planOptionView(fr, "fr", {
    project: HEART,
    lines,
    gate: OPEN,
    choices,
    pipelines: PIPELINES,
    targetRenders: {},
  });
  const laptop = view.groups[1].lines[0];
  assert.equal(
    plain(laptop.segs),
    "Lun. 10 août · 89 × 90 s sur un autre ordinateur (Portable), non combinée",
  );
  assert.deepEqual([laptop.ticked, laptop.disabled], [false, true]);
  assert.equal(view.combined, null);
  assert.equal(
    view.oneNight ?? "",
    "Combinée : Bibande 4 h 54 · 1 nuit. Une seule nuit combinée : l’image du plan est celle de la nuit du 9 août ; aucune capture du plan n’est créée.",
  );
  assert.equal(
    view.otherComputer?.line ?? "",
    "L’intégration du 10 août est sur un autre ordinateur.",
  );
  assert.match(view.otherComputer?.details ?? "", /dossier de travail/);
  assert.equal(planImageWanted(OPEN, lines, choices), false);
});

test("without the right everything is greyed and said, with « Voir les offres »; a private server names no offer", () => {
  const view = planOptionView(fr, "fr", {
    project: HEART,
    lines: LINES,
    gate: RESERVED,
    choices: DEFAULT_PLAN_IMAGE,
    pipelines: PIPELINES,
    targetRenders: {},
  });
  assert.equal(view.state, "reserved");
  assert.equal(view.ticked, false);
  assert.ok(
    view.groups.every((g) => g.lines.every((l) => l.disabled && !l.ticked)),
  );
  assert.equal(
    view.known ?? "",
    "Connues : Bibande 4 h 54 · RGB 2 h 14 · UV/IR-cut 4 h 00 · 3 nuits · 11 h 08",
  );
  assert.equal(
    view.closedLine ?? "",
    "Le plan multi-nuits dans Astro Sync est réservé à Synchronisation avancée+ : vos intégrations restent sur cet ordinateur, rien n’est combiné ni envoyé.",
  );
  assert.equal(view.offers, true);
  assert.equal(view.pipelinesLine, null);
  // Nothing in the plan's view says « nuit de référence » without the right.
  assert.ok(!plain(view.groups[0].lines[0].segs).includes("référence"));
  const unknown = planOptionView(fr, "fr", {
    project: HEART,
    lines: LINES,
    gate: UNKNOWN,
    choices: DEFAULT_PLAN_IMAGE,
    pipelines: PIPELINES,
    targetRenders: {},
  });
  assert.equal(unknown.state, "unknown");
  assert.equal(unknown.offers, false);
  assert.match(
    unknown.closedLine ?? "",
    /^Le service n’a pas encore dit s’il propose les plans multi-nuits/,
  );
  assert.ok(!(unknown.closedLine ?? "").includes("serveur privé"));
  assert.equal(planImageWanted(UNKNOWN, LINES, DEFAULT_PLAN_IMAGE), false);
});

const VIEW = (gate: MultiNightGate): MultiNightView => ({
  eveningDate: "2026-08-12",
  projects: [{ project: HEART, lines: LINES }],
  gate,
  computer: { label: "iMac du bureau", installationId: "inst-1" },
});

test("the summary says the lines of the plan, its image and the masters kept, and the right in the check", () => {
  const choices = {
    "p-heart": { ...DEFAULT_PLAN_IMAGE, unticked: new Set(["n-12|UV/IR-cut"]) },
  };
  const open = multiNightSummary(fr, "fr", {
    view: VIEW(OPEN),
    masters: [],
    integrating: true,
    planImage: choices,
    pipelines: PIPELINES,
    targetRenders: {},
  });
  assert.deepEqual(
    open.sent.map((row) => [plain(row.segs), row.value]),
    [
      [
        "Intégration de la nuit : IC 1805 · UV/IR-cut · 120 × 120 s · 4 h 00",
        "calculé ici",
      ],
      [
        "Intégrations du plan multi-nuits IC 1805 : 3 lignes (9 août, 10 août, 12 août) · des lignes, aucun fichier",
        "calculées ici",
      ],
      [
        "Image du plan multi-nuits IC 1805 · Rendu naturel et Correction bibande · Bibande 4 h 54 · RGB 2 h 14 · nouvelle capture du plan",
        "calculé ici",
      ],
    ],
  );
  assert.deepEqual(
    open.never.map((row) => [plain(row.segs), row.value]),
    [
      [
        "Masters des intégrations : 3 (9 août, 10 août, 12 août), gardés dans stacked/",
        "936 Mo",
      ],
    ],
  );
  assert.equal(open.check?.ok, true);
  const closed = multiNightSummary(fr, "fr", {
    view: VIEW(RESERVED),
    masters: [],
    integrating: true,
    planImage: choices,
    pipelines: PIPELINES,
    targetRenders: {},
  });
  assert.deepEqual(
    closed.sent.map((row) => plain(row.segs)),
    [
      "Intégrations du plan multi-nuits IC 1805 : non envoyées (Synchronisation avancée+)",
      "Image du plan multi-nuits IC 1805 : non fabriquée",
    ],
  );
  assert.deepEqual([closed.check?.ok, closed.check?.offers], [false, true]);
  // The masters stay on this computer either way, said with their size.
  assert.equal(closed.never.length, 1);
});

test("the tag of « Nuits » says the plan, its nights and its known time", () => {
  const tags = [
    {
      projectId: "p-heart",
      name: "IC 1805",
      nightCount: 3,
      totalSeconds: 40_080,
    },
  ];
  assert.equal(
    nightTag(fr, "fr", tags) ?? "",
    "Plan multi-nuits IC 1805 · 3 nuits · 11 h 08",
  );
  assert.equal(
    nightTag(en, "en", tags) ?? "",
    "Multi-night plan IC 1805 · 3 nights · 11 h 08",
  );
  assert.equal(nightTag(fr, "fr", undefined), null);
});

test("a refusal of the service is said in the person's words, the lines sent are counted", () => {
  const said = sentMultiNightLines(fr, "fr", {
    projects: [HEART],
    integrations: [
      {
        result: "refused",
        projectId: "p-heart",
        code: "multi_night_not_active",
        message: "The multi-night plans of Astro Sync are part of …",
      },
    ],
    planImage: {},
  });
  assert.deepEqual(said.ok, []);
  assert.deepEqual(said.notes, [
    "Plan multi-nuits IC 1805, refusé par le service : le plan multi-nuits dans Astro Sync est réservé à Synchronisation avancée+.",
  ]);
  const sent = sentMultiNightLines(fr, "fr", {
    projects: [HEART],
    integrations: [
      { result: "sent", projectId: "p-heart", lines: 1, project: HEART },
    ],
    planImage: {
      "p-heart": {
        kind: "made",
        outcomes: [
          { combinedNightIds: ["n-09", "n-10"] } as never,
          { combinedNightIds: ["n-09", "n-10"] } as never,
        ],
        enqueued: {} as never,
      },
    },
  });
  assert.deepEqual(sent.ok, [
    "1 ligne d’intégration envoyée au plan multi-nuits IC 1805",
    "Image du plan multi-nuits IC 1805 fabriquée ici et envoyée (2 nuits combinées)",
  ]);
});

/* ----------- The image of a device in a plan, the frame ---------- */

/** A text as the screens write it: the narrow no-break spaces of the numbers as plain ones. */
const flat = (text: string | null) => (text ?? "").replace(/\u202f/g, " ");

const DEVICE = (poses: number, seconds: number) => ({
  path: "/w/device/Stacked.fit",
  device: "Seestar S50 Pro",
  frameCount: poses,
  totalSeconds: seconds,
  bytes: 50_000_000,
});

/** The nights of the plan: the 10th with its poses and its device's image, the 11th with the device's image only, the 13th stacked here. */
function deviceLines(): PlanLine[] {
  const tenth = line(
    "n-10",
    "2026-10-10",
    "IRCUT",
    "broadband",
    430,
    30,
    "thisComputer",
  );
  const eleventh: PlanLine = {
    ...line(
      "n-11",
      "2026-10-11",
      "IRCUT",
      "broadband",
      212,
      30,
      "thisComputer",
    ),
    master: null,
    source: "device",
  };
  const thirteenth = line(
    "n-13",
    "2026-10-13",
    "IRCUT",
    "broadband",
    188,
    30,
    "thisNight",
  );
  return [
    { ...tenth, key: "n-10|IRCUT", device: DEVICE(430, 12_900) },
    { ...eleventh, key: "n-11|IRCUT", device: DEVICE(212, 6_360) },
    { ...thirteenth, key: "n-13|IRCUT" },
  ];
}

const view8 = (choices = DEFAULT_PLAN_IMAGE) =>
  planOptionView(fr, "fr", {
    project: HEART,
    lines: deviceLines(),
    gate: OPEN,
    choices,
    pipelines: PIPELINES,
    targetRenders: {},
  });

test("a night with its poses and its device's image takes the poses by default, a night with the device's image only takes it, a night without one is stacked here", () => {
  const lines = deviceLines();
  assert.deepEqual(
    lines.map((item) => sourceOf(item, DEFAULT_PLAN_IMAGE)),
    ["master", "device", "master"],
  );
  const view = view8();
  const rows = view.groups.flatMap((group) => group.lines);
  assert.deepEqual(
    rows.map((row) => [row.source?.kind, row.source?.taken]),
    [
      ["choice", "master"],
      ["deviceOnly", "device"],
      ["here", "master"],
    ],
  );
  assert.equal(
    flat(rows[0].source?.note ?? ""),
    "Les poses sont ici : on repart d’elles, par défaut.",
  );
  assert.equal(
    flat(rows[1].source?.note ?? ""),
    "Ses poses ne sont pas sur cet ordinateur.",
  );
  assert.equal(
    flat(rows[2].source?.note ?? ""),
    "Aucune image d’appareil : les poses seulement.",
  );
  assert.equal(
    flat(plain(rows[1].segs)),
    "Dim. 11 oct. · 212 × 30 s image d’appareil sur cet ordinateur · ligne d’intégration lue dans son en-tête",
  );
});

test("the device's image of a night that has its poses is taken only when asked, and then it weighs what its header says", () => {
  const lines = deviceLines();
  const asked = {
    ...DEFAULT_PLAN_IMAGE,
    sources: { "n-10|IRCUT": "device" as const },
  };
  assert.equal(sourceOf(lines[0], asked), "device");
  const rows = view8(asked).groups.flatMap((group) => group.lines);
  assert.equal(rows[0].source?.taken, "device");
  assert.equal(
    flat(rows[0].source?.note ?? ""),
    "L’image de l’appareil, telle quelle : prête tout de suite, un peu moins fine.",
  );
  assert.deepEqual(chosenSources(lines, asked), { "n-10|IRCUT": "device" });
  // The totals are the device's own, from its header.
  const combined = combinedLines(lines, asked);
  assert.equal(combined[0].totalSeconds, 12_900);
  assert.equal(combined[0].frameCount, 430);
  assert.equal(combined[0].source, "device");
  // A source that is not here is not taken, and not sent.
  const impossible = {
    ...DEFAULT_PLAN_IMAGE,
    sources: {
      "n-13|IRCUT": "device" as const,
      "n-11|IRCUT": "master" as const,
    },
  };
  assert.equal(sourceOf(lines[2], impossible), "master");
  assert.equal(sourceOf(lines[1], impossible), "device");
  assert.deepEqual(chosenSources(lines, impossible), {});
});

test("the nights that enter with their device's image are named with the cost, and why not the device's image everywhere", () => {
  const view = view8();
  assert.equal(
    flat(view.deviceWarning),
    "La nuit du 11 octobre entre avec l’image de l’appareil, telle quelle (16 bits, étoiles environ 12 % plus larges) : l’image du plan sera un peu moins fine que si ses poses étaient ici.",
  );
  assert.equal(
    flat(view.deviceWhy),
    "Pourquoi pas l’image de l’appareil partout ? L’appareil empile chaque nuit à part : son image ne combine jamais deux nuits. Pour le plan, Astro Sync aligne et combine les nuits lui-même ; ses propres intégrations gardent la précision des poses.",
  );
  // The 10th (poses), the 11th (device) and the 13th: 3 nights, 12 900 + 6 360 + 5 640 s.
  assert.equal(
    flat(view.combined),
    "Combinées : IRCUT 6 h 55 · 6 h 55 sur 3 nuits",
  );
  // Two nights from the device: the plural.
  const both = view8({
    ...DEFAULT_PLAN_IMAGE,
    sources: { "n-10|IRCUT": "device" },
  });
  assert.match(
    flat(both.deviceWarning),
    /^Les nuits du 10 octobre et 11 octobre entrent avec l’image de l’appareil/,
  );
  // None: nothing is said.
  const poses = planOptionView(fr, "fr", {
    project: HEART,
    lines: LINES,
    gate: OPEN,
    choices: DEFAULT_PLAN_IMAGE,
    pipelines: PIPELINES,
    targetRenders: {},
  });
  assert.equal(poses.deviceWarning, null);
  assert.equal(poses.deviceWhy, null);
});

test("a plan without any device's image keeps the lines as they were: no source column", () => {
  const view = planOptionView(fr, "fr", {
    project: HEART,
    lines: LINES,
    gate: OPEN,
    choices: DEFAULT_PLAN_IMAGE,
    pipelines: PIPELINES,
    targetRenders: {},
  });
  for (const group of view.groups) {
    for (const item of group.lines) assert.equal(item.source, null);
  }
});

test("the reference is the longest night, the longest without a dual-band filter when the plan mixes them, the earliest on a tie (as the job picks it)", () => {
  const dual = line(
    "n-1",
    "2026-10-01",
    "L-eXtreme",
    "dualband",
    100,
    300,
    "thisComputer",
  );
  const broad = line(
    "n-2",
    "2026-10-02",
    "UV/IR-cut",
    "broadband",
    100,
    120,
    "thisComputer",
  );
  const broader = line(
    "n-3",
    "2026-10-03",
    "RGB",
    "broadband",
    100,
    90,
    "thisComputer",
  );
  assert.equal(referenceLine([dual, broad, broader])?.nightId, "n-2");
  const longer = {
    ...dual,
    nightId: "n-4",
    eveningDate: "2026-10-04",
    totalSeconds: 40_000,
  };
  assert.equal(referenceLine([dual, longer])?.nightId, "n-4");
  const early = { ...broad, nightId: "n-5", eveningDate: "2026-09-30" };
  assert.equal(referenceLine([broad, early])?.nightId, "n-5");
  assert.equal(referenceLine([]), null);
});

test("what was kept for a plan comes back on this computer when the window holds nothing for it", () => {
  const storage = new Map<string, string>();
  const window = {
    getItem: (key: string) => storage.get(key) ?? null,
    setItem: (key: string, value: string) => void storage.set(key, value),
  };
  savePlanKept(window, "p-heart", {
    frame: { top: 1, left: 2, height: 30, width: 40 },
    sources: { "n-10|IRCUT": "device" },
  });
  const found = planChoicesOf({}, "p-heart", window);
  assert.deepEqual(found.frame, { top: 1, left: 2, height: 30, width: 40 });
  assert.deepEqual(found.sources, { "n-10|IRCUT": "device" });
  assert.equal(found.off, false);
  assert.equal(found.unticked.size, 0);
  // What the window holds wins; another plan has nothing kept.
  const held = { ...DEFAULT_PLAN_IMAGE, off: true };
  assert.equal(planChoicesOf({ "p-heart": held }, "p-heart", window), held);
  assert.deepEqual(planChoicesOf({}, "p-other", window), DEFAULT_PLAN_IMAGE);
});

test("the summary says the size of the frame kept, after the capture, only when it is not the whole image", () => {
  const view: MultiNightView = {
    eveningDate: "2026-10-13",
    projects: [{ project: HEART, lines: deviceLines() }],
    gate: OPEN,
    computer: { label: null, installationId: null },
  };
  const input = {
    view,
    masters: [],
    integrating: false,
    planImage: { "p-heart": DEFAULT_PLAN_IMAGE },
    pipelines: PIPELINES,
    targetRenders: {},
  };
  const whole = multiNightSummary(fr, "fr", input);
  assert.ok(
    !whole.sent.some((row) => plain(row.segs).includes("image rognée")),
  );
  const framed = multiNightSummary(fr, "fr", {
    ...input,
    framed: { "p-heart": { width: 2154, height: 1994 } },
  });
  const row = framed.sent.find((item) =>
    plain(item.segs).startsWith("Image du plan multi-nuits"),
  );
  assert.ok(row);
  assert.match(
    flat(plain(row.segs)),
    /nouvelle capture du plan · image rognée 2 154 × 1 994$/,
  );
  assert.equal(typeof row.segs[row.segs.length - 1], "object");
});
