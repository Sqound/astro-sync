import { useEffect, useState } from "react";

import { Icon } from "../../components/Icon";
import { useI18n } from "../../i18n";
import {
  fitsChoose,
  masterMeasureCurrent,
  type CapabilitiesState,
  type MasterOutcome,
  type NightDetection,
} from "../../lib/bridge";
import {
  masterCurrentText,
  masterEntryState,
  masterQualityText,
  masterTargets,
  type MasterTarget,
} from "../../lib/masterMeasure";
import type { NightAction, NightChoices } from "../../lib/nightRecap";
import {
  buttonClassName,
  fieldClassName,
  fieldLabelClassName,
  helpTextClassName,
  inactiveButtonClassName,
} from "../../theme";
import type { NightContext } from "./NightWizard";

/** What the measures of « Traitements » need of the wizard's choices: the
 * rows recorded, and the way to record or undo one. */
export interface MeasureChoices {
  recap: NightChoices;
  onRecap: (action: NightAction) => void;
  /** « Continuer » is writing: nothing is changed meanwhile. */
  working: boolean;
}

/**
 * « Mesurer un master… »: in « Traitements », the entry that gives Astro
 * Sync the linear master of a stacking made elsewhere (PixInsight, Siril) to
 * measure like its own (FWHM, eccentricity, stars). Greyed without
 * `analysis.logs` -- the sentence of the stacking's reserved options -- and
 * without the Python environment (« À installer »).
 *
 * The window **records** the master chosen for a target (and the measure kept
 * that is to be forgotten): a row of the entry per choice, whose case is its
 * undo, and « Continuer » measures them.
 */
export function MasterMeasureEntry({
  context,
  detection,
  capabilitiesState,
  measure,
  onOffers,
  onDependencies,
}: {
  context: NightContext;
  detection: NightDetection | null;
  capabilitiesState: CapabilitiesState | null;
  measure: MeasureChoices;
  onOffers: () => void;
  onDependencies: () => void;
}) {
  const { t } = useI18n();
  const [open, setOpen] = useState(false);
  const targets = masterTargets(detection);
  if (targets.length === 0) return null;
  const state = masterEntryState({
    capabilities: capabilitiesState?.capabilities,
    tools: context.tools,
  });
  const closed = state !== "available";
  const masters = Object.values(measure.recap.measures).filter(
    (pick) => pick.source === "master",
  );
  return (
    <section
      className={
        closed
          ? "as-option as-master-entry is-greyed"
          : "as-option as-master-entry"
      }
    >
      <div className="as-option__head">
        <strong className="as-option__title">{t("master.entry.title")}</strong>
        {state === "reserved" && (
          <span className="as-badge as-badge--neutral">
            <Icon name="lock" />
            {t("wizard.treatments.reservedBadge")}
          </span>
        )}
        {state === "toolMissing" && (
          <span className="as-badge as-badge--warning">
            <Icon name="warning" />
            {t("wizard.treatments.toolMissing")}
          </span>
        )}
      </div>
      <p className="as-option__line">{t("master.entry.line")}</p>
      {state === "reserved" && (
        <div className="as-option-note">
          <Icon name="lock" className="as-line__glyph" />
          <span>{t("wizard.treatments.reserved")}</span>
          <button
            type="button"
            className="as-button as-button--link"
            onClick={onOffers}
          >
            <Icon name="external" className="as-button__icon" />
            {t("wizard.offers")}
          </button>
        </div>
      )}
      {state === "toolMissing" && (
        <div className="as-option-note">
          <span>{t("master.refusal.python_missing")}</span>
          <button
            type="button"
            className="as-button as-button--link"
            onClick={onDependencies}
          >
            {t("wizard.treatments.dependencies")}
          </button>
        </div>
      )}
      {(masters.length > 0 || measure.recap.forget.length > 0) && (
        <div className="as-choices as-master-entry__picks">
          {masters.map((pick) => (
            <div key={pick.targetId} className="as-choice as-target-row">
              <label className="as-target-row__label">
                <input
                  type="checkbox"
                  checked={pick.ticked}
                  disabled={measure.working}
                  onChange={() =>
                    measure.onRecap({
                      type: "tickMeasure",
                      targetId: pick.targetId,
                      on: !pick.ticked,
                    })
                  }
                />
                <span className="as-choice__text">
                  <strong>{pick.designation}</strong>
                  <span className="as-choice__line">
                    {pick.ticked
                      ? t("master.pending.measure", { file: pick.fileName })
                      : t("master.pending.skipped", { file: pick.fileName })}
                  </span>
                </span>
              </label>
            </div>
          ))}
          {measure.recap.forget.map((pick) => (
            <div key={pick.targetId} className="as-choice as-target-row">
              <label className="as-target-row__label">
                <input
                  type="checkbox"
                  checked
                  disabled={measure.working}
                  onChange={() =>
                    measure.onRecap({ type: "tickForget", pick, on: false })
                  }
                />
                <span className="as-choice__text">
                  <strong>{pick.designation}</strong>
                  <span className="as-choice__line">
                    {t("master.pending.forget")}
                  </span>
                </span>
              </label>
            </div>
          ))}
        </div>
      )}
      <div className="as-master-entry__action">
        {closed ? (
          <button type="button" className={inactiveButtonClassName} disabled>
            {t("master.entry.open")}
          </button>
        ) : (
          <button
            type="button"
            className={buttonClassName("secondary")}
            disabled={measure.working}
            onClick={() => setOpen(true)}
          >
            {t("master.entry.open")}
          </button>
        )}
      </div>
      {open && (
        <MasterDialog
          sessionId={context.sessionId}
          targets={targets}
          measure={measure}
          onClose={() => setOpen(false)}
        />
      )}
    </section>
  );
}

