// What Astro Sync says when something is missing or not read: the darks and flats a
// night lacks, the videos and RAW files
// that are not read, the folder where nothing is readable, why a pose has no
// target, and the family width of the poses. Pure TypeScript (no React, no
// Tauri), so it runs under `node --test`.
//
// Two levels, and only those:
//
// - a **warning** when the result will be worse and the person can act:
//   calibration of the kind exists and does not fit the poses (darks of
//   another exposure, no flat for the poses' filter), a folder where nothing
//   is readable, a plate solving that is corrected in « Réglages ›
//   Dépendances »;
// - an **information** when the line only says what the tool does not read or
//   does not know: no dark or flat at all (sometimes a choice), videos (not
//   processed), RAW files (not read), a solver's limit, the width of a family.
//
// No button of a new level: every action is a link.

import type {
  CalibrationNote,
  DetectedTarget,
  Finding,
  Proposal,
} from "./bridge";
import type { Translate } from "../i18n/locale.ts";
import { conjunction, pluralText } from "./written.ts";

export type NoteLevel = "warning" | "information";

// ------------------------------------------------- the darks and the flats

/** `32 s`, `0,5 s`: a duration of the poses, in the language's own way. */
export function secondsText(
  t: Translate,
  locale: string,
  value: number,
): string {
  // A no-break space: the unit never wraps away from its number.
  return t("common.secondsShort", {
    count: new Intl.NumberFormat(locale, { maximumFractionDigits: 3 }).format(
      value,
    ),
  }).replace(/ /g, "\u00a0");
}

export function calibrationNoteLevel(note: CalibrationNote): NoteLevel {
  return note.code === "noDark" || note.code === "noFlat"
    ? "warning"
    : "information";
}

/** The note in the words of the window. */
export function calibrationNoteText(
  t: Translate,
  locale: string,
  note: CalibrationNote,
): string {
  switch (note.code) {
    case "noDark":
      return pluralText(t, locale, "calibration.noDark", note.count, {
        exposure: secondsText(t, locale, note.exposureSeconds),
        found: conjunction(
          locale,
          note.foundSeconds.map((value) => secondsText(t, locale, value)),
        ),
      });
    case "noDarkAtAll":
      return t("calibration.noDarkAtAll", {
        exposure: secondsText(t, locale, note.exposureSeconds),
      });
    case "noFlat":
      return note.filter
        ? t("calibration.noFlat", { filter: note.filter })
        : t("calibration.noFlatUnfiltered");
    case "noFlatAtAll":
      return note.filter
        ? t("calibration.noFlatAtAll", { filter: note.filter })
        : t("calibration.noFlatAtAllUnfiltered");
  }
}

/** The darks a night's warning leads to (« Voir les 10 darks »): none for
 * an information, which has nothing to show. */
export function darksToSee(notes: readonly CalibrationNote[]): number {
  return notes.reduce(
    (count, note) =>
      note.code === "noDark" ? Math.max(count, note.count) : count,
    0,
  );
}

// ---------------------------------- what the files of the night's finding say

/** How a finding of the night folder reads in `Fichiers`, « Traitements »
 * and the summary: the level, the words, and whether the stacking really goes
 * without it (the words of the summary say so only then). */
export interface CalibrationFinding {
  kind: "dark" | "flat";
  level: NoteLevel;
  /** The line of « Fichiers » (the Calibration card). */
  files: string;
  /** The line of « Traitements ». */
  treatment: string;
  /** « Empilée sans dark » (the summary's), `null` when the stacking takes
   * what there is. */
  without: { kind: "dark" | "flat"; filter: string | null } | null;
  /** The folder the link opens (`darks`, `flats`), `null` for an
   * information (nothing to open). */
  folder: "darks" | "flats" | null;
}

function numberOf(value: unknown): number | null {
  return typeof value === "number" ? value : null;
}

/** `30 s · gain 100`: an exposure with its gain. */
function withGain(
  t: Translate,
  locale: string,
  exposure: number,
  gain: number,
): string {
  return [
    secondsText(t, locale, exposure),
    t("wizard.files.gain", { gain }),
  ].join(t("detect.join"));
}

/** The darks found, as `30 s` (with the gain when two share an exposure). */
function foundDarks(
  t: Translate,
  locale: string,
  finding: Finding,
  exposure: number | null,
): string {
  const list = Array.isArray(finding.params.found) ? finding.params.found : [];
  const exposures = list.map((item) => numberOf(item.exposureSeconds));
  return conjunction(
    locale,
    list
      .filter((_, index) => exposures[index] !== null)
      .map((item, index) => {
        const value = exposures[index] as number;
        const shared =
          value === exposure ||
          exposures.filter((other) => other === value).length > 1;
        const gain = numberOf(item.gain);
        return shared && gain !== null
          ? withGain(t, locale, value, gain)
          : secondsText(t, locale, value);
      }),
  );
}

