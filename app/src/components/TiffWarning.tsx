import { useI18n } from "../i18n";

/**
 * The line that says a TIFF weighs on the person's quota. One text, one style,
 * wherever a TIFF can be chosen: « Images à envoyer » (under the box of a
 * card), the night's « Récapitulatif » (under « Joindre aussi les TIFF ») and
 * « Envoi des rendus » of Réglages.
 */
export function TiffWarning({ as = "p" }: { as?: "p" | "span" }) {
  const { t } = useI18n();
  const Tag = as;
  return (
    <Tag className="as-treat__meta as-send__warning" role="note">
      {t("detect.send.tiffWarning")}
    </Tag>
  );
}
