// « Mesurer les poses » must bring the person to the card of the target in
// « Fichiers »: scrolled to the top of its area, below what stays on top of
// it, and focused. Pure: the window gives the elements and the events, this
// file decides the order of things and when to stop.

/** The id of the card of a target in « Fichiers ». */
export function targetCardId(targetId: string): string {
  return `as-file-card-${targetId.replace(/[^A-Za-z0-9_-]/g, "_")}`;
}

/** Space left above the card, past whatever covers the top of the scrolling
 * area, in pixels. */
export const REVEAL_GAP = 8;

/** How long the card is kept in place while what is above it settles (the
 * measure cards of the targets above arrive one by one, the files are read
 * again), in milliseconds. */
export const HOLD_MILLISECONDS = 4000;

/** What the card needs of an element of the page. */
export interface RevealElement {
  style: { scrollMarginTop: string };
  scrollIntoView(options: {
    block: "start";
    behavior: "auto" | "smooth";
  }): void;
  focus(options?: { preventScroll?: boolean }): void;
}

/** Scrolls the card to the top of the scrolling area, below what covers it
 * (`covered`, pixels: the sticky head over the window), then focuses it
 * without scrolling again: the focus follows the scroll, never the other way
 * round. */
export function revealCard(el: RevealElement, covered: number): void {
  el.style.scrollMarginTop = `${Math.ceil(covered) + REVEAL_GAP}px`;
  el.scrollIntoView({ block: "start", behavior: "auto" });
  el.focus({ preventScroll: true });
}

/** What {@link holdReveal} reads of the window. */
export interface HoldEnv {
  /** Puts the card back in view (and focuses it). */
  reveal: () => void;
  /** Calls back when what is above the card may have changed size; returns
   * how to stop watching. */
  watch: (callback: () => void) => () => void;
  /** Calls back when the person scrolls, presses or types; returns how to
   * stop listening. */
  interrupted: (callback: () => void) => () => void;
  /** Calls back after `ms`; returns how to cancel. */
  after: (ms: number, callback: () => void) => () => void;
}

/** Reveals the card now, and again each time the content above it changes
 * size, until the person takes the scroll back or {@link HOLD_MILLISECONDS}
 * pass. Returns the way to stop at once (the step is left). */
export function holdReveal(
  env: HoldEnv,
  milliseconds: number = HOLD_MILLISECONDS,
): () => void {
  let held = true;
  const stops: (() => void)[] = [];
  const stop = () => {
    if (!held) return;
    held = false;
    for (const end of stops) end();
  };
  env.reveal();
  stops.push(env.watch(() => held && env.reveal()));
  stops.push(env.interrupted(stop));
  stops.push(env.after(milliseconds, stop));
  return stop;
}
