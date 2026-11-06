// « Réglages » (and its tab « Dépendances »): the clocks of the logs in
// « Général », and what each row of « Dépendances » says -- the rows fed by the
// registry as the core exposes them (`tools::detect_tools`), ASTAP shown with
// its star database, Tycho-2, and the stacking pipeline (its Python environment
// made here); then the section « Pipelines de traitement » (one block per
// pipeline found, one line per dependency it declares, the trust in a script
// the person added). Pure TypeScript, tested under `node --test`.

import type { TranslateParams } from "../i18n/locale.ts";
import type {
  DownloadFailure,
  EnvFailure,
  EnvProgress,
  LocalSettings,
  PipelineSection,
  PipelineSections,
  SectionDependency,
  ToolRow,
  TrustState,
} from "./bridge";
import { displayPath, homeOf } from "./home.ts";
import {
  formatSize,
  pluralText,
  shortDigest as writtenDigest,
} from "./written.ts";

type Translate = (key: string, params?: TranslateParams) => string;

/* ------------------------------ The UTC offsets --------------------------- */

export type OffsetKey = "ninaUtcOffset" | "phd2UtcOffset" | "asiairUtcOffset";

/** The three clocks, in the order of the row: NINA, PHD2, ASIAIR. */
export const OFFSET_FIELDS: readonly { key: OffsetKey; labelKey: string }[] = [
  { key: "ninaUtcOffset", labelKey: "settings.utc.nina" },
  { key: "phd2UtcOffset", labelKey: "settings.utc.phd2" },
  { key: "asiairUtcOffset", labelKey: "settings.utc.asiair" },
];

export type OffsetDraft = Record<OffsetKey, string>;

export function offsetDraft(settings: LocalSettings | null): OffsetDraft {
  return {
    ninaUtcOffset: settings?.ninaUtcOffset ?? "",
    phd2UtcOffset: settings?.phd2UtcOffset ?? "",
    asiairUtcOffset: settings?.asiairUtcOffset ?? "",
  };
}

/** What a field means: empty is UTC (`null`), `+HH:MM` / `-HH:MM` an
 * offset (« +2 » and « +2:00 » read as « +02:00 »), anything else is not
 * an offset. */
export function readOffset(
  text: string,
): { ok: true; value: string | null } | { ok: false } {
  const trimmed = text.trim();
  if (trimmed === "" || /^(utc|z)$/iu.test(trimmed)) {
    return { ok: true, value: null };
  }
  const match = /^([+-])(\d{1,2})(?::?(\d{2}))?$/u.exec(trimmed);
  if (!match) return { ok: false };
  const hours = Number(match[2]);
  const minutes = Number(match[3] ?? "0");
  if (hours > 14 || minutes > 59) return { ok: false };
  return {
    ok: true,
    value: `${match[1]}${String(hours).padStart(2, "0")}:${String(minutes).padStart(2, "0")}`,
  };
}

/** Whether « Enregistrer les décalages » is the zone's action: something
 * changed and every field reads. */
export function offsetsSavable(
  saved: LocalSettings | null,
  draft: OffsetDraft,
): boolean {
  let changed = false;
  for (const { key } of OFFSET_FIELDS) {
    const read = readOffset(draft[key]);
    if (!read.ok) return false;
    if (read.value !== (saved?.[key] ?? null)) changed = true;
  }
  return changed;
}

/* ------------------------------- Dépendances ------------------------------ */

/** A download running or ended badly, by row id; for the Python
 * environment (rows `stacking-pipeline` and `python-env`), its
 * installation. */
export type DownloadState =
  | { state: "downloading"; bytes: number; total: number | null }
  | { state: "failed"; failure: DownloadFailure }
  | { state: "installing"; progress: EnvProgress | null }
  | { state: "envFailed"; failure: EnvFailure };

/** The rows whose « Installer » makes the Python environment. */
export const ENVIRONMENT_ROWS: readonly string[] = [
  "stacking-pipeline",
  "python-env",
];

export type BadgeTone = "success" | "warning" | "error" | "neutral";

