// What the upload queue says about a file the service did not take, and about
// a run that has to wait or to stop -- pure TypeScript (no React, no Tauri),
// split out the same way `serviceWait.ts` and `space.ts` are, so it runs under
// `node --test`.
//
// Every refusal has its sentence, in the person's language, under
// `errors.<code>` (the codes are the contract's: `capture_content_refused`,
// `storage_quota_reached`, `upload_sessions_limit_reached`, ...). The core's
// own text, which every event also carries, is only the fallback for a code
// this window has no sentence for -- never the raw code, never the service's
// own English.

import type { QueueDriveEvent, QueueEntry } from "./bridge";
import type { TranslateParams } from "../i18n/locale";
import { waitText } from "./space.ts";
import { baseName, parentOf } from "./wizardSteps.ts";

type Translate = (key: string, params?: TranslateParams) => string;

/** The code of a file the queue names but that is no longer where it was
 * (`uploader::SOURCE_FILE_MISSING`). */
export const SOURCE_FILE_MISSING = "source_file_missing";

/** The sentence of `errors.<code>`; `fallback` (the core's own English text)
 * when this window has none. `params` are what the sentence names besides
 * the message. */
export function reasonText(
  t: Translate,
  code: string,
  fallback: string,
  params: TranslateParams = {},
): string {
  const key = `errors.${code}`;
  const text = t(key, { ...params, message: fallback });
  return text === key ? fallback : text;
}

/** « Fichier introuvable : <nom>, il était dans <dossier> »: the file a row
 * names and the folder it was in. */
export function missingFileText(t: Translate, path: string): string {
  return reasonText(t, SOURCE_FILE_MISSING, "", {
    file: baseName(path),
    folder: parentOf(path),
  });
}

/** The line under a file of the queue that was set aside, said in the
 * person's language: why the service refused it, or what it waits for.
 * `null` for a file that is on its way or done. */
export function rowReason(t: Translate, entry: QueueEntry): string | null {
  switch (entry.state) {
    case "refused":
    case "held":
      return entry.lastErrorCode === null
        ? entry.lastError
        : reasonText(t, entry.lastErrorCode, entry.lastError ?? "");
    case "failed":
      if (entry.lastErrorCode === SOURCE_FILE_MISSING)
        return missingFileText(t, entry.sourcePath);
      return entry.lastError === null
        ? null
        : t("errors.upload_error", { message: entry.lastError });
    case "completed":
      // The image replaced the one the capture had, the photo of the
      // session: the core marks the row `replaced`.
      return entry.lastErrorCode === "replaced"
        ? t("queue.replacedPhoto")
        : null;
    default:
      return null;
  }
}

export interface QueueNotice {
  tone: "neutral" | "success" | "warning" | "error";
  text: string;
}

/** The notice a refusal, a hold or a wait of the run puts above the queue;
 * `null` for an event that is not one of them. */
export function noticeOfSetAside(
  t: Translate,
  event: QueueDriveEvent,
): QueueNotice | null {
  switch (event.kind) {
    case "Refused":
      return {
        tone: "error",
        text: t("queue.refusedNotice", {
          file: event.sha256.slice(0, 12),
          reason: reasonText(t, event.code, event.message),
        }),
      };
    case "Held":
      return {
        tone: "warning",
        text: t("queue.heldNotice", {
          reason: reasonText(t, event.code, event.message),
        }),
      };
    case "Waiting":
      return {
        tone: "warning",
        text: t("queue.waitingNotice", {
          reason: reasonText(t, event.code, event.message),
          wait: waitText(t, event.retryInSeconds),
        }),
      };
    default:
      return null;
  }
}
