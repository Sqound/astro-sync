// « Régler l'image »: the settings a person applies to a finished image
// (never to the linear data), their words and the per-pixel arithmetic of the
// preview. The window applies the settings itself on the 1080 px preview, in
// the same order and with the same formulas as `stacking/image_adjust.py`,
// which computes the whole image when the person saves (the smoothing alone is
// the script's, on the preview too). Pure TypeScript, tested under `node
// --test`.

import { screenSpikes } from "./diffractionSpikes.ts";
import { conjunction, pluralText } from "./written.ts";

/** The values; each at its neutral value leaves the image as the render made
 * it. */
export interface AdjustValues {
  /** 0..20 %: a black level on the luminance (0: the render's). */
  blackPoint: number;
  /** -50..50: a power on the luminance, blacks and whites kept. */
  brightness: number;
  /** 0..3: a factor on the chroma of the signal (1: the render's). */
  colour: number;
  /** 0..100 %: NL-means (0: none). */
  smoothing: number;
  /** -100..100 %: the star layer taken away (< 0) or added back (> 0). */
  stars: number;
  /** 0..100 %: diffraction spikes added to the bright stars (0: none). Light
   * that is not in the person's data. */
  spikes: number;
  /** 0..100: which stars get spikes, the brightest only (0) to the medium
   * ones too (100). Says nothing while `spikes` is 0, and is then 50. */
  spikesReach: number;
}

export type AdjustName = keyof AdjustValues;

export const NEUTRAL: Readonly<AdjustValues> = {
  blackPoint: 0,
  brightness: 0,
  colour: 1,
  smoothing: 0,
  stars: 0,
  spikes: 0,
  spikesReach: 50,
};

export interface AdjustSpec {
  name: AdjustName;
  min: number;
  max: number;
  step: number;
  titleKey: string;
  helpKey: string;
  /** The setting is in a group of its own on the screen (« Aigrettes »),
   * not in the list of the five. */
  group?: "spikes";
}

/** The settings in the order of the screen (the study's): point noir,
 * luminosité, couleur, lissage, étoiles, then the group « Aigrettes » (force,
 * étoiles concernées). */
export const ADJUST_SPECS: readonly AdjustSpec[] = [
  {
    name: "blackPoint",
    min: 0,
    max: 20,
    step: 1,
    titleKey: "adjust.blackPoint.title",
    helpKey: "adjust.blackPoint.help",
  },
  {
    name: "brightness",
    min: -50,
    max: 50,
    step: 1,
    titleKey: "adjust.brightness.title",
    helpKey: "adjust.brightness.help",
  },
  {
    name: "colour",
    min: 0,
    max: 3,
    step: 0.1,
    titleKey: "adjust.colour.title",
    helpKey: "adjust.colour.help",
  },
  {
    name: "smoothing",
    min: 0,
    max: 100,
    step: 5,
    titleKey: "adjust.smoothing.title",
    helpKey: "adjust.smoothing.help",
  },
  {
    name: "stars",
    min: -100,
    max: 100,
    step: 5,
    titleKey: "adjust.stars.title",
    helpKey: "adjust.stars.help",
  },
  {
    name: "spikes",
    min: 0,
    max: 100,
    step: 5,
    titleKey: "adjust.spikes.strength.title",
    helpKey: "adjust.spikes.strength.help",
    group: "spikes",
  },
  {
    name: "spikesReach",
    min: 0,
    max: 100,
    step: 5,
    titleKey: "adjust.spikes.reach.title",
    helpKey: "adjust.spikes.reach.help",
    group: "spikes",
  },
];

/** A value brought into its range and onto its step. */
export function clampValue(name: AdjustName, value: number): number {
  const spec = ADJUST_SPECS.find((item) => item.name === name)!;
  if (!Number.isFinite(value)) return NEUTRAL[name];
  const stepped = Math.round(value / spec.step) * spec.step;
  return (
    Math.round(Math.min(spec.max, Math.max(spec.min, stepped)) * 1000) / 1000
  );
}

/** The values read back (a record of before, a saved setting): a missing or
 * strange value is neutral. */
