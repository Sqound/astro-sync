// The contrast of presentation: one setting, shared by every render of the
// window (the night's and the plan's) and by the command line
// (`--contrast none|presentation|strong|GAIN`). The setting is the gain of the
// S-curve, a number from 0 (none) to 5 on the slider; the slider holds on to
// the two values the window always offered, 2.0 « Présentation » and 3.6
// « Fort ». Pure TypeScript, tested under `node --test`.

import { snapTo, type Detent, type SliderSpec } from "./snapSlider.ts";

/** The gain of the S-curve; 0 is « Sans » (the master of the catalogue as it is). */
export type Contrast = number;

/** The largest gain the slider offers (the core accepts up to 6). */
export const CONTRAST_MAX = 5;

/** The detents of the slider: « Sans », « Présentation » (gain 2.0, the
 * screen's reference), « Fort » (gain 3.6, the print gain, good on a screen as
 * well). */
export const CONTRAST_STOPS: readonly (Detent & { gain: number | null })[] = [
  { value: 0, gain: null, labelKey: "wizard.treatments.contrast.none" },
  {
    value: 2,
    gain: 2.0,
    labelKey: "wizard.treatments.contrast.presentation",
  },
  { value: 3.6, gain: 3.6, labelKey: "wizard.treatments.contrast.strong" },
];

export const CONTRAST_SLIDER: SliderSpec = {
  min: 0,
  max: CONTRAST_MAX,
  step: 0.05,
  snap: 0.1,
  detents: CONTRAST_STOPS,
  customKey: "wizard.treatments.contrast.custom",
  minDecimals: 1,
};

/** « Présentation 2,0 » until the person moves the slider (the core's
 * `window_contrast`). */
export const DEFAULT_CONTRAST: Contrast = 2;

const WORDS: Record<string, number> = { none: 0, presentation: 2, strong: 3.6 };

/** The gain of what a record says of an image: a word of the core
 * (`none`, `presentation`, `strong`), a number, or nothing (no curve: 0). */
export function contrastValue(value: unknown): Contrast {
  if (typeof value === "number" && Number.isFinite(value)) {
    return Math.min(CONTRAST_MAX, Math.max(0, value));
  }
  if (typeof value === "string") {
    if (value in WORDS) return WORDS[value];
    const number = Number(value);
    if (value.trim() !== "" && Number.isFinite(number)) {
      return contrastValue(number);
    }
  }
  return 0;
}

/** What a settings file may hold: a value that is not one reads as the
 * default. */
export function contrastOf(value: unknown): Contrast {
  const known =
    typeof value === "number" ||
    (typeof value === "string" &&
      (value in WORDS ||
        (value.trim() !== "" && Number.isFinite(Number(value)))));
  return known ? contrastValue(value) : DEFAULT_CONTRAST;
}

/** Whether the contrast changes the image. */
export function hasContrast(value: unknown): boolean {
  return contrastValue(value) > 0;
}

/** What the slider holds for a raw position (the magnets of the detents). */
export function contrastSnap(raw: number): Contrast {
  return snapTo(CONTRAST_SLIDER, raw);
}

/** What the core is sent: a word for the two historical values, else the number. */
export function contrastToCore(value: Contrast): string | number {
  if (value === 0) return "none";
  if (value === 2) return "presentation";
  if (value === 3.6) return "strong";
  return value;
}
