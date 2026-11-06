// The sending of a session, steps « Dossier » and « Fichiers »: where the person says the frames and the logs
// are, the engine's detection bounded to the session, and what the sheet
// shows and does with it. Pure; the screens only draw it (tested in
// `wizardDetect.test.ts`).

import type {
  CatalogueTarget,
  DetectRequest,
  DetectedLogFile,
  DetectedTarget,
  FrameKind,
  NightOnline,
  OrganiseMode,
  OrganisePlan,
  Proposal,
  ScopeTarget,
  SessionScope,
  SessionSummary,
} from "./bridge";
import type { ReportStorage } from "./lastReport";

/* ------------------------------ The locations ----------------------------- */

/** Where the person says the frames and the logs of the session are. Empty
 * is today's behaviour: the night's folder is read. One tick for both, as
 * the request of the engine has one. */
export interface SendLocations {
  fits: string | null;
  logs: string | null;
  subfolders: boolean;
}

export const NO_LOCATIONS: SendLocations = {
  fits: null,
  logs: null,
  subfolders: true,
};

export function hasLocation(locations: SendLocations): boolean {
  return Boolean(locations.fits || locations.logs);
}

const locationsKey = (sessionId: string) =>
  `astro-sync.sendLocations:${sessionId}`;

/** The locations kept for a session on this computer. */
export function loadLocations(
  storage: ReportStorage | null,
  sessionId: string,
): SendLocations {
  try {
    const text = storage?.getItem(locationsKey(sessionId));
    if (!text) return NO_LOCATIONS;
    const value = JSON.parse(text) as Partial<SendLocations>;
    const folder = (item: unknown) =>
      typeof item === "string" && item !== "" ? item : null;
    return {
      fits: folder(value.fits),
      logs: folder(value.logs),
      subfolders: value.subfolders !== false,
    };
  } catch {
    return NO_LOCATIONS;
  }
}

export function saveLocations(
  storage: ReportStorage | null,
  sessionId: string,
  locations: SendLocations,
): void {
  try {
    storage?.setItem(locationsKey(sessionId), JSON.stringify(locations));
  } catch {
    // A storage that refuses: the choice lasts until the window closes.
  }
}

/* ------------------------------ The detection ----------------------------- */

const TWELVE_HOURS = 12 * 3600 * 1000;

/** The coordinates and the aliases the catalogue knows, by target id. */
export type CatalogueExtras = Record<
  string,
  Pick<CatalogueTarget, "aliases" | "raHours" | "decDegrees">
>;

/** The session's range and its targets, for the engine. A session without a
 * finish is taken for twelve hours. */
export function buildScope(
  session: Pick<SessionSummary, "startedAt" | "finishedAt">,
  online: NightOnline | null,
  extras: CatalogueExtras = {},
): SessionScope {
  const start = new Date(session.startedAt);
  const end = session.finishedAt
    ? new Date(session.finishedAt)
    : new Date(start.getTime() + TWELVE_HOURS);
  const targets: ScopeTarget[] = (online?.targets ?? [])
    .filter((target) => target.targetId)
    .map((target) => {
      const id = target.targetId as string;
      const known = extras[id];
      return {
        id,
        designation: target.name ?? id,
        aliases: known?.aliases ?? [],
        raHours: known?.raHours ?? null,
        decDegrees: known?.decDegrees ?? null,
      };
    });
  return { from: start.toISOString(), to: end.toISOString(), targets };
}

/** The request of the sheet: the frames and the logs, no calibration folder
 * (the night's calibration is read among the frames), bounded to the session. */
export function buildRequest(
  locations: SendLocations,
  scope: SessionScope,
): DetectRequest {
  return {
    fits: locations.fits ? [locations.fits] : [],
    calibration: [],
    logs: locations.logs ? [locations.logs] : [],
    subfolders: locations.subfolders,
    scope,
  };
}

/** The sheet opens by itself when a location is given and the detection has
 * not been answered yet. */
export function opensByItself(
  locations: SendLocations,
  answered: boolean,
): boolean {
  return hasLocation(locations) && !answered;
}

/** A key that changes when the session's range or its targets do (the
 * sheet reads again then, not before). */
export function scopeKey(scope: SessionScope): string {
  return [
    scope.from,
    scope.to,
    ...scope.targets.map((target) => target.id),
  ].join("|");
}

/** What the person answered with « Suivant »: the mode, and the proposal the
 * step reads the frames from (in place) or the tidying that was done. */
