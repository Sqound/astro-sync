import { useEffect, useId, useState } from "react";

import { ContrastSlider } from "../../components/ContrastSlider";
import { FinishSliders } from "../../components/FinishSliders";
import { useCapabilities } from "../../components/CapabilitiesProvider";
import { Icon } from "../../components/Icon";
import { Notice } from "../../components/Notice";
import { PoseDiscard } from "../../components/PoseDiscard";
import { longitudeOf } from "../../lib/nights";
import { posesOf } from "../../lib/poseStats";
import {
  measuresLine,
  measuresPlan,
  type NightAction,
  type NightChoices,
} from "../../lib/nightRecap";
import type { PoseStatsControl } from "../../hooks/usePoseStats";
import { ContrastContext, RenderExtras } from "../../components/RenderExtras";
import { useI18n } from "../../i18n";
import {
  localSettingsGet,
  localSettingsSetColour,
  localSettingsSetContrast,
  localSettingsSetSoftStars,
  nightIntegrationPreview,
  nightPrecheck,
  openInBrowser,
  stackingFrame,
  type CapabilitiesState,
  type DetectionInput,
  type ImagePlan,
  type IntegrationPreview,
  type NightView,
  type PlanLine,
  type PlanSource,
  type PolishedEngine,
  type TargetFrame,
  type ToolRow,
  type Treatment,
  type TrustState,
} from "../../lib/bridge";
import { applyFrameToAll, withTargetFrame } from "../../lib/stackFrame";
import {
  DEFAULT_CONTRAST,
  contrastOf,
  type Contrast,
} from "../../lib/contrast";
import {
  DEFAULT_COLOUR,
  DEFAULT_SOFT_STARS,
  colourOf,
  softStarsOf,
  type Colour,
  type SoftStars,
} from "../../lib/finish";
import {
  chosenKeys,
  chosenSources,
  nightProjects,
  planChoicesOf,
  planOptionView,
  type PlanImageChoices,
  type PlanOptionView,
  type PlanSourceView,
} from "../../lib/multiNight";
import { windowStorage } from "../../lib/lastReport";
import { savePlanKept } from "../../lib/planKept";
import { resolveFrame } from "../../lib/planFraming";
import { usePlanFraming } from "../../hooks/usePlanFraming";
import { PlanFramingBlock } from "../../components/PlanFramingBlock";
import { withDeviceChoice, type DeviceSource } from "../../lib/deviceMaster";
import {
  masterEntryState,
  type MasterEntryState,
} from "../../lib/masterMeasure";
import { NO_PROCESSING_ID, applyToAll } from "../../lib/pipelineChoice";
import {
  applyFinishesToAll,
  finishLine,
  withTargetFinishes,
  type PipelineFinish,
} from "../../lib/finish";
import {
  treatmentsView,
  type CalibrationLine,
  type StackingCard,
  type TreatmentChoices,
  type TreatmentOption,
} from "../../lib/wizardTreatments";
import {
  badgeClassName,
  buttonClassName,
  helpTextClassName,
  noticeClassName,
} from "../../theme";
import type { NightContext } from "./NightWizard";
import { DeviceChoice } from "./DeviceMaster";
import { MasterMeasureEntry, type MeasureChoices } from "./MasterMeasure";
import { PipelineDialog, ToolPillView } from "./PipelineDialog";
import { Lines, Segs, StepFoot, StepWait } from "./parts";
import { useToolInstall } from "./useToolInstall";

/** The offers' page of the space on the web (« Voir les offres »). */
export function offersPage(
  context: NightContext,
  locale: string,
): string | null {
  const web = context.status?.space?.webUrl;
  return web ? `${web.replace(/\/$/, "")}/${locale}/offers` : null;
}

/**
 * « Traitements »: five ticked options, each with its tool and its condition.
 * The three options reserved to « Synchronisation avancée+ » are greyed without
 * `analysis.logs` / `analysis.progression` (one line and « Voir les offres »);
 * on a private server everything is open and no offer is named; a missing tool
 * greys its option with « Outil absent » and the link to Réglages ›
 * Dépendances.
 *
 * « Empilement à partir des
 * FITS » is one box with one card per target -- its tick, its band, its
 * pipelines, « Choisir les pipelines » (the window `PipelineDialog`), its
 * integration of the night --, then the slider of the contrast, shown only
 * when a target is ticked. « Image du plan multi-nuits » stays apart.
 */
