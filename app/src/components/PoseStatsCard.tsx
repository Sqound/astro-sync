import { useEffect, useId, useMemo, useState } from "react";

import { useI18n } from "../i18n";
import { sessionsUserData } from "../lib/bridge";
import { utcOffsetMinutes } from "../lib/detectPlaces";
import { formatDuration } from "../lib/duration";
import { formatError } from "../lib/format";
import {
  clockText,
  curvesOf,
  exposureText,
  familySegment,
  familySpan,
  flaggedOf,
  offsetText,
  placeOffsetMinutes,
  reasonCountsText,
  reasonText,
  sampleText,
  shownFamily,
  statsOf,
  type Curve,
  type Curves,
  type FamilyStats,
  type Metric,
  type PoseMeasure,
  type TargetPoses,
} from "../lib/poseStats";
import { pluralText } from "../lib/written";
import { badgeClassName, buttonClassName, helpTextClassName } from "../theme";
import { Icon } from "./Icon";
import { useNow } from "../hooks/useNow";

/** The place of the night, for the hours and the Sun. */
export interface PoseCardPlace {
  name: string | null;
  latitude: number | null;
  longitude: number | null;
}

/**
 * « Mesure des poses », the card of a target in « Fichiers »: before the
 * measure (what it takes, « Mesurer les poses »), as it goes (the count, the
 * curves drawn as they come, « Arrêter la mesure »), stopped (« Reprendre la
 * mesure »), and its result -- one family at a time, four curves in the
 * hours of the place, the poses that depart with their reason, behind an ⓘ
 * how a pose is flagged. Nothing is sent and nothing is moved.
 */