/** The night folder's `detection.darksMissing` and `detection.flatsMissing`
 * as lines; `null` for any other finding. */
export function calibrationFinding(
  t: Translate,
  locale: string,
  finding: Finding,
): CalibrationFinding | null {
  const p = finding.params;
  const target = String(p.target ?? "");
  const level: NoteLevel =
    finding.severity === "warning" ? "warning" : "information";
  if (finding.key === "detection.darksMissing") {
    const exposure = numberOf(p.exposureSeconds);
    const settings =
      exposure === null
        ? ""
        : numberOf(p.gain) === null
          ? secondsText(t, locale, exposure)
          : withGain(t, locale, exposure, Number(p.gain));
    const none = p.none === true;
    const count = Number(p.count ?? 0);
    const found = foundDarks(t, locale, finding, exposure);
    return {
      kind: "dark",
      level,
      files: none
        ? t("wizard.files.noDark", { settings, target })
        : pluralText(t, locale, "wizard.files.noDarkOther", count, {
            settings,
            target,
            found,
          }),
      treatment: none
        ? t("wizard.treatments.noDark", { settings })
        : pluralText(t, locale, "wizard.treatments.noDarkOther", count, {
            settings,
            found,
          }),
      without: none ? { kind: "dark", filter: null } : null,
      folder: none ? null : "darks",
    };
  }
  if (finding.key === "detection.flatsMissing") {
    const filter = typeof p.filter === "string" ? p.filter : null;
    const none = p.none === true;
    const without = p.stackedWithout === true;
    const files = filter
      ? t(none ? "wizard.files.noFlatAtAll" : "wizard.files.noFlat", { filter })
      : t(
          none
            ? "wizard.files.noFlatAtAllUnfiltered"
            : "wizard.files.noFlatUnfiltered",
        );
    return {
      kind: "flat",
      level,
      files,
      treatment: filter
        ? t(
            without
              ? "wizard.treatments.noFlatWithout"
              : "wizard.treatments.noFlat",
            { filter },
          )
        : t(
            without
              ? "wizard.treatments.noFlatWithoutUnfiltered"
              : "wizard.treatments.noFlatUnfiltered",
          ),
      without: without ? { kind: "flat", filter } : null,
      folder: none ? null : "flats",
    };
  }
  return null;
}

/** The calibration findings of one target, in the order the night gives them
 * (the flats of its filters, then the darks of its settings). */
export function calibrationFindings(
  t: Translate,
  locale: string,
  findings: readonly Finding[],
  targetId: string,
): CalibrationFinding[] {
  // The darks first, then the flats (the display order).
  return findings
    .filter((finding) => finding.targetId === targetId)
    .map((finding) => calibrationFinding(t, locale, finding))
    .filter((item): item is CalibrationFinding => item !== null)
    .sort((a, b) => (a.kind === b.kind ? 0 : a.kind === "dark" ? -1 : 1));
}

/** The summary line of a stacked target: « Empilée sans dark et sans flats
 * du filtre V. », from what the stacking really went without. */
export function stackedWithoutText(
  t: Translate,
  locale: string,
  items: readonly CalibrationFinding[],
): string | null {
  const dark = items.some((item) => item.without?.kind === "dark");
  const filters = [
    ...new Set(
      items
        .filter((item) => item.without?.kind === "flat")
        .map((item) => item.without?.filter ?? ""),
    ),
  ];
  const flats = filters.length > 0;
  if (!dark && !flats) return null;
  const named = filters.filter(Boolean);
  const filter = conjunction(locale, named);
  if (dark && flats) {
    return t(
      named.length > 0
        ? "wizard.summary.stackedWithout.both"
        : "wizard.summary.stackedWithout.bothUnfiltered",
      { filter },
    );
  }
  if (dark) return t("wizard.summary.stackedWithout.dark");
  return t(
    named.length > 0
      ? "wizard.summary.stackedWithout.flats"
      : "wizard.summary.stackedWithout.flatsUnfiltered",
    { filter },
  );
}

// ------------------------------------------------ the files that are not read

export interface UnreadCounts {
  videos: number;
  /** The text files a capture program wrote beside the videos. */
  captures: number;
  raw: number;
}

export function unreadCounts(proposal: Proposal): UnreadCounts {
  const aside = proposal.setAside;
  return {
    videos: aside.videos?.length ?? 0,
    captures: aside.videoCaptures?.length ?? 0,
    raw: aside.rawFiles?.length ?? 0,
  };
}

/** The folders hold no readable pose while they hold RAW files or videos:
 * the person believes everything was read, and nothing will come out of it.
 * A real empty folder changes nothing. */
export function nothingReadable(proposal: Proposal): boolean {
  const { videos, raw } = unreadCounts(proposal);
  return (
    proposal.reading.lights === 0 &&
    proposal.sessions.length === 0 &&
    videos + raw > 0
  );
}

