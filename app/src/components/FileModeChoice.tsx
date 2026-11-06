import type { OrganiseMode } from "../lib/bridge";
import { FILE_MODE_KEY, ORGANISE_MODES, modeAvailable } from "../lib/organise";
import { useI18n } from "../i18n";
import { helpTextClassName } from "../theme";
import { Icon } from "./Icon";

/**
 * The segmented command of the four modes (« Laisser en place », « Copier »,
 * « Déplacer », « Lien symbolique ») and its sentence, shared by « Ranger les
 * fichiers… » and the detection sheet of the sending of a session.
 * `windowsNoLink`: « Lien symbolique » greyed with its reason.
 * The chosen segment is in the secondary look with a tick; a held one has a
 * dashed edge and a dash (the rule of the greyed commands).
 */
export function FileModeChoice({
  mode,
  onChange,
  windowsNoLink,
}: {
  mode: OrganiseMode;
  onChange: (mode: OrganiseMode) => void;
  windowsNoLink: boolean;
}) {
  const { t } = useI18n();
  return (
    <div className="as-file-mode">
      <div
        className="as-segments as-file-mode__segments"
        role="radiogroup"
        aria-label={t("organise.title")}
      >
        {ORGANISE_MODES.map((item) => {
          const held = !modeAvailable(item, windowsNoLink);
          const chosen = item === mode && !held;
          const className = [
            "as-segment",
            chosen ? "is-chosen" : "",
            held ? "is-held" : "",
          ]
            .filter(Boolean)
            .join(" ");
          return (
            <button
              key={item}
              type="button"
              role="radio"
              aria-checked={chosen}
              aria-disabled={held || undefined}
              disabled={held}
              className={className}
              onClick={() => onChange(item)}
            >
              {chosen && <Icon name="check" className="as-file-mode__icon" />}
              {held && <Icon name="none" className="as-file-mode__icon" />}
              {t(FILE_MODE_KEY[item])}
            </button>
          );
        })}
      </div>
      {windowsNoLink && (
        <p className={`${helpTextClassName} as-file-mode__why`}>
          {t("fileMode.windowsNoLink")}
        </p>
      )}
    </div>
  );
}
