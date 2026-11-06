// « Détecter les sessions », the treatments of the sessions created (states
// 6 b and 6 c): one card per target over every night created (a target taken
// on several nights gives one image, its nights stacked), the place left in
// the space, and the images that may be ticked in « Images à envoyer ».
// Pure: the window writes the words and runs the treatments.

import type {
  Band,
  BulkCreateOutcome,
  CreatedMultiNight,
  Proposal,
  SpaceStorage,
  TreatmentNight,
} from "./bridge";
import { hasContrast, type Contrast } from "./contrast.ts";
import { hasFinish, type Colour, type SoftStars } from "./finish.ts";
import { NATURAL_ID } from "./pipelineChoice.ts";

/** `Number.MAX_SAFE_INTEGER`: a number the profile does not cap. */
const UNLIMITED = 9007199254740991;

export interface TreatmentNightRef {
  detectedId: string;
  sessionId: string;
  eveningDate: string;
  frames: number;
}

export interface TreatmentTargetCard {
  targetId: string;
  designation: string;
  band: Band;
  /** The filters the headers name, if any. */
  filters: string[];
  nights: TreatmentNightRef[];
  frames: number;
  /** The multi-night plan created for it, when there is one. */
  multiNight: CreatedMultiNight | null;
}

const DUAL_BAND =
  /dual|duo|tri-?band|quad|l-?e?nhance|l-?extreme|l-?ultimate|alp-?t|nbz|hα|h-?alpha|oiii|o3|s2|sii/i;

/** The band of a target from its filters: dual-band when one names it. */
export function bandOfFilters(filters: readonly string[]): Band {
  return filters.some((filter) => DUAL_BAND.test(filter))
    ? "dualband"
    : "broadband";
}

/** One card per target of the sessions created, the one with the most frames
 * first; a target without a catalogue id is not treated here (it has no
 * folder of its own). */
export function treatmentCards(
  proposal: Proposal,
  outcome: BulkCreateOutcome,
): TreatmentTargetCard[] {
  const cards = new Map<string, TreatmentTargetCard>();
  for (const made of outcome.sessions) {
    const session = proposal.sessions.find((s) => s.id === made.detectedId);
    if (!session) continue;
    for (const target of session.targets) {
      if (!target.targetId || !made.designations.includes(target.designation))
        continue;
      const card = cards.get(target.targetId) ?? {
        targetId: target.targetId,
        designation: target.designation,
        band: "broadband" as Band,
        filters: [],
        nights: [],
        frames: 0,
        multiNight: null,
      };
      card.nights.push({
        detectedId: made.detectedId,
        sessionId: made.sessionId,
        eveningDate: made.eveningDate,
        frames: target.frames,
      });
      card.frames += target.frames;
      for (const filter of target.filters)
        if (!card.filters.includes(filter)) card.filters.push(filter);
      cards.set(target.targetId, card);
    }
  }
  for (const card of cards.values()) {
    card.band = bandOfFilters(card.filters);
    card.nights.sort((a, b) => a.eveningDate.localeCompare(b.eveningDate));
    card.multiNight =
      outcome.multiNight.find(
        (group) =>
          group.designation === card.designation &&
          card.nights.every((night) =>
            group.sessionIds.includes(night.sessionId),
          ),
      ) ?? null;
  }
  return [...cards.values()].sort(
    (a, b) => b.frames - a.frames || a.designation.localeCompare(b.designation),
  );
}

/** The head of 6 b: « 6 nuits · 4 cibles · 453 poses ». */
export function treatmentCounts(cards: readonly TreatmentTargetCard[]): {
  nights: number;
  targets: number;
  frames: number;
} {
  const nights = new Set(
    cards.flatMap((c) => c.nights.map((n) => n.sessionId)),
  );
  return {
    nights: nights.size,
    targets: cards.length,
    frames: cards.reduce((sum, card) => sum + card.frames, 0),
  };
}

