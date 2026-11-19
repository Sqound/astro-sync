// « Détecter les sessions », the treatments of the sessions created (states
// 6 b and 6 c): one card per target over every night created (a target taken
// on several nights gives one image, its nights stacked), the place left in
// the space, and the images that may be ticked in « Images à envoyer ».
// Pure: the window writes the words and runs the treatments.

import { seriesKey, type SeriesOfPoses } from "./seriesOfPoses.ts";
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
import type { MainPick, MainWish } from "./mainImage.ts";

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

/** The words of a dual-band filter's name, letters and digits only, lower
 * case: the core's `DUALBAND_WORDS` (`plan_image.rs`), word for word -- the
 * night's « Traitements » step reads the band from the core, the import's
 * from here, and both must say the same of one filter (a test reads the
 * core's list). One narrowband line alone (OIII, Hα, SII) is broadband. */
export const DUALBAND_WORDS = [
  "dual",
  "duo",
  "bibande",
  "extreme",
  "enhance",
  "ultimate",
  "alpt",
  "nbz",
  "hao3",
  "haoiii",
  "hoo",
  "triad",
  "quadband",
] as const;

/** The band of one filter name, as the core's `band_of` tells it. */
export function bandOfFilter(filter: string): Band {
  const name = filter.replace(/[^A-Za-z0-9]/g, "").toLowerCase();
  return DUALBAND_WORDS.some((word) => name.includes(word))
    ? "dualband"
    : "broadband";
}

/** The band of a target from its filters: dual-band when one names it. */
export function bandOfFilters(filters: readonly string[]): Band {
  return filters.some((filter) => bandOfFilter(filter) === "dualband")
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
  /** The series of poses (one filter, exposure, gain) a night's render was
   * stacked from, when the stacking kept its master: the capture comes from
   * the longest series, and a series far shorter than another of the same
   * filter opens unticked. */
  series?: {
    filter: string | null;
    totalSeconds: number;
    /** One frame's exposure, the gain, the binning and the frames: with the
     * filter, the series (one capture per series). */
    exposureSeconds?: number;
    gain?: number | null;
    binning?: number;
    frames?: number | null;
  };
  path: string;
  size: number;
  /** The TIFF in full resolution of a render that leaves as a PNG
   * (`render_files`): the second box of its PNG's card, ticked by a choice,
   * and only once the PNG is ticked. `twinOf` is the path of that PNG. */
  fullResolution?: boolean;
  twinOf?: string;
  /** `linear`: the file of « Sans traitement » -- a file attached to the
   * capture of its target (a TIFF to download), never an image of it. Listed
   * under the images of its target, ticked by a choice. */
  file?: "linear";
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
  /** `file`: the linear file, listed after the images of its target. */
  role: "capture" | "variant" | "file";
  /** Set when the row cannot be ticked, and why. */
  blocked: ImageBlock | null;
}

/** The key of a row. */
export function imageKey(result: Pick<TreatmentResult, "targetId" | "path">) {
  return `${result.targetId}\u0000${result.path}`;
}

/** Whether `result` carries a finish (a contrast, colours, softened stars). */
function isFinished(result: TreatmentResult): boolean {
  return hasContrast(result.contrast) || hasFinish(result);
}

/** The total exposure of the series of a result; 0 when not known. */
function seriesSeconds(result: TreatmentResult): number {
  return result.series?.totalSeconds ?? 0;
}

/** A series under this share of the total exposure of another series of the
 * same target and filter opens unticked in « Images à envoyer » (it stays
 * tickable). */
export const MINOR_SERIES_SHARE = 0.1;

/** Whether `result` comes from a series far shorter than another series of
 * the same target and filter ([`MINOR_SERIES_SHARE`]): 6 min of 10 s frames
 * beside 3 h 34 of 30 s ones. */
