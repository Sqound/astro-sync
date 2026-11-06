// « Détecter les sessions »: the data of the review, the result and the
// tidying, shaped as the engine and the bridge of `src/lib/bridge.ts` give
// them. Shared by the capture scenarios and by
// `src/lib/detectFixtures.test.ts`.
//
// The reference night: IC 1805 at « Maison » (9 to 11 August, plan « Voile et
// Cœur – RVB »), the ASIAIR of the Askar 50P and the ZWO ASI533MC Pro; made
// up for the captures: the nights of 31 July, 3, 7 and 15 August, « Site noir »,
// the plans « Voile – OIII » and « M33 – RVB », the clock of the 3rd.
//
// Exports: DETECT_USER (UserData, account with the advanced planning),
// DETECT_USER_FREE (the same account without it), PROPOSAL,
// PROPOSAL_FREE, CREATED, ORGANISE_PLAN and
// ORGANISE_OUTCOME, and `proposalLines` (the lines of the
// review as the engine's `propose.rs::lines_of` makes them).

const UNKNOWN_PLACE = { latitude: 35.6582, longitude: 138.5683 };

/** Great-circle distance in km (the engine's `distance_km`). */
export function distanceKm(lat1, lon1, lat2, lon2) {
  const rad = Math.PI / 180;
  const a =
    Math.sin(((lat2 - lat1) * rad) / 2) ** 2 +
    Math.cos(lat1 * rad) *
      Math.cos(lat2 * rad) *
      Math.sin(((lon2 - lon1) * rad) / 2) ** 2;
  return 6371.0088 * 2 * Math.asin(Math.sqrt(a));
}

const MAISON = { latitude: 43.8606, longitude: 7.4312 };
const SITE_NOIR = { latitude: 43.946, longitude: 7.4312 };
// The poses of five nights are 1,4 km south of « Maison »; the 7 August is
// 1,4 km north of it and 8,1 km south of « Site noir ».
const HOME_POSES = { latitude: 43.848, longitude: 7.4312 };
const NEAR_POSES = { latitude: 43.8731, longitude: 7.4312 };

export const DETECT_USER = {
  places: [
    {
      id: "place-maison",
      name: "Maison",
      ...MAISON,
      timeZone: "Europe/Paris",
    },
    {
      id: "place-site-noir",
      name: "Site noir",
      ...SITE_NOIR,
      timeZone: "Europe/Paris",
    },
  ],
  defaultPlaceId: "place-maison",
  equipment: [
    {
      id: "eq-askar",
      name: "Askar 50P",
      kind: "telescope",
      manufacturer: "Askar",
      model: "50P",
      focalLengthMm: 250,
    },
    {
      id: "eq-asi533",
      name: "ZWO ASI533MC Pro",
      kind: "camera",
      manufacturer: "ZWO",
      model: "ASI533MC Pro",
      focalLengthMm: null,
    },
  ],
  sessions: [
    {
      id: "sess-0809",
      planId: "plan-voile-coeur",
      eveningDate: "2026-08-09",
      placeId: "place-maison",
      targets: [
        {
          targetId: "target-ic1805",
          designation: "IC 1805",
          start: "2026-08-09T11:10:00Z",
          end: "2026-08-09T19:05:00Z",
        },
      ],
    },
    {
      id: "sess-0811",
      planId: null,
      eveningDate: "2026-08-11",
      placeId: "place-maison",
      targets: [
        {
          targetId: "target-ic1805",
          designation: "IC 1805",
          start: "2026-08-11T11:15:00Z",
          end: "2026-08-11T18:00:00Z",
        },
      ],
    },
  ],
  plans: [
    {
      id: "plan-m33",
      name: "M33 – RVB",
      eveningDate: "2026-07-31",
      status: "pending",
      targetIds: ["target-m33"],
      designations: ["M33"],
      sessionId: null,
    },
    {
      id: "plan-voile-oiii",
      name: "Voile – OIII",
      eveningDate: "2026-08-07",
      status: "pending",
      targetIds: ["target-sh2-103"],
      designations: ["Sh2-103"],
      sessionId: null,
    },
    {
      id: "plan-voile-coeur",
      name: "Voile et Cœur – RVB",
      eveningDate: "2026-08-09",
      status: "executed",
      targetIds: ["target-ic1805"],
      designations: ["IC 1805"],
      sessionId: "sess-0809",
    },
  ],
  account: { advancedPlanning: true, severalPlansPerNight: true },
};

