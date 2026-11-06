import { Icon } from "../../../components/Icon";
import { useI18n } from "../../../i18n";
import type { PlaceProposal } from "../../../lib/bridge";
import {
  choosePlace,
  formatCoordinates,
  formatKm,
  isBlankName,
  placeLine,
  placeOf,
} from "../../../lib/detectReview";
import { formatDay, pluralText } from "../../../lib/written";
import {
  badgeClassName,
  buttonClassName,
  helpTextClassName,
} from "../../../theme";
import { Tick, type ReviewContext } from "./parts";

/** « Lieux »: the places of the poses matched to « Mes lieux » within 10 km;
 * two of them ask which one; none asks to choose or to create. */
export function PlacesCard(props: ReviewContext) {
  const { t } = useI18n();
  return (
    <section className="as-review__card">
      <h2 className="as-review__title">{t("detect.review.places")}</h2>
      {props.proposal.places.map((place) => (
        <PlaceRow key={place.id} place={place} {...props} />
      ))}
    </section>
  );
}

function PlaceRow({
  place,
  proposal,
  user,
  state,
  change,
}: { place: PlaceProposal } & ReviewContext) {
  const { t, locale } = useI18n();
  const choice = placeOf(proposal, state, place);
  const nights =
    place.sessions.length === 1
      ? t("detect.place.night1", {
          date: formatDay(
            `${proposal.sessions.find((s) => s.id === place.sessions[0])?.eveningDate ?? ""}T12:00:00Z`,
            locale,
            "UTC",
          ),
        })
      : pluralText(t, locale, "detect.nights", place.sessions.length);
  const coordinates =
    place.latitude !== null && place.longitude !== null
      ? formatCoordinates(place.latitude, place.longitude, locale)
      : t("detect.place.noCoordinates");
  const several = place.candidates.length > 1;
  const nameKnown = place.candidates.length === 1;
  const line = placeLine(proposal, place);
  const answer = (next: Parameters<typeof choosePlace>[3]) =>
    change((current) => choosePlace(proposal, current, place, next));

  let title = coordinates;
  let sub: string;
  if (nameKnown) {
    const only = place.candidates[0];
    title = only.name;
    sub = t("detect.place.near", {
      nights,
      distance: formatKm(only.distanceKm, locale),
    });
  } else if (several) {
    sub = t(
      place.candidates.length === 2
        ? "detect.place.several.two"
        : "detect.place.several.other",
      { nights, count: place.candidates.length },
    );
  } else {
    sub = t("detect.place.unknown", { nights });
  }

  return (
    <div className="as-review__row" data-place={place.id}>
      <div className="as-review__row-text">
        <strong>{title}</strong>
        <div className="as-review__sub">{sub}</div>
      </div>
      {nameKnown && line && (
        <span className={`${badgeClassName("success")} as-review__pill`}>
          <Tick />
          {t("detect.place.known")}
        </span>
      )}
      {several && (
        <div
          className="as-review__radios"
          role="radiogroup"
          aria-label={coordinates}
        >
          {place.candidates.map((candidate) => (
            <label key={candidate.placeId} className="as-review__radio">
              <input
                type="radio"
                name={`place-${place.id}`}
                checked={
                  choice?.kind === "known" &&
                  choice.knownPlaceId === candidate.placeId
                }
                onChange={() =>
                  answer({ kind: "known", knownPlaceId: candidate.placeId })
                }
              />
              <span>
                {candidate.name}{" "}
                <span className="as-review__muted">
                  {t("detect.place.distance", {
                    distance: formatKm(candidate.distanceKm, locale),
                  })}
                </span>
              </span>
            </label>
          ))}
        </div>
      )}
      {!nameKnown && !several && (
        <PlaceAnswer
          place={place}
          choice={choice}
          proposal={proposal}
          user={user}
          state={state}
          change={change}
          answer={answer}
        />
      )}
    </div>
  );
}

/** « Choisir un lieu » / « Créer ce lieu », then the answer and « Changer ». */
function PlaceAnswer({
  place,
  choice,
  user,
  answer,
}: {
  place: PlaceProposal;
  choice: ReturnType<typeof placeOf>;
  answer: (next: Parameters<typeof choosePlace>[3]) => void;
} & ReviewContext) {
  const { t } = useI18n();
  const canCreate = place.latitude !== null && place.longitude !== null;
  if (choice?.kind === "known") {
    const name =
      user.places.find((item) => item.id === choice.knownPlaceId)?.name ?? "";
    return (
      <div className="as-review__answer">
        <span className={`${badgeClassName("success")} as-review__pill`}>
          <Tick />
          {name}
        </span>
        <button
          type="button"
          className={buttonClassName("link")}
          onClick={() => answer(null)}
        >
          {t("detect.place.change")}
        </button>
      </div>
    );
  }
  if (choice?.kind === "create") {
    // The name is already taken into account at each keystroke: the pill says
    // what will be created, and the link gives the choice back (it does not
    // « validate » anything).
    const blank = isBlankName(choice.name);
    return (
      <div className="as-review__answer as-review__answer--create">
        <div className="as-review__answer-line">
          {blank ? (
            <span className={`${badgeClassName("warning")} as-review__pill`}>
              {t("detect.place.nameMissing")}
            </span>
          ) : (
            <span className={`${badgeClassName("success")} as-review__pill`}>
              <Tick />
              {t("detect.place.toCreate", { name: choice.name.trim() })}
            </span>
          )}
          <button
            type="button"
            className={buttonClassName("link")}
            onClick={() => answer(null)}
          >
            {t("detect.place.change")}
          </button>
        </div>
        <label className="as-review__name-field">
          <span className={helpTextClassName}>{t("detect.place.name")}</span>
          <input
            type="text"
            className="as-field as-review__name"
            value={choice.name}
            aria-invalid={blank || undefined}
            onChange={(event) =>
              answer({ kind: "create", name: event.target.value })
            }
          />
        </label>
        {blank && (
          <p className={`${helpTextClassName} as-review__name-why`}>
            {t("detect.place.nameMissingWhy")}
          </p>
        )}
      </div>
    );
  }
  return (
    <div className="as-review__answer">
      <label
        className={`${buttonClassName("tertiary")} as-review__select`}
        data-testid="choose-place"
      >
        <Icon name="chevron" className="as-button__icon as-review__chevron" />
        {t("detect.place.choose")}
        <select
          aria-label={t("detect.place.choose")}
          value=""
          onChange={(event) =>
            event.target.value &&
            answer({ kind: "known", knownPlaceId: event.target.value })
          }
        >
          <option value="" />
          {user.places.map((item) => (
            <option key={item.id} value={item.id}>
              {item.name}
            </option>
          ))}
        </select>
      </label>
      {canCreate && (
        <button
          type="button"
          className={buttonClassName("tertiary")}
          onClick={() =>
            // A word, never the position of the frames: the person names the
            // place in the field next to it.
            answer({ kind: "create", name: t("detect.place.defaultName") })
          }
        >
          {t("detect.place.create")}
        </button>
      )}
    </div>
  );
}