export interface DependencyRow {
  id: string;
  name: string;
  /** The registry's own name (« ASTAP »), for the line at the head. */
  shortName: string;
  optional: boolean;
  role: string;
  lines: string[];
  badge: { tone: BadgeTone; text: string };
  /** The download in progress, for the bar. */
  progress: { bytes: number; total: number | null } | null;
  /** `resumeInstall` / `cancelInstall`: the Python environment's. */
  action:
    "install" | "resume" | "cancel" | "resumeInstall" | "cancelInstall" | null;
  page: string | null;
}

/** The rows of the tab, in their order, then the two rows added
 * later (Tycho-2, the stacking pipeline). StarNet2 and the Python environment
 * belong to « Pipelines de traitement ». */
const ORDER = [
  "siril",
  "astap",
  "astrometry-net",
  "tycho-2",
  "stacking-pipeline",
] as const;

const ROLE: Record<(typeof ORDER)[number], string> = {
  siril: "deps.role.siril",
  astap: "deps.role.astap",
  "astrometry-net": "deps.role.astrometry",
  "tycho-2": "deps.role.tycho",
  "stacking-pipeline": "deps.role.stacking",
};

function present(row: ToolRow | undefined): boolean {
  return row?.state === "present";
}

/** A path as the window writes it: the home folder as « ~ ». */
function shownPath(path: string): string {
  return displayPath(path, homeOf(path));
}

/** The words of the Python environment's installation, step by step. */
export function envProgressLine(
  t: Translate,
  progress: EnvProgress | null,
): string {
  if (!progress) return t("deps.env.step.python");
  if (progress.step === "packages" && progress.packagesTotal > 0) {
    return t("deps.env.step.packages", {
      done: String(progress.packagesDone),
      total: String(progress.packagesTotal),
    });
  }
  return t(`deps.env.step.${progress.step}`);
}

/** The bar of the installation: the packages, else the steps. */
export function envProgressBar(progress: EnvProgress | null): {
  bytes: number;
  total: number;
} {
  if (!progress) return { bytes: 0, total: 4 };
  if (progress.step === "packages" && progress.packagesTotal > 0) {
    // Steps 1 and 2 done, the packages fill the third quarter.
    return {
      bytes: 2 * progress.packagesTotal + progress.packagesDone,
      total: 4 * progress.packagesTotal,
    };
  }
  return { bytes: progress.index - 1, total: progress.total };
}

/** Why the environment could not be made, in words (never a code), and
 * whether trying again resumes it. */
export function envFailureLines(
  t: Translate,
  failure: EnvFailure,
): { lines: string[]; resumable: boolean } {
  switch (failure.code) {
    case "no_python":
      return {
        lines: [
          t("deps.env.noPython", { first: failure.first, last: failure.last }),
        ],
        resumable: false,
      };
    case "no_resources":
      return { lines: [t("deps.env.noResources")], resumable: false };
    case "venv":
      return { lines: [t("deps.env.venv")], resumable: true };
    case "packages":
      return {
        lines: [t("deps.env.packages"), t("deps.env.kept")],
        resumable: true,
      };
    case "check":
      return {
        lines: [
          failure.problems.length > 0
            ? t("deps.env.check", { problems: failure.problems.join(", ") })
            : t("deps.env.checkPlain"),
        ],
        resumable: true,
      };
    case "cancelled":
      return {
        lines: [t("deps.env.cancelled"), t("deps.env.kept")],
        resumable: true,
      };
  }
}