/** The same data for an account without the advanced planning nor a
 * subscription: one plan per night. */
export const DETECT_USER_FREE = {
  ...DETECT_USER,
  account: { advancedPlanning: false, severalPlansPerNight: false },
};

// ------------------------------------------------------------------ nights

const INSTRUMENT = "ZWO ASI533MC Pro";

const TARGETS = {
  m31: { id: "target-m31", designation: "M31", name: "Galaxie d’Andromède" },
  ngc7000: {
    id: "target-ngc7000",
    designation: "NGC 7000",
    name: "Amérique du Nord",
  },
  sh2103: { id: "target-sh2-103", designation: "Sh2-103", name: "Voile" },
  ic1805: { id: "target-ic1805", designation: "IC 1805", name: "Cœur" },
};

function target(spec, date, fields) {
  const { frames, exposure, start, end, existing = "new" } = fields;
  const total = frames * exposure;
  const stamp = date.replaceAll("-", "");
  return {
    key: `object:${spec.designation.toLowerCase().replace(/[^a-z0-9]/g, "")}`,
    targetId: spec.id,
    designation: spec.designation,
    name: spec.name,
    source: { by: "object" },
    objects: [spec.designation],
    start: `${date}T${start}:00Z`,
    end: `${date}T${end}:00Z`,
    frames,
    exposureSeconds: total,
    filters: [],
    acquisition: [
      {
        filter: null,
        exposureSeconds: exposure,
        gain: 100,
        frames,
        totalSeconds: total,
      },
    ],
    instrument: INSTRUMENT,
    solveFrame: null,
    files: [
      `Autorun/Light/${spec.designation}/Light_${spec.designation}_${exposure}.0s_gain100_${stamp}-0001.fit`,
      `Autorun/Light/${spec.designation}/Light_${spec.designation}_${exposure}.0s_gain100_${stamp}-0002.fit`,
    ],
    fileFilters: [],
    existing,
  };
}

/** The calibration sets: one per kind and night, `daysAway` 0 unless a night
 * borrows the set of another (the darks of the 9 August serve the 10th). */
const SETS = [];
function set(id, kind, date, fields) {
  SETS.push({
    id,
    kind,
    kindFromFolder: true,
    folder: `Calibration/${kind}s/${date}`,
    frames: fields.frames,
    exposureSeconds: fields.exposure ?? null,
    gain: 100,
    temperatureC: fields.temperature ?? null,
    filter: null,
    instrument: INSTRUMENT,
    start: `${date}T${fields.start}:00Z`,
    end: `${date}T${fields.end}:00Z`,
    files: [`Calibration/${kind}s/${date}/${kind}_0001.fit`],
  });
  return id;
}

const calibrationOf = (setId, kind, daysAway = 0) => ({
  setId,
  kind,
  daysAway,
  matchedOn:
    kind === "dark"
      ? ["exposure", "temperature", "gain", "date"]
      : kind === "flat"
        ? ["gain", "date"]
        : ["temperature", "gain", "date"],
});

const LOGS = [];
function attachedLog(sessionId, kind, date, targets) {
  const id = `log-${kind}-${date}`;
  LOGS.push({
    id,
    path: `Logs/${kind}/${date}/${kind}-${date}.log`,
    kind,
    utcOffset: "+02:00",
    start: `${date}T10:00:00Z`,
    end: `${date}T20:00:00Z`,
    matchedFrames: 40,
    sessions: [sessionId],
  });
  return { logId: id, targets, matchedFrames: 40 };
}

