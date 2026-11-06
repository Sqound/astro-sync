// The only file that calls `@tauri-apps/api`'s `invoke`/`listen` directly.
// Every screen imports typed functions from here instead -- this is the
// TypeScript mirror of `astro-sync-core`'s own serde shapes and
// `app/src-tauri/src/commands.rs`'s command surface, and nothing here
// recomputes or reinterprets a value the core already produced: the GUI
// "displays the core's own JSON", it does not add logic to it.
//
// Field names are camelCase throughout because every Rust struct that
// crosses this boundary is `#[serde(rename_all = "camelCase")]` (or, for
// the plain enums, `kebab-case`/`snake_case` string variants, called out
// per type below).

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  FamiliesPlan,
  PoseEvent,
  PoseMeasure,
  PoseStats,
} from "./poseStats.ts";
import { openUrl } from "@tauri-apps/plugin-opener";

import { SERVICE_EVENT, type ServiceEvent } from "./serviceWait";
import { planFromCore, planToCore, type CorePlan } from "./imagePlans";
import { contrastToCore, type Contrast } from "./contrast";
import type { Colour, SoftStars } from "./finish";

/** Every command's `Err` payload (`app/src-tauri/src/error.rs`). `code` is
 * the stable key the UI looks up in `errors.<code>` of the active locale;
 * `message` is the core's own (English) `Display` text, shown verbatim only
 * as a fallback for a portal-specific refusal reason. */
export interface CommandError {
  code: string;
  message: string;
  /** What a screen needs beside the code, when there is something: the
   * `attemptsRemaining` of a wrong sign-in code, the `retryAt` /
   * `retryAfterSeconds` of a wait. Absent for every other error. */
  details?: Record<string, unknown>;
}

export function isCommandError(value: unknown): value is CommandError {
  return (
    typeof value === "object" &&
    value !== null &&
    "code" in value &&
    "message" in value &&
    typeof (value as { code: unknown }).code === "string"
  );
}

/* -------------------------------- Build info ----------------------------- */

/** The build of this application: the version of `tauri.conf.json`, the
 * 9-character commit built (`unknown` outside a repository) and the build
 * instant, ISO 8601 in UTC (empty when unknown). Stamped at build time. */
export interface BuildInfo {
  version: string;
  gitSha: string;
  buildDate: string;
}

export function appBuildInfo(): Promise<BuildInfo> {
  return invoke<BuildInfo>("app_build_info");
}

/* --------------------------------- Settings ------------------------------ */

export interface GuiSettings {
  portalUrl: string | null;
  deviceLabel: string | null;
  locale: string | null;
}

export function settingsGet(): Promise<GuiSettings> {
  return invoke<GuiSettings>("settings_get");
}

export function settingsSet(settings: GuiSettings): Promise<GuiSettings> {
  return invoke<GuiSettings>("settings_set", { settings });
}

export interface DeviceStatus {
  paired: boolean;
  deviceLabel: string | null;
  portalUrl: string | null;
}

export function deviceStatus(): Promise<DeviceStatus> {
  return invoke<DeviceStatus>("device_status");
}

export function devicePair(
  portalUrl: string,
  code: string,
  deviceLabel: string,
): Promise<GuiSettings> {
  return invoke<GuiSettings>("device_pair", { portalUrl, code, deviceLabel });
}

export function deviceUnpair(): Promise<void> {
  return invoke<void>("device_unpair");
}

/* ------------------------------ The hosted space -------------------------- */
// Astro Sync signs in to the hosted service with an e-mail address, then the
// code of six digits received by e-mail, as the application `sync`. Every shape below is `astro_sync_core::space`'s own
// `Serialize` output; a private server keeps `devicePair`. The token never
// crosses this boundary.

/** What `GET /v1/service` says about how a person signs in
 * (`astro_sync_core::space::SignInAnnouncement`). */
export type SignInAnnouncement =
  | {
      method: "email-code";
      codeLength: number;
      /** `open`: the hosted service, an address without a space gets one.
       * `closed`: a private server signing in by e-mail code, only its
       * owner's address opens it -- the words of a portal, not of a space. */
      spaceCreation: "open" | "closed";
      /** The terms of use, once the service has them configured. */
      termsUrl: string | null;
      /** The privacy policy, once the service has it configured. */
      privacyUrl: string | null;
      /** Additive: the address template (`https://.../{lang}/terms/`) behind
       * `termsUrl`, when the service names a page per language. The screens
       * replace `{lang}` with the tag of the language they are shown in
       * (`legalAddress` of `space.ts`); `termsUrl` is the English page. */
      termsUrlTemplate: string | null;
      privacyUrlTemplate: string | null;
    }
  /** A private server: the code it shows. Also what an absent announcement
   * means. */
  | { method: "pairing" }
  | { method: "none" }
  /** A method newer than this Astro Sync. */
  | { method: "unsupported"; announced: string };

export interface SpaceInfo {
  id: string;
  email: string;
  /** The address of the space on the web. */
  webUrl: string;
}

export interface RegisteredDevice {
  id: string;
  name: string;
}

/** Why the service ended a sign-in: this computer was detached from its
 * space, the space was deleted, or a newer sign-in of this installation
 * replaced the token (`device_authentication_required`). */
export type SignInEnd =
  "device_detached" | "space_deleted" | "connection_replaced";

/** A sign-in the service ended, remembered for the sign-in screen. It holds
 * the reason and the moment, nothing of the space: what this computer knew of
 * it is forgotten. */
export interface SignInEndedNotice {
  reason: SignInEnd;
  at: string;
}

export interface SpaceStatus {
  /** The service in force: the saved address, else the hosted service. */
  serviceUrl: string;
  hosted: boolean;
  /** A token is kept for that service. */
  paired: boolean;
  /** Signed in by e-mail code: the space and the name of this computer. */
  space: SpaceInfo | null;
  device: RegisteredDevice | null;
  ended: SignInEndedNotice | null;
  /** The name a new sign-in registers this computer under. */
  defaultDeviceName: string;
}

export interface CodeRequested {
  /** The address as the service normalised it. */
  email: string;
  codeLength: number;
  expiresAt: string;
  expiresInSeconds: number;
  resendAt: string;
  resendAfterSeconds: number;
}

export interface SignInOutcome {
  /** This sign-in created the space (« Votre espace est prêt »). */
  created: boolean;
  status: SpaceStatus;
}

export interface RenameOutcome {
  status: SpaceStatus;
  /** The service listed the new name; `false` when it does not answer that
   * route yet, and the name is kept on this computer only. */
  serviceUpdated: boolean;
}

export interface DetachOutcome {
  status: SpaceStatus;
  /** The service knows this computer left; `false` when it could not be
   * told (the computer is detached here all the same). */
  serviceNotified: boolean;
}

/** What the service in front of us announces; `portalUrl` names a private
 * server, without it the hosted service. */
export function serviceAnnouncement(
  portalUrl?: string | null,
): Promise<SignInAnnouncement> {
  return invoke<SignInAnnouncement>("service_announcement", {
    portalUrl: portalUrl ?? null,
  });
}

/** The connection, from what is on disk (no network). */
export function spaceStatus(): Promise<SpaceStatus> {
  return invoke<SpaceStatus>("space_status");
}

export function spaceRequestCode(
  portalUrl: string | null,
  email: string,
  language: string,
): Promise<CodeRequested> {
  return invoke<CodeRequested>("space_request_code", {
    portalUrl,
    email,
    language,
  });
}

export function spaceVerifyCode(
  portalUrl: string | null,
  email: string,
  code: string,
): Promise<SignInOutcome> {
  return invoke<SignInOutcome>("space_verify_code", {
    portalUrl,
    email,
    code,
  });
}

/** Asks the service who this computer is and remembers the answer: one
 * short, silent attempt, so what is on disk answers when the service does
 * not. */
export function spaceRefresh(): Promise<SpaceStatus> {
  return invoke<SpaceStatus>("space_refresh");
}

export function spaceRename(name: string): Promise<RenameOutcome> {
  return invoke<RenameOutcome>("space_rename", { name });
}

export function spaceDetach(): Promise<DetachOutcome> {
  return invoke<DetachOutcome>("space_detach");
}

/** The Tauri event of a sign-in the service ended. */
export const SPACE_EVENT = "astro-sync://space-event";

export type SpaceEvent = { kind: "signed_out"; reason: SignInEnd };

export function onSpaceEvent(
  handler: (event: SpaceEvent) => void,
): Promise<UnlistenFn> {
  return listen<SpaceEvent>(SPACE_EVENT, (event) => handler(event.payload));
}

/** Sets the contrast of presentation of every render, night and plan (a gain
 * from 0 to 5; the core reads the two values the window always offered as
 * `presentation` and `strong`); the stacking reads it when it runs. */
export function localSettingsSetContrast(
  contrast: Contrast,
): Promise<LocalSettings> {
  return invoke<LocalSettings>("local_settings_set_contrast", {
    contrast: contrastToCore(contrast),
  });
}

/** Sets the colour intensity of the finish of every render, night and plan (a
 * percent from 0 to 100); the stacking reads it when it runs. */
export function localSettingsSetColour(colour: Colour): Promise<LocalSettings> {
  return invoke<LocalSettings>("local_settings_set_colour", { colour });
}

/** Sets the softened stars of the finish of every render, night and plan (a
 * percent from 0 to 100); the stacking reads it when it runs. */
export function localSettingsSetSoftStars(
  softStars: SoftStars,
): Promise<LocalSettings> {
  return invoke<LocalSettings>("local_settings_set_soft_stars", { softStars });
}

/** Sets the engine of the « Rendu soigné »; the stacking reads it
 * when it runs. */
export function localSettingsSetPolishedEngine(
  engine: PolishedEngine,
): Promise<LocalSettings> {
  return invoke<LocalSettings>("local_settings_set_polished_engine", {
    engine,
  });
}

/** « Image du plan multi-nuits »: the frame the image of a multi-night plan
 * is cut to; read when the image of a plan is made. */
export function localSettingsSetPlanField(
  field: PlanField,
): Promise<LocalSettings> {
  return invoke<LocalSettings>("local_settings_set_plan_field", { field });
}

/** `plan_image::PlanField`: « Bords nets » (the largest rectangle without a
 * pixel no night covers, the default), « Zone commune à toutes les nuits »,
 * « Tout le champ ». */
export type PlanField = "clean" | "common" | "whole";

/** « Ouvrir le dossier des pipelines »: the folder of the person's
 * own definitions, created when missing, opened by the system. */
export function pipelinesFolderOpen(): Promise<void> {
  return invoke<void>("pipelines_folder_open");
}

/** Opens a web address in the system browser (only `http(s)` addresses the
 * service or the space announced). */
export function openInBrowser(url: string): Promise<void> {
  return openUrl(url);
}

/* ------------------------------ Capabilities ------------------------------ */
// `GET /v1/capabilities`, refreshed and cached by the core
// (`astro_sync_core::gui_bridge::refresh_capabilities`) so this resolves
// instantly from the paired portal's last known profile even offline.

/** `astro_sync_core::gui_bridge::Capabilities` -- the portal's neutral
 * capability map, dotted keys verbatim (never renamed to camelCase: these
 * are the exact contract field names). The only field this GUI reads today is
 * `"analysis.logs"`, which no longer gates any screen: the analysis is local
 * and free for everyone. It only says whether the service keeps and shows the
 * reports Astro Sync sends it (`lib/capabilities.ts`). */
export interface Capabilities {
  "planner.multiple": boolean;
  "planner.mosaic": boolean;
  "pointing.solve": boolean;
  "sync.plan_limit": number;
  "gallery.private": boolean;
  "portal.invitations": boolean;
  "capture.variant_limit": number;
  "capture.count_limit": number;
  "capture.public": boolean;
  "analysis.logs": boolean;
  "analysis.progression": boolean;
  "storage.max_file_bytes": number;
  "storage.quota_bytes": number;
  "history.retention_days": number;
}

/** `astro_sync_core::standing::Standing` -- how the billing of the space
 * limits what Astro Sync may send. A payment that failed keeps the space
 * read-only for 90 days: everything stays and is exported, nothing new is
 * accepted. The dates are the service's, as it wrote them: `since` the
 * day the hold started, `readOnlyUntil` the end of the window
 * (`lib/standing.ts` compares it with the clock). */
export type Standing = {
  kind: "paymentFailed";
  since: string;
  readOnlyUntil: string;
};

export interface CapabilitiesState {
  /** `false` when no device is paired at all: there is no service to send
   * an analysis to yet. */
  paired: boolean;
  /** `null` when not paired, or when paired but no profile could be
   * determined yet (a fresh pairing whose first refresh failed before any
   * cache existed). An unknown profile holds nothing back: the service
   * decides when a report is sent. */
  capabilities: Capabilities | null;
  /** `GET /v1/entitlements`, read with the profile and cached beside it:
   * `{ kind: "paymentFailed", ... }` while a payment that failed keeps the
   * space read-only. Absent (or `null`) when nothing limits the space, and
   * from a core older than the field. */
  standing?: Standing | null;
}

export function capabilities(): Promise<CapabilitiesState> {
  return invoke<CapabilitiesState>("capabilities");
}

/* --------------------------------- Sessions ------------------------------ */

export type SessionStatus = "open" | "paused" | "finished";

/** How a night turned out, read by the service from the debriefing of the
 * session's plan (`session.schema.json`, `outcome`). */
export type SessionOutcome = "executed" | "partial" | "abandoned";

export interface SessionSummary {
  id: string;
  planId: string | null;
  status: SessionStatus;
  startedAt: string;
  finishedAt: string | null;
  createdAt: string;
  updatedAt: string;
  eventCount: number;
  supplementCount: number;
  pendingSupplementCount: number;
  /** The night's date (`AAAA-MM-JJ`), as the service defines it: the plan's
   * date, else the evening of the start at the place's longitude. Absent from
   * a service of an earlier version. */
  eveningDate?: string | null;
  /** `null` (or absent, a service of an earlier version) while the session
   * says nothing: not debriefed yet, or no plan. */
  outcome?: SessionOutcome | null;
}

export interface SessionListPage {
  sessions: SessionSummary[];
  nextSince: string | null;
  hasMore: boolean;
}

export function listSessions(
  since?: string | null,
  limit?: number | null,
): Promise<SessionListPage> {
  return invoke<SessionListPage>("list_sessions", {
    since: since ?? null,
    limit: limit ?? null,
  });
}

/* ------------------------------- Folder picker ---------------------------- */

/** Opens the native folder picker, on `start` when it is given and still
 * there (the last folder chosen); `null` means the user dismissed it. */
export function chooseFolder(start?: string | null): Promise<string | null> {
  return invoke<string | null>("choose_folder", { start: start ?? null });
}

/* --------------------------- Scan / dry-run / package --------------------- */

/** `astro_sync_core::scanner::FileClass` (`kebab-case`). */
export type FileClass =
  "image" | "raw" | "video" | "log" | "fits" | "xisf" | "other";

export interface ScannedFile {
  relativePath: string;
  class: FileClass;
  bytes: number;
  sha256: string;
}

