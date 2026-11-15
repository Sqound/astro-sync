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

/** A path cut after each separator, each part kept whole: a long path drawn
 * part by part (each part unbroken) goes to the line after a « / », never
 * inside a folder's name (« Astro Sync »). The parts joined give the path
 * back, so a copy of it is the path. */
export function pathParts(path: string): string[] {
  return path.match(/[^\\/]*(?:[\\/]+|$)/g)?.filter(Boolean) ?? [];
}

/** The home folder of a working folder under `…/Documents/…` (macOS,
 * Linux): what `displayPath` writes « ~ ». */
export function homeOf(root: string): string | null {
  const match = /^(\/(?:Users|home)\/[^/]+)\//.exec(root);
  return match ? match[1] : null;
}

/** What the connection card of « Accueil » says. The token kept on this
 * computer is not enough to say « Connecté »: a space deleted while the
 * service slept still has its token here. So the card says « Vérification
 * de la connexion… » until the service has answered once since the launch,
 * « Connecté » when it did, and that it could not be asked when it did not
 * answer. Signed out: the sign-in card. */
export type ConnectionCard =
  "checking" | "connected" | "unchecked" | "signedOut";

export function connectionCard(input: {
  paired: boolean;
  /** The check of this launch: `null` while it runs (or before it). */
  answered: boolean | null;
}): ConnectionCard {
  if (!input.paired) return "signedOut";
  if (input.answered === null) return "checking";
  return input.answered ? "connected" : "unchecked";
}

/** The service answered once since the launch: the home, mounted again (a
 * click on « Accueil »), says « Connecté » at once instead of asking again.
 * Kept by the window for its life, never on disk. */
let answeredThisLaunch = false;

export function serviceAnsweredThisLaunch(): boolean {
  return answeredThisLaunch;
}

export function rememberServiceAnswered(answered: boolean): void {
  if (answered) answeredThisLaunch = true;
}
