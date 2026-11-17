import { useI18n } from "../i18n";
import type { RendersUpload } from "../lib/bridge";
import {
  fieldClassName,
  fieldGroupClassName,
  helpTextClassName,
} from "../theme";
import { TiffWarning } from "./TiffWarning";

/**
 * « Envoi des rendus » of Réglages: the starting value of the box « Joindre
 * aussi les TIFF » of the night's « Récapitulatif ». « PNG seul » is the
 * default; « PNG + TIFF » carries the quota warning under it.
 */
export function RendersUploadField({
  value,
  disabled,
  onChange,
}: {
  value: RendersUpload;
  disabled?: boolean;
  onChange: (renders: RendersUpload) => void;
}) {
  const { t } = useI18n();
  return (
    <div className={fieldGroupClassName}>
      <select
        className={fieldClassName}
        aria-label={t("settings.rendersUpload.title")}
        value={value}
        disabled={disabled}
        onChange={(event) => onChange(event.target.value as RendersUpload)}
      >
        <option value="png">{t("settings.rendersUpload.png")}</option>
        <option value="both">{t("settings.rendersUpload.both")}</option>
      </select>
      <p className={helpTextClassName}>{t("settings.rendersUpload.line")}</p>
      {value === "both" && <TiffWarning />}
    </div>
  );
}
