// « Mesurer un master… »: what the window says and decides about the measure
// of a master the person stacked elsewhere (PixInsight, Siril). Pure
// TypeScript, tested under `node --test`; `screens/night/MasterMeasure.tsx`
// draws it.

import type { TranslateParams } from "../i18n/locale.ts";
import type {
  Capabilities,
  ImageQuality,
  NightDetection,
  ToolRow,
} from "./bridge";
import { imageQualityText } from "./wizardSummary.ts";

type Translate = (key: string, params?: TranslateParams) => string;

/** The refusals the window has a sentence for (`master.refusal.<code>`). */
export const MASTER_REFUSALS = [
  "stretched",
  "too_few_stars",
  "unsupported_format",
  "unreadable",
  "not_an_image",
  "reserved",
  "python_missing",
  "night_missing",
] as const;

/** `available`; `reserved` without `analysis.logs` (greyed, the same sentence
 * as the stacking's: « réservé à Synchronisation avancée+ »); `toolMissing`
 * without the Python environment of Astro Sync. */
export type MasterEntryState = "available" | "reserved" | "toolMissing";

/** The right comes first, then the tool, as for every treatment. */
export function masterEntryState(input: {
  capabilities: Capabilities | null | undefined;
  tools: readonly Pick<ToolRow, "id" | "state">[];
}): MasterEntryState {
  if (input.capabilities?.["analysis.logs"] !== true) return "reserved";
  const python = input.tools.find((tool) => tool.id === "python-env");
  return python?.state === "present" || python?.state === "downloaded"
    ? "available"
    : "toolMissing";
}

export interface MasterTarget {
  targetId: string;
  designation: string;
  folderName: string;
}

/** The targets of the night a master can be given for: those with a folder. */
export function masterTargets(
  detection: NightDetection | null | undefined,
): MasterTarget[] {
  return (detection?.targets ?? [])
    .filter((target) => target.inNight && target.folderName)
    .map((target) => ({
      targetId: target.targetId,
      designation: target.designation,
      folderName: target.folderName as string,
    }));
}

/** The sentence of a refusal: the word's own, `unreadable` for a word the
 * window does not know. */
export function masterRefusalText(t: Translate, code: string): string {
  const known = (MASTER_REFUSALS as readonly string[]).includes(code);
  return t(`master.refusal.${known ? code : "unreadable"}`);
}

/** The figures, said as the « Récapitulatif » says them (« Qualité mesurée sur
 * votre master : FWHM 2,1″ · excentricité 0,31 · 212 étoiles »). */
export function masterQualityText(
  t: Translate,
  locale: string,
  quality: ImageQuality,
  /** `device`: the master of the device, « sur le master de
   * l'appareil » rather than « sur votre master ». */
  source: "master" | "device" = "master",
): string {
  return imageQualityText(t, locale, quality, source);
}

/** What the target already keeps: « Mesure gardée, faite sur … ». */
export function masterCurrentText(
  t: Translate,
  outcome: { master: string },
): string {
  return t("master.current", { file: outcome.master });
}
