import { useI18n } from "../i18n";
import { openInBrowser, pipelinesFolderOpen } from "../lib/bridge";
import { pipelinesNote } from "../lib/pipelinesNote";
import { buttonClassName, noticeClassName } from "../theme";
import { Icon } from "./Icon";

/**
 * The note that says the pipelines are a base and the person can write their
 * own, with its two links -- the format's documentation (in the browser) and
 * the folder of their definitions. One place for every screen that lists
 * pipelines: the step « Traitements », its window « Pipelines pour {cible} »
 * and Réglages › Dépendances.
 */
export function PipelinesNote() {
  const { t } = useI18n();
  const note = pipelinesNote({
    openUrl: openInBrowser,
    openFolder: pipelinesFolderOpen,
  });
  return (
    <div className={`${noticeClassName("neutral")} as-pipelines-note`}>
      <p>{t(note.textKey)}</p>
      <p className="as-pipelines-note__links">
        {note.links.map((link) => (
          <button
            key={link.labelKey}
            type="button"
            className={buttonClassName("link")}
            onClick={() => void link.open()}
          >
            <Icon
              name={link.kind === "web" ? "external" : "folder"}
              className="as-button__icon"
            />
            {t(link.labelKey)}
          </button>
        ))}
      </p>
    </div>
  );
}
