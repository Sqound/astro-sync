// Pure capability logic -- no React, no Tauri -- split out from App.tsx and
// the screens the same way `i18n/locale.ts` is split from `i18n/index.tsx`:
// JSX cannot be type-stripped by `node --test`, so this file (and its own
// unit tests) stays plain TypeScript.
//
// Rule of the product: **the analysis of a night is local and free
// for everyone** -- the logs, the FWHM, the guiding, the progressions are
// computed on this computer, and the Progression and Analysis screens are
// always there, whatever the account holds. No capability gates their
// calculation or their display here.
//
// What `analysis.logs` governs is what the *service* keeps and shows: sending
// the report of an analysis to it, and its display -- the graphs, the
// progressions, the history on the portal. So it conditions one thing in this application: the **sending** of a
// report, and the sentence that says the analysis is here, on this computer,
// and that the display on the service is a separate feature.
//
// A profile that is not known (a fresh pairing whose first refresh failed
// before any cache existed) holds nothing back: the service refuses a report
// itself (`analysis_not_entitled`) when its account does not include it, and
// the screen says so then.

import type { CapabilitiesState } from "./bridge";

/** `true` only when the paired service says, in its profile, that it does not
 * keep the reports of an analysis (`analysis.logs` is exactly `false`). Not
 * paired, or a profile not known yet: `false` -- nothing is held back, the
 * service answers when a report is sent. */
export function isAnalysisSendingHeld(
  state: CapabilitiesState | null | undefined,
): boolean {
  return !!state?.paired && state.capabilities?.["analysis.logs"] === false;
}

/** Which sentence says that the analysis is here and free while its sending
 * is held: the hosted service keeps a display of it (`hosted`), a private server
 * simply does not keep the reports (`private`). */
export type AnalysisSendingNotice = "hosted" | "private";

/** The sentence to show while the sending of a report is held, or `null`
 * when nothing is held back. */
export function analysisSendingNotice(
  state: CapabilitiesState | null | undefined,
  hosted: boolean,
): AnalysisSendingNotice | null {
  if (!isAnalysisSendingHeld(state)) {
    return null;
  }
  return hosted ? "hosted" : "private";
}

/** The dictionary key of the sentence of [`analysisSendingNotice`]. */
export function analysisSendingNoticeKey(
  notice: AnalysisSendingNotice,
): "analysis.sendingHeldHosted" | "analysis.sendingHeldPrivate" {
  return notice === "hosted"
    ? "analysis.sendingHeldHosted"
    : "analysis.sendingHeldPrivate";
}

/** Whether a new reading of the capabilities differs from the one held: the
 * person's offer changed (an offer bought on the web, or ended). Both must be
 * known profiles: the first reading is never a change. */
export function offerChanged(
  before: CapabilitiesState | null | undefined,
  after: CapabilitiesState | null | undefined,
): boolean {
  const was = before?.capabilities;
  const now = after?.capabilities;
  if (!was || !now) return false;
  return (Object.keys(now) as (keyof typeof now)[]).some(
    (key) => was[key] !== now[key],
  );
}

/** The shortest time between two readings made on their own (the window
 * coming back to the front): coming and going between two windows does not
 * ask the service every time. */
export const CAPABILITIES_REFRESH_GAP_MS = 5000;