export function PoseStatsCard({
  entry,
  place,
  busyElsewhere,
  onStart,
  onStop,
}: {
  entry: TargetPoses;
  place: PoseCardPlace;
  /** Another target's measure is going: this one waits. */
  busyElsewhere: boolean;
  onStart: (resume: boolean) => void;
  onStop: () => void;
}) {
  const { t, locale } = useI18n();
  const run = entry.run;
  const stats = statsOf(entry);
  const running = run.status === "running";
  const now = useNow(running);
  const [chosen, setChosen] = useState<string | null>(null);
  const [info, setInfo] = useState(false);
  const [all, setAll] = useState(false);
  const zone = usePlaceZone(place.name);
  const offset = useMemo(() => {
    const first = stats?.families
      .flatMap((f) => f.measured)
      .find((m) => m.unix !== null)?.unix;
    return placeOffsetMinutes(
      zone,
      place.longitude,
      first ?? Date.now() / 1000,
      utcOffsetMinutes,
    );
  }, [stats, zone, place.longitude]);

  const family = stats
    ? running && run.status === "running"
      ? (stats.families.find((f) => f.label === run.family) ??
        shownFamily(stats, null))
      : shownFamily(stats, chosen)
    : null;
  const measuredTotal = stats
    ? stats.families.reduce((sum, f) => sum + f.measured.length, 0)
    : 0;
  const shapeless =
    run.status === "done" &&
    stats !== null &&
    stats.families.length > 0 &&
    stats.families.every((f) => f.judged && f.shapeMissing !== null);
  const badge =
    run.status === "idle"
      ? { tone: "neutral" as const, key: "idle" }
      : running
        ? { tone: "neutral" as const, key: "running" }
        : run.status === "stopped" || run.status === "failed"
          ? { tone: "warning" as const, key: "stopped" }
          : shapeless
            ? { tone: "warning" as const, key: "tooFew" }
            : { tone: "success" as const, key: "done" };

  return (
    <section
      className={`as-poses as-poses--${run.status}`}
      aria-label={t("poseStats.title")}
    >
      <header className="as-poses__head">
        <h4 className="as-poses__title">{t("poseStats.title")}</h4>
        <span className={badgeClassName(badge.tone)}>
          {t(`poseStats.badge.${badge.key}`)}
        </span>
        {run.status !== "idle" && !running && (
          <button
            type="button"
            className="as-button as-button--link as-poses__info"
            aria-label={t("common.learnMore")}
            title={t("common.learnMore")}
            onClick={() => setInfo(true)}
          >
            <Icon name="info" className="as-button__icon" />
          </button>
        )}
        {(run.status === "done" || run.status === "failed") && (
          <button
            type="button"
            className={`${buttonClassName("link")} as-poses__again`}
            disabled={busyElsewhere}
            onClick={() => onStart(false)}
          >
            {t("poseStats.again")}
          </button>
        )}
      </header>

      {run.status === "idle" && (
        <>
          <p className={helpTextClassName}>{t("poseStats.intro")}</p>
          {entry.plan && (
            <p className="as-poses__sample">
              {sampleText(t, locale, entry.plan)}
            </p>
          )}
          <div className="as-poses__foot">
            <p className={helpTextClassName}>
              {entry.plan && entry.plan.families.length > 1
                ? t("poseStats.neverMixed", {
                    families: entry.plan.families
                      .map((f) => exposureText(t, locale, f.exposureSeconds))
                      .join(" · "),
                  })
                : ""}
            </p>
            <button
              type="button"
              className={buttonClassName("secondary")}
              disabled={busyElsewhere || !entry.plan}
              onClick={() => onStart(false)}
            >
              {t("poseStats.start")}
            </button>
          </div>
        </>
      )}

      {running && run.status === "running" && (
        <>
          <p className="as-poses__progress">
            <strong>
              {pluralText(t, locale, "poseStats.progress", run.done, {
                done: run.done,
                total: run.planned,
              })}
            </strong>
            {family && (
              <span className={helpTextClassName}>
                {t("poseStats.progressFamily", {
                  exposure: exposureText(t, locale, family.exposureSeconds),
                  done: family.measured.length,
                  planned:
                    entry.plan?.families.find((f) => f.label === family.label)
                      ?.planned ?? family.total,
                })}
              </span>
            )}
            <span className={`${helpTextClassName} as-poses__remaining`}>
              {remainingText(t, run, now)}
            </span>
          </p>
          <div className="as-progress" aria-hidden="true">
            <div
              className="as-progress__fill"
              style={{
                width: `${Math.min(100, (run.done / Math.max(1, run.planned)) * 100)}%`,
              }}
            />
          </div>
          <p className={helpTextClassName}>{t("poseStats.runningNote")}</p>
        </>
      )}

      {(run.status === "stopped" || run.status === "failed") && stats && (
        <p className="as-poses__headline as-poses__headline--warning">
          <Icon name="warning" className="as-line__glyph" />
          <span>
            {pluralText(t, locale, "poseStats.stopped", measuredTotal, {
              done: measuredTotal,
              total: stats.planned,
            })}
            {run.status === "failed" && (
              <>
                {" "}
                {t("poseStats.failed", { reason: formatError(t, run.error) })}
              </>
            )}
          </span>
        </p>
      )}
      {run.status === "failed" && !stats && (
        <p className="as-poses__headline as-poses__headline--warning">
          <Icon name="warning" className="as-line__glyph" />
          <span>
            {t("poseStats.failed", { reason: formatError(t, run.error) })}
          </span>
        </p>
      )}

      {stats &&
        stats.families.length > 1 &&
        family &&
        (run.status === "done" || run.status === "failed") && (
          <div className="as-poses__segments" role="group">
            {stats.families.map((f) => (
              <button
                key={f.label}
                type="button"
                className={
                  f.label === family.label
                    ? "as-segment is-chosen"
                    : "as-segment"
                }
                aria-pressed={f.label === family.label}
                onClick={() => setChosen(f.label)}
              >
                {familySegment(t, locale, f)}
              </button>
            ))}
          </div>
        )}

      {family && stats && run.status !== "idle" && (
        <FamilyResult
          family={family}
          stats={stats}
          status={run.status}
          offset={offset}
          running={running}
          shapeless={shapeless}
          all={all}
          onAll={() => setAll((value) => !value)}
          placeKnown={zone !== null || place.longitude !== null}
          measuredTotal={measuredTotal}
        />
      )}

      {running && (
        <div className="as-poses__foot as-poses__foot--end">
          <button
            type="button"
            className={buttonClassName("link")}
            onClick={onStop}
          >
            {t("poseStats.stop")}
          </button>
        </div>
      )}
      {(run.status === "stopped" || run.status === "failed") && (
        <div className="as-poses__foot">
          <p className={helpTextClassName}>{stopNote(t, stats, offset)}</p>
          <button
            type="button"
            className={buttonClassName("secondary")}
            disabled={busyElsewhere}
            onClick={() => onStart(run.status === "stopped")}
          >
            {run.status === "stopped"
              ? t("poseStats.resume")
              : t("poseStats.again")}
          </button>
        </div>
      )}
      {info && <PoseInfoDialog onClose={() => setInfo(false)} />}
    </section>
  );
}

