import type { DetectRequest, FoundSoFar, LastFolders } from "./bridge";

/** The three locations of « Détecter les sessions »: the frames are required,
 * the calibrations and the logs are not. */
export type DetectSlot = "fits" | "calibration" | "logs";

export const DETECT_SLOTS: DetectSlot[] = ["fits", "calibration", "logs"];

/** What the window keeps: a folder or nothing per location, and the ticked
 * « Lire aussi les sous-dossiers » (ticked from the start, for the three). */
export interface DetectWindowState {
  fits: string | null;
  calibration: string | null;
  logs: string | null;
  subfolders: boolean;
}

export const EMPTY_DETECT_WINDOW: DetectWindowState = {
  fits: null,
  calibration: null,
  logs: null,
  subfolders: true,
};

/** « Lancer la détection » needs the frames' folder, whatever the other two
 * hold (a night is born of frames). */
export function windowReady(state: DetectWindowState): boolean {
  return Boolean(state.fits);
}

/** The location's folder chosen, replaced or removed. */
export function withFolder(
  state: DetectWindowState,
  slot: DetectSlot,
  folder: string | null,
): DetectWindowState {
  return { ...state, [slot]: folder };
}

/** The request of the engine: one folder at most per location. */
export function buildRequest(state: DetectWindowState): DetectRequest {
  return {
    fits: state.fits ? [state.fits] : [],
    calibration: state.calibration ? [state.calibration] : [],
    logs: state.logs ? [state.logs] : [],
    subfolders: state.subfolders,
  };
}

/** The bar of the scan, 0 to 100; empty while the total is not known. */
export function scanPercent(read: number, total: number): number {
  if (!(total > 0) || !(read > 0)) return 0;
  return Math.min(100, Math.round((read / total) * 100));
}

/** The error codes of the engine that the window explains itself. */
const ENGINE_ERRORS = ["no_fits_folder", "folder_missing", "scan_failed"];

export function isEngineError(code: string): boolean {
  return ENGINE_ERRORS.includes(code);
}

/** « Arrêter la détection » answers `cancelled` from the engine: back to the
 * window, without a message. */
export function isCancelled(code: string): boolean {
  return code === "cancelled";
}

/** The window opens on the last folders chosen, else empty. */
export function windowFromLast(
  last: LastFolders | undefined,
): DetectWindowState {
  return {
    ...EMPTY_DETECT_WINDOW,
    fits: last?.fits ?? null,
    calibration: last?.calibration ?? null,
    logs: last?.logs ?? null,
  };
}

/** The three folders of « Lancer la détection », kept for next time; the
 * folders of the sending are left as they were. */
export function lastFoldersOf(
  state: DetectWindowState,
  previous: LastFolders | undefined,
): LastFolders {
  return {
    ...previous,
    fits: state.fits,
    calibration: state.calibration,
    logs: state.logs,
  };
}

type Translate = (
  key: string,
  values?: Record<string, string | number>,
) => string;
type Plural = (key: string, count: number) => string;

const CALIBRATION_WORDS: Record<
  FoundSoFar["calibrationKinds"][number],
  string
> = {
  dark: "detect.scan.kind.dark",
  flat: "detect.scan.kind.flat",
  "dark-flat": "detect.scan.kind.darkFlat",
  bias: "detect.scan.kind.bias",
};
const LOG_WORDS: Record<FoundSoFar["logKinds"][number], string> = {
  nina: "N.I.N.A.",
  asiair: "ASIAIR",
  phd2: "PHD2",
};

/** « Trouvé jusqu'ici »: the three lines of the scan, or `null`
 * before the first count. `plural` writes `key.one` / `key.other`. */
export function foundLines(
  t: Translate,
  plural: Plural,
  found: FoundSoFar | null,
): { found: string; files: string | null; solving: string | null } | null {
  if (!found) return null;
  const parts: string[] = [];
  if (found.calibrationFiles > 0)
    parts.push(
      t("detect.scan.calibration", {
        files: plural("detect.scan.files", found.calibrationFiles),
        sets: plural("detect.scan.sets", found.calibrationSets),
        kinds: found.calibrationKinds
          .map((kind) => t(CALIBRATION_WORDS[kind]))
          .join(", "),
      }),
    );
  if (found.logFiles > 0)
    parts.push(
      t("detect.scan.logs", {
        files: plural("detect.scan.files", found.logFiles),
        kinds: found.logKinds.map((kind) => LOG_WORDS[kind]).join(", "),
      }),
    );
  return {
    found: t("detect.scan.found", {
      nights: plural("detect.nights", found.nights),
      targets: plural("detect.scan.targets", found.targets),
      places: plural("detect.scan.places", found.places),
      instruments: plural("detect.scan.instruments", found.instruments),
    }),
    files: parts.length ? parts.join(t("detect.join")) : null,
    solving:
      found.toSolve > 0 ? plural("detect.scan.solving", found.toSolve) : null,
  };
}