export interface DetectAnswer {
  mode: OrganiseMode;
  proposal: Proposal;
  /** The files were copied, moved or linked into the night's folder. */
  organised: boolean;
  /** `scopeKey` of the detection the proposal answers. */
  scopeKey: string;
}

/* ------------------------------- The sheet ------------------------------- */

export interface SheetFile {
  /** Relative to the folder read when it is under it. */
  name: string;
  path: string;
  exposureSeconds: number | null;
  filter: string | null;
  /** The file's start, ISO UTC (the « UTC » column). */
  utc: string | null;
}

export interface SheetTarget {
  id: string;
  designation: string;
  frames: number;
  /** The exposures of the frames, in seconds (usually one). */
  exposures: number[];
  totalSeconds: number;
  files: SheetFile[];
}

export interface SheetGroup {
  key: string;
  designation: string;
  targetId: string | null;
  frames: number;
  /** « 16:10 → 16:46 UTC », `null` when the engine gave no time. */
  span: { start: string; end: string } | null;
  exposures: number[];
  files: SheetFile[];
}

export interface SheetCalibration {
  kind: FrameKind;
  frames: number;
}

export interface SheetLogs {
  files: { name: string; path: string; kind: string }[];
  /** `asiair`, `phd2`, `nina`, in this order. */
  readers: string[];
}

export interface SheetModel {
  targets: SheetTarget[];
  calibration: SheetCalibration[];
  logs: SheetLogs;
  unassigned: SheetGroup[];
  withoutObject: SheetGroup | null;
  otherNights: number;
  /** The frames the sheet keeps (those of the session's targets). */
  frames: number;
}

const KIND_ORDER: FrameKind[] = ["dark", "flat", "dark-flat", "bias"];
const READER_ORDER = ["asiair", "phd2", "nina"];

/** The engine's detected targets that hold a file. */
function detectedByFile(proposal: Proposal) {
  const index = new Map<
    string,
    {
      target: DetectedTarget;
      filter: string | null;
      exposure: number | null;
      utc: string | null;
    }
  >();
  for (const session of proposal.sessions) {
    for (const target of session.targets) {
      const single =
        target.acquisition.length === 1
          ? target.acquisition[0].exposureSeconds
          : null;
      target.files.forEach((file, position) => {
        const filter = target.fileFilters?.[position];
        index.set(file, {
          target,
          filter: filter ? filter : null,
          exposure: single ?? (target.exposureSeconds || null),
          utc: target.fileTimes?.[position] || null,
        });
      });
    }
  }
  return index;
}

/** A path under the folder read, without it (« IC 1805/Light_….fit »). */
export function relativeTo(path: string, folder: string | null): string {
  const clean = path.replace(/\\/g, "/");
  if (folder) {
    const root = folder.replace(/\\/g, "/").replace(/\/$/, "");
    if (clean.startsWith(`${root}/`)) return clean.slice(root.length + 1);
  }
  return clean.split("/").pop() ?? clean;
}

function sheetFiles(
  paths: string[],
  folder: string | null,
  index: ReturnType<typeof detectedByFile>,
): SheetFile[] {
  return paths.map((path) => {
    const known = index.get(path);
    return {
      name: relativeTo(path, folder),
      path,
      exposureSeconds: known?.exposure ?? null,
      filter: known?.filter ?? null,
      utc: known?.utc ?? null,
    };
  });
}

function exposuresOf(files: SheetFile[]): number[] {
  return [
    ...new Set(
      files
        .map((file) => file.exposureSeconds)
        .filter((value): value is number => value !== null),
    ),
  ].sort((a, b) => a - b);
}

function spanOf(
  paths: string[],
  index: ReturnType<typeof detectedByFile>,
): SheetGroup["span"] {
  const found = new Set<DetectedTarget>();
  for (const path of paths) {
    const known = index.get(path);
    if (known) found.add(known.target);
  }
  if (found.size === 0) return null;
  const starts = [...found].map((target) => target.start).sort();
  const ends = [...found].map((target) => target.end).sort();
  return { start: starts[0], end: ends[ends.length - 1] };
}

/** What the sheet shows, from the engine's scoped detection. `folder` is the
 * folder of the frames (names are shown under it). */
