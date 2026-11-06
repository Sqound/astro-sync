import { useEffect, useRef, useState } from "react";

import { Icon } from "../../components/Icon";
import { InfoLine } from "../../components/InfoLine";
import { Notice } from "../../components/Notice";
import { Waiting } from "../../components/Waiting";
import { useI18n } from "../../i18n";
import {
  fitsChoose,
  fitsTargetGuess,
  openInBrowser,
  targetsSearch,
  type CatalogueTarget,
  type FitsGuess,
  type PersonalTargetCreate,
} from "../../lib/bridge";
import { formatError } from "../../lib/format";
import {
  eveningOfSession,
  longitudeOf,
  nightCell,
  placeOf,
} from "../../lib/nights";
import {
  EMPTY_DRAFT,
  PERSONAL_LABELS,
  SEARCH_DELAY_MS,
  SEARCH_PAGE,
  cardLines,
  counterText,
  draftFromGuess,
  draftToCreate,
  fitsView,
  imageAddress,
  isOfflineFailure,
  moreAction,
  searchReady,
  type PersonalTargetDraft,
} from "../../lib/targetSearch";
import {
  ciblesLine,
  ciblesPlan,
  targetsCount,
  type NightAction,
  type NightChoices,
} from "../../lib/nightRecap";
import {
  continueWithTargets,
  targetChoices,
  targetFolderName,
} from "../../lib/wizardSteps";
import {
  buttonClassName,
  fieldClassName,
  fieldLabelClassName,
  helpTextClassName,
  inactiveButtonClassName,
  noticeClassName,
} from "../../theme";
import { nightPage, type NightContext } from "./NightWizard";
import { StepFoot } from "./parts";
import { PlanAttachBlock } from "./PlanAttach";

/** The web planner's « Cadrages », where a custom framing is made. */
export function framingsPage(web: string, locale: string): string {
  return `${web.replace(/\/$/, "")}/${locale}/planner?view=framings`;
}

type Search =
  | { kind: "idle" }
  | { kind: "searching"; text: string; previous: CatalogueTarget[] }
  | { kind: "results"; text: string; total: number; targets: CatalogueTarget[] }
  | { kind: "error"; message: string };

type Fits =
  | { kind: "none" }
  | { kind: "reading"; file: string }
  | { kind: "read"; guess: FitsGuess }
  | { kind: "error"; message: string };

/**
 * « Cibles »: first the night's
 * targets, then, under them, the two ways of **adding** a target -- the
 * catalogue's search and « Trouver la cible avec un FITS » -- whose cards
 * each add one to the list above; « Nouvelle cible hors catalogue (saisie
 * manuelle) » opens the form; the night's targets are
 * tickable for the sending; a target already saved is managed in the
 * Journal on the web, which the step says with its link.
 *
 * Nothing is written on this step before « Continuer »: a card of the catalogue and the form of « Nouvelle
 * cible » add a row to the list -- « nouveau », ticked --, the plan chosen
 * below is a row too, and « Continuer » applies every ticked one; the case of
 * a row is its undo.
 */
