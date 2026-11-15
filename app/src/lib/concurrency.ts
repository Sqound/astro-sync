/**
 * A small limiter for reads that can start together: at most `limit` of
 * them run at once, the others wait their turn. No dependency: the screens
 * need nothing more than this.
 */

export interface Limiter {
  /**
   * Runs `task` when a place is free. Of the tasks waiting, the lowest
   * `priority` goes first (a night's index: the most recent first), then the
   * oldest request. Resolves or rejects as `task` does.
   */
  run<T>(task: () => Promise<T>, priority?: number): Promise<T>;
}

interface Waiting {
  priority: number;
  start: () => void;
}

export function createLimiter(limit: number): Limiter {
  const places = Math.max(1, Math.floor(limit));
  let running = 0;
  const waiting: Waiting[] = [];

  function next(): void {
    while (running < places && waiting.length > 0) {
      let best = 0;
      for (let index = 1; index < waiting.length; index += 1) {
        if (waiting[index].priority < waiting[best].priority) best = index;
      }
      const [entry] = waiting.splice(best, 1);
      running += 1;
      entry.start();
    }
  }

  return {
    run<T>(task: () => Promise<T>, priority = 0): Promise<T> {
      return new Promise<T>((resolve, reject) => {
        waiting.push({
          priority,
          start: () => {
            let started: Promise<T>;
            try {
              started = Promise.resolve(task());
            } catch (caught) {
              started = Promise.reject(caught);
            }
            started.then(resolve, reject).finally(() => {
              running -= 1;
              next();
            });
          },
        });
        next();
      });
    },
  };
}

/**
 * `fn` over `items`, at most `limit` at a time; the results come back in the
 * order of `items`, whatever the order the calls finished in. The first
 * failure rejects (the others are left to finish).
 */
export function mapWithLimit<T, R>(
  items: readonly T[],
  limit: number,
  fn: (item: T, index: number) => Promise<R>,
): Promise<R[]> {
  const limiter = createLimiter(limit);
  return Promise.all(
    items.map((item, index) => limiter.run(() => fn(item, index), index)),
  );
}

/**
 * The latest of a series of reads wins: each read takes a ticket when it
 * starts, and only the ticket of the last one started may write its answer.
 * An older read that comes back later (« Vérifier à nouveau » pressed while
 * the opening read still runs) is dropped instead of overwriting a newer
 * answer with what was on disk before.
 */
export interface LatestOnly {
  /** A read starts: its ticket. */
  begin(): number;
  /** Whether the read of `ticket` is still the last one started. */
  isLatest(ticket: number): boolean;
}

export function latestOnly(): LatestOnly {
  let last = 0;
  return {
    begin: () => {
      last += 1;
      return last;
    },
    isLatest: (ticket) => ticket === last,
  };
}
