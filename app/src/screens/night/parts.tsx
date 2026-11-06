import { Fragment } from "react";

import { Icon, type IconName } from "../../components/Icon";
import { Waiting } from "../../components/Waiting";
import { useI18n } from "../../i18n";
import type { FileLine, Glyph, Seg } from "../../lib/wizardFiles";
import { WizardFoot } from "../../components/WizardFrame";
import { buttonClassName, helpTextClassName } from "../../theme";

/** A line made of pieces: plain, bold, a path in the code face. */
export function Segs({ segs }: { segs: readonly Seg[] }) {
  return (
    <>
      {segs.map((seg, index) => (
        <Fragment key={index}>
          {typeof seg === "string" ? (
            seg
          ) : "strong" in seg ? (
            <strong>{seg.strong}</strong>
          ) : (
            <code className="as-code">{seg.code}</code>
          )}
        </Fragment>
      ))}
    </>
  );
}

const GLYPH: Record<Glyph, IconName> = {
  ok: "check",
  missing: "cross",
  warning: "warning",
  none: "pending",
  phone: "phone",
  variant: "variant",
  online: "check",
};

/** The lines of a card, each with its glyph (tick, cross, warning, empty
 * circle: U5), « Nouveau » when added since the sending. */
export function Lines({
  lines,
  onAction,
}: {
  lines: readonly FileLine[];
  /** The link at the end of a line (« Ouvrir le dossier des darks »). */
  onAction?: (action: NonNullable<FileLine["action"]>) => void;
}) {
  const { t } = useI18n();
  return (
    <ul className="as-lines">
      {lines.map((line, index) => (
        <li
          key={index}
          className={`as-line as-line--${line.glyph}${line.framed ? " is-framed" : ""}${line.banner ? " is-banner" : ""}`}
        >
          <Icon name={GLYPH[line.glyph]} className="as-line__glyph" />
          <span className="as-line__text">
            <Segs segs={line.segs} />
            {line.isNew && (
              <span className="as-badge as-badge--new">{t("wizard.new")}</span>
            )}
          </span>
          {line.action && onAction && (
            <button
              type="button"
              className={`${buttonClassName("link")} as-line__action`}
              onClick={() => onAction(line.action!)}
            >
              <Icon name="folder" className="as-button__icon" />
              {t(`wizard.files.${line.action}`)}
            </button>
          )}
        </li>
      ))}
    </ul>
  );
}

/** The foot of a step: « Précédent » (link) and the step's one primary --
 * the foot every wizard shares (`WizardFoot`). `line` is the sentence above
 * it that says what the primary will do with what is ticked;
 * `working` replaces it while the primary writes. */
export function StepFoot({
  onBack,
  primary,
  enabled = true,
  icon,
  line = null,
  working = null,
  onPrimary,
}: {
  onBack?: () => void;
  primary: string;
  enabled?: boolean;
  icon?: IconName;
  line?: string | null;
  working?: string | null;
  onPrimary: () => void;
}) {
  const said = working ?? line;
  return (
    <WizardFoot
      message={
        said ? (
          <p className={helpTextClassName} role="status">
            {said}
          </p>
        ) : undefined
      }
      previous={working === null ? onBack : undefined}
      primary={{ label: primary, onClick: onPrimary, enabled, icon }}
    />
  );
}

/** « Précédent » alone, in the foot of a step that has no primary. */
export function StepBack({ onBack }: { onBack: () => void }) {
  return <WizardFoot previous={onBack} />;
}

/** A wait that ended badly: what could not be done, « Réessayer » (the one
 * primary) and « Précédent » (link) when there is a step to go back to -- never
 * a wait without an end. The cause is in the error notice above. */
export function Stalled({
  message,
  onRetry,
  onBack,
}: {
  message: string;
  onRetry: () => void;
  onBack?: () => void;
}) {
  const { t } = useI18n();
  return (
    <div className="as-stalled">
      <p>
        <strong>{message}</strong>
      </p>
      <WizardFoot
        previous={onBack}
        primary={{ label: t("wizard.retry"), onClick: onRetry }}
      />
    </div>
  );
}

/** What a step shows while the night's page is not there: the star and what
 * is awaited, or, when its reading failed, the way out. */
export function StepWait({
  label,
  failed,
  onRetry,
  onBack,
}: {
  label: string;
  failed: boolean;
  onRetry: () => void;
  onBack?: () => void;
}) {
  const { t } = useI18n();
  if (failed) {
    return (
      <Stalled
        message={t("wizard.view.failed")}
        onRetry={onRetry}
        onBack={onBack}
      />
    );
  }
  return <Waiting label={label} />;
}

/** Writes a text to the clipboard; `false` when the system refuses. */
export async function copyText(text: string): Promise<boolean> {
  try {
    await navigator.clipboard.writeText(text);
    return true;
  } catch {
    return false;
  }
}
