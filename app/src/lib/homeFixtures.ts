// The invented data of the tests and captures: « Camille », « Col des
// Étoiles », October 2026, NGC 7000, IC 1396 and M31. One source for the tests
// of the screens' words (`*.test.ts`) and for the scenarios of the captures,
// so a capture shows exactly the state a test checks. Synthetic only: no real place, path or person.

import type {
  Capabilities,
  FrameSummary,
  ImagePlan,
  IntegrationPreview,
  Missing,
  MissingCapture,
  NightDetail,
  NightDetection,
  NightOnline,
  NightPrecheck,
  NightResume,
  NightView,
  OnlineCapture,
  Pipeline,
  SessionSummary,
  TargetDetection,
  ToolRow,
  TreatmentState,
} from "./bridge";

export const ROOT = "/home/camille/Documents/Astro Sync";
export const NIGHT_FOLDER = `${ROOT}/2026-10-07 · Col des Étoiles · 7f3a2c`;
export const PLACE = "Col des Étoiles";

export const NGC = "t-ngc7000";
export const IC = "t-ic1396";
export const M31 = "t-m31";

const NAMES: Record<string, string> = {
  [NGC]: "NGC 7000",
  [IC]: "IC 1396",
  [M31]: "M31",
};

export function session(
  id: string,
  startedAt: string,
  finishedAt: string | null,
): SessionSummary {
  return {
    id,
    planId: null,
    status: finishedAt ? "finished" : "open",
    startedAt,
    finishedAt,
    createdAt: startedAt,
    updatedAt: finishedAt ?? startedAt,
    eventCount: 0,
    supplementCount: 0,
    pendingSupplementCount: 0,
  };
}

/** The five nights of the list (hours in Paris, CEST). */
export const SESSIONS: SessionSummary[] = [
  session("night-8", "2026-10-08T19:10:00Z", null),
  session("night-7", "2026-10-07T19:05:00Z", "2026-10-08T02:40:00Z"),
  session("night-5", "2026-10-05T18:50:00Z", "2026-10-06T01:15:00Z"),
  session("night-3", "2026-10-03T18:40:00Z", "2026-10-04T02:05:00Z"),
  session("night-2", "2026-10-02T18:45:00Z", "2026-10-02T23:30:00Z"),
];

export function detail(
  sessionId: string,
  targets: string[],
  debrief: { targetId: string; acquiredMinutes: number }[] = [],
): NightDetail {
  const summary = SESSIONS.find((item) => item.id === sessionId) ?? SESSIONS[1];
  return {
    session: summary,
    night: {
      title: { value: null },
      place: { value: { name: PLACE } },
      targetIds: { value: targets },
    },
    debrief: {
      targets: debrief.map((item) => ({
        targetId: item.targetId,
        state: "done",
        acquiredMinutes: item.acquiredMinutes,
      })),
    },
  };
}

export function image(
  name: string,
  sha256: string,
  byteSize: number,
  options: { source?: string; createdAt?: string; solved?: boolean } = {},
) {
  return {
    id: `image-${sha256.slice(0, 6)}`,
    main: true,
    sha256,
    byteSize,
    originalName: name,
    source: options.source ?? "astro-sync",
    createdAt: options.createdAt ?? "2026-10-08T08:00:00Z",
    skySolution: options.solved ? "present" : "missing",
  };
}

export function capture(
  targetId: string,
  main: ReturnType<typeof image>,
): OnlineCapture {
  return {
    id: `capture-${targetId}`,
    targetIds: [targetId],
    source: main.source,
    stage: "final",
    visibility: "private",
    uploaded: true,
    createdAt: main.createdAt,
    mainImage: main,
    imageCount: 1,
    images: [main],
  };
}

export function online(
  sessionId: string,
  targets: string[],
  captures: OnlineCapture[] = [],
  analysis: { sentAt: string } | null = null,
): NightOnline {
  return {
    sessionId,
    targets: targets.map((targetId) => ({
      targetId,
      name: NAMES[targetId],
      planned: true,
      captures: captures.filter((item) => item.targetIds.includes(targetId)),
    })),
    analysis: {
      present: analysis !== null,
      sentAt: analysis?.sentAt ?? null,
      originDeviceId: null,
      deviceName: null,
      reportVersion: analysis ? 2 : null,
      producer: null,
      stepCount: 0,
      editedStepCount: 0,
    },
  };
}

function frames(
  filters: [string | null, number][],
  exposures: number[],
  bytes: number,
): FrameSummary {
  return {
    filters: filters.map(([filter, count]) => ({ filter, frames: count })),
    exposures,
    gains: [100],
    bytes,
  };
}

