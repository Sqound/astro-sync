// « Détecter les sessions » as one wizard of five steps: 1 Dossier · 2 Nuits et
// matériel · 3 Traitements · 4 Récapitulatif · 5 Terminé. The state lives above
// the screens (`ImportProvider`), so a click in the menu never loses an import,
// and the jobs (the scan, the treatments, the creation and the sending) keep their
// result wherever the person is. Nothing is written to the space before step 4:
// « Continuer » of step 2 computes, on this computer, what would be created
// (`local`, provisional identifiers), the treatments of step 3 run on it, and the
// primary of step 4 creates the sessions and the plans, adds the incidents, tidies
// the files and queues the images, in that order. Pure TypeScript: the reducer,
// where each step may go, the foot of each step, what the menu and « Sessions »
// say of an import -- tested under `node --test`.

import type {
  BulkCreateOutcome,
  BulkCreateRequest,
  FoundSoFar,
  ImageToSend,
  Proposal,
  Settled,
  SpaceStorage,
  UserData,
} from "./bridge";
import {
  EMPTY_DETECT_WINDOW,
  scanPercent,
  type DetectWindowState,
} from "./detectFlow.ts";
import type { ReviewState } from "./detectReview";
import {
  draftPlan,
  draftReducer,
  EMPTY_DRAFT,
  writesSomething,
  type DraftAction,
  type IncidentsDraft,
  type IncidentsPlan,
} from "./incidentsDraft.ts";
import {
  canTickImage,
  imageRoom,
  summaryImages,
  treatmentCards,
  type SummaryImage,
  type TreatmentResult,
  type TreatmentTargetCard,
} from "./detectTreatments.ts";
import type { TargetFinishes } from "./finish";
import type { TargetRenders } from "./pipelineChoice";
import { pluralText } from "./written.ts";
import {
  NO_TIDY,
  type TidyChoice,
  type TidyResult,
  type TidyState,
} from "./tidy.ts";
import type { TreatmentProgress } from "./treatmentProgress";

export type ImportStep = 1 | 2 | 3 | 4 | 5;

export const IMPORT_STEPS: readonly ImportStep[] = [1, 2, 3, 4, 5];

/** The words of the thread, short enough for five steps on one line. */
export const IMPORT_STEP_LABEL_KEY: Record<ImportStep, string> = {
  1: "import.step.folder",
  2: "import.step.nights",
  3: "import.step.treatments",
  4: "import.step.summary",
  5: "import.step.done",
};

/** What « Créer … » brings back: the request as ticked and what the service
 * made of it -- or, before step 4 (`ImportState.local`), what it would make
 * of it, each session and plan to create with a provisional identifier. */
export interface Created {
  request: BulkCreateRequest;
  outcome: BulkCreateOutcome;
  /** The provisional night folders renamed after their session (step 4). */
  nights?: Settled[];
}

/** An image the treatments made for one target (one per pipeline), and how
 * it would leave. */
export interface Made extends TreatmentResult {
  send: ImageToSend;
}

/** The scan: first it looks for the files (`total` is 0 while it does:
 * nothing can be measured yet, the window says what it finds), then it reads
 * their headers. */
export interface ScanProgress {
  read: number;
  total: number;
  current: string | null;
  found: FoundSoFar | null;
  /** The files met by the search so far, and the folder it is in. */
  walked: number;
  folder: string | null;
}

/** What the primary of step 4 did to the incidents of one night. */
export interface IncidentsResult {
  /** Incidents written (added, or already there, or replacing). */
  added: number;
  /** Signalements replaced by a detection. */
  replaced: number;
  /** Incidents the service did not take. */
  rejected: number;
  /** The service refused the replacements (the session moved on). */
  refused: boolean;
}

/** What the report of a night's logs came to: sent (`null`), or why not. */
export type LogsResult = { sent: true } | { sent: false; reason: string };

/** What step 4's primary did, as « Terminé » reports it: the plan folders
 * renamed after the plans created, the incidents of each session (by the
 * session created), the tidying, the report of each night's logs (by the
 * session created). The sessions created are `created`. */
export interface RecapReport {
  plans: Settled[] | null;
  incidents: Record<string, IncidentsResult>;
  tidy: TidyResult | null;
  logs: Record<string, LogsResult>;
}

export const NO_REPORT: RecapReport = {
  plans: null,
  incidents: {},
  tidy: null,
  logs: {},
};

/** The stages of step 4's primary, in their order: the sessions created,
 * the multi-night plan, the incidents, the tidying, the sending. */
export type RecapWork =
  "sessions" | "plan" | "incidents" | "tidy" | "send" | "logs";

/** The report of the logs comes last: the images never wait for it, and a
 * report that fails is said without undoing anything (as « Journaux » of the
 * night's wizard, ticked by default). */
export const RECAP_STAGES: readonly RecapWork[] = [
  "sessions",
  "plan",
  "incidents",
  "tidy",
  "send",
  "logs",
];