/** `astro_sync_core::scanner::LinkIssue`: a symbolic link the walk could
 * not follow. */
export interface LinkIssue {
  relativePath: string;
  problem: "missing-target" | "loop";
  target: string | null;
}

export interface ScanReport {
  root: string;
  files: ScannedFile[];
  /** Absent when every link was followed. */
  links?: LinkIssue[];
}

export function scanFolder(folder: string): Promise<ScanReport> {
  return invoke<ScanReport>("scan_folder", { folder });
}

/** `astro_sync_core::package::AssetKind` (`kebab-case`). */
export type AssetKind =
  "image" | "metric-log" | "event-log" | "progression" | "other";

export interface DryRunEntry {
  sourceRelativePath: string;
  class: FileClass;
  assetKind: AssetKind;
  packageRelativePath: string;
  bytes: number;
  sha256: string;
}

export interface DryRunReport {
  sourceRoot: string;
  entries: DryRunEntry[];
  totalBytes: number;
}

export function dryRunFolder(folder: string): Promise<DryRunReport> {
  return invoke<DryRunReport>("dry_run_folder", { folder });
}

export interface PackageSessionInput {
  label?: string | null;
  startedAt?: string | null;
  finishedAt?: string | null;
}

export interface PackageOutcome {
  outputDir: string;
  manifestPath: string;
  checksumsPath: string;
  assetCount: number;
  totalBytes: number;
}

export function producePackage(
  folder: string,
  outputParent: string,
  session?: PackageSessionInput | null,
): Promise<PackageOutcome> {
  return invoke<PackageOutcome>("produce_package", {
    folder,
    outputParent,
    session: session ?? null,
  });
}

/* ------------------------------------ Queue ------------------------------- */

/** `astro_sync_core::upload_queue::QueueState` (`snake_case`). `refused`: the
 * service refuses the file for good (it is never sent again); `held`: the
 * person has to act first (storage full, subscription ended), and the session
 * already open is kept. */
export type QueueState =
  "queued" | "uploading" | "completed" | "failed" | "refused" | "held";

export interface QueueEntry {
  id: number;
  sourcePath: string;
  sha256: string;
  size: number;
  uploadId: string | null;
  offset: number;
  state: QueueState;
  captureId: string | null;
  /** The technical text of the failure: never shown as it is. */
  lastError: string | null;
  /** The stable code of the refusal or the hold that put the file aside
   * (`capture_content_refused`, `storage_quota_reached`, ...): looked up as
   * `errors.<code>`, in the person's language. */
  lastErrorCode: string | null;
  createdAt: string;
  updatedAt: string;
  /** Only results enter the queue; `plan`: the image of a multi-night plan,
   * a capture of the plan. */
  kind: "final" | "stacked" | "progression" | "plan";
  /** What the upload is attributed to: the night, its targets, the capture
   * it is a variant of. */
  target: {
    sessionId: string | null;
    targetIds: string[];
    variantOf: string | null;
    source: string | null;
    /** The image of a multi-night plan: its plan. */
    plan?: { projectId: string; combinedNightIds: string[] } | null;
  };
}

export interface QueueSummary {
  total: number;
  queued: number;
  uploading: number;
  completed: number;
  failed: number;
  refused: number;
  held: number;
  totalBytes: number;
  sentBytes: number;
}

export interface QueueStatusPayload {
  /** The queue of the linked space only. */
  entries: QueueEntry[];
  summary: QueueSummary;
  /** What became of the sky annotations of the files sent. */
  sky: SkyAnnotation[];
  /** The space the queue is seen from (`null`: not linked to one). */
  spaceId?: string | null;
  /** The rows written before the queue knew its spaces (« Avant la
   * liaison »), while a space is linked. */
  beforeLink?: QueueEntry[];
}

/** `sky_after_upload::SkyAnnotation`: where the sky annotations of an
 * uploaded image stand. `failure` says why the solver gave nothing;
 * `errorCode` why the service could not be asked or told (`errors.<code>`). */
export interface SkyAnnotation {
  sha256: string;
  captureId: string | null;
  state: "running" | "sent" | "already" | "failed";
  failure?: SolveFailure;
  errorCode?: string;
  message?: string;
  elapsedMs?: number;
  source?: "header" | "astap" | "astrometry-net";
  updatedAt: string;
}

export const SKY_EVENT = "astro-sync://sky-event";

/** The annotations of an uploaded image changed (started, sent, failed). */
export function onSkyEvent(
  handler: (annotation: SkyAnnotation) => void,
): Promise<UnlistenFn> {
  return listen<SkyAnnotation>(SKY_EVENT, (event) => handler(event.payload));
}

/** « Réessayer » of the annotations of an uploaded image: the answer is the
 * `astro-sync://sky-event` that follows. */
export function skyRetry(sha256: string): Promise<void> {
  return invoke<void>("sky_retry", { sha256 });
}

export interface EnqueueOutcome {
  inserted: number;
  /** Files the queue of the linked space already holds, in any state. */
  alreadyKnown: number;
  /** Frames, calibration and raw logs left out: they stay on this computer. */
  leftOut: number;
  /** Among them, the files already sent to the linked space: the page says
   * « Déjà envoyé le … » at once instead of « En file d'envoi ». */
  alreadySent?: AlreadySent[];
}

/** `upload_queue::AlreadySent`: a file enqueued again that the linked space
 * already has. */
export interface AlreadySent {
  sha256: string;
  captureId: string | null;
  /** When it was sent (ISO 8601 UTC). */
  sentAt: string;
}

export function enqueueFolder(folder: string): Promise<EnqueueOutcome> {
  return invoke<EnqueueOutcome>("enqueue_folder", { folder });
}

export function queueStatus(): Promise<QueueStatusPayload> {
  return invoke<QueueStatusPayload>("queue_status");
}

export interface UploadOutcomePayload {
  captureId: string;
  deduplicated: boolean;
}

export interface UploadTargetInput {
  sessionId?: string | null;
  planId?: string | null;
  targetIds: string[];
}

export function uploadNext(
  target: UploadTargetInput,
): Promise<UploadOutcomePayload | null> {
  return invoke<UploadOutcomePayload | null>("upload_next", {
    sessionId: target.sessionId ?? null,
    planId: target.planId ?? null,
    targetIds: target.targetIds,
  });
}

export function uploadAll(target: UploadTargetInput): Promise<number> {
  return invoke<number>("upload_all", {
    sessionId: target.sessionId ?? null,
    planId: target.planId ?? null,
    targetIds: target.targetIds,
  });
}

export function uploadPause(): Promise<void> {
  return invoke<void>("upload_pause");
}

export function uploadResume(): Promise<void> {
  return invoke<void>("upload_resume");
}

/** « Réessayer » on a file the queue could not find: it goes back in the
 * queue (one file, or every such file with `null`); how many went back. */
export function uploadRequeueMissing(sha256: string | null): Promise<number> {
  return invoke<number>("upload_requeue_missing", { sha256 });
}

/** « Renvoyer » a file already sent to the linked space: it goes back in the
 * queue (`true`), the next run sends it. */
export function uploadResend(sha256: string): Promise<boolean> {
  return invoke<boolean>("upload_resend", { sha256 });
}

/** « Effacer l'historique », for what came before the link: those rows are
 * deleted. Returns how many. */
export function queueForgetBeforeLink(): Promise<number> {
  return invoke<number>("queue_forget_before_link");
}

/** `astro_sync_core::gui_bridge::QueueDriveEvent`, tagged on `kind`. Emitted
 * under `UPLOAD_EVENT` by both `upload_next` and `upload_all`. */
export type QueueDriveEvent =
  | { kind: "Started"; sha256: string; sourcePath: string; bytes: number }
  | { kind: "Progress"; sha256: string; sent: number; total: number }
  | {
      kind: "Completed";
      sha256: string;
      captureId: string;
      deduplicated: boolean;
      /** What the service left of the proposed metadata (the owner's
       * own values); absent when everything was applied. */
      skipped?: { field: string; reason: string }[];
      /** The image replaced the one its capture had (the photo
       * of the session); absent when it did not. */
      replaced?: boolean;
    }
  | {
      kind: "Failed";
      sha256: string;
      /** A stable code: `network_error`, `service_start_timeout`, ... --
       * looked up as `errors.<code>` instead of showing `message`. */
      code: string;
      /** The core's own text, in English: a fallback for a code this window
       * has no sentence for. */
      message: string;
    }
  /** The service refuses this file for good: it is set aside as `refused`,
   * never sent again, the file on the computer is untouched, and the run
   * goes on with the next one. */
  | { kind: "Refused"; sha256: string; code: string; message: string }
  /** The person has to act (storage full, subscription ended): the file is
   * `held` with what it already sent, and the run stops (`Stopped`,
   * `held`). */
  | { kind: "Held"; sha256: string; code: string; message: string }
  /** Not the file's fault, and it will pass (a network that is down, a
   * service that fails, too many uploads open in the space): the file stays
   * queued and the run tries it again by itself in `retryInSeconds`, waiting
   * longer each time (`attempt`). */
  | {
      kind: "Waiting";
      sha256: string;
      code: string;
      message: string;
      retryInSeconds: number;
      attempt: number;
    }
  | {
      kind: "Stopped";
      sha256: string | null;
      reason: "queue_empty" | "paused" | "no_progress" | "cancelled" | "held";
    };

export const UPLOAD_EVENT = "astro-sync://upload-event";

export function onUploadEvent(
  handler: (event: QueueDriveEvent) => void,
): Promise<UnlistenFn> {
  return listen<QueueDriveEvent>(UPLOAD_EVENT, (event) =>
    handler(event.payload),
  );
}

/* ------------------------------ Hosted service state ---------------------- */
// A server that has been idle can take a long time to answer the first
// request afterwards. While a call waits, the core reports
// it (`astro_sync_core::service_wait`) and the banner says so; "Annuler"
// ends the wait. See `./serviceWait.ts`.

export { SERVICE_EVENT, type ServiceEvent };

export function onServiceEvent(
  handler: (event: ServiceEvent) => void,
): Promise<UnlistenFn> {
  return listen<ServiceEvent>(SERVICE_EVENT, (event) => handler(event.payload));
}

/** Ends every wait in progress; the calls that were waiting reject with the
 * `cancelled` code. */
export function serviceCancel(): Promise<void> {
  return invoke<void>("service_cancel");
}

/* ------------------------------- Progression ------------------------------ */

/** `astro_sync_core::analysis::fits::AcquisitionFamily`. */
export interface AcquisitionFamily {
  key: string;
  filter: string | null;
  exposureSeconds: number | null;
  gain: number | null;
  binning: string | null;
  temperatureClassC: number | null;
  framePaths: string[];
}

/* ------------------------------- Analysis --------------------------------- */
// `astro_sync_core::analysis::report::SessionReport`, the report v3, mirrored
// field-for-field (see this file's own top comment) -- the exact JSON
// `POST /v1/sessions/{id}/analyses`
// (`schemas/api/session-analysis.schema.json` of the contract,
// `$defs.report`) accepts as its body. It replaces the night's analysis (one
// per night); an empty report is refused before it leaves (`analysis_empty`).

export interface TimelineBoundary {
  timestampUtc: string;
  evidence: string;
}

export interface AnalysisSessionEvent {
  type: string;
  timestampUtc: string;
  source: "phd2" | "asiair" | "nina";
  data: Record<string, unknown>;
}

export interface AnalysisTimeline {
  startedAt: TimelineBoundary | null;
  endedAt: TimelineBoundary | null;
  events: AnalysisSessionEvent[];
}

export interface AnalysisSourceCounts {
  phd2FilesParsed: number;
  asiairFilesParsed: number;
  fitsFilesParsed: number;
  ninaFilesParsed: number;
}

export interface AcquisitionStep {
  key: string;
  targetHint: string | null;
  filter: string | null;
  frameCount: number;
  exposureSeconds: number;
  gain: number | null;
  binning: string | null;
  sensorTempC: number | null;
}

export interface GuidingSlice {
  start: string;
  end: string;
  rmsRa: number | null;
  rmsDec: number | null;
  rmsTotal: number | null;
  samples: number;
}

export interface QualityRow {
  timestampUtc: string;
  stepKey: string | null;
  filter: string | null;
  exposureSeconds: number | null;
  gain: number | null;
  iso: number | null;
  binning: string | null;
  sensorTempC: number | null;
  rmsTotalArcsec: number | null;
  rmsRaArcsec: number | null;
  rmsDecArcsec: number | null;
  guidingSampleCount: number | null;
  skyBackgroundAdu: number | null;
  fwhmPixels: number | null;
  eccentricity: number | null;
  starCount: number | null;
  /** The stacking left the frame out, and why. */
  rejected: boolean;
  rejectReason: string | null;
  sourcePath: string;
}

/** `guiding_trace::TracePoint`: one window of 10 s of the night's guiding. */
export interface TracePoint {
  t: number;
  n: number;
  out: number;
  ra: number;
  dec: number;
  raPeak: number;
  decPeak: number;
  raPulseMs: number | null;
  decPulseMs: number | null;
}

export interface TraceMarker {
  type:
    "dither" | "settle" | "star-lost" | "pause" | "meridian-flip" | "autofocus";
  at: string;
  end: string | null;
}

/** `guiding_trace::GuidingTrace`: the guiding reduced (report v3). */
export interface GuidingTrace {
  intervalSeconds: number;
  startUtc: string;
  scaleArcsecPerPx: number | null;
  guidedSeconds: number;
  samples: number;
  rms: {
    raArcsec: number | null;
    decArcsec: number | null;
    totalArcsec: number | null;
    samples: number;
  };
  points: TracePoint[];
  markers: TraceMarker[];
}

/** `report::LogClock`: the clocks the logs were read with. */
export interface LogClock {
  phd2: string | null;
  asiair: string | null;
  nina: string | null;
  detected: boolean;
  phd2From: "given" | "asiair" | "frames" | null;
}

export interface SessionReport {
  reportVersion: 3;
  producer: "astro-sync";
  generatedAtUtc: string;
  source: AnalysisSourceCounts;
  pixelScaleArcsec: number | null;
  timeline: AnalysisTimeline;
  acquisitionSteps: AcquisitionStep[];
  quality: QualityRow[];
  /** Absent without a guiding log. */
  guidingTrace?: GuidingTrace;
  guidingSeries: GuidingSlice[];
  logClock?: LogClock;
}

export interface UploadedAnalysis {
  analysis: {
    id: string;
    sessionId: string;
    reportVersion: number;
    createdAt: string;
    originDeviceId: string | null;
    deviceName: string | null;
  };
  /** The night had an analysis, now replaced (one per night). */
  replaced: boolean;
  steps: { proposed: number; keptEdited: number };
}

/* ------------------------- The measured sky background ------------------- */
// `astro_sync_core::analysis::sky_night`'s own shapes.

export interface SkySite {
  latitude: number;
  longitude: number;
}

/** One night's value: what goes up to the service. */
export interface SkyNight {
  night: string;
  sqm: number;
  poseCount: number;
  methodVersion: string;
  observedAt: string;
  site: SkySite;
}

