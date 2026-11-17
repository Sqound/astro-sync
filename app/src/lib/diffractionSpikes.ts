// Diffraction spikes added to a finished image, the last setting of « Régler
// l'image »: four thin rays (horizontal and vertical) on the bright stars,
// their length and intensity following each star's flux. This is the
// preview's drawing, the same formulas as `stacking/diffraction_spikes.py`
// (which draws the whole image when the person saves), on the same list of
// stars (`stars.json`, measured once at full size by the script). Pure
// TypeScript, tested under `node --test` against the script's own pictures.

/** One star of the list, at the size of the whole image. */
export interface SpikeStar {
  x: number;
  y: number;
  /** The flux, normalised to an image whose long side is 1000 px. */
  flux: number;
  radius: number;
  /** A chroma at a luminance of 1. */
  colour: [number, number, number];
}

export interface SpikeStars {
  version: number;
  stars: SpikeStar[];
}

// The flux of the faintest star that gets spikes, at the two ends of the reach.
const FLUX_AT_REACH_0 = 40;
const FLUX_AT_REACH_100 = 3;
// At full strength a star of FLUX_AT_REACH_0 has arms of this share of the long
// side; the longest arm, whatever the star.
const LENGTH_AT_FULL = 0.1;
const LONGEST = 0.22;
// The Gaussian width of a spike, as a share of the long side.
const WIDTH = 0.00035;
const WHITE_CORE = 0.6;
// The red end of a spike reaches further than the green, the blue less.
const DISPERSION = [1.12, 1, 0.88] as const;
// Horizontal and vertical, x to the right, y down.
const ARMS = [
  [1, 0],
  [0, 1],
  [-1, 0],
  [0, -1],
] as const;

/** The flux of the faintest star that gets spikes at `reach` (0..100). */
export function fluxThreshold(reach: number): number {
  const t = Math.min(100, Math.max(0, reach)) / 100;
  return FLUX_AT_REACH_0 * (FLUX_AT_REACH_100 / FLUX_AT_REACH_0) ** t;
}

/** How many stars the reach lets through (the number the screen shows). */
export function starCount(stars: readonly SpikeStar[], reach: number): number {
  const threshold = fluxThreshold(reach);
  let count = 0;
  for (const star of stars) if (star.flux >= threshold) count += 1;
  return count;
}

/**
 * The light of the spikes alone: width × height × 3 floats in 0..1, or `null`
 * when nothing is drawn (no strength, no star, no star let through).
 * `scale`: this picture's long side over the whole image's (the preview: < 1).
 * `starFactor`: « Étoiles » as a factor (1 + stars / 100), so taking the
 * stars away takes their spikes too.
 */
export function spikeLayer(
  width: number,
  height: number,
  stars: readonly SpikeStar[],
  strength: number,
  reach: number,
  scale: number,
  starFactor: number,
): Float32Array | null {
  if (strength <= 0 || starFactor <= 0 || stars.length === 0) return null;
  const s = Math.min(strength, 100) / 100;
  const longSide = Math.max(width, height);
  const threshold = fluxThreshold(reach);
  const out = new Float32Array(width * height * 3);
  let drawn = false;
  const tint = [0, 0, 0];
  for (const star of stars) {
    if (star.flux < threshold) continue;
    const x0 = (star.x + 0.5) * scale - 0.5;
    const y0 = (star.y + 0.5) * scale - 0.5;
    const r0 = Math.max(star.radius * scale, 1);
    const rel = star.flux / FLUX_AT_REACH_0;
    const length =
      Math.min(LENGTH_AT_FULL * s ** 0.7 * Math.sqrt(rel), LONGEST * s ** 0.7) *
      longSide;
    if (length < 3 * r0) continue;
    const i0 = s ** 0.35 * starFactor * (1 - Math.exp(-1.2 * Math.sqrt(rel)));
    const sigma = Math.max(
      0.5,
      WIDTH * longSide * (1 + 0.35 * Math.log2(1 + rel)),
    );
    for (let k = 0; k < 3; k += 1) {
      tint[k] = Math.max(0, WHITE_CORE + (1 - WHITE_CORE) * star.colour[k]);
    }
    const reachPx = length * DISPERSION[0];
    const half = 6.6 * sigma;
    for (const [ex, ey] of ARMS) {
      const farX = x0 + ex * reachPx;
      const farY = y0 + ey * reachPx;
      const xLo = Math.max(
        0,
        Math.floor(Math.min(x0, farX) - half * Math.abs(ey)),
      );
      const xHi = Math.min(
        width,
        Math.ceil(Math.max(x0, farX) + half * Math.abs(ey)) + 1,
      );
      const yLo = Math.max(
        0,
        Math.floor(Math.min(y0, farY) - half * Math.abs(ex)),
      );
      const yHi = Math.min(
        height,
        Math.ceil(Math.max(y0, farY) + half * Math.abs(ex)) + 1,
      );
      for (let gy = yLo; gy < yHi; gy += 1) {
        for (let gx = xLo; gx < xHi; gx += 1) {
          const dx = gx - x0;
          const dy = gy - y0;
          const along = dx * ex + dy * ey;
          if (along <= 0) continue;
          const across = -dx * ey + dy * ex;
          if (Math.abs(across) >= half) continue;
          // The base of a spike opens out of the star's halo, then settles.
          const wide = sigma * (1 + 1.2 * Math.exp(-along / (2.5 * r0)));
          const shape =
            Math.exp(-0.5 * (across / wide) ** 2) *
            (1 - Math.exp(-((along / r0) ** 2)));
          const at = (gy * width + gx) * 3;
          for (let k = 0; k < 3; k += 1) {
            const lk = length * DISPERSION[k];
            const t = Math.min(1, Math.max(0, along / lk));
            const profile = (1 / (1 + along / (0.07 * lk))) * (1 - t * t) ** 2;
            const v = i0 * tint[k] * profile * shape;
            out[at + k] = 1 - (1 - out[at + k]) * (1 - v);
          }
          drawn = true;
        }
      }
    }
  }
  if (!drawn) return null;
  for (let i = 0; i < out.length; i += 1) out[i] = Math.min(1, out[i]);
  return out;
}

/** One pixel of the image (RGB, 0..1, changed in place) with the spikes' light
 * `layer[at..at + 3]` added in screen mode. A pixel the light does not touch
 * keeps its value exactly (per channel): `1 - (1 - x)` is not always `x`. */
export function screenSpikes(
  p: number[],
  layer: Float32Array,
  at: number,
): void {
  for (let k = 0; k < 3; k += 1) {
    const light = layer[at + k];
    if (light > 0) p[k] = 1 - (1 - p[k]) * (1 - light);
  }
}