export interface ImportState {
  /** An import is open (from « Détecter les sessions » to « Terminé » left). */
  alive: boolean;
  step: ImportStep;
  /** The furthest step reached: the steps up to it stay done in the thread. */
  reached: ImportStep;
  /** The three folders and « Lire aussi les sous-dossiers ». */
  choices: DetectWindowState;
  /** The last folders were read once (the window opens on them). */
  choicesRead: boolean;
  /** Why the last gesture of the current step could not be done. */
  error: string | null;
  /** The scan running, or `null`. */
  scan: ScanProgress | null;
  proposal: Proposal | null;
  user: UserData | null;
  review: ReviewState | null;
  /** « Continuer » of step 2 computes what would be created. */
  creating: boolean;
  /** What step 4 will create, computed on this computer when step 2 is left
   * (provisional identifiers): the treatments of step 3 run on it. */
  local: Created | null;
  /** What the space created, by the first stage of step 4's primary. */
  created: Created | null;
  /** What the service said of the space: the place left, the ceiling of a
   * file, the variants (`null` while unknown: no guard). */
  storage: SpaceStorage | null;
  /** The targets ticked in « Traitements » (`null`: not pre-ticked yet). */
  chosen: string[] | null;
  renders: TargetRenders;
  /** The finish of a pipeline set by hand, by target then pipeline
   * (« Finition » of the window); absent: the « Finition par défaut ». */
  finishes?: TargetFinishes;
  /** The target being treated while the treatments run. */
  running: { target: string } | null;
  /** The folder the treatments made for each target (`<night>/<target>`),
   * once they prepared it: where « Ouvrir le dossier de travail » goes. */
  folders: Record<string, string>;
  /** The folder of each night the treatments prepared, by the night's id in
   * the proposal (`detectedId`): its `logs/` is what the report of the logs
   * reads. */
  nightFolders: Record<string, string>;
  /** « Arrêter les traitements » was asked: the run ends on step 3. */
  stopping: boolean;
  frame: TreatmentProgress | null;
  made: Made[];
  /** A run of the treatments ended (its images may be none). */
  treated: boolean;
  treatErrors: string[];
  /** The targets the offer refused to stack in the last run. */
  treatRefused: number;
  /** The images ticked in « Images à envoyer » (`imageKey`). */
  ticked: string[];
  /** « Joindre aussi les TIFF » above the images: their TIFF rows ticked
   * with it. It starts from « Envoi des rendus » of Réglages. */
  joinTiff: boolean;
  sending: boolean;
  /** « Terminé »: the images that left (0 when none did). */
  sent: number | null;
  /** Of the images that left, how many go as a variant of another
   * one. */
  sentVariants: number;
  /** « Liste des incidents » of each session created: what was read and what
   * the person ticked, kept while they come and go; written by the primary of
   * step 4, never before. */
  incidents: Record<string, IncidentsDraft>;
  /** « Ranger les poses dans le dossier de travail » (unticked at first). */
  tidy: TidyChoice;
  /** What the working folder holds already (`null`: not read yet). */
  tidyOverview: TidyState | null;
  /** The part of the primary that runs while `sending`. */
  working: RecapWork | null;
  /** The run of the primary began before the sessions were created: its
   * button keeps its words (« Créer et envoyer ») while it runs. */
  runCreates: boolean;
  report: RecapReport;
}

export const NO_IMPORT: ImportState = {
  alive: false,
  step: 1,
  reached: 1,
  choices: EMPTY_DETECT_WINDOW,
  choicesRead: false,
  error: null,
  scan: null,
  proposal: null,
  user: null,
  review: null,
  creating: false,
  local: null,
  created: null,
  storage: null,
  chosen: null,
  renders: {},
  running: null,
  folders: {},
  nightFolders: {},
  stopping: false,
  frame: null,
  made: [],
  treated: false,
  treatErrors: [],
  treatRefused: 0,
  ticked: [],
  joinTiff: false,
  sending: false,
  sent: null,
  sentVariants: 0,
  incidents: {},
  tidy: NO_TIDY,
  tidyOverview: null,
  working: null,
  runCreates: false,
  report: NO_REPORT,
};