export interface PlaceSkyMeasure {
  placeId: string;
  night: string;
  sqm: number;
  poseCount: number;
  methodVersion: string;
  observedAt: string;
  sessionId: string | null;
  updatedAt: string;
}

export interface UploadedSkyMeasure {
  measure: PlaceSkyMeasure;
  replaced: boolean;
}

/** Fetches the Tycho-2 reference list (about 25 MB once made) and keeps it
 * on this computer. Called only by the « Installer » of the Tycho-2 row of
 * Réglages › Dépendances, whose line names the list, its size and the CDS. */
export function downloadReferenceStars(): Promise<{ stars: number }> {
  return invoke<{ stars: number }>("download_reference_stars");
}

/* ------------------------- The journey of a night ------------------------ */
// The commands of `app/src-tauri/src/commands_night.rs` (part 3, and
// `night_detail`, `night_create` and `folder_open`). Every shape below is the
// core's serde shape as it is.

/** `working_folder::NightFolderEntry`. */
export interface NightFolderEntry {
  nightId: string;
  folder: string;
  name: string;
}

export function nightFoldersList(): Promise<NightFolderEntry[]> {
  return invoke<NightFolderEntry[]>("night_folders_list");
}

/** `commands_night::WorkingFolderState`. */
export interface WorkingFolderState {
  root: string;
  chosen: boolean;
  exists: boolean;
}

export function workingFolderGet(): Promise<WorkingFolderState> {
  return invoke<WorkingFolderState>("working_folder_get");
}

/** `folder_files::ManagedFile`: a `.md` file Astro Sync writes at the root of
 * the working folder (`README.md`, `GUIDE-ASTRO-SYNC.<language>.md`). */
export interface ManagedFile {
  name: string;
  path: string;
  /** `folder_files::ManagedOutcome` (snake_case). */
  outcome: "created" | "rewritten" | "unchanged" | "kept_as_changed";
  sha256: string | null;
  /** The size on disk, in bytes. */
  bytes: number;
  /** The number of characters of Astro Sync's text. */
  chars: number;
}

/** `commands_night::PreparedRoot`. */
export interface PreparedRoot {
  root: string;
  created: boolean;
  assistantFile: {
    path: string;
    /** `assistant_prompt::AssistantFileOutcome` (snake_case). */
    outcome: "created" | "rewritten" | "unchanged" | "kept_as_changed";
  };
  /** `README.md` in the application's language; none when no language was
   * given or the file could not be made ready. */
  readme?: ManagedFile | null;
}

/** Makes the working folder ready: the root created when absent, then
 * `ASSISTANT-IA.md` written under its rule (an earlier text of Astro Sync is
 * replaced, a file the person changed is kept). Called at startup and before
 * every copy of the prompt for an AI assistant. */
export function workingFolderPrepare(language?: string): Promise<PreparedRoot> {
  return invoke<PreparedRoot>("working_folder_prepare", { language });
}

/** « Utiliser ce dossier » (Réglages › Général): an existing folder becomes
 * the working folder; nothing is moved from the old one. */
export function workingFolderSet(
  folder: string,
  language?: string,
): Promise<PreparedRoot> {
  return invoke<PreparedRoot>("working_folder_set", { folder, language });
}

/** « Poser une question à une IA »: the Guide page in Markdown is written at
 * the root of the working folder as `GUIDE-ASTRO-SYNC.<language>.md`, under
 * the rule of `ASSISTANT-IA.md`. */
export function guideWrite(
  language: string,
  text: string,
): Promise<{ root: string; guide: ManagedFile }> {
  return invoke("guide_write", { language, text });
}

/** « Remplacer par la version d'Astro Sync » for the guide or the README:
 * the person's file is renamed `<name>.modifie-<date>.md` beside it, then
 * Astro Sync's text is written. Nothing is deleted. */
export function managedFileReplace(
  kind: "guide" | "readme",
  language: string,
  text?: string,
): Promise<{ keptCopy: string | null; file: ManagedFile }> {
  return invoke("managed_file_replace", { kind, language, text });
}

/** `guide_prompt::GuidePromptInput`. */
export interface GuidePromptInput {
  root: string;
  /** The guide's file name at the root; none when it could not be written. */
  guideFile: string | null;
}

/** « Poser une question à une IA »: the prompt in the chosen language. */
export function guidePrompt(
  language: string,
  input: GuidePromptInput,
): Promise<string> {
  return invoke<string>("guide_prompt", { language, input });
}

/** « Remplacer par la version d'Astro Sync »: the person's `ASSISTANT-IA.md`
 * is renamed `ASSISTANT-IA.modifie-<date>.md` beside it, then today's text is
 * written. Nothing is deleted. */
export function workingFolderReplaceAssistantFile(): Promise<{
  keptCopy: string | null;
}> {
  return invoke("working_folder_replace_assistant_file");
}

/** `local_settings::LocalSettings`, the fields the window reads. */
export interface LocalSettings {
  workingFolder: string | null;
  phd2UtcOffset: string | null;
  asiairUtcOffset: string | null;
  ninaUtcOffset: string | null;
  /** The contrast of presentation of every render: a gain, or one of the
   * words `none`, `presentation`, `strong` (read with `contrastOf`). */
  contrast?: number | string;
  /** The colour intensity of the finish of every render, in percent. */
  colour?: number | string;
  /** The softened stars of the finish of every render, in percent. */
  softStars?: number | string;
  /** The engine of the « Rendu soigné » (Réglages › Général). */
  polishedEngine?: PolishedEngine;
  /** « Image du plan multi-nuits » (Réglages › Général): the frame the image
   * of a plan is cut to; `clean` by default. */
  planField?: PlanField;
  /** The last folders chosen. */
  lastFolders?: LastFolders;
  /** Whether this installation reports its errors to the hosted service
   * (`error_report`). Not shown in Réglages yet; off by default until it is. */
  reportErrors?: boolean;
}

/** `local_settings::LastFolders`: the folders the pickers open on. */
export interface LastFolders {
  fits?: string | null;
  calibration?: string | null;
  logs?: string | null;
  sessionFits?: string | null;
  sessionLogs?: string | null;
}

/** Keeps the last folders chosen; a `null` one is forgotten. */
export function localSettingsSetLastFolders(
  folders: LastFolders,
): Promise<LocalSettings> {
  return invoke<LocalSettings>("local_settings_set_last_folders", { folders });
}

/** `stacking::PolishedEngine`: GraXpert's AI models, or Siril alone. */
export type PolishedEngine = "graxpert-ai" | "siril";

/** Hands an uncaught error of the window to the core, which cleans it and
 * queues it where the reporting is allowed (`lib/errorReport.ts`). */
export function errorReportRecord(
  message: string,
  stack: string | null,
): Promise<void> {
  return invoke<void>("error_report_record", { message, stack });
}

export function localSettingsGet(): Promise<LocalSettings> {
  return invoke<LocalSettings>("local_settings_get");
}

/** The clocks of the PHD2, ASIAIR and NINA logs (`+HH:MM`, `null` for UTC);
 * a bad offset is refused with `bad_utc_offset` and nothing is kept. */
export function localSettingsSetUtcOffsets(offsets: {
  phd2UtcOffset: string | null;
  asiairUtcOffset: string | null;
  ninaUtcOffset: string | null;
}): Promise<LocalSettings> {
  return invoke<LocalSettings>("local_settings_set_utc_offsets", offsets);
}

/** Stops the local work in progress (a detection, a resolution, a download). */
export function localWorkCancel(): Promise<void> {
  return invoke<void>("local_work_cancel");
}

/** `local_work::WorkKind` (kebab-case). */
export type WorkKind =
  | "scan"
  | "headers"
  | "analysis"
  | "stacking"
  | "progression"
  | "organise"
  | "pose-stats";

/** « Arrêter l'empilement »: stops the works of one kind in progress (the
 * stacking), the others (the progression images, the report of the logs) go
 * on. */
export function localWorkCancelKind(kind: WorkKind): Promise<void> {
  return invoke<void>("local_work_cancel_kind", { kind });
}

/** `local_work::WorkProgress`, emitted on `astro-sync://local-work`: the
 * report of the logs says which file it reads (`kind` `analysis`). */
export interface WorkProgress {
  kind: WorkKind;
  done: number;
  total: number;
  bytesDone: number;
  bytesTotal: number;
  /** Forward-slash path, relative to the folder being read. */
  current: string | null;
}

export const LOCAL_WORK_EVENT = "astro-sync://local-work";

export function onLocalWork(
  handler: (progress: WorkProgress) => void,
): Promise<UnlistenFn> {
  return listen<WorkProgress>(LOCAL_WORK_EVENT, (event) =>
    handler(event.payload),
  );
}

/** `working_folder::NightLocation` (tag `state`, kebab-case). */
export type NightLocation =
  | { state: "found"; folder: string }
  | { state: "duplicated"; folders: string[] }
  | { state: "missing" };

/** `working_folder::NightTargetSpec`. */
export interface NightTargetSpec {
  targetId: string;
  designation: string;
  ticked: boolean;
}

/** `working_folder::NightFolderSpec`. */
export interface NightFolderSpec {
  nightId: string;
  serviceUrl: string;
  eveningDate: string;
  label: string;
  targets: NightTargetSpec[];
  follow: string | null;
}

/** `working_folder::NightFolderOutcome`. */
export interface NightFolderOutcome {
  folder: string;
  created: boolean;
  createdPaths: string[];
  targets: {
    targetId: string;
    designation: string;
    folderName: string;
    state: "created" | "existing" | "out-of-night";
  }[];
  applied: boolean;
}

/** « Retrouver le dossier »: a folder the person chose, which must carry
 * the night (`night_found_again`); refused with `night_folder_without_identity`
 * or `night_folder_other_night` otherwise. */
export function nightFoundAgain(
  nightId: string,
  folder: string,
): Promise<string> {
  return invoke<string>("night_found_again", { nightId, folder });
}

/** « Suivre ce dossier »: two folders carry the night, this one is followed
 * from now on by every step (`night_follow`). */
export function nightFollow(nightId: string, folder: string): Promise<string> {
  return invoke<string>("night_follow", { nightId, folder });
}

export function nightFolderPlan(
  spec: NightFolderSpec,
): Promise<NightFolderOutcome> {
  return invoke<NightFolderOutcome>("night_folder_plan", { spec });
}

export function nightFolderPrepare(
  spec: NightFolderSpec,
): Promise<NightFolderOutcome> {
  return invoke<NightFolderOutcome>("night_folder_prepare", { spec });
}

/** `detection::DetectionTarget`. */
export interface DetectionTarget {
  targetId: string;
  designation: string;
  aliases: string[];
  ticked: boolean;
  onlineFinal: boolean;
  stackingPlanned: boolean;
}

/** `detection::DetectionInput`. The night's window runs from local noon of
 * `eveningDate` to local noon of the next day, local noon drawn from the
 * place's `longitude` (degrees east; none: UTC) -- never from the start
 * typed. */
export interface DetectionInput {
  eveningDate: string | null;
  nightStartedAt: string | null;
  longitude?: number | null;
  targets: DetectionTarget[];
}

/** `detection::Finding`: a rule of the detection, its parameters. */
export interface Finding {
  key: string;
  severity: "info" | "warning" | "error";
  targetId: string | null;
  params: Record<
    string,
    | string
    | number
    | boolean
    | null
    | { exposureSeconds: number | null; gain: number | null }[]
  >;
  files: string[];
}

/** `detection::TargetDetection`. */
export interface TargetDetection {
  targetId: string;
  designation: string;
  folderName: string | null;
  inNight: boolean;
  lightFrames: number;
  filters: string[];
  finalImage: string | null;
  otherFinalImages: string[];
  minimum: "final-file" | "online" | "stacking" | "missing";
  finalBytes: number | null;
  lightSummary: FrameSummary;
  /** The images a device already stacked, found in `device/`; absent when
   * there is none. */
  device?: DeviceDetection;
}

/** `device_master::DeviceImage`: a stacked image of a known device (the
 * Seestar's `Stacked_*.fit`). */
export interface DeviceImage {
  /** Relative to the night's folder. */
  relativePath: string;
  fileName: string;
  bytes: number;
  kind: "seestar";
  /** « Seestar S50 Pro ». */
  device: string;
  /** The poses it stacked (`STACKCNT`). */
  poses: number;
  exposureSeconds: number | null;
  /** The exposure it holds in all (`TOTALEXP`), seconds. */
  totalSeconds: number | null;
  filter: string | null;
  gain: number | null;
  width: number;
  height: number;
  /** The header carries a field solution. */
  solved: boolean;
  /** The label of its family (`IRCUT_30s_g200`). */
  family: string;
}

/** `device_master::DeviceFamily`: a family that has device images, what
 * covers it and what is taken by default. */
export interface DeviceFamily {
  label: string;
  filter: string | null;
  exposureSeconds: number | null;
  gain: number | null;
  /** The poses of the family in `lights/` (0: not on this computer). */
  lights: number;
  /** The file names of its images, the biggest first. */
  images: string[];
  /** One image covers it, or two or more split it. */
  coverage: "covers" | "split";
  defaultSource: "device" | "restack";
  /** The image the device choice takes (`null`: unavailable). */
  useFile: string | null;
  /** The poses of the images the device choice leaves aside. */
  asidePoses: number;
}

/** `detection::DeviceDetection`. */
export interface DeviceDetection {
  images: DeviceImage[];
  families: DeviceFamily[];
  /** The `.jpg` of `device/` with no recognised `.fit` of its name. */
  jpgs: string[];
}

/** `detection::FrameSummary`: « 40 poses · 180 s · Ha 20, OIII 20 · gain
 * 100 · 1,3 Go ». */
export interface FrameSummary {
  filters: { filter: string | null; frames: number }[];
  exposures: number[];
  gains: number[];
  bytes: number;
}

/** `detection::DetectedLog`: a file of `logs/`, its reader, its size. */
export interface DetectedLog {
  relativePath: string;
  kind: "nina" | "phd2" | "asiair" | "unknown";
  bytes: number;
}

/** `detection::NightDetection`. */
export interface NightDetection {
  folder: string;
  fingerprint: string;
  targets: TargetDetection[];
  logs: DetectedLog[];
  calibration: {
    darks: number;
    flats: number;
    darkFlats: number;
    biases: number;
  };
  calibrationSummary: { darks: FrameSummary; flats: FrameSummary };
  findings: Finding[];
  outsideStructure: string[];
  links: LinkIssue[];
}

export function nightDetect(
  folder: string,
  input: DetectionInput,
): Promise<NightDetection> {
  return invoke<NightDetection>("night_detect", { folder, input });
}

/** `night_online::OnlineImage`. */
export interface OnlineImage {
  id: string;
  main: boolean;
  sha256: string;
  byteSize: number;
  originalName: string;
  source: string | null;
  createdAt: string;
  skySolution: string;
}

/** `night_online::OnlineCapture`. */
export interface OnlineCapture {
  id: string;
  targetIds: string[];
  source: string | null;
  stage: string;
  visibility: string;
  uploaded: boolean;
  createdAt: string;
  mainImage: OnlineImage | null;
  imageCount: number;
  images: OnlineImage[];
}