/** « il reste environ 1 min 30 » from the rate so far. */
function remainingText(
  t: ReturnType<typeof useI18n>["t"],
  run: { startedAt: number; done: number; planned: number },
  now: number,
): string {
  const elapsed = now - run.startedAt;
  if (run.done < 3 || elapsed < 2000) return "";
  const left = (elapsed / run.done) * Math.max(0, run.planned - run.done);
  return t("poseStats.remaining", { duration: formatDuration(t, left) });
}

function stopNote(
  t: ReturnType<typeof useI18n>["t"],
  stats: ReturnType<typeof statsOf>,
  offset: number,
): string {
  if (!stats) return "";
  const flagged = flaggedOf(stats).length;
  const last = Math.max(
    0,
    ...stats.families.flatMap((f) => f.measured.map((m) => m.unix ?? 0)),
  );
  const time = last > 0 ? clockText(last, offset) : "";
  return flagged === 0
    ? t("poseStats.stoppedNone", { time })
    : t("poseStats.stoppedSome", { count: flagged, time });
}

/* ---------------------------------- A family's result -------------------- */

function FamilyResult({
  family,
  stats,
  status,
  offset,
  running,
  shapeless,
  all,
  onAll,
  placeKnown,
  measuredTotal,
}: {
  family: FamilyStats;
  stats: NonNullable<ReturnType<typeof statsOf>>;
  status: TargetPoses["run"]["status"];
  offset: number;
  running: boolean;
  shapeless: boolean;
  all: boolean;
  onAll: () => void;
  placeKnown: boolean;
  measuredTotal: number;
}) {
  const { t, locale } = useI18n();
  const span = useMemo(() => {
    const times = family.measured
      .map((m) => m.unix)
      .filter((u): u is number => u !== null);
    return times.length ? familySpan(family) : undefined;
  }, [family]);
  const curves = useMemo(
    () => curvesOf(family, offset, span),
    [family, offset, span],
  );
  const exposure = exposureText(t, locale, family.exposureSeconds);
  const flagged = family.flagged;
  const finished = status === "done";
  return (
    <>
      {finished && measuredTotal < stats.total && (
        <p className={helpTextClassName}>
          {doneText(t, locale, stats, measuredTotal)}
        </p>
      )}
      {finished && !family.judged && (
        <p className="as-poses__headline as-poses__headline--warning">
          <Icon name="warning" className="as-line__glyph" />
          <span>{t("poseStats.notJudged", { min: 10 })}</span>
        </p>
      )}
      {finished && family.judged && shapeless && (
        <p className="as-poses__headline as-poses__headline--warning">
          <Icon name="warning" className="as-line__glyph" />
          <span>
            {pluralText(
              t,
              locale,
              "poseStats.tooFew",
              family.shapeMissing ?? 0,
              {
                count: family.shapeMissing ?? 0,
                total: family.measured.length,
              },
            )}
          </span>
        </p>
      )}
      {finished && family.familyShape && (
        <p className={helpTextClassName}>
          {t("poseStats.familyWidth", {
            exposure,
            value: new Intl.NumberFormat(locale, {
              maximumFractionDigits: 1,
              minimumFractionDigits: 1,
            }).format(family.familyShape.fwhmPixels),
            stars: family.familyShape.stars,
          })}
        </p>
      )}
      {finished && family.judged && !shapeless && flagged.length > 0 && (
        <p className="as-poses__headline as-poses__headline--warning">
          <Icon name="warning" className="as-line__glyph" />
          <span>
            <strong>
              {pluralText(t, locale, "poseStats.flagged", flagged.length, {
                exposure,
              })}
            </strong>{" "}
            · {reasonCountsText(t, flagged)}
          </span>
        </p>
      )}
      {finished && family.judged && !shapeless && flagged.length === 0 && (
        <p className="as-poses__headline">
          <Icon name="check" className="as-line__glyph" />
          <span>
            <strong>{t("poseStats.none", { exposure })}</strong>
            {" · "}
            {pluralText(
              t,
              locale,
              "poseStats.noneDetail",
              family.measured.length,
            )}
          </span>
        </p>
      )}
      {family.measured.length > 0 && (
        <CurvesView
          curves={curves}
          stopped={status === "stopped" || status === "failed"}
          drawing={running}
        />
      )}
      {finished && family.measured.length > 0 && (
        <p className={`${helpTextClassName} as-poses__legend`}>
          {t("poseStats.legend", {
            zone: placeKnown ? offsetText(offset) : "UTC",
          })}
        </p>
      )}
      {finished && family.judged && shapeless && flagged.length === 0 && (
        <p className="as-poses__headline">
          <Icon name="check" className="as-line__glyph" />
          <span>{t("poseStats.quiet")}</span>
        </p>
      )}
      {family.measured.length > 0 && family.judged && flagged.length > 0 && (
        <PoseTable
          family={family}
          rows={flagged.flatMap((f) => {
            const pose = family.measured.find((m) => m.path === f.path);
            return pose ? [{ pose, reason: reasonText(t, f.reason) }] : [];
          })}
          offset={offset}
        />
      )}
      {finished && family.judged && flagged.length > 0 && (
        <div className="as-poses__foot">
          <p className={helpTextClassName}>{t("poseStats.afterLine")}</p>
        </div>
      )}
      {finished && family.measured.length > 0 && (
        <div className="as-poses__foot as-poses__foot--end">
          <button
            type="button"
            className={buttonClassName("link")}
            aria-expanded={all}
            onClick={onAll}
          >
            {all
              ? t("poseStats.hideAll")
              : pluralText(
                  t,
                  locale,
                  "poseStats.viewAll",
                  family.measured.length,
                )}
          </button>
        </div>
      )}
      {all && (
        <PoseTable
          family={family}
          rows={family.measured.map((pose) => {
            const f = family.flagged.find((x) => x.path === pose.path);
            return { pose, reason: f ? reasonText(t, f.reason) : "" };
          })}
          offset={offset}
        />
      )}
    </>
  );
}

