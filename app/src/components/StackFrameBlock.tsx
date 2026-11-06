import { useId } from "react";

import { useI18n } from "../i18n";
import type { TargetFrame } from "../lib/bridge";
import { along } from "../lib/planFraming";
import { FRAME_NOTCHES, frameView, type FrameChoice } from "../lib/stackFrame";
import { badgeClassName, helpTextClassName } from "../theme";
import { Icon } from "./Icon";

/**
 * « Cadre de l'empilement »: in the head of the window « Pipelines pour
 * {cible} », above its pipelines. The field rotation the alignment measured,
 * in words; the slider « Taille du cadre » with five notches (the same
 * classes as the « Cadrage de l'image » of a multi-night plan,
 * `PlanFramingBlock`), the share of the whole image under each, the share of
 * the frame every pose covers and what the choice loses or gains; the
 * outline of the frame kept on a drawing of the field. Before the first
 * stacking it is greyed, « Automatique » alone, « Mesuré à l'alignement »;
 * a field that did not turn has nothing to choose. A native range input: the
 * arrow keys move one notch. The numbers are the engine's; nothing is
 * recomputed here. A setting: no button, no level.
 */
export function StackFrameBlock({
  measured,
  choice,
  seconds,
  onChoose,
}: {
  measured: TargetFrame | null;
  choice: FrameChoice | undefined;
  /** The exposure of the target's poses, for « 20 min au lieu de 2 h ». */
  seconds: number | null;
  onChoose: (rank: number) => void;
}) {
  const { t, locale } = useI18n();
  const id = useId();
  const view = frameView(t, locale, measured, choice, seconds);
  const measure = measured?.measure ?? null;
  const width = measure?.frameWidth || 2;
  const height = measure?.frameHeight || 3;
  const notches = view.enabled && measure ? measure.notches : [];
  const kept = notches[view.rank] ?? null;
  const common = notches[0] ?? null;
  const hatchSize = Math.max(8, Math.round(width / 36));
  const count = view.enabled ? notches.length : FRAME_NOTCHES;
  return (
    <section
      className={
        view.enabled
          ? "as-framing as-stack-frame"
          : "as-framing as-stack-frame is-off"
      }
      aria-labelledby={`${id}-title`}
      data-state={view.state}
      data-notch={view.rank}
    >
      <h3 id={`${id}-title`} className="as-framing__title">
        {t("wizard.frame.title")}
      </h3>
      <p className="as-framing__line">{view.line}</p>
      <div className="as-framing__body">
        <figure className="as-framing__picture" aria-hidden="true">
          <div
            className="as-framing__frame"
            style={{ aspectRatio: `${width} / ${height}` }}
          >
            <svg
              className="as-framing__overlay"
              viewBox={`0 0 ${width} ${height}`}
              preserveAspectRatio="none"
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
              </defs>
              {kept && common && view.rank > 0 && (
                <path
                  d={
                    `M${kept.left} ${kept.top}h${kept.width}v${kept.height}h${-kept.width}Z` +
                    `M${common.left} ${common.top}h${common.width}v${common.height}h${-common.width}Z`
                  }
                  fillRule="evenodd"
                  fill={`url(#${id}-hatch)`}
                />
              )}
              {/* Nothing to choose (or not measured yet): the whole field is kept. */}
              <rect
                x={kept?.left ?? 0}
                y={kept?.top ?? 0}
                width={kept?.width ?? width}
                height={kept?.height ?? height}
                className="as-framing__kept"
              />
            </svg>
          </div>
          <figcaption className="as-framing__legend">
            <span>
              <i className="as-framing__key as-framing__key--kept" />
              {t("wizard.frame.legendKept")}
            </span>
            {view.enabled && (
              <span>
                <i className="as-framing__key as-framing__key--low" />
                {t("wizard.frame.legendLow")}
              </span>
            )}
          </figcaption>
        </figure>
        <div className="as-framing__side">
          <p className="as-framing__name">
            <strong>{view.name}</strong>
            {view.automatic && (
              <span className={badgeClassName("neutral")}>
                {t("wizard.frame.automatic")}
              </span>
            )}
            {view.pending && (
              <span className={badgeClassName("neutral")}>
                {t("wizard.frame.pending")}
              </span>
            )}
          </p>
          {view.size && <p className="as-framing__size">{view.size}</p>}
          <p className={`${helpTextClassName} as-framing__exposure`}>
            {view.exposure}
          </p>
          {view.after && (
            <p className={`${helpTextClassName} as-framing__exposure`}>
              {view.after}
            </p>
          )}
          <div className="as-framing__slider">
            <div className="as-framing__slider-head">
              <label htmlFor={`${id}-range`} className="as-framing__label">
                {t("wizard.frame.slider")}
              </label>
              <span className="as-framing__scale">
                {t("wizard.frame.sliderScale")}
              </span>
            </div>
            <div className="as-framing__track">
              <span className="as-framing__rail" aria-hidden="true">
                <span
                  className="as-framing__fill"
                  style={{ width: `${along(view.rank, count) * 100}%` }}
                />
                {Array.from({ length: count }, (_, position) => (
                  <i
                    key={position}
                    className={
                      position <= view.rank
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
                value={view.rank}
                disabled={!view.enabled}
                aria-valuetext={[view.name, view.size, view.exposure]
                  .filter(Boolean)
                  .join(", ")}
                onChange={(event) => onChoose(Number(event.target.value))}
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
                    {stop.automatic && (
                      <span className={badgeClassName("neutral")}>
                        {t("wizard.frame.automatic")}
                      </span>
                    )}
                  </span>
                </div>
              ))}
            </div>
          </div>
          {view.note && (
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
          )}
        </div>
      </div>
      <p className="as-framing__foot">{view.foot}</p>
    </section>
  );
}
