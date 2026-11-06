// The snackbars of the window: one small card at the bottom centre for what
// passes -- the service starting, a copy, a saved setting, a folder that
// could not be made, an upload on its way --, never a banner that pushes the
// screen down. Several messages never overlap: they wait in a queue and one
// shows at a time, what is running (a snackbar with a progress bar) first,
// the others after it in the order they came. What stays (a blocking error of
// a screen) is said in the screen, not here. Pure TypeScript, so the queue,
// the modes and the durations are tested under `node --test`.

/** The four tones, said by a thin edge and a small mark, never a solid
 * block. */
export type SnackTone = "success" | "info" | "warning" | "error";

/** Persistent: stays until its cross (« Fermer ») or its owner closes it.
 * Timed: closes by itself after its duration. */
export type SnackMode = "persistent" | "timed";

/** How long a timed snackbar stays, in milliseconds. */
export const SNACKBAR_DURATION_MS = 5000;

/** How long a timed snackbar that offers a gesture (« Voir le prompt »)
 * stays: time to read it and reach its link. */
export const SNACKBAR_ACTION_DURATION_MS = 10000;

/** At most this many wait in the queue; the oldest that is not running goes
 * when another comes. */
export const SNACKBAR_MAX = 8;

export interface SnackAction {
  label: string;
  run: () => void;
}

export interface SnackInput {
  tone: SnackTone;
  text: string;
  /** Default: an error is persistent, the rest timed. */
  mode?: SnackMode;
  action?: SnackAction;
  /** One snackbar per key: a new one with the same key takes its place
   * (the seconds of the service's wait, a second copy). */
  key?: string;
  /** A second, muted line under the text (« 3,1 Mo sur 25 Mo · 1,2 Mo/s »). */
  detail?: string;
  /** A progress bar, 0 to 100 (whole numbers): the snackbar of something
   * that is running. It shows before the others of the queue. */
  progress?: number;
  /** The person closed it with its cross, or its time ran out: the owner
   * learns it (closing never cancels what the snackbar talks about). Not
   * called when the owner itself closes it or replaces it. */
  onClose?: () => void;
}

export interface Snack {
  id: number;
  tone: SnackTone;
  text: string;
  mode: SnackMode;
  action?: SnackAction;
  key?: string;
  detail?: string;
  progress?: number;
  onClose?: () => void;
}

export interface SnackState {
  next: number;
  snacks: readonly Snack[];
}

export const NO_SNACKS: SnackState = { next: 1, snacks: [] };

/** The mode a snackbar takes when its owner says none. */
export function modeOf(input: Pick<SnackInput, "tone" | "mode">): SnackMode {
  return input.mode ?? (input.tone === "error" ? "persistent" : "timed");
}

/** How long a snackbar stays; `null` when it waits for its cross. */
export function durationOf(
  snack: Pick<Snack, "mode" | "action">,
): number | null {
  if (snack.mode === "persistent") return null;
  return snack.action ? SNACKBAR_ACTION_DURATION_MS : SNACKBAR_DURATION_MS;
}

/** The percent kept to whole numbers between 0 and 100. */
export function clampPercent(percent: number): number {
  if (!Number.isFinite(percent)) return 0;
  return Math.max(0, Math.min(100, Math.floor(percent)));
}

function sameSnack(a: Snack, b: Snack): boolean {
  return (
    a.tone === b.tone &&
    a.text === b.text &&
    a.mode === b.mode &&
    a.detail === b.detail &&
    a.progress === b.progress &&
    a.action?.label === b.action?.label
  );
}

/** A snackbar comes: it takes the place of the one with its key (keeping
 * its id and its turn, so it does not blink), or joins the queue; the
 * oldest that is not running goes past `SNACKBAR_MAX`. The same message
 * again with the same key changes nothing (the state is returned as it is),
 * so an owner may say it on every change of its own state without restarting
 * the time of the one on screen. */
export function pushSnack(
  state: SnackState,
  input: SnackInput,
): { state: SnackState; id: number } {
  const same = input.key
    ? state.snacks.find((snack) => snack.key === input.key)
    : undefined;
  const id = same ? same.id : state.next;
  const snack: Snack = {
    id,
    tone: input.tone,
    text: input.text,
    mode: modeOf(input),
    action: input.action,
    key: input.key,
    detail: input.detail,
    progress:
      input.progress === undefined ? undefined : clampPercent(input.progress),
    onClose: input.onClose,
  };
  if (same) {
    if (sameSnack(same, snack)) {
      // The gesture may be a new closure: keep the latest, change nothing else.
      same.action = snack.action;
      same.onClose = snack.onClose;
      return { id, state };
    }
    return {
      id,
      state: {
        next: state.next,
        snacks: state.snacks.map((item) => (item.id === id ? snack : item)),
      },
    };
  }
  const snacks = [...state.snacks, snack];
  // Past the maximum the oldest goes, never one that is running.
  while (snacks.length > SNACKBAR_MAX) {
    const index = snacks.findIndex(
      (item) => item.progress === undefined && item.id !== id,
    );
    snacks.splice(index < 0 ? 0 : index, 1);
  }
  return { id, state: { next: state.next + 1, snacks } };
}

/** The one snackbar on screen: what is running (a progress bar) first, then
 * the others in the order they came. `null` when the queue is empty. */
export function visibleSnack(snacks: readonly Snack[]): Snack | null {
  return (
    snacks.find((snack) => snack.progress !== undefined) ?? snacks[0] ?? null
  );
}

/** A snackbar goes (its cross, its time, its owner). */
export function dismissSnack(state: SnackState, id: number): SnackState {
  return {
    next: state.next,
    snacks: state.snacks.filter((snack) => snack.id !== id),
  };
}

/** The snackbar of a key goes (the service answered). */
export function dismissSnackKey(state: SnackState, key: string): SnackState {
  return {
    next: state.next,
    snacks: state.snacks.filter((snack) => snack.key !== key),
  };
}

/** How a screen reader hears it: an error interrupts, the rest waits. */
export function roleOf(tone: SnackTone): "alert" | "status" {
  return tone === "error" ? "alert" : "status";
}
