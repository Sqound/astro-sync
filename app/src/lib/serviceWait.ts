// The state behind the "the service is starting" banner -- pure TypeScript
// (no React, no Tauri), split out the same way `capabilities.ts` is so it
// runs under `node --test`.
//
// A server that has been idle can take a long time to answer the first
// request afterwards. While a call waits, the core
// (`astro_sync_core::service_wait`) reports it through the Tauri event
// `astro-sync://service-event` -- `{"kind":"starting"}` once the service has
// not answered for a few seconds, then `{"kind":"ready"}` when it answers or
// `{"kind":"stopped"}` when the wait ends without an answer (gave up, or the
// person cancelled). Events are balanced per call, and several calls can wait
// at once (the startup refresh and a screen's own request), so the banner
// counts them rather than toggling.

/** `astro_sync_core::service_wait::ServiceEvent`'s JSON. */
export type ServiceEvent =
  { kind: "starting" } | { kind: "ready" } | { kind: "stopped" };

/** The Tauri event name (`app/src-tauri/src/commands.rs`, `SERVICE_EVENT`). */
export const SERVICE_EVENT = "astro-sync://service-event";

export interface ServiceWaitState {
  /** Calls currently announced as waiting. */
  active: number;
  /** When the first of them began waiting (ms since the epoch), else null. */
  since: number | null;
}

export const IDLE: ServiceWaitState = { active: 0, since: null };

export function reduceServiceWait(
  state: ServiceWaitState,
  event: ServiceEvent,
  now: number,
): ServiceWaitState {
  if (event.kind === "starting") {
    return { active: state.active + 1, since: state.since ?? now };
  }
  const active = Math.max(0, state.active - 1);
  return active === 0 ? IDLE : { active, since: state.since };
}

export function isWaiting(state: ServiceWaitState): boolean {
  return state.active > 0;
}

/** Whole seconds since the wait began, never negative. */
export function elapsedSeconds(state: ServiceWaitState, now: number): number {
  return state.since === null
    ? 0
    : Math.max(0, Math.floor((now - state.since) / 1000));
}

/** The refusals the contract names beyond the sign-in, each with a sentence
 * of its own in every language: the catalogue of the core
 * (`astro_sync_core::refusal`). A test reads the Rust catalogue and fails when
 * the two differ, or when a language lacks a sentence. */
export const REFUSAL_CODES: readonly string[] = [
  "capture_content_refused",
  "capture_limit_reached",
  "storage_quota_reached",
  "upload_sessions_limit_reached",
  "file_too_large",
  "payload_too_large",
  "session_limit_reached",
  "session_event_limit_reached",
  "entitlement_read_only",
  "analysis_not_entitled",
  "place_not_found",
  "device_limit_reached",
  "upload_refused",
  "service_error",
  "capture_variant_limit_reached",
  "variant_capture_pending",
  "project_not_found",
  "variant_capture_refused",
  "not_a_result",
  "analysis_empty",
];

/** Failure codes that have a message of their own, in plain words, under
 * `errors.<code>` in every language -- shown instead of the technical text a
 * queue event also carries. (`cancelled` is not a failure: it has its own
 * `queue.stoppedCancelled` notice.) */
export const PLAIN_FAILURE_CODES: ReadonlySet<string> = new Set([
  "network_error",
  "service_start_timeout",
  "service_write_unconfirmed",
  // The service says this computer is no longer attached to its space: the
  // file is not at fault and stays queued.
  "device_detached",
  "space_deleted",
  "device_authentication_required",
  // The service is starting: nothing was checked, nothing was used.
  "service_starting",
  // Every refusal the contract names beyond the sign-in
  // (`astro_sync_core::refusal`).
  ...REFUSAL_CODES,
]);
