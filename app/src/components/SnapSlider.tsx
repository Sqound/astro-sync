import { useId } from "react";

import { useI18n } from "../i18n";
import {
  detentOf,
  formatNumber,
  fractionOf,
  sliderLabel,
  snapTo,
  type SliderSpec,
} from "../lib/snapSlider";
import { helpTextClassName } from "../theme";

/**
 * A continuous slider with magnetic detents at the values the window proposes
 * (« Contraste du rendu », « Intensité des couleurs », « Étoiles adoucies »):
 * the track holds every value of its range, a tick marks each detent and the
 * thumb is held on it when it comes close. Under the track, the name of the
 * detent it sits on, or « Personnalisé » with the exact value. A native range
 * input: the arrow keys, Home and End move it, and a screen reader says the
 * label, not the number.
 */
export function SnapSlider({
  title,
  help,
  spec,
  value,
  onChange,
}: {
  title: string;
  help: string;
  spec: SliderSpec;
  value: number;
  onChange: (next: number) => void;
}) {
  const { t, locale } = useI18n();
  const id = useId();
  const label = sliderLabel(
    spec,
    value,
    t,
    formatNumber(locale, value, spec.minDecimals),
  );
  return (
    <div className="as-contrast as-snap">
      <label htmlFor={id} className="as-contrast__title">
        {title}
      </label>
      <div className="as-snap__track">
        <input
          id={id}
          type="range"
          className="as-contrast__range as-snap__range"
          min={spec.min}
          max={spec.max}
          step={spec.step}
          value={value}
          aria-valuetext={label}
          aria-describedby={`${id}-help`}
          onChange={(event) =>
            onChange(snapTo(spec, Number(event.target.value)))
          }
        />
        <span className="as-snap__ticks" aria-hidden="true">
          {spec.detents.map((detent) => (
            <span
              key={detent.value}
              className={
                detentOf(spec, value)?.value === detent.value
                  ? "as-snap__tick as-snap__tick--on"
                  : "as-snap__tick"
              }
              style={{ left: `${fractionOf(spec, detent.value) * 100}%` }}
            />
          ))}
        </span>
      </div>
      <output className="as-contrast__value" htmlFor={id}>
        {label}
      </output>
      <p id={`${id}-help`} className={helpTextClassName}>
        {help}
      </p>
    </div>
  );
}