export function isMinorSeries(
  result: TreatmentResult,
  results: readonly TreatmentResult[],
): boolean {
  const mine = result.series;
  if (!mine) return false;
  return results.some(
    (other) =>
      other.targetId === result.targetId &&
      other.series !== undefined &&
      other.series.filter === mine.filter &&
      mine.totalSeconds < MINOR_SERIES_SHARE * other.series.totalSeconds,
  );
}

/** A path without its extension. */
function stemOf(path: string): string {
  return path.replace(/\.[^./\\]+$/, "");
}

/** Whether `result` is the render itself of the natural pipeline, the image
 * the night's wizard keeps as the main one of a capture. */
function isNaturalMaster(result: TreatmentResult): boolean {
  return (
    !result.fullResolution &&
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
    // The linear file is no image: it never is the capture nor a variant.
    const files = results.filter(
      (item) => item.targetId === targetId && item.file,
    );
    // With a finish chosen, only the finished image leaves, as the night's
    // wizard sends it (`stacking::stacked_candidates`): the bare render of a
    // pipeline whose finished image is listed, and its TIFF, stay on this
    // computer.
    const listed = results.filter(
      (item) => item.targetId === targetId && !item.file,
    );
    const bare = new Set(
      listed
        .filter(
          (item) =>
            !item.fullResolution &&
            !isFinished(item) &&
            listed.some(
              (other) =>
                !other.fullResolution &&
                isFinished(other) &&
                other.pipelineId === item.pipelineId &&
                stemOf(other.path).startsWith(`${stemOf(item.path)}_`),
            ),
        )
        .map((item) => item.path),
    );
    // The longest series of poses first: the capture comes from it (6 min
    // of 10 s frames never before 3 h 34 of 30 s ones), each series keeping
    // its order.
    const all = listed
      .filter(
        (item) =>
          !bare.has(item.path) && !(item.twinOf && bare.has(item.twinOf)),
      )
      .map((item, index) => ({ item, index }))
      .sort(
        (a, b) =>
          seriesSeconds(b.item) - seriesSeconds(a.item) || a.index - b.index,
      )
      .map(({ item }) => item);
    // The TIFF of a render is never the capture: it is listed right under
    // its PNG (a TIFF whose PNG is not listed stays an image of its own).
    const twins = all.filter(
      (item) =>
        item.fullResolution && all.some((other) => other.path === item.twinOf),
    );
    const mine = all.filter((item) => !twins.includes(item));
    // One capture per series of poses (CAPTURES-SERIES-2): the images of each
    // series, the longest first, each with its own capture — the natural
    // render when there is one, else the first image made without a curve,
    // else the first; the others keep their order. A series not known is
    // one, as before.
    const groups = new Map<string, TreatmentResult[]>();
    for (const item of mine) {
      const key = seriesKeyOfRow(item) ?? "";
      if (!groups.has(key)) groups.set(key, []);
      groups.get(key)!.push(item);
    }
    const heads = new Set<TreatmentResult>();
    const sorted = [...groups.values()].flatMap((group) => {
      const capture =
        group.find(isNaturalMaster) ??
        group.find(
          (item) =>
            !item.fullResolution &&
            !hasContrast(item.contrast) &&
            !hasFinish(item),
        ) ??
        group[0];
      heads.add(capture);
      return [capture, ...group.filter((item) => item !== capture)].flatMap(
        (item) => [item, ...twins.filter((twin) => twin.twinOf === item.path)],
      );
    });
    sorted.forEach((item) => {
      const role = heads.has(item) ? "capture" : "variant";
      rows.push({
        ...item,
        key: imageKey(item),
        role,
        blocked: blockOf(item, role, storage),
      });
    });
    for (const item of files) {
      rows.push({
        ...item,
        key: imageKey(item),
        role: "file",
        blocked: blockOf(item, "file", storage),
      });
    }
  }
  return rows;
}

/** A TIFF the person chooses to send, image by image: the TIFF in full
 * resolution of a render and the linear file. Never ticked from the start. */
