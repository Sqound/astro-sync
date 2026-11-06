import { InfoLine } from "../../../components/InfoLine";
import { useI18n } from "../../../i18n";
import type { DetectedSession } from "../../../lib/bridge";
import type { Counts, CreateLabel } from "../../../lib/detectReview";
import { dayMonth, summaryParts } from "../../../lib/detectReview";
import { pluralText } from "../../../lib/written";
import { buttonClassName } from "../../../theme";

/** The message of the review's foot (above « Précédent » and the primary):
 * « Sera créé : … » -- « Créé : … » once done --, what is left unanswered,
 * and why the primary is greyed. */
export function ReviewSummary({
  found,
  label,
  waiting,
  unnamed,
  created,
}: {
  found: Counts;
  label: CreateLabel;
  /** The nights that wait for their band, oldest first. */
  waiting: DetectedSession[];
  /** The places to create that have no name (« Continuer » waits for one). */
  unnamed: number;
  /** The sessions are created: the review is read-only. */
  created: boolean;
}) {
  const { t, locale } = useI18n();
  const parts = summaryParts(t, locale, found);
  const first = waiting[0];
  // The reason of the greyed primary: only the nights that wait for a band.
  const why = unnamed === 0 && label.kind === "none" && first ? first : null;
  return (
    <div className="as-review__foot-message">
      <InfoLine
        line={
          <span className="as-review__summary">
            {parts.length > 0 ? (
              <>
                <strong>
                  {created
                    ? t("import.review.createdTitle")
                    : t("detect.summary.title")}
                </strong>{" "}
                {t("detect.summary", {
                  parts: parts.join(t("detect.sep")),
                })}
              </>
            ) : (
              t("detect.summary.nothing")
            )}
            {!created &&
              found.unanswered.map((item) => (
                <span
                  key={`${item.designation}-${item.date}`}
                  className="as-review__sub as-review__block"
                >
                  {t("detect.summary.unanswered", {
                    target: item.designation,
                    date: dayMonth(item.date, locale),
                  })}
                </span>
              ))}
          </span>
        }
        details={<p>{t("detect.review.details")}</p>}
      />
      {unnamed > 0 && (
        <p className="as-review__why" id="as-review-why">
          {t("detect.place.nameMissingWhy")}
        </p>
      )}
      {why && (
        <p className="as-review__why" id="as-review-why">
          {pluralText(t, locale, "detect.create.why.filter", waiting.length, {
            date: dayMonth(why.eveningDate, locale),
          })}{" "}
          <button
            type="button"
            className={buttonClassName("link")}
            onClick={() => showFilterChoice(why.id)}
          >
            {t("detect.create.why.go")}
          </button>
        </p>
      )}
    </div>
  );
}

/** « Créer 3 sessions et 1 plan multi-nuits », as the primary names it. */
export function createText(
  label: CreateLabel,
  t: ReturnType<typeof useI18n>["t"],
  locale: string,
): string {
  const sessions = (count: number) =>
    pluralText(t, locale, "detect.count.sessions", count);
  const plans = (count: number) =>
    pluralText(t, locale, "detect.count.multi", count);
  switch (label.kind) {
    case "both":
      return t("detect.create", {
        sessions: sessions(label.sessions),
        plans: plans(label.plans),
      });
    case "sessions":
      return t("detect.create.sessions", {
        sessions: sessions(label.sessions),
      });
    case "plans":
      return t("detect.create.plans", { plans: plans(label.plans) });
    case "update":
      return pluralText(t, locale, "detect.create.update", label.sessions);
    case "none":
      return t("detect.create.none");
  }
}

/** Scrolls to the band choice of a night, marks it and gives it the keyboard
 * focus (its first button). */
function showFilterChoice(sessionId: string) {
  const target = [
    ...document.querySelectorAll<HTMLElement>("[data-filter]"),
  ].find((item) => item.dataset.filter === sessionId);
  if (!target) return;
  target.scrollIntoView({ block: "center", behavior: "smooth" });
  target.classList.add("is-called");
  target.querySelector<HTMLElement>("button")?.focus({ preventScroll: true });
}
