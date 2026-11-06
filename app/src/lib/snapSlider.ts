// The sliders of the finish and of the contrast: a continuous range with
// magnetic detents at the values the window proposes. Pure TypeScript, tested
// under `node --test`.

/** A value the slider holds on to, and the dictionary key of its name. */
export interface Detent {
  value: number;
  /** The label shown under the slider while it sits on the detent. */
  labelKey: string;
}

/** The shape of one slider. */
export interface SliderSpec {
  min: number;
  max: number;
  /** The grain of a keyboard step and of a drag. */
  step: number;
  /** A value this close to a detent (or nearer) is held on it. */
  snap: number;
  detents: readonly Detent[];
  /** The label of a value that is not a detent (`{value}` is the number). */
  customKey: string;
  /** At least this many decimals when the value is written (2,0 and not 2). */
  minDecimals?: number;
}

/** The number of decimals of a step (0.05 -> 2). */
function decimals(step: number): number {
  const text = String(step);
  return text.includes(".") ? text.split(".")[1].length : 0;
}

/**
 * What the slider holds when the range input says `raw`: clamped to the range,
 * set on the grain of the step, and held on the nearest detent when it is within
 * the magnet's reach. A value that is not a number is the minimum.
 */
export function snapTo(spec: SliderSpec, raw: number): number {
  const base = Number.isFinite(raw) ? raw : spec.min;
  const clamped = Math.min(spec.max, Math.max(spec.min, base));
  const grained = Number(
    (
      Math.round((clamped - spec.min) / spec.step) * spec.step +
      spec.min
    ).toFixed(decimals(spec.step)),
  );
  let nearest: Detent | null = null;
  for (const detent of spec.detents) {
    const gap = Math.abs(detent.value - grained);
    if (
      gap <= spec.snap + 1e-9 &&
      (nearest === null || gap < Math.abs(nearest.value - grained))
    ) {
      nearest = detent;
    }
  }
  return nearest ? nearest.value : grained;
}

/** The detent the value sits on, or `null` between two. */
export function detentOf(spec: SliderSpec, value: number): Detent | null {
  return (
    spec.detents.find((detent) => Math.abs(detent.value - value) < 1e-9) ?? null
  );
}

/** Where a detent stands along the track, 0 to 1. */
export function fractionOf(spec: SliderSpec, value: number): number {
  return (value - spec.min) / (spec.max - spec.min);
}

/**
 * The words under the slider: the name of the detent it sits on, or the
 * custom label with the exact value (`formatted`, in the language's own digits).
 */
export function sliderLabel(
  spec: SliderSpec,
  value: number,
  t: (key: string, params?: Record<string, string | number>) => string,
  formatted: string,
): string {
  const detent = detentOf(spec, value);
  return detent ? t(detent.labelKey) : t(spec.customKey, { value: formatted });
}

/** A number as the language writes it (2,4 or 2.4), at most two decimals. */
export function formatNumber(
  locale: string,
  value: number,
  minDecimals = 0,
): string {
  return new Intl.NumberFormat(locale, {
    minimumFractionDigits: minDecimals,
    maximumFractionDigits: 2,
  }).format(value);
}
