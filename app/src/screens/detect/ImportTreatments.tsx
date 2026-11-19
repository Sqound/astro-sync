import { useEffect, useMemo, useState } from "react";

import { useCapabilities } from "../../components/CapabilitiesProvider";
import { Icon } from "../../components/Icon";
import { useImport } from "../../components/ImportProvider";
import { Notice } from "../../components/Notice";
import { ProgressFrame } from "../../components/ProgressFrame";
import { offersOpener, useSpaceWeb } from "../../components/TabParts";
import {
  CardPipelines,
  PipelinesReadNotice,
} from "../../components/CardPipelines";
import { WizardFoot } from "../../components/WizardFrame";
import { useNow } from "../../hooks/useNow";
import { useI18n } from "../../i18n";
import {
  folderOpen,
  localSettingsGet,
  pipelinesList,
  toolsDetect,
  type Pipeline,
  type PolishedEngine,
  type ToolRow,
  type TrustState,
  workingFolderGet,
} from "../../lib/bridge";
import {
  folderToOpen,
  imageRoom,
  treatmentCards,
  treatmentCounts,
  type TreatmentTargetCard,
} from "../../lib/detectTreatments";
import {
  footRule,
  workingOutcome,
  type ImportState,
} from "../../lib/importFlow";
import { contrastOf } from "../../lib/contrast";
import {
  DEFAULT_FINISH,
  applyFinishesToAll,
  changedFinishes,
  colourOf,
  finishLine,
  softStarsOf,
  withTargetFinishes,
  type PipelineFinish,
} from "../../lib/finish";
import { NO_PROCESSING_ID, rendersOf } from "../../lib/pipelineChoice";
import {
  pipelinesActions,
  type PipelinesStatus,
} from "../../lib/pipelinesRead";
import { progressFrameView } from "../../lib/treatmentProgress";
import { reportOpenFailure } from "../../lib/openFailure";
import { pluralText } from "../../lib/written";
import { buttonClassName, helpTextClassName } from "../../theme";
import { PipelineDialog } from "../night/PipelineDialog";
import { useToolInstall } from "../night/useToolInstall";
import { secondaryLabel } from "./importWords";

type Translate = ReturnType<typeof useI18n>["t"];

/** The line under the title: « 3 nuits · 3 cibles · 176 poses ». */
export function treatmentsLine(
  t: Translate,
  locale: string,
  state: ImportState,
): string | null {
  const working = workingOutcome(state);
  if (!state.proposal || !working) return null;
  const counts = treatmentCounts(
    treatmentCards(state.proposal, working.outcome),
  );
  return [
    pluralText(t, locale, "detect.nights", counts.nights),
    pluralText(t, locale, "detect.scan.targets", counts.targets),
    pluralText(t, locale, "detect.treat.frames", counts.frames),
  ].join(t("detect.join"));
}

/** The pipelines of this computer, its tools and its polishing engine, and
 * the name of a pipeline as the person reads it. The pipelines and the tools
 * are read apart (the script of the stacking answers after a while, the tools
 * are asked for their version): the cards and the window of a target
 * take the pipelines as they come; the window says that the tools are still
 * read (`toolsRead`) until they are. */
export function usePipelines() {
  const { locale } = useI18n();
  const [pipelines, setPipelines] = useState<Pipeline[]>([]);
  const [tools, setTools] = useState<ToolRow[]>([]);
  const [engine, setEngine] = useState<PolishedEngine>("graxpert-ai");
  const [status, setStatus] = useState<PipelinesStatus>("loading");
  const [toolsRead, setToolsRead] = useState(false);
  const [attempt, setAttempt] = useState(0);
  useEffect(() => {
    let cancelled = false;
    setStatus((current) => (current === "ready" ? current : "loading"));
    setToolsRead(false);
    pipelinesList(locale)
      .then((list) => {
        if (cancelled) return;
        setPipelines(list.pipelines);
        setStatus("ready");
      })
      .catch(() => {
        if (!cancelled) setStatus("failed");
      });
    toolsDetect()
      .then((rows) => {
        if (!cancelled) setTools(rows);
      })
      .catch(() => undefined)
      .finally(() => {
        if (!cancelled) setToolsRead(true);
      });
    void localSettingsGet()
      .then((settings) => {
        if (!cancelled) setEngine(settings.polishedEngine ?? "graxpert-ai");
      })
      .catch(() => undefined);
    return () => {
      cancelled = true;
    };
  }, [locale, attempt]);
  const pipelineName = (id: string) => {
    const pipeline = pipelines.find((p) => p.id === id);
    return (
      pipeline?.displayName ?? pipeline?.name[locale] ?? pipeline?.name.en ?? id
    );
  };
  return {
    pipelines,
    setPipelines,
    tools,
    setTools,
    engine,
    pipelineName,
    status,
    /** The tools are read too: the window of a target may open. */
    toolsRead,
    /** « Réessayer »: read again. */
    reload: () => {
      setStatus("loading");
      setAttempt((current) => current + 1);
    },
  };
}

