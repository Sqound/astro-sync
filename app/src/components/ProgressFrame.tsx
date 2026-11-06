import { useEffect, useRef } from "react";

import { Icon } from "./Icon";
import { BrandLoader } from "./BrandLoader";
import type { ProgressFrameView } from "../lib/treatmentProgress";

/**
 * The frame of a treatment that runs on this computer: the step in words, the
 * loading star on the left of the bar (the proof the application is alive
 * when the bar cannot say where it stands), the bar -- determined when Siril
 * gives a percentage, a sliding segment otherwise --, the counter, the time
 * elapsed and the last useful line of the journal. What it says comes from
 * `progressFrameView`; this only draws it. With « Réduire les animations »
 * the star is still and the segment becomes a hatched bar (the CSS); the time
 * keeps counting.
 *
 * A run that knows its units draws the share of the whole run in the bar,
 * which never goes back, a thin bar under it for the step in progress, a live
 * line (« Nuit du 9 août · alignement · 3 / 7 · 12 s »), the list of the
 * steps done and, collapsed, the log of the engine.
 */
export function ProgressFrame({
  view,
  location,
  lastLineLabel,
  stop,
}: {
  view: ProgressFrameView;
  /** « sur cet ordinateur ». */
  location: string;
  lastLineLabel: string;
  /** « Arrêter l'empilement »: a link at the foot of the frame, for the
   * treatment that can be stopped on its own. */
  stop?: { label: string; onStop: () => void };
}) {
  return (
    <section className="as-frame" aria-label={view.ariaLabel}>
      <div className="as-sending__row">
        <Icon name="upload" className="as-line__glyph" />
        <span className="as-sending__label">
          {view.label && <strong>{view.label}</strong>}
          {view.headRest}
        </span>
        <span className="as-sending__state">{location}</span>
      </div>
      <div className="as-frame__bar-row">
        <BrandLoader className="as-frame__star" size={22} />
        <div
          className="as-frame__bar"
          role="progressbar"
          aria-label={view.ariaLabel}
          aria-valuemin={0}
          aria-valuemax={100}
          aria-valuenow={view.percent ?? undefined}
        >
          {view.percent === null ? (
            <div className="as-frame__fill as-frame__fill--sliding" />
          ) : (
            <div
              className="as-frame__fill"
              style={{ width: `${view.percent}%` }}
            />
          )}
        </div>
        {view.percentText && (
          <span className="as-frame__percent">{view.percentText}</span>
        )}
      </div>
      {view.stepPercent !== null && (
        <div
          className="as-frame__step-bar"
          role="progressbar"
          aria-label={view.stepBarLabel}
          aria-valuemin={0}
          aria-valuemax={100}
          aria-valuenow={view.stepPercent}
        >
          <div
            className="as-frame__step-fill"
            style={{ width: `${view.stepPercent}%` }}
          />
        </div>
      )}
      <div className="as-frame__facts">
        <span className={view.live ? "as-frame__live" : undefined}>
          {view.live ?? view.counter}
        </span>
        <span>{view.elapsed}</span>
      </div>
      {view.done.length > 0 && (
        <ul className="as-frame__done" aria-label={view.doneLabel}>
          {view.done.map((text, index) => (
            <li key={index}>
              <Icon name="check" className="as-frame__done-glyph" />
              <span>{text}</span>
            </li>
          ))}
        </ul>
      )}
      {view.notes.map((note) => (
        <p key={note} className="as-frame__note">
          {note}
        </p>
      ))}
      {view.lastLine && (
        <div className="as-frame__line" aria-label={lastLineLabel}>
          <code>{view.lastLine}</code>
        </div>
      )}
      {view.silent && <p className="as-frame__silent">{view.silent}</p>}
      {view.live !== null && (
        <FrameDetails
          label={view.detailsLabel}
          empty={view.emptyLog}
          rows={view.log}
        />
      )}
      {stop && (
        <div className="as-frame__foot">
          <button
            type="button"
            className="as-button as-button--link is-destructive"
            onClick={stop.onStop}
          >
            {stop.label}
          </button>
        </div>
      )}
    </section>
  );
}

/** « Détails »: the lines the engine printed in this run, collapsed. The
 * list stays at its last line while it grows. */
function FrameDetails({
  label,
  empty,
  rows,
}: {
  label: string;
  empty: string;
  rows: ProgressFrameView["log"];
}) {
  const list = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const element = list.current;
    if (element) element.scrollTop = element.scrollHeight;
  }, [rows.length]);
  return (
    <details className="as-frame__details">
      <summary>{label}</summary>
      <div ref={list} className="as-frame__log" tabIndex={0}>
        {rows.length === 0 ? (
          <p className="as-frame__log-empty">{empty}</p>
        ) : (
          rows.map((row, index) =>
            row.kind === "heading" ? (
              <p key={index} className="as-frame__log-heading">
                {row.text}
              </p>
            ) : (
              <code key={index} className="as-frame__log-line">
                {row.text}
              </code>
            ),
          )
        )}
      </div>
    </details>
  );
}
