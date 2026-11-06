// What the « Progression » and « Journaux » tabs share: the night
// selector, read from the night folders of the working folder (no network:
// a night is chosen even offline), and the reserved state of a tab whose
// capability the space does not grant. Pure TypeScript, tested under
// `node --test`.

import type {
  CapabilitiesState,
  Capabilities,
  NightFolderEntry,
} from "./bridge";
import { nightDay, nightSentence } from "./nights.ts";

/** One choice of the « Nuit » selector. */
export interface NightChoice {
  nightId: string;
  folder: string;
  /** `AAAA-MM-JJ`, the evening of the night (the folder's first part). */
  eveningDate: string | null;
  /** The plan or the place (the folder's middle part). */
  place: string | null;
  /** « Lun. 5 oct. · Col des Étoiles ». */
  label: string;
}

const NAME = /^(\d{4}-\d{2}-\d{2}) · (.+) · [^·]+$/u;

/** The evening of a night read as noon UTC, so its weekday and day never
 * move with the computer's time zone. */
function evening(date: string): string {
  return `${date}T12:00:00Z`;
}

/** The night folders as choices, the most recent evening first. A folder
 * whose name was changed by hand keeps that name as its label (its identity
 * is in its hidden file). */
export function nightChoices(
  folders: readonly NightFolderEntry[],
  locale: string,
): NightChoice[] {
  return folders
    .map((entry) => {
      const match = NAME.exec(entry.name.normalize("NFC"));
      if (!match) {
        return {
          nightId: entry.nightId,
          folder: entry.folder,
          eveningDate: null,
          place: null,
          label: entry.name,
        };
      }
      const [, date, place] = match;
      return {
        nightId: entry.nightId,
        folder: entry.folder,
        eveningDate: date,
        place,
        label: `${nightDay(evening(date), locale, "UTC")} · ${place}`,
      };
    })
    .sort(
      (a, b) =>
        (b.eveningDate ?? "").localeCompare(a.eveningDate ?? "") ||
        a.label.localeCompare(b.label),
    );
}

/** « 5 oct. », the day inside a sentence (« Nuit du 5 oct. »): the same
 * evening as the cell « Lun. 5 oct. », without the weekday. */
export function dayInSentence(eveningDate: string, locale: string): string {
  return nightSentence(eveningDate, locale);
}

type TabCapability = "analysis.logs" | "analysis.progression";

/** Whether a tab is reserved: the space answered and does not grant its
 * capability. Never an offer name (boundary rule): a private server grants
 * everything, so the state never shows there; an unknown answer (not yet
 * read, or not signed in) holds nothing back. */
export function isReserved(
  state: CapabilitiesState | null,
  capability: TabCapability,
): boolean {
  const capabilities: Capabilities | null | undefined = state?.capabilities;
  return capabilities ? capabilities[capability] === false : false;
}