/**
 * Step 3 of the import, « Traitements »: one card per target over
 * every night created -- a target taken on several nights gives one image,
 * its nights stacked --, ticked when it has poses, its pipelines named on the
 * card with « Modifier » (`PipelineDialog`). The run belongs to the import:
 * the page may be left while it stacks, and while it runs the only way out of
 * the step is « Arrêter les traitements ».
 */
export function ImportTreatments() {
  const { t, locale } = useI18n();
  const { web } = useSpaceWeb();
  const imp = useImport();
  const { state } = imp;
  const offers = offersOpener(web, locale);
  const {
    pipelines,
    setPipelines,
    tools,
    setTools,
    engine,
    pipelineName,
    status,
    toolsRead,
    reload,
  } = usePipelines();
  // The cards are there at once; what each one will run is not known until
  // the pipelines are read, and nothing starts or opens on a list not read.
  // The tools are read apart: the window of a target says it itself while
  // they are asked for their version.
  const actions = pipelinesActions(status, pipelines.length);
  // The stacking is local and never gated by the offer: the capabilities
  // are read again on entering the step for the room of the space only.
  const { refresh } = useCapabilities();
  useEffect(() => {
    void refresh();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  const { progress, install } = useToolInstall(setTools);
  // « Finition par défaut », as « Traitements » of a night keeps it: what a
  // pipeline without its own finish takes.
  const [finishDefault, setFinishDefault] =
    useState<PipelineFinish>(DEFAULT_FINISH);
  useEffect(() => {
    let cancelled = false;
    localSettingsGet()
      .then((settings) => {
        if (cancelled) return;
        setFinishDefault({
          contrast: contrastOf(settings.contrast),
          colour: colourOf(settings.colour),
          softStars: softStarsOf(settings.softStars),
        });
      })
      .catch(() => undefined);
    return () => {
      cancelled = true;
    };
  }, []);
  const [dialog, setDialog] = useState<TreatmentTargetCard | null>(null);
  const proposal = state.proposal;
  // The sessions are not created yet: the cards are what would be.
  const local = workingOutcome(state);
  const cards = useMemo(
    () => (proposal && local ? treatmentCards(proposal, local.outcome) : []),
    [proposal, local],
  );
  const now = useNow(state.running !== null);
  if (!proposal || !local) return null;
  const room = imageRoom(state.storage);
  const several = room === null;
  const chosen = state.chosen ?? [];
  const running = state.running;
  const rule = footRule(state);
  const pipelinesOf = (card: TreatmentTargetCard) =>
    rendersOf(state.renders, card.targetId, card.band, pipelines, several);
  const roomText = room
    ? pluralText(t, locale, "detect.treat.room", room.remaining, {
        quota: room.quota,
      })
    : null;
  // The folder of the target being treated, once the treatments made it;
  // else the working folder, where every result ends.
  const runningCard = cards.find(
    (card) => card.designation === running?.target,
  );
  // The folder of the target when it is there, else the working folder:
  // the click always opens one, or says why none opened (a snackbar, the
  // path in it).
  const openFolder = () => {
    void (async () => {
      try {
        const root = (await workingFolderGet()).root;
        const folder = folderToOpen(
          state.folders,
          runningCard?.targetId ?? null,
          root,
        );
        if (folder)
          await folderOpen(folder, ...(folder === root ? [] : [root]));
      } catch (caught) {
        reportOpenFailure({ kind: "folder", target: "", caught });
      }
    })();
  };
  const launch = () =>
    imp.runTreatments(
      Object.fromEntries(
        cards
          .filter((card) => chosen.includes(card.targetId))
          .map((card) => [card.targetId, pipelinesOf(card)]),
      ),
    );

  return (
    <>
      <Notice tone="neutral" icon="info">
        <p>
          {roomText ? (
            <>
              {t("detect.treat.headRoomBefore")}
              <strong>{roomText}</strong>
              {t("detect.treat.headRoomAfter")}
            </>
          ) : (
            t("detect.treat.head")
          )}
        </p>
        <p>{t("detect.treat.slow")}</p>
        <p>{t("detect.treat.kept")}</p>
        <div className="as-treat__info-links">
          <button
            type="button"
            className={buttonClassName("link")}
            onClick={openFolder}
          >
            <Icon name="folder" className="as-button__icon" />
            {t("detect.treat.openFolder")}
          </button>
          {room && (
            <button
              type="button"
              className={buttonClassName("link")}
              onClick={offers}
            >
              <Icon name="external" className="as-button__icon" />
              {t("detect.offersWeb")}
            </button>
          )}
        </div>
      </Notice>
      {cards.length > 0 ? (
        <p className={helpTextClassName}>{t("import.treat.cardsLead")}</p>
      ) : (
        <p className={helpTextClassName}>{t("import.treat.none")}</p>
      )}
      {cards.length > 0 && (
        <PipelinesReadNotice zone={actions.zone} onRetry={reload} />
      )}
      <ul className="as-treat__cards" aria-busy={status === "loading"}>
        {cards.map((card) => {
          const on = chosen.includes(card.targetId);
          return (
            <li key={card.targetId} className="as-treat__card">
              <label>
                <input
                  type="checkbox"
                  checked={on}
                  disabled={running !== null}
                  onChange={() => imp.toggleTarget(card.targetId)}
                />
                <span className="as-treat__what">
                  <span>
                    <strong>{card.designation}</strong>{" "}
                    <span className={helpTextClassName}>
                      {[
                        t(`wizard.stacking.band.${card.band}`),
                        pluralText(
                          t,
                          locale,
                          "detect.nights",
                          card.nights.length,
                        ),
                        pluralText(
                          t,
                          locale,
                          "detect.treat.frames",
                          card.frames,
                        ),
                      ].join(t("detect.join"))}
                    </span>
                  </span>
                  {/* The renders chosen, by name: never a
                      bare « Choisir les pipelines ». */}
                  <CardPipelines
                    status={status}
                    names={pipelinesOf(card)
                      .map(pipelineName)
                      .join(t("detect.join"))}
                  />
                  {changedFinishes(
                    state.finishes,
                    card.targetId,
                    pipelinesOf(card),
                    finishDefault,
                    NO_PROCESSING_ID,
                  ).map(({ pipelineId, finish }) => (
                    <span key={pipelineId} className="as-treat__finish">
                      <strong>{t("wizard.finish.targetModified")}</strong>
                      {t("wizard.finish.targetLine", {
                        pipeline: pipelineName(pipelineId),
                        finish: finishLine(finish, t, locale),
                      })}
                    </span>
                  ))}
                </span>
              </label>
              <button
                type="button"
                className={buttonClassName("tertiary")}
                disabled={running !== null || !actions.edit}
                onClick={() => setDialog(card)}
              >
                {t("import.treat.edit")}
              </button>
            </li>
          );
        })}
      </ul>
      <Notice tone="error">{state.treatErrors.join(" ")}</Notice>
      {running && state.frame && (
        <div className="as-treat__running">
          <ProgressFrame
            view={progressFrameView(t, {
              label: running.target,
              state: state.frame,
              now,
            })}
            location={t("wizard.sending.local")}
            lastLineLabel={t("wizard.sending.lastLineLabel")}
          />
        </div>
      )}
      <WizardFoot
        message={
          !running &&
          chosen.length === 0 &&
          cards.length > 0 && (
            <p className={helpTextClassName}>{t("detect.treat.needOne")}</p>
          )
        }
        previous={rule.previous !== null ? () => imp.goTo(2) : null}
        secondary={
          rule.secondary && {
            label: secondaryLabel(t, rule.secondary),
            onClick:
              rule.secondary === "stopTreatments"
                ? imp.stopTreatments
                : imp.skipTreatments,
            className: "as-treat__secondary",
          }
        }
        primary={
          rule.primary && {
            label:
              chosen.length > 0
                ? pluralText(t, locale, "detect.treat.start", chosen.length)
                : t("detect.treat.startNone"),
            onClick: launch,
            enabled: rule.primary.enabled && actions.launch,
            className: "as-treat__primary",
          }
        }
      />
      {dialog && (
        <PipelineDialog
          key={dialog.targetId}
          target={{
            targetId: dialog.targetId,
            designation: dialog.designation,
            frames: dialog.frames,
            duration: null,
            filter: dialog.filters[0] ?? null,
            band: dialog.band,
          }}
          pipelines={pipelines}
          tools={tools}
          toolsRead={toolsRead}
          initial={pipelinesOf(dialog)}
          finishDefault={finishDefault}
          initialFinishes={state.finishes?.[dialog.targetId] ?? {}}
          several={several}
          engine={engine}
          progress={progress}
          onInstall={(pill) =>
            void install(
              pill,
              tools.find((row) => row.id === pill.id),
            )
          }
          onTrust={(id: string, trust: TrustState) =>
            setPipelines((held) =>
              held.map((p) => (p.id === id ? { ...p, trust } : p)),
            )
          }
          onCancel={() => setDialog(null)}
          onValidate={(selection, finishes) => {
            imp.setRenders({ ...state.renders, [dialog.targetId]: selection });
            imp.setFinishes(
              withTargetFinishes(
                state.finishes,
                dialog.targetId,
                finishes,
                selection,
                NO_PROCESSING_ID,
              ),
            );
            setDialog(null);
          }}
          onApplyAll={(selection, finishes) => {
            imp.setRenders(
              Object.fromEntries(
                cards.map((card) => [card.targetId, selection]),
              ),
            );
            imp.setFinishes(
              applyFinishesToAll(
                state.finishes,
                cards.map((card) => card.targetId),
                finishes,
                selection,
                NO_PROCESSING_ID,
              ),
            );
            setDialog(null);
          }}
        />
      )}
    </>
  );
}