/** « 2 images sur 3 »: the captures the space may still receive, or `null`
 * when nothing caps them (a subscription, a private server) or the storage
 * is not known. */
export function imageRoom(
  storage: SpaceStorage | null,
): { remaining: number; quota: number } | null {
  if (!storage) return null;
  const quota = storage.captureCountLimit;
  if (!(quota > 0) || quota >= UNLIMITED) return null;
  return { remaining: Math.max(0, quota - storage.captureCount), quota };
}

/** A result of the treatments: one image of a target, what « Images à
 * envoyer » lists. A target has one per pipeline chosen. */
export interface TreatmentResult {
  targetId: string;
  designation: string;
  pipelineId: string;
  /** The gain of the contrast of presentation the image carries; absent or
   * 0: the render itself. */
  contrast?: Contrast;
  /** The colour intensity and the softened stars the image carries, in
   * percent; absent or 0: none. */
  colour?: Colour;
  softStars?: SoftStars;
  /** The nights stacked into it (1 for the image of one night). */
  nights: number;
  path: string;
  size: number;
}

/** Why a row of « Images à envoyer » cannot be ticked. */
export type ImageBlock =
  /** Above the per-file ceiling of the space (`storage.maxFileBytes`). */
  | { kind: "tooHeavy"; size: number; max: number }
  /** The space keeps one image per capture (`storage.variantLimit` is 0). */
  | { kind: "noVariants" };

/** One row of « Images à envoyer »: the capture of a target, or one of its
 * variants beneath it. */
export interface SummaryImage extends TreatmentResult {
  /** The target and the file: what the box of the row stands for. */
  key: string;
  role: "capture" | "variant";
  /** Set when the row cannot be ticked, and why. */
  blocked: ImageBlock | null;
}

/** The key of a row. */
export function imageKey(result: Pick<TreatmentResult, "targetId" | "path">) {
  return `${result.targetId}\u0000${result.path}`;
}

/** Whether `result` is the render itself of the natural pipeline, the image
 * the night's wizard keeps as the main one of a capture. */
function isNaturalMaster(result: TreatmentResult): boolean {
  return (
    result.pipelineId === NATURAL_ID &&
    !hasContrast(result.contrast) &&
    !hasFinish(result)
  );
}

/**
 * The rows of « Images à envoyer »: for each target, in the order of the
 * targets, its images -- the first is the capture, the others are its
 * variants listed beneath it. The order is the night wizard's
 * (`precheck::plan_images`): the natural render is the capture when it is
 * among them, whatever the order of the pipelines, and the other images keep
 * theirs (the ones with the curve of presentation, softer stars or stronger colours are never the capture).
 *
 * `storage` is what the service said of the space (`null` while unknown: no
 * guard then): an image above the per-file ceiling cannot be ticked, nor a
 * variant in a space that keeps none.
 */
export function summaryImages(
  results: readonly TreatmentResult[],
  storage: SpaceStorage | null,
): SummaryImage[] {
  const targets = [...new Set(results.map((item) => item.targetId))];
  const rows: SummaryImage[] = [];
  for (const targetId of targets) {
    const mine = results.filter((item) => item.targetId === targetId);
    // The capture: the natural render when there is one, else the first
    // image made without a curve, else the first; the others keep their order.
    const capture =
      mine.find(isNaturalMaster) ??
      mine.find((item) => !hasContrast(item.contrast) && !hasFinish(item)) ??
      mine[0];
    const sorted = [capture, ...mine.filter((item) => item !== capture)];
    sorted.forEach((item, index) => {
      const role = index === 0 ? "capture" : "variant";
      rows.push({
        ...item,
        key: imageKey(item),
        role,
        blocked: blockOf(item, role, storage),
      });
    });
  }
  return rows;
}

