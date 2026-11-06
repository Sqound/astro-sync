// The live state of the uploads of this window: what the upload queue tells
// through `astro-sync://upload-event` (started, progress, completed, refused,
// held, waiting, failed, stopped), kept as a list of rows that the « Envois »
// page, the badge on its menu entry, the notifications and the banner of the
// result all read. Pure TypeScript (no React, no Tauri), so the reducer is
// tested from a recorded sequence of events under `node --test`;
// `components/UploadsProvider.tsx` feeds it.
//
// The history that outlives the window is NOT here: it is the queue's own
// database (every file keeps its state, its date and its capture), read by
// `uploadHistory.ts`.

import type {
  AlreadySent,
  QueueDriveEvent,
  QueueEntry,
  SkyAnnotation,
} from "./bridge";
import type { TranslateParams } from "../i18n/locale";
import { failureText } from "./annotationsTab.ts";
import { SOURCE_FILE_MISSING, missingFileText, reasonText } from "./queue.ts";
import { waitText } from "./space.ts";
import { READ_ONLY_CODE, heldWindow, paymentFailedNotice } from "./standing.ts";
import type { SnackInput, SnackTone } from "./snackbar.ts";
import { formatBytes, formatDateTime, pluralText } from "./written.ts";

type Translate = (key: string, params?: TranslateParams) => string;

export type UploadStatus =
  "queued" | "uploading" | "waiting" | "done" | "refused" | "held" | "failed";

/** Where an upload was asked for: the detection flow has a banner of its own
 * on the result page; whatever else leaves (a night's summary, the page
 * itself) is `other`. */
export type UploadOrigin = "detect" | "other";

export interface UploadRow {
  sha256: string;
  origin: UploadOrigin;
  /** The group of files one gesture queued together (« Envoyer 2 images »). */
  batch: number;
  fileName: string;
  size: number;
  /** « M 31 », when the gesture that queued the file said it. */
  target: string | null;
  /** « SHO », the rendering (pipeline) of the image. */
  rendering: string | null;
  /** The file's path as the queue named it when its turn came: what
   * « Fichier introuvable » says of a file that is gone. */
  path: string | null;
  status: UploadStatus;
  sent: number;
  total: number;
  /** Bytes per second, smoothed; `null` before two measures. */
  speed: number | null;
  /** The clock (ms) of the last measure of progress. */
  lastAt: number | null;
  captureId: string | null;
  /** The service already held these bytes. */
  deduplicated: boolean;
  /** The file was already sent to the linked space (ISO date): enqueued
   * again, nothing left; the page says « Déjà envoyé le … » and offers
   * « Renvoyer ». `null` for a file this run sent. */
  alreadySentAt: string | null;
  /** The stable code of the refusal, the hold or the failure. */
  code: string | null;
  message: string | null;
  retryInSeconds: number | null;
  /** A failure that left the file in the queue: trying again is possible. */
  retryable: boolean;
  /** The largest file the space takes, when a refusal for size said it. */
  limit: number | null;
  /** Held because the space is read-only (`entitlement_read_only`) while a
   * payment that failed explains it: the end of the window, as the service
   * wrote it (`lib/standing.ts`). `null`: the ordinary sentence of the
   * refusal tells it. */
  heldUntil: string | null;
  /** The upload still has its notification (a snackbar at the bottom of the
   * window): the person may close it, closing never stops the upload. */
  notice: boolean;
}

export interface UploadsState {
  rows: readonly UploadRow[];
  /** The end of the read-only window of a payment that failed, when the
   * space is in one: what a file held for `entitlement_read_only` is
   * explained by. */
  paymentFailedUntil: string | null;
  batch: number;
  /** Bumped when a file reaches an end: the page reads the queue again. */
  revision: number;
  /** The sky annotations of the files sent, by digest: solved here after each
   * upload, told by `astro-sync://sky-event`, and kept in the queue's
   * database for the history. */
  sky: Readonly<Record<string, SkyAnnotation>>;
}

export const NO_UPLOADS: UploadsState = {
  rows: [],
  paymentFailedUntil: null,
  batch: 0,
  revision: 0,
  sky: {},
};

/** A file just queued, as the gesture knows it. */
export interface TrackedFile {
  sha256: string;
  path: string;
  size: number;
  target?: string | null;
  rendering?: string | null;
}

