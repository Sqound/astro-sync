// What a wait says: the sentence
// under the star names what is awaited, per step of the wizard. Pure
// TypeScript, tested under `node --test`; `components/Waiting` draws it.

import type { Phase } from "./wizardSteps.ts";

export interface WizardWait {
  /** The step the wizard draws, `null` while it is not known yet. */
  phase: Phase | null;
  /** The night's page (what is online, what its folder holds) is read. */
  viewReady: boolean;
  /** « Dossier » is making the night's folder. */
  creatingFolder: boolean;
  /** The installed tools are listed (read when « Traitements » opens). */
  toolsReady: boolean;
}

/** The translation key of the sentence a step shows while it waits. */
export function waitingKey(wait: WizardWait): string {
  switch (wait.phase) {
    case null:
    case "cibles":
      return "wizard.wait.night";
    case "dossier":
      if (!wait.viewReady) return "wizard.wait.night";
      return wait.creatingFolder
        ? "wizard.folder.creating"
        : "wizard.wait.folder";
    case "fichiers":
      return "wizard.wait.folder";
    case "traitements":
      return wait.toolsReady ? "wizard.wait.night" : "common.wait.tools";
    case "recapitulatif":
    case "sending":
    case "sent":
      return "wizard.wait.summary";
  }
}