export function target(
  targetId: string,
  lights: number,
  summary: FrameSummary,
  final: { name: string; bytes: number } | null,
  minimum: TargetDetection["minimum"] = final ? "final-file" : "missing",
): TargetDetection {
  return {
    targetId,
    designation: NAMES[targetId],
    folderName: NAMES[targetId],
    inNight: true,
    lightFrames: lights,
    filters: summary.filters
      .map((f) => f.filter)
      .filter((f): f is string => !!f),
    finalImage: final ? `${NAMES[targetId]}/final/${final.name}` : null,
    otherFinalImages: [],
    minimum,
    finalBytes: final?.bytes ?? null,
    lightSummary: summary,
  };
}

export const NGC_LIGHTS = frames(
  [
    ["Ha", 20],
    ["OIII", 20],
  ],
  [180],
  1_300_000_000,
);
export const IC_LIGHTS = frames(
  [
    ["Ha", 19],
    ["OIII", 19],
  ],
  [180],
  1_200_000_000,
);
export const M31_LIGHTS = frames([[null, 42]], [120], 1_300_000_000);

/** Everything found. */
export function detectionAllFound(): NightDetection {
  return {
    folder: NIGHT_FOLDER,
    fingerprint: "f-7",
    targets: [
      target(NGC, 40, NGC_LIGHTS, {
        name: "NGC7000_final.tif",
        bytes: 412_000_000,
      }),
      target(IC, 38, IC_LIGHTS, {
        name: "IC1396_final.png",
        bytes: 96_000_000,
      }),
      target(M31, 42, M31_LIGHTS, { name: "M31_final.jpg", bytes: 14_000_000 }),
    ],
    logs: [
      {
        relativePath: "logs/NINA_2026-10-07.log",
        kind: "nina",
        bytes: 1_400_000,
      },
      {
        relativePath: "logs/PHD2_GuideLog_2026-10-07.txt",
        kind: "phd2",
        bytes: 4_000_000,
      },
    ],
    calibration: { darks: 30, flats: 50, darkFlats: 0, biases: 50 },
    calibrationSummary: {
      darks: frames([[null, 30]], [180, 120], 4_000_000_000),
      flats: frames(
        [
          ["Ha", 25],
          ["OIII", 25],
        ],
        [],
        400_000_000,
      ),
    },
    findings: [],
    outsideStructure: [],
    links: [],
  };
}

export const SHA = {
  ngc: "3f9a0000000000000000000000000000000000000000000000000000000c41e",
  ic: "5d20000000000000000000000000000000000000000000000000000000b9c3",
  m31Online: "aa11000000000000000000000000000000000000000000000000000000bb22",
  m31V2: "8c1f000000000000000000000000000000000000000000000000000000e2a7",
};

export function plan(
  targetId: string,
  name: string,
  sha256: string,
  size: number,
  decision: ImagePlan["decision"],
): ImagePlan {
  return {
    image: {
      targetId,
      path: `${NIGHT_FOLDER}/${NAMES[targetId]}/final/${name}`,
      sha256,
      size,
      kind: "final",
      render: null,
    },
    decision,
  };
}

/** NGC 7000 new, IC 1396 identical online, M31
 * a different image (a variant of the capture). */
export function plansMixed(variantsAllowed = true): ImagePlan[] {
  return [
    plan(NGC, "NGC7000_final.tif", SHA.ngc, 412_000_000, {
      decision: "new_capture",
    }),
    plan(IC, "IC1396_final.png", SHA.ic, 96_000_000, {
      decision: "already_online",
      captureId: `capture-${IC}`,
      imageId: "image-ic",
    }),
    plan(
      M31,
      "M31_final_v2.jpg",
      SHA.m31V2,
      15_000_000,
      variantsAllowed
        ? { decision: "variant", captureId: `capture-${M31}` }
        : { decision: "not_sent", reason: "variants_not_allowed" },
    ),
  ];
}

export function onlineMixed(): NightOnline {
  return online(
    "night-7",
    [NGC, IC, M31],
    [
      capture(IC, image("IC1396_final.png", SHA.ic, 96_000_000)),
      capture(
        M31,
        image("M31_final.jpg", SHA.m31Online, 14_000_000, {
          source: "phone",
          createdAt: "2026-10-08T07:30:00Z",
          solved: true,
        }),
      ),
    ],
  );
}