export type ImportAction =
  | { type: "open" }
  | { type: "close" }
  | { type: "choicesRead"; choices: DetectWindowState }
  | { type: "choices"; choices: DetectWindowState }
  | { type: "scanStart" }
  | { type: "scanProgress"; read: number; total: number; current?: string }
  /** The search for the files, before any header: how many met, in which folder. */
  | { type: "scanWalk"; found: number; folder?: string }
  | { type: "scanFound"; found: FoundSoFar }
  | {
      type: "scanDone";
      proposal: Proposal;
      user: UserData;
      review: ReviewState;
    }
  /** The scan ended without a proposal: stopped (`error` null) or failed. */
  | { type: "scanEnd"; error: string | null }
  /** The account's rights changed since the detection: the proposal followed
   * them (`sessions_refresh_rights`), the review keeps what was decided. */
  | { type: "rightsFollowed"; proposal: Proposal; review: ReviewState }
  | { type: "review"; update: (review: ReviewState) => ReviewState }
  /** « Continuer » of step 2: what would be created is being computed. */
  | { type: "createStart" }
  /** ... and here it is: step 3 opens on it. */
  | { type: "planned"; local: Created }
  | { type: "createFailed"; error: string }
  /** Step 4's first stage: the space created the sessions and plans. */
  | { type: "created"; created: Created }
  /** Step 4's second stage: the plan folders renamed. */
  | { type: "plansSettled"; plans: Settled[] }
  | { type: "goTo"; step: ImportStep }
  | { type: "storage"; storage: SpaceStorage | null }
  | { type: "toggleTarget"; targetId: string }
  | { type: "renders"; renders: TargetRenders }
  | { type: "finishes"; finishes: TargetFinishes }
  | { type: "treatStart"; target: string }
  | { type: "treatTarget"; target: string }
  /** The folders of the targets, once the treatments prepared them. */
  | {
      type: "treatFolders";
      folders: Record<string, string>;
      /** The night folders, by `detectedId`. */
      nights?: Record<string, string>;
    }
  | { type: "frame"; change: (frame: TreatmentProgress) => TreatmentProgress }
  | { type: "frameStart"; frame: TreatmentProgress }
  | { type: "treatStop" }
  | {
      type: "treatDone";
      made: Made[];
      errors: string[];
      /** The targets the offer refused (the same for all of them). */
      refused?: number;
    }
  /** « Continuer sans traitement » / « Continuer avec les images faites ». */
  | { type: "skip" }
  | { type: "toggleImage"; key: string }
  /** « Joindre aussi les TIFF »: every TIFF row ticked (as the place left
   * allows) or unticked. */
  | { type: "joinTiff"; on: boolean }
  /** A choice on the list of a session's incidents. */
  | { type: "incidents"; sessionId: string; action: DraftAction }
  | { type: "tidyTick"; on: boolean }
  /** The sheet « Voir ce qui sera rangé » keeps how and where. */
  | { type: "tidyChoice"; choice: TidyChoice }
  | { type: "tidyOverview"; overview: TidyState | null }
  | { type: "sendStart" }
  | { type: "working"; working: RecapWork }
  | { type: "incidentsApplied"; sessionId: string; result: IncidentsResult }
  | { type: "tidyApplied"; result: TidyResult }
  /** The report of a night's logs, sent or not (by the session created). */
  | { type: "logsSent"; sessionId: string; result: LogsResult }
  | { type: "sendFailed"; error: string }
  | { type: "sent"; count: number; variants: number };

const later = (a: ImportStep, b: ImportStep): ImportStep =>
  (a > b ? a : b) as ImportStep;

