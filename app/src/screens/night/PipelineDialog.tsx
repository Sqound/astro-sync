import { useEffect, useId, useState } from "react";

import noProcessingPreview from "../../assets/no-processing-preview.jpg";
import { Icon } from "../../components/Icon";
import { PipelinesNote } from "../../components/PipelinesNote";
import { SnapSlider } from "../../components/SnapSlider";
import { StackFrameBlock } from "../../components/StackFrameBlock";
import { TrustDialog } from "../../components/TrustDialog";
import { useI18n } from "../../i18n";
import {
  pipelineTrustSet,
  type Pipeline,
  type PolishedEngine,
  type TargetFrame,
  type ToolRow,
  type TrustState,
} from "../../lib/bridge";
import { CONTRAST_SLIDER } from "../../lib/contrast";
import {
  COLOUR_SLIDER,
  SOFT_STARS_SLIDER,
  finishLine,
  type PipelineFinish,
} from "../../lib/finish";
import {
  NO_PROCESSING_ID,
  asksTrust,
  pipelineDialogView,
  toggleChoice,
  type DialogTarget,
  type PipelineCardView,
  type Progress,
  type ToolPill,
} from "../../lib/pipelineChoice";
import { frameChoiceOf, type FrameChoice } from "../../lib/stackFrame";
import {
  buttonClassName,
  helpTextClassName,
  inactiveButtonClassName,
} from "../../theme";

/**
 * « Pipelines pour {cible} »: opened by « Choisir les pipelines »
 * of a target's card. In its head, the note that the pipelines are a base
 * and its two links; then one card per pipeline -- tick (one
 * round button each when the space keeps one image), name, filter,
 * « Recommandé » or « Déconseillé pour ce filtre : … » (never refused),
 * « Voir plus » (the steps), its preview, the line « Finition » of a
 * pipeline ticked (folded; « Modifier » unfolds the three sliders of the
 * contrast, the colours and the stars, for this pipeline of this target),
 * the tools with their download and size, what it makes --, « Sans
 * traitement » last, never with a finish. Above the pipelines, « Cadre de
 * l'empilement » (`StackFrameBlock`): one frame for the target, for all its
 * pipelines. The list scrolls between the fixed head and foot.
 *
 * « Valider » is the only primary, greyed with the reason while nothing is
 * ticked; « Annuler » and « Appliquer à toutes les cibles » are links (cancel;
 * a choice copied elsewhere is not the action of the window);
 * « Télécharger (130 Mo) » is a tertiary in its pill (a useful action, not the
 * expected one); three levels.
 */