/** What the Python environment's rows say when no installation runs. */
function environmentState(
  t: Translate,
  tool: ToolRow,
  range: { first: string; last: string },
): Pick<DependencyRow, "lines" | "badge" | "action"> {
  const blocked = tool.blockedBy ?? [];
  if (tool.state === "present") {
    return {
      lines: [
        tool.version
          ? t("deps.env.ready", { version: tool.version })
          : t("deps.env.readyPlain"),
      ],
      badge: {
        tone: "success",
        text: tool.version
          ? t("deps.badge.present", { version: tool.version })
          : t("deps.badge.presentPlain"),
      },
      action: null,
    };
  }
  if (blocked.includes("python")) {
    return {
      lines: [t("deps.env.noPython", range)],
      badge: { tone: "warning", text: t("deps.badge.missing") },
      action: null,
    };
  }
  if (tool.interrupted) {
    return {
      lines: [t("deps.env.interrupted"), t("deps.env.kept")],
      badge: { tone: "warning", text: t("deps.badge.missing") },
      action: tool.installable ? "resumeInstall" : null,
    };
  }
  if (tool.id === "stacking-pipeline" && !blocked.includes("python-env")) {
    // The environment is there; Siril is what is missing.
    return {
      lines: [t("deps.env.needsSiril")],
      badge: { tone: "warning", text: t("deps.badge.missing") },
      action: null,
    };
  }
  if (tool.state === "outdated") {
    return {
      lines: [t("deps.env.outdated")],
      badge: { tone: "warning", text: t("deps.badge.toRedo") },
      action: tool.installable ? "install" : null,
    };
  }
  return {
    lines: [
      t("deps.env.toInstall"),
      ...(blocked.includes("siril") ? [t("deps.env.sirilToo")] : []),
    ],
    badge: { tone: "warning", text: t("deps.badge.missing") },
    action: tool.installable ? "install" : null,
  };
}

/** The range of Python the environment accepts (`>=3.12,<3.15`). */
export function pythonRange(tools: readonly ToolRow[]): {
  first: string;
  last: string;
} {
  const text = tools.find((row) => row.id === "python-env")?.minVersion ?? "";
  const min = />=\s*(\d+)\.(\d+)/u.exec(text);
  const below = /<\s*(\d+)\.(\d+)/u.exec(text);
  if (!min || !below) return { first: "3.12", last: "3.14" };
  return {
    first: `${min[1]}.${min[2]}`,
    last: `${below[1]}.${Number(below[2]) - 1}`,
  };
}

function failureLines(
  t: Translate,
  locale: string,
  failure: DownloadFailure,
): string[] {
  const size = (bytes: number) => formatSize(t, bytes, locale);
  switch (failure.code) {
    case "interrupted":
    case "cancelled": {
      const total = failure.code === "interrupted" ? failure.total : null;
      return [
        total
          ? t("deps.line.interrupted", {
              done: size(failure.keptBytes),
              total: size(total),
            })
          : t("deps.line.interruptedPlain", { done: size(failure.keptBytes) }),
        t("deps.line.kept"),
      ];
    }
    case "digest_mismatch":
      return [t("deps.line.digest")];
    case "no_automatic_download":
      return [t("deps.line.noAutomatic")];
    case "extraction":
      return [t("deps.line.extraction")];
    default:
      return [t("deps.line.io")];
  }
}