export function importReducer(
  state: ImportState,
  action: ImportAction,
): ImportState {
  switch (action.type) {
    case "open":
      return state.alive ? state : { ...NO_IMPORT, alive: true };
    case "close":
      // The folders chosen stay for the next import (they are also kept on
      // disk, `localSettingsSetLastFolders`).
      return { ...NO_IMPORT, choices: state.choices, choicesRead: true };
    case "choicesRead":
      return state.choicesRead
        ? state
        : { ...state, choices: action.choices, choicesRead: true };
    case "choices":
      return { ...state, choices: action.choices, choicesRead: true };
    case "scanStart":
      return {
        ...state,
        alive: true,
        step: 1,
        error: null,
        scan: {
          read: 0,
          total: 0,
          current: null,
          found: null,
          walked: 0,
          folder: null,
        },
      };
    case "scanWalk":
      return state.scan && state.scan.total <= 0
        ? {
            ...state,
            scan: {
              ...state.scan,
              walked: action.found,
              folder: action.folder ?? state.scan.folder,
            },
          }
        : state;
    case "scanProgress":
      return state.scan
        ? {
            ...state,
            scan: {
              ...state.scan,
              read: action.read,
              total: action.total,
              current: action.current ?? state.scan.current,
            },
          }
        : state;
    case "scanFound":
      return state.scan
        ? { ...state, scan: { ...state.scan, found: action.found } }
        : state;
    case "scanDone":
      // A new reading is a new review: what was answered on the last one
      // does not fit these nights.
      return {
        ...state,
        scan: null,
        proposal: action.proposal,
        user: action.user,
        review: action.review,
        step: 2,
        reached: 2,
      };
    case "scanEnd":
      return { ...state, scan: null, step: 1, error: action.error };
    case "rightsFollowed":
      // Only a review still open: once created, the request has been made.
      return state.proposal && state.review && !reviewReadOnly(state)
        ? { ...state, proposal: action.proposal, review: action.review }
        : state;
    case "review":
      return state.review && !reviewReadOnly(state)
        ? { ...state, review: action.update(state.review) }
        : state;
    case "createStart":
      return { ...state, creating: true, error: null };
    case "planned":
      return withLocal(state, action.local);
    case "createFailed":
      return { ...state, creating: false, error: action.error };
    case "created":
      return {
        ...state,
        created: action.created,
        // The proposal as sent: the bands answered are its filters.
        proposal: action.created.request.proposal,
      };
    case "plansSettled":
      return { ...state, report: { ...state.report, plans: action.plans } };
    case "goTo":
      return canGoTo(state, action.step)
        ? {
            ...state,
            step: action.step,
            reached: later(state.reached, action.step),
            error: null,
          }
        : state;
    case "storage":
      return { ...state, storage: action.storage };
    case "toggleTarget": {
      if (state.running) return state;
      const held = state.chosen ?? [];
      return {
        ...state,
        chosen: held.includes(action.targetId)
          ? held.filter((id) => id !== action.targetId)
          : [...held, action.targetId],
      };
    }
    case "renders":
      return state.running ? state : { ...state, renders: action.renders };
    case "finishes":
      return state.running ? state : { ...state, finishes: action.finishes };
    case "treatStart":
      return {
        ...state,
        running: { target: action.target },
        stopping: false,
        treatErrors: [],
        treatRefused: 0,
        error: null,
      };
    case "treatTarget":
      return state.running
        ? { ...state, running: { target: action.target } }
        : state;
    case "treatFolders":
      return {
        ...state,
        folders: { ...state.folders, ...action.folders },
        nightFolders: { ...state.nightFolders, ...(action.nights ?? {}) },
      };
    case "frameStart":
      return { ...state, frame: action.frame };
    case "frame":
      return state.frame
        ? { ...state, frame: action.change(state.frame) }
        : state;
    case "treatStop":
      return state.running ? { ...state, stopping: true } : state;
    case "treatDone": {
      const stopped = state.stopping;
      const ticked = preTickedImages(
        summaryImages(action.made, state.storage),
        imageRoom(state.storage),
        state.joinTiff,
      );
      return {
        ...state,
        running: null,
        stopping: false,
        frame: null,
        made: action.made,
        treated: true,
        treatErrors: action.errors,
        treatRefused: action.refused ?? 0,
        ticked,
        // A run that ends opens the summary; one that was stopped stays on
        // « Traitements ».
        step: stopped ? 3 : 4,
        reached: stopped ? state.reached : later(state.reached, 4),
      };
    }
    case "skip":
      if (state.step !== 3 || !state.local || state.running) return state;
      return {
        ...state,
        step: 4,
        reached: later(state.reached, 4),
        // Skipped before any run: nothing to send.
        ticked: state.treated ? state.ticked : [],
        error: null,
      };
    case "joinTiff": {
      const rows = summaryImages(state.made, state.storage);
      const room = imageRoom(state.storage);
      const others = state.ticked.filter(
        (key) => !rows.some((row) => row.key === key && row.fullResolution),
      );
      return {
        ...state,
        joinTiff: action.on,
        ticked: action.on ? withTiffTicked(rows, others, room) : others,
      };
    }
    case "toggleImage":
      return {
        ...state,
        ticked: state.ticked.includes(action.key)
          ? state.ticked.filter((key) => key !== action.key)
          : [...state.ticked, action.key],
      };
    case "incidents": {
      const draft = draftReducer(
        state.incidents[action.sessionId] ?? EMPTY_DRAFT,
        action.action,
      );
      return state.incidents[action.sessionId] === draft
        ? state
        : {
            ...state,
            incidents: { ...state.incidents, [action.sessionId]: draft },
          };
    }
    case "tidyTick":
      return state.sending
        ? state
        : { ...state, tidy: { ...state.tidy, on: action.on } };
    case "tidyChoice":
      return state.sending ? state : { ...state, tidy: action.choice };
    case "tidyOverview":
      return { ...state, tidyOverview: action.overview };
    case "sendStart":
      return {
        ...state,
        sending: true,
        working: null,
        error: null,
        runCreates: state.created === null,
      };
    case "working":
      return { ...state, working: action.working };
    case "incidentsApplied":
      return {
        ...state,
        report: {
          ...state.report,
          incidents: {
            ...state.report.incidents,
            [action.sessionId]: action.result,
          },
        },
      };
    case "tidyApplied":
      return { ...state, report: { ...state.report, tidy: action.result } };
    case "logsSent":
      return {
        ...state,
        report: {
          ...state.report,
          logs: { ...state.report.logs, [action.sessionId]: action.result },
        },
      };
    case "sendFailed":
      return { ...state, sending: false, working: null, error: action.error };
    case "sent":
      return {
        ...state,
        sending: false,
        working: null,
        sent: action.count,
        sentVariants: action.variants,
        step: 5,
        reached: 5,
      };
  }
}

/**
 * « Continuer » of step 2 brought what would be created: step 3 opens on it.
 * Coming back to step 2 and changing a choice keeps what the treatments made
 * for what is still there -- a target that left, a plan no longer created,
 * take their images with them -- and the targets ticked.
 */
