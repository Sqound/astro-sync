import { useState, type ReactNode } from "react";

import { Icon } from "../../../components/Icon";
import { offersOpener, useSpaceWeb } from "../../../components/TabParts";
import { useI18n } from "../../../i18n";
import type {
  DetectedSession,
  DetectedTarget,
  ProposalLine,
} from "../../../lib/bridge";
import {
  SHIFT_CHOICES,
  answerLine,
  asideFiles,
  borrowsCalibration,
  calibrationItems,
  chooseFilter,
  chooseShift,
  dayMonth,
  eveningLabel,
  exposures,
  formatHours,
  formatShift,
  logItems,
  logKindName,
  matchCriteria,
  nightLines,
  nightStatus,
  placeNameOf,
  shiftInstant,
  toggleLine,
  utcHour,
  listText,
  type CalibrationItem,
  type NightStatus,
} from "../../../lib/detectReview";
import {
  nothingReadable,
  solveTried,
  unreadAsideParts,
  unreadCounts,
} from "../../../lib/missingMessages";
import type { ScreenId } from "../../../lib/menu";
import { pluralText } from "../../../lib/written";
import { badgeClassName, buttonClassName } from "../../../theme";
import { CalibrationNotes, SolveWhy } from "./MissingParts";
import { Lead, Tick, useJoin, type ReviewContext } from "./parts";

/** « Nuits »: one row per night, one tick per row, what will be done in
 * plain words, the calibration and the logs attached, and the cases of the
 * night (an existing session, a target to add, an overlap, a clock). */
export function NightsCard(
  props: ReviewContext & {
    free: boolean;
    /** « Réglages › Dépendances », from the reasons of a plate solving. */
    onOpen?: (screen: ScreenId) => void;
  },
) {
  const { t, locale } = useI18n();
  const { web } = useSpaceWeb();
  const offers = offersOpener(web, locale);
  const [open, setOpen] = useState(false);
  const aside = asideFiles(props.proposal);
  const asideParts = [
    aside.noDate.length > 0
      ? pluralText(t, locale, "detect.aside.noDate", aside.noDate.length)
      : "",
    aside.calibration.length + aside.logs.length > 0
      ? t("detect.aside.unattached", {
          what: listText(
            [
              aside.calibration.length > 0
                ? pluralText(
                    t,
                    locale,
                    "detect.aside.calibration",
                    aside.calibration.length,
                  )
                : "",
              aside.logs.length > 0
                ? pluralText(t, locale, "detect.aside.logs", aside.logs.length)
                : "",
            ].filter(Boolean),
            locale,
          ),
        })
      : "",
    // The videos and the RAW files the detection does not read; when
    // nothing at all is readable the warning of the page says it.
    ...(nothingReadable(props.proposal)
      ? []
      : unreadAsideParts(t, locale, unreadCounts(props.proposal))),
  ].filter(Boolean);
  return (
    <section className="as-review__card">
      <h2 className="as-review__title">{t("detect.review.nights")}</h2>
      {props.free && (
        <div className="as-notice as-notice--warning as-review__free">
          <p>{t("detect.free.notice")}</p>
          <div className="as-review__end">
            <button
              type="button"
              className={buttonClassName("link")}
              onClick={offers}
            >
              <Icon name="external" className="as-button__icon" />
              {t("detect.offersWeb")}
            </button>
          </div>
        </div>
      )}
      <div className="as-review__nights" role="table">
        <div className="as-review__nights-head" role="row">
          <span role="columnheader" />
          <span role="columnheader">{t("detect.night.head.night")}</span>
          <span role="columnheader">{t("detect.night.head.targets")}</span>
          <span role="columnheader">{t("detect.night.head.done")}</span>
        </div>
        {props.proposal.sessions.map((session) => (
          <NightRow
            key={session.id}
            session={session}
            {...props}
            onSeeFiles={() => setOpen(true)}
            onSettings={
              props.onOpen ? () => props.onOpen?.("dependencies") : undefined
            }
          />
        ))}
      </div>
      {asideParts.length > 0 && (
        <div className="as-review__aside">
          <span className="as-review__muted">
            {t("detect.aside", { parts: asideParts.join(t("detect.sep")) })}
          </span>
          {aside.all.length > 0 && (
            <button
              type="button"
              className={buttonClassName("link")}
              aria-expanded={open}
              onClick={() => setOpen((current) => !current)}
            >
              {t(open ? "detect.aside.hide" : "detect.aside.see", {
                count: aside.all.length,
              })}
            </button>
          )}
        </div>
      )}
      {open && (
        <ul className="as-review__files">
          {aside.all.map((file, index) => (
            <li key={`${file}-${index}`}>{file}</li>
          ))}
        </ul>
      )}
    </section>
  );
}