export function PipelineDialog({
  target,
  pipelines,
  tools,
  initial,
  finishDefault,
  initialFinishes,
  frame,
  several,
  engine,
  progress,
  onInstall,
  onTrust,
  onCancel,
  onValidate,
  onApplyAll,
}: {
  target: DialogTarget;
  pipelines: readonly Pipeline[];
  tools: readonly ToolRow[];
  initial: readonly string[];
  /** « Finition par défaut »: what a pipeline without its own finish takes. */
  finishDefault: PipelineFinish;
  /** The finishes this target's pipelines already have of their own. */
  initialFinishes: Readonly<Record<string, PipelineFinish>>;
  /** « Cadre de l'empilement »: what the last stacking of the target
   * measured (`null` before the first one) and the frame moved by hand;
   * absent: no block (the import stacks its nights for the first time,
   * Automatic). */
  frame?: {
    measured: TargetFrame | null;
    initial: FrameChoice | undefined;
    seconds: number | null;
  };
  several: boolean;
  engine: PolishedEngine;
  progress: Progress;
  onInstall: (pill: ToolPill) => void;
  /** The person answered « Lui faites-vous confiance ? » for a script. */
  onTrust: (id: string, trust: TrustState) => void;
  onCancel: () => void;
  /** The selection, the finishes of their own of the pipelines ticked, and
   * the frame moved by hand (`null`: Automatic). */
  onValidate: (
    selection: string[],
    finishes: Record<string, PipelineFinish>,
    frame: FrameChoice | null,
  ) => void;
  onApplyAll: (
    selection: string[],
    finishes: Record<string, PipelineFinish>,
    frame: FrameChoice | null,
  ) => void;
}) {
  const { t, locale } = useI18n();
  const titleId = useId();
  const [selection, setSelection] = useState<string[]>([...initial]);
  const [open, setOpen] = useState<ReadonlySet<string>>(() => new Set());
  // The finishes of their own, by pipeline: a pipeline not named follows the
  // default; one unticked keeps its finish while the window is open.
  const [finishes, setFinishes] = useState<Record<string, PipelineFinish>>(
    () => ({ ...initialFinishes }),
  );
  const [unfolded, setUnfolded] = useState<ReadonlySet<string>>(
    () => new Set(),
  );
  const [frameChoice, setFrameChoice] = useState<FrameChoice | null>(
    () => frame?.initial ?? null,
  );
  const [asking, setAsking] = useState<Pipeline | null>(null);
  const [answered, setAnswered] = useState<Record<string, TrustState>>({});
  const listed = pipelines.map((pipeline) =>
    answered[pipeline.id]
      ? { ...pipeline, trust: answered[pipeline.id] }
      : pipeline,
  );
  const view = pipelineDialogView(t, locale, {
    target,
    pipelines: listed,
    tools,
    selection,
    several,
    engine,
    progress,
  });

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape" && !asking) onCancel();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [asking, onCancel]);

  const toggle = (id: string) => {
    if (asksTrust(listed, id, selection)) {
      setAsking(listed.find((p) => p.id === id) ?? null);
      return;
    }
    setSelection((current) => toggleChoice(current, id, listed, several));
  };
  const flipIn =
    (set: typeof setOpen) =>
    (id: string): void =>
      set((current) => {
        const next = new Set(current);
        if (next.has(id)) next.delete(id);
        else next.add(id);
        return next;
      });
  const flip = flipIn(setOpen);
  const unfold = flipIn(setUnfolded);
  const setFinish = (id: string, finish: PipelineFinish | null) =>
    setFinishes((current) => {
      const next = { ...current };
      if (finish) next[id] = finish;
      else delete next[id];
      return next;
    });
  const ticked = (): Record<string, PipelineFinish> =>
    Object.fromEntries(
      Object.entries(finishes).filter(([id]) => selection.includes(id)),
    );

  return (
    <div className="as-dialog__scrim" role="presentation">
      <div
        className="as-dialog as-pipeline-dialog"
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
      >
        <div className="as-pipeline-dialog__head">
          <h2 id={titleId} className="as-dialog__title">
            {view.title}
          </h2>
          <p className={helpTextClassName}>{view.subtitle}</p>
          <PipelinesNote />
        </div>
        <div className="as-pipeline-dialog__list">
          {frame && (
            <StackFrameBlock
              measured={frame.measured}
              choice={frameChoice ?? undefined}
              seconds={frame.seconds}
              onChoose={(rank) =>
                setFrameChoice(frameChoiceOf(frame.measured, rank))
              }
            />
          )}
          {view.cards.map((card) => (
            <PipelineCard
              key={card.id}
              card={card}
              several={several}
              open={open.has(card.id)}
              name={`as-pipeline-choice-${target.targetId}`}
              onToggle={() => toggle(card.id)}
              onFlip={() => flip(card.id)}
              onInstall={onInstall}
              finish={
                card.ticked && card.id !== NO_PROCESSING_ID ? (
                  <FinishRow
                    pipelineId={card.id}
                    finish={finishes[card.id] ?? finishDefault}
                    own={card.id in finishes}
                    unfolded={unfolded.has(card.id)}
                    onUnfold={() => unfold(card.id)}
                    onChange={(next) => setFinish(card.id, next)}
                    onReset={() => setFinish(card.id, null)}
                  />
                ) : null
              }
            />
          ))}
        </div>
        <div className="as-pipeline-dialog__foot">
          {several && (
            <div className="as-pipeline-dialog__apply">
              <p className={helpTextClassName}>
                {t("wizard.pipelines.applyAllHelp")}
              </p>
              <button
                type="button"
                className={buttonClassName("link")}
                disabled={!view.valid}
                onClick={() => onApplyAll(selection, ticked(), frameChoice)}
              >
                {t("wizard.pipelines.applyAll")}
              </button>
            </div>
          )}
          <div className="as-pipeline-dialog__actions">
            <p
              className={
                view.valid
                  ? helpTextClassName
                  : `${helpTextClassName} as-pipeline-dialog__why`
              }
            >
              {!view.valid && (
                <Icon name="warning" className="as-line__glyph" />
              )}
              {view.count}
            </p>
            <button
              type="button"
              className={buttonClassName("link")}
              onClick={onCancel}
            >
              {t("common.cancel")}
            </button>
            {view.valid ? (
              <button
                type="button"
                className={buttonClassName("primary")}
                onClick={() => onValidate(selection, ticked(), frameChoice)}
              >
                {t("wizard.pipelines.validate")}
              </button>
            ) : (
              <button
                type="button"
                className={inactiveButtonClassName}
                disabled
              >
                {t("wizard.pipelines.validate")}
              </button>
            )}
          </div>
        </div>
      </div>
      {asking?.scriptSha256 && (
        <TrustDialog
          name={
            asking.displayName ??
            asking.name[locale] ??
            asking.name.en ??
            asking.id
          }
          sha256={asking.scriptSha256}
          changed={asking.trust === "changed"}
          onClose={() => setAsking(null)}
          onAnswer={(trusted) => {
            const id = asking.id;
            const sha256 = asking.scriptSha256 as string;
            void pipelineTrustSet(id, sha256, trusted)
              .then(() => {
                const trust: TrustState = trusted ? "trusted" : "refused";
                onTrust(id, trust);
                setAnswered((current) => ({ ...current, [id]: trust }));
                if (trusted) {
                  setSelection((current) =>
                    toggleChoice(
                      current,
                      id,
                      listed.map((p) => (p.id === id ? { ...p, trust } : p)),
                      several,
                    ),
                  );
                }
              })
              .catch(() => undefined)
              .finally(() => setAsking(null));
          }}
        />
      )}
    </div>
  );
}

