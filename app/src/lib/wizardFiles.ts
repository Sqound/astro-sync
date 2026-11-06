// The wizard's step « Fichiers »: per target, its final image (here, already
// online and identical, or different and sent as a variant of the capture), its
// frames, its findings; then the calibration, the logs and the files outside
// the structure. Pure TypeScript, tested under `node --test`.

import type { TranslateParams } from "../i18n/locale.ts";
import type {
  Finding,
  FrameSummary,
  ImagePlan,
  NightDetection,
  NightOnline,
  OnlineCapture,
  TargetDetection,
} from "./bridge";
import { deviceBox, hasDeviceImage, type DeviceBox } from "./deviceMaster.ts";
import { calibrationFinding } from "./missingMessages.ts";
import { baseName } from "./wizardSteps.ts";
import { formatSize, pluralText, shortDigest } from "./written.ts";

type Translate = (key: string, params?: TranslateParams) => string;

/** A piece of a line: plain text, bold, or a path in the code face. */
export type Seg = string | { strong: string } | { code: string };

export type Glyph =
  "ok" | "missing" | "warning" | "none" | "phone" | "variant" | "online";

export interface FileLine {
  glyph: Glyph;
  segs: Seg[];
  /** « Nouveau »: added to the folder since the night was sent. */
  isNew?: boolean;
  /** The blue-outlined row: what changes for this target (a variant). */
  framed?: boolean;
  /** A link at the end of the line: opens the folder of the darks or of the
   * flats. */
  action?: "openDarks" | "openFlats";
  /** A warning drawn as an amber block (the darks that do not fit, the
   * filter without flats): the result will be worse and the person can act. */
  banner?: boolean;
}

export type Badge =
  | { tone: "success"; key: "ready" | "online"; phone?: boolean }
  | { tone: "error"; key: "empty" }
  | { tone: "warning"; key: "finalMissing" | "framesMissing" }
  | { tone: "warning"; key: "tidy"; count: number };

/** One of the two frames of a target's card: « Poses FITS » (`lights/`) or
 * « Image finale » (`final/`), each with its own state. */
export interface FileBox {
  /** Found (green), missing but the other frame carries the target
   * (amber), missing and nothing else to send (red). */
  state: {
    tone: "success" | "warning" | "error";
    key: "found" | "online" | "missing";
  };
  lines: FileLine[];
}

export interface TargetCard {
  targetId: string;
  designation: string;
  folderName: string;
  badge: Badge;
  frames: FileBox;
  /** The image a device already stacked, under the frames;
   * `null` when there is none. */
  device: DeviceBox | null;
  final: FileBox;
  /** What else the detection said of the target (misfiled frames…). */
  lines: FileLine[];
  /** Misfiled frames: « Demander à un assistant IA » for this target. */
  askAssistant: boolean;
}

export interface FilesView {
  /** The grey line under « Fichiers ». */
  summary: string;
  targets: TargetCard[];
  calibration: FileLine[] | null;
  logs: { lines: FileLine[]; empty: boolean };
  outside: Seg[] | null;
  /** « IC 1396 partira sans capture tant que son image finale manque … ». */
  footer: string | null;
}

const SOURCE_KEY: Record<string, string> = {
  phone: "wizard.files.fromPhone",
  web: "wizard.files.fromWeb",
  "astro-sync": "wizard.files.fromAstroSync",
};

function sourceText(t: Translate, source: string | null | undefined): string {
  return t(SOURCE_KEY[source ?? ""] ?? "wizard.files.fromAstroSync");
}

function shortDate(instant: string, locale: string): string {
  const date = new Date(instant);
  if (Number.isNaN(date.getTime())) return "";
  return new Intl.DateTimeFormat(locale, {
    day: "numeric",
    month: "short",
  }).format(date);
}

function seconds(
  t: Translate,
  locale: string,
  values: readonly number[],
): string {
  const texts = values.map((value) =>
    t("common.secondsShort", {
      count: new Intl.NumberFormat(locale, { maximumFractionDigits: 1 }).format(
        value,
      ),
    }),
  );
  if (texts.length <= 1) return texts[0] ?? "";
  return t("wizard.files.and", {
    first: texts.slice(0, -1).join(", "),
    last: texts[texts.length - 1],
  });
}