function NightRow({
  session,
  proposal,
  user,
  state,
  change,
  onSeeFiles,
  onSettings,
}: {
  session: DetectedSession;
  onSeeFiles: () => void;
  onSettings?: () => void;
} & ReviewContext) {
  const { t, locale } = useI18n();
  const join = useJoin();
  const status = nightStatus(proposal, state, session);
  const lines = nightLines(proposal)[session.id];
  const place = placeNameOf(proposal, user, state, session.place);
  // The hours the night shows are the ones after the shift in force.
  const moved = (instant: string) =>
    utcHour(shiftInstant(instant, status.shiftHours - session.shiftHours));
  const calibration = calibrationItems(proposal, session);
  const logs = logItems(proposal, session);
  const disabled = status.locked || status.waitsPlace || status.waitsFilter;
  const label = eveningLabel(session.eveningDate, locale);
  const targetLine = (target: DetectedTarget) => lines.targets[target.key];

  return (
    <div
      className={disabled ? "as-review__night is-off" : "as-review__night"}
      role="row"
      data-session={session.id}
    >
      <div className="as-review__c0" role="cell">
        {lines.session && (
          <input
            type="checkbox"
            aria-label={label}
            checked={status.on}
            disabled={disabled}
            onChange={() =>
              lines.session &&
              change((current) => toggleLine(current, lines.session!.id))
            }
          />
        )}
      </div>
      <div className="as-review__c1" role="cell">
        <strong>{label}</strong>
        <div className="as-review__sub">
          {t("detect.night.hours", {
            start: moved(session.start),
            end: moved(session.end),
          })}
          <br />
          {place ?? t("detect.place.unknownShort")}
        </div>
      </div>
      <div className="as-review__c2" role="cell">
        {session.targets.map((target) => (
          <div key={target.key}>
            <strong>{target.designation}</strong>{" "}
            <span className="as-review__muted">
              {join([
                "",
                pluralText(t, locale, "detect.night.frames", target.frames),
                t("detect.night.exposure", {
                  seconds: exposures(target).join(t("detect.comma")),
                }),
                formatHours(target.exposureSeconds),
              ])}
            </span>
          </div>
        ))}
        {calibration.length > 0 && (
          <div className="as-review__sub as-review__attached">
            <strong>{t("detect.night.calib")}</strong>
            {join([
              "",
              ...calibration.map((item) => calibrationText(item, t, locale)),
            ])}
          </div>
        )}
        {logs.length > 0 && (
          <div className="as-review__sub as-review__attached">
            <strong>{t("detect.night.logs")}</strong>
            {join([
              "",
              logs
                .map((item) => logKindName(item.kind))
                .join(t("detect.comma")),
            ])}
          </div>
        )}
      </div>
      <div className="as-review__c3" role="cell">
        <WhatWillBeDone session={session} status={status} />
      </div>
      <div className="as-review__below">
        <CalibrationNotes
          notes={session.calibrationNotes ?? []}
          onSeeFiles={onSeeFiles}
        />
        {session.targets.map((target) => {
          const line = targetLine(target);
          if (!line) return null;
          return (
            <TargetCase
              key={target.key}
              session={session}
              target={target}
              line={line}
              status={status}
              moved={moved}
              state={state}
              change={change}
              onSettings={onSettings}
            />
          );
        })}
        {session.filterAsk && (
          <FilterCase session={session} state={state} change={change} />
        )}
        {lines.clock &&
          (lines.clock.reason === "clock.shift" ||
            lines.clock.reason === "clock.suspect") && (
            <ClockCase
              session={session}
              status={status}
              place={place}
              proposal={proposal}
              user={user}
              state={state}
              change={change}
            />
          )}
        {borrowsCalibration(calibration) && (
          <CalibrationDetail
            session={session}
            calibration={calibration}
            proposal={proposal}
          />
        )}
      </div>
    </div>
  );
}