/** State 7's folder: M31 holds M31_final_v2.jpg. */
export function detectionMixed(): NightDetection {
  const detection = detectionAllFound();
  detection.targets[2] = target(M31, 42, M31_LIGHTS, {
    name: "M31_final_v2.jpg",
    bytes: 15_000_000,
  });
  return detection;
}

const ALL: TreatmentState["treatment"][] = [
  "stacking",
  "final_image",
  "sky_annotations",
  "logs",
  "progression",
];

/** The treatments a night offers: `reserved` without the capabilities,
 * `missing` the tools absent. */
export function treatments(
  reserved = false,
  missing: { stacking?: boolean; astap?: boolean } = {},
): TreatmentState[] {
  return ALL.map((treatment) => {
    // The stacking is local and never gated: the logs and the progression
    // only are reserved.
    if (reserved && ["logs", "progression"].includes(treatment)) {
      return {
        treatment,
        available: false,
        unavailable: {
          why: "not_in_subscription",
          capability:
            treatment === "logs" ? "analysis.logs" : "analysis.progression",
        },
      };
    }
    if (
      (treatment === "stacking" && missing.stacking) ||
      (treatment === "sky_annotations" && missing.astap)
    ) {
      return {
        treatment,
        available: false,
        unavailable: {
          why: "tool_missing",
          tool: treatment === "stacking" ? "stacking-pipeline" : "astap",
        },
      };
    }
    return { treatment, available: true, unavailable: null };
  });
}

export function tools(missing: { astap?: boolean } = {}): ToolRow[] {
  const row = (
    id: string,
    name: string,
    version: string | null,
    present: boolean,
  ): ToolRow => ({
    id,
    name: { fr: name, en: name },
    kind: "tool",
    required: true,
    state: present ? "present" : "missing",
    version,
    minVersion: null,
    path: present ? `/Applications/${name}` : null,
    page: null,
    licence: null,
    progressionCompatible: id === "siril" ? true : null,
  });
  return [
    row("siril", "Siril", "1.4.0", true),
    row("astap", "ASTAP", "2026.09", !missing.astap),
  ];
}

export function resume(
  status: NightResume["status"],
  step: NightResume["step"],
  missing: Missing[] = [],
  analysis: NightResume["analysis"] = { kind: "send" },
): NightResume {
  return { status, step, missing, analysis, skySolutions: [] };
}

export function view(
  detection: NightDetection | null,
  onlineState: NightOnline,
  resumeState: NightResume,
  treatmentStates: TreatmentState[] = treatments(),
): NightView {
  return {
    location: detection
      ? { state: "found", folder: detection.folder }
      : { state: "missing" },
    folderTrace: false,
    detection,
    online: onlineState,
    resume: resumeState,
    treatments: treatmentStates,
  };
}

export const CAPABILITIES_PLUS: Capabilities = {
  "planner.multiple": true,
  "planner.mosaic": true,
  "pointing.solve": true,
  "sync.plan_limit": 0,
  "gallery.private": true,
  "portal.invitations": true,
  // The variants are not counted for a subscription: the sentinel.
  "capture.variant_limit": 9_007_199_254_740_991,
  "capture.count_limit": 0,
  "capture.public": true,
  "analysis.logs": true,
  "analysis.progression": true,
  "storage.max_file_bytes": 1_000_000_000,
  "storage.quota_bytes": 250_000_000_000,
  "history.retention_days": 0,
};

export const CAPABILITIES_ADVANCED: Capabilities = {
  ...CAPABILITIES_PLUS,
  "analysis.logs": false,
  "analysis.progression": false,
  "storage.max_file_bytes": 100_000_000,
  "storage.quota_bytes": 50_000_000_000,
};

export function precheck(
  plans: ImagePlan[],
  options: {
    capabilities?: Capabilities;
    remaining?: number;
    allowed?: boolean;
    findings?: NightPrecheck["quota"]["findings"];
    toSend?: number;
  } = {},
): NightPrecheck {
  const caps = options.capabilities ?? CAPABILITIES_PLUS;
  const quota = caps["storage.quota_bytes"];
  const remaining = options.remaining ?? 231_400_000_000;
  const toSend =
    options.toSend ??
    plans
      .filter((p) =>
        ["new_capture", "variant", "variant_of_new"].includes(
          p.decision.decision,
        ),
      )
      .reduce((sum, p) => sum + p.image.size, 0);
  return {
    storage: {
      usedBytes: quota - remaining,
      quotaBytes: quota,
      remainingBytes: remaining,
      maxFileBytes: caps["storage.max_file_bytes"],
      captureCount: 2,
      captureCountLimit: 0,
      variantLimit: caps["capture.variant_limit"],
      capabilities: caps,
    },
    plans,
    quota: {
      allowed: options.allowed ?? true,
      toSendBytes: toSend,
      toSendFiles: plans.length,
      newCaptures: 1,
      findings: options.findings ?? [],
    },
    severalRendersAllowed: caps["capture.variant_limit"] > 0,
  };
}

