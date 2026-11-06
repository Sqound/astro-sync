import { Icon } from "../../components/Icon";
import { InfoLine } from "../../components/InfoLine";
import { useI18n } from "../../i18n";
import {
  chooseFolder,
  localSettingsGet,
  localSettingsSetLastFolders,
} from "../../lib/bridge";
import { displayPath, homeOf } from "../../lib/home";
import type { SendLocations } from "../../lib/wizardDetect";
import { buttonClassName } from "../../theme";

type Slot = "fits" | "logs";

/**
 * The two boxes of « Dossier »: « Emplacement des FITS
 * de la session » and « Journaux de la session ». Empty is today's
 * behaviour (the night's folder is read); « Choisir un dossier… » sets
 * one, « Retirer » takes it back. Nothing is read here: the detection
 * runs when « Fichiers » opens.
 */
export function SessionLocations({
  locations,
  onChange,
}: {
  locations: SendLocations;
  onChange: (next: SendLocations) => void;
}) {
  return (
    <div className="as-send-locations">
      {(["fits", "logs"] as const).map((slot) => (
        <LocationBox
          key={slot}
          slot={slot}
          folder={locations[slot]}
          subfolders={locations.subfolders}
          onFolder={(folder) => onChange({ ...locations, [slot]: folder })}
          onSubfolders={(subfolders) => onChange({ ...locations, subfolders })}
        />
      ))}
    </div>
  );
}

function LocationBox({
  slot,
  folder,
  subfolders,
  onFolder,
  onSubfolders,
}: {
  slot: Slot;
  folder: string | null;
  subfolders: boolean;
  onFolder: (folder: string | null) => void;
  onSubfolders: (subfolders: boolean) => void;
}) {
  const { t } = useI18n();
  // The picker opens on this box's folder, else the last one chosen for a
  // session, kept for the next session.
  async function choose() {
    const settings = await localSettingsGet().catch(() => null);
    const last = settings?.lastFolders;
    const kept = slot === "fits" ? last?.sessionFits : last?.sessionLogs;
    const chosen = await chooseFolder(folder ?? kept ?? null).catch(() => null);
    if (!chosen) return;
    onFolder(chosen);
    void localSettingsSetLastFolders({
      ...last,
      [slot === "fits" ? "sessionFits" : "sessionLogs"]: chosen,
    }).catch(() => undefined);
  }
  return (
    <section
      className="as-send-location"
      data-slot={slot}
      aria-labelledby={`send-location-${slot}`}
    >
      <h3 id={`send-location-${slot}`} className="as-send-location__title">
        {t(`wizard.folder.${slot}.title`)}
      </h3>
      <p className="as-send-location__lead">
        {t(`wizard.folder.${slot}.lead`)}
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
            {folder
              ? displayPath(folder, homeOf(folder))
              : t(`wizard.folder.${slot}.none`)}
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
      <label className="as-detect-subfolders">
        <input
          type="checkbox"
          checked={subfolders}
          onChange={(event) => onSubfolders(event.target.checked)}
        />
        <span>{t("wizard.folder.fits.subfolders")}</span>
      </label>
      <InfoLine
        line={t("wizard.folder.readOnly")}
        details={<p>{t("detect.readOnlyDetails")}</p>}
      />
    </section>
  );
}