export function TreatmentsStep({
  context,
  view,
  viewFailed,
  toolsReady,
  onRetry,
  input,
  capabilitiesState,
  privateServer,
  choices,
  onChoices,
  poses,
  recap,
  onRecap,
  working,
  applyError,
  onTools,
  onTrust,
  onOffers,
  onBack,
  onMeasure,
  onContinue,
}: {
  context: NightContext;
  view: NightView | null;
  viewFailed: boolean;
  /** The installed tools and the pipelines are read. */
  toolsReady: boolean;
  onRetry: () => void;
  input: DetectionInput | null;
  capabilitiesState: CapabilitiesState | null;
  privateServer: string | null;
  choices: TreatmentChoices;
  onChoices: (choices: TreatmentChoices) => void;
  /** « Mesure des poses »: the poses flagged are proposed out of the stacking. */
  poses: PoseStatsControl;
  /** What the person chose on this step and « Continuer » applies: the
   * masters to measure, the measures kept to forget. */
  recap: NightChoices;
  onRecap: (action: NightAction) => void;
  /** The words of what « Continuer » is writing, `null` when it is not. */
  working: string | null;
  /** Why the last « Continuer » stopped, `null` when it did not. */
  applyError: string | null;
  /** The tools read again after a download made here. */
  onTools: (tools: ToolRow[]) => void;
  /** The person answered « Lui faites-vous confiance ? » for a pipeline. */
  onTrust: (id: string, trust: TrustState) => void;
  onOffers: () => void;
  onBack: () => void;
  /** « Mesurer les poses »: back to « Fichiers », at the card of the target. */
  onMeasure: (targetId: string) => void;
  onContinue: () => void;
}) {
  const { t, locale } = useI18n();
  // « Vérifier à nouveau »: the offer read again (bought on the web since),
  // also on entering the step and when the window comes back to the front.
  const { refresh: refreshOffer } = useCapabilities();
  useEffect(() => {
    void refreshOffer();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  const [plans, setPlans] = useState<ImagePlan[]>([]);
  // « Intégration de la nuit »: each target's families in the night's
  // window, the frames left out, the master kept.
  const [previews, setPreviews] = useState<Record<string, IntegrationPreview>>(
    {},
  );
  /** The target whose window « Pipelines pour {cible} » is open. */
  const [choosing, setChoosing] = useState<string | null>(null);
  // « Cadre de l'empilement » of the window: what the last stacking of the
  // target measured, read when the window opens (`null` before the first).
  const [measuredFrame, setMeasuredFrame] = useState<{
    targetId: string;
    frame: TargetFrame | null;
  } | null>(null);
  const { progress, install, installAll } = useToolInstall(onTools);

  // « Finition par défaut »: the contrast, the colours and the stars every
  // pipeline of every target starts from (and the plan image takes), kept in
  // the local settings; a pipeline's own finish is set in its window. The
  // engine of the « Rendu soigné » is chosen in Réglages.
  const [contrast, setContrast] = useState<Contrast>(DEFAULT_CONTRAST);
  const [colour, setColour] = useState<Colour>(DEFAULT_COLOUR);
  const [softStars, setSoftStars] = useState<SoftStars>(DEFAULT_SOFT_STARS);
  const [engine, setEngine] = useState<PolishedEngine>("graxpert-ai");
  useEffect(() => {
    let cancelled = false;
    localSettingsGet()
      .then((settings) => {
        if (cancelled) return;
        setContrast(contrastOf(settings.contrast));
        setColour(colourOf(settings.colour));
        setSoftStars(softStarsOf(settings.softStars));
        setEngine(settings.polishedEngine ?? "graxpert-ai");
      })
      .catch(() => undefined);
    return () => {
      cancelled = true;
    };
  }, []);
  const changeContrast = (next: Contrast) => {
    const before = contrast;
    setContrast(next);
    localSettingsSetContrast(next).catch(() => setContrast(before));
  };
  const changeColour = (next: Colour) => {
    const before = colour;
    setColour(next);
    localSettingsSetColour(next).catch(() => setColour(before));
  };
  const changeSoftStars = (next: SoftStars) => {
    const before = softStars;
    setSoftStars(next);
    localSettingsSetSoftStars(next).catch(() => setSoftStars(before));
  };

  useEffect(() => {
    if (!input) return;
    let cancelled = false;
    nightPrecheck(context.sessionId, input, [])
      .then((precheck) => {
        if (!cancelled) setPlans(precheck.plans);
      })
      .catch(() => undefined);
    return () => {
      cancelled = true;
    };
  }, [context.sessionId, input]);

  const detection = view?.detection ?? null;
  useEffect(() => {
    if (!detection || !input) return;
    let cancelled = false;
    const targets = detection.targets.filter(
      (target) => target.inNight && target.lightFrames > 0 && target.folderName,
    );
    void Promise.allSettled(
      targets.map((target) =>
        nightIntegrationPreview(
          detection.folder,
          target.folderName as string,
          input,
        ).then((preview) => [target.targetId, preview] as const),
      ),
    ).then((answers) => {
      if (cancelled) return;
      setPreviews(
        Object.fromEntries(
          answers
            .filter((answer) => answer.status === "fulfilled")
            .map((answer) => answer.value),
        ),
      );
    });
    return () => {
      cancelled = true;
    };
  }, [detection, input]);

  // « Cadre de l'empilement »: read when the window of a target opens.
  const dialogFolder =
    detection?.targets.find((target) => target.targetId === choosing)
      ?.folderName ?? null;
  const nightFolder = detection?.folder ?? null;
  useEffect(() => {
    if (!choosing || !dialogFolder || !nightFolder) return;
    let cancelled = false;
    void stackingFrame(nightFolder, dialogFolder)
      .then((frame) => {
        if (!cancelled) setMeasuredFrame({ targetId: choosing, frame });
      })
      .catch(() => {
        if (!cancelled) setMeasuredFrame({ targetId: choosing, frame: null });
      });
    return () => {
      cancelled = true;
    };
  }, [choosing, dialogFolder, nightFolder]);

  if (!view || !toolsReady) {
    return (
      <StepWait
        label={t(toolsReady ? "wizard.wait.night" : "common.wait.tools")}
        failed={!view && viewFailed}
        onRetry={onRetry}
        onBack={onBack}
      />
    );
  }
  const variantsAllowed =
    (capabilitiesState?.capabilities?.["capture.variant_limit"] ?? 0) > 0;
  const finishDefault: PipelineFinish = { contrast, colour, softStars };
  const multi = context.multiNight;
  const result = treatmentsView(t, locale, {
    treatments: view.treatments,
    tools: context.tools,
    detection: view.detection,
    online: view.online,
    plans,
    variantsAllowed,
    privateServer,
    pipelines: context.pipelines,
    choices,
    previews,
    eveningDate: multi?.eveningDate ?? input?.eveningDate ?? null,
    engine,
    progress,
    finishDefault,
  });
  const offers = () => {
    const page = offersPage(context, locale);
    if (page) void openInBrowser(page);
    else onOffers();
  };
  const toggleOption = (treatment: Treatment) => {
    const unticked = new Set(choices.unticked);
    if (unticked.has(treatment)) unticked.delete(treatment);
    else unticked.add(treatment);
    onChoices({ ...choices, unticked });
  };
  // The multi-night plans of the night: the stacking becomes « Intégration
  // de la nuit », and « Image du plan multi-nuits » comes after it, one
  // per plan.
  const projects = nightProjects(multi, (targetId) =>
    (view.detection?.targets ?? []).some(
      (target) => target.targetId === targetId && target.inNight,
    ),
  );
  const planChoices = (projectId: string): PlanImageChoices =>
    planChoicesOf(choices.planImage, projectId, windowStorage());
  const setPlanChoices = (projectId: string, next: PlanImageChoices) => {
    // The source of each night and the frame stay with the plan.
    savePlanKept(windowStorage(), projectId, {
      frame: next.frame,
      sources: next.sources,
    });
    onChoices({
      ...choices,
      planImage: { ...(choices.planImage ?? {}), [projectId]: next },
    });
  };
  const planViews: PlanOptionView[] =
    multi && projects.length > 0
      ? projects.map(({ project, lines }) =>
          planOptionView(t, locale, {
            project,
            lines,
            gate: multi.gate,
            choices: planChoices(project.projectId),
            pipelines: context.pipelines,
            targetRenders: choices.targetRenders,
          }),
        )
      : [];
  const planTargets = new Set(projects.map(({ project }) => project.targetId));
  const stacking = result.options.find((o) => o.treatment === "stacking");
  const cards = stacking?.cards ?? [];
  const toggleTarget = (targetId: string) => {
    const stackTargets = new Set(choices.stackTargets);
    if (stackTargets.has(targetId)) stackTargets.delete(targetId);
    else stackTargets.add(targetId);
    onChoices({ ...choices, stackTargets });
  };
  // Leaving the step, the choice of each card is written as it is shown
  // (its default included): the summary stacks exactly that.
  const continueToSummary = () => {
    const targetRenders: Record<string, string[]> = {};
    for (const [key, value] of Object.entries(choices.targetRenders)) {
      targetRenders[key] = [...value];
    }
    for (const card of cards) targetRenders[card.targetId] = card.chosen;
    onChoices({ ...choices, targetRenders });
    onContinue();
  };
  const dialogCard = cards.find((card) => card.targetId === choosing) ?? null;
  const measure: MeasureChoices = {
    recap,
    onRecap,
    working: working !== null,
  };
  const line = measuresLine(measuresPlan(recap), t);

  return (
    <ContrastContext.Provider
      value={{
        contrast,
        onContrast: changeContrast,
        colour,
        onColour: changeColour,
        softStars,
        onSoftStars: changeSoftStars,
      }}
    >
      <div className="as-step">
        <h2 className="as-step__heading">{t("wizard.treatments.title")}</h2>
        <p className={helpTextClassName}>{t("wizard.treatments.lead")}</p>
        {result.privateServer && (
          <p className={helpTextClassName}>
            {t("wizard.treatments.privateServer", {
              url: result.privateServer,
            })}
          </p>
        )}
        {result.reservedLine && (
          <div className="as-option-note">
            <Icon name="lock" className="as-line__glyph" />
            <span>{t("wizard.treatments.reserved")}</span>
            <button
              type="button"
              className="as-button as-button--link"
              onClick={offers}
            >
              <Icon name="external" className="as-button__icon" />
              {t("wizard.offers")}
            </button>
            <button
              type="button"
              className="as-button as-button--link"
              onClick={() => void refreshOffer()}
            >
              <Icon name="refresh" className="as-button__icon" />
              {t("wizard.files.checkAgain")}
            </button>
          </div>
        )}
        <div className="as-options">
          {result.options.map((option) =>
            option.treatment === "stacking" ? (
              [
                <StackingOption
                  key="stacking"
                  option={
                    projects.length > 0
                      ? {
                          ...option,
                          titleKey: "wizard.treatments.integration.title",
                          description:
                            option.state === "toolMissing"
                              ? option.description
                              : t("wizard.treatments.integration.line"),
                        }
                      : option
                  }
                  planTargets={planTargets}
                  onToggle={() => toggleOption(option.treatment)}
                  onTarget={toggleTarget}
                  onChoose={setChoosing}
                  onInstall={(pill) =>
                    void install(
                      pill,
                      context.tools.find((row) => row.id === pill.id),
                    )
                  }
                  onInstallAll={(ids) => void installAll(ids)}
                  onDependencies={onOffers}
                  onFiles={onBack}
                  onMeasure={onMeasure}
                  sessionId={context.sessionId}
                  measure={measure}
                  measureState={masterEntryState({
                    capabilities: capabilitiesState?.capabilities,
                    tools: context.tools,
                  })}
                  onDevice={(targetId, label, source) =>
                    onChoices({
                      ...choices,
                      deviceUse: withDeviceChoice(
                        choices.deviceUse,
                        targetId,
                        label,
                        source,
                      ),
                    })
                  }
                  poses={poses}
                  place={{
                    name: context.detail?.night?.place?.value?.name ?? null,
                    longitude: longitudeOf(context.detail),
                  }}
                />,
                result.contrastShown ? (
                  <DefaultFinish
                    key="contrast"
                    finish={finishDefault}
                    onContrast={changeContrast}
                    onColour={changeColour}
                    onSoftStars={changeSoftStars}
                  />
                ) : null,
                ...planViews.map((plan) => (
                  <PlanImageOption
                    key={`plan:${plan.projectId}`}
                    plan={plan}
                    lines={
                      projects.find(
                        (item) => item.project.projectId === plan.projectId,
                      )?.lines ?? []
                    }
                    choices={planChoices(plan.projectId)}
                    onChoices={(next) => setPlanChoices(plan.projectId, next)}
                    onOffers={offers}
                    slider={!result.contrastShown}
                  />
                )),
              ]
            ) : (
              <Option
                key={option.treatment}
                option={option}
                onToggle={() => toggleOption(option.treatment)}
                onOffers={offers}
                onDependencies={onOffers}
              />
            ),
          )}
        </div>
        <MasterMeasureEntry
          context={context}
          detection={view.detection}
          capabilitiesState={capabilitiesState}
          measure={measure}
          onOffers={offers}
          onDependencies={onOffers}
        />
        <Notice tone="error">{applyError}</Notice>
        <StepFoot
          onBack={onBack}
          line={line}
          working={working}
          enabled={working === null}
          primary={t("wizard.treatments.continue")}
          onPrimary={continueToSummary}
        />
        {dialogCard && (
          <PipelineDialog
            key={dialogCard.targetId}
            target={{
              targetId: dialogCard.targetId,
              designation: dialogCard.designation,
              frames: dialogCard.frames,
              duration: dialogCard.duration,
              filter: dialogCard.filter,
              band: dialogCard.band,
            }}
            pipelines={context.pipelines}
            tools={context.tools}
            initial={dialogCard.chosen}
            finishDefault={finishDefault}
            initialFinishes={
              choices.targetFinishes?.[dialogCard.targetId] ?? {}
            }
            frame={{
              measured:
                measuredFrame?.targetId === dialogCard.targetId
                  ? measuredFrame.frame
                  : null,
              initial: choices.targetFrames?.[dialogCard.targetId],
              seconds: dialogCard.seconds,
            }}
            several={variantsAllowed}
            engine={engine}
            progress={progress}
            onInstall={(pill) =>
              void install(
                pill,
                context.tools.find((row) => row.id === pill.id),
              )
            }
            onTrust={onTrust}
            onCancel={() => setChoosing(null)}
            onValidate={(selection, finishes, frame) => {
              onChoices({
                ...choices,
                targetRenders: {
                  ...choices.targetRenders,
                  [dialogCard.targetId]: selection,
                },
                targetFinishes: withTargetFinishes(
                  choices.targetFinishes,
                  dialogCard.targetId,
                  finishes,
                  selection,
                  NO_PROCESSING_ID,
                ),
                targetFrames: withTargetFrame(
                  choices.targetFrames,
                  dialogCard.targetId,
                  frame,
                ),
              });
              setChoosing(null);
            }}
            onApplyAll={(selection, finishes, frame) => {
              const targets = [dialogCard.targetId, ...choices.stackTargets];
              onChoices({
                ...choices,
                targetRenders: applyToAll(
                  choices.targetRenders,
                  targets,
                  selection,
                ),
                // The finish of each pipeline ticked, by pipeline id.
                targetFinishes: applyFinishesToAll(
                  choices.targetFinishes,
                  targets,
                  finishes,
                  selection,
                  NO_PROCESSING_ID,
                ),
                // The frame by the rank of its notch, only when it was moved
                // by hand: Automatic stays Automatic.
                targetFrames: applyFrameToAll(
                  choices.targetFrames,
                  dialogCard.targetId,
                  targets,
                  frame,
                ),
              });
              setChoosing(null);
            }}
          />
        )}
      </div>
    </ContrastContext.Provider>
  );
}

/**
 * « Empilement à partir des FITS »: one box, one
 * card per target found in lights/ -- its tick in the same column for every
 * card, its name and band, « 40 poses · 2 h 00 · 1 famille », the pipelines
 * chosen as labels with a gold tick, « Choisir les pipelines » (tertiary: a
 * choice always exists; secondary when none does, the equal
 * cards staying tertiary), then, ticked, its
 * integration of the night and what becomes of its final image. Unticked:
 * greyed, its choice kept. Siril or Python missing: the option greyed as
 * before, the cards kept with the reason, each tool with its download and
 * « Tout installer ».
 */
function StackingOption({
  option,
  planTargets,
  onToggle,
  onTarget,
  onChoose,
  onInstall,
  onInstallAll,
  onDependencies,
  onFiles,
  onMeasure,
  sessionId,
  measure,
  measureState,
  onDevice,
  poses,
  place,
}: {
  option: TreatmentOption;
  planTargets: ReadonlySet<string>;
  onToggle: () => void;
  onTarget: (targetId: string) => void;
  onChoose: (targetId: string) => void;
  onInstall: (pill: StackingInstall) => void;
  onInstallAll: (ids: string[]) => void;
  onDependencies: () => void;
  onFiles: () => void;
  onMeasure: (targetId: string) => void;
  /** The night: « Mesurer cette image » records a measure on it. */
  sessionId: string;
  measure: MeasureChoices;
  measureState: MasterEntryState;
  /** The source a family takes: the device's image or a new stacking. */
  onDevice: (targetId: string, label: string, source: DeviceSource) => void;
  poses: PoseStatsControl;
  place: { name: string | null; longitude: number | null };
}) {
  const { t } = useI18n();
  const greyed = option.state !== "available";
  return (
    <section
      className={
        greyed ? "as-option as-stacking is-greyed" : "as-option as-stacking"
      }
    >
      <OptionHead option={option} onToggle={onToggle} />
      {option.state === "toolMissing" && (
        <div className="as-option__aside">
          <button
            type="button"
            className={buttonClassName("link")}
            onClick={onDependencies}
          >
            <Icon name="gear" className="as-button__icon" />
            {t("wizard.treatments.dependencies")}
          </button>
        </div>
      )}
      <p className="as-option__tool">
        <Segs segs={option.tool} />
      </p>
      <p className="as-option__line">{option.description}</p>
      {option.toInstall.length > 0 && (
        <div className="as-stacking__install">
          <span className="as-pipeline-card__label">
            {t("wizard.treatments.toolMissing")}
          </span>
          <span className="as-pills">
            {option.toInstall.map((pill) => (
              <ToolPillView key={pill.id} pill={pill} onInstall={onInstall} />
            ))}
          </span>
          <button
            type="button"
            className={buttonClassName("tertiary")}
            disabled={option.toInstall.every((pill) => !pill.action)}
            onClick={() =>
              onInstallAll(
                option.toInstall
                  .filter((pill) => pill.action && pill.action.kind !== "page")
                  .map((pill) => pill.id),
              )
            }
          >
            {t("deps.installAll")}
          </button>
        </div>
      )}
      {option.noFrames && (
        <div className="as-option__line as-stacking__none">
          <p>{t("wizard.stacking.noFrames")}</p>
          <button
            type="button"
            className={buttonClassName("link")}
            onClick={onFiles}
          >
            {t("wizard.stacking.backToFiles")}
          </button>
        </div>
      )}
      {option.cards.length > 0 && (
        <>
          <p className={`${helpTextClassName} as-option__line`}>
            {t("wizard.stacking.cardsLead")}
          </p>
          <div className="as-stack-cards">
            {option.cards.map((card) => (
              <TargetCard
                key={card.targetId}
                card={card}
                planTarget={planTargets.has(card.targetId)}
                onTick={() => onTarget(card.targetId)}
                onChoose={() => onChoose(card.targetId)}
                deviceBlock={
                  card.device ? (
                    <DeviceChoice
                      targetId={card.targetId}
                      designation={card.designation}
                      folderName={card.folderName}
                      sessionId={sessionId}
                      view={card.device}
                      measure={measure}
                      measureState={measureState}
                      onChoose={(label, source) =>
                        onDevice(card.targetId, label, source)
                      }
                    />
                  ) : null
                }
                poseBlock={
                  card.folderName && card.frames > 0 && card.restacksPoses ? (
                    <PoseDiscard
                      entry={posesOf(poses.poses, card.targetId)}
                      total={card.frames}
                      place={place}
                      onAll={(discard) =>
                        poses.discardAll(card.targetId, discard)
                      }
                      onOne={(path, discard) =>
                        poses.discardOne(card.targetId, path, discard)
                      }
                      onMeasure={() => onMeasure(card.targetId)}
                    />
                  ) : null
                }
              />
            ))}
          </div>
          {option.severalRenders && option.state === "available" && (
            <p className={`${helpTextClassName} as-option__line`}>
              {t("wizard.stacking.variants")}
            </p>
          )}
        </>
      )}
    </section>
  );
}

type StackingInstall = Parameters<typeof ToolPillView>[0]["pill"];

/**
 * « Finition par défaut », under the box « Empilement »: folded, one line
 * (its three settings, « Modifier ») and what it is -- the starting point of
 * every pipeline of every target, the finish of a pipeline being set in its
 * window --; unfolded, the three sliders of before. The plan image of a
 * multi-night plan takes it too.
 */
function DefaultFinish({
  finish,
  onContrast,
  onColour,
  onSoftStars,
}: {
  finish: PipelineFinish;
  onContrast: (next: Contrast) => void;
  onColour: (next: Colour) => void;
  onSoftStars: (next: SoftStars) => void;
}) {
  const { t, locale } = useI18n();
  const [unfolded, setUnfolded] = useState(false);
  const panel = useId();
  return (
    <section className="as-option as-contrast-card as-default-finish">
      <p className="as-default-finish__line">
        <strong className="as-default-finish__title">
          {t("wizard.finish.defaultTitle")}
        </strong>
        <span className="as-default-finish__summary">
          {finishLine(finish, t, locale)}
        </span>
        <button
          type="button"
          className={`${buttonClassName("link")} as-default-finish__edit`}
          aria-expanded={unfolded}
          aria-controls={panel}
          onClick={() => setUnfolded((open) => !open)}
        >
          {t(unfolded ? "wizard.finish.fold" : "wizard.finish.edit")}
        </button>
      </p>
      <p className={helpTextClassName}>{t("wizard.finish.defaultLead")}</p>
      {unfolded && (
        <div id={panel}>
          <ContrastSlider value={finish.contrast} onChange={onContrast} />
          <FinishSliders
            colour={finish.colour}
            softStars={finish.softStars}
            onColour={onColour}
            onSoftStars={onSoftStars}
          />
        </div>
      )}
    </section>
  );
}

/** One card of the box « Empilement »: a grid of three columns -- the
 * tick, the content, the button -- so that every tick stands in one column. */
function TargetCard({
  card,
  planTarget,
  onTick,
  onChoose,
  deviceBlock,
  poseBlock,
}: {
  card: StackingCard;
  planTarget: boolean;
  onTick: () => void;
  onChoose: () => void;
  /** The image the device stacked and the choice it gives. */
  deviceBlock: React.ReactNode;
  /** « Mesure des poses » of the target. */
  poseBlock: React.ReactNode;
}) {
  const { t } = useI18n();
  const off = !card.ticked;
  return (
    <div className={off ? "as-stack-card is-off" : "as-stack-card"}>
      <input
        type="checkbox"
        className="as-stack-card__tick"
        checked={card.ticked}
        disabled={!card.enabled}
        aria-label={card.designation}
        onChange={onTick}
      />
      <div className="as-stack-card__head">
        <p className="as-stack-card__name">
          <strong>{card.designation}</strong>
          <span
            className={
              card.band === "dualband" ? "as-tag as-tag--gold" : "as-tag"
            }
          >
            {card.bandLabel}
          </span>
        </p>
        <p className={helpTextClassName}>{card.line}</p>
      </div>
      <div className="as-stack-card__action">
        {card.button && (
          <button
            type="button"
            className={buttonClassName(card.button)}
            disabled={off}
            onClick={onChoose}
          >
            {t("wizard.stacking.choose")}
          </button>
        )}
      </div>
      <div className="as-stack-card__body">
        {card.noPipeline ? (
          <p className="as-stack-card__warning">
            <Icon name="warning" className="as-line__glyph" />
            {card.noPipeline}
          </p>
        ) : (
          <p className="as-picks">
            {card.chips.map((chip) => (
              <span key={chip.id} className="as-pick">
                <Icon name="check" className="as-pick__tick" />
                {chip.name}
                {chip.warning && (
                  <span className="as-pick__warning">
                    <Icon name="warning" />
                    {chip.warning}
                  </span>
                )}
              </span>
            ))}
          </p>
        )}
        {card.finishes.length > 0 && (
          <div className="as-stack-card__finishes">
            {card.finishes.map((line) => (
              <p key={line.pipeline}>
                <strong>{t("wizard.finish.targetModified")}</strong>
                {t("wizard.finish.targetLine", {
                  pipeline: line.pipeline,
                  finish: line.finish,
                })}
              </p>
            ))}
          </div>
        )}
        {card.frame && (
          <div className="as-stack-card__finishes">
            <p>{card.frame}</p>
          </div>
        )}
        {card.ticked && deviceBlock}
        <CalibrationLines lines={card.calibration} />
        {card.integration && (
          <div className="as-stack-card__integration">
            <p className="as-stack-card__legend">
              {t("wizard.treatments.integration.title")}
            </p>
            {card.integration.families.map((family) => (
              <div key={family.text}>
                <p>{family.text}</p>
                {family.master && (
                  <p className={helpTextClassName}>{family.master}</p>
                )}
              </div>
            ))}
            {card.integration.outside && (
              <p className="as-stack-card__warning">
                <Icon name="warning" className="as-line__glyph" />
                {card.integration.outside}
              </p>
            )}
          </div>
        )}
        {card.ticked && planTarget && (
          <p className={helpTextClassName}>{t("wizard.stacking.nightImage")}</p>
        )}
        {card.ticked && poseBlock}
        {card.notes.map((note, index) => (
          <p
            key={index}
            className={`as-stack-card__note as-stack-card__note--${note.tone}`}
          >
            {note.tone === "variant" && (
              <Icon name="variant" className="as-line__glyph" />
            )}
            <span>
              <Segs segs={note.segs} />
            </span>
          </p>
        ))}
      </div>
    </div>
  );
}

/** What the target's poses lack in calibration, before the stacking runs:
 * an amber block for the warnings, a grey line for an
 * information. No link: « Retour » of the step leads to « Fichiers ». */
function CalibrationLines({ lines }: { lines: readonly CalibrationLine[] }) {
  const warnings = lines.filter((line) => line.level === "warning");
  const informations = lines.filter((line) => line.level === "information");
  if (lines.length === 0) return null;
  return (
    <>
      {warnings.length > 0 && (
        <div className="as-stack-card__missing" data-calibration="warning">
          {warnings.map((line) => (
            <p key={line.text}>
              <Icon name="warning" className="as-line__glyph" />
              <span>{line.text}</span>
            </p>
          ))}
        </div>
      )}
      {informations.map((line) => (
        <p
          key={line.text}
          className={`${helpTextClassName} as-stack-card__info`}
          data-calibration="information"
        >
          <Icon name="info" className="as-line__glyph" />
          <span>{line.text}</span>
        </p>
      ))}
    </>
  );
}

function OptionHead({
  option,
  onToggle,
}: {
  option: TreatmentOption;
  onToggle: () => void;
}) {
  const { t } = useI18n();
  return (
    <label className="as-option__head">
      <input
        type="checkbox"
        checked={option.ticked}
        disabled={option.state !== "available"}
        onChange={onToggle}
      />
      <strong className="as-option__title">{t(option.titleKey)}</strong>
      {option.badge && (
        <span className={badgeClassName(option.badge.tone)}>
          {option.badge.icon && <Icon name={option.badge.icon} />}
          {option.badge.label}
        </span>
      )}
    </label>
  );
}

/** « Image du plan multi-nuits »: the plan's integrations by filter, the
 * total combined, the render; without the right, greyed with one line and
 * « Voir les offres » (a link); on a private server, no offer named. */
function PlanImageOption({
  plan,
  lines,
  choices,
  onChoices,
  onOffers,
  slider,
}: {
  plan: PlanOptionView;
  /** The plan's lines, for the frame on offer (read from the record of the
   * last image made with the same integrations). */
  lines: readonly PlanLine[];
  choices: PlanImageChoices;
  onChoices: (next: PlanImageChoices) => void;
  onOffers: () => void;
  /** The slider of the contrast here too, when the night's box does not
   * show it (no target ticked for the stacking). */
  slider: boolean;
}) {
  const { t } = useI18n();
  const [howTo, setHowTo] = useState(false);
  const open = plan.state === "available";
  const toggleLine = (key: string) => {
    const unticked = new Set(choices.unticked);
    if (unticked.has(key)) unticked.delete(key);
    else unticked.add(key);
    onChoices({ ...choices, unticked });
  };
  // The source of a night that has the image of its device and its own master.
  const takeSource = (key: string, source: PlanSource) =>
    onChoices({ ...choices, sources: { ...choices.sources, [key]: source } });
  // The frame on offer: read from the record of the last
  // image made with the same integrations; nothing before the first one.
  const chosen = chosenKeys(lines, choices);
  const framing = usePlanFraming({
    projectId: plan.projectId,
    lines,
    chosen,
    sources: chosenSources(lines, choices),
    enabled: open && plan.ticked && plan.pipelinesLine !== null,
  });
  const framed = resolveFrame(framing, choices.frame);
  return (
    <section
      className={
        open ? "as-option as-plan-image" : "as-option as-plan-image is-greyed"
      }
    >
      <label className="as-option__head">
        <input
          type="checkbox"
          checked={plan.ticked}
          disabled={!open}
          onChange={() => onChoices({ ...choices, off: !choices.off })}
        />
        <strong className="as-option__title">
          {t("multiNight.planImage.title")}
        </strong>
        {plan.state === "reserved" && (
          <span className="as-badge as-badge--neutral">
            <Icon name="lock" />
            {t("wizard.treatments.reservedBadge")}
          </span>
        )}
      </label>
      <p className="as-option__tool">
        {open
          ? `${t("multiNight.planImage.tool")} · ${t("wizard.treatments.toolPresent")}`
          : t("multiNight.planImage.tool")}
      </p>
      <p className="as-option__line">{plan.line}</p>
      <div className="as-plan-image__groups">
        {plan.groups.map((group) => (
          <div key={group.label} className="as-plan-image__group">
            <p className="as-plan-image__label">{group.label}</p>
            <ul className="as-sub-choices">
              {group.lines.map((line) => (
                <li
                  key={line.key}
                  className={
                    line.place === "otherComputer"
                      ? "is-elsewhere"
                      : line.source
                        ? "as-plan-image__sourced"
                        : undefined
                  }
                >
                  <label>
                    <input
                      type="checkbox"
                      checked={line.ticked}
                      disabled={line.disabled}
                      onChange={() => toggleLine(line.key)}
                    />
                    <span className="as-integration__row">
                      <span>
                        <Segs segs={line.segs} />
                      </span>
                      <span className="as-integration__time">
                        {line.duration}
                      </span>
                    </span>
                  </label>
                  {line.source && (
                    <SourceCell
                      source={line.source}
                      disabled={line.disabled}
                      onTake={(source) => takeSource(line.key, source)}
                    />
                  )}
                </li>
              ))}
            </ul>
          </div>
        ))}
      </div>
      {plan.combined && <p className="as-plan-image__total">{plan.combined}</p>}
      {plan.deviceWarning && (
        <p className="as-amber as-plan-image__indent">
          <Icon name="warning" />
          {plan.deviceWarning}
        </p>
      )}
      {plan.deviceWhy && (
        <p className="as-plan-image__device-why">{plan.deviceWhy}</p>
      )}
      {plan.notCombined && (
        <p className="as-sub-choices__note as-plan-image__indent">
          {plan.notCombined}
        </p>
      )}
      {plan.oneNight && <p className="as-plan-image__total">{plan.oneNight}</p>}
      {plan.otherComputer && (
        <div className="as-plan-image__indent">
          <p className="as-sub-choices__note">
            {plan.otherComputer.line}{" "}
            <button
              type="button"
              className="as-button as-button--link"
              aria-expanded={howTo}
              onClick={() => setHowTo((current) => !current)}
            >
              {t("multiNight.howToCombine")}
            </button>
          </p>
          {howTo && (
            <p className={noticeClassName("neutral")}>
              {plan.otherComputer.details}
            </p>
          )}
        </div>
      )}
      {plan.known && <p className="as-plan-image__total">{plan.known}</p>}
      {framing && framed && (
        <PlanFramingBlock
          framing={framing}
          choice={framed}
          onChoose={(notch) =>
            onChoices({
              ...choices,
              frame: notch.default
                ? null
                : {
                    top: notch.top,
                    left: notch.left,
                    height: notch.height,
                    width: notch.width,
                  },
            })
          }
        />
      )}
      {plan.pipelinesLine && (
        <div className="as-renders">
          <p>{plan.pipelinesLine}</p>
          <RenderExtras slider={slider} />
        </div>
      )}
      {open && (
        <p className="as-sub-choices__note as-plan-image__indent">
          {plan.whereItGoes}
        </p>
      )}
      {plan.closedLine && (
        <div className="as-option-note as-plan-image__indent">
          {plan.state === "reserved" && (
            <span className="as-badge as-badge--neutral">
              <Icon name="lock" />
              {t("wizard.treatments.reservedBadge")}
            </span>
          )}
          <span>{plan.closedLine}</span>
          {plan.offers && (
            <button
              type="button"
              className="as-button as-button--link"
              onClick={onOffers}
            >
              <Icon name="external" className="as-button__icon" />
              {t("wizard.offers")}
            </button>
          )}
        </div>
      )}
    </section>
  );
}

/** The source of a night in « Image du plan multi-nuits »:
 * two choices when the image of the device and the poses are both
 * here, a tag when there is only one source. */
function SourceCell({
  source,
  disabled,
  onTake,
}: {
  source: PlanSourceView;
  disabled: boolean;
  onTake: (source: PlanSource) => void;
}) {
  const { t } = useI18n();
  return (
    <div className="as-plan-source">
      {source.kind === "choice" ? (
        <div
          className="as-segments"
          role="radiogroup"
          aria-label={t("multiNight.source.label")}
        >
          {(["device", "master"] as const).map((item) => {
            const chosen = source.taken === item;
            return (
              <button
                key={item}
                type="button"
                role="radio"
                aria-checked={chosen}
                disabled={disabled}
                className={chosen ? "as-segment is-chosen" : "as-segment"}
                onClick={() => onTake(item)}
              >
                {t(
                  item === "device"
                    ? "multiNight.source.device"
                    : "multiNight.source.restack",
                )}
              </button>
            );
          })}
        </div>
      ) : (
        <span
          className={
            source.kind === "deviceOnly"
              ? badgeClassName("outline")
              : "as-badge as-badge--dotted"
          }
        >
          {t(
            source.kind === "deviceOnly"
              ? "multiNight.source.deviceOnly"
              : "multiNight.source.here",
          )}
        </span>
      )}
      <p className="as-plan-source__note">{source.note}</p>
    </div>
  );
}

function Option({
  option,
  onToggle,
  onOffers,
  onDependencies,
}: {
  option: TreatmentOption;
  onToggle: () => void;
  onOffers: () => void;
  onDependencies: () => void;
}) {
  const { t } = useI18n();
  const greyed = option.state !== "available";
  return (
    <section className={greyed ? "as-option is-greyed" : "as-option"}>
      <OptionHead option={option} onToggle={onToggle} />
      {option.state === "toolMissing" && (
        <div className="as-option__aside">
          <button
            type="button"
            className="as-button as-button--link"
            onClick={onDependencies}
          >
            <Icon name="gear" className="as-button__icon" />
            {t("wizard.treatments.dependencies")}
          </button>
        </div>
      )}
      {option.offersLink && (
        <div className="as-option__aside">
          <button
            type="button"
            className="as-button as-button--link"
            onClick={onOffers}
          >
            <Icon name="external" className="as-button__icon" />
            {t("wizard.offers")}
          </button>
        </div>
      )}
      <p className="as-option__tool">
        <Segs segs={option.tool} />
      </p>
      <p className="as-option__line">{option.description}</p>
      {option.lines.length > 0 && <Lines lines={option.lines} />}
    </section>
  );
}