/** Four captures without a sky
 * solution -- two from the phone, one from the web, one from a night whose
 * folder is on this computer. */
export const MISSING_CAPTURES: MissingCapture[] = [
  {
    id: "capture-m31-5",
    sessionId: "night-5",
    originalName: "M31_final.jpg",
    sha256: "c1a0000000000000000000000000000000000000000000000000000000000001",
    byteSize: 14_000_000,
    mainImageId: "image-m31-5",
    mainImageSource: "phone",
    nightDate: "2026-10-05",
    galleryLabel: null,
    thumbnail: true,
    targets: [
      {
        id: M31,
        designation: "M31",
        name: "Galaxie d’Andromède",
        raHours: 0.712,
        decDegrees: 41.27,
      },
    ],
  },
  {
    id: "capture-ngc-3",
    sessionId: "night-3",
    originalName: "NGC7000_v2.png",
    sha256: "c1a0000000000000000000000000000000000000000000000000000000000002",
    byteSize: 22_000_000,
    mainImageId: "image-ngc-3",
    mainImageSource: "web",
    nightDate: "2026-10-03",
    galleryLabel: null,
    thumbnail: true,
    targets: [
      {
        id: NGC,
        designation: "NGC 7000",
        name: "Nébuleuse de l’Amérique du Nord",
        raHours: 20.98,
        decDegrees: 44.33,
      },
    ],
  },
  {
    id: "capture-ic-7",
    sessionId: "night-7",
    originalName: "IC1396_final.png",
    sha256: SHA.ic,
    byteSize: 96_000_000,
    mainImageId: "image-ic",
    mainImageSource: "astro-sync",
    nightDate: "2026-10-07",
    galleryLabel: null,
    thumbnail: true,
    targets: [
      {
        id: IC,
        designation: "IC 1396",
        name: "Nébuleuse de la Trompe d’Éléphant",
        raHours: 21.65,
        decDegrees: 57.5,
      },
    ],
  },
  {
    id: "capture-m31-2",
    sessionId: "night-2",
    originalName: "M31_LRGB.jpg",
    sha256: "c1a0000000000000000000000000000000000000000000000000000000000004",
    byteSize: 18_000_000,
    mainImageId: "image-m31-2",
    mainImageSource: "phone",
    nightDate: "2026-10-02",
    galleryLabel: null,
    thumbnail: true,
    targets: [
      {
        id: M31,
        designation: "M31",
        name: "Galaxie d’Andromède",
        raHours: 0.712,
        decDegrees: 41.27,
      },
    ],
  },
];

/** Réglages › « Dépendances »: the rows the
 * core reads from the registry, ASTAP with its database, Tycho-2 and the
 * stacking pipeline. `astap: "missing"` makes ASTAP absent. */