function withLocal(state: ImportState, local: Created): ImportState {
  const cards = treatmentCards(local.request.proposal, local.outcome);
  const before = state.local
    ? treatmentCards(state.local.request.proposal, state.local.outcome).map(
        (card) => card.targetId,
      )
    : null;
  const chosen =
    before === null || state.chosen === null
      ? preChosenTargets(cards)
      : [
          ...state.chosen.filter((id) =>
            cards.some((card) => card.targetId === id),
          ),
          ...preChosenTargets(cards).filter((id) => !before.includes(id)),
        ];
  const made = state.made.filter((item) => {
    const card = cards.find((c) => c.targetId === item.targetId);
    if (!card) return false;
    const send = item.send;
    return send.kind === "night"
      ? card.nights.some((night) => night.sessionId === send.sessionId)
      : card.multiNight?.planId === send.outcome.projectId;
  });
  const keys = new Set(summaryImages(made, state.storage).map((r) => r.key));
  return {
    ...state,
    creating: false,
    local,
    // The proposal as it will be sent: the bands answered are its filters.
    proposal: local.request.proposal,
    chosen,
    made,
    ticked: state.ticked.filter((key) => keys.has(key)),
    step: 3,
    reached: later(state.reached, 3),
  };
}

/** The prefix of a provisional identifier (`local-<hash>`): a session or a
 * plan step 4 has not created yet. */
export const PROVISIONAL_PREFIX = "local-";

export function isProvisionalId(id: string): boolean {
  return id.startsWith(PROVISIONAL_PREFIX);
}

/** The provisional sessions and plans of the import: their folders are what
 * « Abandonner l'import » removes. */
export function provisionalIds(state: ImportState): string[] {
  const outcome = state.local?.outcome;
  if (!outcome) return [];
  return [
    ...outcome.sessions.map((s) => s.sessionId),
    ...outcome.multiNight.map((m) => m.planId),
  ].filter(isProvisionalId);
}

/** What steps 3 and 4 show: what the space created, or before that what it
 * would create (provisional identifiers). */
export function workingOutcome(state: ImportState): Created | null {
  return state.created ?? state.local;
}

/** A job of the import is running: nothing else may start, the thread and
 * « Précédent » wait. */
export function isBusy(state: ImportState): boolean {
  return (
    state.scan !== null ||
    state.creating ||
    state.running !== null ||
    state.sending
  );
}

/**
 * Whether the person may open `step` from where the import is:
 * - never while a job runs (the scan, the treatments -- they stop with their
 *   own button first --, step 4's primary) nor once it is over;
 * - « Dossier » only while nothing is created (a new reading is a new
 *   import);
 * - « Nuits et matériel » once read (read-only once created: nothing is
 *   undone);
 * - « Traitements » once step 2 was left with « Continuer », until the
 *   sessions are created (its folders then carry them);
 * - « Récapitulatif » once treated or reached.
 */
export function canGoTo(state: ImportState, step: ImportStep): boolean {
  if (!state.alive || isBusy(state) || state.sent !== null) return false;
  if (step === state.step) return false;
  switch (step) {
    case 1:
      return state.created === null;
    case 2:
      return state.proposal !== null;
    case 3:
      return state.local !== null && state.created === null;
    case 4:
      return state.local !== null && (state.treated || state.reached >= 4);
    case 5:
      return false;
  }
}

/** The step « Précédent » opens, or `null` (step 1, a job running, step 5,
 * or nothing to go back to). */
export function previousOf(state: ImportState): ImportStep | null {
  const before = (state.step - 1) as ImportStep;
  return state.step > 1 && canGoTo(state, before) ? before : null;
}

/** The thread: done before the current one (and the ones reached after it,
 * when the person went back), each done step reopenable when `canGoTo`. */
export function importThread(state: ImportState): {
  step: ImportStep;
  status: "done" | "current" | "waiting";
  reopens: boolean;
}[] {
  return IMPORT_STEPS.map((step) => {
    const status =
      step === state.step
        ? "current"
        : step < state.step ||
            step <= state.reached ||
            // « Terminé » ticks them all.
            state.sent !== null
          ? "done"
          : "waiting";
    return {
      step,
      status,
      reopens: status === "done" && canGoTo(state, step),
    };
  });
}

/** The review is shown read-only: the sessions are created (or being
 * created), nothing is undone or created again. Before step 4 it stays open:
 * nothing is written yet. */
export function reviewReadOnly(state: ImportState): boolean {
  return state.created !== null || state.sending;
}

/* ---------------------------------- Foot ---------------------------------- */

export type FootSecondary =
  /** Step 1 while reading: « Arrêter la détection ». */
  | "stopScan"
  /** Step 3: « Continuer sans traitement ». */
  | "skipTreatments"
  /** Step 3 after a run: « Continuer avec les images faites ». */
  | "keepImages"
  /** Step 3 while treating: « Arrêter les traitements ». */
  | "stopTreatments"
  /** Step 4 with images ticked: « Terminer sans envoyer ». */
  | "finishWithoutSending"
  /** The same before the creation: « Créer sans envoyer ». */
  | "createWithoutSending";