/** `night_online::NightOnline`. */
export interface NightOnline {
  sessionId: string;
  targets: {
    targetId: string | null;
    name: string | null;
    planned: boolean;
    captures: OnlineCapture[];
  }[];
  analysis: {
    present: boolean;
    sentAt: string | null;
    originDeviceId: string | null;
    deviceName: string | null;
    reportVersion: number | null;
    producer: string | null;
    stepCount: number;
    editedStepCount: number;
  };
}

/** `night_resume::WizardStep`. */
export type WizardStep =
  "cibles" | "dossier" | "fichiers" | "traitements" | "recapitulatif";

/** `night_resume::Missing` (tag `key`). */
export type Missing =
  | { key: "noTarget" }
  | { key: "folder"; state: "found" | "duplicated" | "missing" }
  | { key: "targetFolder"; targetId: string }
  | { key: "finalImage"; targetId: string; name: string }
  | { key: "logsAdded" }
  | { key: "analysis" }
  | { key: "skyAnnotations"; count: number }
  | { key: "progression"; targetId: string }
  | { key: "imagesToSend"; count: number }
  | { key: "queuePending"; count: number };

/** `night_resume::NightResume`. */
export interface NightResume {
  status: "not_started" | "to_complete" | "complete";
  step: WizardStep | null;
  missing: Missing[];
  analysis:
    | { kind: "nothing" }
    | { kind: "send" }
    | { kind: "replace"; previousSentAt: string }
    | { kind: "up_to_date" };
  skySolutions: unknown[];
}

/** `precheck::Treatment`. */
export type Treatment =
  "stacking" | "final_image" | "sky_annotations" | "logs" | "progression";

/** `precheck::TreatmentState`. */
export interface TreatmentState {
  treatment: Treatment;
  available: boolean;
  unavailable:
    | { why: "not_in_subscription"; capability: string }
    | { why: "tool_missing"; tool: string }
    | { why: "nothing_to_treat" }
    | null;
}

/** `commands_night::NightView`: a night's page. */
export interface NightView {
  location: NightLocation;
  /** This computer has a trace of a folder for the night (a sending from
   * here, a folder found again or followed). */
  folderTrace: boolean;
  detection: NightDetection | null;
  online: NightOnline;
  resume: NightResume;
  treatments: TreatmentState[];
  /** The masters kept in the night's folder. */
  integrations?: IntegrationMaster[];
}

export function nightOnline(sessionId: string): Promise<NightOnline> {
  return invoke<NightOnline>("night_online", { sessionId });
}

export function nightView(
  sessionId: string,
  input: DetectionInput,
): Promise<NightView> {
  return invoke<NightView>("night_view", { sessionId, input });
}

/** `GET /v1/sessions/{id}` as the service wrote it (`session.schema.json`,
 * `detail`): the window reads only `session`, `night` and `debrief`. */
export interface NightDetail {
  session: SessionSummary;
  night?: {
    title: { value: string | null };
    place: {
      value: {
        name: string | null;
        latitude?: number | null;
        longitude?: number | null;
      } | null;
    };
    targetIds: { value: string[] | null };
  };
  debrief?: {
    targets?: {
      targetId: string;
      state?: "done" | "skipped" | "modified" | "partial";
      acquiredMinutes?: number | null;
    }[];
  } | null;
}

export function nightDetail(sessionId: string): Promise<NightDetail> {
  return invoke<NightDetail>("night_detail", { sessionId });
}

/** `night_service::NightCreate`: the « Créer une nuit » window's fields. */
export interface NightCreate {
  eveningDate: string;
  start: string | null;
  end: string | null;
  place: string;
  title: string | null;
}

export function nightCreate(night: NightCreate): Promise<NightDetail> {
  return invoke<NightDetail>("night_create", { night });
}

/** Opens an existing folder in the Finder, the Explorer or the file manager. */
export function folderOpen(folder: string): Promise<void> {
  return invoke<void>("folder_open", { folder });
}

/** `assistant_prompt::PromptInput`. */
export interface PromptInput {
  root: string;
  nightFolder: string;
  targets: string[];
  focus?: string | null;
  messages?: string[];
  /** The common names by target folder. */
  names?: Record<string, string>;
}

/** « Demander à un assistant IA »: the prompt in the window's language. */
export function assistantPrompt(
  language: string,
  input: PromptInput,
): Promise<string> {
  return invoke<string>("assistant_prompt", { language, input });
}

/** `precheck::ImageCandidate`. */
export interface ImageCandidate {
  targetId: string;
  path: string;
  sha256: string;
  size: number;
  kind: "final" | "stacked" | "progression";
  render: string | null;
  /** A progression image: the light frames its milestone stacks. */
  frameCount?: number | null;
}

/** `precheck::Decision` (tag `decision`). A progression image is the
 * history of its target's capture -- online (`progression`) or the one
 * this sending creates (`progression_of_new`) --, never a variant. */
export type Decision =
  | { decision: "already_online"; captureId: string; imageId: string }
  | { decision: "new_capture" }
  | { decision: "variant"; captureId: string }
  | { decision: "variant_of_new"; firstSha256: string }
  | { decision: "progression"; captureId: string }
  | { decision: "progression_of_new"; firstSha256: string }
  | {
      decision: "not_sent";
      reason:
        | "variants_not_allowed"
        | "replaced_by_stacking"
        | "one_render_only"
        | "other_final_image"
        | "no_capture_for_progression"
        | "too_large_for_space";
    };

/** `metadata::Proposals`: what Astro Sync proposes for an image and its
 * capture. The window passes the plans back as it got them; nothing here is
 * shown or edited. */
export interface CaptureProposals {
  acquisition?: {
    filter: string | null;
    frameCount: number;
    exposureSeconds: number;
    gain: number | null;
  }[];
  calibration?: string;
  sensor?: { temperatureC?: number; binning?: number };
  processing?: { software?: string; steps?: string };
  /** The image's generated title and description, in the ten languages
   * (keyed by tag, English always there): the service shows each reader its
   * own, apart from the title the owner types. */
  variantGeneratedTitle?: Record<string, string>;
  variantGeneratedDescription?: Record<string, string>;
  /** The quality of the stacked image, as Astro Sync measured it on its
   * linear master; absent without `analysis.logs`, on an image processed
   * elsewhere and under twenty stars. */
  imageQuality?: ImageQuality;
}

/** `metadata::ImageQuality` (`capture.schema.json` `imageQuality`). */
export interface ImageQuality {
  fwhmPixels: number;
  /** `null` when the pixel scale of the frames is not known. */
  fwhmArcsec: number | null;
  /** `null` when not measured. */
  eccentricity: number | null;
  starCount: number;
  measuredOn: "linear";
  measuredAt: string;
  tool: { name: string; version: string };
}

/** `precheck::ImagePlan`. */
export interface ImagePlan {
  image: ImageCandidate;
  decision: Decision;
  metadata?: CaptureProposals;
  /** The render started from the image a device stacked. */
  fromDevice?: boolean;
}

/** `precheck::QuotaFinding` (tag `limit`). */
export type QuotaFinding =
  | {
      limit: "storage_quota";
      toSendBytes: number;
      remainingBytes: number;
      quotaBytes: number;
    }
  | {
      limit: "file_too_large";
      path: string;
      size: number;
      maxFileBytes: number;
    }
  | {
      limit: "variant_limit";
      targetId: string;
      captureId: string | null;
      existing: number;
      adding: number;
      variantLimit: number;
    }
  | {
      limit: "capture_count";
      adding: number;
      captureCount: number;
      captureCountLimit: number;
    }
  | { limit: "refused"; path: string; code: string };

/** `precheck::QuotaCheck`. */
export interface QuotaCheck {
  allowed: boolean;
  toSendBytes: number;
  toSendFiles: number;
  newCaptures: number;
  findings: QuotaFinding[];
}

/** `precheck::SpaceStorage`. */
export interface SpaceStorage {
  usedBytes: number;
  quotaBytes: number;
  remainingBytes: number;
  maxFileBytes: number;
  captureCount: number;
  captureCountLimit: number;
  variantLimit: number;
  capabilities: Capabilities;
}

/** `night_steps::NightPrecheck`: « Récapitulatif », nothing sent. */
export interface NightPrecheck {
  storage: SpaceStorage;
  plans: ImagePlan[];
  quota: QuotaCheck;
  severalRendersAllowed: boolean;
}

export function nightPrecheck(
  sessionId: string,
  input: DetectionInput,
  extra: ImageCandidate[],
  /** The language of the window, for the title of a rendering proposed. */
  language?: string,
  /** The targets about to be stacked again: the stacked images an earlier
   * stacking left in `stacked/` are replaced by it. */
  restack?: string[],
): Promise<NightPrecheck> {
  return invoke<NightPrecheck>("night_precheck", {
    sessionId,
    input,
    extra,
    language: language ?? null,
    restack: restack ?? [],
  }).then((check) => ({
    ...check,
    // The core flattens each decision into its plan.
    plans: (check.plans as unknown as CorePlan[]).map(planFromCore),
  }));
}

/** `commands_night::EnqueueAnswer`. */
export type EnqueueAnswer =
  | {
      result: "enqueued";
      outcome: EnqueueOutcome;
      /** The proposals for the images already online. */
      proposals: {
        sha256: string;
        captureId: string;
        skipped: { field: string; reason: string }[];
        note: string | null;
        refused: string | null;
      }[];
      proposalsError: string | null;
    }
  | { result: "refused"; findings: QuotaFinding[] };

export function nightEnqueue(
  sessionId: string,
  plans: ImagePlan[],
  quota: QuotaCheck,
): Promise<EnqueueAnswer> {
  return invoke<EnqueueAnswer>("night_enqueue", {
    sessionId,
    plans: plans.map(planToCore),
    quota,
  });
}

/** The sky solutions of the night's images online, solved here and sent. */
export function nightSolve(
  sessionId: string,
  input: DetectionInput,
): Promise<unknown[]> {
  return invoke<unknown[]>("night_solve", {
    sessionId,
    input,
    choice: null,
    hints: null,
  });
}

/** `log_offset::LogsUtcOffset`: the clocks the logs were read with. `detected`
 * is true when Astro Sync found at least one from the frames' `DATE-OBS`; the
 * offsets are `+HH:MM`, `null` for a kind of log the night does not hold. */
export interface LogsUtcOffset {
  detected: boolean;
  phd2: string | null;
  asiair: string | null;
  nina: string | null;
  frames: number;
  phd2MatchedFrames: number | null;
  asiairMatchedFrames: number | null;
  ninaMatchedFrames: number | null;
}

/** `night_analysis::NightAnalysis`: what a night's folder gives. */
export interface NightAnalysis {
  report: SessionReport;
  skyMeasure: SkyNight | null;
  skyMissingReason: string | null;
  logsUtcOffset: LogsUtcOffset | null;
}

/** « Journaux »: the report of a night's logs, read here (nothing sent). */
export function nightAnalysis(folder: string): Promise<NightAnalysis> {
  return invoke<NightAnalysis>("night_analysis", { folder });
}

/** `night_analysis::NightAnalysisSent`. */
export interface NightAnalysisSent {
  analysis: UploadedAnalysis | null;
  skyMeasure:
    | { kind: "sent"; stored: UploadedSkyMeasure }
    | { kind: "refused"; code: string }
    | { kind: "not_measured" };
  logsUtcOffset: LogsUtcOffset | null;
}

/** « Analyser et envoyer le rapport »: the report replaces the night's
 * analysis; the raw logs stay. */
export function nightSendLogs(
  sessionId: string,
  folder: string,
): Promise<NightAnalysisSent> {
  return invoke<NightAnalysisSent>("night_send_logs", { sessionId, folder });
}

/* ------------------------------ Sky solutions ----------------------------- */

/** `sky_solution::ListedTarget`. */
export interface ListedTarget {
  id: string;
  designation: string | null;
  name: string | null;
  raHours: number | null;
  decDegrees: number | null;
}

/** `sky_solution::MissingCapture`: a capture whose main image has no sky
 * solution, from the phone, the web or a night. */
export interface MissingCapture {
  id: string;
  sessionId: string | null;
  originalName: string;
  sha256: string;
  byteSize: number;
  mainImageId: string | null;
  mainImageSource: string | null;
  nightDate: string | null;
  galleryLabel: string | null;
  thumbnail: boolean | null;
  targets: ListedTarget[];
}

/** « Annotations »: the captures of the space without a sky solution. */
export function skyMissing(): Promise<MissingCapture[]> {
  return invoke<MissingCapture[]>("sky_missing");
}

/** `sky_solution::SolveFailure` (tag `code`). */
export interface SolveFailure {
  code:
    | "solver_missing"
    | "unreadable_image"
    | "too_few_stars"
    | "not_solved"
    | "outside_database"
    | "no_database"
    | "no_index"
    | "timeout"
    | "cancelled"
    | "failed";
  seconds?: number;
  detail?: string;
}

/** `sky_solution::SolveOutcome` (tag `outcome`). */
export type SolveOutcome =
  | { outcome: "sent"; captureId: string }
  | {
      outcome: "failed";
      captureId: string;
      failure: SolveFailure;
      message: string;
    };

/** « Résoudre et envoyer »: from the night's file of the same digest when
 * one is known (`here`: SHA-256 → path), else from the original. */
export function skySolveCapture(
  capture: MissingCapture,
  here: Record<string, string> | null,
): Promise<SolveOutcome> {
  return invoke<SolveOutcome>("sky_solve_capture", {
    capture,
    here,
    choice: null,
  });
}

/** `tools::ToolRow`. */
export interface ToolRow {
  id: string;
  name: Record<string, string>;
  kind: string;
  required: boolean;
  state: "present" | "missing" | "outdated" | "not_checked" | "downloaded";
  version: string | null;
  minVersion: string | null;
  path: string | null;
  page: string | null;
  licence: string | null;
  /** The automatic download for this system, if the registry has one. */
  download?: { url: string; sha256: string; bytes: number | null } | null;
  /** What the person downloads for this system, in bytes. */
  downloadBytes?: number | null;
  /** Bytes of an interrupted download kept (« Reprendre le
   * téléchargement »). */
  partialBytes?: number | null;
  progressionCompatible: boolean | null;
  /** The Python environment (and the stacking pipeline): an installation
   * that did not finish (« Reprendre l'installation »). */
  interrupted?: boolean;
  /** Astro Sync can make it here (a Python in the range was found). */
  installable?: boolean;
  /** What it waits for: `python` (none in the range), `siril`,
   * `python-env`. */
  blockedBy?: string[];
}

export function toolsDetect(): Promise<ToolRow[]> {
  return invoke<ToolRow[]>("tools_detect");
}

/** « Vérifier à nouveau »: the Python environment checked by its own
 * script, then every row detected again. */
export function toolsVerify(): Promise<ToolRow[]> {
  return invoke<ToolRow[]>("tools_verify");
}

/** `python_env::EnvFailure` (tag `code`). */
export type EnvFailure =
  | { code: "no_python"; first: string; last: string }
  | { code: "no_resources" }
  | { code: "venv"; detail: string }
  | { code: "packages"; detail: string }
  | { code: "check"; detail: string; problems: string[] }
  | { code: "cancelled" };