export function dependencyTools(
  options: {
    astap?: "present" | "missing";
    partial?: number;
    /** This system has no file in the registry for ASTAP. */
    noDownload?: boolean;
    /** The stacking's Python environment. */
    environment?: "present" | "missing" | "interrupted" | "noPython";
  } = {},
): ToolRow[] {
  const astap = options.astap ?? "present";
  const environment = options.environment ?? "present";
  const envPresent = environment === "present";
  const envRow = {
    state: envPresent ? ("present" as const) : ("missing" as const),
    version: envPresent ? "3.13.7" : null,
    path: envPresent
      ? "~/Library/Application Support/Astro Sync/stacking-env"
      : null,
    page: "https://www.python.org/downloads/",
    interrupted: environment === "interrupted",
    installable: environment === "missing" || environment === "interrupted",
    blockedBy: envPresent
      ? []
      : environment === "noPython"
        ? ["python"]
        : ["python-env"],
  };
  const base = {
    minVersion: null,
    page: null,
    licence: null,
    download: null,
    partialBytes: null,
    progressionCompatible: null,
  };
  const name = (text: string) => ({ fr: text, en: text });
  return [
    {
      ...base,
      id: "siril",
      name: name("Siril"),
      kind: "tool",
      required: true,
      state: "present",
      version: "1.4.0",
      minVersion: "1.4",
      path: "/Applications/Siril.app",
      page: "https://siril.org/download/",
      progressionCompatible: true,
    },
    {
      ...base,
      id: "starnet2",
      name: name("StarNet2"),
      kind: "tool",
      required: false,
      state: "missing",
      version: null,
      path: null,
    },
    {
      ...base,
      ...envRow,
      id: "python-env",
      name: name("Environnement Python"),
      kind: "python-packages",
      required: false,
      minVersion: ">=3.12,<3.15",
    },
    {
      ...base,
      id: "astap",
      name: name("ASTAP"),
      kind: "tool",
      required: true,
      state: astap,
      version: astap === "present" ? "2026.09" : null,
      path: astap === "present" ? "/Applications/ASTAP.app" : null,
      page: "https://www.hnsky.org/astap.htm",
      download:
        astap === "present" || options.noDownload
          ? null
          : {
              url: "https://exemple.test/astap.zip",
              sha256: "00",
              bytes: 10_000_000,
            },
      partialBytes: options.partial ?? null,
    },
    {
      ...base,
      id: "astap-database",
      name: name("Base d’étoiles d’ASTAP"),
      kind: "data",
      required: true,
      state: astap,
      version: null,
      path: astap === "present" ? "/Applications/ASTAP.app/d50" : null,
      download:
        astap === "present" || options.noDownload
          ? null
          : {
              url: "https://exemple.test/d20_star_database.zip",
              sha256: "00",
              bytes: 400_000_000,
            },
    },
    {
      ...base,
      id: "astrometry-net",
      name: name("astrometry.net"),
      kind: "tool",
      required: false,
      state: "missing",
      version: null,
      path: null,
      page: "https://astrometry.net/use.html",
    },
    {
      ...base,
      id: "tycho-2",
      name: { fr: "Liste d’étoiles Tycho-2", en: "Tycho-2 star list" },
      kind: "data",
      required: false,
      state: "missing",
      version: null,
      path: null,
      page: "https://cdsarc.cds.unistra.fr/viz-bin/cat/I/259",
    },
    {
      ...base,
      ...envRow,
      id: "stacking-pipeline",
      name: { fr: "Pipeline d’empilement", en: "Stacking pipeline" },
      kind: "pipeline",
      required: false,
      licence: "MIT",
    },
  ];
}

/** The four shipped pipelines, as `list-pipelines` gives them. */
export const PIPELINES: Pipeline[] = [
  ["raw", "Rendu naturel", "none", ["broadband"], []],
  [
    "dualband-correction",
    "Correction bibande",
    "dual-band",
    ["dualband"],
    ["broadband"],
  ],
  ["hoo", "Palette HOO", "dual-band", ["dualband"], ["broadband"]],
  ["polished", "Rendu soigné", "none", ["broadband"], []],
].map(([id, name, filter, recommendedFor, discouragedFor]) => ({
  id: id as string,
  name: { fr: name as string },
  description: {},
  displayName: name as string,
  displayDescription: null,
  scriptSha256: null,
  filter: filter as string,
  outputs: ["tif", "png"],
  recommendedFor: recommendedFor as Pipeline["recommendedFor"],
  discouragedFor: discouragedFor as Pipeline["discouragedFor"],
  dependencies: [
    { id: "siril", required: true, bytes: 75_765_778 },
    { id: "starnet2", required: false, bytes: 127_977_753 },
    ...(id === "polished"
      ? [{ id: "graxpert", required: false, bytes: 130_103_079 }]
      : []),
    { id: "python-env", required: true, bytes: 81_841_919 },
  ],
}));

/** « Intégration de la nuit » of the three targets: NGC 7000 behind
 * a dual-band filter, IC 1396 and M31 broadband. */
export function previews(): Record<string, IntegrationPreview> {
  const family = (
    label: string,
    filter: string,
    band: "broadband" | "dualband",
    frames: number,
    exposure: number,
  ) => ({
    label,
    filter,
    band,
    exposureSeconds: exposure,
    gain: 100,
    binning: "1",
    temperatureC: -10,
    frameCount: frames,
    totalSeconds: frames * exposure,
    master: null,
  });
  return {
    [NGC]: {
      targetFolder: "NGC 7000",
      outside: 0,
      families: [
        family("L-eXtreme_180s_g100", "L-eXtreme", "dualband", 40, 180),
      ],
    },
    [IC]: {
      targetFolder: "IC 1396",
      outside: 1,
      families: [
        family("UV_IR-cut_120s_g100", "UV/IR-cut", "broadband", 38, 120),
      ],
    },
    [M31]: {
      targetFolder: "M31",
      outside: 0,
      families: [
        family("UV_IR-cut_120s_g100", "UV/IR-cut", "broadband", 42, 120),
      ],
    },
  };
}