function MasterDialog({
  sessionId,
  targets,
  measure,
  onClose,
}: {
  sessionId: string;
  targets: MasterTarget[];
  measure: MeasureChoices;
  onClose: () => void;
}) {
  const { t, locale } = useI18n();
  const [targetId, setTargetId] = useState(targets[0].targetId);
  const target =
    targets.find((item) => item.targetId === targetId) ?? targets[0];
  const [current, setCurrent] = useState<MasterOutcome | null>(null);

  // What the target already keeps, read again for each target.
  useEffect(() => {
    let cancelled = false;
    setCurrent(null);
    masterMeasureCurrent(sessionId, target.folderName)
      .then((kept) => {
        if (!cancelled) setCurrent(kept);
      })
      .catch(() => undefined);
    return () => {
      cancelled = true;
    };
  }, [sessionId, target.folderName]);

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  /** The file picker, then the choice is recorded: « Continuer » measures. */
  async function choose() {
    const file = await fitsChoose().catch(() => null);
    if (!file) return;
    measure.onRecap({
      type: "pickMeasure",
      pick: {
        targetId: target.targetId,
        folderName: target.folderName,
        designation: target.designation,
        source: "master",
        file,
        fileName: file.split(/[\\/]/).pop() ?? file,
      },
    });
    onClose();
  }

  const kept = current?.result === "measured" ? current : null;
  const forgetPick = {
    targetId: target.targetId,
    folderName: target.folderName,
    designation: target.designation,
  };
  const forgetting = measure.recap.forget.some(
    (item) => item.targetId === target.targetId,
  );
  return (
    <div className="as-dialog__scrim" role="presentation">
      <div
        className="as-dialog as-master-dialog"
        role="dialog"
        aria-modal="true"
        aria-labelledby="master-measure-title"
      >
        <h2 id="master-measure-title" className="as-dialog__title">
          {t("master.title")}
        </h2>
        <p className="as-master-dialog__give">{t("master.give")}</p>
        <p className={helpTextClassName}>{t("master.lead")}</p>
        {targets.length > 1 ? (
          <label className="as-dialog__field">
            <span className={fieldLabelClassName}>{t("master.target")}</span>
            <select
              className={fieldClassName}
              value={targetId}
              onChange={(event) => setTargetId(event.target.value)}
            >
              {targets.map((item) => (
                <option key={item.targetId} value={item.targetId}>
                  {item.designation}
                </option>
              ))}
            </select>
          </label>
        ) : (
          <p className={helpTextClassName}>
            {t("master.targetOne", { target: target.designation })}
          </p>
        )}
        {kept && (
          <div className="as-master-dialog__kept">
            <p>{masterCurrentText(t, kept)}</p>
            <p>{masterQualityText(t, locale, kept.quality)}</p>
            <label className="as-master-dialog__forget">
              <input
                type="checkbox"
                checked={forgetting}
                onChange={() =>
                  measure.onRecap({
                    type: "tickForget",
                    pick: forgetPick,
                    on: !forgetting,
                  })
                }
              />
              {t("master.forget")}
            </label>
          </div>
        )}
        <div className="as-actions">
          <button
            type="button"
            className={buttonClassName("link")}
            onClick={onClose}
          >
            {t("common.close")}
          </button>
          <button
            type="button"
            className={buttonClassName("primary")}
            onClick={() => void choose()}
          >
            {t("master.choose")}
          </button>
        </div>
      </div>
    </div>
  );
}