export function sheetModel(
  proposal: Proposal,
  folder: string | null,
): SheetModel | null {
  const scoped = proposal.scoped;
  if (!scoped) return null;
  const index = detectedByFile(proposal);
  const targets: SheetTarget[] = scoped.targets.map((target) => {
    const files = sheetFiles(target.files, folder, index);
    // The seconds acquired by the frames the target keeps: the exposure of
    // each, as the engine read it.
    const totalSeconds = files.reduce(
      (sum, file) => sum + (file.exposureSeconds ?? 0),
      0,
    );
    return {
      id: target.id,
      designation: target.designation,
      frames: files.length,
      exposures: exposuresOf(files),
      totalSeconds,
      files,
    };
  });
  const group = (
    key: string,
    designation: string,
    targetId: string | null,
    paths: string[],
  ): SheetGroup => {
    const files = sheetFiles(paths, folder, index);
    return {
      key,
      designation,
      targetId,
      frames: files.length,
      span: spanOf(paths, index),
      exposures: exposuresOf(files),
      files,
    };
  };
  return {
    targets,
    calibration: calibrationOf(proposal),
    logs: logsOf(proposal),
    unassigned: scoped.unassigned.map((item) =>
      group(item.key, item.designation, item.targetId, item.files),
    ),
    withoutObject:
      scoped.withoutObject.length > 0
        ? group("without-object", "", null, scoped.withoutObject)
        : null,
    otherNights: scoped.otherNights,
    frames: targets.reduce((sum, target) => sum + target.frames, 0),
  };
}

/** The calibration of the night: the sets the sessions of the range took,
 * all the sets read when none was attached. */
export function calibrationOf(proposal: Proposal): SheetCalibration[] {
  const attached = new Set(
    proposal.sessions.flatMap((session) =>
      session.calibration.map((item) => item.setId),
    ),
  );
  const sets = proposal.calibrationSets.filter(
    (set) => attached.size === 0 || attached.has(set.id),
  );
  const out: SheetCalibration[] = [];
  for (const kind of KIND_ORDER) {
    const frames = sets
      .filter((set) => set.kind === kind)
      .reduce((sum, set) => sum + set.frames, 0);
    if (frames > 0) out.push({ kind, frames });
  }
  return out;
}

/** The files of the calibration the sheet counts. */
export function calibrationFiles(proposal: Proposal): string[] {
  const attached = new Set(
    proposal.sessions.flatMap((session) =>
      session.calibration.map((item) => item.setId),
    ),
  );
  return proposal.calibrationSets
    .filter((set) => attached.size === 0 || attached.has(set.id))
    .flatMap((set) => set.files);
}

/** The logs tied to the session (those the engine attached to a night). */
export function logsOf(proposal: Proposal): SheetLogs {
  const kept: DetectedLogFile[] = proposal.logs.filter(
    (log) => log.sessions.length > 0 || log.matchedFrames > 0,
  );
  const readers = READER_ORDER.filter((kind) =>
    kept.some((log) => log.kind === kind),
  );
  return {
    files: kept.map((log) => ({
      name: relativeTo(log.path, null),
      path: log.path,
      kind: log.kind,
    })),
    readers,
  };
}

/** The readers' names, as the tools write them. */
export const READER_NAME: Record<string, string> = {
  asiair: "ASIAIR",
  phd2: "PHD2",
  nina: "N.I.N.A.",
};

/** « 11:45 » in UTC. */
export function utcTime(iso: string, locale: string): string {
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return iso;
  return new Intl.DateTimeFormat(locale, {
    hour: "2-digit",
    minute: "2-digit",
    hourCycle: "h23",
    timeZone: "UTC",
  }).format(date);
}

/** « 10 août » (the day of the session's start, in UTC). */
export function utcDay(iso: string, locale: string): string {
  return new Intl.DateTimeFormat(locale, {
    day: "numeric",
    month: "long",
    timeZone: "UTC",
  }).format(new Date(iso));
}

/* ---------------------------- Where the files go --------------------------- */

/** The proposal as the sheet tidies it: the frames of the session's targets
 * only (not the unassigned ones, nor those without OBJECT), with the night's
 * calibration and its logs. */
export function organiseProposal(proposal: Proposal): Proposal {
  const scoped = proposal.scoped;
  if (!scoped) return proposal;
  const kept = new Set(scoped.targets.flatMap((target) => target.files));
  const sessions = proposal.sessions
    .map((session) => ({
      ...session,
      targets: session.targets
        .map((target) => {
          const positions = target.files
            .map((file, position) => (kept.has(file) ? position : -1))
            .filter((position) => position >= 0);
          return {
            ...target,
            files: positions.map((position) => target.files[position]),
            fileFilters: target.fileFilters
              ? positions.map(
                  (position) => target.fileFilters?.[position] ?? "",
                )
              : target.fileFilters,
            fileTimes: target.fileTimes
              ? positions.map((position) => target.fileTimes?.[position] ?? "")
              : target.fileTimes,
            frames: positions.length,
          };
        })
        .filter((target) => target.files.length > 0),
    }))
    .filter((session) => session.targets.length > 0);
  return { ...proposal, sessions };
}

