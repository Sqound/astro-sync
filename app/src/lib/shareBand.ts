// « Exporter pour partager »: what the band of a shared copy says and how
// tall it is. The band exists only on a copy made on request, under the
// image and never over it; it is never written into the files sent to the
// space nor into the PNG and the TIFF of a render. It says what Astro Sync
// measured or read, and nothing else: no place, no coordinate, no name of a
// person; a datum that is missing is left out, never replaced by a word.
//
// Pure TypeScript, tested under `node --test`; the drawing is
// `shareBandDraw.ts`.

import type { Translate } from "../i18n/locale.ts";
import type { TreatmentResult } from "./detectTreatments.ts";
import { formatDuration } from "./duration.ts";
import { pluralText } from "./written.ts";

const NB = " ";

/** The band's height, as a share of the image's width. */
export const BAND_SHARE = 0.08;
/** The strip of the mark alone, as a share of the width. */
const STRIP_SHARE = 0.034;

/** The address of the site, in the band's mark. */
export const BAND_ADDRESS = "planastro.space";
/** The name of the application, in the band's mark (never translated). */
export const BAND_BRAND = `Astro${NB}Sync`;

export function bandHeight(width: number): number {
  return Math.max(40, Math.min(240, Math.round(BAND_SHARE * width)));
}

export function stripHeight(width: number): number {
  return Math.max(28, Math.min(110, Math.round(STRIP_SHARE * width)));
}

/** What the band may say of an image. */
export interface BandInput {
  /** The usual name of the target, when the catalogue has one. */
  name: string | null;
  /** The catalogue designation (M45, Sh2-190). */
  designation: string;
  /** The night(s): their count, and their dates (`AAAA-MM-JJ`) when known. */
  night: { count: number; dates: string[] } | null;
  /** The frames kept and the exposure of one of them, in seconds. */
  frames: number | null;
  exposureSeconds: number | null;
  /** The integration time, in seconds. */
  totalSeconds: number | null;
}

/** The data of an image row. `cardNights` are the evening dates of the
 * nights of the row's target, sorted: a night is told only when the row and
 * the card agree on how many there are. */
export function bandInputFromRow(
  row: Pick<TreatmentResult, "designation" | "nights" | "series">,
  name: string | null,
  cardNights: readonly string[],
): BandInput {
  let night: BandInput["night"] = null;
  if (row.nights === 1) {
    if (cardNights.length === 1) night = { count: 1, dates: [...cardNights] };
  } else if (row.nights > 1) {
    night = {
      count: row.nights,
      dates: cardNights.length === row.nights ? [...cardNights] : [],
    };
  }
  const series = row.series;
  const frames = series?.frames;
  const exposure = series?.exposureSeconds;
  return {
    name: name && name.trim() ? name.trim() : null,
    designation: row.designation,
    night,
    frames: frames && frames > 0 ? frames : null,
    exposureSeconds: exposure && exposure > 0 ? exposure : null,
    totalSeconds:
      series && series.totalSeconds > 0 ? series.totalSeconds : null,
  };
}

/** The words of the band: two on the left, one on the right. */
export interface BandText {
  top: string | null;
  bottom: string | null;
  integration: string | null;
}

const noBreak = (text: string) => text.replace(/ /g, NB);

function dateOf(value: string): Date | null {
  const date = new Date(`${value}T00:00:00Z`);
  return Number.isNaN(date.getTime()) ? null : date;
}

function nightText(
  t: Translate,
  locale: string,
  night: NonNullable<BandInput["night"]>,
): string | null {
  const dates = night.dates
    .map(dateOf)
    .filter((date): date is Date => date !== null);
  const format = new Intl.DateTimeFormat(locale, {
    dateStyle: "long",
    timeZone: "UTC",
  });
  if (night.count <= 1) {
    return dates.length === 1
      ? t("share.band.night", { date: format.format(dates[0]) })
      : null;
  }
  if (dates.length >= 2) {
    const first = dates[0];
    const last = dates[dates.length - 1];
    // `formatRange` writes « 22–24 octobre 2026 » in the language of the
    // window; an engine without it gets the two dates.
    const ranged = format as Intl.DateTimeFormat & {
      formatRange?: (from: Date, to: Date) => string;
    };
    const range = ranged.formatRange
      ? ranged.formatRange(first, last)
      : `${format.format(first)} – ${format.format(last)}`;
    return pluralText(t, locale, "share.band.span", night.count, { range });
  }
  return pluralText(t, locale, "share.band.count", night.count);
}