export function TargetsStep({
  context,
  choices,
  recap,
  onRecap,
  planReads,
  working,
  applyError,
  onToggle,
  onOffers,
  onContinue,
}: {
  context: NightContext;
  choices: ReturnType<typeof targetChoices>;
  /** What the person chose on this step and « Continuer » applies. */
  recap: NightChoices;
  onRecap: (action: NightAction) => void;
  /** Bumped when the primary wrote: the plan block reads again. */
  planReads: number;
  /** The words of what « Continuer » is writing, `null` when it is not. */
  working: string | null;
  /** Why the last « Continuer » stopped, `null` when it did not. */
  applyError: string | null;
  onToggle: (targetId: string) => void;
  /** « Voir les offres » when the web's page is not known. */
  onOffers: () => void;
  onContinue: () => void;
}) {
  const { t, locale } = useI18n();
  const [text, setText] = useState("");
  const [search, setSearch] = useState<Search>({ kind: "idle" });
  const [limit, setLimit] = useState(SEARCH_PAGE);
  const [offline, setOffline] = useState(false);
  const [fits, setFits] = useState<Fits>({ kind: "none" });
  const [error, setError] = useState<string | null>(null);
  const [form, setForm] = useState<PersonalTargetDraft | null>(null);
  const [retry, setRetry] = useState(0);
  const sequence = useRef(0);
  const field = useRef<HTMLInputElement>(null);

  const session = context.session;
  const web =
    context.status?.space?.webUrl ?? context.status?.serviceUrl ?? null;
  const serviceUrl = context.status?.serviceUrl ?? null;
  const inNight = new Set(choices.map((choice) => choice.targetId));
  const plan = ciblesPlan(recap);
  const ticked = targetsCount(
    choices.filter((choice) => choice.ticked).length,
    plan,
  );
  // The catalogue's cards already in the list, ticked.
  const listed = new Set(
    recap.targets
      .filter((target) => target.ticked && target.targetId !== null)
      .map((target) => target.targetId as string),
  );
  const line = ciblesLine(plan, t, locale);

  // The catalogue, after two characters and a pause; an answer that comes
  // back after a newer question is dropped.
  useEffect(() => {
    if (!searchReady(text)) {
      setSearch({ kind: "idle" });
      return;
    }
    const asked = ++sequence.current;
    const timer = window.setTimeout(() => {
      setSearch((current) => ({
        kind: "searching",
        text,
        previous: current.kind === "results" ? current.targets : [],
      }));
      targetsSearch(text, locale, 0, limit)
        .then((found) => {
          if (asked !== sequence.current) return;
          setOffline(false);
          setSearch({
            kind: "results",
            text,
            total: found.total,
            targets: found.targets,
          });
        })
        .catch((caught) => {
          if (asked !== sequence.current) return;
          if (isOfflineFailure(caught)) {
            setOffline(true);
            setSearch({ kind: "idle" });
          } else {
            setSearch({ kind: "error", message: formatError(t, caught) });
          }
        });
    }, SEARCH_DELAY_MS);
    return () => window.clearTimeout(timer);
  }, [text, locale, limit, t, retry]);

  /** A card of the catalogue: one more row of the list, ticked. */
  function add(target: { id: string; designation: string }) {
    setError(null);
    onRecap({
      type: "pickCatalogue",
      targetId: target.id,
      designation: target.designation,
    });
    // The field empties and keeps its place: the next one follows.
    setText("");
    setLimit(SEARCH_PAGE);
    setFits({ kind: "none" });
    field.current?.focus();
  }

  async function readFits(fill: ((guess: FitsGuess) => void) | null = null) {
    setError(null);
    const file = await fitsChoose().catch(() => null);
    if (!file) return;
    if (fill) {
      try {
        fill(await fitsTargetGuess(file, locale));
      } catch (caught) {
        setError(formatError(t, caught));
      }
      return;
    }
    setFits({ kind: "reading", file });
    try {
      const guess = await fitsTargetGuess(file, locale);
      setFits({ kind: "read", guess });
      if (guess.kind === "nothing") field.current?.focus();
    } catch (caught) {
      if (isOfflineFailure(caught)) {
        setOffline(true);
        setFits({ kind: "none" });
      } else setFits({ kind: "error", message: formatError(t, caught) });
    }
  }

  const card = (target: CatalogueTarget) => {
    const lines = cardLines(t, locale, target);
    const image = imageAddress(serviceUrl, target.imagePath);
    const already = inNight.has(target.id);
    const inList = listed.has(target.id);
    return (
      <li key={target.id} className="as-target-card">
        <span className="as-target-card__preview" aria-hidden="true">
          {image ? (
            <img src={image} alt="" loading="lazy" />
          ) : (
            <Icon name="sparkle" />
          )}
        </span>
        <span className="as-target-card__text">
          <span>
            <strong>{lines.designation}</strong>
            {lines.name && (
              <span className="as-target-card__name"> · {lines.name}</span>
            )}
          </span>
          {lines.kind && (
            <span className="as-target-card__kind">{lines.kind}</span>
          )}
          {lines.details && (
            <span className="as-target-card__details">{lines.details}</span>
          )}
        </span>
        {already || inList ? (
          <span className="as-target-card__in-night">
            <Icon name="check" className="as-button__icon" />
            {t(
              already
                ? "wizard.targets.card.inNight"
                : "wizard.targets.card.inList",
            )}
          </span>
        ) : (
          <button
            type="button"
            className={buttonClassName("tertiary")}
            disabled={offline || working !== null}
            onClick={() => add(target)}
          >
            <Icon name="plus" className="as-button__icon" />
            {t("wizard.targets.card.add", { name: target.designation })}
          </button>
        )}
      </li>
    );
  };

  const createOffer = (draft: PersonalTargetDraft) => (
    <button
      type="button"
      className={buttonClassName("tertiary")}
      disabled={offline}
      onClick={() => setForm(draft)}
    >
      <Icon name="plus" className="as-button__icon" />
      {t("wizard.targets.search.createAs", { text: draft.name })}
    </button>
  );

  const shown =
    search.kind === "results"
      ? search.targets
      : search.kind === "searching"
        ? search.previous
        : [];
  const more =
    search.kind === "results"
      ? moreAction(search.targets.length, search.total)
      : null;

  return (
    <div className="as-step">
      <h2 className="as-step__heading">{t("wizard.targets.title")}</h2>
      <p className={helpTextClassName}>
        {[
          session
            ? nightCell(
                eveningOfSession(session, longitudeOf(context.detail)),
                locale,
              )
            : null,
          placeOf(context.detail),
        ]
          .filter(Boolean)
          .join(" · ")}
      </p>

      {/* 1. The night's targets, which each addition joins; the zone that
          adds comes under them, whether the list is empty, holds
          one target or several. */}
      <section className="as-targets-list" aria-labelledby="targets-list-title">
        <h3 id="targets-list-title" className="as-targets__subheading">
          {t("wizard.targets.list.title")}
        </h3>
        {choices.length === 0 && recap.targets.length === 0 ? (
          <div className="as-targets-empty">
            <p>
              <strong>{t("wizard.targets.list.emptyTitle")}</strong>
            </p>
            <p className={helpTextClassName}>
              {t("wizard.targets.list.emptyLine")}
            </p>
          </div>
        ) : (
          <div className="as-choices">
            {choices.map((choice) => (
              <div key={choice.targetId} className="as-choice as-target-row">
                <label className="as-target-row__label">
                  <input
                    type="checkbox"
                    checked={choice.ticked}
                    disabled={working !== null}
                    onChange={() => onToggle(choice.targetId)}
                  />
                  <span className="as-choice__text">
                    <span>
                      <strong>{choice.designation}</strong>
                    </span>
                    {choice.line && (
                      <span className="as-choice__line">{choice.line}</span>
                    )}
                  </span>
                </label>
                <code className="as-choice__folder">{choice.folderName}</code>
              </div>
            ))}
            {/* The targets « Continuer » will add: a row each, « nouveau »;
                its case is the undo. */}
            {recap.targets.map((target) => (
              <div
                key={target.key}
                className="as-choice as-target-row"
                data-pending={target.kind}
              >
                <label className="as-target-row__label">
                  <input
                    type="checkbox"
                    checked={target.ticked}
                    disabled={working !== null}
                    onChange={() =>
                      onRecap({
                        type: "tickTarget",
                        key: target.key,
                        on: !target.ticked,
                      })
                    }
                  />
                  <span className="as-choice__text">
                    <span>
                      <strong>{target.designation}</strong>
                      <span className="as-badge as-badge--new">
                        {t("wizard.new")}
                      </span>
                    </span>
                    <span className="as-choice__line">
                      {t(
                        !target.ticked
                          ? "wizard.targets.pending.skipped"
                          : target.kind === "create"
                            ? "wizard.targets.pending.create"
                            : "wizard.targets.pending.add",
                      )}
                    </span>
                  </span>
                </label>
                <code className="as-choice__folder">{`${targetFolderName(target.designation)}/`}</code>
              </div>
            ))}
          </div>
        )}
        <div className="as-targets-framing">
          <InfoLine
            line={t("wizard.targets.list.manage")}
            details={<p>{t("wizard.targets.list.manageDetails")}</p>}
          />
          {/* Greyed offline, as the framing link. */}
          <button
            type="button"
            className={buttonClassName("link")}
            disabled={offline || !web}
            onClick={() =>
              web &&
              void openInBrowser(nightPage(web, locale, context.sessionId))
            }
          >
            <Icon name="external" className="as-button__icon" />
            {t("wizard.targets.list.manageOpen")}
          </button>
        </div>
      </section>

      <Notice tone="error">{error}</Notice>

      {/* 2. Adding, under the targets the night already has: the
          catalogue's search, a capture's FITS, the form. */}
      <section className="as-targets-add" aria-labelledby="targets-add-title">
        <h3 id="targets-add-title" className="as-targets__subheading">
          {t("wizard.targets.add.title")}
        </h3>
        {offline && (
          <div
            className={`${noticeClassName("neutral")} as-targets-offline`}
            role="status"
          >
            <span>{t("wizard.targets.search.offline")}</span>
            <button
              type="button"
              className={buttonClassName("link")}
              onClick={() => {
                setOffline(false);
                setRetry((current) => current + 1);
              }}
            >
              {t("wizard.retry")}
            </button>
          </div>
        )}
        <div className="as-targets-add__row">
          <label className="as-targets-search">
            <span className={fieldLabelClassName}>
              {t("wizard.targets.search.label")}
            </span>
            <span className="as-targets-search__box">
              <Icon name="search" className="as-targets-search__icon" />
              <input
                ref={field}
                className={fieldClassName}
                type="search"
                autoFocus={choices.length === 0}
                disabled={offline}
                placeholder={t("wizard.targets.search.placeholder")}
                value={text}
                onChange={(event) => {
                  setLimit(SEARCH_PAGE);
                  setText(event.target.value);
                }}
              />
            </span>
          </label>
          <button
            type="button"
            className={`${buttonClassName("tertiary")} as-targets-fits`}
            disabled={offline || fits.kind === "reading"}
            onClick={() => void readFits()}
          >
            <Icon name="file" className="as-button__icon" />
            {t("wizard.targets.fits.choose")}
          </button>
        </div>

        {search.kind === "searching" && (
          <Waiting label={t("wizard.targets.search.searching")} size="inline" />
        )}
        {search.kind === "error" && (
          <Notice tone="error">{search.message}</Notice>
        )}
        {shown.length > 0 && (
          <>
            <ul className="as-target-cards">{shown.map(card)}</ul>
            {search.kind === "results" && (
              <div className="as-targets-more">
                <span className={helpTextClassName}>
                  {counterText(t, locale, search.targets.length, search.total)}
                </span>
                {more === "more" && (
                  <button
                    type="button"
                    className={buttonClassName("tertiary")}
                    onClick={() => setLimit((current) => current + SEARCH_PAGE)}
                  >
                    {t("wizard.targets.search.more")}
                  </button>
                )}
                {more === "refine" && (
                  <span className={helpTextClassName}>
                    {t("wizard.targets.search.refine")}
                  </span>
                )}
              </div>
            )}
          </>
        )}
        {search.kind === "results" && search.targets.length === 0 && (
          <div className="as-targets-none">
            <p>
              {t("wizard.targets.search.none", { text: search.text.trim() })}
            </p>
            {createOffer({ ...EMPTY_DRAFT, name: search.text.trim() })}
          </div>
        )}

        {fits.kind !== "none" && (
          <FitsBlock
            fits={fits}
            onAnother={() => void readFits()}
            renderCard={card}
            renderCreate={createOffer}
          />
        )}

        <div className="as-targets-manual">
          <button
            type="button"
            className={buttonClassName("tertiary")}
            disabled={offline}
            onClick={() => setForm({ ...EMPTY_DRAFT })}
          >
            <Icon name="plus" className="as-button__icon" />
            {t("wizard.targets.manual.open")}
          </button>
          <span className={helpTextClassName}>
            {t("wizard.targets.manual.line")}
          </span>
        </div>
        <div className="as-targets-framing">
          <InfoLine
            line={t("wizard.targets.framing.line")}
            details={<p>{t("wizard.targets.framing.details")}</p>}
          />
          <button
            type="button"
            className={buttonClassName("link")}
            disabled={offline || !web}
            onClick={() => web && void openInBrowser(framingsPage(web, locale))}
          >
            <Icon name="external" className="as-button__icon" />
            {t("wizard.targets.framing.open")}
          </button>
        </div>
      </section>

      {/* 3. The multi-night plan the night belongs to. */}
      <PlanAttachBlock
        context={context}
        recap={recap}
        onRecap={onRecap}
        reads={planReads}
        working={working !== null}
        onOffers={onOffers}
      />

      <InfoLine
        line={t("wizard.targets.folderNext")}
        details={<p>{t("wizard.targets.folderNextDetails")}</p>}
      />
      <Notice tone="error">{applyError}</Notice>
      <StepFoot
        line={line}
        working={working}
        primary={continueWithTargets(t, locale, ticked)}
        enabled={ticked > 0 && working === null}
        onPrimary={onContinue}
      />

      {form && (
        <NewTargetDialog
          initial={form}
          web={web}
          onReadFits={(fill) => void readFits(fill)}
          onCancel={() => setForm(null)}
          onRecord={(target) => {
            onRecap({ type: "pickNew", create: target });
            setForm(null);
            setText("");
            setFits({ kind: "none" });
          }}
        />
      )}
    </div>
  );
}

