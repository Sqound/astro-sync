import { useI18n } from "../i18n";
import {
  COLOUR_SLIDER,
  SOFT_STARS_SLIDER,
  type Colour,
  type SoftStars,
} from "../lib/finish";
import { helpTextClassName } from "../theme";
import { SnapSlider } from "./SnapSlider";

/**
 * The finish of a render under the contrast of presentation: « Intensité des
 * couleurs » (0 to +100 %, detents at +20 % and +40 %) and « Étoiles adoucies »
 * (0 to 100 %, detents at 50 % and 85 %), two continuous sliders of the same
 * shape as the contrast's. Here, the « Finition par défaut » (and the plan
 * image's); a pipeline of a target has its own in its window. « Sans
 * traitement » is never touched.
 */
export function FinishSliders({
  colour,
  softStars,
  onColour,
  onSoftStars,
}: {
  colour: Colour;
  softStars: SoftStars;
  onColour: (next: Colour) => void;
  onSoftStars: (next: SoftStars) => void;
}) {
  const { t } = useI18n();
  return (
    <>
      <SnapSlider
        title={t("wizard.treatments.colour.title")}
        help={t("wizard.finish.colour.help")}
        spec={COLOUR_SLIDER}
        value={colour}
        onChange={onColour}
      />
      <SnapSlider
        title={t("wizard.treatments.softStars.title")}
        help={t("wizard.finish.stars.help")}
        spec={SOFT_STARS_SLIDER}
        value={softStars}
        onChange={onSoftStars}
      />
      <p className={`${helpTextClassName} as-contrast--next`}>
        {t("wizard.treatments.finish.note")}
      </p>
    </>
  );
}
