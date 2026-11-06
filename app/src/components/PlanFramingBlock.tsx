import { useId } from "react";

import { useI18n } from "../i18n";
import type { FramingNotch, PlanFraming } from "../lib/bridge";
import {
  along,
  dimPath,
  framingView,
  hatchPath,
  type FramingChoice,
} from "../lib/planFraming";
import { badgeClassName, helpTextClassName } from "../theme";
import { Icon } from "./Icon";

/**
 * « Cadrage de l'image »: in the card « Image du plan
 * multi-nuits », when corners of the image do not have every night. A slider of
 * notches -- the cropped image (what every integration covers), the mark (the
 * largest frame with at most 5 % of its pixels at reduced exposure), the stops
 * between, the whole image (the default, the last notch) --, the outline of the
 * frame on the picture of the plan, and for the notch chosen its size, its
 * share of the whole image and its exposure. A native range input: the arrow
 * keys move one notch, and a screen reader says the name of the notch. The
 * numbers are the engine's, measured when the image was made; nothing is
 * recomputed here. The slider is a setting: no button, no level.
 */
export function PlanFramingBlock({
  framing,
  choice,
  onChoose,
}: {
  framing: PlanFraming;
  choice: FramingChoice;
  onChoose: (notch: FramingNotch) => void;
}) {
  const { t, locale } = useI18n();
  const id = useId();
  const view = framingView(t, locale, framing, choice);
  const count = framing.notches.length;
  const mark = framing.notches.find((notch) => notch.recommended) ?? null;
  const hatch = hatchPath(framing);
  const hatchSize = Math.max(8, Math.round(framing.frameWidth / 36));
  const kept = choice.notch;
  return (
    <section
      className="as-framing"
      aria-labelledby={`${id}-title`}
      data-notch={choice.index}
    >
      <h3 id={`${id}-title`} className="as-framing__title">
        {t("multiNight.framing.title")}
      </h3>
      <p className="as-framing__line">{view.line}</p>
      <div className="as-framing__body">
        <figure className="as-framing__picture">
          <div
            className="as-framing__frame"
            style={{
              aspectRatio: `${framing.frameWidth} / ${framing.frameHeight}`,
            }}
          >
            {framing.preview && (
              <img
                src={framing.preview}
                alt={t("multiNight.framing.previewAlt")}
                className="as-framing__image"
              />
            )}
            <svg
              className="as-framing__overlay"
              viewBox={`0 0 ${framing.frameWidth} ${framing.frameHeight}`}
              preserveAspectRatio="none"
              aria-hidden="true"
            >
              <defs>
                <pattern
                  id={`${id}-hatch`}
                  width={hatchSize}
                  height={hatchSize}
                  patternUnits="userSpaceOnUse"
                  patternTransform="rotate(45)"
                >
                  <rect
                    width={hatchSize / 3}
                    height={hatchSize}
                    className="as-framing__hatch"
                  />
                </pattern>
                <clipPath id={`${id}-clip`}>
                  <rect
                    x={kept.left}
                    y={kept.top}
                    width={kept.width}
                    height={kept.height}
                  />
                </clipPath>
              </defs>
              {hatch && (
                <path
                  d={hatch}
                  fillRule="evenodd"
                  fill={`url(#${id}-hatch)`}
                  clipPath={`url(#${id}-clip)`}
                />
              )}
              <path
                d={dimPath(framing, kept)}
                fillRule="evenodd"
                className="as-framing__dim"
              />
              {view.showMark && mark && (
                <rect
                  x={mark.left}
                  y={mark.top}
                  width={mark.width}
                  height={mark.height}
                  className="as-framing__mark"
                />
              )}
              <rect
                x={kept.left}
                y={kept.top}
                width={kept.width}
                height={kept.height}
                className="as-framing__kept"
              />
            </svg>
          </div>
          <figcaption className="as-framing__legend">
            <span>
              <i className="as-framing__key as-framing__key--kept" />
              {t("multiNight.framing.legendKept")}
            </span>
            <span>
              <i className="as-framing__key as-framing__key--low" />
              {t("multiNight.framing.legendLow")}
            </span>
            {view.showMark && (
              <span>
                <i className="as-framing__key as-framing__key--mark" />
                {t("multiNight.framing.legendMark")}
              </span>
            )}
          </figcaption>
        </figure>
        <div className="as-framing__side">
          <p className="as-framing__name">
            <strong>{view.name}</strong>
            {view.mark && (
              <span className={badgeClassName("neutral")}>
                {t("multiNight.framing.stopMark")}
              </span>
            )}
            {view.isDefault && (
              <span className={badgeClassName("neutral")}>
                {t("multiNight.framing.default")}
              </span>
            )}
          </p>
          <p className="as-framing__size">{view.size}</p>
          <p className={`${helpTextClassName} as-framing__exposure`}>
            {view.exposure}
          </p>
          <div className="as-framing__slider">
            <div className="as-framing__slider-head">
              <label htmlFor={`${id}-range`} className="as-framing__label">
                {t("multiNight.framing.slider")}
              </label>
              <span className="as-framing__scale">
                {t("multiNight.framing.sliderScale")}
              </span>
            </div>
            <div className="as-framing__track">
              <span className="as-framing__rail" aria-hidden="true">
                <span
                  className="as-framing__fill"
                  style={{ width: `${along(choice.index, count) * 100}%` }}
                />
                {framing.notches.map((notch, position) => (
                  <i
                    key={`${notch.kind}-${position}`}
                    className={
                      position <= choice.index
                        ? "as-framing__dot is-reached"
                        : "as-framing__dot"
                    }
                    style={{ ["--along" as string]: along(position, count) }}
                  />
                ))}
              </span>
              <input
                id={`${id}-range`}
                type="range"
                className="as-framing__range"
                min={0}
                max={count - 1}
                step={1}
                value={choice.index}
                aria-valuetext={`${view.name}, ${view.size}, ${view.exposure}`}
                onChange={(event) =>
                  onChoose(framing.notches[Number(event.target.value)])
                }
              />
            </div>
            <div className="as-framing__stops" aria-hidden="true">
              {view.stops.map((stop, position) => (
                <div
                  key={position}
                  className={
                    position === 0
                      ? "as-framing__stop is-first"
                      : position === count - 1
                        ? "as-framing__stop is-last"
                        : "as-framing__stop"
                  }
                  style={{ ["--along" as string]: along(position, count) }}
                >
                  <span className="as-framing__percent">{stop.percent}</span>
                  <span className="as-framing__stopname">
                    {stop.name && <span>{stop.name}</span>}
                    {stop.mark && (
                      <span className={badgeClassName("neutral")}>
                        {t("multiNight.framing.stopMark")}
                      </span>
                    )}
                    {stop.isDefault && (
                      <span className={badgeClassName("neutral")}>
                        {t("multiNight.framing.default")}
                      </span>
                    )}
                  </span>
                </div>
              ))}
            </div>
          </div>
          <p
            className={`as-framing__note as-framing__note--${view.note.tone}`}
            role="status"
          >
            <Icon
              name={
                view.note.tone === "ok"
                  ? "check"
                  : view.note.tone === "warn"
                    ? "warning"
                    : "info"
              }
            />
            {view.note.text}
          </p>
        </div>
      </div>
      <p className="as-framing__foot">{view.foot}</p>
    </section>
  );
}