/** `python_env::EnvProgress`, emitted on `astro-sync://python-env`. */
export interface EnvProgress {
  step: "python" | "environment" | "packages" | "check";
  index: number;
  total: number;
  packagesDone: number;
  packagesTotal: number;
}

export const PYTHON_ENV_EVENT = "astro-sync://python-env";

export function onPythonEnv(
  handler: (progress: EnvProgress) => void,
): Promise<UnlistenFn> {
  return listen<EnvProgress>(PYTHON_ENV_EVENT, (event) =>
    handler(event.payload),
  );
}

/** `commands_night::EnvAnswer`. */
export type EnvAnswer =
  | { result: "done"; check: { ok: boolean; pythonVersion: string } }
  | { result: "failed"; failure: EnvFailure };

/** « Installer » / « Reprendre l'installation » of the stacking's Python
 * environment. */
export function pythonEnvInstall(): Promise<EnvAnswer> {
  return invoke<EnvAnswer>("python_env_install");
}

/** « Réinstaller… » of the stacking's Python environment (Réglages ›
 * Général): removed, then made again like « Installer ». Refused with
 * `processing_files_busy` while a stacking runs. */
export function pythonEnvReinstall(): Promise<EnvAnswer> {
  return invoke<EnvAnswer>("python_env_reinstall");
}

/** `processing_files::CategoryKind`. */
export type ProcessingKind =
  "masters" | "nightResults" | "planRenders" | "temp";

/** `processing_files::CategoryScan`. */
export interface ProcessingCategory {
  kind: ProcessingKind;
  bytes: number;
  /** Nights (plans for `planRenders`) that hold something of it. */
  count: number;
  /** For the results: what is already in the space, and what is not. */
  sentBytes: number;
  unsentBytes: number;
  sentCount: number;
  unsentCount: number;
}

/** `processing_files::ProcessingScan`. */
export interface ProcessingScan {
  /** The working folder is there; otherwise every size is meaningless. */
  reachable: boolean;
  root: string;
  computedAt: string;
  folderBytes: number;
  /** What the person copied, moved or linked in: never touched. */
  userBytes: number;
  pythonEnvBytes: number;
  categories: ProcessingCategory[];
}

/** `processing_files::CleanKind`: the boxes of the confirmation. */
export type ProcessingCleanKind =
  "masters" | "sentResults" | "unsentResults" | "temp";

/** `processing_files::CleanOutcome`. */
export interface ProcessingCleanOutcome {
  freedBytes: number;
  failed: { path: string; code: string }[];
}

/** « Calculer » of Réglages › Général › « Fichiers de traitement »: reads
 * only, off the window's thread. */
export function processingFilesScan(): Promise<ProcessingScan> {
  return invoke<ProcessingScan>("processing_files_scan");
}

/** « Supprimer » of the confirmation: only what `kinds` names, only what the
 * pipelines produced. Refused with `processing_files_busy` while a stacking
 * runs. */
export function processingFilesClean(
  kinds: ProcessingCleanKind[],
): Promise<ProcessingCleanOutcome> {
  return invoke<ProcessingCleanOutcome>("processing_files_clean", { kinds });
}

/** `tools::DownloadFailure` (tag `code`). */
export type DownloadFailure =
  | { code: "no_automatic_download" }
  | {
      code: "interrupted";
      keptBytes: number;
      total: number | null;
      detail: string;
    }
  | { code: "cancelled"; keptBytes: number }
  | { code: "digest_mismatch" }
  | { code: "extraction"; detail: string }
  | { code: "io"; detail: string };

/** `commands_night::DownloadAnswer` (tag `result`). */
export type DownloadAnswer =
  | { result: "done"; id: string }
  | { result: "failed"; id: string; failure: DownloadFailure }
  | { result: "env_failed"; id: string; failure: EnvFailure };

/** « Installer » (one row) or « Reprendre le téléchargement ». */
export function toolsDownload(id: string): Promise<DownloadAnswer> {
  return invoke<DownloadAnswer>("tools_download", { id });
}

/** « Tout installer »: the required tools missing that have a download. */
export function toolsInstallAll(): Promise<DownloadAnswer[]> {
  return invoke<DownloadAnswer[]>("tools_install_all");
}

/** `tools::DownloadProgress`, emitted as `[id, progress]`. */
export interface DownloadProgress {
  bytes: number;
  total: number | null;
}

export const TOOL_DOWNLOAD_EVENT = "astro-sync://tool-download";

export function onToolDownload(
  handler: (id: string, progress: DownloadProgress) => void,
): Promise<UnlistenFn> {
  return listen<[string, DownloadProgress]>(TOOL_DOWNLOAD_EVENT, (event) =>
    handler(event.payload[0], event.payload[1]),
  );
}

/** `pipeline_trust::TrustState`. */
export type TrustState =
  "not_needed" | "ask" | "trusted" | "refused" | "changed";

/** `pipelines::Pipeline`, the fields the window reads. */
export interface Pipeline {
  id: string;
  name: Record<string, string>;
  description: Record<string, string>;
  displayName: string | null;
  displayDescription: string | null;
  scriptSha256: string | null;
  /** `shipped` or `user`. */
  source?: string;
  trust?: TrustState | null;
  /** The capture filter it is made for: `none`, `dual-band`, or a word of
   * the person's definition. */
  filter?: string;
  /** `tif`, `png`: what it writes (« Produit : TIFF · PNG »). */
  outputs?: string[];
  /** What it needs, by the registry's id (the tools of the card). */
  dependencies?: {
    id: string;
    required: boolean;
    /** What the person downloads for it on this system. */
    bytes?: number | null;
    /** Why this pipeline needs it, per language. */
    reason?: Record<string, string>;
  }[];
  /** The steps of `details` in the language of the listing (« Voir plus »). */
  displayDetails?: { title: string; text: string }[] | null;
  /** The checked relative path of its preview image (`preview` of its
   * `pipeline.json`), null or absent when it has none. */
  preview?: string | null;
  /** That image as a data URL, read when the list is made (the banner under
   * the description). */
  previewData?: string | null;
  /** Why a `preview` that was written is not shown. */
  previewIssue?: string | null;
  /** The bands of target it is recommended and discouraged for. */
  recommendedFor?: Band[];
  discouragedFor?: Band[];
}

/** `plan_image::Band`: what a filter lets through. */
export type Band = "broadband" | "dualband";

/** `pipelines::SectionDependency`. */
export interface SectionDependency {
  id: string;
  name: string;
  reason: string;
  required: boolean;
  state: ToolRow["state"];
  version: string | null;
  minVersion: string | null;
  licence: string;
  page: string;
  bytes: number | null;
  installable: boolean;
  interrupted: boolean;
  blockedBy: string[];
}

/** `pipelines::PipelineSection`. */
export interface PipelineSection {
  id: string;
  name: string;
  description: string;
  source: string;
  author: string;
  licence: string;
  trust: TrustState;
  scriptSha256: string | null;
  dependencies: SectionDependency[];
  /** The steps of `details` in the person's language; none for a
   * definition that gives no steps. */
  details?: { title: string; text: string }[];
}

/** `commands_night::PipelineSections`. */
export interface PipelineSections {
  sections: PipelineSection[];
  ignored: { folder: string; source: string; reason: string }[];
}

/** The section « Pipelines de traitement » of « Dépendances ». */
export function pipelineSections(language: string): Promise<PipelineSections> {
  return invoke<PipelineSections>("pipeline_sections", { language });
}

/** The person's answer to « Lui faites-vous confiance ? » for a script. */
export function pipelineTrustSet(
  id: string,
  sha256: string,
  trusted: boolean,
): Promise<void> {
  return invoke<void>("pipeline_trust_set", { id, sha256, trusted });
}

export interface PipelineList {
  /** Where the person's own definitions go, when the listing says. */
  userFolder?: string;
  pipelines: Pipeline[];
}

export function pipelinesList(language: string): Promise<PipelineList> {
  return invoke<PipelineList>("pipelines_list", { language });
}

/** `stacking::StackedRender`: one render the stacking wrote. */
export interface StackedRender {
  pipelineId: string;
  /** What the image carries (a gain or a word, a percent): absent is none. */
  contrast?: number | string;
  colour?: number | string;
  softStars?: number | string;
  path: string;
  sha256: string;
  size: number;
  family?: string | null;
}

/** `commands_night::StackRun`. */
export interface StackRun {
  /** `stacking::StackOutcome`, the fields the window reads. */
  outcome?: { renders: StackedRender[] };
  candidates: ImageCandidate[];
}

/** `stacking::StackEvent`, emitted on `astro-sync://stacking`. */
export type StackEvent =
  | { event: "family"; family: string | null; index: number; total: number }
  | { event: "step"; step: string; index: number; total: number }
  | { event: "output"; pipeline: string; file: string }
  /** What the registration said of the lights: how far the field turned
   * along the night (degrees, `null` when it could not be measured), the
   * framing that follows (`full`: the whole field of the reference light,
   * `common`: the area every light covers), and the lights Siril could not
   * align, left out and named. */
  | {
      event: "registration";
      /** What the frames are: the lights of a night, or the masters of the
       * nights of a plan. */
      unit: "lights" | "masters";
      frames: number;
      registered: number;
      unregistered: string[];
      fieldRotationDegrees: number | null;
      framing: "common" | "full";
      /** The frame of the stacking of a night (its notches, measured from the
       * transforms); absent for a plan. */
      frame?: FrameMeasure;
    }
  /** What the combination of a plan's masters said of the frame: the
   * share of the image (0 to 1) no dual-band master reaches; only sent
   * when it is not nil. */
  | { event: "coverage"; noDualBandShare: number }
  /** What the image of a plan was cropped to: the largest rectangle without
   * a pixel no night covers (`clean`) or every night covers (`common`);
   * `applied: false` when nothing was cut. */
  | {
      event: "crop";
      mode: PlanField;
      applied: boolean;
      reason: string;
      top: number;
      left: number;
      height: number;
      width: number;
      frameHeight: number;
      frameWidth: number;
      /** The share of the frame kept (0 to 1). */
      keptShare: number;
    }
  /** What Siril's journal says: the last percentage of its current command
   * and the last useful line; `at`: when that line was read (Unix
   * milliseconds, the clock of this computer). */
  | {
      event: "progress";
      percent: number | null;
      line: string | null;
      at: number;
    };

export const STACKING_EVENT = "astro-sync://stacking";

export function onStacking(
  handler: (event: StackEvent) => void,
): Promise<UnlistenFn> {
  return listen<StackEvent>(STACKING_EVENT, (event) => handler(event.payload));
}

/** `progression_images::ProgressionEvent`, emitted on
 * `astro-sync://progression`. */
export type ProgressionEvent =
  | { event: "stage"; stage: "registration" | "milestones" }
  | { event: "milestone"; index: number; total: number }
  | {
      event: "progress";
      percent: number | null;
      line: string | null;
      at: number;
    };

export const PROGRESSION_EVENT = "astro-sync://progression";

export function onProgression(
  handler: (event: ProgressionEvent) => void,
): Promise<UnlistenFn> {
  return listen<ProgressionEvent>(PROGRESSION_EVENT, (event) =>
    handler(event.payload),
  );
}

export function stackingRun(args: {
  nightFolder: string;
  targetFolder: string;
  targetId: string;
  eveningDate: string;
  /** The place's longitude: the lights outside the night (local noon to
   * local noon) are left out of the integration. */
  longitude?: number | null;
  /** Each pipeline with its own finish when it has one (« Finition » of its
   * card); without, the « Finition par défaut » of the local settings. */
  pipelines: {
    pipelineId: string;
    trustedSha256: string | null;
    finish?: { contrast: string | number; colour: number; softStars: number };
  }[];
  /** The poses « Mesure des poses » flagged and the person left out: not
   * integrated, never deleted. */
  discarded?: string[] | null;
  /** The files of `<cible>/device/` the families take as their image instead
   * of a stacking of their poses; none: all are stacked. */
  device?: { file: string }[] | null;
  /** « Cadre de l'empilement »: the rank of a notch set by hand, 0 (the area
   * every light covers) to 4 (the whole field); none: Automatic. */
  frame?: number | null;
}): Promise<StackRun> {
  return invoke<StackRun>("stacking_run", args);
}

/** One notch of the frame of a stacking (`stacking::FrameNotch`), measured at
 * the registration: its place and size in pixels, the share of the whole
 * field it keeps, the share of it every light covers, the lights at its
 * lowest point. */
export interface FrameNotch {
  rank: number;
  top: number;
  left: number;
  height: number;
  width: number;
  shareOfFrame: number;
  fullShare: number;
  lowestFrames: number;
}

/** The frame of a stacking (`stacking::FrameMeasure`): no notch when the
 * field did not turn (nothing to choose); `automatic` is the rank Automatic
 * gives, `rank` the one that was cut. */
export interface FrameMeasure {
  frameHeight: number | null;
  frameWidth: number | null;
  notches: FrameNotch[];
  automatic: number | null;
  rank: number | null;
}

/** What the last stacking of a target measured of its frame
 * (`stacking::TargetFrame`). */
export interface TargetFrame {
  fieldRotationDegrees: number | null;
  registered: number;
  measure: FrameMeasure;
}

/** « Cadre de l'empilement »: the frame the last stacking of a target
 * measured; `null` before the first one. */
export function stackingFrame(
  nightFolder: string,
  targetFolder: string,
): Promise<TargetFrame | null> {
  return invoke<TargetFrame | null>("stacking_frame", {
    nightFolder,
    targetFolder,
  });
}

/* ---------------------------- Liste des incidents ------------------------ */

/** `incidents::IncidentKind`: the genres of the list. */
export type IncidentKind =
  | "pause"
  | "condition"
  | "equipment_issue"
  | "note"
  | "exposure_change"
  | "guiding_timeouts"
  | "sensor_temperature_drift"
  | "guiding_spikes";

/** `incidents::Evidence`: where a detection was read and what, as a code and
 * parameters the window says in the person's language. */
export interface IncidentEvidence {
  source: "asiair" | "phd2" | "nina" | "fits";
  code: string;
  params: Record<string, string | number | boolean | null>;
}

/** `incidents::DetectedIncident`. */
export interface DetectedIncident {
  /** The detection key, `genre:instant`. */
  id: string;
  eventType: IncidentKind;
  start: string;
  end: string | null;
  code: string;
  params: Record<string, string | number | boolean | null>;
  /** Neutral English, what goes to the service when nobody edits it. */
  description: string;
  evidence: IncidentEvidence;
  payload: Record<string, unknown>;
}

/** `incidents::Unproposed`: detected, with no signalement to match. */
export interface UnproposedIncident {
  code: string;
  count: number;
  params: Record<string, string | number | boolean | null>;
}

/** `incidents::Detection`. */
export interface IncidentDetection {
  incidents: DetectedIncident[];
  unproposed: UnproposedIncident[];
  poses: number;
  problems: { code: string; detail: string }[];
}

