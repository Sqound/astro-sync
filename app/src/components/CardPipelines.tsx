import { useI18n } from "../i18n";
import type { PipelinesStatus } from "../lib/pipelinesRead";
import { buttonClassName, helpTextClassName } from "../theme";
import { Waiting } from "./Waiting";

/** The renders of a target's card in « Traitements »: the star and « Recherche
 * des traitements possibles… » while the pipelines are read (the line keeps
 * its height, nothing moves when the names arrive), then the names; nothing
 * when the reading failed or the card has none (the notice above the cards says so). */
export function CardPipelines({
  status,
  names,
}: {
  status: PipelinesStatus;
  /** The renders of the card, by name. */
  names: string;
}) {
  const { t } = useI18n();
  if (status === "loading") {
    return (
      <span className="as-treat__pipelines as-treat__pipelines--waiting">
        <Waiting label={t("import.treat.wait")} size="inline" />
      </span>
    );
  }
  if (status === "failed" || names === "") return null;
  return <span className="as-treat__pipelines">{names}</span>;
}

/** Above the cards, when the pipelines could not be read (« Réessayer ») or
 * when this computer has none: never the same thing as waiting. */
export function PipelinesReadNotice({
  zone,
  onRetry,
}: {
  zone: "waiting" | "failed" | "none" | "listed";
  onRetry: () => void;
}) {
  const { t } = useI18n();
  if (zone === "failed") {
    return (
      <div
        className="as-notice as-notice--error as-notice--inline"
        role="alert"
      >
        <span>{t("import.treat.pipelinesFailed")}</span>
        <button
          type="button"
          className={buttonClassName("link")}
          onClick={onRetry}
        >
          {t("wizard.retry")}
        </button>
      </div>
    );
  }
  if (zone === "none") {
    return (
      <p className={helpTextClassName}>{t("import.treat.pipelinesNone")}</p>
    );
  }
  return null;
}