export function isTiffChoice(
  row: Pick<TreatmentResult, "fullResolution" | "file">,
): boolean {
  return !!row.fullResolution || !!row.file;
}

/** The row of the PNG whose TIFF in full resolution `row` is (`twinOf` is its
 * path, in the same target); `null` for any other row, or when the PNG is not
 * listed. */
export function pngRowOf(
  row: Pick<SummaryImage, "targetId" | "fullResolution" | "twinOf">,
  rows: readonly SummaryImage[],
): SummaryImage | null {
  if (!row.fullResolution || !row.twinOf) return null;
  return (
    rows.find(
      (other) =>
        other.targetId === row.targetId &&
        other.path === row.twinOf &&
        !other.fullResolution,
    ) ?? null
  );
}

/** The keys of the TIFF rows of `row` (the one PNG of a card has at most
 * one): they leave with it, never alone. */
export function tiffKeysOf(
  row: Pick<SummaryImage, "targetId" | "path" | "fullResolution">,
  rows: readonly SummaryImage[],
): string[] {
  if (row.fullResolution) return [];
  return rows
    .filter(
      (other) =>
        other.fullResolution &&
        other.targetId === row.targetId &&
        other.twinOf === row.path,
    )
    .map((other) => other.key);
}

/** Whether `row` is the TIFF of a render whose PNG is not ticked: a TIFF
 * never leaves without its PNG, so its box stays shut (a TIFF whose PNG is
 * not listed has nothing to go with). */
export function tiffWaitsForPng(
  row: SummaryImage,
  rows: readonly SummaryImage[],
  ticked: readonly string[],
): boolean {
  if (!row.fullResolution) return false;
  const png = pngRowOf(row, rows);
  return !png || !ticked.includes(png.key);
}

function blockOf(
  item: TreatmentResult,
  role: "capture" | "variant" | "file",
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
    rows
      .filter((row) => row.role !== "file" && ticked.includes(row.key))
      .map((row) => row.targetId),
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
  if (tiffWaitsForPng(row, rows, ticked)) return false;
  const taken = tickedTargets(rows, ticked);
  // The linear file takes no place of its own: it is attached to the capture
  // of an image of its target, so it opens once one is ticked.
  if (row.role === "file") return taken.has(row.targetId);
  if (taken.has(row.targetId)) return true;
  return room === null || taken.size < room.remaining;
}

/** One image that leaves, and what it leaves as. */
export interface SendItem {
  row: SummaryImage;
  /** The image of the same sending whose capture this one is a variant of;
   * `null`: this image creates its capture, or is a variant of
   * `variantOfCapture`. */
  variantOf: SummaryImage | null;
  /** A capture of the space this image is a variant of, whatever it holds. */
  variantOfCapture?: string;
  /** What the space must confirm as the image leaves, for the image that
   * creates the capture of a series the space may hold already. */
  mainWish?: MainWish;
}

/** The images of `picks`' groups, the one that leads first with its TIFF. */
function leadFirst(
  rows: readonly SummaryImage[],
  picks: Readonly<Record<string, MainPick>>,
  groupOf: (row: SummaryImage) => string,
): SummaryImage[] {
  let list = [...rows];
  for (const pick of Object.values(picks)) {
    if (pick.kind !== "row") continue;
    const lead = list.find((row) => row.key === pick.key);
    if (!lead) continue;
    const moved = [
      lead,
      ...list.filter(
        (row) =>
          row.fullResolution &&
          row.targetId === lead.targetId &&
          row.twinOf === lead.path,
      ),
    ];
    const rest = list.filter((row) => !moved.includes(row));
    const at = rest.findIndex(
      (row) => row.role !== "file" && groupOf(row) === groupOf(lead),
    );
    rest.splice(
      at < 0 ? Math.min(list.indexOf(lead), rest.length) : at,
      0,
      ...moved,
    );
    list = rest;
  }
  return list;
}