export function cleanValues(raw: Partial<Record<string, unknown>> | null) {
  const out = { ...NEUTRAL };
  for (const spec of ADJUST_SPECS) {
    const value = raw?.[spec.name];
    if (typeof value === "number")
      out[spec.name] = clampValue(spec.name, value);
  }
  return canonicalValues(out);
}

/** The values as they are written: with no spikes the reach says nothing, so
 * it is the neutral one (two settings that draw the same image are equal). */
export function canonicalValues(values: AdjustValues): AdjustValues {
  return values.spikes === 0
    ? { ...values, spikesReach: NEUTRAL.spikesReach }
    : values;
}

export function isChanged(values: AdjustValues, name: AdjustName): boolean {
  if (name === "spikesReach" && values.spikes === 0) return false;
  return values[name] !== NEUTRAL[name];
}

/** Whether every setting is at the render's value: « Rien n'a changé ». */
export function isNeutral(values: AdjustValues): boolean {
  return ADJUST_SPECS.every((spec) => !isChanged(values, spec.name));
}

type Translate = (
  key: string,
  params?: Record<string, string | number>,
) => string;

function decimal(locale: string, value: number, digits: number): string {
  return new Intl.NumberFormat(locale, {
    minimumFractionDigits: digits,
    maximumFractionDigits: digits,
  }).format(value);
}

/** The value under a slider: « Comme le rendu », « 8 % », « +12 », « × 1,7
 * », « Sans », « 40 % », « Réduites 40 % », « Renforcées 30 % ». */
export function valueText(
  t: Translate,
  locale: string,
  name: AdjustName,
  value: number,
): string {
  if (name === "smoothing" || name === "spikes") {
    return value === 0
      ? t("adjust.value.none")
      : t("adjust.value.percent", { value: decimal(locale, value, 0) });
  }
  if (name === "spikesReach") return t(reachWordKey(value));
  if (value === NEUTRAL[name]) return t("adjust.value.asRender");
  switch (name) {
    case "blackPoint":
      return t("adjust.value.percent", { value: decimal(locale, value, 0) });
    case "brightness":
      return `${value > 0 ? "+" : "−"}${decimal(locale, Math.abs(value), 0)}`;
    case "colour":
      return t("adjust.value.factor", { value: decimal(locale, value, 1) });
    case "stars":
      return t(
        value < 0 ? "adjust.value.starsLess" : "adjust.value.starsMore",
        {
          value: decimal(locale, Math.abs(value), 0),
        },
      );
  }
}

/** The word of a reach: the brightest stars only, bright ones, down to the
 * medium ones. */
export function reachWordKey(reach: number): string {
  if (reach < 25) return "adjust.spikes.reach.brightest";
  if (reach < 75) return "adjust.spikes.reach.bright";
  return "adjust.spikes.reach.medium";
}

/** « Brillantes · 63 étoiles »: the word of the reach and how many stars it
 * lets through. */
export function reachText(
  t: Translate,
  locale: string,
  reach: number,
  count: number,
): string {
  return t("adjust.spikes.reach.value", {
    word: t(reachWordKey(reach)),
    count: pluralText(t, locale, "adjust.spikes.reach.count", count),
  });
}

/** The value of the folded group: « Sans » or « Ajoutées 60 % ». */
export function spikesGroupText(
  t: Translate,
  locale: string,
  values: AdjustValues,
): string {
  return values.spikes === 0
    ? t("adjust.value.none")
    : t("adjust.spikes.group.added", {
        value: valueText(t, locale, "spikes", values.spikes),
      });
}

/** « Réglée : point noir 8 %, luminosité +12, couleur × 1,7 »: the settings
 * moved, in the order of the screen. */
export function adjustedLine(
  t: Translate,
  locale: string,
  values: AdjustValues,
  quarterTurns = 0,
): string {
  return t("adjust.applied", {
    list: adjustedList(t, locale, values, quarterTurns),
  });
}

/** The words of a turned image set in its line, by quarter turns clockwise. */
const TURN_WORDS: readonly (string | null)[] = [
  null,
  "adjust.applied.turn.right",
  "adjust.applied.turn.half",
  "adjust.applied.turn.left",
];

/** « point noir 8 %, luminosité +12, couleur × 1,7 »: the settings moved, in
 * the order of the screen (the line of a saved setting, the help of its
 * name); « Comme le rendu » when none is. */