/** `incidents::DeclaredEvent`: a signal the session holds. */
export interface DeclaredIncident {
  eventId: string;
  eventType: IncidentKind;
  recordedAt: string;
  payload: Record<string, unknown>;
  /** `declared`, or `astro-sync` for a detection added before. */
  source: string | null;
  timing: string | null;
  targetId: string | null;
  /** The end of a pause (its `resume`). */
  until: string | null;
  resumeEventId: string | null;
}

export interface DeclaredIncidents {
  events: DeclaredIncident[];
  truncated: boolean;
}

/** `incidents::AddedIncident` and `incidents::Replacement`. */
export interface AddedIncident {
  key: string;
  eventType: IncidentKind;
  at: string;
  until: string | null;
  payload: Record<string, unknown>;
}

export interface IncidentReplacement {
  eventId: string;
  eventType: IncidentKind;
  at: string;
  until: string | null;
  payload: Record<string, unknown>;
}

export interface IncidentsApply {
  sessionId: string;
  adds: AddedIncident[];
  replacements: IncidentReplacement[];
}

/** `incidents::ApplyOutcome`. */
export interface IncidentsApplied {
  added: { key: string; eventId: string; resumeEventId: string | null }[];
  duplicate: string[];
  rejected: { key: string; code: string; message: string }[];
  replaced: string[];
  replacementsRefused: { code: string; message: string } | null;
}

/** What the night's files say (no network). */
export function incidentsDetect(args: {
  nightFolder: string;
  targetFolder?: string | null;
  eveningDate: string;
  longitude?: number | null;
}): Promise<IncidentDetection> {
  return invoke<IncidentDetection>("incidents_detect", args);
}

/** The signalements the session holds (`GET /v1/sessions/{id}/events`). */
export function incidentsDeclared(
  sessionId: string,
): Promise<DeclaredIncidents> {
  return invoke<DeclaredIncidents>("incidents_declared", { sessionId });
}

/** « Ajouter N incidents à la session ». */
export function incidentsApply(
  request: IncidentsApply,
): Promise<IncidentsApplied> {
  return invoke<IncidentsApplied>("incidents_apply", { request });
}

/** The `eventId` of each detection key in a session. */
export function incidentsEventIds(
  sessionId: string,
  keys: string[],
): Promise<string[]> {
  return invoke<string[]>("incidents_event_ids", { sessionId, keys });
}

/* ----------------------------- Mesure des poses -------------------------- */

/** What the measure will take, before it starts (`commands_poses`). */
export function posesPlan(args: {
  nightFolder: string;
  targetFolder: string;
  eveningDate: string;
  longitude?: number | null;
}): Promise<FamiliesPlan> {
  return invoke<FamiliesPlan>("pose_stats_plan", args);
}

/** « Mesurer les poses » (`pose_stats::MeasureRequest`): the poses of
 * `resume` (a stopped measure) are not measured again. */
export interface PosesRequest {
  nightFolder: string;
  targetFolder: string;
  eveningDate: string;
  longitude?: number | null;
  latitude?: number | null;
  resume?: PoseMeasure[];
}

/** The measure, pose by pose; `localWorkCancelKind("pose-stats")` stops it
 * between two poses and what is measured comes back (`stopped`). */
export function posesRun(request: PosesRequest): Promise<PoseStats> {
  return invoke<PoseStats>("pose_stats_run", { request });
}

export const POSE_STATS_EVENT = "astro-sync://pose-stats";

export function onPoseStats(
  handler: (event: PoseEvent) => void,
): Promise<UnlistenFn> {
  return listen<PoseEvent>(POSE_STATS_EVENT, (event) => handler(event.payload));
}

/** « Progression »: the families of a target's lights. */
export function progressionFamilies(
  nightFolder: string,
  targetFolder: string,
): Promise<AcquisitionFamily[]> {
  return invoke<AcquisitionFamily[]>("progression_families", {
    nightFolder,
    targetFolder,
  });
}

/** `progression_images::ProgressionImages`, passed back as it is. */
export type ProgressionImages = Record<string, unknown>;

export function progressionImagesRun(args: {
  nightId: string;
  nightFolder: string;
  targetFolder: string;
  family: AcquisitionFamily;
}): Promise<ProgressionImages> {
  return invoke<ProgressionImages>("progression_images_run", args);
}

export function progressionImagesCandidates(
  images: ProgressionImages,
  targetId: string,
): Promise<ImageCandidate[]> {
  return invoke<ImageCandidate[]>("progression_images_candidates", {
    images,
    targetId,
  });
}

/** What a sending that went through changes in the night's ledger. */
export function nightRecordSending(
  nightId: string,
  sent: {
    analysisFingerprint: string | null;
    progressionFrames: Record<string, number>;
  },
): Promise<unknown> {
  return invoke<unknown>("night_record_sending", { nightId, sent });
}

/* ----------------------- The step « Cibles » ----------------- */

/** `catalogue_targets::CatalogueTarget`: one card of the catalogue. */
export interface CatalogueTarget {
  id: string;
  designation: string;
  name: string | null;
  aliases: string[];
  typeLabel: string | null;
  constellationLabel: string | null;
  raHours: number | null;
  decDegrees: number | null;
  magnitude: number | null;
  sizeLabel: string | null;
  sizeArcmin: number | null;
  /** A path on the service (`/…`), the photo or else the drawn view. */
  imagePath: string | null;
}

/** `catalogue_targets::TargetSearch`. */
export interface TargetSearch {
  query: string;
  total: number;
  targets: CatalogueTarget[];
}

/** « Rechercher une cible du catalogue »: `limit` cards from `offset`. */
export function targetsSearch(
  text: string,
  locale: string,
  offset: number,
  limit: number,
): Promise<TargetSearch> {
  return invoke<TargetSearch>("targets_search", {
    text,
    locale,
    offset,
    limit,
  });
}

/** `catalogue_targets::NightTargetChange`: an addition, or taking back a
 * target added in this opening of the wizard (nothing else is offered). */
export type NightTargetChange =
  { kind: "add"; targetId: string } | { kind: "undoAdd"; targetId: string };

/** The night's targets after the change, as the service holds them. */
export function nightTargetsChange(
  sessionId: string,
  change: NightTargetChange,
): Promise<string[]> {
  return invoke<string[]>("night_targets_change", { sessionId, change });
}

/** The system's file picker for a capture's frame; `null`: dismissed. */
export function fitsChoose(): Promise<string | null> {
  return invoke<string | null>("fits_choose");
}

/** `master_quality::Outcome` (« Mesurer un master… »): the quality read on a
 * master the person stacked elsewhere and kept for the target, or the word of
 * what could not be measured (`stretched`, `too_few_stars`,
 * `unsupported_format`, `unreadable`, `not_an_image`, `reserved`,
 * `python_missing`, `night_missing`). */
export type MasterOutcome =
  | { result: "measured"; master: string; quality: ImageQuality }
  | { result: "refused"; code: string };

/** Measures the master `file` (FITS or XISF, linear) for the target whose
 * folder in the night is `folderName`; the figures are kept there and leave
 * with the target's images, under `analysis.logs`. */
export function masterMeasure(
  sessionId: string,
  folderName: string,
  file: string,
): Promise<MasterOutcome> {
  return invoke<MasterOutcome>("master_measure", {
    sessionId,
    folderName,
    file,
  });
}

/** What is kept for the target: `null` when no master was measured. */
export function masterMeasureCurrent(
  sessionId: string,
  folderName: string,
): Promise<MasterOutcome | null> {
  return invoke<MasterOutcome | null>("master_measure_current", {
    sessionId,
    folderName,
  });
}

/** « Retirer la mesure »: the target's images leave without a quality again. */
export function masterMeasureForget(
  sessionId: string,
  folderName: string,
): Promise<void> {
  return invoke<void>("master_measure_forget", { sessionId, folderName });
}

/** `catalogue_targets::HeaderTarget`. */
export interface HeaderTarget {
  object: string | null;
  raDegrees: number | null;
  decDegrees: number | null;
  positionSource: "crval" | "ra-dec" | "objctra" | null;
  fieldDegrees: [number, number];
  fieldFromHeader: boolean;
}

/** `catalogue_targets::FitsGuess`. */
export interface FitsGuess {
  fileName: string;
  header: HeaderTarget;
  kind: "found" | "hesitates" | "candidates" | "unknown-object" | "nothing";
  named: CatalogueTarget | null;
  candidates: { target: CatalogueTarget; distanceDegrees: number }[];
}

/** The header of `file` read on this computer (nothing of it is sent). */
export function fitsTargetGuess(
  file: string,
  locale: string,
): Promise<FitsGuess> {
  return invoke<FitsGuess>("fits_target_guess", { file, locale });
}

/** `catalogue_targets::PersonalTargetCreate`. */
export interface PersonalTargetCreate {
  name: string;
  /** One of the service's seven labels (`PERSONAL_LABELS`). */
  label: string;
  raHours: number | null;
  decDegrees: number | null;
}

/** `catalogue_targets::PersonalTargetCreated`. */
export interface PersonalTargetCreated {
  target: { id: string; name: string };
  nightTargetIds: string[] | null;
}

/** « Créer et ajouter à la nuit ». */
export function personalTargetCreate(
  target: PersonalTargetCreate,
  sessionId: string,
): Promise<PersonalTargetCreated> {
  return invoke<PersonalTargetCreated>("personal_target_create", {
    target,
    sessionId,
  });
}

/* --------------------------- The multi-night plans ------------------------- */
// `astro_sync_core::multi_night` and `plan_image`, through
// `commands_multi_night`.

/** `multi_night::IntegrationMaster`: a master kept, read from its header. */
export interface IntegrationMaster {
  path: string;
  family: string;
  filter: string | null;
  exposureSeconds: number | null;
  gain: number | null;
  binning: number | null;
  temperatureC: number | null;
  frameCount: number;
  totalSeconds: number;
  sensor: string | null;
  bytes: number;
  colour: boolean;
}

/** `multi_night::FamilyPreview`. */
export interface FamilyPreview {
  label: string;
  filter: string | null;
  /** What the filter lets through; absent: broadband. */
  band?: Band;
  exposureSeconds: number | null;
  gain: number | null;
  binning: string | null;
  temperatureC: number | null;
  frameCount: number;
  totalSeconds: number;
  master: IntegrationMaster | null;
}

/** `multi_night::IntegrationPreview`. */
export interface IntegrationPreview {
  targetFolder: string;
  families: FamilyPreview[];
  outside: number;
}

/** `multi_night::ServiceIntegration` (`integration.schema.json`). */
export interface ServiceIntegration {
  id: string;
  nightId: string;
  targetId: string;
  filter: string | null;
  exposureSeconds: number;
  gain: number | null;
  binning: number | null;
  temperatureC: number | null;
  frameCount: number;
  totalSeconds: number;
  eveningDate: string;
  sensor: string | null;
  computerLabel: string | null;
  installationId: string | null;
}

/** `multi_night::NightProject`. */
export interface NightProject {
  projectId: string;
  targetId: string;
  name: string;
  commonName: string | null;
  integrations: ServiceIntegration[];
  totalSeconds: number;
  nightCount: number;
  planCapture: Record<string, unknown> | null;
}

export type MultiNightClosed = "not_in_subscription" | "unknown";

/** `multi_night::MultiNightGate`. */
export interface MultiNightGate {
  allowed: boolean;
  closed: MultiNightClosed | null;
}

/**
 * `multi_night::AttachGate`: the right of the gestures on a plan itself --
 * attach, detach, create from a night -- is `planner.mosaic` (« Planification
 * avancée »); `MultiNightGate` stays `analysis.progression` (the integrations
 * and the image of the plan).
 */
export interface AttachGate {
  allowed: boolean;
  closed: MultiNightClosed | null;
}

/** `plan_image::PlanSource`: what a night's integration is made from: the
 * master Astro Sync kept of its poses, or the image its device stacked. */
export type PlanSource = "master" | "device";

/** `plan_image::DeviceCandidate`: the image a device stacked for a night. */
export interface DeviceCandidate {
  path: string;
  /** « Seestar S50 Pro ». */
  device: string;
  frameCount: number;
  totalSeconds: number;
  bytes: number;
}

/** `plan_image::PlanLine`. */
export interface PlanLine {
  key: string;
  nightId: string;
  eveningDate: string;
  filter: string | null;
  band: "broadband" | "dualband";
  frameCount: number;
  exposureSeconds: number | null;
  totalSeconds: number;
  place: "thisNight" | "thisComputer" | "otherComputer";
  computerLabel: string | null;
  /** The master Astro Sync kept of the night's poses (« Empiler à nouveau »). */
  master: string | null;
  bytes: number;
  /** What the job takes when nothing is chosen: the master when one is kept. */
  source: PlanSource;
  /** The image the night's device stacked, when it is here. */
  device?: DeviceCandidate;
}

/** `plan_image::PlanFrame`: the four integers of a frame of the whole image. */
export interface PlanFrame {
  top: number;
  left: number;
  height: number;
  width: number;
}

/** `plan_image::FramingNotch`: one stop of the slider of the frame, as
 * `plan_framing.py` measured it. */
export interface FramingNotch extends PlanFrame {
  /** The share of the whole image (0 to 1). */
  shareOfFrame: number;
  /** The share of this frame below the full exposure (0 to 1). */
  degradedShare: number;
  lowestSeconds: number;
  lowestDualBandSeconds: number;
  kind: "cropped" | "mark" | "between" | "whole";
  recommended: boolean;
  default: boolean;
}

/** `plan_image::PlanFraming`: what the screen offers for the frame of the
 * image of a plan, from the record of the last image made with the same
 * integrations. */
export interface PlanFraming {
  record: string;
  frameHeight: number;
  frameWidth: number;
  degradedShare: number;
  noDualBandShare: number;
  hasDualBand: boolean;
  fullSeconds: number;
  fullDualBandSeconds: number;
  recommendedDegradedShare: number;
  /** The corners (`[x, y]`) of the area every integration covers. */
  fullPolygon: [number, number][];
  /** Cropped first, the whole image last. */
  notches: FramingNotch[];
  /** At least 1 % of the image below the full exposure and a smaller frame to take. */
  offered: boolean;
  /** A data URL of a small picture of the whole image. */
  preview: string | null;
}

/** `commands_multi_night::MultiNightView`. */
export interface MultiNightView {
  eveningDate: string | null;
  projects: { project: NightProject; lines: PlanLine[] }[];
  gate: MultiNightGate;
  computer: { label: string | null; installationId: string | null };
}

/** `multi_night::IntegrationsSent`. */
export type IntegrationsSent =
  | { result: "sent"; projectId: string; lines: number; project: NightProject }
  | { result: "closed"; projectId: string; closed: MultiNightClosed }
  | { result: "nothing"; projectId: string }
  | { result: "refused"; projectId: string; code: string; message: string };