/** « 30 darks », « 30 darks du 9 août » when the set was taken another day. */
function calibrationText(
  item: CalibrationItem,
  t: ReturnType<typeof useI18n>["t"],
  locale: string,
): string {
  const label = t(`detect.night.calibKind.${item.kind}`, {
    count: item.frames,
  });
  return item.daysAway > 0 && item.date
    ? t("detect.night.calibFrom", {
        label,
        date: dayMonth(item.date, locale),
      })
    : label;
}

/** The last column: what will be done, in plain words. */
function WhatWillBeDone({
  session,
  status,
}: {
  session: DetectedSession;
  status: NightStatus;
}) {
  const { t, locale } = useI18n();
  const existing = session.existing;
  if (status.waitsPlace) {
    return (
      <span className="as-review__muted">{t("detect.night.placeMissing")}</span>
    );
  }
  if (status.waitsFilter) {
    return (
      <span className="as-review__muted">
        {t("detect.night.filterMissing")}
      </span>
    );
  }
  if (existing.kind === "create") {
    return (
      <span>
        {status.shiftHours !== 0
          ? t("detect.night.newShifted", {
              shift: formatShift(status.shiftHours),
            })
          : t("detect.night.new")}
      </span>
    );
  }
  if (existing.kind === "existing") {
    const inside = session.targets.filter(
      (target) => target.existing === "in-session",
    );
    return (
      <div className="as-review__case">
        <span className={`${badgeClassName("outline")} as-review__pill`}>
          {t("detect.night.exists")}
        </span>
        <span>
          {inside.length === 0
            ? t("detect.night.existsOnly")
            : inside.map((target) => (
                <span key={target.key} className="as-review__block">
                  <Lead
                    lead={target.designation}
                    body={pluralText(
                      t,
                      locale,
                      // The short form when the night also adds a
                      // target to the session (11 August), the long one else.
                      session.targets.some(
                        (other) => other.existing === "add-to-session",
                      )
                        ? "detect.night.useSessionShort"
                        : "detect.night.useSession",
                      target.frames,
                    )}
                  />
                </span>
              ))}
        </span>
      </div>
    );
  }
  if (existing.kind === "reuse-plan") {
    return (
      <div className="as-review__case">
        <span className={`${badgeClassName("outline")} as-review__pill`}>
          {t("detect.night.planBadge")}
        </span>
        <span>{t("detect.night.planReuse", { plan: existing.planName })}</span>
      </div>
    );
  }
  if (existing.kind === "replace-plan") {
    const targets = listText(
      session.targets.map((target) => target.designation),
      locale,
    );
    return (
      <div className="as-review__case">
        <span className={`${badgeClassName("outline")} as-review__pill`}>
          {t("detect.night.planBadge")}
        </span>
        <span>
          <Lead
            lead={t("detect.night.planReplaceLead", {
              plan: existing.planName,
              target: targets,
            })}
            body={t("detect.night.planReplace", { target: targets })}
          />
        </span>
      </div>
    );
  }
  return (
    <span className="as-review__muted">
      {t("detect.night.dateTaken", { plan: existing.planName })}
    </span>
  );
}