function normal(path: string): string {
  return path.replace(/\\/g, "/").replace(/\/+$/, "");
}

/** The engine's plan puts the files under `<destination>/<night>/…`; the
 * session's tidying goes into the night's folder itself, so the first level
 * is dropped. The plan keeps its sources and its order. */
export function intoNightFolder(
  plan: OrganisePlan,
  nightFolder: string,
): OrganisePlan {
  const root = normal(plan.destination);
  const target = normal(nightFolder);
  const rewrite = (destination: string): string => {
    const clean = normal(destination);
    if (!clean.startsWith(`${root}/`)) return destination;
    const rest = clean.slice(root.length + 1).split("/");
    return [target, ...rest.slice(1)].join("/");
  };
  return {
    ...plan,
    destination: nightFolder,
    actions: plan.actions.map((action) => ({
      ...action,
      destination: rewrite(action.destination),
    })),
    conflicts: plan.conflicts.map((conflict) => ({
      ...conflict,
      destination: rewrite(conflict.destination),
    })),
  };
}

/** « IC 1805/lights/, Sh2-103/lights/, calibration/, logs/ »: the folders
 * the plan makes in the night's folder, from its actions (the engine's plan,
 * before `intoNightFolder`). */
export function treeFolders(plan: OrganisePlan): string[] {
  const root = normal(plan.destination);
  const folders: string[] = [];
  const calibration = new Set<string>();
  let logs = false;
  for (const action of plan.actions) {
    if (action.kind === "already-there") continue;
    const clean = normal(action.destination);
    if (!clean.startsWith(`${root}/`)) continue;
    const parts = clean.slice(root.length + 1).split("/");
    // <night>/<target>/lights/[<filter>/]<file>, <night>/calibration/..., <night>/logs/...
    if (parts[1] === "logs") logs = true;
    else if (parts[1] === "calibration") calibration.add("calibration/");
    else if (parts[2] === "lights") {
      const filtered =
        parts.length >= 5
          ? `${parts[1]}/lights/${parts[3]}/`
          : `${parts[1]}/lights/`;
      if (!folders.includes(filtered)) folders.push(filtered);
    }
  }
  return [...folders, ...calibration, ...(logs ? ["logs/"] : [])];
}

/** The sentence under the modes and its tone (the move warns). */
export function modeSentence(mode: OrganiseMode): {
  key: string;
  tone: "success" | "warning";
} {
  switch (mode) {
    case "copy":
      return { key: "wizard.detect.mode.copy", tone: "success" };
    case "move":
      return { key: "wizard.detect.mode.move", tone: "warning" };
    case "symlink":
      return { key: "wizard.detect.mode.link", tone: "success" };
    default:
      return { key: "wizard.detect.mode.leave", tone: "success" };
  }
}

/* ------------------------- The step after the sheet ------------------------ */

/** The folder that holds a target's frames when read in place: the deepest
 * folder its files share. */
export function framesFolder(files: string[]): string {
  const lists = files.map((file) => normal(file).split("/").slice(0, -1));
  if (lists.length === 0) return "";
  const common = lists.reduce((shared, parts) => {
    let n = 0;
    while (n < shared.length && n < parts.length && shared[n] === parts[n]) n++;
    return shared.slice(0, n);
  });
  return common.join("/");
}

export interface InPlaceTarget {
  /** The folder of the frames, the target's own when the files share it. */
  folder: string;
  frames: number;
  exposures: number[];
  gains: number[];
}

/** What « Fichiers » says of a target's frames read in place, by target id. */
export function inPlaceTargets(
  proposal: Proposal,
): Record<string, InPlaceTarget> {
  const scoped = proposal.scoped;
  if (!scoped) return {};
  const index = detectedByFile(proposal);
  const out: Record<string, InPlaceTarget> = {};
  for (const target of scoped.targets) {
    if (target.files.length === 0) continue;
    const files = sheetFiles(target.files, null, index);
    const gains = new Set<number>();
    for (const file of target.files) {
      const known = index.get(file);
      for (const row of known?.target.acquisition ?? []) {
        if (row.gain !== null) gains.add(row.gain);
      }
    }
    out[target.id] = {
      folder: framesFolder(target.files),
      frames: files.length,
      exposures: exposuresOf(files),
      gains: [...gains].sort((a, b) => a - b),
    };
  }
  return out;
}