/** `plan_image::PlanImageOutcome`. */
export interface PlanImageOutcome {
  projectId: string;
  targetId: string;
  render: { pipelineId: string; path: string; sha256: string; size: number };
  /** The other treatments of the same render (contrast, colours, softened
   * stars): they leave with it as images of the plan's capture. */
  finishes?: {
    render: { pipelineId: string; path: string; sha256: string; size: number };
    processing?: { software?: string | null; steps?: string | null } | null;
  }[];
  otherFiles: string[];
  combinedNightIds: string[];
  totalsByFilter: {
    filter: string | null;
    frameCount: number;
    totalSeconds: number;
  }[];
  planRender: "natural" | "dualband" | "polished" | "hoo" | "none" | null;
  /** What made the image (« Astro Sync 0.9.1 · Siril 1.4.4 », the combination
   * method...): proposed with it so the service never reads the TIFF's
   * `Software` tag. */
  processing?: { software?: string | null; steps?: string | null } | null;
}

/** `commands_multi_night::PlanImageRun`. */
export type PlanImageRun =
  | {
      kind: "made";
      outcomes: PlanImageOutcome[];
      enqueued: EnqueueOutcome;
      /** The image of the plan after each night (asked with `progression`),
       * queued as the history of the plan's capture unless the run kept the
       * images here. */
      progression: PlanProgressionStep[];
      /** Why the progression stopped, when it did; the images are made. */
      progressionFailure: string | null;
    }
  | { kind: "oneNight"; nightId: string }
  | { kind: "nothing" };

/** One milestone of the progression of a plan: the image of the plan after a
 * night (`plan_progression::PlanProgressionStep`). */
export interface PlanProgressionStep {
  nightId: string;
  eveningDate: string;
  /** The nights combined so far, this one included. */
  nights: number;
  /** The poses combined so far. */
  frames: number;
  seconds: number;
  jpeg: string;
  sha256: string;
  size: number;
}

/** `multi_night::PlanTag`. */
export interface PlanTag {
  projectId: string;
  name: string;
  nightCount: number;
  totalSeconds: number;
}

/** The multi-night plans the night's targets work for, their integrations
 * here and elsewhere, and whether they are open here. */
export function nightMultiNight(
  sessionId: string,
  planId: string | null,
  eveningDate: string,
): Promise<MultiNightView> {
  return invoke<MultiNightView>("night_multi_night", {
    sessionId,
    planId,
    eveningDate,
  });
}

/** « Intégration de la nuit » of one target: its families in the night's
 * window, the frames left out, the master kept of each. */
export function nightIntegrationPreview(
  nightFolder: string,
  targetFolder: string,
  input: DetectionInput,
): Promise<IntegrationPreview> {
  return invoke<IntegrationPreview>("night_integration_preview", {
    nightFolder,
    targetFolder,
    input,
  });
}

/** After the sending: the night's lines for each multi-night plan. */
export function nightIntegrationsSend(args: {
  sessionId: string;
  planId: string | null;
  nightFolder: string;
  eveningDate: string;
}): Promise<IntegrationsSent[]> {
  return invoke<IntegrationsSent[]>("night_integrations_send", args);
}

/** « Image du plan multi-nuits »: made here with each pipeline of the plan's
 * target and queued as captures of the plan. */
export function planImageRun(args: {
  sessionId: string;
  planId: string | null;
  projectId: string;
  chosen: string[];
  pipelines: { pipelineId: string; trustedSha256: string | null }[];
  eveningDate: string;
  /** `false` keeps the images here (« Images à envoyer » queues the ones
   * ticked); absent: queued as made. */
  queue?: boolean;
  /** The source taken for a night, by line key; a night not named takes its
   * default (the master when one is kept). */
  sources?: Record<string, PlanSource>;
  /** The frame kept (a notch of `planFramingRead`); absent: the whole image. */
  frame?: PlanFrame;
  /** Also make the image of the plan after each night (the history of the
   * capture), with the main image's pipeline. */
  progression?: boolean;
}): Promise<PlanImageRun> {
  return invoke<PlanImageRun>("plan_image_run", args);
}

/** The frames on offer for the image of a plan: the notches measured on the
 * last image made with the same integrations, and its picture; `null` before
 * the first image of that set (it is made whole). Reads only here. */
export function planFramingRead(args: {
  projectId: string;
  lines: PlanLine[];
  chosen: string[];
  sources?: Record<string, PlanSource>;
}): Promise<PlanFraming | null> {
  return invoke<PlanFraming | null>("plan_framing_read", args);
}

/** « Nuits »: the tags of the nights of a multi-night plan, by night. */
export function multiNightTags(): Promise<Record<string, PlanTag[]>> {
  return invoke<Record<string, PlanTag[]>>("multi_night_tags");
}

/* ---------------- Attaching a night to a multi-night plan -------- */
// `astro_sync_core::multi_night_attach` through `commands_multi_night`;
// the command line is `astro-sync plan-attach`.

/** `commands_multi_night::AttachedPlan`: a multi-night plan the night is in. */
export interface AttachedPlan {
  projectId: string;
  name: string;
  commonName: string | null;
  nightCount: number;
  totalSeconds: number;
}

/** `commands_multi_night::NightAttachment`. */
export interface NightAttachment {
  plans: AttachedPlan[];
  gate: AttachGate;
}

/** `multi_night_attach::ChoiceState`. */
export type ChoiceState = "available" | "current" | "moves" | "other_target";

/** `multi_night_attach::PlanChoice`. */
export interface PlanChoice {
  projectId: string;
  name: string;
  commonName: string | null;
  targetId: string;
  goalMinutes: number | null;
  nightCount: number;
  totalSeconds: number;
  state: ChoiceState;
  fromProjectId: string | null;
  fromName: string | null;
}

/** `multi_night_attach::PlanChoices`. */
export interface PlanChoices {
  plans: PlanChoice[];
  noTarget: boolean;
  total: number;
}

/** `multi_night_attach::NightPlanChange`. */
export type NightPlanChange =
  | { result: "attached"; projectId: string; nights: number }
  | { result: "detached"; projectId: string; nights: number }
  | { result: "closed"; projectId: string; closed: MultiNightClosed }
  | { result: "refused"; projectId: string; code: string; message: string };

/** `multi_night_attach::NewPlanTarget`. */
export interface NewPlanTarget {
  targetId: string;
  name: string;
  commonName: string | null;
}

/** `multi_night_attach::PlanCreated`. */
export type PlanCreated =
  | {
      result: "created";
      projectId: string;
      name: string;
      goalMinutes: number;
      nights: number;
    }
  | { result: "closed"; closed: MultiNightClosed }
  | { result: "missing"; missing: "place" | "target" }
  | { result: "refused"; code: string; message: string };

/** The multi-night plan(s) the night is in (by its plan's items or by
 * itself), and whether attaching is open here. */
export function nightAttachment(
  sessionId: string,
  planId: string | null,
): Promise<NightAttachment> {
  return invoke<NightAttachment>("night_attachment", { sessionId, planId });
}

/** The plans proposed for the night: those of its target, the current one
 * first. */
export function nightPlanChoices(
  sessionId: string,
  hasTarget: boolean,
): Promise<PlanChoices> {
  return invoke<PlanChoices>("night_plan_choices", { sessionId, hasTarget });
}

/** The search of a plan by the name of its target. */
export function nightPlanSearch(
  sessionId: string,
  query: string,
): Promise<PlanChoice[]> {
  return invoke<PlanChoice[]>("night_plan_search", { sessionId, query });
}

/** Attaches the night to a plan (a night of another plan is moved). */
export function nightPlanAttach(
  sessionId: string,
  projectId: string,
): Promise<NightPlanChange> {
  return invoke<NightPlanChange>("night_plan_attach", {
    sessionId,
    projectId,
  });
}

/** Detaches the night from a plan. */
export function nightPlanDetach(
  sessionId: string,
  projectId: string,
): Promise<NightPlanChange> {
  return invoke<NightPlanChange>("night_plan_detach", {
    sessionId,
    projectId,
  });
}

/** Creates a blank multi-night plan for a target of the night, the night in
 * it. */
export function nightPlanCreate(
  sessionId: string,
  target: NewPlanTarget,
): Promise<PlanCreated> {
  return invoke<PlanCreated>("night_plan_create", { sessionId, target });
}

export const PLAN_IMAGE_EVENT = "astro-sync://plan-image";

/** The image of a plan in progress: the stacking's events. */
export function onPlanImage(
  handler: (event: StackEvent) => void,
): Promise<UnlistenFn> {
  return listen<StackEvent>(PLAN_IMAGE_EVENT, (event) =>
    handler(event.payload),
  );
}

/* ------------------------- Détecter les sessions ------------------------- */
// The mirror of `astro_sync_core::session_detection`
// and of `app/src-tauri/src/commands_detect.rs`. The proposal is displayed as
// the core made it: the window only carries the person's answers back.

/** `session_detection::DetectRequest`. */
export interface DetectRequest {
  fits: string[];
  calibration: string[];
  logs: string[];
  subfolders: boolean;
  scope?: SessionScope | null;
}

export interface SessionScope {
  /** ISO 8601 UTC. */
  from: string;
  to: string;
  targets: ScopeTarget[];
}

export interface ScopeTarget {
  id: string;
  designation: string;
  aliases: string[];
  raHours?: number | null;
  decDegrees?: number | null;
}

/** `session_detection::KnownPlace`: the window computes `utcOffsetMinutes`
 * from `timeZone` (the core has no time-zone database). */
export interface KnownPlace {
  id: string;
  name: string;
  latitude: number;
  longitude: number;
  timeZone?: string | null;
  utcOffsetMinutes?: number | null;
}

export interface KnownEquipment {
  id: string;
  name: string;
  kind: string;
  manufacturer?: string | null;
  model?: string | null;
  focalLengthMm?: number | null;
}

export interface KnownSlot {
  targetId?: string | null;
  designation?: string | null;
  start?: string | null;
  end?: string | null;
}

export interface KnownSession {
  id: string;
  planId?: string | null;
  eveningDate: string;
  placeId?: string | null;
  targets: KnownSlot[];
}

export interface KnownPlan {
  id: string;
  name: string;
  eveningDate: string;
  /** `pending`, `executed`, `abandoned`, `archived`. */
  status: string;
  targetIds: string[];
  designations: string[];
  sessionId?: string | null;
}

export interface AccountRights {
  advancedPlanning: boolean;
  severalPlansPerNight: boolean;
}

/** `session_detection::UserData`. */
export interface UserData {
  places: KnownPlace[];
  defaultPlaceId?: string | null;
  equipment: KnownEquipment[];
  sessions: KnownSession[];
  plans: KnownPlan[];
  account: AccountRights;
}

export interface ReadingSummary {
  filesRead: number;
  lights: number;
  calibrationFrames: number;
  logFiles: number;
  nights: number;
  targets: number;
  links?: { relativePath: string; problem: string; target?: string | null }[];
}

export interface PlaceCandidate {
  placeId: string;
  name: string;
  distanceKm: number;
}

export type PlaceAction = "use" | "use-nearest" | "create" | "choose";

export interface PlaceProposal {
  id: string;
  latitude: number | null;
  longitude: number | null;
  candidates: PlaceCandidate[];
  chosen: string | null;
  action: PlaceAction;
  sessions: string[];
}

export interface EquipmentMatch {
  equipmentId: string;
  name: string;
  by: string;
}

export interface EquipmentProposal {
  id: string;
  telescop: string | null;
  instrume: string | null;
  focalLengthMm: number | null;
  camera: EquipmentMatch | null;
  telescope: EquipmentMatch | null;
  mount: EquipmentMatch | null;
  sessions: string[];
}

export type TargetSource =
  | { by: "object" }
  | { by: "coordinates"; distanceDegrees: number }
  | { by: "folder"; folder: string }
  | { by: "plate-solved"; frame: string; distanceDegrees: number }
  | { by: "unknown-object" }
  | { by: "to-solve" };

export interface AcquisitionRow {
  filter: string | null;
  exposureSeconds: number | null;
  gain: number | null;
  frames: number;
  totalSeconds: number;
}

export type TargetMatch = "new" | "in-session" | "add-to-session" | "overlap";

export interface DetectedTarget {
  key: string;
  targetId: string | null;
  designation: string;
  name: string | null;
  source: TargetSource;
  objects: string[];
  start: string;
  end: string;
  frames: number;
  exposureSeconds: number;
  filters: string[];
  acquisition: AcquisitionRow[];
  instrument: string;
  solveFrame?: string | null;
  /** Why the solver of this computer gave nothing on that frame: the Rust
   * `SolveFailure`, tagged by `code`. Absent when none was asked. */
  solveFailure?: { code: string; [detail: string]: unknown } | null;
  /** The centre `[ra, dec]`, in degrees, the solver found when it matched no
   * target of the catalogue. */
  solvedPosition?: [number, number] | null;
  files: string[];
  fileFilters?: string[];
  /** The start of each file, UTC (the « UTC » column). */
  fileTimes?: string[];
  /** The images a known device stacked for this target and night: not
   * poses, put in `device/` by « Ranger les fichiers ». */
  deviceImages?: string[];
  existing: TargetMatch;
}

export type SessionMatch =
  | { kind: "create" }
  | { kind: "existing"; sessionId: string }
  | { kind: "reuse-plan"; planId: string; planName: string }
  | { kind: "replace-plan"; planId: string; planName: string }
  | { kind: "plan-exists"; planId: string; planName: string };

export type FrameKind = "light" | "dark" | "flat" | "dark-flat" | "bias";

export interface ClockFinding {
  place:
    | { from: "header" }
    | { from: "default-place"; placeId: string }
    | { from: "none" };
  framesChecked: number;
  suspect: boolean;
  reasons: {
    code:
      | "sun-up"
      | "target-below-horizon"
      | "altitude-disagrees"
      | "lst-disagrees";
    frames: number;
  }[];
  consistentAsWritten: number;
  consistentAfterShift: number;
  proposedShiftHours: number | null;
  samples: {
    shiftHours: number;
    utc: string;
    sunAltitudeDeg: number;
    targetAltitudeDeg: number | null;
  }[];
}

export interface AttachedCalibration {
  setId: string;
  kind: FrameKind;
  daysAway: number;
  matchedOn: string[];
}

export interface AttachedLog {
  logId: string;
  targets: string[];
  matchedFrames: number;
}

export interface DetectedSession {
  id: string;
  eveningDate: string;
  writtenEveningDate: string;
  place: string;
  equipment: string[];
  start: string;
  end: string;
  targets: DetectedTarget[];
  clock?: ClockFinding | null;
  shiftHours: number;
  calibration: AttachedCalibration[];
  /** What the night's poses lack in calibration. */
  calibrationNotes?: CalibrationNote[];
  logs: AttachedLog[];
  existing: SessionMatch;
  /** Some target names no filter and the session that exists declares none:
   * the review asks the band of the night. */
  filterAsk?: boolean;
  /** The band the names of the night say (folders, file names, a
   * SESSION.md heading) when `filterAsk` holds: preselected in the review,
   * with where it comes from. */
  filterGuess?: BandGuess | null;
}

/** `session_detection::band::BandGuess`. */
export interface BandGuess {
  band: "broadband" | "dualband";
  /** Where the words were read: a folder, a file name, a note's heading. */
  origin: "folder" | "file" | "note";
  /** What is shown: the name (without its date) or the heading. */
  source: string;
  /** The note's file name (`SESSION.md`) when `origin` is `note`. */
  noteFile?: string;
}