export type UploadsAction =
  | { type: "track"; origin: UploadOrigin; files: readonly TrackedFile[] }
  | { type: "event"; event: QueueDriveEvent; now: number }
  /** The files the queue holds from an earlier run of the window. */
  | { type: "seed"; entries: readonly QueueEntry[] }
  /** What the queue says now of the files that failed: still in the queue
   * (trying again is possible) or set aside for good. */
  | { type: "reconcile"; entries: readonly QueueEntry[] }
  /** The files the queue of the linked space already had sent, as the
   * enqueueing said: their rows end at once. */
  | { type: "alreadySent"; files: readonly AlreadySent[] }
  /** « Renvoyer » was asked for a file already sent: it is queued again. */
  | { type: "resend"; sha256: string }
  | { type: "dismiss"; sha256: string }
  | { type: "dismissAll" }
  | { type: "requeue"; sha256: string | null }
  | { type: "limit"; sha256: string; bytes: number }
  /** What the service says of the billing of the space: the end of the
   * read-only window of a payment that failed, or `null`. The files held
   * for `entitlement_read_only` say it. */
  | { type: "standing"; until: string | null }
  /** The annotations of an uploaded image changed (the sky event). */
  | { type: "sky"; annotation: SkyAnnotation }
  /** What the queue's database keeps of the annotations of the files sent. */
  | { type: "skySeed"; annotations: readonly SkyAnnotation[] }
  /** « Réessayer » was asked: the annotations are running again. */
  | { type: "skyRetry"; sha256: string }
  /** A run that could not even start (not paired, ...). */
  | { type: "runFailed"; code: string; message: string };

/** The last part of a path, whatever its separator. */
export function baseName(path: string): string {
  const parts = path.replace(/\\/g, "/").split("/");
  return parts[parts.length - 1] || path;
}

function blank(sha256: string, batch: number): UploadRow {
  return {
    sha256,
    origin: "other",
    batch,
    fileName: sha256.slice(0, 12),
    size: 0,
    target: null,
    rendering: null,
    path: null,
    status: "queued",
    sent: 0,
    total: 0,
    speed: null,
    lastAt: null,
    captureId: null,
    deduplicated: false,
    alreadySentAt: null,
    code: null,
    message: null,
    retryInSeconds: null,
    retryable: false,
    limit: null,
    heldUntil: null,
    notice: false,
  };
}

function change(
  state: UploadsState,
  sha256: string,
  update: (row: UploadRow) => UploadRow,
  create?: () => UploadRow,
): UploadsState {
  const index = state.rows.findIndex((row) => row.sha256 === sha256);
  if (index < 0) {
    return {
      ...state,
      rows: [
        ...state.rows,
        update((create ?? (() => blank(sha256, state.batch)))()),
      ],
    };
  }
  const rows = state.rows.slice();
  rows[index] = update(rows[index]);
  return { ...state, rows };
}

const ENDED: ReadonlySet<UploadStatus> = new Set([
  "done",
  "refused",
  "failed",
  "held",
]);