export function dependencyRows(
  t: Translate,
  locale: string,
  tools: readonly ToolRow[],
  downloads: Readonly<Record<string, DownloadState>>,
): DependencyRow[] {
  const byId = new Map(tools.map((row) => [row.id, row]));
  const rows: DependencyRow[] = [];
  for (const id of ORDER) {
    const tool = byId.get(id);
    if (!tool) continue;
    const database = id === "astap" ? byId.get("astap-database") : undefined;
    const isPresent =
      id === "astap" ? present(tool) && present(database) : present(tool);
    const download = downloads[id];
    const shortName = tool.name[locale] ?? tool.name.en ?? tool.name.fr ?? id;
    const name = id === "astap" ? t("deps.name.astap") : shortName;
    const optional = !tool.required && tool.kind !== "pipeline";
    let lines: string[] = [];
    let badge: DependencyRow["badge"];
    let action: DependencyRow["action"] = null;
    let progress: DependencyRow["progress"] = null;

    if (download?.state === "installing") {
      lines = [envProgressLine(t, download.progress)];
      progress = envProgressBar(download.progress);
      badge = { tone: "neutral", text: t("deps.badge.installing") };
      action = "cancelInstall";
    } else if (download?.state === "envFailed") {
      const failure = envFailureLines(t, download.failure);
      lines = failure.lines;
      badge = { tone: "error", text: t("deps.badge.failed") };
      action = failure.resumable ? "resumeInstall" : null;
    } else if (id === "stacking-pipeline") {
      ({ lines, badge, action } = environmentState(
        t,
        tool,
        pythonRange(tools),
      ));
    } else if (download?.state === "downloading") {
      progress = { bytes: download.bytes, total: download.total };
      lines = [
        id === "tycho-2"
          ? t("sky.referenceStars.downloading")
          : download.total
            ? t("deps.line.progress", {
                done: formatSize(t, download.bytes, locale),
                total: formatSize(t, download.total, locale),
              })
            : t("deps.line.progressPlain", {
                done: formatSize(t, download.bytes, locale),
              }),
      ];
      badge = { tone: "neutral", text: t("deps.badge.downloading") };
      action = id === "tycho-2" ? null : "cancel";
    } else if (
      download?.state === "failed" &&
      download.failure.code !== "no_automatic_download"
    ) {
      // (« no automatic download » is not a failure: the row below says
      // « À installer » and keeps its page.)
      lines = failureLines(t, locale, download.failure);
      badge = { tone: "error", text: t("deps.badge.failed") };
      action =
        download.failure.code === "interrupted" ||
        download.failure.code === "cancelled"
          ? "resume"
          : download.failure.code === "extraction"
            ? "install"
            : null;
    } else if (isPresent) {
      if (id === "astap") {
        lines = [t("deps.line.database")];
      } else if (id === "tycho-2") {
        lines = [t("sky.referenceStars.credit")];
      } else if (tool.path) {
        lines = [
          tool.progressionCompatible
            ? t("deps.line.compatible", { path: shownPath(tool.path) })
            : t("deps.line.found", { path: shownPath(tool.path) }),
        ];
      }
      badge = {
        tone: "success",
        text: tool.version
          ? t("deps.badge.present", { version: tool.version })
          : t("deps.badge.presentPlain"),
      };
    } else if (tool.state === "outdated") {
      lines = tool.minVersion
        ? [t("deps.line.minimum", { minimum: tool.minVersion })]
        : [];
      badge = {
        tone: "warning",
        text: t("deps.badge.outdated", { version: tool.version ?? "" }),
      };
      action = "install";
    } else if (tool.state === "downloaded") {
      lines = tool.path ? [shownPath(tool.path)] : [];
      badge = { tone: "neutral", text: t("deps.badge.downloaded") };
    } else {
      // What « Installer » would fetch: the program and the star database
      // for ASTAP, each only when it is missing.
      const parts = (
        id === "astap"
          ? [
              present(tool) ? undefined : tool,
              present(database) ? undefined : database,
            ]
          : [tool]
      ).filter((part): part is ToolRow => part !== undefined);
      const automatic =
        id !== "tycho-2" &&
        parts.length > 0 &&
        parts.every((part) => part.download);
      const partialBytes = parts.reduce(
        (kept, part) => kept || (part.partialBytes ?? 0),
        0,
      );
      if (id === "astap") {
        const bytes = automatic
          ? parts.reduce((sum, part) => sum + (part.download?.bytes ?? 0), 0)
          : 0;
        lines = [
          !automatic
            ? t("deps.line.noAutomatic")
            : bytes
              ? t("deps.line.toDownload", {
                  size: formatSize(t, bytes, locale),
                })
              : t("deps.line.toDownloadPlain"),
        ];
      } else if (id === "tycho-2") {
        lines = [t("deps.line.tycho")];
      } else if (!automatic) {
        lines = [t("deps.line.noAutomatic")];
      }
      if (id !== "tycho-2" && !automatic) {
        // No file of the registry for this system: not a failure, the
        // download page is the way (« À installer »).
        badge = { tone: "neutral", text: t("deps.badge.toInstall") };
        action = null;
      } else {
        badge = optional
          ? { tone: "neutral", text: t("deps.badge.notInstalled") }
          : { tone: "warning", text: t("deps.badge.missing") };
        action = partialBytes ? "resume" : "install";
      }
    }
    rows.push({
      id,
      name,
      shortName,
      optional,
      role: t(ROLE[id]),
      lines,
      badge,
      progress,
      action,
      page:
        id === "stacking-pipeline" && !(tool.blockedBy ?? []).includes("python")
          ? null
          : tool.page,
    });
  }
  return rows;
}

