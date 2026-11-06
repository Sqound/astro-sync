// « Images à envoyer »: the words of one row of the list the detection's
// « Récapitulatif » draws, kept apart from the window so that they can be
// tested under `node --test`. Pure TypeScript (no React, no Tauri).

import type { TranslateParams } from "../i18n/locale.ts";
import { CONTRAST_STOPS, hasContrast } from "./contrast.ts";
import {
  formatOf,
  type ImageBlock,
  type SummaryImage,
} from "./detectTreatments.ts";
import { finishSummaries } from "./finish.ts";
import { formatBytes, pluralText } from "./written.ts";

type Translate = (key: string, params?: TranslateParams) => string;

/** What a row says of itself. */
export interface SummaryImageView {
  /** « IC 1805 » for the capture, « Variante de la capture de IC 1805 » for a
   * variant: the strong part of the first line. */
  title: string;
  /** « Rendu naturel », or « Correction bibande · Présentation · 2,0 · Couleurs +40 % ». */
  render: string;
  /** « 3 nuits empilées · TIFF · 46,5 Mo ». */
  meta: string;
  /** Why the box cannot be ticked, or `null`. */
  blocked: string | null;
}

/** « Trop lourde pour votre espace (46,5 Mo > 26,2 Mo) ». */
export function blockText(
  t: Translate,
  locale: string,
  block: ImageBlock,
): string {
  return block.kind === "tooHeavy"
    ? t("detect.send.tooHeavy", {
        size: formatBytes(t, block.size, locale),
        max: formatBytes(t, block.max, locale),
      })
    : t("detect.send.noVariants");
}

/** The words of `row`. `pipelineName` gives the name of a pipeline. */
export function summaryImageView(
  t: Translate,
  locale: string,
  row: SummaryImage,
  pipelineName: (id: string) => string,
): SummaryImageView {
  const stop = CONTRAST_STOPS.find((item) => item.value === row.contrast);
  const curve = !hasContrast(row.contrast)
    ? null
    : stop
      ? t(stop.labelKey)
      : t("wizard.treatments.contrast.summary.custom", {
          value: new Intl.NumberFormat(locale, {
            minimumFractionDigits: 1,
            maximumFractionDigits: 2,
          }).format(row.contrast ?? 0),
        });
  return {
    title:
      row.role === "variant"
        ? t("wizard.sending.variant", { target: row.designation })
        : row.designation,
    render: [pipelineName(row.pipelineId), curve, ...finishSummaries(row, t)]
      .filter(Boolean)
      .join(t("detect.join")),
    meta: [
      row.role === "capture" && row.nights > 1
        ? pluralText(t, locale, "detect.send.stacked", row.nights)
        : null,
      formatOf(row.path),
      formatBytes(t, row.size, locale),
    ]
      .filter(Boolean)
      .join(t("detect.join")),
    blocked: row.blocked ? blockText(t, locale, row.blocked) : null,
  };
}