/** « 40 poses · 180 s · Ha 20, OIII 20 · gain 100 · 1,3 Go ». */
export function framesText(
  t: Translate,
  locale: string,
  summary: FrameSummary,
  frames: number,
  options: { filters?: boolean; size?: boolean } = {},
): string {
  const parts = [pluralText(t, locale, "wizard.files.frames", frames)];
  if (summary.exposures.length > 0) {
    parts.push(seconds(t, locale, summary.exposures));
  }
  if (options.filters !== false) {
    const named = summary.filters.filter((f) => f.filter !== null);
    if (named.length > 0) {
      parts.push(
        summary.filters
          .map((f) =>
            f.filter === null
              ? t("wizard.files.noFilterCount", { count: f.frames })
              : `${f.filter} ${f.frames}`,
          )
          .join(", "),
      );
    } else if (summary.filters.length > 0) {
      parts.push(t("wizard.files.noFilter"));
    }
  }
  if (summary.gains.length > 0) {
    parts.push(
      t("wizard.files.gain", {
        gain: summary.gains
          .map((gain) => new Intl.NumberFormat(locale).format(gain))
          .join(", "),
      }),
    );
  }
  if (options.size !== false && summary.bytes > 0) {
    parts.push(formatSize(t, summary.bytes, locale));
  }
  return parts.join(" · ");
}

/** The sentence of a target with no final image whose device stacked one:
 * the device's image is the source, and the poses it does not cover are the
 * only ones stacked -- never « seules les poses FITS ». */
function finalFromDeviceText(t: Translate, restackFrames: number): string {
  return restackFrames > 0
    ? t("wizard.files.finalFromDeviceAndFrames", { count: restackFrames })
    : t("wizard.files.finalFromDevice");
}

/** The poses of a target that no device image covers: those that are stacked. */
function restackFramesOf(target: TargetDetection): number {
  const covered = (target.device?.families ?? [])
    .filter((family) => family.defaultSource === "device")
    .reduce((sum, family) => sum + family.lights, 0);
  return Math.max(0, target.lightFrames - covered);
}

/** A finding of the detection in the window's words (one line). The key
 * is the core's (`detection::keys`); the sentence is the sheet's. */
export function findingSegs(
  t: Translate,
  locale: string,
  finding: Finding,
): Seg[] {
  const p = finding.params;
  const text = (name: string) =>
    p[name] === null || p[name] === undefined ? "" : String(p[name]);
  const count = Number(p.count ?? finding.files.length ?? 1) || 1;
  switch (finding.key) {
    case "detection.finalMissing":
      return [t("wizard.files.finalMissing")];
    case "detection.finalFromDevice":
      return [finalFromDeviceText(t, Number(p.restackFrames ?? 0))];
    case "detection.framesMissing":
      return [t("wizard.files.framesMissing")];
    case "detection.targetEmpty":
      return [
        t("wizard.files.targetEmptyLead"),
        { strong: text("lightsFolder") },
        t("wizard.files.targetEmptyOr"),
        { strong: text("folder") },
        ".",
      ];
    case "detection.finalFitsRefused":
      return [t("wizard.files.finalFitsRefused")];
    case "detection.framesMisfiled": {
      const kind = text("kind") || "flat";
      const head = pluralText(
        t,
        locale,
        `wizard.files.misfiled.${kind}`,
        count,
      );
      if (!text("to")) {
        return [
          head,
          " ",
          { code: text("from") },
          t("wizard.files.misfiledToLights"),
        ];
      }
      return [
        head,
        " ",
        { code: text("from") },
        p.fromLights === true
          ? pluralText(t, locale, "wizard.files.misfiledHeader", count, {
              header: text("header"),
            })
          : t("wizard.files.misfiledMove"),
        { code: text("to") },
        ".",
      ];
    }
    case "detection.calibrationHeaderDiffers": {
      // The folder is right; the header says another type. Not a
      // tidying (nothing is moved), a warning to check.
      const kind = text("kind") || "dark";
      return [
        pluralText(t, locale, "wizard.files.calibrationHeaderDiffers", count),
        " ",
        { code: text("folder") },
        pluralText(
          t,
          locale,
          "wizard.files.calibrationHeaderDiffersTail",
          count,
          {
            header: text("header"),
            kind: pluralText(
              t,
              locale,
              `wizard.files.calibrationKind.${kind}`,
              count,
            ),
          },
        ),
      ];
    }
    case "detection.framesOtherTarget":
      return [
        pluralText(t, locale, "wizard.files.otherTarget", count, {
          object: text("object"),
        }),
        " ",
        { code: text("folder") },
        t("wizard.files.otherTargetHeader"),
        { code: `${text("object")}/lights/` },
        ".",
      ];
    case "detection.flatsMissing":
    case "detection.darksMissing":
      return [calibrationFinding(t, locale, finding)?.files ?? finding.key];
    case "detection.framesOtherNight":
      return [
        pluralText(t, locale, "wizard.files.otherNight", count, {
          date: shortDate(text("date"), locale),
        }),
        " ",
        { code: text("folder") },
        ".",
      ];
    case "detection.linkTargetMissing":
      return [t("wizard.files.linkMissing"), " ", { code: text("path") }];
    case "detection.linkLoop":
      return [t("wizard.files.linkLoop"), " ", { code: text("path") }];
    default:
      return [finding.key];
  }
}