/**
 * What leaves for a target: its rows ticked, the first of them the capture
 * and the others its variants. When the capture's row is not ticked, the
 * first row ticked is sent as the capture of the target. `picks` (what the
 * person chose, `mainPlan`) says better: the image chosen leads its series,
 * or every image of the series is a variant of a capture the space holds.
 */
export function sendOrder(
  rows: readonly SummaryImage[],
  ticked: readonly string[],
  picks: Readonly<Record<string, MainPick>> = {},
): SendItem[] {
  const sent: SendItem[] = [];
  // The head of each series of a target: the image that creates its capture,
  // or the capture of the space its images are variants of.
  const heads = new Map<
    string,
    { targetId: string; row: SummaryImage | null; capture: string | null }
  >();
  // One capture per series of poses: the first image ticked of each series of
  // a target is its capture, the others its variants (a series not known is
  // one, as before).
  const headKey = (row: SummaryImage) =>
    `${row.targetId}\u0000${seriesKeyOfRow(row) ?? ""}`;
  for (const row of leadFirst(rows, picks, headKey)) {
    if (!ticked.includes(row.key)) continue;
    // A TIFF never leaves without its PNG, whatever was ticked.
    if (row.fullResolution && tiffWaitsForPng(row, rows, ticked)) continue;
    const key = headKey(row);
    const head =
      heads.get(key) ??
      (row.role === "file"
        ? ([...heads.values()].find((h) => h.targetId === row.targetId) ?? null)
        : null);
    const pick = row.role === "file" ? undefined : picks[key];
    if (row.role === "file") {
      // A file needs the capture of an image sent with it; without one, it
      // stays here.
      if (head?.row) sent.push({ row, variantOf: head.row });
      else if (head?.capture)
        sent.push({ row, variantOf: null, variantOfCapture: head.capture });
      continue;
    }
    if (pick?.kind === "capture") {
      heads.set(key, {
        targetId: row.targetId,
        row: null,
        capture: pick.captureId,
      });
      sent.push({
        row,
        variantOf: null,
        variantOfCapture: pick.captureId,
      });
      continue;
    }
    if (!head) {
      heads.set(key, { targetId: row.targetId, row, capture: null });
      sent.push({
        row,
        variantOf: null,
        ...(pick?.kind === "row" && pick.key === row.key && pick.wish
          ? { mainWish: pick.wish }
          : {}),
      });
      continue;
    }
    sent.push({ row, variantOf: head.row });
  }
  return sent;
}

/** The series of poses of a row, when known, as `uploadMetadata.series`. */
export function seriesOfRow(
  row: Pick<TreatmentResult, "series">,
): SeriesOfPoses | null {
  const series = row.series;
  if (!series || typeof series.exposureSeconds !== "number") return null;
  return {
    filter: series.filter,
    exposureSeconds: series.exposureSeconds,
    gain: series.gain ?? null,
    binning: series.binning ?? 1,
    frames: series.frames ?? null,
    totalSeconds: series.totalSeconds,
  };
}

/** The key of the series of a row (`IRCUT|30|200|1`), `null` when unknown. */
export function seriesKeyOfRow(
  row: Pick<TreatmentResult, "series">,
): string | null {
  const series = seriesOfRow(row);
  return series ? seriesKey(series) : null;
}

/** How many images are ticked: the linear file is no image, it is not
 * counted. */
export function tickedImageCount(
  rows: readonly Pick<SummaryImage, "key" | "role">[],
  ticked: readonly string[],
): number {
  return ticked.filter(
    (key) => !rows.some((row) => row.key === key && row.role === "file"),
  ).length;
}

/** The primary of 6 c: « Envoyer 2 images », or « Terminer » with nothing
 * ticked. The linear file is not an image: it is not counted. */
export function sendPrimary(
  ticked: readonly string[],
  rows: readonly Pick<SummaryImage, "key" | "role">[] = [],
): { kind: "send"; count: number } | { kind: "done" } {
  const count = tickedImageCount(rows, ticked);
  return count > 0 ? { kind: "send", count } : { kind: "done" };
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