/** What a capture's FITS says. */
function FitsBlock({
  fits,
  onAnother,
  renderCard,
  renderCreate,
}: {
  fits: Exclude<Fits, { kind: "none" }>;
  onAnother: () => void;
  renderCard: (target: CatalogueTarget) => React.ReactNode;
  renderCreate: (draft: PersonalTargetDraft) => React.ReactNode;
}) {
  const { t, locale } = useI18n();
  const name =
    fits.kind === "read"
      ? fits.guess.fileName
      : fits.kind === "reading"
        ? fits.file.split(/[\\/]/).pop()
        : null;
  const view = fits.kind === "read" ? fitsView(t, locale, fits.guess) : null;
  return (
    <div className="as-targets-fits-block">
      {name && (
        <div className="as-targets-fits-block__file">
          <Icon name="file" className="as-button__icon" />
          <code>{name}</code>
          <InfoLine
            line={t("wizard.targets.fits.headerOnly")}
            details={<p>{t("wizard.targets.fits.headerOnlyDetails")}</p>}
          />
          <button
            type="button"
            className={buttonClassName("link")}
            onClick={onAnother}
          >
            {t("wizard.targets.fits.another")}
          </button>
        </div>
      )}
      {fits.kind === "reading" && (
        <Waiting label={t("wizard.targets.fits.reading")} size="inline" />
      )}
      {fits.kind === "error" && <Notice tone="error">{fits.message}</Notice>}
      {view &&
        (view.mute ? (
          <div className={noticeClassName("neutral")} role="status">
            <p>{view.heading}</p>
            {view.proof && <p className={helpTextClassName}>{view.proof}</p>}
          </div>
        ) : (
          <>
            <p className="as-targets-fits-block__heading">{view.heading}</p>
            {view.cards.length > 0 && (
              <ul className="as-target-cards">{view.cards.map(renderCard)}</ul>
            )}
            {view.proof && <p className={helpTextClassName}>{view.proof}</p>}
            {view.create && renderCreate(view.create)}
          </>
        ))}
    </div>
  );
}

