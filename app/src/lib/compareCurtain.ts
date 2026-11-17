// « Régler l'image »: the curtain that compares « Avant » (the render, left of
// the bar) and « Après » (the settings, right of it). The position is the
// share of the image, 0..100 %, that shows « Avant »: 0 is all « Après » (how
// the screen opens, and where it returns when a setting moves), 100 is all
// « Avant ». Pure TypeScript, tested under `node --test`.

export const CURTAIN_MIN = 0;
export const CURTAIN_MAX = 100;
/** Where the bar rests when the screen opens and when a setting moves. */
export const CURTAIN_START = 0;
/** One press of an arrow, and with Shift. */
export const CURTAIN_STEP = 2;
export const CURTAIN_STEP_BIG = 10;
/** A label « Avant » / « Après » fades when the bar is nearer than this
 * (in px) to the corner it sits in. */
export const LABEL_CLEARANCE_PX = 96;

/** A position brought into 0..100 (a value that is not a number is the
 * start). */
export function clampCurtain(position: number): number {
  if (!Number.isFinite(position)) return CURTAIN_START;
  return Math.min(CURTAIN_MAX, Math.max(CURTAIN_MIN, position));
}

/** The position of the bar for a pointer at `clientX` over an image whose
 * left edge is `left` and whose width is `width` (px). */
export function curtainFromPointer(
  clientX: number,
  left: number,
  width: number,
): number {
  if (!(width > 0)) return CURTAIN_START;
  return clampCurtain(((clientX - left) / width) * 100);
}

/** What a key does to the handle: the new position, or `null` for a key that
 * is not the handle's. Arrows move 2 % (10 % with Shift), Home goes to
 * « tout Après » and End to « tout Avant ». */
export function curtainFromKey(
  position: number,
  key: string,
  shift: boolean,
): number | null {
  const step = shift ? CURTAIN_STEP_BIG : CURTAIN_STEP;
  switch (key) {
    case "ArrowLeft":
    case "ArrowDown":
      return clampCurtain(position - step);
    case "ArrowRight":
    case "ArrowUp":
      return clampCurtain(position + step);
    case "Home":
      return CURTAIN_MIN;
    case "End":
      return CURTAIN_MAX;
    default:
      return null;
  }
}

/** A double click on the bar: all « Avant » when it was nearer « Après », all
 * « Après » otherwise. */
export function curtainToggle(position: number): number {
  return position < 50 ? CURTAIN_MAX : CURTAIN_MIN;
}

/** Whether the label of a side shows: not when the bar is within
 * [`LABEL_CLEARANCE_PX`] of its corner. `width` is the image's width in px. */
export function labelVisible(
  side: "before" | "after",
  position: number,
  width: number,
): boolean {
  const bar = (clampCurtain(position) / 100) * width;
  return side === "before"
    ? bar >= LABEL_CLEARANCE_PX
    : width - bar >= LABEL_CLEARANCE_PX;
}
