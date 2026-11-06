import { Icon } from "../../../components/Icon";
import { offersOpener, useSpaceWeb } from "../../../components/TabParts";
import { useI18n } from "../../../i18n";
import type { MultiNightGroup } from "../../../lib/bridge";
import {
  datesText,
  dayMonth,
  formatHours,
  groupStatus,
  listText,
  placeNameOf,
  singleNightTargets,
  toggleLine,
} from "../../../lib/detectReview";
import { pluralText } from "../../../lib/written";
import { buttonClassName } from "../../../theme";
import { useJoin, type ReviewContext } from "./parts";

/** « Plans multi-nuits »: one row per target taken over several nights; the
 * goal is the total acquired. Without the advanced planning the groups are
 * shown locked, with « Débloquer la fonctionnalité ». */
export function MultiNightCard(props: ReviewContext) {
  const { t, locale } = useI18n();
  const { web } = useSpaceWeb();
  const offers = offersOpener(web, locale);
  const { proposal } = props;
  if (proposal.multiNight.length === 0) return null;
  const single = singleNightTargets(proposal);
  const locked = proposal.multiNight.some((group) => group.locked);
  return (
    <section
      className={
        locked ? "as-review__card as-review__card--locked" : "as-review__card"
      }
    >
      <h2 className="as-review__title">{t("detect.review.multiNight")}</h2>
      {proposal.multiNight.map((group) => (
        <GroupRow key={group.id} group={group} {...props} />
      ))}
      {locked && (
        <div className="as-review__end">
          <button
            type="button"
            className={buttonClassName("tertiary")}
            onClick={offers}
          >
            <Icon name="external" className="as-button__icon" />
            {t("detect.multi.unlock")}
          </button>
        </div>
      )}
      {single.length > 0 && (
        <div className="as-review__aside as-review__muted">
          {t("detect.multi.single", { targets: listText(single, locale) })}
        </div>
      )}
    </section>
  );
}

function GroupRow({
  group,
  proposal,
  user,
  state,
  change,
}: { group: MultiNightGroup } & ReviewContext) {
  const { t, locale } = useI18n();
  const join = useJoin();
  const status = groupStatus(proposal, state, group);
  const line = proposal.lines.find(
    (item) => item.kind === "multi-night" && item.subject === group.id,
  );
  // Waiting or locked, the group shows all its nights; on, the ones left on.
  const nights =
    status.waitsPlace || group.locked ? group.nights : status.nights;
  const seconds =
    status.waitsPlace || group.locked ? group.goalSeconds : status.seconds;
  const frames =
    status.waitsPlace || group.locked ? group.frames : status.frames;
  const dates = datesText(
    nights.map((night) => night.eveningDate),
    locale,
  );
  const firstSession = proposal.sessions.find(
    (session) => session.id === group.nights[0]?.sessionId,
  );
  const place = firstSession
    ? placeNameOf(proposal, user, state, firstSession.place)
    : null;
  const off = group.locked || status.waitsPlace || status.short;
  const sub = join([
    pluralText(t, locale, "detect.nights", nights.length),
    dates,
    place ?? "",
    pluralText(t, locale, "detect.night.frames", frames),
    formatHours(seconds),
  ]);
  const tag = (
    <span className="as-review__tag">
      <Icon name="clock" className="as-icon" />
      {t("detect.multi.tag")}
    </span>
  );
  return (
    <div
      className={off ? "as-review__group is-off" : "as-review__group"}
      data-group={group.id}
    >
      <div className="as-review__c0">
        {group.locked ? (
          <Icon
            name="lock"
            className="as-icon as-review__lock"
            aria-label={t("detect.multi.lockedLabel")}
            role="img"
          />
        ) : (
          <input
            type="checkbox"
            aria-label={group.designation}
            checked={status.on}
            disabled={status.waitsPlace || status.short}
            onChange={() =>
              line && change((current) => toggleLine(current, line.id))
            }
          />
        )}
      </div>
      <div className="as-review__row-text">
        <div className="as-review__group-title">
          <strong>
            {group.locked
              ? t("detect.multi.titleLocked", {
                  target: group.designation,
                  nights: pluralText(t, locale, "detect.nights", nights.length),
                })
              : group.designation}
          </strong>
          {tag}
        </div>
        <div className="as-review__sub">{sub}</div>
      </div>
      <div className="as-review__group-text">
        {group.locked ? (
          <span className="as-review__muted">
            {t("detect.multi.locked", { target: group.designation })}
          </span>
        ) : status.waitsPlace ? (
          <span className="as-review__muted">
            {t("detect.multi.waitsPlace", {
              date: dayMonth(status.waitingDate ?? "", locale),
            })}
          </span>
        ) : status.short ? (
          <span className="as-review__muted">
            {t("detect.multi.needTwo", { target: group.designation })}
          </span>
        ) : (
          t("detect.multi.new", {
            dates: datesText(
              status.nights.map((night) => night.eveningDate),
              locale,
            ),
            goal: formatHours(status.seconds),
          })
        )}
      </div>
    </div>
  );
}
