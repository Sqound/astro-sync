// « Liste des incidents » at the result of « Détecter les sessions »: one list per
// session the import created. Each session
// is read from the folders its frames came from -- not from a night's working
// folder -- so this file works out what the incidents engine needs for it
// (the folder to read, the target's folder inside it, the evening the files
// were written on, the clock shift the import applied) and nothing else. Pure
// TypeScript, tested under `node --test`.

import type {
  CreatedSession,
  DetectedIncident,
  DetectedTarget,
  Proposal,
} from "./bridge";
import type { IncidentsNight } from "../hooks/useIncidents.ts";

const normal = (path: string): string => path.replace(/\\/g, "/");

/** The directory part of a path (no trailing slash; `""` for a bare name). */
export function dirOf(path: string): string {
  const parts = normal(path).split("/");
  parts.pop();
  return parts.join("/");
}

/** The deepest directory that holds all the paths (`""` when they share
 * none). Works on absolute and on relative paths alike. */
export function commonDir(paths: readonly string[]): string {
  if (paths.length === 0) return "";
  const split = paths.map((path) => dirOf(path).split("/"));
  const first = split[0];
  let length = first.length;
  for (const other of split.slice(1)) {
    let same = 0;
    while (same < length && same < other.length && first[same] === other[same])
      same += 1;
    length = same;
  }
  return first.slice(0, length).join("/");
}

/** A folder that is a filesystem root (`/`, `C:`) or nothing: never read as a
 * night, it would be the whole disk. */
export function isRootFolder(folder: string): boolean {
  const cleaned = normal(folder).replace(/\/+$/, "");
  return cleaned === "" || /^[A-Za-z]:$/.test(cleaned);
}

/** `dir` seen from `base`: `""` when they are the same, `null` when `dir` is
 * not inside `base`. */
export function relativeTo(base: string, dir: string): string | null {
  const from = normal(base).replace(/\/+$/, "");
  const to = normal(dir).replace(/\/+$/, "");
  if (to === from) return "";
  return to.startsWith(`${from}/`) ? to.slice(from.length + 1) : null;
}

/** How many folders above the frames the logs may be looked for (an ASIAIR
 * `Autorun` folder holds `Light/<target>` and `Log` side by side). */
export const LOG_WIDEN_LEVELS = 2;

/** The folder the incidents engine reads for a session: the one the frames
 * are in, widened by at most {@link LOG_WIDEN_LEVELS} folders until the logs
 * attached to the session are inside it too. `null` when that would be a
 * filesystem root. */
export function nightFolderOf(
  frames: readonly string[],
  logs: readonly string[],
): string | null {
  let folder = commonDir(frames);
  if (isRootFolder(folder)) return null;
  for (let level = 0; level < LOG_WIDEN_LEVELS; level += 1) {
    if (logs.every((log) => relativeTo(folder, dirOf(log)) !== null)) break;
    const up = dirOf(folder);
    if (isRootFolder(up)) break;
    folder = up;
  }
  return folder;
}

/** The session the person sees in the selector and the engine reads. */
export interface ImportedSession {
  /** `created`: the import made the session; `updated`: it only added poses
   * (and maybe targets) to a session of the space. */
  action: CreatedSession["action"];
  sessionId: string;
  /** « M45 », « IC 1805, Sh2-103 ». */
  designations: string;
  /** `AAAA-MM-JJ`, the evening of the session in the service. */
  eveningDate: string;
  place: string | null;
  night: IncidentsNight;
  /** The target the lines are said for, when the session has one only. */
  targetName: string | null;
}

/** The sessions the import made or added poses to, with what the engine needs
 * to read each (the sessions it only added frames to have their list too, the
 * poses added being what is read). A session whose frames cannot be placed in
 * a folder (none, or only the root of a disk) is left out: there is nothing
 * to read.
 *
 * `finished`: the sessions that already existed and are finished (a rework
 * can replace a signalement declared on them); a session the import made is
 * a night already carried out. */