export function uploadsReducer(
  state: UploadsState,
  action: UploadsAction,
): UploadsState {
  switch (action.type) {
    case "track": {
      const batch = state.batch + 1;
      let next: UploadsState = { ...state, batch };
      for (const file of action.files) {
        next = change(
          next,
          file.sha256,
          (row) => ({
            ...blank(file.sha256, batch),
            origin: action.origin,
            fileName: baseName(file.path),
            path: file.path,
            size: file.size,
            total: file.size,
            target: file.target ?? row.target,
            rendering: file.rendering ?? row.rendering,
          }),
          () => blank(file.sha256, batch),
        );
      }
      return next;
    }
    case "seed": {
      let next = state;
      for (const entry of action.entries) {
        if (
          entry.state !== "queued" &&
          entry.state !== "uploading" &&
          entry.state !== "held"
        )
          continue;
        if (next.rows.some((row) => row.sha256 === entry.sha256)) continue;
        next = change(next, entry.sha256, (row) => ({
          ...row,
          fileName: baseName(entry.sourcePath),
          path: entry.sourcePath,
          size: entry.size,
          total: entry.size,
          sent: entry.offset,
          status: entry.state === "held" ? "held" : "queued",
          code: entry.state === "held" ? entry.lastErrorCode : null,
          message: entry.state === "held" ? entry.lastError : null,
          heldUntil:
            entry.state === "held" && entry.lastErrorCode === READ_ONLY_CODE
              ? state.paymentFailedUntil
              : null,
        }));
      }
      return next;
    }
    case "reconcile": {
      const rows = state.rows.map((row) => {
        // A file waiting for a run that the queue of the linked space already
        // has sent: nothing will leave, the row says so instead of waiting.
        if (row.status === "queued") {
          const entry = action.entries.find(
            (item) => item.sha256 === row.sha256,
          );
          return entry?.state === "completed" ? sentBefore(row, entry) : row;
        }
        if (row.status !== "failed") return row;
        const entry = action.entries.find((item) => item.sha256 === row.sha256);
        if (!entry) return row;
        // A file set aside because it was not found can be tried again once
        // it is back (« Réessayer » puts it in the queue again).
        return {
          ...row,
          retryable:
            entry.state !== "failed" ||
            entry.lastErrorCode === SOURCE_FILE_MISSING,
        };
      });
      return { ...state, rows };
    }
    case "alreadySent": {
      let next = state;
      for (const file of action.files) {
        const tracked = next.rows.find((row) => row.sha256 === file.sha256);
        if (!tracked || tracked.status !== "queued") continue;
        next = change(next, file.sha256, (row) =>
          sentBefore(row, {
            captureId: file.captureId,
            updatedAt: file.sentAt,
          }),
        );
      }
      return next;
    }
    case "resend":
      return change(state, action.sha256, (row) => ({
        ...row,
        status: "queued",
        alreadySentAt: null,
        deduplicated: false,
        sent: 0,
        speed: null,
        code: null,
        message: null,
        notice: false,
      }));
    case "dismiss":
      return change(state, action.sha256, (row) => ({ ...row, notice: false }));
    case "dismissAll":
      return {
        ...state,
        rows: state.rows.map((row) => ({ ...row, notice: false })),
      };
    case "requeue":
      return {
        ...state,
        rows: state.rows.map((row) =>
          (action.sha256 === null || row.sha256 === action.sha256) &&
          (row.status === "failed" || row.status === "waiting") &&
          row.retryable
            ? {
                ...row,
                status: "queued",
                code: null,
                message: null,
                speed: null,
                retryable: false,
              }
            : row,
        ),
      };
    case "limit":
      return change(state, action.sha256, (row) => ({
        ...row,
        limit: action.bytes,
      }));
    case "standing":
      return {
        ...state,
        paymentFailedUntil: action.until,
        rows: state.rows.map((row) =>
          row.code === READ_ONLY_CODE
            ? { ...row, heldUntil: action.until }
            : row,
        ),
      };
    case "runFailed":
      return {
        ...state,
        rows: state.rows.map((row) =>
          row.status === "queued"
            ? {
                ...row,
                status: "failed",
                code: action.code,
                message: action.message,
                retryable: true,
                notice: true,
              }
            : row,
        ),
        revision: state.revision + 1,
      };
    case "event":
      return applyEvent(state, action.event, action.now);
    case "sky":
      return {
        ...state,
        sky: { ...state.sky, [action.annotation.sha256]: action.annotation },
      };
    case "skySeed": {
      const sky = { ...state.sky };
      for (const annotation of action.annotations) {
        const known = sky[annotation.sha256];
        // The event that just came is newer than the read of the database.
        if (!known || known.updatedAt <= annotation.updatedAt)
          sky[annotation.sha256] = annotation;
      }
      return { ...state, sky };
    }
    case "skyRetry": {
      const known = state.sky[action.sha256];
      if (!known) return state;
      const {
        failure: _failure,
        errorCode: _code,
        message: _message,
        ...rest
      } = known;
      return {
        ...state,
        sky: { ...state.sky, [action.sha256]: { ...rest, state: "running" } },
      };
    }
  }
}

/** A row whose file the linked space already has: ended, with its capture and
 * the day it was sent. */
function sentBefore(
  row: UploadRow,
  entry: Pick<QueueEntry, "captureId" | "updatedAt">,
): UploadRow {
  return {
    ...row,
    status: "done",
    captureId: entry.captureId,
    alreadySentAt: entry.updatedAt,
    sent: row.total || row.size,
    speed: null,
    notice: true,
  };
}

