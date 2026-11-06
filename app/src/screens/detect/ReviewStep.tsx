import { useState } from "react";

import { useImport } from "../../components/ImportProvider";
import { InfoLine } from "../../components/InfoLine";
import { Notice } from "../../components/Notice";
import { WizardFoot } from "../../components/WizardFrame";
import { useI18n } from "../../i18n";
import {
  counts,
  createLabel,
  nightsWaitingBand,
  placesWithoutName,
  type ReviewState,
} from "../../lib/detectReview";
import { displayPath, homeOf } from "../../lib/home";
import {
  footRule,
  reviewReadOnly,
  type ImportState,
} from "../../lib/importFlow";
import type { ScreenId } from "../../lib/menu";
import { pluralText } from "../../lib/written";
import { EquipmentCard } from "./review/EquipmentCard";
import { MultiNightCard } from "./review/MultiNightCard";
import { NothingReadable } from "./review/MissingParts";
import { NightsCard } from "./review/NightsCard";
import { PlacesCard } from "./review/PlacesCard";
import { ReviewSummary } from "./review/SummaryFoot";
import { plainPrimaryLabel } from "./importWords";

type Translate = ReturnType<typeof useI18n>["t"];

/** The line under the title: the folder read, what was read, the nights and
 * the targets. */
export function reviewLine(
  t: Translate,
  locale: string,
  state: ImportState,
): string | null {
  const proposal = state.proposal;
  if (!proposal) return null;
  const reading = proposal.reading;
  const folder = state.choices.fits;
  return [
    folder ? displayPath(folder, homeOf(folder)) : null,
    t("detect.review.read", {
      files: fmt(reading.filesRead, locale),
      lights: fmt(reading.lights, locale),
      calibration: fmt(reading.calibrationFrames, locale),
      logs: fmt(reading.logFiles, locale),
    }),
    pluralText(t, locale, "detect.nights", reading.nights),
    pluralText(t, locale, "detect.review.targets", reading.targets),
  ]
    .filter(Boolean)
    .join(t("detect.join"));
}

/**
 * Step 2 of the import, « Nuits et matériel »: places, equipment, nights,
 * multi-night plans, one tick per line, then « Continuer » -- nothing is
 * written (the sessions and the plans are
 * created by the primary of « Récapitulatif »). Until then the step opens
 * again as it was left, its choices still open; once the sessions are
 * created it opens read-only (the choices as sent): nothing is undone or
 * created again. The choices live in the import (`ImportProvider`).
 */
export function ReviewStep({ onOpen }: { onOpen: (screen: ScreenId) => void }) {
  const { t } = useI18n();
  const imp = useImport();
  const { state } = imp;
  const [listOpen, setListOpen] = useState(false);
  const proposal = state.proposal;
  const user = state.user;
  const review = state.review;
  if (!proposal || !user || !review) return null;
  const readOnly = reviewReadOnly(state);
  const context = {
    proposal,
    user,
    state: review,
    change: (update: (current: ReviewState) => ReviewState) =>
      imp.changeReview(update),
  };
  const free =
    !user.account.advancedPlanning && !user.account.severalPlansPerNight;
  const label = createLabel(proposal, review);
  // A place to create with no name: « Continuer » waits, and says why.
  const unnamed = placesWithoutName(proposal, review);
  const rule = footRule(state, {
    canCreate: label.kind !== "none" && unnamed.length === 0,
  });
  const waiting = nightsWaitingBand(proposal, review);
  const previous = rule.previous;
  return (
    <>
      {readOnly ? (
        <Notice tone="neutral">{t("import.review.readOnly")}</Notice>
      ) : (
        <InfoLine
          line={t("detect.review.nothingYet")}
          details={<p>{t("detect.review.details")}</p>}
        />
      )}
      <Notice tone="error">{state.error}</Notice>
      <NothingReadable
        proposal={proposal}
        open={listOpen}
        onToggle={() => setListOpen((current) => !current)}
      />
      {/* Read-only (or while creating): every control of the cards is
          disabled at once. */}
      <fieldset
        className="as-import__review"
        disabled={readOnly || state.creating}
      >
        <PlacesCard {...context} />
        <EquipmentCard {...context} />
        <NightsCard {...context} free={free} onOpen={onOpen} />
        <MultiNightCard {...context} />
      </fieldset>
      <WizardFoot
        message={
          <ReviewSummary
            found={counts(proposal, review)}
            label={label}
            waiting={readOnly ? [] : waiting}
            unnamed={readOnly ? 0 : unnamed.length}
            created={readOnly}
          />
        }
        previous={previous !== null ? () => imp.goTo(previous) : null}
        primary={
          rule.primary && {
            // « Continuer »: what would be created is computed here, nothing
            // is written.
            label: plainPrimaryLabel(t, "continue"),
            onClick:
              rule.primary.kind === "create"
                ? imp.create
                : () => imp.goTo(state.created ? 4 : 3),
            enabled: rule.primary.enabled,
            className: "as-review__create",
            // The reason of the greyed primary: the nights that wait for a
            // band (`ReviewSummary` writes it).
            describedBy:
              !readOnly &&
              (unnamed.length > 0 ||
                (label.kind === "none" && waiting.length > 0))
                ? "as-review-why"
                : undefined,
          }
        }
      />
    </>
  );
}

const fmt = (value: number, locale: string) =>
  new Intl.NumberFormat(locale).format(value);