export type FootPrimary =
  | "startScan"
  /** Step 2: « Continuer » (nothing is written). */
  | "create"
  | "continue"
  | "treat"
  /** Step 4 before the creation: « Créer et envoyer », or « Créer » with no
   * image ticked. */
  | "createSend"
  | "createOnly"
  | "send"
  | "finish"
  | "backToSessions";

export interface FootRule {
  /** « Précédent » (a link, far left). */
  previous: ImportStep | null;
  /** The secondary action (a tertiary button). */
  secondary: FootSecondary | null;
  /** The step's one primary, greyed when it cannot go. */
  primary: { kind: FootPrimary; enabled: boolean } | null;
}

/**
 * The foot of the step shown: the message above, then
 * « Précédent » (link, absent on step 1, while a job runs and on step 5), the
 * secondary (a tertiary button, never a second link) and the primary.
 * `canCreate`: the review has something to create (`createLabel` is not
 * `none`); `ready`: the folders let the detection start. The stacking of
 * step 3 is local and never gated by the offer.
 */
export function footRule(
  state: ImportState,
  options: { ready?: boolean; canCreate?: boolean } = {},
): FootRule {
  const previous = previousOf(state);
  switch (state.step) {
    case 1:
      return state.scan
        ? { previous: null, secondary: "stopScan", primary: null }
        : {
            previous: null,
            secondary: null,
            primary: { kind: "startScan", enabled: options.ready === true },
          };
    case 2:
      return reviewReadOnly(state)
        ? {
            previous,
            secondary: null,
            primary: { kind: "continue", enabled: true },
          }
        : {
            previous: state.creating ? null : previous,
            secondary: null,
            primary: {
              kind: "create",
              enabled: !state.creating && options.canCreate === true,
            },
          };
    case 3: {
      if (state.running)
        return { previous: null, secondary: "stopTreatments", primary: null };
      const chosen = state.chosen ?? [];
      return {
        previous,
        secondary: state.treated ? "keepImages" : "skipTreatments",
        primary: { kind: "treat", enabled: chosen.length > 0 },
      };
    }
    case 4: {
      const creates =
        state.created === null || (state.sending && state.runCreates);
      const images = state.ticked.length > 0;
      return {
        previous: state.sending ? null : previous,
        secondary:
          images && !state.sending
            ? creates
              ? "createWithoutSending"
              : "finishWithoutSending"
            : null,
        primary: {
          kind: creates
            ? images
              ? "createSend"
              : "createOnly"
            : images
              ? "send"
              : "finish",
          // The lists of incidents are being read: what is ticked is not
          // known yet.
          enabled: !state.sending && !incidentsLoading(state),
        },
      };
    }
    case 5:
      return {
        previous: null,
        secondary: null,
        primary: { kind: "backToSessions", enabled: true },
      };
  }
}

/* --------------------- What the primary of step 4 applies ------------------ */

/** A list of incidents is being read: the primary waits for it. */
export function incidentsLoading(state: ImportState): boolean {
  return Object.values(state.incidents).some(
    (draft) => draft.status === "loading",
  );
}

/** What the primary of step 4 applies, when the person goes to the next step
 * (nothing is written before it): the sessions and the plans to create, the incidents ticked of
 * every night -- not only the one shown --, the tidying when its option is
 * ticked and some file is left to put in place, the images. */
export interface RecapPlan {
  /** The sessions to create and to update, the plans to create (0 once
   * created). */
  sessions: number;
  updated: number;
  plans: number;
  /** The nights with something to write, in the order of the sessions. */
  incidents: { sessionId: string; plan: IncidentsPlan }[];
  /** Incidents written in all (the detections added or replacing). */
  incidentCount: number;
  isolated: number;
  /** The tidying to ask the engine for, or `null`: the option is ticked, or
   * every file is already in place (the report then says so). */
  tidy: TidyChoice | null;
  /** The files the tidying would put in place (0 when ticked off). */
  tidyFiles: number;
  images: number;
  /** The nights whose logs leave as their report (the guiding, the poses, the
   * steps of the night), by the session as the proposal made it and the
   * folder the treatments prepared: a night with logs, prepared. */
  logs: NightLogs[];
}

/** A night whose logs leave: its session (provisional before step 4 creates
 * it) and its folder (renamed with it). */
export interface NightLogs {
  sessionId: string;
  folder: string;
}

/**
 * The nights of the import whose report of the logs leaves, as « Journaux »
 * of the night's wizard is ticked by default: each night of the proposal
 * that holds logs, once the treatments prepared its folder (the report reads
 * its `logs/` and its lights). A night the space will not hold (no session)
 * is left.
 */
export function nightsWithLogs(state: ImportState): NightLogs[] {
  const sessions = state.proposal?.sessions ?? [];
  const outcome = state.local?.outcome.sessions ?? [];
  const logs: NightLogs[] = [];
  for (const session of sessions) {
    if (!session.logs?.length) continue;
    const folder = state.nightFolders[session.id];
    const made = outcome.find((s) => s.detectedId === session.id);
    if (!folder || !made) continue;
    logs.push({ sessionId: made.sessionId, folder });
  }
  return logs;
}

