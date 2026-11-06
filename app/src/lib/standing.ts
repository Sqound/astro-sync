// A payment that failed, said by Astro Sync.
//
// The rule: a subscription whose payment failed does not take the space away.
// Until the end of the window the service reports, the space is
// **read-only** -- it keeps everything, exports everything, and accepts
// nothing new -- then it follows the ordinary end of a subscription. Astro
// Sync sells and counts nothing; it says that the space is read-only and
// until when, where the state of the space is shown (« Espace Astro Portal », the card of « Envois »), before
// the last step of a night or of an import, and as the reason a file is held.
//
// Pure TypeScript (no React, no Tauri), so it runs under `node --test`. The
// core reads the state from `GET /v1/entitlements` and keeps it beside the
// capabilities (`astro_sync_core::standing`); it never expires it: a window
// that is over is told by the clock here (`paymentFailedWindow`).

import type { CapabilitiesState, Standing } from "./bridge";
import type { TranslateParams } from "../i18n/locale";

type Translate = (key: string, params?: TranslateParams) => string;

/** The code of the refusal an upload meets in a read-only space (`403`). */
export const READ_ONLY_CODE = "entitlement_read_only";

/** The window of a payment that failed, while it lasts. */
export interface PaymentFailedWindow {
  /** The end of the read-only window, as the service wrote it. */
  until: string;
  /** The same instant, in milliseconds since the epoch. */
  untilMs: number;
}

/** The standing as the interface reads it: `null` when the payment did not
 * fail, when the state is not known, when `readOnlyUntil` is not an instant,
 * or when the window is over at `now` (the space is free again, or the
 * ordinary end of a subscription says what remains). */
export function paymentFailedWindow(
  standing: Standing | null | undefined,
  now: number,
): PaymentFailedWindow | null {
  if (!standing || standing.kind !== "paymentFailed") return null;
  const untilMs = Date.parse(standing.readOnlyUntil);
  if (Number.isNaN(untilMs) || untilMs <= now) return null;
  return { until: standing.readOnlyUntil, untilMs };
}

/** The window of the state `capabilities()` gave (`null` while it is not
 * known, or unpaired). */
export function windowOfState(
  state: CapabilitiesState | null | undefined,
  now: number,
): PaymentFailedWindow | null {
  return state?.paired ? paymentFailedWindow(state.standing, now) : null;
}

/** « 29 décembre 2026 »: the day the window ends, on this computer's
 * calendar, in the language's own order. */
export function untilText(
  locale: string,
  window: Pick<PaymentFailedWindow, "untilMs">,
): string {
  return new Intl.DateTimeFormat(locale, {
    year: "numeric",
    month: "long",
    day: "numeric",
  }).format(new Date(window.untilMs));
}

/** « Paiement en échec · lecture seule jusqu'au 29 décembre 2026 »: the state
 * in one line, where the state of the space is shown. */
export function paymentFailedLine(
  t: Translate,
  locale: string,
  window: PaymentFailedWindow,
): string {
  return t("standing.paymentFailed.line", { date: untilText(locale, window) });
}

/** The sentence of the notice: the payment did not go through, the space is
 * read-only until the day, then free again, and how to get the rights back at
 * once. */
export function paymentFailedNotice(
  t: Translate,
  locale: string,
  window: PaymentFailedWindow,
): string {
  return t("standing.paymentFailed.notice", {
    date: untilText(locale, window),
  });
}

/** « Votre espace est en lecture seule jusqu'au 29 décembre 2026 : rien ne
 * sera envoyé. » -- said before the last step of a night or of an import. */
export function beforeSendingText(
  t: Translate,
  locale: string,
  window: PaymentFailedWindow,
): string {
  return t("standing.paymentFailed.beforeSending", {
    date: untilText(locale, window),
  });
}

/** The window a file held in a read-only space is explained by: the window
 * of the payment that failed (`heldUntil`, the end the row was held under)
 * while it lasts at `now`; `null` when the ordinary sentence of the refusal
 * (« l'abonnement a pris fin ») is the one that tells it. */
export function heldWindow(
  code: string | null,
  heldUntil: string | null,
  now: number,
): PaymentFailedWindow | null {
  if (code !== READ_ONLY_CODE || heldUntil === null) return null;
  return paymentFailedWindow(
    { kind: "paymentFailed", since: heldUntil, readOnlyUntil: heldUntil },
    now,
  );
}
