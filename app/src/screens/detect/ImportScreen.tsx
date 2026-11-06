import { useEffect, useRef } from "react";

import { useImport } from "../../components/ImportProvider";
import { WizardFrame } from "../../components/WizardFrame";
import { useI18n } from "../../i18n";
import type { WizardStep } from "../../lib/bridge";
import {
  IMPORT_STEP_LABEL_KEY,
  importThread,
  type ImportState,
} from "../../lib/importFlow";
import type { ScreenId } from "../../lib/menu";
import { ImportDone } from "./ImportDone";
import { ImportSummary, summaryHead } from "./ImportSummary";
import { ImportTreatments, treatmentsLine } from "./ImportTreatments";
import { FoldersStep } from "./FoldersStep";
import { ReviewStep, reviewLine } from "./ReviewStep";

/**
 * « Détecter les sessions »: one wizard of five steps in the frame of the
 * night's wizard -- 1 Dossier (the folders, then the reading), 2 Nuits et
 * matériel (the review, « Continuer »: nothing written), 3 Traitements (on
 * provisional nights), 4 Récapitulatif (what will be created, « Images à
 * envoyer », « Créer et envoyer »), 5 Terminé. The import lives in
 * `ImportProvider`: leaving the page (« Retour aux sessions », the menu)
 * keeps it, the page reopens at the same step. Leaving « Terminé », or a page
 * where nothing was read yet, closes it.
 */
export function ImportScreen({
  onLeave,
  onOpenNight,
  onOpen,
}: {
  /** « Retour aux sessions ». */
  onLeave: () => void;
  onOpenNight: (
    sessionId: string,
    step: WizardStep | null,
    targetId?: string | null,
  ) => void;
  onOpen: (screen: ScreenId) => void;
}) {
  const { t, locale } = useI18n();
  const imp = useImport();
  const { state } = imp;
  const held = useRef(state);
  held.current = state;
  const { open, close } = imp;

  useEffect(() => {
    open();
    // An import that read nothing yet, or that is over, is not kept when the
    // page is left (whatever the way: the link, the menu).
    return () => {
      const now = held.current;
      if (now.sent !== null || (!now.scan && !now.proposal)) close();
    };
  }, [open, close]);

  const head = headOf(t, locale, state);
  const steps = importThread(state).map((item) => ({
    step: String(item.step),
    n: item.step,
    labelKey: IMPORT_STEP_LABEL_KEY[item.step],
    status: item.status,
    go: item.reopens ? () => imp.goTo(item.step) : undefined,
  }));

  return (
    <WizardFrame
      className={`as-import as-import--step-${state.step}`}
      back={{ label: t("import.back"), onClick: onLeave }}
      eyebrow={t("detect.title")}
      title={head.title}
      line={head.line}
      steps={steps}
      stepKey={state.step}
    >
      {state.step === 1 && <FoldersStep />}
      {state.step === 2 && <ReviewStep onOpen={onOpen} />}
      {state.step === 3 && <ImportTreatments />}
      {state.step === 4 && <ImportSummary onOpenNight={onOpenNight} />}
      {state.step === 5 && <ImportDone onLeave={onLeave} />}
    </WizardFrame>
  );
}

type Translate = ReturnType<typeof useI18n>["t"];

/** The title and the line of the page, step by step. */
function headOf(
  t: Translate,
  locale: string,
  state: ImportState,
): { title: string; line: string | null } {
  switch (state.step) {
    case 1:
      return state.scan
        ? { title: t("detect.scan.title"), line: t("detect.lead") }
        : { title: t("import.folders.title"), line: t("detect.lead") };
    case 2:
      return {
        title: t("detect.review.title"),
        line: reviewLine(t, locale, state),
      };
    case 3:
      return {
        title: t("import.treat.title"),
        line: treatmentsLine(t, locale, state),
      };
    case 4:
      return summaryHead(t, locale, state);
    case 5:
      return { title: t("import.done.title"), line: t("import.done.lead") };
  }
}
