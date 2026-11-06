// The finish of a render, beside the contrast of presentation: the colour
// intensity and the softened stars. Two more settings of the same kind --
// continuous sliders with magnets at the values proposed, shared by every
// render of the window (the night's and the plan's)
// and by the command line (`--colour none|PERCENT`, `--soft-stars none|PERCENT`).
// Pure TypeScript, tested under `node --test`.

import {
  DEFAULT_CONTRAST,
  contrastToCore,
  contrastValue,
  type Contrast,
} from "./contrast.ts";
import {
  formatNumber,
  snapTo,
  type Detent,
  type SliderSpec,
} from "./snapSlider.ts";

/** The percent the colours are strengthened by, 0 (« Naturelle ») to 100: the
 * chroma of the image multiplied, its luminance kept. */
export type Colour = number;

/** The percent of each bright star taken out, 0 (« Sans ») to 100. */
export type SoftStars = number;

/** The detents of the colour slider: « Naturelle », « Renforcée » (+20 %),
 * « Forte » (+40 %). */
export const COLOUR_STOPS: readonly (Detent & {
  /** The dictionary key of the words that name the setting on an image that
   * carries it (« Couleurs +40 % »); `null` for « none ». */
  summaryKey: string | null;
})[] = [
  { value: 0, labelKey: "wizard.treatments.colour.none", summaryKey: null },
  {
    value: 20,
    labelKey: "wizard.treatments.colour.enhanced",
    summaryKey: "wizard.treatments.colour.summary.enhanced",
  },
  {
    value: 40,
    labelKey: "wizard.treatments.colour.strong",
    summaryKey: "wizard.treatments.colour.summary.strong",
  },
];

/** The detents of the stars slider: « Sans », « Légère » (half of each bright
 * star taken out), « Forte » (85 %). */
export const SOFT_STARS_STOPS: readonly (Detent & {
  summaryKey: string | null;
})[] = [
  { value: 0, labelKey: "wizard.treatments.softStars.none", summaryKey: null },
  {
    value: 50,
    labelKey: "wizard.treatments.softStars.light",
    summaryKey: "wizard.treatments.softStars.summary.light",
  },
  {
    value: 85,
    labelKey: "wizard.treatments.softStars.strong",
    summaryKey: "wizard.treatments.softStars.summary.strong",
  },
];

export const COLOUR_SLIDER: SliderSpec = {
  min: 0,
  max: 100,
  step: 1,
  snap: 3,
  detents: COLOUR_STOPS,
  customKey: "wizard.treatments.colour.custom",
};

export const SOFT_STARS_SLIDER: SliderSpec = {
  min: 0,
  max: 100,
  step: 1,
  snap: 3,
  detents: SOFT_STARS_STOPS,
  customKey: "wizard.treatments.softStars.custom",
};

/** « Naturelle » and « Sans » until the person moves a slider: the finish
 * changes nothing by default (the core's `Colour::None`, `SoftStars::None`). */
export const DEFAULT_COLOUR: Colour = 0;
export const DEFAULT_SOFT_STARS: SoftStars = 0;

const COLOUR_WORDS: Record<string, number> = {
  none: 0,
  enhanced: 20,
  strong: 40,
};
const SOFT_STARS_WORDS: Record<string, number> = {
  none: 0,
  light: 50,
  strong: 85,
};

/** A percent from what a settings file or a record holds: a number, a word of
 * the first version, or nothing (0). Outside 0..100 is clamped. */
function percentOf(value: unknown, words: Record<string, number>): number {
  if (typeof value === "number" && Number.isFinite(value)) {
    return Math.min(100, Math.max(0, Math.round(value)));
  }
  if (typeof value === "string") {
    if (value in words) return words[value];
    const number = Number(value);
    if (value.trim() !== "" && Number.isFinite(number)) {
      return percentOf(number, words);
    }
  }
  return 0;
}

export function colourOf(value: unknown): Colour {
  return percentOf(value, COLOUR_WORDS);
}

export function softStarsOf(value: unknown): SoftStars {
  return percentOf(value, SOFT_STARS_WORDS);
}

/** What the slider holds for a raw position (the magnets of the detents). */
export function colourSnap(raw: number): Colour {
  return snapTo(COLOUR_SLIDER, raw);
}

export function softStarsSnap(raw: number): SoftStars {
  return snapTo(SOFT_STARS_SLIDER, raw);
}

/** The finish an image carries, as the stacking reported it (absent or `0`:
 * nothing). The values may still be words of the core. */
export interface Finish {
  colour?: unknown;
  softStars?: unknown;
}

/** Whether an image carries a colour intensity or softened stars: it is then
 * a variant of the render, never the capture's main image. */