function night(fields) {
  const id = `s-${fields.date}-1`;
  return {
    id,
    eveningDate: fields.date,
    writtenEveningDate: fields.date,
    place: fields.place,
    equipment: ["equip-1"],
    start: fields.targets.map((item) => item.start).sort()[0],
    end: fields.targets
      .map((item) => item.end)
      .sort()
      .at(-1),
    targets: fields.targets,
    clock: fields.clock ?? null,
    shiftHours: fields.shiftHours ?? 0,
    calibration: fields.calibration,
    logs: fields.logs.map(([kind, keys]) =>
      attachedLog(id, kind, fields.date, keys),
    ),
    existing: fields.existing,
  };
}

const t0731 = target(TARGETS.m31, "2026-07-31", {
  frames: 64,
  exposure: 300,
  start: "12:10",
  end: "18:25",
});
const t0803 = target(TARGETS.ngc7000, "2026-08-03", {
  frames: 72,
  exposure: 180,
  start: "12:20",
  end: "15:56",
});
const t0807 = target(TARGETS.sh2103, "2026-08-07", {
  frames: 40,
  exposure: 300,
  start: "11:30",
  end: "15:00",
});
const t0809ic = target(TARGETS.ic1805, "2026-08-09", {
  frames: 98,
  exposure: 180,
  start: "11:10",
  end: "19:05",
  existing: "in-session",
});
const t0809sh = target(TARGETS.sh2103, "2026-08-09", {
  frames: 36,
  exposure: 300,
  start: "13:40",
  end: "16:40",
  existing: "overlap",
});
const t0810 = target(TARGETS.ic1805, "2026-08-10", {
  frames: 45,
  exposure: 180,
  start: "11:40",
  end: "19:10",
});
const t0811ic = target(TARGETS.ic1805, "2026-08-11", {
  frames: 80,
  exposure: 180,
  start: "11:15",
  end: "18:00",
  existing: "in-session",
});
const t0811sh = target(TARGETS.sh2103, "2026-08-11", {
  frames: 18,
  exposure: 300,
  start: "18:10",
  end: "19:40",
  existing: "add-to-session",
});
const t0815 = target(TARGETS.ngc7000, "2026-08-15", {
  frames: 60,
  exposure: 180,
  start: "12:05",
  end: "15:05",
});

/** Darks, flats and bias of the night, all taken the same day. */
function setsOfTheDay(date, at) {
  return [
    calibrationOf(
      set(`dark-${date}`, "dark", date, {
        frames: 30,
        exposure: 180,
        temperature: -10,
        start: at[0],
        end: at[1],
      }),
      "dark",
    ),
    calibrationOf(
      set(`flat-${date}`, "flat", date, {
        frames: 25,
        start: at[2],
        end: at[3],
      }),
      "flat",
    ),
    calibrationOf(
      set(`bias-${date}`, "bias", date, {
        frames: 50,
        temperature: -10,
        start: at[4],
        end: at[5],
      }),
      "bias",
    ),
  ];
}

const flatAndBias = (date) => [
  calibrationOf(
    set(`flat-${date}`, "flat", date, {
      frames: 25,
      start: "15:30",
      end: "15:50",
    }),
    "flat",
  ),
  calibrationOf(
    set(`bias-${date}`, "bias", date, {
      frames: 50,
      temperature: -10,
      start: "15:55",
      end: "16:05",
    }),
    "bias",
  ),
];

const MORNING = ["09:00", "09:30", "19:00", "19:20", "19:30", "19:40"];

