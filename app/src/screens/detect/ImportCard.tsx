import { useState } from "react";

import { useImport } from "../../components/ImportProvider";
import { useI18n } from "../../i18n";
import { importCardLine } from "../../lib/importFlow";
import {
  buttonClassName,
  helpTextClassName,
  progressClassName,
} from "../../theme";

/**
 * The card of « Sessions » while an import is alive: « Import en cours · NGC
 * 7000 · Étape 3 · Traitements · 12 % » with its bar, or « Import en attente de
 * vous · Étape 4 · Récapitulatif », and « Reprendre ». Until step 4's primary,
 * the card says nothing is written to the space yet. « Abandonner l'import »
 * closes an import that waits: nothing created is undone; once the treatments
 * may have made provisional folders, it asks first, and removes them (their
 * links and Astro Sync's images, never a file of the person's).
 */
export function ImportCard({ onResume }: { onResume: () => void }) {
  const { t } = useI18n();
  const imp = useImport();
  const presence = imp.presence;
  const [asking, setAsking] = useState(false);
  if (!presence.shown || presence.done) return null;
  const line = importCardLine(presence, (key, params) => t(key, params));
  return (
    <section className="as-card as-import-card" aria-live="polite">
      <div className="as-import-card__body">
        <strong>
          {presence.waiting
            ? t("import.card.waiting")
            : t("import.card.running")}
        </strong>
        <span className={helpTextClassName}>{line}</span>
        {!presence.written && (
          <span className={`${helpTextClassName} as-import-card__nothing`}>
            {t("import.card.nothingWritten")}
          </span>
        )}
        {presence.percent !== null && (
          <div
            className={progressClassName.track}
            role="progressbar"
            aria-valuemin={0}
            aria-valuemax={100}
            aria-valuenow={presence.percent}
          >
            <div
              className={progressClassName.fill}
              style={{ width: `${presence.percent}%` }}
            />
          </div>
        )}
      </div>
      <div className="as-import-card__actions">
        {presence.waiting && (
          <button
            type="button"
            className={`${buttonClassName("link")} as-import-card__abandon`}
            onClick={() =>
              presence.confirmAbandon ? setAsking(true) : imp.abandon()
            }
          >
            {t("import.card.abandon")}
          </button>
        )}
        <button
          type="button"
          className={`${buttonClassName("tertiary")} as-import-card__resume`}
          onClick={onResume}
        >
          {t("import.card.resume")}
        </button>
      </div>
      {asking && (
        <div className="as-dialog__scrim" role="presentation">
          <div
            className="as-dialog"
            role="dialog"
            aria-modal="true"
            aria-labelledby="import-abandon-title"
          >
            <h2 id="import-abandon-title" className="as-dialog__title">
              {t("import.abandon.title")}
            </h2>
            <p className={helpTextClassName}>{t("import.abandon.body")}</p>
            <p className={helpTextClassName}>
              {presence.written
                ? t("import.abandon.created")
                : t("import.abandon.nothingCreated")}
            </p>
            <div className="as-actions">
              <button
                type="button"
                className={buttonClassName("link")}
                onClick={() => setAsking(false)}
              >
                {t("common.cancel")}
              </button>
              <button
                type="button"
                className={buttonClassName("primary", { destructive: true })}
                onClick={() => {
                  setAsking(false);
                  imp.abandon();
                }}
              >
                {t("import.card.abandon")}
              </button>
            </div>
          </div>
        </div>
      )}
    </section>
  );
}