export function hasFinish(finish: Finish): boolean {
  return colourOf(finish.colour) > 0 || softStarsOf(finish.softStars) > 0;
}

/** The words that name the finish of an image (« Couleurs +40 % »,
 * « Étoiles adoucies 85 % »), in the order the stacking applies them: the name
 * of a detent when the value is one, else the same words with the value. */
export function finishSummaries(
  finish: Finish,
  t: (key: string, params?: Record<string, string | number>) => string,
): string[] {
  const colour = colourOf(finish.colour);
  const stars = softStarsOf(finish.softStars);
  const out: string[] = [];
  if (colour > 0) {
    const stop = COLOUR_STOPS.find((s) => s.value === colour);
    out.push(
      stop?.summaryKey
        ? t(stop.summaryKey)
        : t("wizard.treatments.colour.summary.custom", { value: colour }),
    );
  }
  if (stars > 0) {
    const stop = SOFT_STARS_STOPS.find((s) => s.value === stars);
    out.push(
      stop?.summaryKey
        ? t(stop.summaryKey)
        : t("wizard.treatments.softStars.summary.custom", { value: stars }),
    );
  }
  return out;
}

/* --------------------- The finish of each pipeline ------------------------ */

/** The three settings of one finish: the « Finition par défaut » of the
 * night, or the finish of one pipeline of one target (« Finition » of its
 * card in « Pipelines pour {cible} »). */
export interface PipelineFinish {
  contrast: Contrast;
  colour: Colour;
  softStars: SoftStars;
}

/** The finish of a window that was never touched: « Présentation »,
 * « Naturelle », « Sans » (the core's defaults). */
export const DEFAULT_FINISH: PipelineFinish = {
  contrast: DEFAULT_CONTRAST,
  colour: DEFAULT_COLOUR,
  softStars: DEFAULT_SOFT_STARS,
};

/** The finishes a person set by hand, by target id then pipeline id. A
 * pipeline not named follows the « Finition par défaut », whatever it
 * becomes; one named keeps its values (« Modifiée ») until « Rétablir le
 * défaut ». « Sans traitement » is never named. */
export type TargetFinishes = Readonly<
  Record<string, Readonly<Record<string, PipelineFinish>>>
>;

export function sameFinish(a: PipelineFinish, b: PipelineFinish): boolean {
  return (
    a.contrast === b.contrast &&
    a.colour === b.colour &&
    a.softStars === b.softStars
  );
}

/** The finish of `pipelineId` for `targetId`, and whether it is its own. */
export function finishOf(
  finishes: TargetFinishes | undefined,
  targetId: string,
  pipelineId: string,
  defaults: PipelineFinish,
): { finish: PipelineFinish; own: boolean } {
  const own = finishes?.[targetId]?.[pipelineId];
  return own ? { finish: own, own: true } : { finish: defaults, own: false };
}

/** « Présentation · Couleurs +20 % · Étoiles 50 % »: the folded line of a
 * finish, each setting named even at zero (« Sans contraste », « Couleurs
 * naturelles », « Étoiles intactes »). */
export function finishLine(
  finish: PipelineFinish,
  t: (key: string, params?: Record<string, string | number>) => string,
  locale: string,
): string {
  const contrast =
    finish.contrast === 0
      ? t("wizard.finish.contrast.none")
      : finish.contrast === 2
        ? t("wizard.finish.contrast.presentation")
        : finish.contrast === 3.6
          ? t("wizard.finish.contrast.strong")
          : t("wizard.finish.contrast.custom", {
              value: formatNumber(locale, finish.contrast, 1),
            });
  const colour =
    finish.colour === 0
      ? t("wizard.finish.colour.none")
      : t("wizard.finish.colour.value", { value: finish.colour });
  const stars =
    finish.softStars === 0
      ? t("wizard.finish.stars.none")
      : t("wizard.finish.stars.value", { value: finish.softStars });
  return [contrast, colour, stars].join(" · ");
}

/** The finishes of one target that differ from the default, in the order
 * of `chosen` (« Sans traitement » never): the lines « Finition modifiée »
 * of its card. A finish set by hand to the default's values says nothing. */
export function changedFinishes(
  finishes: TargetFinishes | undefined,
  targetId: string,
  chosen: readonly string[],
  defaults: PipelineFinish,
  noProcessingId: string,
): { pipelineId: string; finish: PipelineFinish }[] {
  return chosen
    .filter((id) => id !== noProcessingId)
    .map((pipelineId) => ({
      pipelineId,
      ...finishOf(finishes, targetId, pipelineId, defaults),
    }))
    .filter((item) => item.own && !sameFinish(item.finish, defaults))
    .map(({ pipelineId, finish }) => ({ pipelineId, finish }));
}