export function importedSessions(
  proposal: Proposal,
  created: readonly CreatedSession[],
  finished: ReadonlySet<string> = new Set(),
): ImportedSession[] {
  const detected = new Map(proposal.sessions.map((s) => [s.id, s]));
  const places = new Map(proposal.places.map((p) => [p.id, p]));
  const logs = new Map(proposal.logs.map((l) => [l.id, l]));
  const result: ImportedSession[] = [];
  for (const made of created) {
    const found = detected.get(made.detectedId);
    if (!found) continue;
    // A session that already existed receives the poses of the targets the
    // import attached (the ones in it, the ones added): the other targets of
    // the night were not taken, their poses are not read.
    const fresh = made.action === "created";
    const targets = fresh
      ? found.targets
      : found.targets.filter((target) =>
          made.designations.includes(target.designation),
        );
    const frames = targets.flatMap((target) => target.files);
    const logPaths = found.logs
      .map((attached) => logs.get(attached.logId)?.path)
      .filter((path): path is string => typeof path === "string");
    const folder = nightFolderOf(frames, logPaths);
    if (folder === null || frames.length === 0) continue;
    const place = places.get(found.place);
    const chosen = place?.candidates.find((c) => c.placeId === place.chosen);
    result.push({
      action: made.action,
      sessionId: made.sessionId,
      designations: made.designations.join(", "),
      eveningDate: made.eveningDate,
      place: chosen?.name ?? null,
      targetName: targets.length === 1 ? targets[0].designation : null,
      night: {
        sessionId: made.sessionId,
        // A session the import made is a night already carried out; one it
        // added poses to is what the space says it is.
        finished: fresh || finished.has(made.sessionId),
        nightFolder: folder,
        targets: targetFolders(targets, folder),
        // The files carry the clock they were written with: the window of
        // the night is the evening those dates give; the shift the import
        // applied moves what is found onto the session's own clock.
        eveningDate: found.writtenEveningDate || made.eveningDate,
        longitude: place?.longitude ?? null,
        startedAt: found.start,
        // The shift in force is the one the person left ticked (what the
        // service was told), not the one the proposal started with.
        shiftSeconds: made.shiftHours * 3600,
        // Nothing was declared on a session the import made: the card says
        // so. Neither kind has its poses measured here.
        imported: fresh,
        noMeasure: true,
      },
    });
  }
  return result;
}

/** The target folders of a session, inside the folder read; two targets whose
 * frames share a folder are read once (the engine cannot tell them apart). */
function targetFolders(
  targets: readonly DetectedTarget[],
  folder: string,
): IncidentsNight["targets"] {
  const seen = new Set<string>();
  const result: IncidentsNight["targets"] = [];
  for (const target of targets) {
    const relative = relativeTo(folder, commonDir(target.files));
    if (relative === null || seen.has(relative)) continue;
    seen.add(relative);
    result.push({
      targetId: target.targetId ?? target.key,
      folderName: relative,
    });
  }
  return result;
}

const ISO_KEYS = ["firstAt", "lastAt"] as const;

function shiftedIso(value: string, seconds: number): string {
  const unix = Date.parse(value);
  if (!Number.isFinite(unix)) return value;
  return new Date(unix + seconds * 1000)
    .toISOString()
    .replace(/\.\d{3}Z$/, "Z");
}

function shiftedMap<T extends Record<string, unknown>>(
  map: T,
  seconds: number,
): T {
  const next: Record<string, unknown> = { ...map };
  for (const key of ISO_KEYS) {
    if (typeof next[key] === "string")
      next[key] = shiftedIso(next[key] as string, seconds);
  }
  return next as T;
}

/** A detection moved onto the session's clock: the import corrected the
 * written dates by `seconds`, so what is found in the files (their instants,
 * the first and last instants a payload names) moves by the same. The key
 * (`id`) is an identity, not a time: it stays. */
export function shiftIncident(
  incident: DetectedIncident,
  seconds: number,
): DetectedIncident {
  if (!seconds) return incident;
  return {
    ...incident,
    start: shiftedIso(incident.start, seconds),
    end: incident.end ? shiftedIso(incident.end, seconds) : null,
    params: shiftedMap(incident.params, seconds),
    payload: shiftedMap(incident.payload, seconds),
    evidence: {
      ...incident.evidence,
      params: shiftedMap(incident.evidence.params, seconds),
    },
  };
}

/** The text of a session's segment in the selector: « M45 · 10 oct. · 3 »
 * (the count when the list has been read). */
export function sessionSegment(
  locale: string,
  session: Pick<ImportedSession, "designations" | "eveningDate">,
  count: number | null,
  join: string,
): string {
  const day = new Intl.DateTimeFormat(locale, {
    day: "numeric",
    month: "short",
    timeZone: "UTC",
  }).format(new Date(`${session.eveningDate}T12:00:00Z`));
  return [session.designations, day, count === null ? null : String(count)]
    .filter((part): part is string => part !== null)
    .join(join);
}