function PipelineCard({
  card,
  several,
  open,
  name,
  onToggle,
  onFlip,
  onInstall,
  finish,
}: {
  card: PipelineCardView;
  several: boolean;
  open: boolean;
  name: string;
  onToggle: () => void;
  onFlip: () => void;
  onInstall: (pill: ToolPill) => void;
  /** The line « Finition » of a pipeline ticked; null otherwise. */
  finish: React.ReactNode;
}) {
  const { t } = useI18n();
  const panel = useId();
  const classes = [
    "as-pipeline-card",
    card.ticked ? "is-chosen" : "",
    card.disabled ? "is-greyed" : "",
  ]
    .filter(Boolean)
    .join(" ");
  return (
    <section className={classes}>
      <div className="as-pipeline-card__head">
        <label className="as-pipeline-card__pick">
          <input
            type={several ? "checkbox" : "radio"}
            name={name}
            checked={card.ticked}
            disabled={card.disabled}
            onChange={onToggle}
          />
          <strong>{card.name}</strong>
        </label>
        <span className="as-tag">{card.filterLabel}</span>
        {card.added && (
          <span className="as-tag as-tag--gold">
            {t("wizard.pipelines.added")}
          </span>
        )}
        {card.recommended && (
          <span className="as-tag as-tag--good">
            <Icon name="check" />
            {t("wizard.pipelines.recommended")}
          </span>
        )}
        {(card.steps.length > 0 || card.why.length > 0) && (
          <button
            type="button"
            className={`${buttonClassName("link")} as-pipeline-card__more`}
            aria-expanded={open}
            aria-controls={panel}
            onClick={onFlip}
          >
            {t(open ? "wizard.pipelines.less" : "wizard.pipelines.more")}
            <Icon
              name="chevron"
              className={open ? "as-how__chevron is-open" : "as-how__chevron"}
            />
          </button>
        )}
      </div>
      <p className="as-pipeline-card__text">{card.description}</p>
      {card.preview && (
        <img
          className="as-pipeline-card__preview"
          src={
            card.preview.kind === "image"
              ? card.preview.src
              : noProcessingPreview
          }
          alt={card.preview.alt}
          loading="lazy"
          draggable={false}
        />
      )}
      {card.discouraged && (
        <p className="as-pipeline-card__warning">
          <Icon name="warning" className="as-line__glyph" />
          {card.discouraged}
        </p>
      )}
      {card.fallback && (
        <p className="as-pipeline-card__warning">
          <Icon name="warning" className="as-line__glyph" />
          {card.fallback}
        </p>
      )}
      {card.note && (
        <p className={`${helpTextClassName} as-pipeline-card__note`}>
          {card.note}
        </p>
      )}
      {card.engine && (
        <p className={`${helpTextClassName} as-pipeline-card__note`}>
          {card.engine}
        </p>
      )}
      {finish}
      <div className="as-pipeline-card__tools">
        <span className="as-pipeline-card__label">
          {t("wizard.pipelines.tools")}
        </span>
        <span className="as-pills">
          {card.tools.map((pill) => (
            <ToolPillView key={pill.id} pill={pill} onInstall={onInstall} />
          ))}
        </span>
        <span className="as-pipeline-card__produces">{card.produces}</span>
      </div>
      {open && (
        <div id={panel} className="as-pipeline-card__details">
          {card.steps.length > 0 && (
            <>
              <p className="as-pipeline-card__label">
                {t("wizard.pipelines.stepsTitle")}
              </p>
              <ol className="as-pipeline__steps">
                {card.steps.map((step, index) => (
                  <li key={index}>
                    <strong>{step.title}</strong>
                    <span className={helpTextClassName}>{step.text}</span>
                  </li>
                ))}
              </ol>
            </>
          )}
          {card.why.length > 0 && (
            <>
              <p className="as-pipeline-card__label">
                {t("wizard.pipelines.whyTools")}
              </p>
              <ul className="as-pipeline-card__why">
                {card.why.map((item) => (
                  <li key={item.name}>
                    <strong>{item.name}</strong>
                    {item.detail && (
                      <span className={helpTextClassName}> {item.detail}</span>
                    )}
                    <span className={helpTextClassName}>{item.reason}</span>
                  </li>
                ))}
              </ul>
            </>
          )}
        </div>
      )}
    </section>
  );
}