export function recapPlan(state: ImportState, separator: string): RecapPlan {
  const outcome = workingOutcome(state)?.outcome;
  const order = outcome?.sessions.map((s) => s.sessionId) ?? [];
  const toCreate = state.created === null ? (outcome ?? null) : null;
  const known = Object.keys(state.incidents);
  const ids = [
    ...order.filter((id) => known.includes(id)),
    ...known.filter((id) => !order.includes(id)),
  ];
  const incidents: RecapPlan["incidents"] = [];
  let incidentCount = 0;
  let isolated = 0;
  for (const sessionId of ids) {
    const draft = state.incidents[sessionId];
    if (draft.status !== "ready") continue;
    const plan = draftPlan(draft, separator);
    if (!writesSomething(plan)) continue;
    incidents.push({ sessionId, plan });
    incidentCount += plan.adds.length + plan.replacements.length;
    isolated += plan.summary.isolated;
  }
  const inPlace = state.tidyOverview?.kind === "done";
  return {
    sessions:
      toCreate?.sessions.filter((s) => s.action === "created").length ?? 0,
    updated:
      toCreate?.sessions.filter((s) => s.action === "updated").length ?? 0,
    plans: toCreate?.multiNight.length ?? 0,
    incidents,
    incidentCount,
    isolated,
    tidy: state.tidy.on || inPlace ? { ...state.tidy, on: true } : null,
    tidyFiles: state.tidy.on ? (state.tidyOverview?.toDo ?? 0) : 0,
    images: state.ticked.length,
    logs: nightsWithLogs(state),
  };
}

/** The sentence above the foot of step 4: what the primary will do -- the
 * images it sends, the incidents it adds, the tidying -- or that it does
 * nothing but go on. */
export function recapLine(
  plan: RecapPlan,
  t: Translate,
  locale: string,
): string {
  const parts = [
    plan.sessions > 0
      ? pluralText(t, locale, "import.recap.sessions", plan.sessions)
      : null,
    plan.updated > 0
      ? pluralText(t, locale, "import.recap.updated", plan.updated)
      : null,
    plan.plans > 0
      ? pluralText(t, locale, "import.recap.plans", plan.plans)
      : null,
    plan.images > 0
      ? pluralText(t, locale, "import.recap.images", plan.images)
      : null,
    plan.incidentCount > 0
      ? pluralText(t, locale, "import.recap.incidents", plan.incidentCount)
      : null,
    plan.tidy && plan.tidyFiles > 0 ? t("import.recap.tidy") : null,
  ].filter((part): part is string => part !== null);
  return parts.length === 0
    ? t("import.recap.none")
    : t("import.recap.lead", { list: parts.join(t("detect.join")) });
}

/** The stages step 4's primary will go through, in their order: the
 * sessions while none is created, the plan when one is to create, the
 * incidents, the tidying, the images, when there are some. */
export function recapStages(state: ImportState, plan: RecapPlan): RecapWork[] {
  const local = state.local?.outcome;
  return RECAP_STAGES.filter((stage) => {
    switch (stage) {
      case "sessions":
        return true;
      case "plan":
        return (local?.multiNight.length ?? 0) > 0;
      case "incidents":
        return plan.incidents.length > 0;
      case "tidy":
        return plan.tidy !== null;
      case "send":
        return plan.images > 0;
      case "logs":
        return plan.logs.length > 0;
    }
  });
}

/** Whether a stage of step 4's primary went through already (a second try
 * skips it). */
export function stageDone(state: ImportState, stage: RecapWork): boolean {
  switch (stage) {
    case "sessions":
      return state.created !== null;
    case "plan":
      return state.report.plans !== null;
    case "incidents":
      return false;
    case "tidy":
      return state.report.tidy !== null;
    case "send":
      return state.sent !== null;
    case "logs":
      return false;
  }
}

/** The progress line of step 4's primary: each stage gone through, said
 * done, then the one running, « … »: « Sessions créées · Plan multi-nuits
 * créé · Ajout des incidents… ». After a failure, the stages done stay
 * said. */
export function recapProgress(
  state: ImportState,
  plan: RecapPlan,
  t: Translate,
): string | null {
  const stages = recapStages(state, plan);
  const done = stages.filter(
    (stage) => stage !== state.working && stageDone(state, stage),
  );
  const parts = done.map((stage) => t(`import.recap.done.${stage}`));
  if (state.working) parts.push(t(`import.recap.working.${state.working}`));
  return parts.length > 0 ? parts.join(t("detect.join")) : null;
}

/* ------------------------------- Pre-ticking ------------------------------ */

/** The targets « Traitements » opens with ticked: every one that has poses
 * (the person unticks the ones not to process). */
export function preChosenTargets(
  cards: readonly Pick<TreatmentTargetCard, "targetId" | "frames">[],
): string[] {
  return cards.filter((card) => card.frames > 0).map((card) => card.targetId);
}