function targetFindings(
  detection: NightDetection,
  targetId: string,
  folderName: string,
): Finding[] {
  return detection.findings.filter(
    (finding) =>
      (finding.targetId === targetId ||
        (finding.targetId === null &&
          String(finding.params.from ?? "").startsWith(`${folderName}/`))) &&
      finding.key !== "detection.finalMissing" &&
      finding.key !== "detection.finalFromDevice" &&
      finding.key !== "detection.framesMissing" &&
      finding.key !== "detection.targetEmpty" &&
      finding.key !== "detection.noLogs" &&
      finding.key !== "detection.filesOutsideStructure" &&
      // The darks and the flats that are missing are said in the Calibration
      // card, not under each target.
      finding.key !== "detection.darksMissing" &&
      finding.key !== "detection.flatsMissing",
  );
}

function captureOf(
  online: NightOnline | null,
  targetId: string,
): OnlineCapture | null {
  const target = online?.targets.find((item) => item.targetId === targetId);
  return target?.captures.find((capture) => capture.mainImage) ?? null;
}

// « framesOtherNight » is not here: those frames are kept and sent with the
// target, so it is a plain warning, not something to tidy.
const MISFILED_KEYS = new Set([
  "detection.framesMisfiled",
  "detection.framesOtherTarget",
]);

