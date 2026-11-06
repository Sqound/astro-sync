import { holdReveal, revealCard } from "../../lib/wizardAnchor";

/** How often the position of the card is looked at while it is held. */
const WATCH_MILLISECONDS = 50;

/** Brings the card of a target to the top of its area and focuses it, and
 * keeps it there while what is above it settles (the notice of the step, the
 * measure cards of the targets before it arrive after the first draw), until
 * the person takes the scroll back. Returns the way to stop at once. The
 * logic is `lib/wizardAnchor.ts`; this is its reading of the window. */
export function holdTargetCard(card: HTMLElement): () => void {
  const head = () => document.querySelector<HTMLElement>(".as-page__top");
  // How much of the scrolling area the sticky head covers: the head stays
  // on top of the window, and the area the card scrolls in may start under
  // it (the window scrolls) or below it (a frame scrolls by itself).
  const covered = () => {
    const bar = head();
    if (!bar) return 0;
    const area = scrollingParent(card);
    const top = area ? area.getBoundingClientRect().top : 0;
    return Math.max(0, bar.getBoundingClientRect().bottom - top);
  };
  // Where the card and the head were left by the last reveal: when either
  // moves, something above changed size and the card is put back.
  const place = () =>
    `${Math.round(card.getBoundingClientRect().top)}|${Math.round(
      head()?.getBoundingClientRect().bottom ?? 0,
    )}`;
  let settled = "";
  // What the person does with their own hands: not `scroll`, which the
  // reveal itself causes.
  const own = ["wheel", "touchstart", "keydown", "pointerdown"] as const;
  return holdReveal({
    reveal: () => {
      revealCard(card, covered());
      settled = place();
    },
    watch: (callback) => {
      // A timer, not an animation frame: a window in the background draws
      // none, and the card must still be put back.
      const timer = window.setInterval(() => {
        if (place() !== settled) callback();
      }, WATCH_MILLISECONDS);
      return () => window.clearInterval(timer);
    },
    interrupted: (callback) => {
      for (const name of own)
        window.addEventListener(name, callback, { passive: true });
      return () => {
        for (const name of own) window.removeEventListener(name, callback);
      };
    },
    after: (ms, callback) => {
      const timer = window.setTimeout(callback, ms);
      return () => window.clearTimeout(timer);
    },
  });
}

/** The nearest ancestor that scrolls by itself, `null` when the window does. */
function scrollingParent(el: HTMLElement): HTMLElement | null {
  for (let node = el.parentElement; node; node = node.parentElement) {
    const { overflowY } = window.getComputedStyle(node);
    if (
      (overflowY === "auto" || overflowY === "scroll") &&
      node.scrollHeight > node.clientHeight
    )
      return node;
  }
  return null;
}
