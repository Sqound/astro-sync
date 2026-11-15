// A series of poses: the light frames of one night and one target taken with
// the same filter, exposure, gain and binning. They are stacked together and
// never with another exposure; the service keeps one capture per session,
// target and series (`crate::series`, `capture-series.js`). What « Images à
// envoyer » and the « Récapitulatif » say of a target with several series:
// the banner, one line and one box per series, the main one (the longest)
// and the minor ones (under a tenth of another series of the same filter),
// unticked by default. Pure TypeScript, tested under `node --test`.

import type { TranslateParams } from "../i18n/locale.ts";

type Translate = (key: string, params?: TranslateParams) => string;

/** `uploadMetadata.series`, as Astro Sync sends it. */
export interface SeriesOfPoses {
  filter: string | null;
  exposureSeconds: number;
  gain: number | null;
  binning?: number;
  frames: number | null;
  totalSeconds: number | null;
}

/** A series under this share of the time of another series of the same
 * target and filter is minor: unticked by default, its box stays free. */
export const MINOR_SERIES_SHARE = 0.1;

const plain = (value: number) => String(Math.round(value * 1000) / 1000);

/** The key of a series (`IRCUT|30|200|1`), the service's `seriesKey`. */
export function seriesKey(series: SeriesOfPoses): string {
  const filter = (series.filter ?? "").trim().toUpperCase() || "-";
  const gain = series.gain === null ? "-" : plain(series.gain);
  return `${filter}|${plain(series.exposureSeconds)}|${gain}|${series.binning ?? 1}`;
}

/** The total seconds: what it says, else frames × exposure, else 0. */
export function seriesSeconds(series: SeriesOfPoses): number {
  if (series.totalSeconds !== null && series.totalSeconds >= 0)
    return series.totalSeconds;
  if (series.frames !== null) return series.frames * series.exposureSeconds;
  return 0;
}

/** One series of a target and what is decided of it. */
export interface TargetSeries {
  key: string;
  series: SeriesOfPoses;
  /** The longest series of the target: its image is the main one. */
  main: boolean;
  /** Under a tenth of the time of another series of the same filter. */
  minor: boolean;
}

/** The series of each target that has several, the longest first. `items`:
 * anything that names a target and, maybe, a series. */
export function targetsWithSeries(
  items: readonly {
    targetId: string;
    series?: SeriesOfPoses | null;
  }[],
): Map<string, TargetSeries[]> {
  const byTarget = new Map<string, Map<string, SeriesOfPoses>>();
  for (const item of items) {
    if (!item.series) continue;
    const held = byTarget.get(item.targetId) ?? new Map();
    const key = seriesKey(item.series);
    const known = held.get(key);
    // The series that says the most (its frames and total) is kept.
    if (!known || seriesSeconds(item.series) > seriesSeconds(known))
      held.set(key, item.series);
    byTarget.set(item.targetId, held);
  }
  const out = new Map<string, TargetSeries[]>();
  for (const [targetId, held] of byTarget) {
    if (held.size < 2) continue;
    const list = [...held.entries()]
      .map(([key, series]) => ({ key, series }))
      .sort(
        (a, b) =>
          seriesSeconds(b.series) - seriesSeconds(a.series) ||
          a.key.localeCompare(b.key),
      );
    out.set(
      targetId,
      list.map((entry, index) => ({
        ...entry,
        main: index === 0,
        minor: list.some(
          (other) =>
            other.key !== entry.key &&
            (other.series.filter ?? "").trim().toUpperCase() ===
              (entry.series.filter ?? "").trim().toUpperCase() &&
            seriesSeconds(entry.series) <
              MINOR_SERIES_SHARE * seriesSeconds(other.series),
        ),
      })),
    );
  }
  return out;
}

/** The boxes left unticked by default: the minor series, as
 * `<target>\u0000<key>`. */
export function defaultUntickedSeries(
  targets: ReadonlyMap<string, TargetSeries[]>,
): Set<string> {
  const out = new Set<string>();
  for (const [targetId, list] of targets)
    for (const entry of list)
      if (entry.minor) out.add(seriesBoxKey(targetId, entry.key));
  return out;
}

/** The key of the box of one series of one target. */
export function seriesBoxKey(targetId: string, key: string): string {
  return `${targetId}\u0000${key}`;
}

const NBSP = " ";

function exposureText(locale: string, seconds: number): string {
  return `${new Intl.NumberFormat(locale, { maximumFractionDigits: 3 }).format(seconds)}${NBSP}s`;
}

/** A duration as a series writes it (« 45 s », « 6 min », « 3 h 34 »,
 * « 2 h »), the service's `seriesDuration`. */
export function seriesDuration(seconds: number): string {
  const total = Math.max(0, Math.floor(seconds || 0));
  if (total < 60) return `${total}${NBSP}s`;
  const minutes = Math.floor(total / 60);
  if (minutes < 60) return `${minutes}${NBSP}min`;
  const hours = Math.floor(minutes / 60);
  const rest = minutes % 60;
  return rest
    ? `${hours}${NBSP}h${NBSP}${String(rest).padStart(2, "0")}`
    : `${hours}${NBSP}h`;
}

function durationText(_t: Translate, series: SeriesOfPoses): string {
  return seriesDuration(seriesSeconds(series));
}

/** « Série 30 s · gain 200 · 429 poses · 3 h 34 ». */
export function seriesRowText(
  t: Translate,
  locale: string,
  series: SeriesOfPoses,
): string {
  const exposure = exposureText(locale, series.exposureSeconds);
  const duration = durationText(t, series);
  if (series.gain !== null && series.frames !== null)
    return t("series.row", {
      exposure,
      gain: String(series.gain),
      frames: new Intl.NumberFormat(locale).format(series.frames),
      duration,
    });
  return t("series.rowShort", { exposure, duration });
}

/** The banner of a target with several series: its title, what they are and
 * which one gives the main image. */
export function seriesBanner(
  t: Translate,
  locale: string,
  target: string,
  list: readonly TargetSeries[],
): { title: string; body: string; main: string } {
  const items = list.map((entry) =>
    t("series.item", {
      filter: entry.series.filter ?? t("series.noFilter"),
      exposure: exposureText(locale, entry.series.exposureSeconds),
      frames:
        entry.series.frames === null
          ? "?"
          : new Intl.NumberFormat(locale).format(entry.series.frames),
      duration: durationText(t, entry.series),
    }),
  );
  // « A et B », in the words of the language (`Intl.ListFormat`).
  const ListFormat = (
    Intl as unknown as {
      ListFormat: new (
        locale: string,
        options: { style: string; type: string },
      ) => { format: (items: string[]) => string };
    }
  ).ListFormat;
  const joined = new ListFormat(locale, {
    style: "long",
    type: "conjunction",
  }).format(items);
  const main = list.find((entry) => entry.main) ?? list[0];
  return {
    title:
      list.length === 2
        ? t("series.banner.titleTwo", { target })
        : t("series.banner.title", { target, count: String(list.length) }),
    body: t("series.banner.body", { list: joined }),
    main: t("series.banner.main", { duration: durationText(t, main.series) }),
  };
}
