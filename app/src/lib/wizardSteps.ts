// The wizard of a night: its thread of five steps, « Cibles » and « Dossier ».
// Pure TypeScript, tested under `node --test`; `screens/night/` draws it.

import type { TranslateParams } from "../i18n/locale.ts";
import type {
  NightDetail,
  NightFolderOutcome,
  NightLocation,
  NightOnline,
  WizardStep,
} from "./bridge";
import { formatMinutes, pluralText } from "./written.ts";

type Translate = (key: string, params?: TranslateParams) => string;

/** The five steps, in order. */
export const STEPS: readonly WizardStep[] = [
  "cibles",
  "dossier",
  "fichiers",
  "traitements",
  "recapitulatif",
];

export const STEP_LABEL_KEY: Record<WizardStep, string> = {
  cibles: "wizard.step.targets",
  dossier: "wizard.step.folder",
  fichiers: "wizard.step.files",
  traitements: "wizard.step.treatments",
  recapitulatif: "wizard.step.summary",
};

/** Where the wizard is: one of its steps, the sending, or « Nuit envoyée ». */
export type Phase = WizardStep | "sending" | "sent";

export type ThreadStatus = "done" | "current" | "waiting";

export interface ThreadStep {
  /** The step's id: a step of the night (`WizardStep`) or of the import. */
  step: string;
  n: number;
  labelKey: string;
  status: ThreadStatus;
}

/** The thread at the head of the wizard: the steps before the current one
 * ticked, the current one numbered in gold, the next ones waiting; the
 * sending keeps « Récapitulatif » current, « Nuit envoyée » ticks them all;
 * while the step is not known (the night is being read) none is current. */
export function thread(phase: Phase | null): ThreadStep[] {
  const current =
    phase === null
      ? -1 // not known yet: no step is current, all are waiting
      : phase === "sent"
        ? STEPS.length
        : phase === "sending"
          ? STEPS.length - 1
          : STEPS.indexOf(phase);
  return STEPS.map((step, index) => ({
    step,
    n: index + 1,
    labelKey: STEP_LABEL_KEY[step],
    status:
      index < current ? "done" : index === current ? "current" : "waiting",
  }));
}

export function nextStep(step: WizardStep): WizardStep | null {
  return STEPS[STEPS.indexOf(step) + 1] ?? null;
}

export function previousStep(step: WizardStep): WizardStep | null {
  return STEPS[STEPS.indexOf(step) - 1] ?? null;
}

/** The step a night's page opens at: the resume's, else « Cibles ». */
export function openingStep(
  resumeStep: WizardStep | null | undefined,
): WizardStep {
  return resumeStep ?? "cibles";
}

/** What the wizard's opening shows while it has no step to draw (never a
 * wait without an end). `loading`: the star; `not-found`: the night is
 * neither in the space's list nor on the service; `failed`: the night's
 * page could not be read (the error, « Réessayer », « ← Nuits »). */
export type OpeningState = "ready" | "loading" | "not-found" | "failed";

export function openingState(read: {
  /** The reads of the night (list, detail, working folder…) are back. */
  contextRead: boolean;
  /** The night is known (in the list, or by its detail). */
  hasSession: boolean;
  /** The step the wizard draws, once known. */
  phase: Phase | null;
  /** The reading of the night's page failed. */
  viewFailed: boolean;
}): OpeningState {
  if (!read.contextRead) return "loading";
  // Even at a step « Nuits » named: without the night, no step can be drawn.
  if (!read.hasSession) return "not-found";
  if (read.phase) return "ready";
  return read.viewFailed ? "failed" : "loading";
}

/** A step that needs the night's page: drawn, waiting for it, or stuck
 * because its reading failed (the error, « Réessayer », « Retour »). */
export type StepReading = "ready" | "loading" | "failed";

export function stepReading(
  view: unknown | null,
  viewFailed: boolean,
): StepReading {
  if (view) return "ready";
  return viewFailed ? "failed" : "loading";
}

/* --------------------------------- Cibles -------------------------------- */

const DEBRIEF_STATE_KEY: Record<string, string> = {
  done: "wizard.targets.state.done",
  modified: "wizard.targets.state.modified",
  partial: "wizard.targets.state.partial",
  skipped: "wizard.targets.state.skipped",
};

/** One target of « Quelles cibles ? »: its designation, its state and
 * integration from the debriefing, its folder's name in grey. */
export interface TargetChoice {
  targetId: string;
  designation: string;
  /** « Faite · Intégration 2 h 20 », or `null` without a debriefing. */
  line: string | null;
  /** « NGC 7000/ ». */
  folderName: string;
  ticked: boolean;
}

/** The name the « Dossier » step gives a target's folder: the
 * designation, its forbidden characters replaced by « - ». The step's own
 * plan (`night_folder_plan`) is what is shown when it is known. */