const SESSIONS = [
  night({
    date: "2026-07-31",
    place: "place-1",
    targets: [t0731],
    calibration: setsOfTheDay("2026-07-31", MORNING),
    logs: [
      ["asiair", ["object:m31"]],
      ["phd2", ["object:m31"]],
    ],
    existing: {
      kind: "replace-plan",
      planId: "plan-m33",
      planName: "M33 – RVB",
    },
  }),
  night({
    date: "2026-08-03",
    place: "place-1",
    targets: [t0803],
    shiftHours: -9,
    clock: {
      place: { from: "default-place", placeId: "place-maison" },
      framesChecked: 72,
      suspect: true,
      reasons: [{ code: "sun-up", frames: 72 }],
      consistentAsWritten: 0,
      consistentAfterShift: 72,
      proposedShiftHours: -9,
      samples: [
        {
          shiftHours: 0,
          utc: "2026-08-03T23:08:00Z",
          sunAltitudeDeg: 14.2,
          targetAltitudeDeg: -20,
        },
        {
          shiftHours: -9,
          utc: "2026-08-03T14:08:00Z",
          sunAltitudeDeg: -24.1,
          targetAltitudeDeg: 38.4,
        },
      ],
    },
    calibration: [
      calibrationOf(
        set("dark-2026-08-03", "dark", "2026-08-03", {
          frames: 30,
          exposure: 180,
          temperature: -10,
          start: "09:00",
          end: "09:30",
        }),
        "dark",
      ),
      calibrationOf(
        set("flat-2026-08-03", "flat", "2026-08-03", {
          frames: 25,
          start: "16:30",
          end: "16:50",
        }),
        "flat",
      ),
    ],
    logs: [["asiair", ["object:ngc7000"]]],
    existing: { kind: "create" },
  }),
  night({
    date: "2026-08-07",
    place: "place-2",
    targets: [t0807],
    calibration: flatAndBias("2026-08-07"),
    logs: [
      ["asiair", ["object:sh2103"]],
      ["phd2", ["object:sh2103"]],
    ],
    existing: {
      kind: "reuse-plan",
      planId: "plan-voile-oiii",
      planName: "Voile – OIII",
    },
  }),
  night({
    date: "2026-08-09",
    place: "place-1",
    targets: [t0809ic, t0809sh],
    calibration: setsOfTheDay("2026-08-09", [
      "09:00",
      "09:30",
      "19:30",
      "19:50",
      "20:00",
      "20:10",
    ]),
    logs: [
      ["asiair", ["object:ic1805", "object:sh2103"]],
      ["phd2", ["object:ic1805"]],
    ],
    existing: { kind: "existing", sessionId: "sess-0809" },
  }),
  night({
    date: "2026-08-10",
    place: "place-1",
    targets: [t0810],
    calibration: [
      calibrationOf(
        set("flat-2026-08-10", "flat", "2026-08-10", {
          frames: 25,
          start: "19:20",
          end: "19:40",
        }),
        "flat",
      ),
      // The darks of the 9th: none were taken on the 10th.
      calibrationOf("dark-2026-08-09", "dark", 0.7),
    ],
    logs: [["asiair", ["object:ic1805"]]],
    existing: { kind: "create" },
  }),
  night({
    date: "2026-08-11",
    place: "place-1",
    targets: [t0811ic, t0811sh],
    calibration: setsOfTheDay("2026-08-11", [
      "09:00",
      "09:30",
      "19:50",
      "20:10",
      "20:20",
      "20:30",
    ]),
    logs: [
      ["asiair", ["object:ic1805", "object:sh2103"]],
      ["phd2", ["object:ic1805"]],
    ],
    existing: { kind: "existing", sessionId: "sess-0811" },
  }),
  night({
    date: "2026-08-15",
    place: "place-3",
    targets: [t0815],
    calibration: flatAndBias("2026-08-15"),
    logs: [["asiair", ["object:ngc7000"]]],
    existing: { kind: "create" },
  }),
];

const sid = (date) => `s-${date}-1`;

const groupNights = (rows) =>
  rows.map(([date, frames, seconds]) => ({
    sessionId: sid(date),
    eveningDate: date,
    frames,
    seconds,
  }));

const MULTI_NIGHT = [
  {
    id: "mn-ic1805",
    targetKey: "object:ic1805",
    targetId: "target-ic1805",
    designation: "IC 1805",
    instrument: INSTRUMENT,
    nights: groupNights([
      ["2026-08-09", 98, 17640],
      ["2026-08-10", 45, 8100],
      ["2026-08-11", 80, 14400],
    ]),
    goalSeconds: 40140,
    frames: 223,
    status: "finished",
    locked: false,
  },
  {
    id: "mn-ngc7000",
    targetKey: "object:ngc7000",
    targetId: "target-ngc7000",
    designation: "NGC 7000",
    instrument: INSTRUMENT,
    nights: groupNights([
      ["2026-08-03", 72, 12960],
      ["2026-08-15", 60, 10800],
    ]),
    goalSeconds: 23760,
    frames: 132,
    status: "finished",
    locked: false,
  },
];

