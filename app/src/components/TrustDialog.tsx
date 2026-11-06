import { useI18n } from "../i18n";
import { shortDigest } from "../lib/settingsTab";
import { buttonClassName, helpTextClassName } from "../theme";

/**
 * « Lui faites-vous confiance ? »: asked at the first use of a pipeline the
 * person added with its own script, and again when that script changed. The
 * dialog of « Changer de dossier » is its model. One primary, « Lui faire
 * confiance » (it names its object); the refusal, recorded too, a tertiary at
 * its left; « Plus tard » closes without an answer, a link. Three levels.
 */
export function TrustDialog({
  name,
  sha256,
  changed,
  onAnswer,
  onClose,
}: {
  name: string;
  sha256: string | null;
  changed: boolean;
  onAnswer: (trusted: boolean) => void;
  onClose: () => void;
}) {
  const { t } = useI18n();
  return (
    <div className="as-dialog__scrim" role="presentation">
      <div
        className="as-dialog"
        role="dialog"
        aria-modal="true"
        aria-labelledby="pipeline-trust-title"
      >
        <h2 id="pipeline-trust-title" className="as-dialog__title">
          {t("trust.title", { name })}
        </h2>
        <p>{t("trust.question")}</p>
        {changed && <p className={helpTextClassName}>{t("trust.changed")}</p>}
        <p className={helpTextClassName}>
          {t("trust.details", { digest: shortDigest(sha256) })}
        </p>
        <div className="as-actions">
          <button
            type="button"
            className={buttonClassName("link")}
            onClick={onClose}
          >
            {t("trust.later")}
          </button>
          <button
            type="button"
            className={buttonClassName("tertiary")}
            onClick={() => onAnswer(false)}
          >
            {t("trust.refuse")}
          </button>
          <button
            type="button"
            className={buttonClassName("primary")}
            onClick={() => onAnswer(true)}
          >
            {t("trust.accept")}
          </button>
        </div>
      </div>
    </div>
  );
}
