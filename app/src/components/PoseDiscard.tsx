import { useEffect, useRef, useState } from "react";

import { useI18n } from "../i18n";
import { utcOffsetMinutes } from "../lib/detectPlaces";
import {
  discardTick,
  discardView,
  discardedPaths,
  exposureText,
  placeOffsetMinutes,
  reasonCountsText,
  reasonText,
  statsOf,
  type TargetPoses,
} from "../lib/poseStats";
import { pluralText } from "../lib/written";
import { buttonClassName, helpTextClassName } from "../theme";
import { Icon } from "./Icon";
import { PoseTable, usePlaceZone } from "./PoseStatsCard";

/**
 * What « Traitements » says about the measure of a target's poses: the poses flagged are proposed
 * out of the stacking -- **one case**, ticked by default, with the reason of
 * that default, the count of what will be stacked, and the list behind
 * « Voir les N poses » (a case each); without a measure, with a measure
 * stopped, or with too few stars, the card says what will be stacked and
 * why, never a silence. « Mesurer les poses » is a link back to « Fichiers
 * », where the measure goes.
 */
export function PoseDiscard({
  entry,
  total,
  place,
  onAll,
  onOne,
  onMeasure,
}: {
  entry: TargetPoses;
  /** The target's poses in the night. */
  total: number;
  /** The night's place: the hours of the list are its own. */
  place: { name: string | null; longitude: number | null };
  onAll: (discard: boolean) => void;
  onOne: (path: string, discard: boolean) => void;
  /** Back to « Fichiers » (« Mesurer les poses », « Reprendre la mesure »). */
  onMeasure: () => void;
}) {
  const { t, locale } = useI18n();
  const view = discardView(entry, total);
  const [open, setOpen] = useState(false);
  const master = useRef<HTMLInputElement>(null);
  const tick = discardTick(entry);
  useEffect(() => {
    if (master.current) master.current.indeterminate = tick === "some";
  }, [tick, view.kind]);
  const stats = statsOf(entry);
  const zone = usePlaceZone(place.name);
  const first = stats?.families
    .flatMap((f) => f.measured)
    .find((m) => m.unix !== null)?.unix;
  const offset = placeOffsetMinutes(
    zone,
    place.longitude,
    first ?? Date.now() / 1000,
    utcOffsetMinutes,
  );

  if (view.kind === "none") return null;
  if (view.kind === "unmeasured") {
    return (
      <div className="as-pose-discard as-pose-discard--plain">
        <p>
          {pluralText(t, locale, "poseDiscard.unmeasured", view.poses, {
            count: view.poses,
          })}
        </p>
        <button
          type="button"
          className={buttonClassName("link")}
          onClick={onMeasure}
        >
          {t("poseStats.start")}
        </button>
      </div>
    );
  }
  if (view.kind === "partial") {
    return (
      <div className="as-pose-discard as-pose-discard--plain">
        <p>
          <Icon name="warning" className="as-line__glyph" />
          <span>
            {t("poseDiscard.partial", {
              others: Math.max(0, view.planned - view.measured),
              count: total,
            })}
          </span>
        </p>
        <button
          type="button"
          className={buttonClassName("link")}
          onClick={onMeasure}
        >
          {t("poseStats.resume")}
        </button>
      </div>
    );
  }
  if (view.kind === "few") {
    return (
      <div className="as-pose-discard as-pose-discard--plain">
        <p>
          <Icon name="check" className="as-line__glyph" />
          <span>{t("poseDiscard.few", { count: total })}</span>
        </p>
      </div>
    );
  }

  const flagged = view.flagged;
  const left = discardedPaths(entry);
  // What each family stacks: its poses less the ones left out.
  const lines = (stats?.families ?? [])
    .map((family) => {
      const mine = new Set(family.flagged.map((f) => f.path));
      const out = left.filter((path) => mine.has(path)).length;
      return out > 0
        ? t("poseDiscard.stackedCount", {
            kept: family.total - out,
            exposure: exposureText(t, locale, family.exposureSeconds),
            total: family.total,
          })
        : null;
    })
    .filter((line): line is string => line !== null);
  const partial = tick === "some" || tick === "none";
  return (
    <div className="as-pose-discard">
      <label className="as-pose-discard__head">
        <input
          ref={master}
          type="checkbox"
          className="as-pose-discard__tick"
          checked={tick === "all"}
          onChange={(event) => onAll(event.target.checked)}
        />
        <strong>
          {partial
            ? t("poseDiscard.titlePartial", {
                left: left.length,
                count: flagged.length,
              })
            : pluralText(t, locale, "poseDiscard.title", flagged.length)}
        </strong>
      </label>
      <p className={helpTextClassName}>
        {reasonCountsText(t, flagged, true, " · ")}
      </p>
      <p className="as-pose-discard__stacked">
        {t("poseDiscard.stackedLead")}{" "}
        <strong>
          {lines.length > 0
            ? lines.join(" · ")
            : t("poseDiscard.stackedAll", { count: total })}
        </strong>
        {" · "}
        {pluralText(t, locale, "poseDiscard.stackedTail", left.length, {
          count: left.length,
        })}
      </p>
      <div className="as-pose-discard__foot">
        <p className={helpTextClassName}>
          {partial ? "" : t("poseDiscard.why")}
        </p>
        <button
          type="button"
          className={buttonClassName("link")}
          aria-expanded={open}
          onClick={() => setOpen((value) => !value)}
        >
          {open
            ? t("poseStats.hideAll")
            : pluralText(t, locale, "poseDiscard.seeAll", flagged.length)}
        </button>
      </div>
      {open &&
        stats?.families
          .filter((family) => family.flagged.length > 0)
          .map((family) => (
            <PoseTable
              key={family.label}
              family={family}
              offset={offset}
              rows={family.flagged.flatMap((f) => {
                const pose = family.measured.find((m) => m.path === f.path);
                return pose ? [{ pose, reason: reasonText(t, f.reason) }] : [];
              })}
              select={{
                ticked: (path) => left.includes(path),
                onToggle: (path, ticked) => onOne(path, ticked),
              }}
            />
          ))}
    </div>
  );
}