// ------------------------------------------------------------------- lines

function line(kind, subject, checked, reason, text, flags = {}) {
  return {
    id: `${reason}:${subject}`,
    kind,
    subject,
    checked,
    locked: Boolean(flags.locked),
    ask: Boolean(flags.ask),
    reason,
    text,
  };
}

const SESSION_REASONS = {
  create: "session.create",
  existing: "session.existing",
  "reuse-plan": "plan.reuse",
  "replace-plan": "plan.replace",
  "plan-exists": "plan.freeRuleExisting",
};

/** The lines of the review, as `propose.rs::lines_of` writes them: places,
 * equipment, then each night (the night, its targets, its clock, its
 * calibration, its logs), then the multi-night plans. The `text` is the
 * core's French, never shown. */
export function proposalLines(proposal) {
  const lines = [];
  for (const place of proposal.places) {
    const chosen = place.candidates.find((c) => c.placeId === place.chosen);
    if (place.action === "use" && chosen) {
      lines.push(
        line("place", place.id, true, "place.known", `Lieu : ${chosen.name}`),
      );
    } else if (place.action === "use-nearest" && chosen) {
      lines.push(
        line(
          "place",
          place.id,
          true,
          "place.nearest",
          `Lieu : ${chosen.name}, le plus proche de ${place.candidates.length} lieux à moins de 10 km`,
        ),
      );
    } else if (place.action === "choose") {
      lines.push(
        line("place", place.id, false, "place.choose", "Lieu : à choisir", {
          ask: true,
        }),
      );
    } else {
      lines.push(
        line("place", place.id, true, "place.create", "Lieu inconnu : à créer"),
      );
    }
  }
  for (const item of proposal.equipment) {
    lines.push(
      line("equipment", item.id, true, "equipment.known", "Matériel reconnu"),
    );
  }
  for (const session of proposal.sessions) {
    const free = session.existing.kind === "plan-exists";
    lines.push(
      line(
        "session",
        session.id,
        !free,
        SESSION_REASONS[session.existing.kind],
        `Nuit du ${session.eveningDate}`,
        { locked: free },
      ),
    );
    for (const item of session.targets) {
      const subject = `${session.id}/${item.key}`;
      const flags = free ? { locked: true } : {};
      const reasons = {
        "in-session": [
          "target.inSession",
          `${item.designation} : déjà dans la session`,
        ],
        "add-to-session": [
          "target.addToSession",
          `Ajouter ${item.designation} à la session`,
        ],
        new: ["target.detected", `${item.designation} : ${item.frames} poses`],
      };
      if (item.existing === "overlap") {
        lines.push(
          line(
            "target",
            subject,
            false,
            "target.overlap",
            `${item.designation} chevauche une autre cible`,
            { ask: true, locked: free },
          ),
        );
      } else {
        const [reason, text] = reasons[item.existing];
        lines.push(line("target", subject, !free, reason, text, flags));
      }
    }
    if (session.clock?.proposedShiftHours != null) {
      lines.push(
        line(
          "clock",
          session.id,
          true,
          "clock.shift",
          `Horloge suspecte : décaler de ${session.clock.proposedShiftHours} h`,
        ),
      );
    }
    for (const attached of session.calibration) {
      lines.push(
        line(
          "calibration",
          `${session.id}/${attached.setId}`,
          true,
          "calibration.attached",
          `Calibration : ${attached.kind}`,
        ),
      );
    }
    for (const attached of session.logs) {
      lines.push(
        line(
          "log",
          `${session.id}/${attached.logId}`,
          true,
          "log.attached",
          `Journal ${attached.logId}`,
        ),
      );
    }
  }
  for (const group of proposal.multiNight) {
    lines.push(
      group.locked
        ? line(
            "multi-night",
            group.id,
            false,
            "multiNight.locked",
            `Plan multi-nuits ${group.designation} : verrouillé`,
            { locked: true },
          )
        : line(
            "multi-night",
            group.id,
            true,
            "multiNight.create",
            `Plan multi-nuits ${group.designation}`,
          ),
    );
  }
  return lines;
}