function applyEvent(
  state: UploadsState,
  event: QueueDriveEvent,
  now: number,
): UploadsState {
  switch (event.kind) {
    case "Started":
      return change(state, event.sha256, (row) => ({
        ...row,
        fileName: event.sourcePath ? baseName(event.sourcePath) : row.fileName,
        path: event.sourcePath || row.path,
        size: row.size || event.bytes,
        total: event.bytes,
        sent: 0,
        status: "uploading",
        speed: null,
        lastAt: now,
        code: null,
        message: null,
        retryable: false,
        notice: true,
      }));
    case "Progress":
      return change(state, event.sha256, (row) => {
        const seconds = row.lastAt === null ? 0 : (now - row.lastAt) / 1000;
        let speed = row.speed;
        if (seconds > 0 && event.sent >= row.sent) {
          const instant = (event.sent - row.sent) / seconds;
          speed = speed === null ? instant : 0.6 * speed + 0.4 * instant;
        }
        return {
          ...row,
          status: "uploading",
          sent: event.sent,
          total: event.total,
          size: row.size || event.total,
          speed,
          lastAt: now,
        };
      });
    case "Completed":
      return end(state, event.sha256, (row) => ({
        ...row,
        status: "done",
        captureId: event.captureId,
        deduplicated: event.deduplicated,
        alreadySentAt: null,
        sent: row.total || row.size,
        total: row.total || row.size,
        speed: null,
        notice: true,
      }));
    case "Failed":
      return end(state, event.sha256, (row) => ({
        ...row,
        status: "failed",
        code: event.code,
        message: event.message,
        speed: null,
        // Settled by `reconcile` once the queue is read again.
        retryable: true,
        notice: true,
      }));
    case "Refused":
      return end(state, event.sha256, (row) => ({
        ...row,
        status: "refused",
        code: event.code,
        message: event.message,
        speed: null,
        heldUntil:
          event.code === READ_ONLY_CODE ? state.paymentFailedUntil : null,
        notice: true,
      }));
    case "Held":
      return end(state, event.sha256, (row) => ({
        ...row,
        status: "held",
        code: event.code,
        message: event.message,
        speed: null,
        // A payment that failed explains the read-only space: the file is
        // held with that reason, not refused as an error.
        heldUntil:
          event.code === READ_ONLY_CODE ? state.paymentFailedUntil : null,
        notice: true,
      }));
    case "Waiting":
      return change(state, event.sha256, (row) => ({
        ...row,
        status: "waiting",
        code: event.code,
        message: event.message,
        retryInSeconds: event.retryInSeconds,
        speed: null,
      }));
    case "Stopped": {
      const bump = { ...state, revision: state.revision + 1 };
      if (event.reason === "paused" || event.reason === "cancelled") {
        return {
          ...bump,
          rows: bump.rows.map((row) =>
            row.status === "uploading" || row.status === "waiting"
              ? { ...row, status: "queued", speed: null }
              : row,
          ),
        };
      }
      if (event.reason === "no_progress" && event.sha256 !== null) {
        return change(bump, event.sha256, (row) =>
          row.status === "failed"
            ? row
            : {
                ...row,
                status: "failed",
                code: "no_progress",
                message: null,
                retryable: true,
                speed: null,
                notice: true,
              },
        );
      }
      return bump;
    }
  }
}

function end(
  state: UploadsState,
  sha256: string,
  update: (row: UploadRow) => UploadRow,
): UploadsState {
  const next = change(state, sha256, update);
  return { ...next, revision: next.revision + 1 };
}

/** What a run attributes the files without an attribution of their own to:
 * the session (and its plan) when every night image of the gesture belongs to
 * one, as the night wizard does; nothing otherwise -- each queued row carries
 * its own session, targets and plan (a plan's image says its plan itself). */
export function runTarget(
  sessions: readonly { sessionId: string; planId: string | null }[],
  used: readonly (string | null | undefined)[],
): { sessionId: string | null; planId: string | null; targetIds: string[] } {
  const ids = [...new Set(used.filter((id): id is string => !!id))];
  if (ids.length !== 1) return { sessionId: null, planId: null, targetIds: [] };
  const found = sessions.find((session) => session.sessionId === ids[0]);
  return { sessionId: ids[0], planId: found?.planId ?? null, targetIds: [] };
}

/* -------------------------------- Selectors ------------------------------ */

/** The uploads in progress or queued: what the badge counts. A file that
 * waits for the person (held) is not one of them. */
export function pendingCount(state: UploadsState): number {
  return state.rows.filter(
    (row) =>
      row.status === "queued" ||
      row.status === "uploading" ||
      row.status === "waiting",
  ).length;
}

/** Whether the queue of the linked space holds a file a run would send:
 * waiting (`queued`, `uploading`) or held for the person (a run releases
 * it). Otherwise « Tout envoyer » has nothing to start. */
