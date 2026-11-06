// The history of the uploads, the part that outlives the window. The queue's
// own database already keeps every file with its state, its date, its
// capture and the reason it was set aside; what it does not know is what the
// file IS (« M 31 · SHO »): the gesture that queued it says so, and the
// window keeps that beside the settings, per computer (`localStorage`, a
// convenience: without it a row shows the file's name). « Effacer
// l'historique » hides what is finished up to now; it deletes nothing in the
// queue (a file already sent stays known, so it is never sent twice). Pure
// TypeScript, tested under `node --test`.

import type { QueueEntry } from "./bridge";
import type { ReportStorage } from "./lastReport.ts";
import { baseName } from "./uploads.ts";

/** What a file is, as the gesture that queued it said. */
export interface UploadLabel {
  target: string | null;
  rendering: string | null;
  /** When it was said (ms), to forget the oldest first. */
  at: number;
}

export type UploadLabels = Record<string, UploadLabel>;

/** How many labels are kept: the newest. */
export const LABELS_MAX = 500;

export const LABELS_KEY = "astro-sync.upload-labels";
export const CLEARED_KEY = "astro-sync.upload-history-cleared";

function readJson(storage: ReportStorage | null, key: string): unknown {
  try {
    const raw = storage?.getItem(key);
    return raw ? (JSON.parse(raw) as unknown) : null;
  } catch {
    return null;
  }
}

export function loadLabels(storage: ReportStorage | null): UploadLabels {
  const raw = readJson(storage, LABELS_KEY);
  if (typeof raw !== "object" || raw === null) return {};
  const labels: UploadLabels = {};
  for (const [sha256, value] of Object.entries(raw)) {
    if (typeof value !== "object" || value === null) continue;
    const item = value as Partial<UploadLabel>;
    labels[sha256] = {
      target: typeof item.target === "string" ? item.target : null,
      rendering: typeof item.rendering === "string" ? item.rendering : null,
      at: typeof item.at === "number" ? item.at : 0,
    };
  }
  return labels;
}

export function saveLabels(
  storage: ReportStorage | null,
  labels: UploadLabels,
): void {
  try {
    storage?.setItem(LABELS_KEY, JSON.stringify(labels));
  } catch {
    // Nothing kept: the rows show the name of the file.
  }
}

/** The labels with `items` added (a file queued again takes its new label),
 * the oldest forgotten beyond `LABELS_MAX`. */
export function rememberLabels(
  labels: UploadLabels,
  items: readonly {
    sha256: string;
    target?: string | null;
    rendering?: string | null;
  }[],
  now: number,
): UploadLabels {
  const next: UploadLabels = { ...labels };
  for (const item of items) {
    if (!item.target && !item.rendering) continue;
    next[item.sha256] = {
      target: item.target ?? null,
      rendering: item.rendering ?? null,
      at: now,
    };
  }
  const kept = Object.entries(next).sort((a, b) => b[1].at - a[1].at);
  return kept.length <= LABELS_MAX
    ? next
    : Object.fromEntries(kept.slice(0, LABELS_MAX));
}

/** The moment (ISO text) before which the finished uploads are hidden, or
 * `null`. */
export function loadClearedAt(storage: ReportStorage | null): string | null {
  const raw = readJson(storage, CLEARED_KEY);
  return typeof raw === "string" ? raw : null;
}

export function saveClearedAt(
  storage: ReportStorage | null,
  clearedAt: string | null,
): void {
  try {
    if (clearedAt !== null)
      storage?.setItem(CLEARED_KEY, JSON.stringify(clearedAt));
  } catch {
    // The history comes back at the next opening: nothing was lost.
  }
}

export interface HistoryRow {
  sha256: string;
  fileName: string;
  size: number;
  target: string | null;
  rendering: string | null;
  state: "completed" | "refused" | "failed";
  captureId: string | null;
  /** The technical text and the stable code of what set it aside. */
  entry: QueueEntry;
  /** When it ended (ISO text). */
  at: string;
}

/** The finished uploads, newest first, without what was cleared. A file
 * still on its way (queued, uploading, held) is not history; neither is a
 * failure the queue will try again. */
export function historyRows(
  entries: readonly QueueEntry[],
  labels: UploadLabels,
  clearedAt: string | null,
): HistoryRow[] {
  const rows: HistoryRow[] = [];
  for (const entry of entries) {
    if (
      entry.state !== "completed" &&
      entry.state !== "refused" &&
      entry.state !== "failed"
    )
      continue;
    if (clearedAt !== null && entry.updatedAt <= clearedAt) continue;
    const label = labels[entry.sha256];
    rows.push({
      sha256: entry.sha256,
      fileName: baseName(entry.sourcePath),
      size: entry.size,
      target: label?.target ?? null,
      rendering: label?.rendering ?? null,
      state: entry.state,
      captureId: entry.captureId,
      entry,
      at: entry.updatedAt,
    });
  }
  return rows.sort((a, b) =>
    a.at < b.at ? 1 : a.at > b.at ? -1 : b.entry.id - a.entry.id,
  );
}

/** A row of « Avant la liaison »: written before the queue knew its spaces,
 * in any state (a file queued then and never sent included). */
export interface BeforeLinkRow {
  sha256: string;
  fileName: string;
  size: number;
  target: string | null;
  rendering: string | null;
  state: QueueEntry["state"];
  at: string;
}

/** What came before the link, newest first: shown apart, never cleared by
 * the line of the history (« Effacer l'historique » deletes it). */
export function beforeLinkRows(
  entries: readonly QueueEntry[],
  labels: UploadLabels,
): BeforeLinkRow[] {
  return entries
    .map((entry) => {
      const label = labels[entry.sha256];
      return {
        sha256: entry.sha256,
        fileName: baseName(entry.sourcePath),
        size: entry.size,
        target: label?.target ?? null,
        rendering: label?.rendering ?? null,
        state: entry.state,
        at: entry.updatedAt,
      };
    })
    .sort((a, b) => (a.at < b.at ? 1 : a.at > b.at ? -1 : 0));
}

/** « Effacer l'historique »: the newest end among the finished uploads
 * becomes the line the history starts after (never the clock of the
 * computer: a file that ends a moment later still shows). */
export function clearedAfter(
  entries: readonly QueueEntry[],
  previous: string | null,
): string | null {
  let latest = previous;
  for (const entry of entries) {
    if (
      (entry.state === "completed" ||
        entry.state === "refused" ||
        entry.state === "failed") &&
      (latest === null || entry.updatedAt > latest)
    )
      latest = entry.updatedAt;
  }
  return latest;
}

/** A capture's page in the space on the web (`<web>/<langue>/manage/
 * captures/<id>`). */
export function capturePage(
  web: string,
  locale: string,
  captureId: string,
): string {
  return `${web.replace(/\/$/, "")}/${locale}/manage/captures/${captureId}`;
}