/** The pieces added to the grey line under « Nuits »: the videos (not
 * processed, with their capture files) then the RAW files (not read). None
 * when nothing is readable: the warning says it then. */
export function unreadAsideParts(
  t: Translate,
  locale: string,
  counts: UnreadCounts,
): string[] {
  const parts: string[] = [];
  if (counts.videos > 0) {
    parts.push(
      counts.captures > 0
        ? pluralText(t, locale, "detect.aside.videosCaptured", counts.videos, {
            captures: counts.captures,
          })
        : pluralText(t, locale, "detect.aside.videos", counts.videos),
    );
  }
  if (counts.raw > 0) {
    parts.push(pluralText(t, locale, "detect.aside.raw", counts.raw));
  }
  return parts;
}

/** « Aucune pose lisible dans ce dossier. Astro Sync lit les poses FITS et
 * XISF : … ». */
export function nothingReadableText(
  t: Translate,
  locale: string,
  counts: UnreadCounts,
): string {
  const clauses: string[] = [];
  if (counts.raw > 0) {
    clauses.push(pluralText(t, locale, "detect.nothingRead.raw", counts.raw));
  }
  if (counts.videos > 0) {
    clauses.push(
      pluralText(t, locale, "detect.nothingRead.videos", counts.videos),
    );
  }
  return t("detect.nothingRead", {
    clauses: clauses.join(t("detect.comma")),
  });
}

/** The folder sheet's line (« Les FITS de la session »): what of this folder
 * is not read, nothing when there is nothing. */
export function sheetUnreadText(
  t: Translate,
  locale: string,
  counts: UnreadCounts,
): string | null {
  const parts: string[] = [];
  if (counts.videos > 0) {
    parts.push(
      pluralText(t, locale, "wizard.detect.unread.videos", counts.videos),
    );
  }
  if (counts.raw > 0) {
    parts.push(pluralText(t, locale, "wizard.detect.unread.raw", counts.raw));
  }
  return parts.length > 0
    ? t("wizard.detect.unread", { parts: parts.join(t("detect.sep")) })
    : null;
}

// --------------------------------------------------- why a pose has no target

/** `22 h 05 min 09 s +46° 28′ 44″`: the centre the solver found. */
export function skyPositionText(raDegrees: number, decDegrees: number): string {
  const total = Math.round(((((raDegrees % 360) + 360) % 360) / 15) * 3600);
  const wrapped = total % 86_400;
  const two = (value: number) => String(value).padStart(2, "0");
  const arc = Math.round(Math.abs(decDegrees) * 3600);
  return (
    `${two(Math.floor(wrapped / 3600))} h ${two(Math.floor((wrapped % 3600) / 60))} min ${two(wrapped % 60)} s ` +
    `${decDegrees < 0 ? "−" : "+"}${Math.floor(arc / 3600)}° ${two(Math.floor((arc % 3600) / 60))}′ ${two(arc % 60)}″`
  );
}

export interface SolveWhy {
  level: NoteLevel;
  text: string;
  /** « Réglages › Dépendances » helps: no solver, no ASTAP database, no
   * astrometry.net index. */
  settings: boolean;
}

/** The codes of a solver that the person corrects in « Réglages ›
 * Dépendances » (`outside_database`: another ASTAP star database suits the
 * field). */
const FIXED_IN_SETTINGS = new Set([
  "solver_missing",
  "no_database",
  "no_index",
  "outside_database",
]);

/** Why the solver, which was asked on one pose of this group, gave no target:
 * the words of its failure, or the position of the field when no known target
 * is there (an information with the position, no link: a known limit). `null` when no solver was
 * asked (the command line without `--solve`). */
export function solveWhy(
  t: Translate,
  target: Pick<DetectedTarget, "solveFailure" | "solvedPosition">,
): SolveWhy | null {
  const failure = target.solveFailure;
  if (failure) {
    const settings = FIXED_IN_SETTINGS.has(failure.code);
    return {
      level: settings ? "warning" : "information",
      // The sentence of astrometry.net's index is cut before its colon: the
      // link beside it already says where to go.
      text: t(
        failure.code === "no_index"
          ? "detect.night.solveWhy.no_index"
          : `annotations.failure.${failure.code}`,
        { seconds: Number(failure.seconds ?? 0) },
      ),
      settings,
    };
  }
  if (target.solvedPosition) {
    const [ra, dec] = target.solvedPosition;
    return {
      level: "information",
      text: t("detect.night.solvedAt", { position: skyPositionText(ra, dec) }),
      settings: false,
    };
  }
  return null;
}

/** Whether the solver was asked and gave nothing (« essayé »), else the
 * group is only waiting for it (« à faire »). */
export function solveTried(
  target: Pick<DetectedTarget, "solveFailure" | "solvedPosition">,
): boolean {
  return Boolean(target.solveFailure || target.solvedPosition);
}