/** One tool of a pipeline: present (a tick), missing (its button and size),
 * or downloading (its percent and a thin bar). */
export function ToolPillView({
  pill,
  onInstall,
}: {
  pill: ToolPill;
  onInstall: (pill: ToolPill) => void;
}) {
  const { t } = useI18n();
  return (
    <span className={`as-pill as-pill--${pill.state}`}>
      <Icon
        name={
          pill.state === "present"
            ? "check"
            : pill.state === "missing" && !pill.optional
              ? "warning"
              : "pending"
        }
        className="as-pill__glyph"
      />
      <span>{pill.name}</span>
      {pill.stateLabel && (
        <span className="as-pill__state">{pill.stateLabel}</span>
      )}
      {pill.state === "downloading" && (
        <span className="as-pill__progress">
          {t("wizard.pipelines.downloadingPill", {
            percent: String(pill.percent ?? 0),
          })}
          <span className="as-pill__bar" aria-hidden="true">
            <span style={{ width: `${pill.percent ?? 0}%` }} />
          </span>
        </span>
      )}
      {pill.action && (
        <button
          type="button"
          className={`${buttonClassName("tertiary")} as-pill__action`}
          onClick={() => onInstall(pill)}
        >
          {pill.action.label}
        </button>
      )}
    </span>
  );
}

/**
 * « Finition » of one pipeline ticked, under its preview: folded, one line
 * -- the three settings, « Par défaut » or « Modifiée », « Modifier » --;
 * unfolded, the three sliders with detents of « Traitements » (the same
 * components) for this pipeline of this target. Moving one freezes the
 * finish (« Modifiée »); « Rétablir le défaut » lets it follow the default
 * again.
 */
function FinishRow({
  pipelineId,
  finish,
  own,
  unfolded,
  onUnfold,
  onChange,
  onReset,
}: {
  pipelineId: string;
  finish: PipelineFinish;
  own: boolean;
  unfolded: boolean;
  onUnfold: () => void;
  onChange: (next: PipelineFinish) => void;
  onReset: () => void;
}) {
  const { t, locale } = useI18n();
  const panel = useId();
  return (
    <div className={unfolded ? "as-finish-row is-unfolded" : "as-finish-row"}>
      <p className="as-finish-row__line">
        <span className="as-pipeline-card__label">
          {t("wizard.finish.label")}
        </span>
        <span className="as-finish-row__summary">
          {finishLine(finish, t, locale)}
        </span>
        <span className={own ? "as-tag as-tag--gold" : "as-tag"}>
          {t(own ? "wizard.finish.modified" : "wizard.finish.default")}
        </span>
        <span className="as-finish-row__actions">
          {own && (
            <button
              type="button"
              className={buttonClassName("link")}
              onClick={onReset}
            >
              {t("wizard.finish.reset")}
            </button>
          )}
          <button
            type="button"
            className={buttonClassName("link")}
            aria-expanded={unfolded}
            aria-controls={panel}
            onClick={onUnfold}
          >
            {t(unfolded ? "wizard.finish.fold" : "wizard.finish.edit")}
          </button>
        </span>
      </p>
      {unfolded && (
        <div id={panel} className="as-finish-row__panel">
          <div className="as-finish-row__sliders">
            <SnapSlider
              title={t("wizard.treatments.contrast.title")}
              help={t("wizard.finish.contrast.help")}
              spec={CONTRAST_SLIDER}
              value={finish.contrast}
              onChange={(contrast) => onChange({ ...finish, contrast })}
            />
            <SnapSlider
              title={t("wizard.treatments.colour.title")}
              help={t(
                pipelineId === HOO_ID
                  ? "wizard.finish.colour.helpHoo"
                  : "wizard.finish.colour.help",
              )}
              spec={COLOUR_SLIDER}
              value={finish.colour}
              onChange={(colour) => onChange({ ...finish, colour })}
            />
            <SnapSlider
              title={t("wizard.treatments.softStars.title")}
              help={t("wizard.finish.stars.help")}
              spec={SOFT_STARS_SLIDER}
              value={finish.softStars}
              onChange={(softStars) => onChange({ ...finish, softStars })}
            />
          </div>
          <p className={helpTextClassName}>{t("wizard.finish.note")}</p>
        </div>
      )}
    </div>
  );
}

/** The HOO palette: its colour slider says what it does on the palette. */
const HOO_ID = "hoo";