function blockOf(
  item: TreatmentResult,
  role: "capture" | "variant",
  storage: SpaceStorage | null,
): ImageBlock | null {
  if (!storage) return null;
  if (storage.maxFileBytes > 0 && item.size > storage.maxFileBytes) {
    return { kind: "tooHeavy", size: item.size, max: storage.maxFileBytes };
  }
  if (role === "variant" && !(storage.variantLimit > 0)) {
    return { kind: "noVariants" };
  }
  return null;
}

/** The targets that have at least one row ticked: each takes one place of the
 * space (a variant takes none). */
export function tickedTargets(
  rows: readonly SummaryImage[],
  ticked: readonly string[],
): Set<string> {
  return new Set(
    rows.filter((row) => ticked.includes(row.key)).map((row) => row.targetId),
  );
}

/** Whether the box of `row` may be ticked: a ticked row stays; a blocked row
 * never opens; a row of a target already ticked takes no new place; the
 * first row of another target only while the place left is not taken. */
export function canTickImage(
  row: SummaryImage,
  rows: readonly SummaryImage[],
  ticked: readonly string[],
  room: { remaining: number } | null,
): boolean {
  if (ticked.includes(row.key)) return true;
  if (row.blocked) return false;
  const taken = tickedTargets(rows, ticked);
  if (taken.has(row.targetId)) return true;
  return room === null || taken.size < room.remaining;
}

/**
 * What leaves for a target: its rows ticked, the first of them the capture
 * and the others its variants. When the capture's row is not ticked, the
 * first row ticked is sent as the capture of the target.
 */
export function sendOrder(
  rows: readonly SummaryImage[],
  ticked: readonly string[],
): { row: SummaryImage; variantOf: SummaryImage | null }[] {
  const sent: { row: SummaryImage; variantOf: SummaryImage | null }[] = [];
  const firstOf = new Map<string, SummaryImage>();
  for (const row of rows) {
    if (!ticked.includes(row.key)) continue;
    const head = firstOf.get(row.targetId) ?? null;
    if (!head) firstOf.set(row.targetId, row);
    sent.push({ row, variantOf: head });
  }
  return sent;
}

/** The primary of 6 c: « Envoyer 2 images », or « Terminer » with nothing
 * ticked. */
export function sendPrimary(
  ticked: readonly string[],
): { kind: "send"; count: number } | { kind: "done" } {
  return ticked.length > 0
    ? { kind: "send", count: ticked.length }
    : { kind: "done" };
}

/** « TIFF » from the file's name. */
export function formatOf(path: string): string {
  const extension = path.split(".").pop() ?? "";
  return extension
    .toUpperCase()
    .replace("TIF", "TIFF")
    .replace("TIFFF", "TIFF");
}

/** The folder of each target the treatments prepared: its folder in the
 * first night that holds it (`<night>/<target>`), by target. */
/** The folder of each night the treatments prepared, by the night's id in
 * the proposal: where its report of the logs reads. */
export function treatmentNightFolders(
  nights: readonly TreatmentNight[],
): Record<string, string> {
  const folders: Record<string, string> = {};
  for (const night of nights) folders[night.detectedId] = night.nightFolder;
  return folders;
}

export function treatmentFolders(
  nights: readonly TreatmentNight[],
): Record<string, string> {
  const folders: Record<string, string> = {};
  for (const night of nights) {
    const separator = night.nightFolder.includes("\\") ? "\\" : "/";
    for (const target of night.targets) {
      folders[target.targetId] ??=
        `${night.nightFolder.replace(/[\\/]+$/, "")}${separator}${target.folderName}`;
    }
  }
  return folders;
}

/** What « Ouvrir le dossier de travail » opens: the folder of the target
 * being treated when the treatments made it, else the working folder. */
export function folderToOpen(
  folders: Readonly<Record<string, string>>,
  targetId: string | null,
  workingFolder: string | null,
): string | null {
  return (targetId !== null ? folders[targetId] : undefined) ?? workingFolder;
}
