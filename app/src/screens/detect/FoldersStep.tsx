import { Icon } from "../../components/Icon";
import { useImport } from "../../components/ImportProvider";
import { InfoLine } from "../../components/InfoLine";
import { Notice } from "../../components/Notice";
import { ScanHead } from "../../components/ScanHead";
import { WizardFoot } from "../../components/WizardFrame";
import { useI18n } from "../../i18n";
import { chooseFolder } from "../../lib/bridge";
import {
  DETECT_SLOTS,
  foundLines,
  windowReady,
  withFolder,
  type DetectSlot,
} from "../../lib/detectFlow";
import { displayPath, homeOf } from "../../lib/home";
import { footRule } from "../../lib/importFlow";
import { pluralText } from "../../lib/written";
import { buttonClassName, helpTextClassName } from "../../theme";
import { plainPrimaryLabel, secondaryLabel } from "./importWords";

/**
 * Step 1 of the import, « Dossier » (a page, no longer a window over the list):
 * the three locations, the frames' one required, the sub-folders and the
 * read-only reminder; then, on the same page, the reading with its bar and
 * « Trouvé jusqu'ici ». The window's Échap and click outside became one button
 * of the foot, « Arrêter la détection »; « Annuler » is the link at the head,
 * « Retour aux sessions ».
 */
export function FoldersStep() {
  const { t, locale } = useI18n();
  const imp = useImport();
  const { state } = imp;
  const choices = state.choices;
  const ready = windowReady(choices);
  const rule = footRule(state, { ready });
  if (state.scan) {
    const scan = state.scan;
    const lines = foundLines(
      t,
      (key, count) => pluralText(t, locale, key, count),
      scan.found,
    );
    return (
      <>
        <div className="as-detect-scan" role="status">
          <ScanHead
            read={scan.read}
            total={scan.total}
            walked={scan.walked}
            folder={scan.folder}
          />
          {scan.total > 0 && scan.current && (
            <p className="as-detect-scan__current" title={scan.current}>
              {displayPath(scan.current, homeOf(scan.current))}
            </p>
          )}
          {lines && (
            <ul className="as-detect-scan__found">
              <li>
                <Icon name="check" />
                <span>{lines.found}</span>
              </li>
              {lines.files && (
                <li>
                  <Icon name="check" />
                  <span>{lines.files}</span>
                </li>
              )}
              {lines.solving && (
                <li className="is-later">
                  <Icon name="clock" />
                  <span>{lines.solving}</span>
                </li>
              )}
            </ul>
          )}
        </div>
        <WizardFoot
          secondary={
            rule.secondary && {
              label: secondaryLabel(t, rule.secondary),
              onClick: imp.stopScan,
              className: "as-detect-stop",
            }
          }
        />
      </>
    );
  }
  return (
    <>
      {DETECT_SLOTS.map((slot) => (
        <FolderRow
          key={slot}
          slot={slot}
          folder={choices[slot]}
          onFolder={(folder) =>
            imp.setChoices(withFolder(choices, slot, folder))
          }
        />
      ))}
      <label className="as-detect-subfolders">
        <input
          type="checkbox"
          checked={choices.subfolders}
          onChange={(event) =>
            imp.setChoices({ ...choices, subfolders: event.target.checked })
          }
        />
        <span>{t("detect.subfolders")}</span>
      </label>
      <InfoLine
        line={t("detect.readOnly")}
        details={<p>{t("detect.readOnlyDetails")}</p>}
      />
      <Notice tone="error">{state.error}</Notice>
      <WizardFoot
        message={
          !ready && (
            <p className={`${helpTextClassName} as-detect-foot__why`}>
              <Icon name="lock" />
              {t("detect.needFolder")}
            </p>
          )
        }
        primary={
          rule.primary && {
            label: plainPrimaryLabel(t, "startScan"),
            onClick: imp.startScan,
            enabled: rule.primary.enabled,
            className: "as-detect-start",
          }
        }
      />
    </>
  );
}

function FolderRow({
  slot,
  folder,
  onFolder,
}: {
  slot: DetectSlot;
  folder: string | null;
  onFolder: (folder: string | null) => void;
}) {
  const { t } = useI18n();
  async function choose() {
    const chosen = await chooseFolder(folder).catch(() => null);
    if (chosen) onFolder(chosen);
  }
  return (
    <div className="as-detect-folder" data-slot={slot}>
      <p className="as-detect-folder__label">
        <strong>{t(`detect.folder.${slot}`)}</strong>
        <span>{t(`detect.folder.${slot}Hint`)}</span>
      </p>
      <div className="as-detect-folder__line">
        <span
          className={
            folder
              ? "as-detect-folder__path"
              : "as-detect-folder__path is-empty"
          }
          title={folder ?? undefined}
        >
          <Icon name="folder" />
          <span>
            {folder ? displayPath(folder, homeOf(folder)) : t("detect.none")}
          </span>
        </span>
        {folder && (
          <button
            type="button"
            className={`${buttonClassName("link")} as-detect-folder__remove`}
            onClick={() => onFolder(null)}
          >
            {t("detect.remove")}
          </button>
        )}
        <button
          type="button"
          className={`${buttonClassName("tertiary")} as-detect-folder__choose`}
          onClick={() => void choose()}
        >
          {folder ? t("detect.changeFolder") : t("detect.chooseFolder")}
        </button>
      </div>
    </div>
  );
}