export function hasSomethingToSend(entries: readonly QueueEntry[]): boolean {
  return entries.some(
    (entry) =>
      entry.state === "queued" ||
      entry.state === "uploading" ||
      entry.state === "held",
  );
}

/** Whether a file is still on its way (the live section of the page), or was
 * just found already sent (it stays there, with « Renvoyer »). */
export function isLive(row: UploadRow): boolean {
  return (
    Boolean(row.alreadySentAt) ||
    row.status === "queued" ||
    row.status === "uploading" ||
    row.status === "waiting" ||
    row.status === "held" ||
    (row.status === "failed" && row.retryable)
  );
}

export function liveRows(state: UploadsState): UploadRow[] {
  return state.rows.filter(isLive);
}

/** The uploads whose notification is still wanted (the snackbar queue shows
 * them one at a time). */
export function noticeRows(state: UploadsState): UploadRow[] {
  return state.rows.filter((row) => row.notice);
}

/** The files of the latest gesture of an origin (the result page's). */
export function latestBatch(
  state: UploadsState,
  origin: UploadOrigin,
): UploadRow[] {
  let batch = 0;
  for (const row of state.rows)
    if (row.origin === origin && row.batch > batch) batch = row.batch;
  return batch === 0
    ? []
    : state.rows.filter((row) => row.origin === origin && row.batch === batch);
}

export function isEnded(row: UploadRow): boolean {
  return ENDED.has(row.status);
}

/** How far a file is, 0 to 100 (whole numbers). */
export function percentOf(
  row: Pick<UploadRow, "sent" | "total" | "size">,
): number {
  const total = row.total || row.size;
  if (total <= 0) return 0;
  return Math.max(0, Math.min(100, Math.floor((row.sent / total) * 100)));
}

/** How far a group of files is: bytes sent over bytes to send. */
export function percentOfAll(
  rows: readonly Pick<UploadRow, "sent" | "total" | "size">[],
): number {
  let sent = 0;
  let total = 0;
  for (const row of rows) {
    sent += row.sent;
    total += row.total || row.size;
  }
  return total <= 0
    ? 0
    : Math.max(0, Math.min(100, Math.floor((sent / total) * 100)));
}

/* ---------------------------------- Words -------------------------------- */

/** « M 31 · SHO · 12,4 Mo », or the file's name when the gesture that queued
 * it said nothing of what it is. */
export function titleLine(
  t: Translate,
  locale: string,
  row: Pick<UploadRow, "target" | "rendering" | "fileName" | "size">,
): string {
  const parts = [row.target, row.rendering].filter(
    (part): part is string => !!part,
  );
  if (parts.length === 0) parts.push(row.fileName);
  if (row.size > 0) parts.push(formatBytes(t, row.size, locale));
  return parts.join(t("detect.join"));
}

/** « 3,1 Mo sur 25 Mo · 1,2 Mo/s » under the bar of a file on its way. */
export function progressLine(
  t: Translate,
  locale: string,
  row: Pick<UploadRow, "sent" | "total" | "size" | "speed">,
): string {
  const parts = [
    t("wizard.sending.overall", {
      sent: formatBytes(t, row.sent, locale),
      total: formatBytes(t, row.total || row.size, locale),
    }),
  ];
  if (row.speed !== null && row.speed > 0) {
    parts.push(
      t("uploads.row.speed", { speed: formatBytes(t, row.speed, locale) }),
    );
  }
  return parts.join(t("detect.join"));
}

/** Why a file was refused, held or failed, in the person's language. */
export function reasonOf(
  t: Translate,
  locale: string,
  row: Pick<UploadRow, "code" | "message" | "size" | "limit"> & {
    heldUntil?: string | null;
    path?: string | null;
  },
  now: number = Date.now(),
): string {
  if (row.code === "no_progress") return t("uploads.row.stalled");
  if (row.code === SOURCE_FILE_MISSING && row.path)
    return missingFileText(t, row.path);
  // A space that is read-only because a payment failed: the sentence of the
  // notice, with the day the window ends.
  const window = heldWindow(row.code, row.heldUntil ?? null, now);
  if (window) return afterColon(paymentFailedNotice(t, locale, window), locale);
  if (row.code === "file_too_large" && row.limit !== null && row.size > 0) {
    return t("uploads.row.tooLarge", {
      size: formatBytes(t, row.size, locale),
      limit: formatBytes(t, row.limit, locale),
    });
  }
  return reasonText(t, row.code ?? "", row.message ?? "");
}

