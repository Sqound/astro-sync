// « Nuits », the reading of the rows: the reads of the nights start together
// instead of one after the other, under two limits -- the calls to the
// service, and the reads of a night's folder on disk, which compete for the
// same drive. Pure TypeScript (no React, no Tauri): the order in which the
// answers come back is tested under `node --test`;
// `screens/SessionsScreen.tsx` draws the rows.

import { createLimiter } from "./concurrency.ts";
import type {
  NightDetail,
  NightOnline,
  NightView,
  SessionSummary,
} from "./bridge";

/** Calls to the service (a night's detail, what is online) at the same time. */
export const SERVICE_READS = 4;
/** Reads of a night's folder on disk at the same time. */
export const FOLDER_READS = 2;

/** What the screen asks of the app and the service, one night at a time. */
export interface NightReaders {
  detail(sessionId: string): Promise<NightDetail>;
  online(sessionId: string): Promise<NightOnline>;
  /** The folder read, from what the service said is online. */
  view(session: SessionSummary, online: NightOnline): Promise<NightView>;
}

/** A night read: its row can be filled. */
export interface NightRead {
  index: number;
  session: SessionSummary;
  detail: NightDetail | null;
  view: NightView | null;
}

/**
 * Reads every night at once under the two limits. The most recent ones
 * (`sessions` comes newest first) are served first; only the first
 * `readInFull` have their folder read. `onRead` is called as each night
 * comes back, in the order the answers arrive; the rows keep their place in
 * the list, the caller puts each one at its `index`. Resolves with every
 * night read, in the order of `sessions`, or with `null` as soon as
 * `isCurrent()` says the reading was replaced (no call is then started and
 * `onRead` is not called any more).
 */
export async function readNights(input: {
  sessions: readonly SessionSummary[];
  readInFull: number;
  readers: NightReaders;
  isCurrent: () => boolean;
  onRead: (read: NightRead) => void;
}): Promise<NightRead[] | null> {
  const { sessions, readInFull, readers, isCurrent, onRead } = input;
  const service = createLimiter(SERVICE_READS);
  const folder = createLimiter(FOLDER_READS);
  const stale = Symbol("stale");

  /** A read that starts only if the reading is still the current one. */
  const guarded =
    <T>(task: () => Promise<T>): (() => Promise<T | typeof stale>) =>
    () =>
      isCurrent() ? task() : Promise.resolve(stale);

  const reads = await Promise.all(
    sessions.map(async (session, index) => {
      const detailRead = service.run(
        guarded(() => readers.detail(session.id).catch(() => null)),
        index,
      );
      const viewRead =
        index < readInFull
          ? service
              .run(
                guarded(() => readers.online(session.id)),
                index,
              )
              .then(async (online) => {
                if (online === stale) return stale;
                return folder.run(
                  guarded(() => readers.view(session, online)),
                  index,
                );
              })
              .catch(() => null)
          : Promise.resolve(null);
      const [detail, view] = await Promise.all([detailRead, viewRead]);
      if (detail === stale || view === stale || !isCurrent()) return null;
      const read: NightRead = { index, session, detail, view };
      onRead(read);
      return read;
    }),
  );
  if (!isCurrent() || reads.some((read) => read === null)) return null;
  return reads as NightRead[];
}
