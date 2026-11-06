// A length of time in words for the sending's frames: « 47 s » under a minute,
// « 2 min 14 s » under an hour, « 1 h 02 min » after. The units come from the
// dictionary (`wizard.sending.duration*`), so each language writes its own;
// seconds (and minutes after an hour) keep two digits once there is a larger
// unit before them.

import type { Translate } from "../i18n/locale.ts";

const two = (value: number): string => String(value).padStart(2, "0");

export function formatDuration(t: Translate, milliseconds: number): string {
  const total = Math.max(
    0,
    Math.floor((Number.isFinite(milliseconds) ? milliseconds : 0) / 1000),
  );
  if (total < 60) {
    return t("wizard.sending.durationSeconds", { seconds: total });
  }
  if (total < 3600) {
    return t("wizard.sending.durationMinutes", {
      minutes: Math.floor(total / 60),
      seconds: two(total % 60),
    });
  }
  return t("wizard.sending.durationHours", {
    hours: Math.floor(total / 3600),
    minutes: two(Math.floor((total % 3600) / 60)),
  });
}
