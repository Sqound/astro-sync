// The state of « Accueil » -- pure TypeScript (no React, no Tauri), so the rule
// runs under `node --test` and the screen only draws what it says.
//
// The home explains; the work happens in « Nuits »:
//   - the connection card (today's « Espace Astro Portal », or the sign-in
//     card when signed out);
//   - « À reprendre »: a clickable row per night « à compléter », which
//     opens its wizard at the step where something is missing;
//   - « Comment ça marche » in four steps of one sentence, folded after a
//     first sending;
//   - one primary, « Commencer », which opens « Nuits » -- greyed, with its
//     reason in one line, while this computer is not signed in.

import type { WizardStep } from "./bridge";

/** A night to resume, already in words. */
export interface ResumeRow {
  sessionId: string;
  /** « Nuit du lun. 5 oct. · Col des Étoiles ». */
  title: string;
  /** « À compléter : journaux ajoutés au dossier depuis l'envoi ». */
  line: string;
  step: WizardStep | null;
}

export interface HomeInput {
  connected: boolean;
  /** The nights « à compléter » of this computer, newest first. */
  toResume: readonly ResumeRow[];
  /** A night has already been sent from this computer. */
  sentOnce: boolean;
}

export interface HomeState {
  /** « Commencer » opens « Nuits »; greyed while signed out. */
  startEnabled: boolean;
  /** The one line that says why it is greyed. */
  startReasonKey: string | null;
  /** « Comment ça marche » folded (one row that unfolds). */
  howFolded: boolean;
  resume: readonly ResumeRow[];
  /** Zones of the screen, each with its own primary: the sign-in
   * card when signed out, « Comment ça marche » always. */
  primaries: { signIn: number; how: number };
}

/** How many nights to resume the home shows at most. */
export const RESUME_LIMIT = 3;

export function homeState(input: HomeInput): HomeState {
  return {
    startEnabled: input.connected,
    startReasonKey: input.connected ? null : "home.start.signInFirst",
    howFolded: input.connected && input.sentOnce,
    resume: input.connected ? input.toResume.slice(0, RESUME_LIMIT) : [],
    primaries: {
      signIn: input.connected ? 0 : 1,
      // « Commencer » greyed is still the zone's primary, inactive.
      how: 1,
    },
  };
}

/** The four steps of « Comment ça marche », in order. */
export const HOW_STEPS = [
  { titleKey: "home.how.step1.title", lineKey: "home.how.step1.line" },
  { titleKey: "home.how.step2.title", lineKey: "home.how.step2.line" },
  { titleKey: "home.how.step3.title", lineKey: "home.how.step3.line" },
  { titleKey: "home.how.step4.title", lineKey: "home.how.step4.line" },
] as const;

/** A path shown to the person: their home folder written « ~ » (macOS and
 * Linux), as the screens write it. */
export function displayPath(path: string, home: string | null): string {
  if (!home) return path;
  const trimmed = home.replace(/[\\/]+$/, "");
  if (trimmed && (path === trimmed || path.startsWith(`${trimmed}/`))) {
    return `~${path.slice(trimmed.length)}`;
  }
  return path;
}

/** The home folder of a working folder under `…/Documents/…` (macOS,
 * Linux): what `displayPath` writes « ~ ». */
export function homeOf(root: string): string | null {
  const match = /^(\/(?:Users|home)\/[^/]+)\//.exec(root);
  return match ? match[1] : null;
}
