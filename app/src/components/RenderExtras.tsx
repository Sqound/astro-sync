import { createContext, useContext } from "react";

import { DEFAULT_CONTRAST, type Contrast } from "../lib/contrast";
import {
  DEFAULT_COLOUR,
  DEFAULT_SOFT_STARS,
  type Colour,
  type SoftStars,
} from "../lib/finish";
import { ContrastSlider } from "./ContrastSlider";
import { FinishSliders } from "./FinishSliders";
import { PipelinesNote } from "./PipelinesNote";

/** The contrast of presentation and the finish (colour intensity, softened
 * stars) shared by every render of « Traitements » (the night's and the
 * plan's): three settings, kept in the local settings. */
export const ContrastContext = createContext<{
  contrast: Contrast;
  onContrast: (next: Contrast) => void;
  colour: Colour;
  onColour: (next: Colour) => void;
  softStars: SoftStars;
  onSoftStars: (next: SoftStars) => void;
}>({
  contrast: DEFAULT_CONTRAST,
  onContrast: () => undefined,
  colour: DEFAULT_COLOUR,
  onColour: () => undefined,
  softStars: DEFAULT_SOFT_STARS,
  onSoftStars: () => undefined,
});

/**
 * What stands under the renders of « Image du plan multi-nuits »: the sliders
 * of the contrast of presentation and of the finish -- unless the box
 * « Empilement » already shows it (one slider on the page) --,
 * then the note that says the pipelines are a base and the person can write
 * their own, with its two links -- the format's documentation and the
 * folder of their definitions.
 */
export function RenderExtras({ slider = true }: { slider?: boolean }) {
  const { contrast, onContrast, colour, onColour, softStars, onSoftStars } =
    useContext(ContrastContext);
  return (
    <>
      {slider && <ContrastSlider value={contrast} onChange={onContrast} />}
      {slider && (
        <FinishSliders
          colour={colour}
          softStars={softStars}
          onColour={onColour}
          onSoftStars={onSoftStars}
        />
      )}
      <PipelinesNote />
    </>
  );
}
