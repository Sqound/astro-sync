// Small, dependency-free formatting helpers shared by the screens. Every
// error message is rendered through the active
// locale's `t()` rather than hardcoded here, so this file only ever picks
// *which* translation key and *what* numbers to interpolate.

import { isCommandError } from "./bridge";
import { progressionSpaceText, stackingFailureText } from "./wizardSummary";
import type { TranslateParams } from "../i18n";

type Translate = (key: string, params?: TranslateParams) => string;

// Dates, hours and byte counts are written by `./written.ts` (pure, tested).
export { formatBytes, formatDateTime, formatDay, oneDecimal } from "./written";

/** Turns any failure caught from a bridge call into a localized message:
 * a `CommandError`'s `code` is looked up as `errors.<code>` (with its
 * `message` available as `{message}`), and anything else falls back to
 * `errors.unknown_error`. */
export function formatError(t: Translate, error: unknown): string {
  if (isCommandError(error)) {
    if (error.code === "stacking_failed" && error.details) {
      return stackingFailureText(t, error.details);
    }
    if (
      error.code === "progression_failed" &&
      error.details?.why === "not_enough_space"
    ) {
      return progressionSpaceText(t, error.details);
    }
    return t(`errors.${error.code}`, { message: error.message });
  }
  const message = error instanceof Error ? error.message : String(error);
  return t("errors.unknown_error", { message });
}