export function adjustedList(
  t: Translate,
  locale: string,
  values: AdjustValues,
  quarterTurns = 0,
): string {
  const turn = TURN_WORDS[((quarterTurns % 4) + 4) % 4];
  // The reach is not a term of its own: it only says which stars the spikes
  // go to.
  const moved = ADJUST_SPECS.filter(
    (spec) => spec.name !== "spikesReach" && isChanged(values, spec.name),
  ).map((spec) =>
    spec.name === "stars"
      ? t(
          values.stars < 0
            ? "adjust.applied.starsLess"
            : "adjust.applied.starsMore",
          { value: decimal(locale, Math.abs(values.stars), 0) },
        )
      : t(`adjust.applied.${spec.name}`, {
          value: valueText(t, locale, spec.name, values[spec.name]),
        }),
  );
  const parts = turn ? [t(turn), ...moved] : moved;
  return parts.length > 0
    ? parts.join(t("adjust.applied.join"))
    : t("adjust.value.asRender");
}

/** A row of « Images à envoyer » that can be set: an image of a capture or a
 * variant in PNG -- never « Sans traitement » (the linear file) nor a TIFF in
 * full resolution. */
export function isAdjustable(row: {
  file?: "linear";
  fullResolution?: boolean;
  path: string;
}): boolean {
  return !row.file && !row.fullResolution && /\.png$/i.test(row.path);
}

/** One file of an image set, as the core names it. */
export interface AdjustedFile {
  path: string;
  sha256: string;
  size: number;
}

/** The image set of a render as the core reads it (`AdjustState`): its PNG,
 * and its TIFF when the render has a TIFF in full resolution. */
export interface AdjustedSet extends AdjustedFile {
  values: AdjustValues;
  /** The quarter turns, clockwise, written into the image set. */
  quarterTurns?: number;
  tiff?: AdjustedFile | null;
}

/** What leaves for a row of « Images à envoyer », once its render is set:
 * the image set's PNG for the render's row, the image set's TIFF for the row
 * of its TIFF in full resolution (`twinOf` names the PNG), with the values
 * the row says. `null`: the row leaves as the render made it. */
export function rowAdjustment(
  row: { path: string; fullResolution?: boolean; twinOf?: string },
  sets: Readonly<Record<string, AdjustedSet | null | undefined>>,
): (AdjustedFile & { values: AdjustValues; quarterTurns: number }) | null {
  if (row.fullResolution) {
    const set = row.twinOf ? sets[row.twinOf] : null;
    return set?.tiff
      ? {
          ...set.tiff,
          values: set.values,
          quarterTurns: set.quarterTurns ?? 0,
        }
      : null;
  }
  const set = sets[row.path];
  return set
    ? {
        path: set.path,
        sha256: set.sha256,
        size: set.size,
        values: set.values,
        quarterTurns: set.quarterTurns ?? 0,
      }
    : null;
}

/* ------------------------------- Arithmetic ------------------------------ */

const LR = 0.2126;
const LG = 0.7152;
const LB = 0.0722;

/**
 * The settings but the smoothing on the preview, pixel by pixel: `base` (the
 * preview, or the preview smoothed by the script), `stars` (the star layer
 * previewed, or `null`), `mask` (where the signal is, the colour's weight, one
 * byte per pixel in its red channel), all RGBA of the same size. Writes `out`.
 * A setting at its neutral value is skipped, as the script does. `spikes`: the
 * light of the diffraction spikes (`spikeLayer`), screened over the result;
 * `null`: none.
 */