const candidate = (placeId, name, from, known) => ({
  placeId,
  name,
  distanceKm:
    Math.round(
      distanceKm(
        from.latitude,
        from.longitude,
        known.latitude,
        known.longitude,
      ) * 10,
    ) / 10,
});

const numbered = (prefix, count, extension) =>
  Array.from(
    { length: count },
    (_, index) =>
      `${prefix}_${String(index + 1).padStart(4, "0")}.${extension}`,
  );

/** The engine's proposal for the reference folder, an account with
 * the advanced planning. 1 423 files read (1 186 FITS, 214 calibrations, 23
 * logs), 7 nights, 4 targets. */
export const PROPOSAL = (() => {
  const proposal = {
    reading: {
      filesRead: 1423,
      lights: 1186,
      calibrationFrames: 214,
      logFiles: 23,
      nights: 7,
      targets: 4,
      links: [],
    },
    places: [
      {
        id: "place-1",
        ...HOME_POSES,
        candidates: [candidate("place-maison", "Maison", HOME_POSES, MAISON)],
        chosen: "place-maison",
        action: "use",
        sessions: [
          sid("2026-07-31"),
          sid("2026-08-03"),
          sid("2026-08-09"),
          sid("2026-08-10"),
          sid("2026-08-11"),
        ],
      },
      {
        id: "place-2",
        ...NEAR_POSES,
        candidates: [
          candidate("place-maison", "Maison", NEAR_POSES, MAISON),
          candidate("place-site-noir", "Site noir", NEAR_POSES, SITE_NOIR),
        ].sort((a, b) => a.distanceKm - b.distanceKm),
        chosen: "place-maison",
        action: "use-nearest",
        sessions: [sid("2026-08-07")],
      },
      {
        id: "place-3",
        ...UNKNOWN_PLACE,
        candidates: [],
        chosen: null,
        action: "create",
        sessions: [sid("2026-08-15")],
      },
    ],
    equipment: [
      {
        id: "equip-1",
        telescop: "Askar 50P",
        instrume: INSTRUMENT,
        focalLengthMm: 250,
        camera: { equipmentId: "eq-asi533", name: INSTRUMENT, by: "name" },
        telescope: { equipmentId: "eq-askar", name: "Askar 50P", by: "name" },
        mount: null,
        sessions: SESSIONS.map((session) => session.id),
      },
    ],
    sessions: SESSIONS,
    multiNight: MULTI_NIGHT,
    calibrationSets: SETS,
    logs: LOGS,
    setAside: {
      withoutDate: numbered("Autorun/Light/Inconnu/Light_sans-date", 12, "fit"),
      unreadable: [],
      stacks: [],
      calibrationUnattached: numbered(
        "Calibration/Autres/dark_orphelin",
        11,
        "fit",
      ),
      logsUnattached: [
        "Logs/PHD2/PHD2_GuideLog_2026-06-02.txt",
        "Logs/PHD2/PHD2_GuideLog_2026-06-03.txt",
      ],
      otherFiles: [],
    },
    scoped: null,
    lines: [],
    notes: [],
  };
  proposal.lines = proposalLines(proposal);
  return proposal;
})();

/** State 5: the same folder for an account without the advanced planning
 * (`DETECT_USER_FREE`): the multi-night groups come locked. */
export const PROPOSAL_FREE = (() => {
  const proposal = {
    ...PROPOSAL,
    multiNight: PROPOSAL.multiNight.map((group) => ({
      ...group,
      locked: true,
    })),
    notes: [
      {
        key: "account.free",
        text: "Sans abonnement ni Planification avancée : un plan par nuit.",
      },
    ],
    lines: [],
  };
  proposal.lines = proposalLines(proposal);
  return proposal;
})();

// ----------------------------------------------------------------- created

const created = (date, fields) => ({
  detectedId: sid(date),
  sessionId: `created-${date}`,
  eveningDate: date,
  action: "created",
  planAction: "created",
  planId: `plan-created-${date}`,
  designations: [],
  frames: 0,
  shiftHours: 0,
  ...fields,
});

