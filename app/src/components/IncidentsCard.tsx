import {
  Fragment,
  useEffect,
  useId,
  useMemo,
  useState,
  type ReactNode,
} from "react";

import { useI18n } from "../i18n";
import { formatDuration } from "../lib/duration";
import {
  EDITABLE_GENRES,
  TEXT_LIMITS,
  edited,
  fatesOf,
  gapBetween,
  isEditableGenre,
  pairsOf,
  quickOf,
  summaryOf,
  type Fate,
  type Genre,
  type Line,
  type Quick,
  type Ticks,
} from "../lib/incidents";
import { planOf } from "../lib/incidentsDraft";
import { clockText } from "../lib/poseStats";
import { pluralText } from "../lib/written";
import { buttonClassName, helpTextClassName } from "../theme";
import { Icon } from "./Icon";

/** What the card shows: the list, or why there is none. */
export type IncidentsView =
  | { kind: "loading" }
  | {
      kind: "list";
      lines: Line[];
      ticks: Ticks;
      /** The night is finished: a detection may replace a declared signalement. */
      canReplace: boolean;
      /** What the detection could not say (a log it could not read…). */
      note: string | null;
      /** No pose of the night is measured: the card says that the lines of
       * the measure are not in the list. */
      unmeasured?: boolean;
      /** The detected lines that have no signalement of the contract to match:
       * not proposed. */
      unproposed: { text: string }[];
    }
  | { kind: "empty"; canMeasure: boolean }
  | { kind: "failed"; reason: string };

/** The duration of an incident: « 12 min », « 11 min 47 s », « 1 min ». */
export function incidentDuration(
  t: ReturnType<typeof useI18n>["t"],
  seconds: number,
): string {
  const whole = Math.round(seconds);
  if (whole % 60 === 0 && whole >= 60) {
    return t("common.minutesShort", { count: whole / 60 });
  }
  return formatDuration(t, whole * 1000);
}

/**
 * « Liste des incidents »: the signalements declared during the execution and
 * what Astro Sync detected, in one list by time, with a badge « Déclaré » or
 * « Détecté » (labels of state, never buttons), the pairs that seem to go
 * together and the quick choice above the list. The card has no button that
 * writes: what is ticked is added to the session by
 * the primary of the step -- « Envoyer cette nuit » for a night, « Envoyer N
 * images » or « Terminer » for the import --, and the foot says so.
 */