/** The line that says where a file is, except while it is on its way (the
 * bar says that). */
export function stateLine(
  t: Translate,
  locale: string,
  row: UploadRow,
): string {
  switch (row.status) {
    case "queued":
      return t("uploads.row.queued");
    case "uploading":
      return t("status.uploading");
    case "waiting":
      return t("uploads.row.waiting", {
        wait: waitText(t, row.retryInSeconds ?? 1),
      });
    case "done":
      if (row.alreadySentAt)
        return t("uploads.row.alreadySent", {
          date: formatDateTime(row.alreadySentAt, locale),
        });
      return row.deduplicated
        ? t("uploads.row.deduplicated")
        : t("uploads.row.done");
    case "refused":
      return t("uploads.row.refused", { reason: reasonOf(t, locale, row) });
    case "held":
      return t("uploads.row.held", { reason: reasonOf(t, locale, row) });
    case "failed":
      return row.code === "network_error"
        ? t("uploads.row.network")
        : t("uploads.row.failed", { reason: reasonOf(t, locale, row) });
  }
}

/** A line about the sky annotations of an uploaded image. */
export interface SkyLine {
  tone: "neutral" | "success" | "warning";
  text: string;
  /** « Réessayer » is possible: the annotations are not there. */
  retry: boolean;
}

/** Whole seconds, at least one: « résolues en 4 s ». */
function skySeconds(annotation: SkyAnnotation): number {
  return Math.max(1, Math.round((annotation.elapsedMs ?? 0) / 1000));
}

/** Whether a sentence of `locale` goes on with a lowercase letter after a
 * colon: every language written in a cased script, except German, whose
 * nouns keep their capital (Japanese and Chinese have no case: the first
 * letter is not a Latin or Cyrillic capital, so nothing is lowered). */
function lowercasesAfterColon(locale: string, reason: string): boolean {
  const language = locale.split("-")[0].toLowerCase();
  if (language === "de") return false;
  return /^[\p{Script=Latin}\p{Script=Cyrillic}]/u.test(reason);
}

/** Product names that stay capitalised wherever they stand in a sentence. */
const KEPT_CAPITALS = new Set([
  "Astro",
  "Siril",
  "Python",
  "PixInsight",
  "Windows",
  "Mac",
  "macOS",
  "Linux",
  "Tycho",
]);

/**
 * A reason as it follows a colon (« Annotations du ciel non résolues :
 * trop peu d'étoiles… »): the first letter lowercased in the languages that
 * do, unless the first word is a name (« ASTAP », « Siril », « StarNet2 »).
 * The reasons are written as sentences; this is done once here, not in every
 * string.
 */