/** « Nouvelle cible »: a small form without a map, its own
 * window, whose primary names its object. */
function NewTargetDialog({
  initial,
  web,
  onReadFits,
  onCancel,
  onRecord,
}: {
  initial: PersonalTargetDraft;
  web: string | null;
  onReadFits: (fill: (guess: FitsGuess) => void) => void;
  onCancel: () => void;
  /** The form is recorded as a row of the list; the target is made by
   * « Continuer ». */
  onRecord: (target: PersonalTargetCreate) => void;
}) {
  const { t, locale } = useI18n();
  const [draft, setDraft] = useState<PersonalTargetDraft>(initial);
  const checked = draftToCreate(draft);
  const set = (key: keyof PersonalTargetDraft) => (value: string) =>
    setDraft((current) => ({ ...current, [key]: value }));
  const coordinatesWrong =
    !checked.ok && checked.missing === "coordinates" && (draft.ra || draft.dec);

  return (
    <div className="as-dialog__scrim" role="presentation">
      <div
        className="as-dialog"
        role="dialog"
        aria-modal="true"
        aria-labelledby="new-target-title"
      >
        <h2 id="new-target-title" className="as-dialog__title">
          {t("wizard.targets.new")}
        </h2>
        <p className={helpTextClassName}>{t("wizard.targets.form.lead")}</p>
        <label className="as-dialog__field">
          <span className={fieldLabelClassName}>
            {t("wizard.targets.form.name")}
          </span>
          <input
            className={fieldClassName}
            autoFocus
            value={draft.name}
            maxLength={80}
            onChange={(event) => set("name")(event.target.value)}
          />
          <span className={helpTextClassName}>
            {t("wizard.targets.form.nameHelp")}
          </span>
        </label>
        <div className="as-dialog__field">
          <span className={fieldLabelClassName} id="new-target-label">
            {t("wizard.targets.form.label")}
          </span>
          <div
            className="as-target-labels"
            role="radiogroup"
            aria-labelledby="new-target-label"
          >
            {PERSONAL_LABELS.map((label) => (
              <button
                key={label.value}
                type="button"
                role="radio"
                aria-checked={draft.label === label.value}
                className={
                  draft.label === label.value
                    ? "as-target-label is-active"
                    : "as-target-label"
                }
                onClick={() => set("label")(label.value)}
              >
                {t(label.key)}
              </button>
            ))}
          </div>
        </div>
        <div className="as-dialog__grid as-dialog__grid--coordinates">
          <label className="as-dialog__field">
            <span className={fieldLabelClassName}>
              {t("wizard.targets.form.ra")}{" "}
              <span className={helpTextClassName}>
                {t("wizard.targets.form.optional")}
              </span>
            </span>
            <input
              className={fieldClassName}
              inputMode="decimal"
              value={draft.ra}
              onChange={(event) => set("ra")(event.target.value)}
            />
          </label>
          <label className="as-dialog__field">
            <span className={fieldLabelClassName}>
              {t("wizard.targets.form.dec")}{" "}
              <span className={helpTextClassName}>
                {t("wizard.targets.form.optional")}
              </span>
            </span>
            <input
              className={fieldClassName}
              inputMode="decimal"
              value={draft.dec}
              onChange={(event) => set("dec")(event.target.value)}
            />
          </label>
          <button
            type="button"
            className={`${buttonClassName("tertiary")} as-dialog__inline-action`}
            onClick={() =>
              onReadFits((guess) => {
                const filled = draftFromGuess(guess);
                setDraft((current) => ({
                  ...current,
                  name: current.name || filled.name,
                  ra: filled.ra || current.ra,
                  dec: filled.dec || current.dec,
                }));
              })
            }
          >
            <Icon name="file" className="as-button__icon" />
            {t("wizard.targets.form.readFits")}
          </button>
        </div>
        {coordinatesWrong ? (
          <Notice tone="warning">
            {t("wizard.targets.form.badCoordinates")}
          </Notice>
        ) : (
          <InfoLine
            line={t("wizard.targets.form.noPosition")}
            details={<p>{t("wizard.targets.form.noPositionDetails")}</p>}
          />
        )}
        <div className="as-targets-framing">
          <p className={helpTextClassName}>
            {t("wizard.targets.form.framing")}
          </p>
          <button
            type="button"
            className={buttonClassName("link")}
            disabled={!web}
            onClick={() => web && void openInBrowser(framingsPage(web, locale))}
          >
            <Icon name="external" className="as-button__icon" />
            {t("wizard.targets.framing.open")}
          </button>
        </div>
        <div className="as-actions">
          <button
            type="button"
            className={buttonClassName("link")}
            onClick={onCancel}
          >
            {t("common.cancel")}
          </button>
          {checked.ok ? (
            <button
              type="button"
              className={buttonClassName("primary")}
              onClick={() => onRecord(checked.target)}
            >
              {t("wizard.targets.form.submit")}
            </button>
          ) : (
            <button type="button" className={inactiveButtonClassName} disabled>
              {t("wizard.targets.form.submit")}
            </button>
          )}
        </div>
      </div>
    </div>
  );
}