/** State 6: what the service made of `PROPOSAL` as left ticked (the case
 * left unanswered is not added, the 15 August waits for its place): four
 * sessions created, two updated, the plan of IC 1805. */
export const CREATED = {
  sessions: [
    created("2026-07-31", {
      designations: ["M31"],
      frames: 64,
      planAction: "replaced",
      planId: "plan-m33",
    }),
    created("2026-08-03", {
      designations: ["NGC 7000"],
      frames: 72,
      shiftHours: -9,
    }),
    created("2026-08-07", {
      designations: ["Sh2-103"],
      frames: 40,
      planAction: "reused",
      planId: "plan-voile-oiii",
    }),
    created("2026-08-10", { designations: ["IC 1805"], frames: 45 }),
    created("2026-08-09", {
      sessionId: "sess-0809",
      action: "updated",
      planAction: "none",
      planId: "plan-voile-coeur",
      designations: ["IC 1805"],
      frames: 98,
    }),
    created("2026-08-11", {
      sessionId: "sess-0811",
      action: "updated",
      planAction: "none",
      planId: null,
      designations: ["IC 1805", "Sh2-103"],
      frames: 98,
      // « Sh2-103 ajoutée à la session du 11 août ».
      addedTargets: ["Sh2-103"],
    }),
  ],
  multiNight: [
    {
      groupId: "mn-ic1805",
      planId: "plan-multi-ic1805",
      designation: "IC 1805",
      sessionIds: ["sess-0809", "created-2026-08-10", "sess-0811"],
      goalSeconds: 40140,
    },
  ],
  placesCreated: [],
};

// ---------------------------------------------------------------- organise

const DESTINATION = "/home/astro/Documents/Astro Sync";
const NIGHTS = [
  "2026-07-31 · Maison · 5c18d4",
  "2026-08-03 · Maison · a02f61",
  "2026-08-07 · Maison · 91b7c0",
  "2026-08-09 · Maison · 7d44e2",
  "2026-08-10 · Maison · e5a903",
  "2026-08-11 · Maison · 3e7a91",
];

/** 978 files: 395 poses, 560 of calibration and 23 logs, over the nights. */
function organiseActions() {
  return Array.from({ length: 978 }, (_, index) => {
    const night = NIGHTS[index % NIGHTS.length];
    const number = String(index + 1).padStart(4, "0");
    const [name, source, folder, bytes] =
      index < 395
        ? [
            `Light_${number}.fit`,
            "ASIAIR/Autorun/Light",
            `${night}/IC 1805/lights`,
            18_000_000,
          ]
        : index < 955
          ? [
              `Cal_${number}.fit`,
              "Calibration",
              `${night}/calibration/darks`,
              18_000_000,
            ]
          : [`Log_${number}.log`, "Logs", `${night}/logs`, 40_000];
    return {
      source: `/home/astro/Astro/${source}/${name}`,
      destination: `${DESTINATION}/${folder}/${name}`,
      kind: "copy",
      bytes,
    };
  });
}

const ACTIONS = organiseActions();

/** State 7: the dry run of « Copier » for the sessions created (978 files). */
export const ORGANISE_PLAN = {
  mode: "copy",
  destination: DESTINATION,
  actions: ACTIONS,
  conflicts: [],
  files: ACTIONS.length,
  bytes: ACTIONS.reduce((sum, action) => sum + action.bytes, 0),
  windowsNoLink: false,
  // « 412 Go libres sur ce disque ».
  freeBytes: 412_000_000_000,
  summary: "Copie de 978 fichiers.",
};

/** State 8: the end of a « Déplacer » with three checks failed: those files
 * stay in their source, intact. */
export const ORGANISE_OUTCOME = {
  done: 975,
  verified: 975,
  removedSources: 975,
  alreadyThere: 0,
  failed: ACTIONS.slice(10, 13).map((action) => ({
    source: action.source,
    destination: action.destination,
    reason: "checksum-differs",
    detail: null,
  })),
  stopped: false,
};