/** The line at the head of the tab, and whether « Tout installer » is
 * there (the required tools that are missing). */
export function dependenciesHead(
  t: Translate,
  locale: string,
  rows: readonly DependencyRow[],
  tools: readonly ToolRow[],
  /** « Tout installer » also makes what the pipelines need (the Python
   * environment). */
  pipelinesToInstall: readonly string[] = [],
): { line: string; installAll: boolean; downloading: boolean } {
  const running = rows.find((row) => row.progress !== null);
  const required = rows.filter((row) => {
    const tool = tools.find((item) => item.id === row.id);
    return tool?.required && row.badge.tone !== "success";
  });
  const installAll = required.length > 0 || pipelinesToInstall.length > 0;
  if (running) {
    return {
      line:
        running.action === "cancelInstall"
          ? t("deps.installing", { name: running.shortName })
          : t("deps.downloading", { name: running.shortName }),
      installAll,
      downloading: true,
    };
  }
  return {
    line:
      required.length > 0
        ? pluralText(t, locale, "deps.missing", required.length)
        : pipelinesToInstall.length > 0
          ? t("deps.pipelinesMissing")
          : t("deps.allPresent"),
    installAll,
    downloading: false,
  };
}

/* ------------------------- « Pipelines de traitement » -------------------- */

export interface SectionLineView {
  id: string;
  name: string;
  reason: string;
  optional: boolean;
  /** Version found and minimum, licence, size of the download. */
  lines: string[];
  badge: { tone: BadgeTone; text: string };
  action: "install" | "resumeInstall" | null;
  page: string | null;
}

export interface TrustView {
  state: TrustState;
  badge: { tone: BadgeTone; text: string };
  line: string;
  /** `decide` opens the question; `revoke` takes the trust back. */
  action: "decide" | "revoke";
}

export interface PipelineSectionView {
  id: string;
  name: string;
  description: string;
  origin: string;
  trust: TrustView | null;
  dependencies: SectionLineView[];
  /** « Détails »: the pipeline's steps, what each does and why. */
  details: { title: string; text: string }[];
}

/** A digest as the window writes it (`abcd…wxyz`), for a line. */
export function shortDigest(sha256: string | null): string {
  return sha256 ? writtenDigest(sha256) : "";
}

export function trustView(
  t: Translate,
  state: TrustState,
  sha256: string | null,
): TrustView | null {
  const line = t("deps.trust.digest", { digest: shortDigest(sha256) });
  switch (state) {
    case "not_needed":
      return null;
    case "trusted":
      return {
        state,
        badge: { tone: "success", text: t("deps.trust.trusted") },
        line,
        action: "revoke",
      };
    case "refused":
      return {
        state,
        badge: { tone: "neutral", text: t("deps.trust.refused") },
        line,
        action: "decide",
      };
    case "changed":
      return {
        state,
        badge: { tone: "warning", text: t("deps.trust.changed") },
        line,
        action: "decide",
      };
    case "ask":
      return {
        state,
        badge: { tone: "warning", text: t("deps.trust.ask") },
        line,
        action: "decide",
      };
  }
}