/** The case a target brings to its night: to add, to answer, to create. */
function TargetCase({
  session,
  target,
  line,
  status,
  moved,
  state,
  change,
  onSettings,
}: {
  session: DetectedSession;
  target: DetectedTarget;
  line: ProposalLine;
  status: NightStatus;
  moved: (instant: string) => string;
  onSettings?: () => void;
} & Pick<ReviewContext, "state" | "change">) {
  const { t, locale } = useI18n();
  const window = {
    start: moved(target.start),
    end: moved(target.end),
  };
  if (line.reason === "target.addToSession") {
    return (
      <label className="as-review__case as-review__add">
        <input
          type="checkbox"
          checked={status.on && Boolean(state.ticked[line.id])}
          disabled={!status.on}
          onChange={() => change((current) => toggleLine(current, line.id))}
        />
        <span>
          <Lead
            lead={t("detect.night.addTargetLead", {
              target: target.designation,
            })}
            body={t("detect.night.addTarget", window)}
          />
        </span>
      </label>
    );
  }
  if (line.reason === "target.createPersonal") {
    return (
      <label className="as-review__case as-review__add">
        <input
          type="checkbox"
          checked={status.on && Boolean(state.ticked[line.id])}
          disabled={!status.on}
          onChange={() => change((current) => toggleLine(current, line.id))}
        />
        <span>
          {t("detect.night.createTarget", { target: target.designation })}
        </span>
      </label>
    );
  }
  if (line.reason === "target.toSolve") {
    return (
      <div className="as-review__ask">
        <div className="as-review__ask-text">
          <Icon name="clock" className="as-icon as-review__ask-icon" />
          <span>
            {t(
              solveTried(target)
                ? "detect.night.toSolveTried"
                : "detect.night.toSolve",
              {
                count: target.frames,
                frame: target.solveFrame ?? "",
              },
            )}
          </span>
        </div>
        <SolveWhy target={target} onSettings={onSettings} />
      </div>
    );
  }
  if (line.reason === "target.overlap") {
    const answer = state.answers[line.id];
    const other = listText(
      session.targets
        .filter((item) => item.key !== target.key)
        .map((item) => item.designation),
      locale,
    );
    return (
      <div className="as-review__ask" role="group">
        <div className="as-review__ask-text">
          <Icon name="warning" className="as-icon as-review__ask-icon" />
          <span>
            <strong>{target.designation}</strong>{" "}
            {t("detect.night.overlap", { ...window, other })}
          </span>
        </div>
        <div className="as-review__end">
          {(["skip", "add"] as const).map((choice) => (
            <button
              key={choice}
              type="button"
              disabled={!status.on}
              aria-pressed={answer === choice}
              className={`${buttonClassName("tertiary")} as-review__answer-button${answer === choice ? " is-chosen" : ""}`}
              onClick={() =>
                change((current) => answerLine(current, line.id, choice))
              }
            >
              {t(
                choice === "skip"
                  ? "detect.night.overlapSkip"
                  : "detect.night.overlapAdd",
                { target: target.designation },
              )}
            </button>
          ))}
        </div>
      </div>
    );
  }
  return null;
}

/** « Filtre de cette nuit ? »: the poses name none. When the names of the
 * night say its band, that band is preselected and the line says where it
 * was read; else nothing is supposed and the person says the band of the
 * night (one answer per night). The two-band limit is said under it. */
