import { useEffect, useRef, useState } from "react";

import { Icon } from "../../components/Icon";
import { Waiting } from "../../components/Waiting";
import { useI18n } from "../../i18n";
import {
  nightAttachment,
  nightPlanChoices,
  nightPlanSearch,
  openInBrowser,
  type NightAttachment,
  type PlanChoice,
  type PlanChoices,
} from "../../lib/bridge";
import { windowStorage } from "../../lib/lastReport";
import type { NightAction, NightChoices } from "../../lib/nightRecap";
import {
  SEARCH_DELAY_MS,
  blockView,
  choiceView,
  createRows,
  listed,
  loadAttachment,
  loadChoices,
  saveAttachment,
  saveChoices,
  searchNone,
  searchReady,
  windowIntro,
} from "../../lib/planAttach";
import { isOfflineFailure } from "../../lib/targetSearch";
import {
  badgeClassName,
  buttonClassName,
  fieldClassName,
  fieldLabelClassName,
  helpTextClassName,
} from "../../theme";
import { offersPage } from "./TreatmentsStep";
import type { NightContext } from "./NightWizard";

/**
 * The « Plan multi-nuits » block of « Cibles »: a night in no multi-night
 * plan offers « Rattacher à un plan multi-nuits » (a tertiary, alone in its
 * zone), a night in one says it and offers the case « Détacher de ce plan »
 * (in the destructive tint, nothing is deleted). Without the right
 * (`analysis.progression`) or on a private server the link is replaced by
 * the sentence of « Traitements » and « Voir les offres ». A read that fails
 * says so with « Réessayer », and shows the last answer when there is one.
 *
 * Nothing is written here: the window of the
 * plans only **records** the plan chosen -- or the plan to create -- as a
 * row of the block, and the case of « Détacher » is a choice too;
 * « Continuer » applies them. The case of a row is its undo.
 */
export function PlanAttachBlock({
  context,
  recap,
  onRecap,
  reads,
  working,
  onOffers,
}: {
  context: NightContext;
  recap: NightChoices;
  onRecap: (action: NightAction) => void;
  /** Bumped when « Continuer » wrote: the plans are read again. */
  reads: number;
  /** « Continuer » is writing: nothing is changed meanwhile. */
  working: boolean;
  onOffers: () => void;
}) {
  const { t, locale } = useI18n();
  const sessionId = context.sessionId;
  const planId = context.session?.planId ?? null;
  const [attachment, setAttachment] = useState<NightAttachment | null>(null);
  const [failed, setFailed] = useState(false);
  const [open, setOpen] = useState(false);
  const [tries, setTries] = useState(0);
  const targets = (context.online?.targets ?? []).filter(
    (target) => target.targetId,
  );

  // The plans the night is in; read again at each opening of the step and
  // after « Continuer » wrote.
  useEffect(() => {
    let cancelled = false;
    setFailed(false);
    nightAttachment(sessionId, planId)
      .then((read) => {
        if (cancelled) return;
        setAttachment(read);
        saveAttachment(windowStorage(), sessionId, read);
      })
      .catch(() => {
        if (cancelled) return;
        setAttachment(null);
        setFailed(true);
      });
    return () => {
      cancelled = true;
    };
  }, [sessionId, planId, tries, reads]);

  const reload = () => setTries((current) => current + 1);

  const view = blockView(t, locale, {
    attachment,
    failed,
    cached: failed ? loadAttachment(windowStorage(), sessionId) : null,
  });

  const offers = () => {
    const page = offersPage(context, locale);
    if (page) void openInBrowser(page);
    else onOffers();
  };

  const pick = recap.plan;
  // The window is offered where attaching is open: a night in no plan, or in
  // one it may leave (a night of another plan is moved).
  const canAttach =
    (view.kind === "none" && view.attach) ||
    (view.kind === "attached" && view.detach);
  const attachButton = canAttach && (
    <div className="as-plan-attach__actions">
      <button
        type="button"
        className={buttonClassName("tertiary")}
        disabled={working}
        onClick={() => setOpen(true)}
      >
        <Icon name="link" className="as-button__icon" />
        {t("planAttach.attach")}
      </button>
    </div>
  );

  return (
    <section className="as-plan-attach" aria-labelledby="plan-attach-title">
      <h3 id="plan-attach-title" className="as-targets__subheading">
        {t("planAttach.title")}
      </h3>
      {view.kind === "loading" && (
        <Waiting label={t("planAttach.reading")} size="inline" />
      )}
      {view.kind === "failed" && (
        <div className="as-notice as-notice--neutral as-notice--inline">
          <span>{view.text}</span>
          <button
            type="button"
            className={buttonClassName("link")}
            onClick={reload}
          >
            {t("wizard.retry")}
          </button>
        </div>
      )}
      {view.kind === "none" && (
        <>
          <p className="as-plan-attach__line">{view.text}</p>
          {view.closed && (
            <ClosedNotice closed={view.closed} onOffers={offers} />
          )}
        </>
      )}
      {view.kind === "attached" && (
        <>
          {view.offline && (
            <div className="as-notice as-notice--neutral as-notice--inline">
              <span>{view.offline}</span>
              <button
                type="button"
                className={buttonClassName("link")}
                onClick={reload}
              >
                {t("wizard.retry")}
              </button>
            </div>
          )}
          <ul className="as-plan-attach__plans">
            {view.plans.map((plan) => {
              const leaving = recap.detach.some(
                (item) => item.projectId === plan.projectId,
              );
              return (
                <li key={plan.projectId} className="as-plan-attach__plan">
                  <span className="as-plan-attach__text">
                    <strong>
                      {t("planAttach.attachedTo", { title: plan.title })}
                    </strong>
                    <span className={helpTextClassName}>
                      {leaving ? t("planAttach.pending.detach") : plan.facts}
                    </span>
                  </span>
                  {view.detach && (
                    <label className="as-plan-attach__detach">
                      <input
                        type="checkbox"
                        checked={leaving}
                        disabled={working}
                        onChange={() =>
                          onRecap({
                            type: "tickDetach",
                            projectId: plan.projectId,
                            title: plan.title,
                            on: !leaving,
                          })
                        }
                      />
                      {t("planAttach.detach")}
                    </label>
                  )}
                </li>
              );
            })}
          </ul>
          {view.closed && (
            <ClosedNotice closed={view.closed} onOffers={offers} />
          )}
        </>
      )}
      {pick && (
        <div className="as-choice as-plan-pick" data-pending={pick.kind}>
          <label className="as-target-row__label">
            <input
              type="checkbox"
              checked={pick.ticked}
              disabled={working}
              onChange={() => onRecap({ type: "tickPlan", on: !pick.ticked })}
            />
            <span className="as-choice__text">
              <span>
                <strong>
                  {t("planAttach.attachedTo", { title: pick.title })}
                </strong>
                {pick.kind === "create" && (
                  <span className="as-badge as-badge--new">
                    {t("wizard.new")}
                  </span>
                )}
              </span>
              <span className="as-choice__line">
                {!pick.ticked
                  ? t("planAttach.pending.skipped")
                  : pick.kind === "create"
                    ? t("planAttach.createBody", { target: pick.title })
                    : pick.movesFromName
                      ? t("planAttach.moves", { name: pick.movesFromName })
                      : t("planAttach.pending.attach")}
              </span>
            </span>
          </label>
        </div>
      )}
      {view.kind !== "loading" && view.kind !== "failed" && attachButton}
      {open && (
        <PlanAttachDialog
          context={context}
          hasTarget={targets.length > 0}
          onClose={() => setOpen(false)}
          onPick={(next) => {
            onRecap({ type: "pickPlan", pick: next });
            setOpen(false);
          }}
        />
      )}
    </section>
  );
}