/** What the core is sent for one pipeline (`PipelineChoice::finish`):
 * nothing when it follows the default, else its three settings. */
export function finishToCore(
  finish: PipelineFinish | undefined,
): { contrast: string | number; colour: number; softStars: number } | null {
  if (!finish) return null;
  return {
    contrast: contrastToCore(finish.contrast),
    colour: finish.colour,
    softStars: finish.softStars,
  };
}

/** The pipelines of one target as the stacking is asked for them: each
 * with its own finish when it has one. */
export function pipelinesToRun(
  ids: readonly string[],
  finishes: TargetFinishes | undefined,
  targetId: string,
  noProcessingId: string,
): {
  pipelineId: string;
  trustedSha256: null;
  finish?: { contrast: string | number; colour: number; softStars: number };
}[] {
  return ids.map((pipelineId) => {
    const own =
      pipelineId === noProcessingId
        ? null
        : finishToCore(finishes?.[targetId]?.[pipelineId]);
    return own
      ? { pipelineId, trustedSha256: null, finish: own }
      : { pipelineId, trustedSha256: null };
  });
}

/** One target's finishes after the window « Pipelines pour {cible} »: those
 * of the pipelines left ticked, « Sans traitement » never. */
export function withTargetFinishes(
  finishes: TargetFinishes | undefined,
  targetId: string,
  own: Readonly<Record<string, PipelineFinish>>,
  selection: readonly string[],
  noProcessingId: string,
): Record<string, Record<string, PipelineFinish>> {
  const next: Record<string, Record<string, PipelineFinish>> = {};
  for (const [key, value] of Object.entries(finishes ?? {})) {
    next[key] = { ...value };
  }
  const kept: Record<string, PipelineFinish> = {};
  for (const id of selection) {
    if (id !== noProcessingId && own[id]) kept[id] = { ...own[id] };
  }
  if (Object.keys(kept).length > 0) next[targetId] = kept;
  else delete next[targetId];
  return next;
}

/** « Appliquer à toutes les cibles »: each target receives the finish of
 * each pipeline ticked, by pipeline id -- its own finish, or the default
 * when the source follows it. */
export function applyFinishesToAll(
  finishes: TargetFinishes | undefined,
  targetIds: Iterable<string>,
  own: Readonly<Record<string, PipelineFinish>>,
  selection: readonly string[],
  noProcessingId: string,
): Record<string, Record<string, PipelineFinish>> {
  let next: Record<string, Record<string, PipelineFinish>> = { ...finishes };
  for (const targetId of targetIds) {
    next = withTargetFinishes(next, targetId, own, selection, noProcessingId);
  }
  return next;
}

/* ------------------------ Kept on this computer --------------------------- */

interface FinishStorage {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
}

const finishesKey = (sessionId: string) => `astro-sync.finishes:${sessionId}`;

/** A finish from what the storage holds; `null` for what is not one. */
function storedFinish(value: unknown): PipelineFinish | null {
  if (!value || typeof value !== "object") return null;
  const held = value as Record<string, unknown>;
  return {
    contrast: contrastValue(held.contrast),
    colour: colourOf(held.colour),
    softStars: softStarsOf(held.softStars),
  };
}

/** The finishes set by hand for the targets of a night, kept on this
 * computer beside the pipelines chosen (`astro-sync.pipelines:<night>`): they
 * survive the window closed and the next day. What cannot be read follows the
 * default. */
export function loadTargetFinishes(
  storage: FinishStorage | null,
  sessionId: string,
): Record<string, Record<string, PipelineFinish>> {
  const out: Record<string, Record<string, PipelineFinish>> = {};
  try {
    const text = storage?.getItem(finishesKey(sessionId));
    const parsed = text ? (JSON.parse(text) as unknown) : null;
    if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) {
      return out;
    }
    for (const [targetId, pipelines] of Object.entries(
      parsed as Record<string, unknown>,
    )) {
      if (!pipelines || typeof pipelines !== "object") continue;
      const kept: Record<string, PipelineFinish> = {};
      for (const [pipelineId, value] of Object.entries(
        pipelines as Record<string, unknown>,
      )) {
        const finish = storedFinish(value);
        if (finish) kept[pipelineId] = finish;
      }
      if (Object.keys(kept).length > 0) out[targetId] = kept;
    }
  } catch {
    // Finishes that cannot be read: every pipeline follows the default.
  }
  return out;
}

export function saveTargetFinishes(
  storage: FinishStorage | null,
  sessionId: string,
  finishes: TargetFinishes,
): void {
  try {
    storage?.setItem(finishesKey(sessionId), JSON.stringify(finishes));
  } catch {
    // A finish lost only brings the default back.
  }
}
