// The adjustable frame of the image of a multi-night plan in « Traitements »:
// a slider of notches from the cropped image (what every
// integration covers) to the whole image (the default and the last notch), the
// numbers each notch says, the line beyond the mark, what « Récapitulatif »
// says. The notches are measured by `plan_stack.py` and read from the record of
// the last image made with the same integrations (`planFramingRead`); nothing is
// recomputed here. Pure TypeScript, tested under `node --test`.

import type { TranslateParams } from "../i18n/locale.ts";
import type { FramingNotch, PlanFrame, PlanFraming } from "./bridge";
import { integrationTime } from "./multiNight.ts";

type Translate = (key: string, params?: TranslateParams) => string;

/** Whether two frames are the same four integers. */
export function sameFrame(a: PlanFrame, b: PlanFrame): boolean {
  return (
    a.top === b.top &&
    a.left === b.left &&
    a.height === b.height &&
    a.width === b.width
  );
}

/** The notch the slider stands on. */
export interface FramingChoice {
  index: number;
  notch: FramingNotch;
}

/**
 * The notch chosen: the one whose four integers are the frame kept, the last
 * (the whole image, the default) when none is. `null` when there is nothing to
 * choose -- no record yet (the first image is made whole), less than 1 % of the
 * image below the full exposure, no smaller frame to take: the block is not
 * shown and no frame is sent.
 */
export function resolveFrame(
  framing: PlanFraming | null,
  kept: PlanFrame | null,
): FramingChoice | null {
  if (!framing?.offered || framing.notches.length < 2) return null;
  const found = kept
    ? framing.notches.findIndex((notch) => sameFrame(notch, kept))
    : -1;
  const index = found >= 0 ? found : framing.notches.length - 1;
  return { index, notch: framing.notches[index] };
}

/** The frame the job carries: the notch chosen, unless it is the whole image
 * (nothing to cut). */
export function frameToSend(
  framing: PlanFraming | null,
  kept: PlanFrame | null,
): PlanFrame | undefined {
  const choice = resolveFrame(framing, kept);
  if (!choice || choice.notch.default) return undefined;
  const { top, left, height, width } = choice.notch;
  return { top, left, height, width };
}

/** What « Récapitulatif » says of the frame: its size, only when it is not the
 * whole image (`multiNight.summary.cropped`). */
export function summaryFrame(
  framing: PlanFraming | null,
  kept: PlanFrame | null,
): { width: number; height: number } | null {
  const frame = frameToSend(framing, kept);
  return frame ? { width: frame.width, height: frame.height } : null;
}

/** One stop of the slider, with what is written under it. */
export interface StopView {
  /** « 15 % »: the share of the image at reduced exposure, under each notch. */
  percent: string;
  /** « Rognée », « Entière »: the first and the last; the stops between say nothing. */
  name: string | null;
  mark: boolean;
  isDefault: boolean;
}

/** What the block says of the notch the slider stands on. */
export interface FramingView {
  /** « Des coins de l'image n'ont pas toutes les nuits : 38 % … ». */
  line: string;
  /** « Tout le champ » … */
  name: string;
  mark: boolean;
  isDefault: boolean;
  /** « 2 154 × 1 994 px · 52 % de l'image entière ». */
  size: string;
  /** « 38 % du cadre à 1 h 48 au lieu de 3 h 35, sans Hα ». */
  exposure: string;
  stops: StopView[];
  index: number;
  /** The line under the slider: the mark (green), beyond it (amber), the smallest (grey). */
  note: { tone: "ok" | "warn" | "muted"; text: string };
  /** The mark is drawn dotted on the picture: the choice is beyond it. */
  showMark: boolean;
  /** The foot: the whole image is always kept, the choice is kept for the plan. */
  foot: string;
}

function whole(value: number, locale: string): string {
  return new Intl.NumberFormat(locale).format(value);
}

/** A share (0 to 1) as it is written: whole percent, at least 1 for what is
 * not nil, at most 99 for what is not all. */
export function percentOf(share: number, all = false): number {
  const rounded = Math.round(share * 100);
  if (all) return rounded;
  return Math.min(99, share > 0 ? Math.max(1, rounded) : rounded);
}

const NAME_KEY: Record<FramingNotch["kind"], string> = {
  cropped: "multiNight.framing.nameCropped",
  mark: "multiNight.framing.nameMark",
  between: "multiNight.framing.nameBetween",
  whole: "multiNight.framing.nameWhole",
};

