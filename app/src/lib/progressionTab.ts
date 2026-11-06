// « Progression »: Siril's state in one line, the state of the tab (reserved,
// Siril missing, choosing, generated), the milestones. Pure TypeScript, tested
// under `node --test`; `screens/ProgressionScreen.tsx` draws it.

import type { TranslateParams } from "../i18n/locale.ts";
import type { ToolRow } from "./bridge";
import { progressionSpaceText } from "./wizardSummary.ts";

type Translate = (key: string, params?: TranslateParams) => string;

/** Siril as Réglages › Dépendances found it. */
export type SirilLine =
  | { state: "compatible"; version: string }
  | { state: "unchecked"; version: string }
  | { state: "missing" };

export function sirilLine(tools: readonly ToolRow[] | null): SirilLine | null {
  if (tools === null) return null;
  const siril = tools.find((row) => row.id === "siril");
  if (!siril || siril.state !== "present" || !siril.version) {
    return { state: "missing" };
  }
  return siril.progressionCompatible
    ? { state: "compatible", version: siril.version }
    : { state: "unchecked", version: siril.version };
}

export function sirilText(t: Translate, line: SirilLine): string {
  switch (line.state) {
    case "compatible":
      return t("progression.sirilCompatible", { version: line.version });
    case "unchecked":
      return t("progression.sirilUnchecked", { version: line.version });
    default:
      return t("progression.sirilMissing");
  }
}

/** The four states of the tab: 1 choosing, 2 generated, 3 Siril
 * missing, 4 reserved. */
export type ProgressionState =
  "reserved" | "sirilMissing" | "choose" | "generated";

export function progressionState(input: {
  reserved: boolean;
  siril: SirilLine | null;
  generated: boolean;
}): ProgressionState {
  if (input.reserved) return "reserved";
  if (input.generated) return "generated";
  if (input.siril?.state === "missing") return "sirilMissing";
  return "choose";
}

/** Whether « Générer la progression » is the step's action, or greyed. */
export function canGenerate(input: {
  state: ProgressionState;
  family: string | null;
  running: boolean;
}): boolean {
  return input.state === "choose" && input.family !== null && !input.running;
}

/** One milestone of a progression made (`progression_images::ProgressionImage`). */
export interface Milestone {
  index: number;
  frameCount: number;
  path: string;
}

/** The milestones read from what the core returned, in their order. */
export function milestones(images: unknown): Milestone[] {
  const list = (images as { images?: unknown } | null)?.images;
  if (!Array.isArray(list)) return [];
  return list
    .filter(
      (item): item is Milestone =>
        typeof item === "object" &&
        item !== null &&
        typeof (item as Milestone).index === "number" &&
        typeof (item as Milestone).frameCount === "number",
    )
    .sort((a, b) => a.index - b.index);
}

/** The words of a failure of `progression_images_run` (`details.why`). */
export function failureText(
  t: Translate,
  details: Record<string, unknown> | undefined,
): string {
  const why = typeof details?.why === "string" ? details.why : "failed";
  switch (why) {
    case "not_in_subscription":
      return t("progression.reserved");
    case "siril_missing":
      return t("progression.sirilMissing");
    case "no_frames":
      return t("progression.noFamiliesFound");
    case "cancelled":
      return t("progression.skippedNotice");
    case "not_enough_space":
      return progressionSpaceText(t, details ?? {});
    default: {
      const detail =
        typeof details?.detail === "string"
          ? details.detail
          : typeof details?.file === "string"
            ? details.file
            : why;
      return t("progression.unavailableReason", { reason: detail });
    }
  }
}