/** « 157 poses mesurées sur 466 : 36 de 10 s, 121 de 30 s (une sur quatre,
 * plus 13 voisines de poses signalées). » */
function doneText(
  t: ReturnType<typeof useI18n>["t"],
  locale: string,
  stats: NonNullable<ReturnType<typeof statsOf>>,
  measuredTotal: number,
): string {
  const parts = stats.families.map((family) => {
    const exposure = exposureText(t, locale, family.exposureSeconds);
    const count = family.measured.length;
    if (count >= family.total) {
      return t("poseStats.done.whole", { count, exposure });
    }
    // The first pass took one pose in four; the rest are neighbours of poses
    // that depart.
    const base = Math.ceil(family.total / 4);
    const extra = Math.max(0, count - base - 1);
    return extra > 0
      ? t("poseStats.done.sampledPlus", { count, exposure, step: 4, extra })
      : t("poseStats.done.sampled", { count, exposure, step: 4 });
  });
  return pluralText(t, locale, "poseStats.done", measuredTotal, {
    done: measuredTotal,
    total: stats.total,
    parts: parts.join(", "),
  });
}

/* ------------------------------------ The table -------------------------- */

export function PoseTable({
  family,
  rows,
  offset,
  select,
}: {
  family: FamilyStats;
  rows: { pose: PoseMeasure; reason: string }[];
  offset: number;
  /** A column of cases, one per pose (« Traitements »): which are ticked. */
  select?: {
    ticked: (path: string) => boolean;
    onToggle: (path: string, ticked: boolean) => void;
  };
}) {
  const { t, locale } = useI18n();
  const number = (value: number | null, digits: number) =>
    value === null
      ? "—"
      : new Intl.NumberFormat(locale, {
          minimumFractionDigits: digits,
          maximumFractionDigits: digits,
        }).format(value);
  return (
    <div className="as-poses__table-wrap">
      <table className="as-poses__table">
        <thead>
          <tr>
            {select && <th aria-hidden="true" />}
            <th>{t("poseStats.table.pose")}</th>
            <th>{t("poseStats.table.time")}</th>
            <th>{t("poseStats.table.fwhm")}</th>
            <th>{t("poseStats.table.eccentricity")}</th>
            <th>{t("poseStats.table.stars")}</th>
            <th>{t("poseStats.table.sky")}</th>
            <th>{t("poseStats.table.why")}</th>
          </tr>
        </thead>
        <tbody>
          <tr className="as-poses__median">
            {select && <td />}
            <td>{t("poseStats.table.median")}</td>
            <td>—</td>
            <td>{number(family.medians.fwhmPixels, 1)}</td>
            <td>{number(family.medians.eccentricity, 2)}</td>
            <td>{number(family.medians.starCount, 0)}</td>
            <td>{number(family.medians.skyBackground, 0)}</td>
            <td>{t("poseStats.table.medianWhy")}</td>
          </tr>
          {rows.map(({ pose, reason }) => (
            <tr
              key={pose.path}
              className={
                reason
                  ? select && !select.ticked(pose.path)
                    ? "is-flagged is-kept"
                    : "is-flagged"
                  : undefined
              }
            >
              {select && (
                <td>
                  <input
                    type="checkbox"
                    className="as-poses__tick"
                    checked={select.ticked(pose.path)}
                    aria-label={t("poseStats.table.number", {
                      number: pose.index + 1,
                    })}
                    onChange={(event) =>
                      select.onToggle(pose.path, event.target.checked)
                    }
                  />
                </td>
              )}
              <td>{t("poseStats.table.number", { number: pose.index + 1 })}</td>
              <td>{pose.unix === null ? "—" : clockText(pose.unix, offset)}</td>
              <td>{number(pose.fwhmPixels, 1)}</td>
              <td>{number(pose.eccentricity, 2)}</td>
              <td>{number(pose.starCount, 0)}</td>
              <td>{number(pose.skyBackground, 0)}</td>
              <td>
                {reason && (
                  <>
                    <Icon name="warning" className="as-line__glyph" /> {reason}
                  </>
                )}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

/* ------------------------------------ The curves ------------------------- */

const WIDTH = 690;
const HEIGHT = 56;
const LEFT = 36;
const RIGHT = 8;

function CurvesView({
  curves,
  stopped,
  drawing,
}: {
  curves: Curves;
  stopped: boolean;
  drawing: boolean;
}) {
  const { t, locale } = useI18n();
  const pattern = `as-poses-hatch-${useId().replace(/[^A-Za-z0-9_-]/g, "")}`;
  const number = (value: number, metric: Metric) =>
    new Intl.NumberFormat(locale, {
      maximumFractionDigits:
        metric === "eccentricity" ? 2 : metric === "fwhm" ? 1 : 0,
      minimumFractionDigits:
        metric === "eccentricity" ? 2 : metric === "fwhm" ? 1 : 0,
    }).format(value);
  const medianText = (curve: Curve) =>
    curve.medianValue === null
      ? ""
      : t(`poseStats.median.${curve.metric}`, {
          value: number(curve.medianValue, curve.metric),
        });
  const px = (x: number) => LEFT + x * (WIDTH - LEFT - RIGHT);
  const py = (y: number) => HEIGHT - 6 - y * (HEIGHT - 16);
  const shown = stopped ? curves.curves.slice(0, 1) : curves.curves;
  return (
    <div className="as-poses__curves">
      {shown.map((curve, index) => (
        <figure key={curve.metric} className="as-poses__curve">
          <figcaption>
            <strong>{t(`poseStats.curve.${curve.metric}`)}</strong>
            <span className={helpTextClassName}>{medianText(curve)}</span>
          </figcaption>
          <svg
            viewBox={`0 0 ${WIDTH} ${HEIGHT + (index === shown.length - 1 ? 16 : 0)}`}
            role="img"
            aria-label={`${t(`poseStats.curve.${curve.metric}`)} — ${medianText(curve)}`}
          >
            <defs>
              <pattern
                id={pattern}
                width="8"
                height="8"
                patternUnits="userSpaceOnUse"
                patternTransform="rotate(45)"
              >
                <line x1="0" y1="0" x2="0" y2="8" className="as-poses__hatch" />
              </pattern>
            </defs>
            <text
              x={LEFT - 6}
              y={py(1) + 4}
              className="as-poses__axis"
              textAnchor="end"
            >
              {number(curve.top, curve.metric)}
            </text>
            <text
              x={LEFT - 6}
              y={py(0) + 4}
              className="as-poses__axis"
              textAnchor="end"
            >
              {number(curve.bottom, curve.metric)}
            </text>
            <line
              x1={LEFT}
              x2={WIDTH - RIGHT}
              y1={py(0)}
              y2={py(0)}
              className="as-poses__baseline"
            />
            {curves.ticks.map((tick) => (
              <line
                key={tick.label + tick.x}
                x1={px(tick.x)}
                x2={px(tick.x)}
                y1={py(1)}
                y2={py(0)}
                className="as-poses__grid"
              />
            ))}
            {curve.points
              .filter((p) => p.flagged)
              .map((p) => (
                <rect
                  key={`band-${p.path}`}
                  x={px(p.x) - 4}
                  y={py(1) - 4}
                  width="8"
                  height={py(0) - py(1) + 8}
                  className="as-poses__band"
                />
              ))}
            {curve.median !== null && (
              <line
                x1={LEFT}
                x2={WIDTH - RIGHT}
                y1={py(curve.median)}
                y2={py(curve.median)}
                className="as-poses__median-line"
              />
            )}
            {curve.twenty !== null && (
              <>
                <line
                  x1={LEFT}
                  x2={WIDTH - RIGHT}
                  y1={py(curve.twenty)}
                  y2={py(curve.twenty)}
                  className="as-poses__twenty"
                />
                <text
                  x={WIDTH - RIGHT}
                  y={py(curve.twenty) - 3}
                  className="as-poses__mark"
                  textAnchor="end"
                >
                  {t("poseStats.twentyStars")}
                </text>
              </>
            )}
            {curves.unmeasured && (
              <>
                <rect
                  x={px(curves.unmeasured.from)}
                  y={py(1) - 4}
                  width={px(curves.unmeasured.to) - px(curves.unmeasured.from)}
                  height={py(0) - py(1) + 8}
                  fill={`url(#${pattern})`}
                />
                {index === 0 && (
                  <text
                    x={
                      (px(curves.unmeasured.from) + px(curves.unmeasured.to)) /
                      2
                    }
                    y={py(0.5)}
                    className="as-poses__axis"
                    textAnchor="middle"
                  >
                    {t("poseStats.unmeasured")}
                  </text>
                )}
              </>
            )}
            <polyline
              className="as-poses__line"
              fill="none"
              points={curve.points
                .map((p) => `${px(p.x)},${py(p.y)}`)
                .join(" ")}
            />
            {curve.points.map((p) => (
              <circle
                key={p.path}
                cx={px(p.x)}
                cy={py(p.y)}
                r={p.flagged ? 3.5 : 1.8}
                className={
                  p.flagged ? "as-poses__dot is-flagged" : "as-poses__dot"
                }
              />
            ))}
            {index === 0 &&
              curves.marks.map((mark) => (
                <text
                  key={`${mark.kind}-${mark.x}`}
                  x={px(mark.x)}
                  y={8}
                  className="as-poses__mark"
                  textAnchor="middle"
                >
                  {t(`poseStats.mark.${mark.kind}`)}
                </text>
              ))}
            {index === shown.length - 1 &&
              curves.ticks.map((tick) => (
                <text
                  key={`label-${tick.label}${tick.x}`}
                  x={px(tick.x)}
                  y={HEIGHT + 10}
                  className="as-poses__axis"
                  textAnchor="middle"
                >
                  {tick.label}
                </text>
              ))}
          </svg>
        </figure>
      ))}
      {drawing && <span className="as-poses__drawing" aria-hidden="true" />}
    </div>
  );
}

/* ------------------------------------ How a pose is flagged -------------- */

function PoseInfoDialog({ onClose }: { onClose: () => void }) {
  const { t } = useI18n();
  const titleId = `as-pose-info-${useId().replace(/[^A-Za-z0-9_-]/g, "")}`;
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        onClose();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);
  const rows = ["sky", "stars", "trailed", "wide", "fewStars", "fewPoses"];
  return (
    <div className="as-dialog__scrim" role="presentation" onClick={onClose}>
      <div
        className="as-dialog as-poses__dialog"
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        onClick={(event) => event.stopPropagation()}
      >
        <h2 id={titleId} className="as-dialog__title">
          {t("poseStats.info.title")}
        </h2>
        <p className={helpTextClassName}>{t("poseStats.info.lead")}</p>
        <dl className="as-poses__rules">
          {rows.map((row) => (
            <div key={row}>
              <dt>{t(`poseStats.info.${row}.title`)}</dt>
              <dd>{t(`poseStats.info.${row}.text`)}</dd>
            </div>
          ))}
        </dl>
        <p className={helpTextClassName}>{t("poseStats.info.foot")}</p>
        <div className="as-actions">
          <button
            type="button"
            className={buttonClassName("link")}
            onClick={onClose}
          >
            {t("common.close")}
          </button>
        </div>
      </div>
    </div>
  );
}

/* ---------------------------------- The place's time zone ---------------- */

let placesCache: Promise<{ name: string; timeZone: string | null }[]> | null =
  null;

/** The time zone the person's data gives the night's place (by its name). */
export function usePlaceZone(name: string | null): string | null {
  const [zone, setZone] = useState<string | null>(null);
  useEffect(() => {
    if (!name) return;
    placesCache ??= sessionsUserData()
      .then((data) =>
        data.places.map((place) => ({
          name: place.name,
          timeZone: place.timeZone ?? null,
        })),
      )
      .catch(() => {
        placesCache = null;
        return [];
      });
    let cancelled = false;
    void placesCache.then((places) => {
      if (cancelled) return;
      const wanted = name.trim().toLowerCase();
      setZone(
        places.find((place) => place.name.trim().toLowerCase() === wanted)
          ?.timeZone ?? null,
      );
    });
    return () => {
      cancelled = true;
    };
  }, [name]);
  return zone;
}