export function IncidentsCard({
  view,
  offset,
  targetName,
  onTick,
  onQuick,
  onEdit,
  onRemove,
  onAdd,
  onRetry,
  onMeasure,
  imported = false,
  noMeasure = false,
  selector = null,
  withStep,
}: {
  view: IncidentsView;
  /** Minutes east of UTC of the night's place. */
  offset: number;
  /** The night's target, when it has one only. */
  targetName: string | null;
  onTick: (id: string, value: boolean) => void;
  onQuick: (choice: Quick) => void;
  onEdit: (line: Line) => void;
  onRemove: (id: string) => void;
  onAdd: () => void;
  onRetry: () => void;
  /** « Mesurer les poses »; `null` where it cannot be done yet (the import's
   * « Récapitulatif » before the sessions are created). */
  onMeasure: (() => void) | null;
  /** The session was made by « Détecter les sessions »:
   * nothing was declared on it. */
  imported?: boolean;
  /** The poses are not measured on this screen (the result of « Détecter les
   * sessions »): the card says so, with the way to measure them. */
  noMeasure?: boolean;
  /** What stands under the title: the choice of session at the result of
   * « Détecter les sessions », one card per session created. */
  selector?: ReactNode;
  /** What is ticked is written by the primary of the step, `send` or
   * `finish`, and the foot says so. */
  withStep: "send" | "finish" | "create";
}) {
  const { t, locale } = useI18n();
  const [help, setHelp] = useState(false);
  const title = (
    <header className="as-incidents__head">
      <h3 className="as-incidents__title">{t("incidents.title")}</h3>
      {view.kind === "list" && (
        <span className="as-badge as-badge--neutral">
          {pluralText(t, locale, "incidents.lines", view.lines.length)}
        </span>
      )}
      <button
        type="button"
        className="as-button as-button--link as-incidents__info"
        aria-label={t("common.learnMore")}
        title={t("common.learnMore")}
        aria-expanded={help}
        onClick={() => setHelp((value) => !value)}
      >
        <Icon name="info" className="as-button__icon" />
      </button>
    </header>
  );
  // The poses are not measured: the lines of the measure (cloud, dawn,
  // trailing) are not in the list, and the card says so, with the way to get
  // them. At the result of « Détecter les sessions » there is no « Fichiers »
  // step to measure in; in a session's « Récapitulatif » the measure was not
  // made (the same note, the same way).
  const measureNote = (imported ||
    noMeasure ||
    (view.kind === "list" && view.unmeasured)) && (
    <p className={`${helpTextClassName} as-incidents__no-measure`}>
      <span>{t("incidents.noMeasure")}</span>
      {onMeasure && (
        <>
          {" "}
          <button
            type="button"
            className={buttonClassName("link")}
            onClick={onMeasure}
          >
            {t("poseStats.start")}
          </button>
        </>
      )}
    </p>
  );
  const helpBlock = help && (
    <div className="as-notice as-notice--neutral as-incidents__help">
      <p>{t("incidents.helpStep")}</p>
    </div>
  );

  if (view.kind === "loading") {
    return (
      <section className="as-incidents" aria-busy="true">
        {title}
        {helpBlock}
        {selector}
        <p className={helpTextClassName}>{t("incidents.loading")}</p>
      </section>
    );
  }
  if (view.kind === "failed") {
    return (
      <section className="as-incidents">
        {title}
        {helpBlock}
        {selector}
        <p className="as-incidents__state as-incidents__state--warning">
          <Icon name="warning" className="as-line__glyph" />
          <span>{t("incidents.failed", { reason: view.reason })}</span>
        </p>
        <div className="as-incidents__foot">
          <button
            type="button"
            className={buttonClassName("link")}
            onClick={onRetry}
          >
            {t("incidents.retry")}
          </button>
        </div>
      </section>
    );
  }
  if (view.kind === "empty") {
    return (
      <section className="as-incidents">
        {title}
        {helpBlock}
        {selector}
        <p className="as-incidents__state">
          <Icon name="check" className="as-line__glyph" />
          <span>{t("incidents.none")}</span>
          {view.canMeasure && onMeasure && (
            <button
              type="button"
              className={buttonClassName("link")}
              onClick={onMeasure}
            >
              {t("poseStats.start")}
            </button>
          )}
        </p>
        {measureNote}
        <div className="as-incidents__foot">
          <button
            type="button"
            className={buttonClassName("tertiary")}
            onClick={onAdd}
          >
            <Icon name="plus" className="as-button__icon" />
            {t("incidents.addOne")}
          </button>
        </div>
      </section>
    );
  }
  // The list.
  const { lines, ticks } = view;
  const fates = fatesOf(lines, ticks);
  const pairs = pairsOf(lines);
  const quick = quickOf(lines, ticks);
  const summary = summaryOf(lines, ticks);
  // What the primary of the step will write for this night.
  const plan = planOf(lines, ticks, "");
  const willAdd = plan.adds.length + plan.replacements.length;
  const detail: ReactNode[] = [
    summary.isolated > 0
      ? pluralText(t, locale, "incidents.sum.isolated", summary.isolated)
      : null,
    summary.replace > 0
      ? pluralText(t, locale, "incidents.sum.replace", summary.replace)
      : null,
    summary.alongside > 0 ? (
      <span className="as-incidents__double">
        {pluralText(t, locale, "incidents.sum.alongside", summary.alongside)}
      </span>
    ) : null,
  ].filter((part) => part !== null);
  const partnerOf = new Map<string, string>();
  for (const pair of pairs) {
    partnerOf.set(pair.declared, pair.detected);
    partnerOf.set(pair.detected, pair.declared);
  }
  // Group a pair under one frame, in the order of the list.
  const groups: Line[][] = [];
  const seen = new Set<string>();
  for (const line of lines) {
    if (seen.has(line.id)) continue;
    const other = partnerOf.get(line.id);
    if (other) {
      const mate = lines.find((l) => l.id === other) as Line;
      groups.push(line.origin === "declared" ? [line, mate] : [mate, line]);
      seen.add(other);
    } else groups.push([line]);
    seen.add(line.id);
  }
  const row = (line: Line, paired: boolean) => (
    <IncidentRow
      key={line.id}
      line={line}
      fate={fates[line.id]}
      paired={paired}
      offset={offset}
      targetName={targetName}
      choice={line.id in ticks ? ticks[line.id] : null}
      canReplace={view.canReplace}
      onTick={(value) => onTick(line.id, value)}
      onEdit={() => onEdit(line)}
      onRemove={() => onRemove(line.id)}
    />
  );
  const gapText = (a: Line, b: Line) => {
    const gap = gapBetween(a, b);
    return gap === 0
      ? t("incidents.pair.overlap")
      : t("incidents.pair.apart", { gap: incidentDuration(t, gap) });
  };
  return (
    <section className="as-incidents">
      {title}
      {helpBlock}
      {selector}
      <p className={helpTextClassName}>
        {t(
          imported && !lines.some((line) => line.origin === "declared")
            ? "incidents.introImportedStep"
            : "incidents.introStep",
        )}
      </p>
      {pairs.length > 0 && (
        <div
          className="as-incidents__quick"
          role="group"
          aria-label={pluralText(
            t,
            locale,
            "incidents.quick.label",
            pairs.length,
          )}
        >
          <span className={helpTextClassName}>
            {pluralText(t, locale, "incidents.quick.label", pairs.length)}
          </span>
          <span className="as-incidents__segments">
            {(["mine", "detections", "both"] as const).map((choice) => {
              const blocked = choice === "detections" && !view.canReplace;
              return (
                <button
                  key={choice}
                  type="button"
                  className={
                    choice === quick ? "as-segment is-chosen" : "as-segment"
                  }
                  aria-pressed={choice === quick}
                  disabled={blocked}
                  title={blocked ? t("incidents.quick.notFinished") : undefined}
                  onClick={() => onQuick(choice)}
                >
                  {t(`incidents.quick.${choice}`)}
                </button>
              );
            })}
          </span>
          {quick === null && (
            <span className={helpTextClassName}>
              {t("incidents.quick.custom")}
            </span>
          )}
          {!view.canReplace && (
            <span className={helpTextClassName}>
              {t("incidents.quick.notFinished")}
            </span>
          )}
        </div>
      )}
      <div className="as-incidents__list">
        {groups.map((group) =>
          group.length === 2 ? (
            <div key={group[0].id} className="as-incidents__pair">
              <p className="as-incidents__pair-head">
                <Icon name="link" className="as-line__glyph" />
                <span>
                  <strong>{t("incidents.pair.title")}</strong>{" "}
                  <span className={helpTextClassName}>
                    {t("incidents.pair.detail", {
                      genre: t(`incidents.genre.${group[0].genre}`),
                      when: gapText(group[0], group[1]),
                    })}
                  </span>
                </span>
              </p>
              {group.map((line) => row(line, true))}
            </div>
          ) : (
            row(group[0], false)
          ),
        )}
      </div>
      {view.unproposed.length > 0 && (
        <p className={`${helpTextClassName} as-incidents__unproposed`}>
          {t("incidents.notProposed", {
            count: view.unproposed.length,
            list: view.unproposed.map((u) => u.text).join(" · "),
          })}
        </p>
      )}
      {measureNote}
      <p className="as-incidents__summary">
        <strong>
          {willAdd === 0
            ? t("incidents.willAdd.none")
            : pluralText(t, locale, `incidents.willAdd.${withStep}`, willAdd)}
        </strong>
        {willAdd > 0 && detail.length > 0 && (
          <>
            {" ("}
            {detail.map((part, index) => (
              <Fragment key={index}>
                {index > 0 && " · "}
                {part}
              </Fragment>
            ))}
            {")"}
          </>
        )}
        {summary.declared > 0 && (
          <>
            <br />
            <span className={helpTextClassName}>
              {t("incidents.sum.kept", {
                kept: summary.kept,
                total: summary.declared,
              })}
            </span>
          </>
        )}
      </p>
      <div className="as-incidents__foot">
        <button
          type="button"
          className={buttonClassName("tertiary")}
          onClick={onAdd}
        >
          <Icon name="plus" className="as-button__icon" />
          {t("incidents.addOne")}
        </button>
      </div>
    </section>
  );
}

