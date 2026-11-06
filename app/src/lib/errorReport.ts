// What the window hands to the core when it meets an error nobody caught
// (`window.onerror`, a promise nobody awaited). The window sends the text
// and the stack as they are and decides nothing: the core cleans them (no
// path, no address, nothing between quotes), cuts them, and queues them
// only where the reporting is allowed -- the person did not turn it off, the
// service in force is the hosted one, and the setting is on the screen
// (`astro_sync_core::error_report`). Pure TypeScript for the decisions
// (tested under `node --test`); `installErrorReporting` is the thin
// installer `main.tsx` calls.

/** A message and, when the engine gave one, its stack. */
export interface DescribedError {
  message: string;
  stack: string | null;
}

/** The longest text handed over: the core cuts it to its own limits. */
const HAND_OVER_LIMIT = 4000;

const cut = (text: string) => text.slice(0, HAND_OVER_LIMIT);

/** The message and the stack of whatever was thrown (an `Error`, a string,
 * anything). */
export function describeError(thrown: unknown): DescribedError {
  if (thrown instanceof Error) {
    return {
      message: cut(thrown.message || thrown.name || "Error"),
      stack: thrown.stack ? cut(thrown.stack) : null,
    };
  }
  if (typeof thrown === "string") {
    return { message: cut(thrown) || "Error", stack: null };
  }
  return { message: "Uncaught value", stack: null };
}

/** What the window's error and rejection events carry. */
export interface ErrorEventLike {
  error?: unknown;
  message?: unknown;
  reason?: unknown;
}

/** What the installer needs of `window`. */
export interface ErrorTarget {
  addEventListener(
    type: "error" | "unhandledrejection",
    listener: (event: ErrorEventLike) => void,
  ): void;
}

/** Hands one error to the core. */
export type RecordError = (
  message: string,
  stack: string | null,
) => Promise<unknown>;

/** The same error is handed over once in this many milliseconds: an error
 * thrown in a loop is one report, not a thousand calls. */
export const SAME_ERROR_GAP_MS = 10_000;

/** Hands every uncaught error of `target` to `record`, once per gap for the
 * same one. A refusal or a failure of `record` is swallowed: reporting an
 * error must never be an error. */
export function installErrorReporting(
  target: ErrorTarget,
  record: RecordError,
  now: () => number = Date.now,
): void {
  const lastSeen = new Map<string, number>();
  const hand = (thrown: unknown) => {
    const described = describeError(thrown);
    const key = `${described.message}\n${described.stack?.split("\n")[0] ?? ""}`;
    const at = now();
    const before = lastSeen.get(key);
    if (before !== undefined && at - before < SAME_ERROR_GAP_MS) return;
    lastSeen.set(key, at);
    if (lastSeen.size > 50) {
      lastSeen.delete(lastSeen.keys().next().value as string);
    }
    try {
      void record(described.message, described.stack).catch(() => undefined);
    } catch {
      // Nothing more to do.
    }
  };
  target.addEventListener("error", (event) =>
    hand(event.error ?? event.message),
  );
  target.addEventListener("unhandledrejection", (event) => hand(event.reason));
}