export function afterColon(reason: string, locale: string): string {
  if (!lowercasesAfterColon(locale, reason)) {
    return reason;
  }
  const first = reason.charAt(0);
  if (first === "" || first === first.toLowerCase()) return reason;
  const word = reason.split(/[\s’'.,;:!?]/)[0];
  // A name or an acronym: a capital or a digit after the first letter, or a
  // product name.
  if (/[\p{Lu}\d]/u.test(word.slice(1)) || KEPT_CAPITALS.has(word)) {
    return reason;
  }
  return first.toLowerCase() + reason.slice(1);
}

/** Why the annotations are not there, in the person's language: the solver's
 * failure (« Trop peu d'étoiles reconnues… »), or the service's code. Said
 * after a colon, so it starts in lowercase where the language does. */
export function skyReason(
  t: Translate,
  locale: string,
  annotation: SkyAnnotation,
): string {
  const reason = annotation.failure
    ? failureText(t, annotation.failure)
    : reasonText(t, annotation.errorCode ?? "", annotation.message ?? "");
  return afterColon(reason, locale);
}

/**
 * What became of the sky annotations of a file sent, as the « Envois » page
 * says it under the file: « Annotations du ciel : envoyées · résolues en 4 s »,
 * « … : déjà présentes », « … : solveur absent », « Annotations du ciel non
 * résolues : <raison> ». Only a failure offers « Réessayer ».
 */
export function skyLine(
  t: Translate,
  locale: string,
  annotation: SkyAnnotation,
): SkyLine {
  switch (annotation.state) {
    case "running":
      return { tone: "neutral", text: t("uploads.sky.running"), retry: false };
    case "sent":
      return {
        tone: "success",
        text:
          annotation.source === "header"
            ? t("uploads.sky.sentHeader")
            : annotation.elapsedMs === undefined
              ? t("uploads.sky.sent")
              : t("uploads.sky.sentIn", { seconds: skySeconds(annotation) }),
        retry: false,
      };
    case "already":
      return { tone: "success", text: t("uploads.sky.already"), retry: false };
    case "failed":
      return {
        tone: "warning",
        text:
          annotation.failure?.code === "solver_missing"
            ? t("uploads.sky.solverMissing")
            : t("uploads.sky.failed", {
                reason: skyReason(t, locale, annotation),
              }),
        retry: true,
      };
  }
}

export interface BannerLine {
  tone: "neutral" | "success" | "warning" | "error";
  text: string;
}

/** Counts of a group of files, by what they are doing. */
export function tally(rows: readonly UploadRow[]) {
  const active = rows.filter(
    (row) =>
      row.status === "queued" ||
      row.status === "uploading" ||
      row.status === "waiting",
  );
  return {
    active,
    started: active.filter((row) => row.status !== "queued" || row.sent > 0),
    done: rows.filter((row) => row.status === "done" && !row.alreadySentAt),
    /** Files the linked space already had: nothing left. */
    known: rows.filter((row) => Boolean(row.alreadySentAt)),
    refused: rows.filter((row) => row.status === "refused"),
    failed: rows.filter((row) => row.status === "failed"),
    held: rows.filter((row) => row.status === "held"),
  };
}

/** The truth about a group of files, one line per kind: « 1 image en cours
 * d'envoi · 12 % » while it leaves, then « 1 image envoyée » or « 1 image
 * refusée : … ». `reasons`: the reason of the first file that was set aside
 * is said (the banner), or not (the short state in the card). */
export function bannerLines(
  t: Translate,
  locale: string,
  rows: readonly UploadRow[],
  reasons = true,
  sky: Readonly<Record<string, SkyAnnotation>> = {},
): BannerLine[] {
  const group = tally(rows);
  const lines: BannerLine[] = [];
  if (group.active.length > 0) {
    lines.push(
      group.started.length > 0
        ? {
            tone: "neutral",
            text: pluralText(
              t,
              locale,
              "detect.result.active",
              group.active.length,
              { percent: percentOfAll(group.active) },
            ),
          }
        : {
            tone: "neutral",
            // The sentence of the queue ends with a full stop of its own.
            text: pluralText(
              t,
              locale,
              "detect.result.sent",
              group.active.length,
            ).replace(/[.。]$/, ""),
          },
    );
  }
  // The sky annotations of the files sent: « 1 image envoyée · annotations
  // du ciel envoyées », « … en cours », or a line of their own when they are
  // not there.
  const annotated = group.done.map((row) => sky[row.sha256]);
  const skyFailed = group.done.filter(
    (row) => sky[row.sha256]?.state === "failed",
  );
  if (group.done.length > 0) {
    let text = pluralText(t, locale, "detect.result.done", group.done.length);
    const settled = annotated.every(
      (item) => item?.state === "sent" || item?.state === "already",
    );
    const going = annotated.some((item) => item?.state === "running");
    if (settled) {
      text += t("detect.join") + t("detect.result.sky.sent");
    } else if (going && skyFailed.length === 0) {
      text += t("detect.join") + t("detect.result.sky.running");
    }
    lines.push({ tone: "success", text });
  }
  if (group.known.length > 0) {
    lines.push({
      tone: "neutral",
      text: pluralText(t, locale, "uploads.alreadySent", group.known.length),
    });
  }
  const said = (base: string, list: readonly UploadRow[]): string => {
    const state = pluralText(t, locale, base, list.length);
    return reasons
      ? t("detect.result.withReason", {
          state,
          reason: reasonOf(t, locale, list[0]),
        })
      : state;
  };
  if (group.refused.length > 0) {
    lines.push({
      tone: "error",
      text: said("detect.result.refused", group.refused),
    });
  }
  if (group.failed.length > 0) {
    lines.push({
      tone: "warning",
      text: pluralText(t, locale, "detect.result.failed", group.failed.length),
    });
  }
  if (group.held.length > 0) {
    lines.push({
      tone: "warning",
      text: said("detect.result.held", group.held),
    });
  }
  if (skyFailed.length > 0) {
    const state = pluralText(
      t,
      locale,
      "detect.result.sky.failed",
      skyFailed.length,
    );
    const first = sky[skyFailed[0].sha256];
    lines.push({
      tone: "warning",
      text: reasons
        ? t("detect.result.withReason", {
            state,
            reason: skyReason(t, locale, first),
          })
        : state,
    });
  }
  return lines;
}

/* ------------------------------ Notifications ----------------------------- */

/** The key of the snackbar that says the files sent (all of them in one). */
export const DONE_NOTICE_KEY = "upload:done";

/** The key of the snackbar that says the files the space already had. */
export const ALREADY_SENT_NOTICE_KEY = "upload:already-sent";

/** The key of the snackbar of one upload: on its way, refused, failed. */
export function noticeKey(sha256: string): string {
  return `upload:${sha256}`;
}

/** The tone of a notification: the edge and the mark. Never a primary
 * button: a notice offers links only. */
export function noticeTone(row: Pick<UploadRow, "status">): SnackTone {
  switch (row.status) {
    case "done":
      return "success";
    case "refused":
    case "failed":
      return "error";
    case "held":
    case "waiting":
      return "warning";
    default:
      return "info";
  }
}

/** What an upload's notifications may ask of the window. */
export interface NoticeHandlers {
  /** Opens the « Envois » page. */
  open: () => void;
  /** Tries a failed file again. */
  retry: (sha256: string) => void;
  /** The person closed the notification of these files (or its time ran
   * out): they stop being announced. Never stops an upload. */
  close: (sha256s: string[]) => void;
}

/**
 * The snackbars the uploads ask for, from the rows that still have their
 * notification: one for each file on its way (a progress bar, persistent --
 * the Snackbar shows it before the others), one for each file refused,
 * failed or held (persistent, with « Réessayer » when trying again is
 * possible), and ONE for all the files sent (timed, so twenty images do not
 * make twenty messages). Each carries its key: asking again with the same
 * state changes nothing on screen.
 */
export function uploadNotices(
  t: Translate,
  locale: string,
  state: UploadsState,
  handlers: NoticeHandlers,
): SnackInput[] {
  const notices: SnackInput[] = [];
  const open = { label: t("uploads.open"), run: handlers.open };
  const done = noticeRows(state).filter((row) => row.status === "done");
  const sent = done.filter((row) => !row.alreadySentAt);
  const known = done.filter((row) => Boolean(row.alreadySentAt));
  for (const row of noticeRows(state)) {
    const common = {
      key: noticeKey(row.sha256),
      tone: noticeTone(row),
      text: titleLine(t, locale, row),
      onClose: () => handlers.close([row.sha256]),
    };
    switch (row.status) {
      case "uploading":
        notices.push({
          ...common,
          mode: "persistent",
          progress: percentOf(row),
          detail: progressLine(t, locale, row),
          action: open,
        });
        break;
      case "waiting":
        notices.push({
          ...common,
          mode: "persistent",
          progress: percentOf(row),
          detail: stateLine(t, locale, row),
          action: open,
        });
        break;
      case "refused":
      case "failed":
      case "held":
        notices.push({
          ...common,
          mode: "persistent",
          detail: stateLine(t, locale, row),
          action:
            row.status === "failed" && row.retryable
              ? {
                  label: t("uploads.row.retry"),
                  run: () => handlers.retry(row.sha256),
                }
              : open,
        });
        break;
      default:
        break;
    }
  }
  if (sent.length > 0) {
    notices.push({
      key: DONE_NOTICE_KEY,
      tone: "success",
      mode: "timed",
      text:
        sent.length === 1
          ? titleLine(t, locale, sent[0])
          : pluralText(t, locale, "detect.result.done", sent.length),
      detail: sent.length === 1 ? stateLine(t, locale, sent[0]) : undefined,
      action: open,
      onClose: () => handlers.close(sent.map((row) => row.sha256)),
    });
  }
  // The files the linked space already had: one notice for all of them.
  if (known.length > 0) {
    notices.push({
      key: ALREADY_SENT_NOTICE_KEY,
      tone: "info",
      mode: "timed",
      text:
        known.length === 1
          ? titleLine(t, locale, known[0])
          : pluralText(t, locale, "uploads.alreadySent", known.length),
      detail: known.length === 1 ? stateLine(t, locale, known[0]) : undefined,
      action: open,
      onClose: () => handlers.close(known.map((row) => row.sha256)),
    });
  }
  return notices;
}