/** One frame's exposure: in seconds up to ten minutes (« 180 s », as the
 * poses are spoken of), then in minutes. */
function exposureText(t: Translate, locale: string, seconds: number): string {
  if (seconds >= 600) return formatDuration(t, seconds * 1000);
  return t("wizard.sending.durationSeconds", {
    seconds: new Intl.NumberFormat(locale).format(
      Math.round(seconds * 10) / 10,
    ),
  });
}

export function bandText(
  t: Translate,
  locale: string,
  input: BandInput,
): BandText {
  const designation = input.designation.trim() || null;
  const top = input.name ?? designation;
  const night = input.night ? nightText(t, locale, input.night) : null;
  const bottom =
    [input.name ? designation : null, night]
      .filter((part): part is string => !!part)
      .join(" · ") || null;

  let integration: string | null = null;
  const many = input.night && input.night.count > 1;
  const total =
    input.totalSeconds ??
    (input.frames && input.exposureSeconds
      ? input.frames * input.exposureSeconds
      : null);
  if (many) {
    if (total && input.night) {
      integration = `${pluralText(t, locale, "share.band.count", input.night.count)} · ${noBreak(formatDuration(t, total * 1000))}`;
    }
  } else if (input.frames && input.exposureSeconds && total) {
    integration = `${input.frames}${NB}×${NB}${noBreak(exposureText(t, locale, input.exposureSeconds))} · ${noBreak(formatDuration(t, total * 1000))}`;
  } else if (total) {
    integration = noBreak(formatDuration(t, total * 1000));
  }
  return { top, bottom, integration };
}

/** The two switches of the export sheet. */
export interface ShareOptions {
  data: boolean;
  mark: boolean;
}

export const SHARE_OPTIONS_KEY = "astro-sync.share-band";

/** The remembered switches; both on when nothing was chosen yet. */
export function parseShareOptions(raw: string | null): ShareOptions {
  const options = { data: true, mark: true };
  if (!raw) return options;
  try {
    const value = JSON.parse(raw) as Partial<ShareOptions> | null;
    if (value && typeof value.data === "boolean") options.data = value.data;
    if (value && typeof value.mark === "boolean") options.mark = value.mark;
  } catch {
    // Unreadable: the defaults.
  }
  return options;
}

export function serialiseShareOptions(options: ShareOptions): string {
  return JSON.stringify({ data: options.data, mark: options.mark });
}

export type BandKind = "none" | "band" | "strip";

/** The copy: the image at (0, 0), the band beneath it. */
export interface CopyLayout {
  kind: BandKind;
  width: number;
  imageHeight: number;
  bandHeight: number;
  height: number;
}

/** The data wanted and known: the full band (with the mark when it is
 * wanted); else the mark alone: a thin strip under the image; else the
 * image alone. */
export function copyLayout(
  width: number,
  imageHeight: number,
  text: BandText,
  options: ShareOptions,
): CopyLayout {
  const known = !!(text.top || text.bottom || text.integration);
  let kind: BandKind = "none";
  if (options.data && known) kind = "band";
  else if (options.mark) kind = "strip";
  const band =
    kind === "band"
      ? bandHeight(width)
      : kind === "strip"
        ? stripHeight(width)
        : 0;
  return {
    kind,
    width,
    imageHeight,
    bandHeight: band,
    height: imageHeight + band,
  };
}

/** The file name proposed: from the designation, safe on every system. */
export function shareFileName(designation: string): string {
  const safe = designation
    .normalize("NFC")
    .replace(/[^\p{L}\p{N}-]+/gu, "-")
    .replace(/^-+|-+$/g, "");
  return safe ? `${safe}-astro-sync.png` : "astro-sync.png";
}

/** The dictionary key of the sheet's second button: « Annuler » until the
 * copy is saved, « Fermer » once it is (nothing is left to cancel). */
export function shareCloseKey(saved: boolean): string {
  return saved ? "common.close" : "share.cancel";
}