export function filesView(
  t: Translate,
  locale: string,
  input: {
    detection: NightDetection;
    online: NightOnline | null;
    plans: readonly ImagePlan[];
    /** The resume found logs or frames added since the sending. */
    added: boolean;
    eveningDay: string;
  },
): FilesView {
  const { detection, online, plans, added } = input;
  const somethingOnline = (online?.targets ?? []).some((target) =>
    target.captures.some((capture) => capture.mainImage),
  );
  const cards: TargetCard[] = [];
  const statuses: {
    name: string;
    status: "ready" | "online" | "variant" | "missing" | "empty" | "tidy";
    tidy: number;
  }[] = [];
  let warnings = 0;
  for (const target of detection.targets.filter((item) => item.inNight)) {
    const finalLines: FileLine[] = [];
    const lines: FileLine[] = [];
    const capture = captureOf(online, target.targetId);
    const plan = plans.find(
      (item) =>
        item.image.targetId === target.targetId && item.image.kind === "final",
    );
    const finalName = target.finalImage ? baseName(target.finalImage) : null;
    let status: (typeof statuses)[number]["status"] = "ready";
    if (plan?.decision.decision === "already_online") {
      status = "online";
      finalLines.push({
        glyph: "ok",
        segs: [
          { strong: finalName ?? baseName(plan.image.path) },
          t("wizard.files.sizeThen", {
            size: formatSize(t, plan.image.size, locale),
          }),
          { strong: t("wizard.files.identical") },
          t("wizard.files.identicalDigest", {
            digest: shortDigest(plan.image.sha256),
          }),
        ],
      });
    } else if (capture?.mainImage && finalName && plan) {
      status = "variant";
      const main = capture.mainImage;
      finalLines.push({
        glyph: main.source === "phone" ? "phone" : "online",
        segs: [
          t("wizard.files.onlineLead"),
          { strong: main.originalName },
          t("wizard.files.onlineKept", {
            source: sourceText(t, main.source),
            date: shortDate(main.createdAt, locale),
            size: formatSize(t, main.byteSize, locale),
          }),
        ],
      });
      finalLines.push({
        glyph: "variant",
        framed: true,
        segs: [
          { strong: finalName },
          ` · ${formatSize(t, plan.image.size, locale)}, `,
          { strong: t("wizard.files.different") },
          t("wizard.files.differentDigest", {
            digest: shortDigest(plan.image.sha256),
          }),
          { strong: t("wizard.files.variantOfCapture") },
          ".",
        ],
      });
    } else if (capture?.mainImage && !finalName) {
      status = "online";
      const main = capture.mainImage;
      const annotated = main.skySolution === "present";
      finalLines.push({
        glyph: main.source === "phone" ? "phone" : "online",
        segs: [
          t("wizard.files.onlineOnlyLead"),
          { strong: main.originalName },
          annotated
            ? t("wizard.files.onlineOnlyAnnotated", {
                source: sourceText(t, main.source),
              })
            : t("wizard.files.onlineOnly", {
                source: sourceText(t, main.source),
              }),
        ],
      });
    } else if (finalName) {
      finalLines.push({
        glyph: "ok",
        segs: [
          { code: "final/" },
          ` · ${finalName}${
            target.finalBytes
              ? ` · ${formatSize(t, target.finalBytes, locale)}`
              : ""
          }${somethingOnline ? t("wizard.files.notYetOnline") : ""}`,
        ],
      });
    }
    const hasFinal = finalLines.length > 0;
    const hasFrames = target.lightFrames > 0;
    // The image a device stacked stands for the poses when they are not
    // here: the target has something to render.
    const hasDevice = hasDeviceImage(target);
    const hasSource = hasFrames || hasDevice;
    const folder = target.folderName ?? target.designation;
    if (!hasFinal) {
      status = hasSource ? "missing" : "empty";
      finalLines.push({
        glyph: hasSource ? "warning" : "missing",
        segs: hasSource
          ? [
              hasDevice
                ? finalFromDeviceText(t, restackFramesOf(target))
                : t("wizard.files.finalMissing"),
            ]
          : [t("wizard.files.finalNone"), { strong: `${folder}/final/` }, "."],
      });
    }
    const framesLines: FileLine[] = [];
    if (hasFrames) {
      framesLines.push({
        glyph: "ok",
        isNew: added,
        segs: [
          { code: "lights/" },
          ` · ${framesText(t, locale, target.lightSummary, target.lightFrames)}`,
        ],
      });
    } else if (hasDevice) {
      framesLines.push({
        glyph: "warning",
        segs: [t("wizard.files.framesDeviceOnly")],
      });
    } else {
      framesLines.push({
        glyph: hasFinal ? "warning" : "missing",
        segs: hasFinal
          ? [t("wizard.files.framesMissing")]
          : [
              t("wizard.files.framesNone"),
              { strong: `${folder}/lights/` },
              ".",
            ],
      });
    }
    if (!hasFinal && !hasSource) {
      lines.push({
        glyph: "missing",
        segs: [
          t("wizard.files.targetEmptyLead"),
          { strong: `${folder}/lights/` },
          t("wizard.files.targetEmptyOr"),
          { strong: `${folder}/final/` },
          ".",
        ],
      });
    }
    let tidy = 0;
    for (const finding of targetFindings(
      detection,
      target.targetId,
      target.folderName ?? target.designation,
    )) {
      if (MISFILED_KEYS.has(finding.key)) tidy += 1;
      else warnings += 1;
      lines.push({ glyph: "warning", segs: findingSegs(t, locale, finding) });
    }
    const framesGap = hasFinal && !hasSource;
    if (framesGap) warnings += 1;
    if (tidy > 0) status = "tidy";
    statuses.push({ name: target.designation, status, tidy });
    cards.push({
      targetId: target.targetId,
      designation: target.designation,
      folderName: target.folderName ?? target.designation,
      badge:
        !hasFinal && !hasSource
          ? { tone: "error", key: "empty" }
          : status === "tidy"
            ? { tone: "warning", key: "tidy", count: tidy }
            : !hasFinal
              ? { tone: "warning", key: "finalMissing" }
              : framesGap
                ? { tone: "warning", key: "framesMissing" }
                : status === "online" || status === "variant"
                  ? {
                      tone: "success",
                      key: "online",
                      phone: capture?.mainImage?.source === "phone",
                    }
                  : { tone: "success", key: "ready" },
      frames: {
        state: hasFrames
          ? { tone: "success", key: "found" }
          : {
              tone: hasFinal || hasDevice ? "warning" : "error",
              key: "missing",
            },
        lines: framesLines,
      },
      device: deviceBox(t, locale, target),
      final: {
        state: hasFinal
          ? {
              tone: "success",
              key: capture?.mainImage ? "online" : "found",
            }
          : { tone: hasFrames ? "warning" : "error", key: "missing" },
        lines: finalLines,
      },
      lines,
      askAssistant: tidy > 0,
    });
  }

  // The calibration, when there is some. The darks and the flats that are
  // missing are said here: a warning under the folder
  // concerned when calibration of the kind exists and does not fit, an
  // information when there is none at all.
  const c = detection.calibration;
  const cs = detection.calibrationSummary;
  const missing = (kind: "dark" | "flat"): FileLine[] => {
    const lines: FileLine[] = [];
    for (const finding of detection.findings) {
      const item = calibrationFinding(t, locale, finding);
      if (!item || item.kind !== kind) continue;
      if (lines.some((line) => line.segs[0] === item.files)) continue;
      if (item.level === "warning") warnings += 1;
      lines.push({
        glyph: item.level === "warning" ? "warning" : "none",
        banner: item.level === "warning",
        segs: [item.files],
        action: item.folder
          ? item.folder === "darks"
            ? "openDarks"
            : "openFlats"
          : undefined,
      });
    }
    return lines;
  };
  const darkGaps = missing("dark");
  const flatGaps = missing("flat");
  let calibration: FileLine[] | null = null;
  if (
    c.darks + c.flats + c.darkFlats + c.biases > 0 ||
    darkGaps.length + flatGaps.length > 0
  ) {
    calibration = [];
    if (c.darks > 0) {
      calibration.push({
        glyph: "ok",
        segs: [
          { code: "darks/" },
          ` · ${framesText(t, locale, cs.darks, c.darks, { filters: false, size: false })}`,
        ],
      });
    }
    calibration.push(...darkGaps);
    if (c.flats > 0) {
      calibration.push({
        glyph: "ok",
        segs: [
          { code: "flats/" },
          // The same form as the darks and the lights: « 50 poses · Ha 25,
          // OIII 25 », « 3 poses · sans filtre », « 3 poses · Ha 2, sans
          // filtre 1 » (the exposure and the gain of the flats are not said
          // here, as before).
          ` · ${framesText(
            t,
            locale,
            { ...cs.flats, exposures: [], gains: [] },
            c.flats,
            { size: false },
          )}`,
        ],
      });
    } else if (flatGaps.length === 0) {
      calibration.push({ glyph: "none", segs: [t("wizard.files.noFlats")] });
    }
    calibration.push(...flatGaps);
    if (c.biases > 0) {
      calibration.push({
        glyph: "ok",
        segs: [
          { code: "biases/" },
          ` · ${pluralText(t, locale, "wizard.files.frames", c.biases)}`,
        ],
      });
    }
    if (c.darkFlats > 0) {
      calibration.push({
        glyph: "ok",
        segs: [
          { code: "dark-flats/" },
          ` · ${pluralText(t, locale, "wizard.files.frames", c.darkFlats)}`,
        ],
      });
    } else if (c.biases > 0) {
      calibration.push({
        glyph: "ok",
        segs: [
          { code: "dark-flats/" },
          ` · ${t("wizard.files.darkFlatsEmpty")}`,
        ],
      });
    }
  }

  // A calibration frame whose header says another type than its
  // folder: said in the calibration zone, once per folder and header.
  if (calibration) {
    for (const finding of detection.findings.filter(
      (item) => item.key === "detection.calibrationHeaderDiffers",
    )) {
      warnings += 1;
      calibration.push({
        glyph: "warning",
        segs: findingSegs(t, locale, finding),
      });
    }
  }

  const logs: FileLine[] = detection.logs.map((log) => {
    const file = baseName(log.relativePath);
    const what =
      log.kind === "nina"
        ? t("wizard.files.logNina", { day: input.eveningDay })
        : log.kind === "phd2"
          ? t("wizard.files.logPhd2", { file })
          : log.kind === "asiair"
            ? t("wizard.files.logAsiair", { file })
            : t("wizard.files.logUnknown", { file });
    return {
      glyph: log.kind === "unknown" ? "warning" : "ok",
      isNew: added,
      segs: [{ code: "logs/" }, ` · ${what}`],
    };
  });

  const outside =
    detection.outsideStructure.length > 0
      ? ([
          pluralText(
            t,
            locale,
            "wizard.files.outsideLead",
            detection.outsideStructure.length,
          ),
          { code: baseName(detection.outsideStructure[0]) },
          detection.outsideStructure.length > 1
            ? pluralText(
                t,
                locale,
                "wizard.files.outsideOthers",
                detection.outsideStructure.length - 1,
              )
            : t("wizard.files.outsideOne"),
        ] as Seg[])
      : null;

  const missingNames = statuses
    .filter((s) => s.status === "missing")
    .map((s) => s.name);
  const footer =
    missingNames.length > 0
      ? pluralText(
          t,
          locale,
          "wizard.files.footerMissing",
          missingNames.length,
          {
            targets: missingNames.join(", "),
          },
        )
      : null;

  return {
    summary: filesSummary(t, locale, statuses, warnings, added),
    targets: cards,
    calibration,
    logs: { lines: logs, empty: logs.length === 0 },
    outside,
    footer,
  };
}

