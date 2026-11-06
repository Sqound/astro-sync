import { useI18n } from "../i18n";
import { CONTRAST_SLIDER, type Contrast } from "../lib/contrast";
import { SnapSlider } from "./SnapSlider";

/**
 * « Contraste du rendu »: a continuous slider from « Sans » to a gain of 5,
 * held on three detents -- « Sans » (the master of the catalogue as it is),
 * « Présentation » (2,0), « Fort » (3,6) --, the exact value under it.
 */
export function ContrastSlider({
  value,
  onChange,
}: {
  value: Contrast;
  onChange: (next: Contrast) => void;
}) {
  const { t } = useI18n();
  return (
    <SnapSlider
      title={t("wizard.treatments.contrast.title")}
      help={t("wizard.finish.contrast.help")}
      spec={CONTRAST_SLIDER}
      value={value}
      onChange={onChange}
    />
  );
}
