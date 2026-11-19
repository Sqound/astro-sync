// No text selected while something is dragged.
//
// A slider or the curtain of « Régler l'image » is dragged with the pointer
// held down; as soon as the pointer leaves the frame (or the window), the web
// view starts selecting the text under it and the whole panel turns blue. The
// style sheet (`theme/selection.css`) forbids selecting the interface; this
// guard covers the drag itself: from the pointer going down on a slider to the
// pointer going up, `as-dragging` is on the root (nothing is selectable, in
// any zone), `selectstart` is refused, and what a selection had caught is
// cleared when it ends. Pure decisions here, tested under `node --test`;
// `installDragGuard` is the thin installer `main.tsx` calls.

/** What is dragged: a range input, anything that says it is a slider, and
 * what declares itself a drag surface (`data-as-drag`, the curtain's frame). */
export const DRAG_SELECTOR =
  'input[type="range"], [role="slider"], [data-as-drag]';

/** The class the root carries while something is dragged. */
export const DRAGGING_CLASS = "as-dragging";

/** The events that end a drag, wherever the pointer is. */
export const END_EVENTS = [
  "pointerup",
  "pointercancel",
  "mouseup",
  "dragend",
  "blur",
] as const;

/** `blur` is heard on the window itself only (the application loses the
 * focus): in the capture phase it would also hear the field that loses the
 * focus when a slider is clicked, and end the drag as it starts. */
export function listensInCapture(type: string): boolean {
  return type !== "blur";
}

/** A pointer going down starts a drag only with the main button (or a
 * finger) on something draggable. */
export function startsDrag(event: {
  button: number;
  matchesDraggable: boolean;
}): boolean {
  return event.button === 0 && event.matchesDraggable;
}

/** The slice of the window the guard uses (what a test replaces). */
export interface DragHost {
  addEventListener(
    type: string,
    listener: (event: DragEventLike) => void,
    capture: boolean,
  ): void;
  removeEventListener(
    type: string,
    listener: (event: DragEventLike) => void,
    capture: boolean,
  ): void;
  /** The root element's class list. */
  rootClasses: { add(name: string): void; remove(name: string): void };
  /** Forgets whatever is selected. */
  clearSelection(): void;
}

export interface DragEventLike {
  button?: number;
  preventDefault(): void;
}

/** Installs the guard on `host`, `isDraggable` saying whether the element a
 * pointer went down on can be dragged; the function it returns removes it. */
export function createDragGuard(
  host: DragHost,
  isDraggable: (event: DragEventLike) => boolean,
): () => void {
  let dragging = false;
  const refuseSelection = (event: DragEventLike) => {
    if (dragging) event.preventDefault();
  };
  const end = () => {
    if (!dragging) return;
    dragging = false;
    host.rootClasses.remove(DRAGGING_CLASS);
    host.removeEventListener("selectstart", refuseSelection, true);
    for (const type of END_EVENTS)
      host.removeEventListener(type, end, listensInCapture(type));
    // Whatever a selection caught on the way is let go.
    host.clearSelection();
  };
  const down = (event: DragEventLike) => {
    if (
      dragging ||
      !startsDrag({
        button: event.button ?? 0,
        matchesDraggable: isDraggable(event),
      })
    ) {
      return;
    }
    dragging = true;
    host.rootClasses.add(DRAGGING_CLASS);
    host.clearSelection();
    host.addEventListener("selectstart", refuseSelection, true);
    for (const type of END_EVENTS)
      host.addEventListener(type, end, listensInCapture(type));
  };
  host.addEventListener("pointerdown", down, true);
  return () => {
    end();
    host.removeEventListener("pointerdown", down, true);
  };
}

/** The guard on the window of the application. */
export function installDragGuard(target: Window): () => void {
  return createDragGuard(
    {
      addEventListener: (type, listener, capture) =>
        target.addEventListener(type, listener as EventListener, capture),
      removeEventListener: (type, listener, capture) =>
        target.removeEventListener(type, listener as EventListener, capture),
      rootClasses: target.document.documentElement.classList,
      clearSelection: () => target.getSelection()?.removeAllRanges(),
    },
    (event) => {
      const element = (event as unknown as Event).target;
      return (
        element instanceof Element && element.closest(DRAG_SELECTOR) !== null
      );
    },
  );
}