export function FilterCase({
  session,
  state,
  change,
}: { session: DetectedSession } & Pick<ReviewContext, "state" | "change">) {
  const { t, locale } = useI18n();
  const answer = state.filters[session.id];
  const guess = session.filterGuess;
  // The line stays while the guess is the answer; once changed, it is gone.
  const guessed = guess && answer === guess.band ? guess : null;
  const bandName = (band: "broadband" | "dualband") =>
    t(`wizard.stacking.band.${band}`);
  return (
    <div
      className={`as-review__ask${answer ? " is-answered" : " is-unanswered"}`}
      role="group"
      data-filter={session.id}
    >
      <div className="as-review__ask-text">
        {answer ? (
          <Icon
            name="check"
            className="as-icon as-review__ask-icon is-answered"
          />
        ) : (
          <Icon name="warning" className="as-icon as-review__ask-icon" />
        )}
        <span>
          {answer ? (
            // Answered: a statement, not a question -- the band and where it
            // comes from.
            <>
              <strong>
                {t("detect.night.filterAnswered", {
                  band: inSentence(bandName(answer), locale),
                })}
              </strong>{" "}
              {guessed
                ? guessed.origin === "note"
                  ? t("detect.night.filterGuessNote", {
                      source: guessed.source,
                      file: guessed.noteFile ?? "SESSION.md",
                    })
                  : t("detect.night.filterGuess", { source: guessed.source })
                : t("detect.night.filterAnsweredBody")}
            </>
          ) : (
            <>
              <strong>{t("detect.night.filterTitle")}</strong>{" "}
              {t("detect.night.filterBody")}
            </>
          )}
        </span>
      </div>
      <div
        className="as-review__end"
        role="radiogroup"
        aria-label={t("detect.night.filterTitle")}
      >
        {(["broadband", "dualband"] as const).map((band) => (
          <button
            key={band}
            type="button"
            role="radio"
            aria-checked={answer === band}
            className={`${buttonClassName("tertiary")} as-review__answer-button${answer === band ? " is-chosen" : ""}`}
            onClick={() =>
              change((current) =>
                chooseFilter(
                  current,
                  session.id,
                  answer === band ? null : band,
                ),
              )
            }
          >
            {answer === band && <Tick />}
            {bandName(band)}
          </button>
        ))}
      </div>
      <p className="as-review__ask-note">{t("detect.night.filterLimit")}</p>
    </div>
  );
}

/** A band's name inside a sentence: « Large bande » → « large bande ». German
 * nouns keep their capital. */
function inSentence(label: string, locale: string): string {
  return locale.startsWith("de") ? label : label.toLocaleLowerCase(locale);
}

/** « Horloge de la caméra décalée ? »: why, and the shift to apply. */
function ClockCase({
  session,
  status,
  place,
  user,
  state,
  change,
  proposal,
}: {
  session: DetectedSession;
  status: NightStatus;
  place: string | null;
} & ReviewContext) {
  const { t, locale } = useI18n();
  const clock = session.clock;
  if (!clock) return null;
  const written = (instant: string) =>
    utcHour(shiftInstant(instant, -session.shiftHours));
  const clockPlace = clock.place;
  const where =
    clockPlace.from === "default-place"
      ? (user.places.find((item) => item.id === clockPlace.placeId)?.name ??
        place)
      : place;
  const proposed = clock.proposedShiftHours;
  const hours = state.shifts[session.id] ?? 0;
  const current = status.shiftHours;
  const code = clock.reasons[0]?.code ?? "sun-up";
  const body =
    proposed === null
      ? t("detect.night.clockNone")
      : t(`detect.night.clock.${code}`, {
          start: written(session.start),
          end: written(session.end),
          place: where ?? t("detect.place.unknownShort"),
          shift: formatShift(proposed),
          target: listText(
            session.targets.map((target) => target.designation),
            locale,
          ),
        });
  return (
    <div className="as-review__ask" data-clock={session.id}>
      <div className="as-review__ask-text">
        <Icon name="warning" className="as-icon as-review__ask-icon" />
        <span>
          <strong>{t("detect.night.clockTitle")}</strong> {body}
        </span>
      </div>
      <div className="as-review__clock-controls">
        <label className="as-review__check">
          <input
            type="checkbox"
            checked={current !== 0}
            disabled={!status.on || (current === 0 && hours === 0)}
            onChange={() =>
              change((now) =>
                chooseShift(
                  proposal,
                  now,
                  session.id,
                  current !== 0 ? 0 : hours,
                ),
              )
            }
          />
          <span>{t("detect.night.clockApply")}</span>
        </label>
        <ShiftSelect
          value={current}
          disabled={!status.on}
          onChange={(next) =>
            change((now) => chooseShift(proposal, now, session.id, next))
          }
        />
      </div>
      <div className="as-review__sub">{t("detect.night.clockScope")}</div>
    </div>
  );
}

