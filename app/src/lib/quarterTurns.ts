// « Régler l'image »: the quarter turns of an image, clockwise, as the portal
// turns a capture (0, 90, 180, 270 degrees, no free angle, no mirror). Here the
// turn is written into the image: the preview is drawn turned, the whole image
// is written turned. A turn belongs to the image, not to the five settings: a
// saved setting never carries it. Pure TypeScript, tested under `node --test`.

/** A count of quarter turns, clockwise: 0, 1 (90° right), 2, 3 (90° left). */
export type Turns = 0 | 1 | 2 | 3;

/** Any number brought into 0..3 (a value that is not a number is 0). */
export function cleanTurns(raw: unknown): Turns {
  if (typeof raw !== "number" || !Number.isFinite(raw)) return 0;
  const whole = Math.trunc(raw);
  return (((whole % 4) + 4) % 4) as Turns;
}

/** « Pivoter à droite » (+1) or « Pivoter à gauche » (-1). */
export function turnBy(turns: Turns, direction: "left" | "right"): Turns {
  return cleanTurns(turns + (direction === "right" ? 1 : -1));
}

/** The size of an image of `width` × `height` once turned: the sides swap on
 * an odd count. */
export function turnedSize(
  turns: Turns,
  width: number,
  height: number,
): { width: number; height: number } {
  return turns % 2 === 1 ? { width: height, height: width } : { width, height };
}

/** The matrix `[a, b, c, d, e, f]` of `CanvasRenderingContext2D.setTransform`
 * that draws an image of `width` × `height` turned: a point (x, y) of the
 * image lands at (a·x + c·y + e, b·x + d·y + f) of the turned picture. */
export function turnMatrix(
  turns: Turns,
  width: number,
  height: number,
): [number, number, number, number, number, number] {
  switch (turns) {
    case 1:
      return [0, 1, -1, 0, height, 0];
    case 2:
      return [-1, 0, 0, -1, width, height];
    case 3:
      return [0, -1, 1, 0, 0, width];
    default:
      return [1, 0, 0, 1, 0, 0];
  }
}

/** The key of the words that say the state of a turned image (« Tourné de
 * 90° à droite », « de 180° », « de 90° à gauche »); `null` when it is not
 * turned. */
export function turnStateKey(turns: Turns): string | null {
  switch (turns) {
    case 1:
      return "adjust.rotate.state.right";
    case 2:
      return "adjust.rotate.state.half";
    case 3:
      return "adjust.rotate.state.left";
    default:
      return null;
  }
}

/** The pixels of `data` (RGBA, `width` × `height`) turned, the same
 * arithmetic as the matrix and as the script's `numpy.rot90`: used to test the
 * mapping, and by the preview when it must read the turned pixels. */
export function turnPixels(
  data: Uint8ClampedArray,
  width: number,
  height: number,
  turns: Turns,
): Uint8ClampedArray {
  if (turns === 0) return data;
  const out = new Uint8ClampedArray(data.length);
  const outWidth = turns % 2 === 1 ? height : width;
  for (let y = 0; y < height; y++) {
    for (let x = 0; x < width; x++) {
      const [nx, ny] =
        turns === 1
          ? [height - 1 - y, x]
          : turns === 2
            ? [width - 1 - x, height - 1 - y]
            : [y, width - 1 - x];
      const from = (y * width + x) * 4;
      const to = (ny * outWidth + nx) * 4;
      out[to] = data[from];
      out[to + 1] = data[from + 1];
      out[to + 2] = data[from + 2];
      out[to + 3] = data[from + 3];
    }
  }
  return out;
}