export function adjustPixels(
  base: ArrayLike<number>,
  stars: ArrayLike<number> | null,
  mask: ArrayLike<number>,
  values: AdjustValues,
  out: { [index: number]: number; length: number },
  spikes: Float32Array | null = null,
): void {
  const k = values.stars / 100;
  const doStars = values.stars !== 0 && stars !== null;
  const b = values.blackPoint / 100;
  const gamma = 2 ** (-values.brightness / 50);
  const factor = values.colour;
  const pixel = [0, 0, 0];
  for (let i = 0; i < base.length; i += 4) {
    pixel[0] = base[i] / 255;
    pixel[1] = base[i + 1] / 255;
    pixel[2] = base[i + 2] / 255;
    adjustPixel(
      pixel,
      doStars ? k : 0,
      doStars ? stars![i] / 255 : 0,
      doStars ? stars![i + 1] / 255 : 0,
      doStars ? stars![i + 2] / 255 : 0,
      b,
      values.brightness === 0 ? 1 : gamma,
      factor,
      mask[i] / 255,
    );
    // The spikes' light, last of all (`spikeLayer`: one pixel is three floats).
    if (spikes) screenSpikes(pixel, spikes, (i / 4) * 3);
    out[i] = Math.round(pixel[0] * 255);
    out[i + 1] = Math.round(pixel[1] * 255);
    out[i + 2] = Math.round(pixel[2] * 255);
    out[i + 3] = 255;
  }
}

/** Negative channels to 0; a pixel whose brightest channel passes 1 brought
 * towards white at its luminance, never darkened (dividing it by that channel
 * darkened the coloured halo of a bright star under its white core: a dark
 * ring). `image_adjust.py`'s `normalise`. */
function normalise(p: number[]): void {
  p[0] = Math.max(0, p[0]);
  p[1] = Math.max(0, p[1]);
  p[2] = Math.max(0, p[2]);
  const top = Math.max(p[0], p[1], p[2]);
  if (top > 1) {
    const level = Math.min(1, Math.max(0, LR * p[0] + LG * p[1] + LB * p[2]));
    const room = Math.min(
      1,
      Math.max(0, (1 - level) / Math.max(top - level, 1e-6)),
    );
    p[0] = level + room * (p[0] - level);
    p[1] = level + room * (p[1] - level);
    p[2] = level + room * (p[2] - level);
  }
  p[0] = Math.min(1, p[0]);
  p[1] = Math.min(1, p[1]);
  p[2] = Math.min(1, p[2]);
}

function withLuminance(p: number[], before: number, after: number): void {
  const scale = after / Math.max(before, 1e-6);
  p[0] *= scale;
  p[1] *= scale;
  p[2] *= scale;
  normalise(p);
}

/** One pixel (RGB in 0..1, changed in place): stars, black point,
 * brightness, colour -- `image_adjust.py`'s `apply` without the smoothing. */
export function adjustPixel(
  p: number[],
  starShare: number,
  sr: number,
  sg: number,
  sb: number,
  blackPoint: number,
  gamma: number,
  colour: number,
  weight: number,
): void {
  if (starShare !== 0) {
    p[0] = Math.min(1, Math.max(0, p[0] + starShare * sr));
    p[1] = Math.min(1, Math.max(0, p[1] + starShare * sg));
    p[2] = Math.min(1, Math.max(0, p[2] + starShare * sb));
  }
  if (blackPoint > 0) {
    const L = LR * p[0] + LG * p[1] + LB * p[2];
    const f = Math.min(1, Math.max(0, (L - blackPoint) / (1 - blackPoint)));
    withLuminance(p, L, f);
  }
  if (gamma !== 1) {
    const L = LR * p[0] + LG * p[1] + LB * p[2];
    withLuminance(p, L, Math.min(1, Math.max(0, L)) ** gamma);
  }
  if (colour !== 1) {
    const L = LR * p[0] + LG * p[1] + LB * p[2];
    const gain = 1 + (colour - 1) * weight;
    p[0] = L + (p[0] - L) * gain;
    p[1] = L + (p[1] - L) * gain;
    p[2] = L + (p[2] - L) * gain;
    normalise(p);
  }
}

/** What an import of saved settings says: how many were added, and the names
 * left out because they were already taken (never overwritten). */
export function importText(
  t: Translate,
  locale: string,
  answer: { added: readonly string[]; skipped: readonly string[] },
): string {
  const parts = [
    pluralText(t, locale, "adjust.saved.imported", answer.added.length),
  ];
  if (answer.skipped.length > 0) {
    parts.push(
      t("adjust.saved.skipped", {
        names: conjunction(
          locale,
          answer.skipped.map((name) => t("adjust.saved.quoted", { name })),
        ),
      }),
    );
  }
  return parts.join(" ");
}