/** The menu of the hours: « Aucun décalage », then −12 h to +12 h. */
function ShiftSelect({
  value,
  disabled,
  onChange,
}: {
  value: number;
  disabled: boolean;
  onChange: (hours: number) => void;
}) {
  const { t } = useI18n();
  return (
    <label
      className={`${buttonClassName("tertiary")} as-review__select as-review__select--small`}
    >
      <Icon name="chevron" className="as-button__icon as-review__chevron" />
      {value === 0 ? t("detect.night.clockNo") : formatShift(value)}
      <select
        aria-label={t("detect.night.clockApply")}
        value={value}
        disabled={disabled}
        onChange={(event) => onChange(Number(event.target.value))}
      >
        <option value={0}>{t("detect.night.clockNo")}</option>
        {SHIFT_CHOICES.map((hours) => (
          <option key={hours} value={hours}>
            {formatShift(hours)}
          </option>
        ))}
      </select>
    </label>
  );
}

/** The calibration and the logs of a night that borrows the set of another
 * day: what each is, when it was taken and why it fits. */
function CalibrationDetail({
  session,
  calibration,
  proposal,
}: {
  session: DetectedSession;
  calibration: CalibrationItem[];
  proposal: ReviewContext["proposal"];
}) {
  const { t, locale } = useI18n();
  const join = useJoin();
  const logs = logItems(proposal, session);
  const criteria = matchCriteria(calibration);
  const filtered = criteria.includes("filter");
  const dayDiff = (item: CalibrationItem) =>
    item.date
      ? Math.round(
          (Date.parse(`${session.eveningDate}T12:00:00Z`) -
            Date.parse(`${item.date}T12:00:00Z`)) /
            86_400_000,
        )
      : 0;
  const line = (item: CalibrationItem): ReactNode => {
    const kind = t(`detect.night.calibName.${item.kind}`);
    const date = item.date ? dayMonth(item.date, locale) : "";
    const sameDay = item.daysAway === 0 && item.hour;
    const matched = ["exposure", "temperature", "gain", "filter"]
      .filter((name) => item.matchedOn.includes(name))
      .map((name) => t(`detect.night.same.${name}`));
    const note =
      item.daysAway > 0
        ? dayDiff(item) === 1
          ? t("detect.night.calibEve", {
              kind: t(`detect.night.calibOne.${item.kind}`),
              matched: matched.join(t("detect.comma")),
            })
          : t("detect.night.calibOtherDay", {
              kind: t(`detect.night.calibOne.${item.kind}`),
              date,
              matched: matched.join(t("detect.comma")),
            })
        : "";
    return (
      <div key={item.setId} className="as-review__detail-line">
        <strong>{kind}</strong>
        {join([
          "",
          String(item.frames),
          item.exposureSeconds !== null
            ? t("detect.night.exposure", { seconds: item.exposureSeconds })
            : "",
          item.temperatureC !== null
            ? t("detect.night.temperature", {
                value: String(item.temperatureC).replace("-", "−"),
              })
            : "",
          item.gain !== null ? t("detect.night.gain", { gain: item.gain }) : "",
          item.filter ?? "",
          item.date
            ? sameDay
              ? t("detect.night.takenAt", { date, hour: item.hour ?? "" })
              : t("detect.night.taken", { date })
            : "",
        ])}
        {note && <> {note}</>}
      </div>
    );
  };
  return (
    <div className="as-review__detail">
      {calibration.map(line)}
      {logs.map((item) => (
        <div key={item.kind} className="as-review__detail-line">
          <strong>{t("detect.night.logs")}</strong>
          {join([
            "",
            t("detect.night.logLine", {
              kind: logKindName(item.kind),
              date: item.date ? dayMonth(item.date, locale) : "",
              files: pluralText(t, locale, "detect.night.files", item.count),
              read: t(
                ["asiair", "phd2", "nina"].includes(item.kind)
                  ? `detect.night.logRead.${item.kind}`
                  : "detect.night.logRead.other",
              ),
            }),
          ])}
        </div>
      ))}
      <div className="as-review__sub">
        {t(
          filtered
            ? "detect.night.calibBasis"
            : "detect.night.calibBasisNoFilter",
          {
            criteria: listText(
              criteria.map((name) => t(`detect.night.by.${name}`)),
              locale,
            ),
          },
        )}
      </div>
    </div>
  );
}