function exposureText(
  t: Translate,
  framing: PlanFraming,
  notch: FramingNotch,
): string {
  const total = integrationTime(t, framing.fullSeconds);
  const dual = integrationTime(t, notch.lowestDualBandSeconds);
  if (notch.degradedShare === 0) {
    return framing.hasDualBand && notch.lowestDualBandSeconds > 0
      ? t("multiNight.framing.exposureFull", { total, dual })
      : t("multiNight.framing.exposureFullAny", { total });
  }
  const params = {
    share: percentOf(notch.degradedShare),
    low: integrationTime(t, notch.lowestSeconds),
    total,
    dual,
  };
  if (!framing.hasDualBand)
    return t("multiNight.framing.exposureLowAny", params);
  return notch.lowestDualBandSeconds > 0
    ? t("multiNight.framing.exposureLowDual", params)
    : t("multiNight.framing.exposureLow", params);
}

/** What the block says for the notch chosen, from the record's numbers. */
export function framingView(
  t: Translate,
  locale: string,
  framing: PlanFraming,
  choice: FramingChoice,
): FramingView {
  const { notch, index } = choice;
  const notches = framing.notches;
  const dualLine = framing.noDualBandShare > 0;
  const share = percentOf(
    dualLine ? framing.noDualBandShare : framing.degradedShare,
  );
  const size = notch.default
    ? t("multiNight.framing.sizeWhole", {
        width: whole(notch.width, locale),
        height: whole(notch.height, locale),
      })
    : t("multiNight.framing.size", {
        width: whole(notch.width, locale),
        height: whole(notch.height, locale),
        share: percentOf(notch.shareOfFrame),
      });
  const mark = notches.findIndex((item) => item.recommended);
  const beyond = mark >= 0 && index > mark;
  let note: FramingView["note"];
  if (notch.recommended) {
    note = {
      tone: "ok",
      text: t("multiNight.framing.markNote", {
        percent: Math.round(framing.recommendedDegradedShare * 100),
      }),
    };
  } else if (beyond) {
    note = {
      tone: "warn",
      text: t(
        framing.hasDualBand
          ? "multiNight.framing.beyond"
          : "multiNight.framing.beyondAny",
      ),
    };
  } else {
    note = { tone: "muted", text: t("multiNight.framing.smallest") };
  }
  return {
    line: t(
      dualLine ? "multiNight.framing.line" : "multiNight.framing.lineAny",
      { share },
    ),
    name: t(NAME_KEY[notch.kind]),
    mark: notch.recommended,
    isDefault: notch.default,
    size,
    exposure: exposureText(t, framing, notch),
    stops: notches.map((item, position) => ({
      percent: t("multiNight.framing.percent", {
        value: percentOf(item.degradedShare, true),
      }),
      name:
        position === 0
          ? t("multiNight.framing.stopCropped")
          : position === notches.length - 1
            ? t("multiNight.framing.stopWhole")
            : null,
      mark: item.recommended,
      isDefault: item.default,
    })),
    index,
    note,
    showMark: beyond,
    foot: t("multiNight.framing.foot"),
  };
}

/** The share along the track of the notch at ``index`` of ``count`` (0 to 1). */
export function along(index: number, count: number): number {
  return count > 1 ? index / (count - 1) : 0;
}

/** An SVG path (even-odd) of the whole picture without the polygon: the part
 * hatched where the exposure is below the full one. */
export function hatchPath(framing: PlanFraming): string {
  const rectangle = `M0 0H${framing.frameWidth}V${framing.frameHeight}H0Z`;
  const corners = framing.fullPolygon;
  if (corners.length < 3) return "";
  const polygon = corners
    .map(([x, y], position) => `${position === 0 ? "M" : "L"}${x} ${y}`)
    .join("");
  return `${rectangle}${polygon}Z`;
}

/** An SVG path (even-odd) of the whole picture without the frame: what the
 * choice leaves out, dimmed. */
export function dimPath(framing: PlanFraming, notch: PlanFrame): string {
  const x = notch.left;
  const y = notch.top;
  return (
    `M0 0H${framing.frameWidth}V${framing.frameHeight}H0Z` +
    `M${x} ${y}H${x + notch.width}V${y + notch.height}H${x}Z`
  );
}
