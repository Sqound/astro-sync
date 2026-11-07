import { useI18n } from "../i18n";
import { joinTiffLabel } from "../lib/renderFormat";
import { helpTextClassName } from "../theme";

/**
 * « Joindre aussi les TIFF (pleine résolution, ≈ 47 Mo chacun) »: the one box
 * of a sending that adds the TIFF of its renders to their PNG -- unticked
 * unless « Envoi des rendus » of Réglages says otherwise --, and why it is a
 * choice: the gallery shows the PNG, the TIFF only serves the download in
 * full resolution. The same box on the night's « Récapitulatif » and on
 * « Images à envoyer » of the import.
 */
export function JoinTiffBox({
  checked,
  disabled,
  fullResolution,
  onChange,
}: {
  checked: boolean;
  disabled?: boolean;
  /** The TIFF the sending could join, when they are known (their weight). */
  fullResolution: { count: number; bytes: number } | null;
  onChange: (checked: boolean) => void;
}) {
  const { t, locale } = useI18n();
  return (
    <div className="as-join-tiff">
      <label className="as-tidy__option">
        <input
          type="checkbox"
          checked={checked}
          disabled={disabled}
          onChange={(event) => onChange(event.target.checked)}
        />
        <strong>{joinTiffLabel(t, locale, fullResolution)}</strong>
      </label>
      <p className={`${helpTextClassName} as-join-tiff__help`}>
        {t("uploadFormat.help")}
      </p>
    </div>
  );
}