/** The sentence that stands for the link, and « Voir les offres ». */
function ClosedNotice({
  closed,
  onOffers,
}: {
  closed: { text: string; offers: boolean };
  onOffers: () => void;
}) {
  const { t } = useI18n();
  return (
    <div className="as-notice as-notice--neutral as-notice--inline">
      <span>{closed.text}</span>
      {closed.offers && (
        <button
          type="button"
          className={buttonClassName("link")}
          onClick={onOffers}
        >
          <Icon name="external" className="as-button__icon" />
          {t("wizard.offers")}
        </button>
      )}
    </div>
  );
}

/**
 * « Rattacher à un plan multi-nuits »: the plans of the night's target (the
 * night's current plan first), a search by the name of a target, « Créer un
 * plan multi-nuits » for each target of the night. A click on a plan -- or on
 * a creation -- **records** it as the plan of the block (a night of another
 * plan is moved, said under the plan); « Continuer » applies it; the window
 * has no primary. Offline, the plans read last time are shown, greyed.
 */
function PlanAttachDialog({
  context,
  hasTarget,
  onClose,
  onPick,
}: {
  context: NightContext;
  hasTarget: boolean;
  onClose: () => void;
  onPick: (pick: Extract<NightAction, { type: "pickPlan" }>["pick"]) => void;
}) {
  const { t, locale } = useI18n();
  const sessionId = context.sessionId;
  const [choices, setChoices] = useState<PlanChoices | null>(null);
  const [failure, setFailure] = useState<{ offline: boolean } | null>(null);
  const [stale, setStale] = useState<PlanChoices | null>(null);
  const [query, setQuery] = useState("");
  const [found, setFound] = useState<PlanChoice[] | null>(null);
  const [searching, setSearching] = useState(false);
  const [tries, setTries] = useState(0);
  const sequence = useRef(0);
  const targets = (context.online?.targets ?? []).filter(
    (target) => target.targetId,
  );
  const names = targets.map((target) => target.name ?? target.targetId ?? "");

  useEffect(() => {
    let cancelled = false;
    setFailure(null);
    nightPlanChoices(sessionId, hasTarget)
      .then((read) => {
        if (cancelled) return;
        setChoices(read);
        setStale(null);
        saveChoices(windowStorage(), sessionId, read);
      })
      .catch((caught) => {
        if (cancelled) return;
        setChoices(null);
        setStale(loadChoices(windowStorage(), sessionId));
        setFailure({ offline: isOfflineFailure(caught) });
      });
    return () => {
      cancelled = true;
    };
  }, [sessionId, hasTarget, tries]);

  // The search, after a pause; an answer that comes back after a newer
  // question is dropped.
  useEffect(() => {
    if (!searchReady(query) || failure) {
      sequence.current += 1;
      setFound(null);
      setSearching(false);
      return;
    }
    const asked = ++sequence.current;
    const timer = window.setTimeout(() => {
      setSearching(true);
      nightPlanSearch(sessionId, query)
        .then((plans) => {
          if (asked === sequence.current) setFound(plans);
        })
        .catch(() => {
          if (asked === sequence.current) setFound([]);
        })
        .finally(() => {
          if (asked === sequence.current) setSearching(false);
        });
    }, SEARCH_DELAY_MS);
    return () => window.clearTimeout(timer);
  }, [query, sessionId, failure]);

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  // Offline, the plans read last time; any other failure of a read shows
  // its phrase and nothing else.
  const offline = failure !== null && failure.offline && stale !== null;
  const shownChoices = choices ?? (offline ? stale : null);
  const plans = listed(shownChoices, found);

  const attach = (choice: PlanChoice) => {
    const view = choiceView(t, locale, choice);
    onPick({
      kind: "attach",
      projectId: choice.projectId,
      title: view.title,
      facts: view.facts,
      movesFromId: choice.state === "moves" ? choice.fromProjectId : null,
      movesFromName: choice.state === "moves" ? choice.fromName : null,
    });
  };

  const create = (row: ReturnType<typeof createRows>[number]) =>
    onPick({ kind: "create", target: row.target, title: row.target.name });

  const intro = windowIntro(t, shownChoices, names);
  const rows = createRows(t, targets);
  return (
    <div className="as-dialog__scrim" role="presentation">
      <div
        className="as-dialog as-plan-dialog"
        role="dialog"
        aria-modal="true"
        aria-labelledby="plan-attach-dialog-title"
      >
        <h2 id="plan-attach-dialog-title" className="as-dialog__title">
          {t("planAttach.attach")}
        </h2>
        {!shownChoices && !failure && (
          <Waiting label={t("planAttach.reading")} size="inline" />
        )}
        {failure && !offline && (
          <div className="as-notice as-notice--neutral as-notice--inline">
            <span>{t("planAttach.readFailed")}</span>
            <button
              type="button"
              className={buttonClassName("link")}
              onClick={() => setTries((current) => current + 1)}
            >
              {t("wizard.retry")}
            </button>
          </div>
        )}
        {failure && offline && (
          <div className="as-notice as-notice--neutral as-notice--inline">
            <span>{t("planAttach.offline.window")}</span>
            <button
              type="button"
              className={buttonClassName("link")}
              onClick={() => setTries((current) => current + 1)}
            >
              {t("wizard.retry")}
            </button>
          </div>
        )}
        {intro && <p className={helpTextClassName}>{intro}</p>}
        {(shownChoices || failure) && (
          <label className="as-dialog__field">
            <span className={fieldLabelClassName}>
              {t("planAttach.searchLabel")}
            </span>
            <span className="as-targets-search__box">
              <Icon name="search" className="as-targets-search__icon" />
              <input
                className={fieldClassName}
                type="search"
                disabled={failure !== null}
                placeholder={t("planAttach.searchPlaceholder")}
                value={query}
                onChange={(event) => setQuery(event.target.value)}
              />
            </span>
          </label>
        )}
        {searching && <Waiting label={t("planAttach.reading")} size="inline" />}
        {plans.length > 0 && (
          <ul className="as-plan-choices">
            {plans.map((choice) => {
              const view = choiceView(t, locale, choice);
              return (
                <li key={choice.projectId}>
                  <button
                    type="button"
                    className="as-plan-choice"
                    disabled={view.disabled || failure !== null}
                    onClick={() => attach(choice)}
                  >
                    <span className="as-plan-choice__head">
                      <strong>{view.title}</strong>
                      {view.tag && (
                        <span className={badgeClassName(view.tag.tone)}>
                          {view.tag.text}
                        </span>
                      )}
                    </span>
                    <span className="as-plan-choice__facts">{view.facts}</span>
                    {view.note && (
                      <span className="as-plan-choice__note">{view.note}</span>
                    )}
                  </button>
                </li>
              );
            })}
          </ul>
        )}
        {found !== null &&
          found.length === 0 &&
          !searching &&
          searchReady(query) && (
            <p className={helpTextClassName}>{searchNone(t, query)}</p>
          )}
        {rows.length > 0 && !failure && (
          <div className="as-plan-create">
            {rows.map((row) => (
              <div key={row.target.targetId} className="as-plan-create__row">
                <span className={helpTextClassName}>{row.body}</span>
                <button
                  type="button"
                  className={buttonClassName("tertiary")}
                  onClick={() => create(row)}
                >
                  <Icon name="plus" className="as-button__icon" />
                  {row.label}
                </button>
              </div>
            ))}
          </div>
        )}
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