/** The images « Récapitulatif » opens with ticked: every image made, in the
 * order of the list, unless the space cannot take it -- too heavy, a variant in
 * a space that keeps none, or past the place left (the guard `canTickImage`).
 * The TIFF of a render is ticked only with « Joindre aussi les TIFF »
 * (`joinTiff`): it serves the download in full resolution, never the gallery. */
export function preTickedImages(
  rows: readonly SummaryImage[],
  room: { remaining: number } | null,
  joinTiff = false,
): string[] {
  const ticked: string[] = [];
  for (const row of rows) {
    if (row.fullResolution && !joinTiff) continue;
    if (canTickImage(row, rows, ticked, room)) ticked.push(row.key);
  }
  return ticked;
}

/** `ticked` and every TIFF row the space can take beside it. */
export function withTiffTicked(
  rows: readonly SummaryImage[],
  ticked: readonly string[],
  room: { remaining: number } | null,
): string[] {
  const next = [...ticked];
  for (const row of rows) {
    if (
      row.fullResolution &&
      !next.includes(row.key) &&
      canTickImage(row, rows, next, room)
    )
      next.push(row.key);
  }
  return next;
}

/* ------------------------- The menu and « Sessions » ---------------------- */

export interface ImportPresence {
  /** The menu shows « Import en cours » and « Sessions » its card. */
  shown: boolean;
  /** Nothing runs: the import waits for the person. */
  waiting: boolean;
  /** « Terminé »: shown until the person leaves it, with no badge. */
  done: boolean;
  /** How far the job is, 0–100, when it can say it. */
  percent: number | null;
  /** The target being treated, when the treatments run. */
  target: string | null;
  step: ImportStep;
  /** The job running, for the card's line. */
  job: "scan" | "create" | "treat" | "send" | null;
  /** Something was written to the space (step 4's primary created the
   * sessions); before, the card says nothing is. */
  written: boolean;
  /** « Abandonner l'import » asks first: the treatments may have made
   * provisional folders, which it removes. */
  confirmAbandon: boolean;
}

/**
 * What the menu and « Sessions » say of an import: nothing before the first
 * reading starts (choosing the folders is still « Sessions »), then « Import
 * en cours » with a percentage while a job runs, « à vous » while it waits
 * for the person, until « Terminé » is left (`close`).
 */
export function importPresence(state: ImportState): ImportPresence {
  const job = state.scan
    ? "scan"
    : state.running
      ? "treat"
      : state.sending
        ? state.working === "sessions" || state.working === "plan"
          ? "create"
          : "send"
        : null;
  // No percentage while the files are searched (nothing to measure yet):
  // never a « 0 % » that looks stuck.
  const percent =
    job === "scan" && state.scan
      ? state.scan.total > 0
        ? scanPercent(state.scan.read, state.scan.total)
        : null
      : job === "treat" && state.frame?.overall != null
        ? Math.floor(state.frame.overall)
        : null;
  return {
    shown: state.alive && (state.scan !== null || state.proposal !== null),
    waiting: job === null && state.sent === null,
    done: state.sent !== null,
    percent,
    target: state.running?.target ?? null,
    step: state.step,
    job,
    written: state.created !== null,
    confirmAbandon: state.local !== null,
  };
}

type Translate = (
  key: string,
  params?: Record<string, string | number>,
) => string;

/** The line of the card of « Sessions »: « NGC 7000 · Étape 3 ·
 * Traitements · 12 % » while it treats, « Étape 1 · Dossier · lecture des
 * fichiers · 40 % » while it reads, « Étape 4 · Récapitulatif » while it
 * waits for the person. */
export function importCardLine(presence: ImportPresence, t: Translate): string {
  const job =
    presence.job === "scan"
      ? t("import.job.scan")
      : presence.job === "create"
        ? t("import.job.create")
        : presence.job === "send"
          ? t("import.job.send")
          : null;
  return [
    presence.job === "treat" ? presence.target : null,
    t("import.card.step", {
      n: presence.step,
      label: t(IMPORT_STEP_LABEL_KEY[presence.step]),
    }),
    job,
    presence.percent !== null
      ? t("wizard.sending.percent", { percent: presence.percent })
      : null,
  ]
    .filter((part): part is string => !!part)
    .join(t("detect.join"));
}

/** Why « Terminé » says « Aucune image envoyée » (one reason, by priority):
 * the offer does not include the stacking; the person ticked no image; the
 * treatments were passed; they failed. `null`: nothing explains it more (the
 * treatments had nothing to produce) and the line stays plain. */
export type NoneSentReason = "offer" | "unticked" | "skipped" | "failed";

export function noneSentReason(state: ImportState): NoneSentReason | null {
  if (state.treatRefused > 0) return "offer";
  if (!state.treated) return "skipped";
  if (state.made.length > 0 && state.ticked.length === 0) return "unticked";
  if (state.treatErrors.length > 0) return "failed";
  return null;
}