export function targetFolderName(designation: string): string {
  const cleaned = designation
    // eslint-disable-next-line no-control-regex
    .replace(/[/\\:*?"<>|\u0000-\u001f]/g, "-")
    .replace(/^[\s.]+|[\s.]+$/g, "");
  return cleaned === "" ? "-" : cleaned;
}

export function targetChoices(
  t: Translate,
  online: NightOnline | null,
  detail: NightDetail | null,
  unticked: ReadonlySet<string>,
  plan: NightFolderOutcome | null = null,
): TargetChoice[] {
  const debrief = new Map(
    (detail?.debrief?.targets ?? []).map((target) => [target.targetId, target]),
  );
  return (online?.targets ?? [])
    .filter((target) => target.targetId)
    .map((target) => {
      const targetId = target.targetId as string;
      const designation = target.name ?? targetId;
      const entry = debrief.get(targetId);
      const stateKey = entry?.state ? DEBRIEF_STATE_KEY[entry.state] : null;
      const parts = [
        stateKey ? t(stateKey) : null,
        typeof entry?.acquiredMinutes === "number"
          ? t("wizard.targets.integration", {
              duration: formatMinutes(t, entry.acquiredMinutes),
            })
          : null,
      ].filter((part): part is string => part !== null);
      const planned = plan?.targets.find((item) => item.targetId === targetId);
      return {
        targetId,
        designation,
        line: parts.length > 0 ? parts.join(" · ") : null,
        folderName: `${planned?.folderName ?? targetFolderName(designation)}/`,
        ticked: !unticked.has(targetId),
      };
    });
}

/** « Continuer avec 3 cibles ». */
export function continueWithTargets(
  t: Translate,
  locale: string,
  count: number,
): string {
  return pluralText(t, locale, "wizard.targets.continue", count);
}

/* -------------------------------- Dossier -------------------------------- */

/** What the « Dossier » step does when it opens, from where the night's
 * folder is:
 * - `prepare`: create what is missing, without a question (the core is
 *   idempotent: a folder that is there is kept, a target ticked since gets
 *   its folder) -- a folder found, or none, even when one was known before
 *   (moved or deleted: a new, empty one is made, and the person may still
 *   point at the old one, `folderWasKnown`);
 * - `choose`: two folders carry the night, the person says which to follow
 *   (making a third would solve nothing). */
export type FolderAction = "prepare" | "choose";

export function folderStepAction(location: NightLocation): FolderAction {
  return location.state === "duplicated" ? "choose" : "prepare";
}

/** The night had a folder on this computer that is no longer in the working
 * folder: the new one is made all the same, and the step says so with
 * « Retrouver l'ancien dossier ». */
export function folderWasKnown(
  location: NightLocation,
  folderTrace: boolean,
): boolean {
  return location.state === "missing" && folderTrace;
}

/** « Dossier créé dans votre dossier de travail, avec un dossier pour
 * chacune des 3 cibles cochées. » -- or the folder that already was there. */
export function folderLine(
  t: Translate,
  locale: string,
  outcome: Pick<NightFolderOutcome, "created" | "targets">,
): string {
  const ticked = outcome.targets.filter(
    (target) => target.state !== "out-of-night",
  ).length;
  return pluralText(
    t,
    locale,
    outcome.created ? "wizard.folder.created" : "wizard.folder.existing",
    ticked,
  );
}

/** The name of a path's last folder. */
export function baseName(path: string): string {
  const parts = path.split(/[\\/]+/).filter(Boolean);
  return parts[parts.length - 1] ?? path;
}

/** The folder a path is in. */
export function parentOf(path: string): string {
  const trimmed = path.replace(/[\\/]+$/, "");
  const index = Math.max(trimmed.lastIndexOf("/"), trimmed.lastIndexOf("\\"));
  return index > 0 ? trimmed.slice(0, index) : trimmed;
}

/** The tree « Comment ranger vos fichiers » draws for this night: the
 * working folder, `ASSISTANT-IA.md`, the night, `logs/`, `calibration/` and
 * its four folders, then each ticked target (the first one detailed). */
export interface TreeRow {
  depth: number;
  name: string;
  /** A translation key of the grey note beside it, or a literal. */
  noteKey: string | null;
  note?: string;
  file?: boolean;
}

export function folderTree(
  rootName: string,
  nightFolder: string,
  targetFolders: readonly string[],
): TreeRow[] {
  const rows: TreeRow[] = [
    { depth: 0, name: `${rootName}/`, noteKey: null },
    {
      depth: 1,
      name: "ASSISTANT-IA.md",
      noteKey: "wizard.tree.assistant",
      file: true,
    },
    { depth: 1, name: `${nightFolder}/`, noteKey: null },
    { depth: 2, name: "logs/", noteKey: "wizard.tree.logs" },
    { depth: 2, name: "calibration/", noteKey: null },
    { depth: 3, name: "darks/", noteKey: null },
    { depth: 3, name: "flats/", noteKey: "wizard.tree.flats" },
    { depth: 3, name: "dark-flats/", noteKey: null },
    { depth: 3, name: "biases/", noteKey: null },
  ];
  targetFolders.forEach((folder, index) => {
    const name = folder.endsWith("/") ? folder : `${folder}/`;
    if (index === 0) {
      rows.push({ depth: 2, name, noteKey: "wizard.tree.target" });
      rows.push({ depth: 3, name: "lights/", noteKey: "wizard.tree.lights" });
      rows.push({ depth: 3, name: "final/", noteKey: "wizard.tree.final" });
    } else {
      rows.push({ depth: 2, name, noteKey: null, note: "lights/, final/" });
    }
  });
  return rows;
}

/** The language the copied prompt is written in: the window's. */
export function promptLanguage(locale: string): string {
  return locale;
}