function sectionLine(
  t: Translate,
  locale: string,
  dependency: SectionDependency,
  running: DownloadState | undefined,
  range: { first: string; last: string },
): SectionLineView {
  const lines: string[] = [];
  if (dependency.version && dependency.minVersion) {
    lines.push(
      t("deps.pipelines.versionMin", {
        version: dependency.version,
        minimum: dependency.minVersion,
      }),
    );
  } else if (dependency.version) {
    lines.push(t("deps.pipelines.version", { version: dependency.version }));
  } else if (dependency.minVersion && dependency.id !== "python-env") {
    lines.push(t("deps.line.minimum", { minimum: dependency.minVersion }));
  }
  const details = [
    t("deps.pipelines.licence", { licence: dependency.licence }),
  ];
  if (dependency.bytes && dependency.state !== "present") {
    details.push(
      t("deps.pipelines.size", {
        size: formatSize(t, dependency.bytes, locale),
      }),
    );
  }
  lines.push(details.join(" · "));
  let badge: SectionLineView["badge"];
  let action: SectionLineView["action"] = null;
  if (running?.state === "installing") {
    lines.unshift(envProgressLine(t, running.progress));
    badge = { tone: "neutral", text: t("deps.badge.installing") };
  } else if (running?.state === "envFailed") {
    const failure = envFailureLines(t, running.failure);
    lines.unshift(...failure.lines);
    badge = { tone: "error", text: t("deps.badge.failed") };
    action = failure.resumable ? "resumeInstall" : null;
  } else if (dependency.state === "present") {
    badge = {
      tone: "success",
      text: dependency.version
        ? t("deps.badge.present", { version: dependency.version })
        : t("deps.badge.presentPlain"),
    };
  } else if (dependency.state === "outdated") {
    badge = {
      tone: "warning",
      text: dependency.version
        ? t("deps.badge.outdated", { version: dependency.version })
        : t("deps.badge.toRedo"),
    };
    action = dependency.installable ? "install" : null;
  } else {
    if (
      dependency.id === "python-env" &&
      dependency.blockedBy.includes("python")
    ) {
      lines.unshift(t("deps.env.noPython", range));
    } else if (dependency.interrupted) {
      lines.unshift(t("deps.env.interrupted"));
    }
    badge = dependency.required
      ? { tone: "warning", text: t("deps.badge.missing") }
      : { tone: "neutral", text: t("deps.badge.notInstalled") };
    action = dependency.installable
      ? dependency.interrupted
        ? "resumeInstall"
        : "install"
      : null;
  }
  return {
    id: dependency.id,
    name: dependency.name,
    reason: dependency.reason,
    optional: !dependency.required,
    lines,
    badge,
    action,
    // The Python environment is made here: python.org only when no Python
    // in the range was found.
    page:
      dependency.id === "python-env" && !dependency.blockedBy.includes("python")
        ? null
        : dependency.page,
  };
}

/** The blocks of the section, in the order of the list (« Rendu naturel »
 * first), with the trust overridden by the answers given on this screen. */
export function pipelineSectionViews(
  t: Translate,
  locale: string,
  answer: PipelineSections,
  tools: readonly ToolRow[],
  downloads: Readonly<Record<string, DownloadState>>,
  trust: Readonly<Record<string, TrustState>> = {},
): PipelineSectionView[] {
  const range = pythonRange(tools);
  return answer.sections.map((section: PipelineSection) => ({
    id: section.id,
    name: section.name,
    description: section.description,
    origin:
      section.source === "user"
        ? t("deps.pipelines.added", {
            author: section.author,
            licence: section.licence,
          })
        : t("deps.pipelines.shipped"),
    trust: trustView(
      t,
      trust[section.id] ?? section.trust,
      section.scriptSha256,
    ),
    dependencies: section.dependencies.map((dependency) =>
      sectionLine(
        t,
        locale,
        dependency,
        dependency.id === "python-env"
          ? (downloads["python-env"] ?? downloads["stacking-pipeline"])
          : downloads[dependency.id],
        range,
      ),
    ),
    details: section.details ?? [],
  }));
}

/** « Tout installer » of the section: the required dependencies, missing,
 * that Astro Sync can install here, each once. */
export function sectionsToInstall(answer: PipelineSections | null): string[] {
  const ids: string[] = [];
  for (const dependency of (answer?.sections ?? []).flatMap(
    (section) => section.dependencies,
  )) {
    if (
      dependency.required &&
      dependency.installable &&
      (dependency.state === "missing" || dependency.state === "outdated") &&
      !ids.includes(dependency.id)
    ) {
      ids.push(dependency.id);
    }
  }
  return ids;
}
