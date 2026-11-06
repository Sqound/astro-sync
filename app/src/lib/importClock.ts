// The clock correction « Détecter les sessions » applied to a session, kept
// on this computer so that the wizard of that session reads its files on the
// clock they were written with. The correction is applied to the session,
// never to the files: the files still carry the hours the camera wrote, so a
// night window drawn from the session's evening would leave their poses out.
// Pure, but for the storage handed in; tested under `node --test`.

import type { BulkCreateOutcome, DetectionInput, Proposal } from "./bridge";
import type { ReportStorage } from "./lastReport";

export interface ImportClock {
  /** The evening the files were written on (their own clock). */
  writtenEvening: string;
  /** Hours the session's clock is ahead of the files' (−9: the files say
   * nine hours later than the session). */
  shiftHours: number;
}

const key = (sessionId: string) => `astro-sync.importClock:${sessionId}`;

/** The corrections of the sessions the import made or added poses to: only
 * the ones where a shift was applied. */
export function clocksOf(
  proposal: Proposal,
  outcome: BulkCreateOutcome,
): Record<string, ImportClock> {
  const found = new Map(proposal.sessions.map((s) => [s.id, s]));
  const clocks: Record<string, ImportClock> = {};
  for (const made of outcome.sessions) {
    const session = found.get(made.detectedId);
    if (!session || made.shiftHours === 0) continue;
    clocks[made.sessionId] = {
      writtenEvening: session.writtenEveningDate || made.eveningDate,
      shiftHours: made.shiftHours,
    };
  }
  return clocks;
}

export function saveImportClocks(
  storage: ReportStorage | null,
  clocks: Record<string, ImportClock>,
): void {
  for (const [sessionId, clock] of Object.entries(clocks)) {
    try {
      storage?.setItem(key(sessionId), JSON.stringify(clock));
    } catch {
      // A storage that refuses: the wizard reads the session's own clock.
    }
  }
}

export function loadImportClock(
  storage: ReportStorage | null,
  sessionId: string,
): ImportClock | null {
  try {
    const text = storage?.getItem(key(sessionId));
    if (!text) return null;
    const value = JSON.parse(text) as Partial<ImportClock>;
    if (
      typeof value.writtenEvening !== "string" ||
      typeof value.shiftHours !== "number" ||
      value.shiftHours === 0
    )
      return null;
    return value as ImportClock;
  } catch {
    return null;
  }
}

/** The detection of a night, on the clock of the files: the evening of the
 * window is the one they were written on. Left as it is when no correction
 * is kept for the session. */
export function withImportClock(
  input: DetectionInput,
  clock: ImportClock | null,
): DetectionInput {
  if (!clock) return input;
  return { ...input, eveningDate: clock.writtenEvening };
}