/** `session_detection::CalibrationNote`: a warning when calibration of the
 * kind exists and does not fit (`noDark`, `noFlat`), an information when
 * there is none at all (`noDarkAtAll`, `noFlatAtAll`). */
export type CalibrationNote =
  | {
      code: "noDark";
      exposureSeconds: number;
      count: number;
      foundSeconds: number[];
    }
  | { code: "noDarkAtAll"; exposureSeconds: number }
  | { code: "noFlat"; filter: string | null }
  | { code: "noFlatAtAll"; filter: string | null };

export interface CalibrationSet {
  id: string;
  kind: FrameKind | null;
  kindFromFolder: boolean;
  folder: string;
  frames: number;
  exposureSeconds: number | null;
  gain: number | null;
  temperatureC: number | null;
  filter: string | null;
  instrument: string | null;
  start: string | null;
  end: string | null;
  files: string[];
}

export interface DetectedLogFile {
  id: string;
  path: string;
  /** `phd2`, `asiair`, `nina`. */
  kind: string;
  utcOffset: string | null;
  start: string | null;
  end: string | null;
  matchedFrames: number;
  sessions: string[];
}

export interface SetAside {
  withoutDate: string[];
  unreadable: string[];
  stacks: string[];
  calibrationUnattached: string[];
  logsUnattached: string[];
  otherFiles: string[];
  /** Videos (AVI, SER…): not processed. */
  videos?: string[];
  /** The text files a capture program wrote beside those videos. */
  videoCaptures?: string[];
  /** Camera RAW files whose metadata cannot be read (a variant, a damaged
   * file): not read. A RAW file that can be read is a pose. */
  rawFiles?: string[];
}

export interface UnassignedGroup {
  key: string;
  designation: string;
  targetId: string | null;
  files: string[];
}

/** The detection of the sending of one session (bounded by its scope). */
export interface ScopedDetection {
  targets: { id: string; designation: string; files: string[] }[];
  unassigned: UnassignedGroup[];
  withoutObject: string[];
  otherNights: number;
}

export interface MultiNightGroup {
  id: string;
  targetKey: string;
  targetId: string | null;
  designation: string;
  instrument: string;
  nights: {
    sessionId: string;
    eveningDate: string;
    frames: number;
    seconds: number;
  }[];
  goalSeconds: number;
  frames: number;
  status: string;
  locked: boolean;
}

export type LineKind =
  | "place"
  | "equipment"
  | "session"
  | "target"
  | "multi-night"
  | "calibration"
  | "log"
  | "clock";

export interface ProposalLine {
  id: string;
  kind: LineKind;
  subject: string;
  checked: boolean;
  locked: boolean;
  ask: boolean;
  /** A stable key (`session.create`, `plan.replace`, …). */
  reason: string;
  /** The core's English text: never shown (the window writes its own, in the
   * language of the window, from `reason` and the data). */
  text: string;
}

export interface Proposal {
  reading: ReadingSummary;
  places: PlaceProposal[];
  equipment: EquipmentProposal[];
  sessions: DetectedSession[];
  multiNight: MultiNightGroup[];
  calibrationSets: CalibrationSet[];
  logs: DetectedLogFile[];
  setAside: SetAside;
  scoped?: ScopedDetection | null;
  lines: ProposalLine[];
  notes: { key: string; text: string }[];
  /** The rights of the account the proposal was computed for. */
  account?: { advancedPlanning: boolean; severalPlansPerNight: boolean };
}

/** The proposal followed to the rights the account holds now
 * (`sessions_refresh_rights`): the multi-night plans open or close without
 * reading the folders again. `unlocked`: the ids of the multi-night lines
 * that just opened (ticked by default); `stale`: the rights changed in a way
 * the proposal cannot follow, the detection must run again. */
export interface RightsFollowed {
  proposal: Proposal;
  /** The rights were not the ones the proposal was computed for. */
  changed: boolean;
  unlocked: string[];
  stale: boolean;
}

export function sessionsRefreshRights(
  proposal: Proposal,
): Promise<RightsFollowed> {
  return invoke<RightsFollowed>("sessions_refresh_rights", { proposal });
}

/** « Lancer la détection » (`sessions_detect`): the three locations read
 * (headers only), the frames grouped, the person's data matched. Nothing is
 * written. `user` carries the places with their `utcOffsetMinutes` (computed
 * by the window from each `timeZone`); `null`: the core reads the person's
 * data from the service. The progress arrives on `astro-sync://local-work`
 * (`kind` `headers`); `localWorkCancel()` stops it without writing. */
export function sessionsDetect(
  request: DetectRequest,
  user: UserData | null,
  locale: string,
): Promise<Proposal> {
  return invoke<Proposal>("sessions_detect", {
    request,
    user,
    catalogue: null,
    locale,
  });
}

export type OrganiseMode = "leave-in-place" | "copy" | "move" | "symlink";

export interface OrganiseAction {
  source: string;
  destination: string;
  kind: "copy" | "move" | "link" | "already-there";
  bytes: number;
  /** `already-there` only: how the file is there, a symbolic link (the
   * treatments link every frame into the working folder) or a file of its
   * own. */
  inPlaceAs?: "link" | "file";
}

export interface OrganiseFailure {
  source: string;
  destination: string;
  reason: string;
  detail?: string | null;
}

export interface OrganisePlan {
  mode: OrganiseMode;
  destination: string;
  actions: OrganiseAction[];
  conflicts: OrganiseFailure[];
  files: number;
  bytes: number;
  windowsNoLink: boolean;
  /** The free space of the destination's disk (« 412 Go libres »). */
  freeBytes?: number | null;
  /** The core's English summary: never shown. */
  summary: string;
}

export interface OrganiseOutcome {
  done: number;
  verified: number;
  removedSources: number;
  alreadyThere: number;
  failed: OrganiseFailure[];
  stopped: boolean;
}

/** The summary before acting (`sessions_organise_plan`, the dry run). */
export function sessionsOrganisePlan(
  proposal: Proposal,
  destination: string,
  mode: OrganiseMode,
  sessions: string[] | null,
  names?: Record<string, NightNaming> | null,
): Promise<OrganisePlan> {
  return invoke<OrganisePlan>("sessions_organise_plan", {
    proposal,
    destination,
    mode,
    sessions,
    names: names ?? null,
  });
}

/** `organise::NightNaming`: a night's folder is `AAAA-MM-JJ · lieu · id`. */
export interface NightNaming {
  place?: string | null;
  sessionId?: string | null;
}

/** The tidying, file by file (`kind` `organise` on the progress event;
 * `localWorkCancelKind("organise")` stops it between two files). */
export function sessionsOrganise(plan: OrganisePlan): Promise<OrganiseOutcome> {
  return invoke<OrganiseOutcome>("sessions_organise", { plan });
}

/** The person's answer for one place of the proposal. */
export type PlaceAnswer =
  | { kind: "known"; placeId: string; knownPlaceId: string }
  | {
      kind: "create";
      placeId: string;
      name: string;
      latitude: number;
      longitude: number;
      timeZone: string | null;
    };

/** « Créer … »: the proposal as ticked. Nothing outside `ticked` is written. */
export interface BulkCreateRequest {
  proposal: Proposal;
  /** The ids of the `ProposalLine`s left ticked. */
  ticked: string[];
  places: PlaceAnswer[];
  /** The lines that ask: `add` or `skip` (no answer: not added). */
  answers: { lineId: string; answer: "add" | "skip" }[];
  /** The whole hours chosen for a suspect clock (session only). */
  shifts: { sessionId: string; hours: number }[];
}

/** An element not written, and why (partial results). */
export interface SkippedElement {
  kind: "session" | "target" | "multi-night" | "place";
  id: string;
  reason: string;
}

export interface CreatedSession {
  detectedId: string;
  sessionId: string;
  eveningDate: string;
  /** `created` (« Importée par Astro Sync ») or `updated`. */
  action: "created" | "updated";
  /** `created`, `reused`, `replaced`, `none`. */
  planAction: "created" | "reused" | "replaced" | "none";
  planId: string | null;
  designations: string[];
  frames: number;
  shiftHours: number;
  /** The targets added to an existing session. */
  addedTargets?: string[];
}

export interface CreatedMultiNight {
  groupId: string;
  planId: string;
  designation: string;
  sessionIds: string[];
  goalSeconds: number;
}

export interface BulkCreateOutcome {
  sessions: CreatedSession[];
  multiNight: CreatedMultiNight[];
  placesCreated: string[];
  /** What was not written, with the reason (partial results). */
  skipped?: SkippedElement[];
  /** The same choices were already sent: nothing new was written. */
  replayed?: boolean;
}

/** `sessions_create_bulk`: the service's `POST /v1/sessions/bulk`, one
 * idempotent call (a retry replays). */
export function sessionsCreateBulk(
  request: BulkCreateRequest,
): Promise<BulkCreateOutcome> {
  return invoke<BulkCreateOutcome>("sessions_create_bulk", { request });
}

/** The person's places, equipment and plans (read only), for the window to
 * add each place's offset from UTC before `sessionsDetect`. */
export function sessionsUserData(): Promise<UserData> {
  return invoke<UserData>("sessions_user_data");
}

/* ---------------------- « Détecter les sessions » ---------------------- */

/** `session_detection::FoundSoFar`: « Trouvé jusqu'ici » during the scan. */
export interface FoundSoFar {
  nights: number;
  targets: number;
  places: number;
  instruments: number;
  lights: number;
  calibrationFiles: number;
  calibrationSets: number;
  calibrationKinds: ("dark" | "flat" | "dark-flat" | "bias")[];
  logFiles: number;
  logKinds: ("nina" | "asiair" | "phd2")[];
  toSolve: number;
}

export const DETECT_FOUND_EVENT = "detect-found";

export function onDetectFound(
  handler: (found: FoundSoFar) => void,
): Promise<UnlistenFn> {
  return listen<FoundSoFar>(DETECT_FOUND_EVENT, (event) =>
    handler(event.payload),
  );
}

/** `GET /v1/space/storage` (read only): « Place restante : 2 images sur 3 ». */
export function spaceStorageGet(): Promise<SpaceStorage> {
  return invoke<SpaceStorage>("space_storage_get");
}

/** `session_detection::TreatmentNight`: a session created, as a night folder
 * of the working folder, ready for its treatments. */
export interface TreatmentNight {
  detectedId: string;
  sessionId: string;
  eveningDate: string;
  nightFolder: string;
  longitude: number | null;
  targets: {
    targetId: string;
    designation: string;
    folderName: string;
    frames: number;
  }[];
  linked: number;
  alreadyThere: number;
  conflicts: number;
}

/** « Lancer les traitements »: the nights of the sessions created that hold
 * one of `targetIds`, prepared in the working folder (frames linked). */
export function detectTreatmentsPrepare(
  proposal: Proposal,
  created: BulkCreateOutcome,
  targetIds: string[],
): Promise<TreatmentNight[]> {
  return invoke<TreatmentNight[]>("detect_treatments_prepare", {
    proposal,
    created,
    targetIds,
  });
}

/* -------------- Nothing in the space before step 4 ------------ */

/** What « Créer … » would make of the review, computed on this computer: a
 * provisional identifier (`local-<hash>`) for each session and plan to
 * create. Nothing is asked of the service, nothing is written. */
export function sessionsProvisional(
  request: BulkCreateRequest,
): Promise<BulkCreateOutcome> {
  return invoke<BulkCreateOutcome>("sessions_provisional", { request });
}

/** `session_detection::provisional::Settled`: a provisional folder renamed
 * after what the space created. */
export interface Settled {
  fromId: string;
  toId: string;
  from: string;
  to: string;
}

/** `commands_detect::ImportCreated`. */
export interface ImportCreated {
  outcome: BulkCreateOutcome;
  nights: Settled[];
}

/** Step 4's first stage: the sessions and plans created (one idempotent
 * call), the night folders renamed after their session. */
export function detectImportCreate(
  request: BulkCreateRequest,
  provisional: BulkCreateOutcome,
): Promise<ImportCreated> {
  return invoke<ImportCreated>("detect_import_create", {
    request,
    provisional,
  });
}

/** Step 4's second stage: the plan folders renamed after the plans created. */
export function detectImportPlans(nights: Settled[]): Promise<Settled[]> {
  return invoke<Settled[]>("detect_import_plans", { nights });
}

/** « Abandonner l'import », confirmed: the provisional folders removed
 * (links and Astro Sync's own files only). */
export function detectImportDiscard(
  ids: string[],
): Promise<{ removed: number; kept: number }> {
  return invoke<{ removed: number; kept: number }>("detect_import_discard", {
    ids,
  });
}

/** A multi-night plan the import will create: its provisional identifier,
 * its target and the import's nights. */
export interface LocalPlan {
  projectId: string;
  targetId: string;
  name: string;
  nights: string[];
}

/** « Image du plan multi-nuits » of a plan the import will create, from the
 * masters of its nights; nothing is asked of the space, nothing is queued. */
export function planImageRunLocal(
  plan: LocalPlan,
  pipelines: { pipelineId: string; trustedSha256: string | null }[],
  progression = false,
): Promise<PlanImageRun> {
  return invoke<PlanImageRun>("plan_image_run_local", {
    plan,
    pipelines,
    progression,
  });
}

/** One image ticked in « Images à envoyer ». */
export type ImageToSend =
  | {
      kind: "night";
      sessionId: string;
      targetId: string;
      render: StackedRender;
      /** The digest of the image of the same sending that creates the
       * capture this render is a variant of; `null`: this render is the
       * capture. */
      variantOfSha256?: string | null;
    }
  | {
      kind: "plan";
      outcome: PlanImageOutcome;
      /** As for a night's render, the digest of the image of the same
       * sending that creates the capture of the plan this render is a
       * variant of; `null`: this render is the capture. */
      variantOfSha256?: string | null;
      /** The image of the plan after each night, made with this image: it
       * leaves as the history of the capture when this image is that capture. */
      progression?: PlanProgressionStep[];
    };

/** « Envoyer N images »: the images ticked, queued for the sending. */
export function detectImagesSend(
  images: ImageToSend[],
): Promise<EnqueueOutcome> {
  return invoke<EnqueueOutcome>("detect_images_send", { images });
}

/** « Ouvrir »: an image of « Images à envoyer » opened by the system's usual
 * application. */
export function resultFileOpen(path: string): Promise<void> {
  return invoke<void>("result_file_open", { path });
}

/** « Afficher dans le Finder »: the image selected in its folder. */
export function resultFileReveal(path: string): Promise<void> {
  return invoke<void>("result_file_reveal", { path });
}

/** The PNG written beside a TIFF render to look at (the one with the curve of
 * presentation when there is one), as bytes; `null` when there is none. */
export async function resultFilePreview(path: string): Promise<Blob | null> {
  const bytes = await invoke<ArrayBuffer>("result_file_preview", { path });
  return bytes.byteLength > 0 ? new Blob([bytes], { type: "image/png" }) : null;
}