/* ------------------------------------------------------------ One line ----- */

function IncidentRow({
  line,
  fate,
  paired,
  offset,
  targetName,
  choice,
  canReplace,
  onTick,
  onEdit,
  onRemove,
}: {
  line: Line;
  fate: Fate;
  paired: boolean;
  offset: number;
  targetName: string | null;
  /** `null`: not a choice (a declared line alone). */
  choice: boolean | null;
  /** The night is finished: a signalement of the session can be replaced. */
  canReplace: boolean;
  onTick: (value: boolean) => void;
  onEdit: () => void;
  onRemove: () => void;
}) {
  const { t } = useI18n();
  const skipped = fate === "skipped" || fate === "replaced";
  return (
    <div className={skipped ? "as-incident is-dim" : "as-incident"}>
      <div className="as-incident__tick">
        {choice === null ? (
          <Icon name="check" className="as-incident__fixed" />
        ) : (
          <input
            type="checkbox"
            checked={choice}
            aria-label={t("incidents.tick", { text: line.text })}
            onChange={(event) => onTick(event.target.checked)}
          />
        )}
      </div>
      <div className="as-incident__when">
        <strong>{clockText(line.start, offset)}</strong>
        {line.end !== null && (
          <span className="as-incident__until">
            {" "}
            → {clockText(line.end, offset)}
          </span>
        )}
        {line.end !== null && line.end > line.start && (
          <div className="as-incident__duration">
            {incidentDuration(t, line.end - line.start)}
          </div>
        )}
      </div>
      <div className="as-incident__body">
        <p className="as-incident__head">
          <strong>{t(`incidents.genre.${line.genre}`)}</strong>{" "}
          <span
            className={
              line.origin === "declared" || line.manual
                ? "as-incident__badge as-incident__badge--declared"
                : "as-incident__badge as-incident__badge--detected"
            }
          >
            {t(`incidents.badge.${line.manual ? "declared" : line.origin}`)}
          </span>
          {targetName && (
            <span className={helpTextClassName}> · {targetName}</span>
          )}
        </p>
        <p className="as-incident__text">{line.text}</p>
        <p className={`${helpTextClassName} as-incident__evidence`}>
          {line.stored
            ? t("incidents.storedBefore")
            : line.origin === "declared"
              ? line.recordedAt === null
                ? t("incidents.declaredAnon")
                : t("incidents.declaredAt", {
                    time: clockText(line.recordedAt, offset),
                  })
              : line.evidence
                ? t(
                    `incidents.evidence.${line.evidence.code}`,
                    line.evidence.params,
                  )
                : null}
        </p>
        <p className={`${helpTextClassName} as-incident__fate`}>
          {t(
            `incidents.fate.${fate}${fate === "skipped" && !paired ? "Alone" : ""}`,
          )}
        </p>
      </div>
      <div className="as-incident__tools">
        {isEditableGenre(line.genre) ? (
          <button
            type="button"
            className="as-button as-button--link as-incident__tool"
            aria-label={t("incidents.edit.title")}
            title={
              line.origin === "declared" && !line.manual && !canReplace
                ? t("incidents.quick.notFinished")
                : t("incidents.edit.title")
            }
            disabled={line.origin === "declared" && !line.manual && !canReplace}
            onClick={onEdit}
          >
            <Icon name="pencil" className="as-button__icon" />
          </button>
        ) : (
          // A genre Astro Sync writes is removed, never edited: a
          // lock (a state, not a button: no focus stop) says so, in place of
          // the pencil.
          <span
            className="as-incident__tool as-incident__lock"
            role="img"
            aria-label={t("incidents.lock.title")}
            title={t("incidents.lock.title")}
          >
            <Icon name="lock" className="as-button__icon" />
          </span>
        )}
        <button
          type="button"
          className="as-button as-button--link as-incident__tool"
          aria-label={t("incidents.edit.remove")}
          title={t("incidents.edit.remove")}
          onClick={onRemove}
        >
          <Icon name="trash" className="as-button__icon" />
        </button>
      </div>
    </div>
  );
}