/** The grey line under « Fichiers »: what stands out, in one line. */
export function filesSummary(
  t: Translate,
  locale: string,
  statuses: readonly { name: string; status: string; tidy: number }[],
  warnings: number,
  added: boolean,
): string {
  if (added) return t("wizard.files.summaryResume");
  if (statuses.some((s) => s.status === "online" || s.status === "variant")) {
    return statuses
      .map((s) =>
        s.status === "online"
          ? t("wizard.files.summaryOnline", { target: s.name })
          : s.status === "variant"
            ? t("wizard.files.summaryVariant", { target: s.name })
            : s.status === "missing"
              ? t("wizard.files.summaryMissing", { target: s.name })
              : s.status === "empty"
                ? t("wizard.files.summaryEmpty", { target: s.name })
                : t("wizard.files.summaryReady", { target: s.name }),
      )
      .join(" · ");
  }
  const tidy = statuses.filter((s) => s.status === "tidy");
  if (tidy.length > 0) {
    const ready = statuses
      .filter((s) => s.status === "ready")
      .map((s) => s.name);
    const parts = [];
    if (ready.length > 0) {
      parts.push(
        pluralText(t, locale, "wizard.files.summaryReadyNames", ready.length, {
          targets: listText(t, ready),
        }),
      );
    }
    for (const target of tidy) {
      parts.push(
        pluralText(t, locale, "wizard.files.summaryTidy", target.tidy, {
          target: target.name,
        }),
      );
    }
    return parts.join(" · ");
  }
  const missing = statuses.filter(
    (s) => s.status === "missing" || s.status === "empty",
  ).length;
  if (missing > 0 || warnings > 0) {
    const ready = statuses.length - missing;
    return [
      t("wizard.files.summaryCount", { ready, total: statuses.length }),
      warnings > 0
        ? pluralText(t, locale, "wizard.files.warnings", warnings)
        : null,
    ]
      .filter(Boolean)
      .join(" · ");
  }
  return t("wizard.files.summaryAllFound");
}

/** « NGC 7000 et IC 1396 »: a list in the language's words. */
export function listText(t: Translate, names: readonly string[]): string {
  if (names.length <= 1) return names[0] ?? "";
  return t("wizard.files.and", {
    first: names.slice(0, -1).join(", "),
    last: names[names.length - 1],
  });
}