/* ------------------------------------------------------------ The dialog --- */

/** « Modifier l'incident » / « Ajouter un incident »: the genre (the four of the
 * execution), the start, the end (a pause only) and the text. */
export function IncidentDialog({
  line,
  adding,
  offset,
  base,
  lockGenre = false,
  onSave,
  onRemove,
  onClose,
}: {
  line: Line;
  adding: boolean;
  offset: number;
  /** The instant the times are written near (the line's, or the night's). */
  base: number;
  /** A signalement of the session is replaced by one of its own type: the
   * genre is not a choice. */
  lockGenre?: boolean;
  onSave: (line: Line) => void;
  onRemove: () => void;
  onClose: () => void;
}) {
  const { t, locale } = useI18n();
  const titleId = `as-incident-dialog-${useId().replace(/[^A-Za-z0-9_-]/g, "")}`;
  const [genre, setGenre] = useState<Genre>(
    EDITABLE_GENRES.includes(line.genre) ? line.genre : "note",
  );
  const [start, setStart] = useState(clockText(line.start, offset));
  const [end, setEnd] = useState(
    line.end === null ? "" : clockText(line.end, offset),
  );
  const [text, setText] = useState(line.text);
  const max = TEXT_LIMITS[genre];
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
  const startUnix = useMemo(
    () => unixOfClock(start, base, offset),
    [start, base, offset],
  );
  const endUnix = useMemo(
    () => (end ? unixOfClock(end, startUnix ?? base, offset, true) : null),
    [end, startUnix, base, offset],
  );
  const valid =
    startUnix !== null &&
    text.trim().length > 0 &&
    (genre !== "pause" ||
      end === "" ||
      (endUnix !== null && endUnix >= startUnix));
  return (
    <div className="as-dialog__scrim" role="presentation" onClick={onClose}>
      <div
        className="as-dialog as-incident-dialog"
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        onClick={(event) => event.stopPropagation()}
      >
        <h2 id={titleId} className="as-dialog__title">
          {adding ? t("incidents.add.title") : t("incidents.edit.title")}
        </h2>
        {!adding && (
          <p className={helpTextClassName}>
            <span
              className={
                line.origin === "declared"
                  ? "as-incident__badge as-incident__badge--declared"
                  : "as-incident__badge as-incident__badge--detected"
              }
            >
              {t(`incidents.badge.${line.origin}`)}
            </span>{" "}
            {line.origin === "detected" && line.evidence
              ? t(
                  `incidents.evidence.${line.evidence.code}`,
                  line.evidence.params,
                )
              : null}
          </p>
        )}
        <p className="as-incident-dialog__label">{t("incidents.edit.genre")}</p>
        <div className="as-incident-dialog__segments" role="group">
          {EDITABLE_GENRES.map((item) => (
            <button
              key={item}
              type="button"
              className={item === genre ? "as-segment is-chosen" : "as-segment"}
              aria-pressed={item === genre}
              disabled={lockGenre && item !== genre}
              onClick={() => setGenre(item)}
            >
              {t(`incidents.genre.${item}`)}
            </button>
          ))}
        </div>
        {adding && (
          <p
            className={`${helpTextClassName} as-incident-dialog__detected-help`}
          >
            {t("incidents.genre.detectedHelp")}
          </p>
        )}
        <div className="as-incident-dialog__times">
          <label>
            <span className="as-incident-dialog__label">
              {t("incidents.edit.start")}
            </span>
            <input
              type="time"
              className="as-field"
              value={start}
              onChange={(event) => setStart(event.target.value)}
            />
          </label>
          <label>
            <span className="as-incident-dialog__label">
              {t("incidents.edit.end")}
            </span>
            <input
              type="time"
              className="as-field"
              value={end}
              disabled={genre !== "pause"}
              placeholder={t("incidents.edit.endOnlyPause")}
              onChange={(event) => setEnd(event.target.value)}
            />
          </label>
        </div>
        {genre !== "pause" && (
          <p className={helpTextClassName}>
            {t("incidents.edit.endOnlyPause")}
          </p>
        )}
        <label>
          <span className="as-incident-dialog__label">
            {t("incidents.edit.text")}
          </span>
          <textarea
            className="as-field as-incident-dialog__text"
            value={text}
            maxLength={max}
            onChange={(event) => setText(event.target.value)}
          />
        </label>
        <p className={`${helpTextClassName} as-incident-dialog__count`}>
          {new Intl.NumberFormat(locale).format([...text].length)} /{" "}
          {new Intl.NumberFormat(locale).format(max)}
        </p>
        <div className="as-actions as-incident-dialog__foot">
          {!adding && (
            <button
              type="button"
              className={buttonClassName("link")}
              onClick={onRemove}
            >
              {t("incidents.edit.remove")}
            </button>
          )}
          <span className="as-incident-dialog__end">
            <button
              type="button"
              className={buttonClassName("link")}
              onClick={onClose}
            >
              {t("common.cancel")}
            </button>
            <button
              type="button"
              className={buttonClassName("primary")}
              disabled={!valid}
              onClick={() =>
                startUnix !== null &&
                onSave(
                  edited(line, {
                    genre,
                    start: startUnix,
                    end: genre === "pause" ? endUnix : null,
                    text,
                  }),
                )
              }
            >
              {adding ? t("incidents.add.save") : t("incidents.edit.save")}
            </button>
          </span>
        </div>
      </div>
    </div>
  );
}

/** The instant of a clock text « 02:52 » nearest to `reference` (the night
 * crosses midnight: « 00:14 » is after « 23:50 »); `after` takes the first
 * instant at or after the reference. */
export function unixOfClock(
  text: string,
  reference: number,
  offsetMinutes: number,
  after = false,
): number | null {
  const match = /^(\d{1,2}):(\d{2})$/.exec(text.trim());
  if (!match) return null;
  const hours = Number(match[1]);
  const minutes = Number(match[2]);
  if (hours > 23 || minutes > 59) return null;
  const local = reference + offsetMinutes * 60;
  const midnight = Math.floor(local / 86400) * 86400;
  const candidates = [-1, 0, 1].map(
    (day) =>
      midnight + day * 86400 + hours * 3600 + minutes * 60 - offsetMinutes * 60,
  );
  if (after) {
    return candidates.find((c) => c >= reference) ?? candidates[2];
  }
  return candidates.reduce((best, c) =>
    Math.abs(c - reference) < Math.abs(best - reference) ? c : best,
  );
}
