import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import { Icon } from "../../components/Icon";
import { ProgressFrame } from "../../components/ProgressFrame";
import { Waiting } from "../../components/Waiting";
import { InfoLine } from "../../components/InfoLine";
import { JoinTiffBox } from "../../components/JoinTiffBox";
import { Notice } from "../../components/Notice";
import { PaymentFailedNotice } from "../../components/PaymentFailedNotice";
import type { DraftStore } from "../../hooks/useIncidents";
import { useNow } from "../../hooks/useNow";
import { useI18n } from "../../i18n";
import {
  nightEnqueue,
  nightIntegrationsSend,
  nightPrecheck,
  nightRecordSending,
  nightSendLogs,
  nightSolve,
  localSettingsGet,
  localWorkCancelKind,
  masterMeasureCurrent,
  onLocalWork,
  onProgression,
  onPlanImage,
  onUploadEvent,
  openInBrowser,
  planImageRun,
  progressionFamilies,
  progressionImagesCandidates,
  progressionImagesRun,
  queueStatus,
  onStacking,
  stackingRun,
  uploadAll,
  uploadPause,
  type DetectionInput,
  type ImageCandidate,
  type ImageQuality,
  type NightDetection,
  type NightFolderOutcome,
  type NightPrecheck,
  type NightView,
  type PlanFraming,
  type PlanImageRun,
  type Treatment,
  planFramingRead,
} from "../../lib/bridge";
import {
  chosenKeys,
  chosenSources,
  multiNightSummary,
  nightMastersRow,
  nightProjects,
  planChoicesOf,
  planImageWanted,
  planPipelines,
  sentMultiNightLines,
} from "../../lib/multiNight";
import { windowStorage } from "../../lib/lastReport";
import { frameToSend, summaryFrame } from "../../lib/planFraming";
import {
  deviceUses,
  hasDeviceImage,
  usedDeviceImages,
  NO_DEVICE_CHOICES,
} from "../../lib/deviceMaster";
import { formatError } from "../../lib/format";
import type { IncidentsPlan } from "../../lib/incidentsDraft";
import { applyIncidents } from "../../lib/nightApply";
import {
  incidentsLine,
  reportLines,
  type ReportItem,
} from "../../lib/nightRecap";
import {
  eveningOfSession,
  longitudeOf,
  nightCell,
  nightSentence,
  oneFinalStop,
} from "../../lib/nights";
import { sendNight, type ProgressionResult } from "../../lib/sendNight";
import { NO_PROCESSING_ID, rendersOf } from "../../lib/pipelineChoice";
import { pipelinesToRun } from "../../lib/finish";
import { discardedForRun, posesOf, type NightPoses } from "../../lib/poseStats";
import {
  tickedTreatmentsOf,
  type TreatmentChoices,
} from "../../lib/wizardTreatments";
import {
  itemStateText,
  overallText,
  sendingItems,
  sentLines,
  summaryView,
  type SendingItem,
} from "../../lib/wizardSummary";
import {
  beginProgress,
  finishProgress,
  isStackingCancelled,
  progressFrameView,
  stackingCancelledError,
  startUnit,
  withProgressionEvent,
  withStackEvent,
  withWorkProgress,
  type Step,
  type TreatmentKey,
  type TreatmentProgress,
} from "../../lib/treatmentProgress";
import type { Phase } from "../../lib/wizardSteps";
import { formatSize, pluralText } from "../../lib/written";
import type { ScreenId } from "../../lib/menu";
import { frameToRun } from "../../lib/stackFrame";
import { helpTextClassName, progressClassName } from "../../theme";
import { nightPage, type NightContext } from "./NightWizard";
import { IncidentsPanel } from "./IncidentsPanel";
import { NIGHT_APPLY_DEPS, failureText } from "./applyDeps";
import { Segs, StepFoot } from "./parts";
import { offersPage } from "./TreatmentsStep";

const isTreatmentKey = (key: string): key is TreatmentKey =>
  key === "stacking" ||
  key === "progression" ||
  key === "logs" ||
  key === "sky" ||
  key === "planImage";

/** The treatments ticked and available on this night
 * (`wizardTreatments.tickedTreatmentsOf`: « Journaux » ticked by default
 * when the night holds PHD2 or ASIAIR logs). */
export function tickedTreatments(
  view: NightView | null,
  choices: TreatmentChoices,
): Set<Treatment> {
  return tickedTreatmentsOf(
    view?.treatments ?? [],
    view?.detection ?? null,
    view?.online ?? null,
    choices,
  );
}

/**
 * « Récapitulatif » (14, 18), the sending (15) and « Nuit envoyée » (16):
 * what will be sent, kept online, kept on this computer; the four limits
 * checked before anything is sent (« Envoi impossible : rien n'est
 * envoyé » when one is passed); the sending of the results only; the
 * night sent.
 */
export function SummaryStep({
  context,
  view,
  input,
  folder,
  phase,
  privateServer,
  choices,
  poses,
  incidents,
  report,
  onReport,
  onPhase,
  onMeasure,
  onBack,
  onNights,
  onOpen,
}: {
  context: NightContext;
  view: NightView | null;
  input: DetectionInput;
  folder: NightFolderOutcome | null;
  phase: Phase;
  privateServer: string | null;
  choices: TreatmentChoices;
  /** « Mesure des poses »: the poses flagged and left out of the stacking. */
  poses: NightPoses;
  /** The choices on the list of incidents, kept by the wizard: « Envoyer »
   * adds the ticked ones to the session. */
  incidents: DraftStore;
  /** What the primaries of the steps did on the way, for « Nuit envoyée ». */
  report: readonly ReportItem[];
  onReport: (item: Extract<ReportItem, { kind: "incidents" }>) => void;
  onPhase: (phase: Phase) => void;
  /** « Mesurer les poses »: back to « Fichiers », at the card of the target. */
  onMeasure: (targetId: string | null) => void;
  onBack: () => void;
  onNights: () => void;
  onOpen: (screen: ScreenId) => void;
}) {
  const { t, locale } = useI18n();
  const [precheck, setPrecheck] = useState<NightPrecheck | null>(null);
  // « Joindre aussi les TIFF »: the renders leave as PNG unless the person
  // joins their TIFF to this sending. It starts from « Envoi des rendus » of
  // Réglages (`null` until read) and never changes it.
  const [includeTiff, setIncludeTiff] = useState<boolean | null>(null);
  useEffect(() => {
    let gone = false;
    localSettingsGet()
      .then((kept) => !gone && setIncludeTiff(kept.rendersUpload === "both"))
      .catch(() => !gone && setIncludeTiff(false));
    return () => {
      gone = true;
    };
  }, []);
  const [error, setError] = useState<string | null>(null);
  // What « Envoyer » adds to the session before it sends: the incidents
  // ticked, once their list is read.
  const [incidentsPlan, setIncidentsPlan] = useState<{
    plan: IncidentsPlan | null;
    loading: boolean;
  }>({ plan: null, loading: false });
  const onPlan = useCallback(
    (plan: IncidentsPlan, loading: boolean) =>
      setIncidentsPlan({ plan, loading }),
    [],
  );
  const [adding, setAdding] = useState(false);
  const [applyError, setApplyError] = useState<string | null>(null);
  const [progress, setProgress] = useState<
    Record<string, Partial<SendingItem>>
  >({});
  const [bytes, setBytes] = useState({ sent: 0, total: 0 });
  // What each treatment that runs here says about itself.
  const [frames, setFrames] = useState<
    Partial<Record<TreatmentKey, TreatmentProgress>>
  >({});
  // « Arrêter l'empilement » was clicked: the stacking of the targets still
  // to come is not started, and the summary says it was stopped.
  const stackingStopped = useRef(false);
  const [stopped, setStopped] = useState(false);
  const now = useNow(phase === "sending");
  const [outcome, setOutcome] = useState<{
    solved: number;
    progression: number;
    notes: string[];
    failed: Treatment[];
    /** The multi-night plans: what was sent, said. */
    multiNight: string[];
  }>({ solved: 0, progression: 0, notes: [], failed: [], multiNight: [] });
  const running = useRef(false);
  const treatments = tickedTreatments(view, choices);
  // A resumed night: sent before, files added to its folder since.
  const resume =
    view?.resume.status === "to_complete" &&
    view.resume.missing.some((item) => item.key === "logsAdded");
  const detection = view?.detection ?? null;
  // The multi-night plans of the night: their lines and the image of each
  // plan leave after the night's own images.
  const multi = context.multiNight;
  const projects = nightProjects(multi, (targetId) =>
    (detection?.targets ?? []).some(
      (target) => target.targetId === targetId && target.inNight,
    ),
  );
  const planChoices = (projectId: string) =>
    planChoicesOf(choices.planImage, projectId, windowStorage());
  const plansToMake = multi
    ? projects.filter(({ project, lines }) =>
        planImageWanted(multi.gate, lines, planChoices(project.projectId)),
      )
    : [];
  const eveningDate = multi?.eveningDate ?? input.eveningDate ?? "";
  // The frame kept for the image of each plan: read from
  // the record of the last image made with the same integrations.
  const [framings, setFramings] = useState<Record<string, PlanFraming | null>>(
    {},
  );
  const framingKey = JSON.stringify(
    plansToMake.map(({ project, lines }) => [
      project.projectId,
      chosenKeys(lines, planChoices(project.projectId)),
      chosenSources(lines, planChoices(project.projectId)),
    ]),
  );
  useEffect(() => {
    let gone = false;
    for (const { project, lines } of plansToMake) {
      const choice = planChoices(project.projectId);
      void planFramingRead({
        projectId: project.projectId,
        lines,
        chosen: chosenKeys(lines, choice),
        sources: chosenSources(lines, choice),
      })
        .then(
          (found) =>
            !gone &&
            setFramings((held) => ({ ...held, [project.projectId]: found })),
        )
        .catch(() => undefined);
    }
    return () => {
      gone = true;
    };
    // The key stands for the plans and their choices.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [framingKey]);

  // The targets about to be stacked again: what an earlier stacking left in
  // `stacked/` is replaced by it, not counted.
  const restackKey = treatments.has("stacking")
    ? [...choices.stackTargets].sort().join("\n")
    : "";
  const restack = useMemo(
    () => (restackKey ? restackKey.split("\n") : []),
    [restackKey],
  );

  useEffect(() => {
    if (phase !== "recapitulatif" || includeTiff === null) return;
    let cancelled = false;
    nightPrecheck(context.sessionId, input, [], locale, restack, includeTiff)
      .then((next) => {
        if (!cancelled) setPrecheck(next);
      })
      .catch((caught) => {
        if (!cancelled) setError(formatError(t, caught));
      });
    return () => {
      cancelled = true;
    };
  }, [phase, context.sessionId, input, t, locale, restack, includeTiff]);

  // The figures the person measured on the image of a device (« Mesurer cette
  // image »), by target: the render of that image leaves with them.
  const [deviceQuality, setDeviceQuality] = useState<
    Record<string, ImageQuality>
  >({});
  const deviceUse = choices.deviceUse ?? NO_DEVICE_CHOICES;
  useEffect(() => {
    if (phase !== "recapitulatif" || !detection) return;
    let cancelled = false;
    const targets = detection.targets.filter(
      (target) =>
        target.inNight &&
        target.folderName &&
        choices.stackTargets.has(target.targetId) &&
        usedDeviceImages(target, deviceUse).length > 0,
    );
    void Promise.allSettled(
      targets.map(async (target) => {
        const kept = await masterMeasureCurrent(
          context.sessionId,
          target.folderName as string,
        );
        const used = usedDeviceImages(target, deviceUse);
        return kept?.result === "measured" &&
          used.some((image) => image.fileName === kept.master)
          ? ([target.targetId, kept.quality] as const)
          : null;
      }),
    ).then((answers) => {
      if (cancelled) return;
      setDeviceQuality(
        Object.fromEntries(
          answers.flatMap((answer) =>
            answer.status === "fulfilled" && answer.value ? [answer.value] : [],
          ),
        ),
      );
    });
    return () => {
      cancelled = true;
    };
  }, [phase, detection, choices.stackTargets, deviceUse, context.sessionId]);

  const mark = useCallback((key: string, item: Partial<SendingItem>) => {
    setProgress((current) => ({
      ...current,
      [key]: { ...current[key], ...item },
    }));
  }, []);

  const startedAt = useRef<Partial<Record<TreatmentKey, number>>>({});
  const beginFrame = useCallback(
    (key: TreatmentKey, step?: Step, units?: readonly number[]) => {
      const at = Date.now();
      startedAt.current[key] = at;
      setFrames((current) => ({
        ...current,
        [key]: beginProgress(at, step, units),
      }));
    },
    [],
  );
  const updateFrame = useCallback(
    (
      key: TreatmentKey,
      change: (state: TreatmentProgress) => TreatmentProgress,
    ) => {
      setFrames((current) => {
        const state = current[key];
        return state ? { ...current, [key]: change(state) } : current;
      });
    },
    [],
  );
  /** A treatment ends: the line says « fait » and how long it took. */
  const endFrame = useCallback(
    (key: TreatmentKey, item: Partial<SendingItem>) => {
      const finishedAt = Date.now();
      setFrames((current) => {
        const state = current[key];
        return state
          ? { ...current, [key]: finishProgress(state, finishedAt) }
          : current;
      });
      mark(key, {
        state: "done",
        localDone: true,
        durationMs: finishedAt - (startedAt.current[key] ?? finishedAt),
        ...item,
      });
    },
    [mark],
  );

  // The progression images and the report of the logs say how far they are.
  useEffect(() => {
    if (phase !== "sending") return;
    let stop = false;
    const unlisten: Array<() => void> = [];
    const keep = (pending: Promise<() => void>) =>
      pending
        .then((off) => (stop ? off() : unlisten.push(off)))
        .catch(() => undefined);
    keep(
      onProgression((event) =>
        updateFrame("progression", (state) =>
          withProgressionEvent(state, event),
        ),
      ),
    );
    keep(
      onLocalWork((progress) =>
        updateFrame("logs", (state) =>
          withWorkProgress(state, progress, Date.now()),
        ),
      ),
    );
    keep(
      onPlanImage((event) =>
        updateFrame("planImage", (state) => withStackEvent(state, event)),
      ),
    );
    return () => {
      stop = true;
      for (const off of unlisten) off();
    };
  }, [phase, updateFrame]);

  // The queue moves the bytes of each image as it sends them.
  useEffect(() => {
    if (phase !== "sending") return;
    let stop: (() => void) | undefined;
    let stopped = false;
    const refresh = () =>
      queueStatus()
        .then((status) => {
          const mine = status.entries.filter(
            (entry) => entry.target.sessionId === context.sessionId,
          );
          setBytes({
            sent: mine.reduce(
              (sum, entry) =>
                sum + (entry.state === "completed" ? entry.size : entry.offset),
              0,
            ),
            total: mine.reduce((sum, entry) => sum + entry.size, 0),
          });
          for (const entry of mine) {
            mark(`image:${entry.sha256}`, {
              state:
                entry.state === "completed"
                  ? "done"
                  : entry.state === "uploading"
                    ? "sending"
                    : entry.state === "failed" || entry.state === "refused"
                      ? "failed"
                      : "waiting",
              detail: t("wizard.sending.overall", {
                sent: formatSize(t, entry.offset, locale),
                total: formatSize(t, entry.size, locale),
              }),
              progress: entry.size > 0 ? entry.offset / entry.size : null,
            });
          }
        })
        .catch(() => undefined);
    onUploadEvent((event) => {
      // The service already held these bytes (same SHA-256): the line says
      // so, it does not say « envoyé ».
      if (event.kind === "Completed" && event.deduplicated) {
        mark(`image:${event.sha256}`, { alreadyOnline: true });
      }
      void refresh();
    })
      .then((unlisten) => {
        if (stopped) unlisten();
        else stop = unlisten;
      })
      .catch(() => undefined);
    void refresh();
    return () => {
      stopped = true;
      stop?.();
    };
  }, [phase, context.sessionId, mark, t, locale]);

  /** The stacking of the targets chosen, one after the other: Siril keeps
   * the machine busy. */
  async function stackAll(
    detection: NightDetection,
    sessionPresent: boolean,
  ): Promise<ImageCandidate[]> {
    const extra: ImageCandidate[] = [];
    if (!sessionPresent) return extra;
    let current = "";
    let family: string | null = null;
    const done: string[] = [];
    const named = () => (family ? `${current} (${family})` : current);
    // The targets that will really be stacked, in order: the frame draws
    // the share of the whole stacking, each target weighing its lights.
    const stackable = (targetId: string) => {
      const target = detection.targets.find(
        (item) => item.targetId === targetId,
      );
      // The image a device stacked is something to render, poses or not.
      return target?.folderName &&
        (target.lightFrames > 0 || hasDeviceImage(target))
        ? { ...target, folderName: target.folderName }
        : null;
    };
    const units = [...choices.stackTargets].flatMap((targetId) => {
      const target = stackable(targetId);
      return target ? [Math.max(1, target.lightFrames)] : [];
    });
    let unit = 0;
    mark("stacking", { state: "local" });
    beginFrame("stacking", undefined, units);
    const stop = await onStacking((event) => {
      if (event.event === "family") {
        family = event.family;
        mark("stacking", {
          label: t("wizard.sending.frameStacking", { target: named() }),
        });
      }
      updateFrame("stacking", (state) => withStackEvent(state, event));
    }).catch(() => undefined);
    try {
      for (const targetId of choices.stackTargets) {
        const target = stackable(targetId);
        if (!target) continue;
        // « Arrêter l'empilement » between two targets: the next one is not
        // started.
        if (stackingStopped.current) throw stackingCancelledError();
        current = target.designation;
        family = null;
        const unitIndex = unit;
        unit += 1;
        updateFrame("stacking", (state) =>
          startUnit(state, unitIndex, target.designation, Date.now()),
        );
        mark("stacking", {
          state: "local",
          label: t("wizard.sending.frameStacking", { target: named() }),
        });
        const run = await stackingRun({
          nightFolder: detection.folder,
          targetFolder: target.folderName,
          targetId,
          eveningDate: input.eveningDate ?? "",
          // The lights outside the night are left out.
          longitude: input.longitude ?? null,
          // The digest of a trusted script is added by the core, from
          // Astro Sync's data (`pipeline_trust`).
          // The pipelines of this target, as « Traitements » left
          // them (its card resolved them when the step was left).
          // Each with its own finish when it has one.
          pipelines: pipelinesToRun(
            rendersOf(
              choices.targetRenders,
              targetId,
              "broadband",
              context.pipelines,
            ),
            choices.targetFinishes,
            targetId,
            NO_PROCESSING_ID,
          ),
          // The poses « Mesure des poses » flagged and the person left out:
          // not integrated, never deleted.
          discarded: discardedForRun(posesOf(poses, targetId)),
          // The families that take the image their device stacked;
          // none: every family is stacked from its poses.
          device: deviceUses(target, choices.deviceUse ?? NO_DEVICE_CHOICES),
          // « Cadre de l'empilement » moved by hand; none: Automatic.
          frame: frameToRun(choices.targetFrames, targetId),
        });
        extra.push(...run.candidates);
        done.push(current);
      }
    } catch (caught) {
      mark("stacking", { state: "failed", detail: null });
      throw caught;
    } finally {
      stop?.();
    }
    endFrame("stacking", {
      detail: null,
      label:
        done.length > 0
          ? t("wizard.sending.frameStacking", { target: done.join(", ") })
          : t("wizard.sending.stacking"),
    });
    return extra;
  }

  /** The progression images of every target: a target that fails does not
   * stop the others. */
  async function progressionAll(
    detection: NightDetection,
  ): Promise<ProgressionResult> {
    const result: ProgressionResult = {
      candidates: [],
      frames: {},
      failure: null,
    };
    mark("progression", { state: "local" });
    const targets = detection.targets.filter(
      (item) => item.inNight && item.lightFrames > 0 && item.folderName,
    );
    beginFrame(
      "progression",
      undefined,
      targets.map((item) => item.lightFrames),
    );
    for (const [index, target] of targets.entries()) {
      mark("progression", {
        label: t("wizard.sending.frameProgression", {
          target: target.designation,
        }),
      });
      // A new target starts its two parts again; the bar of the whole run
      // stays where it was.
      updateFrame("progression", (state) =>
        startUnit(state, index, target.designation, Date.now()),
      );
      try {
        const families = await progressionFamilies(
          detection.folder,
          target.folderName!,
        );
        if (families.length === 0) continue;
        const images = await progressionImagesRun({
          nightId: context.sessionId,
          nightFolder: detection.folder,
          targetFolder: target.folderName!,
          family: families[0],
        });
        const candidates = await progressionImagesCandidates(
          images,
          target.targetId,
        );
        result.candidates.push(...candidates);
        result.frames[target.targetId] = target.lightFrames;
      } catch (caught) {
        result.failure ??= { error: caught };
      }
    }
    if (result.failure) {
      mark("progression", { state: "failed" });
    } else {
      endFrame("progression", { label: t("wizard.sending.progression") });
    }
    return result;
  }

  /** « Arrêter l'empilement »: this work only, the others go on. */
  function stopStacking() {
    stackingStopped.current = true;
    void localWorkCancelKind("stacking");
  }

  /** The image of each multi-night plan ticked: made here, queued as the
   * capture of the plan, then sent. */
  async function planImages(): Promise<Record<string, PlanImageRun>> {
    const made: Record<string, PlanImageRun> = {};
    for (const { project, lines } of plansToMake) {
      const choice = planChoices(project.projectId);
      const key = "planImage";
      mark(key, { state: "local" });
      // One unit: the steps of the plan's image (alignment, combination)
      // share the bar, which does not start again at each of them.
      beginFrame(key, undefined, [1]);
      try {
        // The frame is read again here, from the record as it is now: what is sent is the notch kept.
        const sources = chosenSources(lines, choice);
        const framing = await planFramingRead({
          projectId: project.projectId,
          lines,
          chosen: chosenKeys(lines, choice),
          sources,
        }).catch(() => null);
        made[project.projectId] = await planImageRun({
          sessionId: context.sessionId,
          planId: context.session?.planId ?? null,
          projectId: project.projectId,
          chosen: chosenKeys(lines, choice),
          sources,
          frame: frameToSend(framing, choice.frame),
          // The pipelines chosen for the plan's target.
          pipelines: planPipelines(
            lines,
            choice,
            choices.targetRenders,
            project.targetId,
            context.pipelines,
          ).map((pipelineId) => ({ pipelineId, trustedSha256: null })),
          eveningDate,
          // The history of the plan's capture follows the night's: its image
          // after each night, when the progression is ticked.
          progression: treatments.has("progression"),
          // The TIFF of each image only when the person joined them.
          includeTiff: includeTiff ?? false,
        });
      } catch (caught) {
        mark(key, { state: "failed", detail: null });
        throw caught;
      }
      endFrame(key, {});
    }
    if (Object.values(made).some((run) => run.kind === "made")) {
      await uploadAll({
        sessionId: context.sessionId,
        planId: context.session?.planId ?? null,
        targetIds: [],
      });
    }
    return made;
  }

  async function send() {
    if (running.current || !detection) return;
    running.current = true;
    setError(null);
    setApplyError(null);
    // The incidents ticked go into the session first: nothing leaves when
    // they could not be added (unticking them lets the night go without).
    if (incidentsPlan.plan) {
      setAdding(true);
      const end = await applyIncidents(
        NIGHT_APPLY_DEPS,
        context.sessionId,
        incidentsPlan.plan,
        context.session
          ? nightCell(
              eveningOfSession(context.session, longitudeOf(context.detail)),
              locale,
            )
          : "",
        { t, locale, reason: (caught) => formatError(t, caught) },
        { work: () => undefined },
      );
      setAdding(false);
      if (!end.ok) {
        setApplyError(failureText(t, end.failure));
        running.current = false;
        return;
      }
      if (end.item) {
        onReport(end.item);
        // The session holds them now: the list is read again.
        incidents.dispatch({ type: "reload" });
      }
    }
    setStopped(false);
    stackingStopped.current = false;
    setFrames({});
    startedAt.current = {};
    setOutcome({
      solved: 0,
      progression: 0,
      notes: [],
      failed: [],
      multiNight: [],
    });
    onPhase("sending");
    try {
      // The stacking, the progression images and the report of the logs
      // have nothing to do with each other: they run together.
      const result = await sendNight({
        stacking: treatments.has("stacking")
          ? () => stackAll(detection, !!context.session)
          : null,
        progression: treatments.has("progression")
          ? () => progressionAll(detection)
          : null,
        logs: treatments.has("logs")
          ? async () => {
              mark("logs", { state: "local" });
              beginFrame("logs", { kind: "logs" });
              try {
                await nightSendLogs(context.sessionId, detection.folder);
              } catch (caught) {
                mark("logs", { state: "failed" });
                throw caught;
              }
              // The report is sent: its line keeps « envoyé ».
              mark("logs", { state: "done" });
            }
          : null,
        precheck: (extra) =>
          nightPrecheck(
            context.sessionId,
            input,
            extra,
            locale,
            undefined,
            includeTiff ?? false,
          ),
        keep: (plan) =>
          plan.image.kind !== "final" || treatments.has("final_image"),
        enqueue: (plans, quota) =>
          nightEnqueue(context.sessionId, plans, quota),
        upload: () =>
          uploadAll({
            sessionId: context.sessionId,
            planId: context.session?.planId ?? null,
            targetIds: [],
          }),
        integrations:
          projects.length > 0
            ? async () => {
                mark("integrations", { state: "sending" });
                const sent = await nightIntegrationsSend({
                  sessionId: context.sessionId,
                  planId: context.session?.planId ?? null,
                  nightFolder: detection.folder,
                  eveningDate,
                });
                mark("integrations", {
                  state: sent.some((item) => item.result === "refused")
                    ? "failed"
                    : "done",
                });
                return sent;
              }
            : null,
        planImage: plansToMake.length > 0 ? planImages : null,
        solve: treatments.has("sky_annotations")
          ? async () => {
              mark("sky", { state: "local" });
              beginFrame("sky", { kind: "sky" });
              const solved = (await nightSolve(context.sessionId, input))
                .length;
              // Nothing to resolve: the line stays, « rien à traiter ».
              endFrame("sky", solved === 0 ? { state: "none" } : {});
              return solved;
            }
          : null,
        record: (progressionFrames) =>
          nightRecordSending(context.sessionId, {
            analysisFingerprint: null,
            progressionFrames,
          }),
      });
      setPrecheck(result.precheck);
      if (result.kind === "refused") {
        running.current = false;
        onPhase("recapitulatif");
        return;
      }
      // A report or progression images that failed are said, not hidden.
      const notes: string[] = [];
      const failed: Treatment[] = [];
      if (result.failures.logs) {
        failed.push("logs");
        notes.push(
          `${t("wizard.sending.report")} : ${formatError(t, result.failures.logs.error)}`,
        );
      }
      if (result.failures.progression) {
        failed.push("progression");
        notes.push(
          `${t("wizard.sending.progression")} : ${formatError(t, result.failures.progression.error)}`,
        );
      }
      // The multi-night plans: a refusal said in clear, never hidden.
      const multiNight = sentMultiNightLines(t, locale, {
        projects: projects.map((item) => item.project),
        integrations: result.multiNight.integrations,
        planImage: result.multiNight.planImage,
      });
      notes.push(...multiNight.notes);
      if (result.failures.multiNight) {
        notes.push(
          `${t("multiNight.planImage.title")} : ${formatError(t, result.failures.multiNight.error)}`,
        );
      }
      setOutcome({
        solved: result.solved,
        progression: result.progressionImages,
        notes,
        failed,
        multiNight: multiNight.ok,
      });
      onPhase("sent");
    } catch (caught) {
      // The person stopped the stacking: nothing failed, and nothing was
      // sent; the summary says so.
      if (isStackingCancelled(caught)) setStopped(true);
      else setError(formatError(t, caught));
      onPhase("recapitulatif");
    } finally {
      running.current = false;
    }
  }

  if (phase === "sending") {
    const items = sendingItems(t, locale, {
      plans: precheck?.plans ?? [],
      detection,
      online: view?.online ?? null,
      treatments,
      progress,
    });
    // The multi-night plans, after the night's images.
    for (const { project } of projects.slice(0, 1)) {
      if (multi?.gate.allowed) {
        items.push({
          key: "integrations",
          label: t("multiNight.sending.integrations", { name: project.name }),
          state: "waiting",
          detail: null,
          progress: null,
          ...progress.integrations,
        });
      }
    }
    for (const { project } of plansToMake.slice(0, 1)) {
      items.push({
        key: "planImage",
        label: t("multiNight.sending.planImage", { name: project.name }),
        state: "waiting",
        detail: null,
        progress: null,
        ...progress.planImage,
      });
    }
    const total = bytes.total || precheck?.quota.toSendBytes || 0;
    return (
      <div className="as-step">
        <h2 className="as-step__heading">{t("wizard.sending.title")}</h2>
        <p className={helpTextClassName}>{t("wizard.sending.resumes")}</p>
        <div className={progressClassName.track}>
          <div
            className={progressClassName.fill}
            style={{
              width: `${total > 0 ? Math.min(100, (bytes.sent / total) * 100) : 0}%`,
            }}
          />
        </div>
        <p className={helpTextClassName}>
          {overallText(t, locale, bytes.sent, total)}
        </p>
        <ul className="as-sending">
          {items.map((item) => {
            const frame = isTreatmentKey(item.key)
              ? frames[item.key]
              : undefined;
            if (item.state === "local" && frame) {
              return (
                <li key={item.key} className="as-sending__item is-local">
                  <ProgressFrame
                    view={progressFrameView(t, {
                      label: item.label,
                      state: frame,
                      now,
                    })}
                    location={t("wizard.sending.local")}
                    lastLineLabel={t("wizard.sending.lastLineLabel")}
                    stop={
                      item.key === "stacking"
                        ? {
                            label: t("wizard.sending.stopStacking"),
                            onStop: stopStacking,
                          }
                        : undefined
                    }
                  />
                </li>
              );
            }
            return (
              <li
                key={item.key}
                className={`as-sending__item is-${item.state}`}
              >
                <span className="as-sending__row">
                  <Icon
                    name={
                      item.state === "done"
                        ? "check"
                        : item.state === "waiting"
                          ? "clock"
                          : item.state === "none"
                            ? "none"
                            : "upload"
                    }
                    className="as-line__glyph"
                  />
                  <span className="as-sending__label">{item.label}</span>
                  <span className="as-sending__state">
                    {itemStateText(t, item)}
                  </span>
                </span>
                {item.progress !== null && item.state === "sending" && (
                  <span className={progressClassName.track}>
                    <span
                      className={progressClassName.fill}
                      style={{ width: `${Math.round(item.progress * 100)}%` }}
                    />
                  </span>
                )}
              </li>
            );
          })}
        </ul>
        <div className="as-actions">
          <button
            type="button"
            className="as-button as-button--link"
            onClick={() => void uploadPause()}
          >
            {t("wizard.sending.pause")}
          </button>
        </div>
      </div>
    );
  }

  if (phase === "sent") {
    const lines = sentLines(t, locale, {
      plans: precheck?.plans ?? [],
      detection,
      online: view?.online ?? null,
      treatments: new Set(
        [...treatments].filter((item) => !outcome.failed.includes(item)),
      ),
      solved: outcome.solved,
      progressionImages: outcome.progression,
    });
    const web = context.status?.space?.webUrl ?? context.status?.serviceUrl;
    const targets =
      detection?.targets.filter((target) => target.inNight).length ?? 0;
    // What the primaries of the steps did on the way.
    const done = reportLines(report, t, locale);
    return (
      <div className="as-step">
        <div className="as-sent">
          <span className="as-sent__mark">
            <Icon name="check" strokeWidth={2.4} />
          </span>
          <div>
            <h2 className="as-step__heading">{t("wizard.sent.title")}</h2>
            <p className={helpTextClassName}>
              {[
                context.session
                  ? nightCell(
                      eveningOfSession(
                        context.session,
                        longitudeOf(context.detail),
                      ),
                      locale,
                    )
                  : null,
                pluralText(t, locale, "wizard.sent.targets", targets),
                t("wizard.sent.bytes", {
                  size: formatSize(t, precheck?.quota.toSendBytes ?? 0, locale),
                }),
              ]
                .filter(Boolean)
                .join(" · ")}
            </p>
          </div>
        </div>
        {outcome.notes.map((note) => (
          <Notice key={note} tone="warning">
            {note}
          </Notice>
        ))}
        <ul className="as-lines">
          {[...lines, ...outcome.multiNight].map((line) => (
            <li key={line} className="as-line as-line--ok">
              <Icon name="check" className="as-line__glyph" />
              <span className="as-line__text">{line}</span>
            </li>
          ))}
        </ul>
        {done.length > 0 && (
          <section className="as-summary-group" data-done="applied">
            <h3 className="as-summary-group__title">
              <Icon name="check" className="as-line__glyph" />
              {t("wizard.recap.report.title")}
            </h3>
            <ul className="as-lines">
              {done.map((line) => (
                <li key={line} className="as-line as-line--ok">
                  <Icon name="check" className="as-line__glyph" />
                  <span className="as-line__text">{line}</span>
                </li>
              ))}
            </ul>
          </section>
        )}
        <InfoLine
          line={t("wizard.sent.later")}
          details={<p>{t("wizard.sent.laterDetails")}</p>}
        />
        <div className="as-actions">
          <button
            type="button"
            className="as-button as-button--link"
            onClick={onNights}
          >
            {t("wizard.sent.backToNights")}
          </button>
          <button
            type="button"
            className="as-button as-button--primary"
            onClick={() =>
              web &&
              void openInBrowser(nightPage(web, locale, context.sessionId))
            }
          >
            <Icon name="external" className="as-button__icon" />
            {t("wizard.sent.openNight")}
          </button>
        </div>
      </div>
    );
  }

  if (!precheck) {
    return (
      <div className="as-step">
        <h2 className="as-step__heading">{t("wizard.summary.title")}</h2>
        <Notice tone="error">{error}</Notice>
        {!error && <Waiting label={t("wizard.wait.summary")} />}
        <StepFoot
          onBack={onBack}
          primary={t("wizard.summary.send")}
          enabled={false}
          onPrimary={() => undefined}
        />
      </div>
    );
  }
  const summary = summaryView(t, locale, {
    precheck,
    detection,
    online: view?.online ?? null,
    treatments,
    stackTargets: choices.stackTargets,
    deviceUse,
    deviceQuality,
    resume,
    privateServer: privateServer !== null,
    renderName: (id) => {
      const pipeline = context.pipelines.find((item) => item.id === id);
      return (
        pipeline?.displayName ??
        pipeline?.name[locale] ??
        pipeline?.name.en ??
        id
      );
    },
    includeTiff: includeTiff ?? false,
    // « la session du 7 oct. »: the sentence form, the evening of the night.
    date: context.session
      ? nightSentence(
          eveningOfSession(context.session, longitudeOf(context.detail)),
          locale,
        )
      : "—",
  });
  const offers = () => {
    const page = offersPage(context, locale);
    if (page) void openInBrowser(page);
  };
  // The multi-night plans in the summary: their lines and image under
  // « Envoyé à votre espace », the masters under « Ne quitte jamais cet
  // ordinateur », the right in the check.
  const masters = view?.integrations ?? [];
  const multiSummary = multiNightSummary(t, locale, {
    view: multi,
    masters,
    integrating: treatments.has("stacking"),
    // What each plan holds, the kept choices of this computer included.
    planImage: Object.fromEntries(
      projects.map(({ project }) => [
        project.projectId,
        planChoices(project.projectId),
      ]),
    ),
    targetRenders: choices.targetRenders,
    pipelines: context.pipelines,
    framed: Object.fromEntries(
      plansToMake.map(({ project }) => [
        project.projectId,
        summaryFrame(
          framings[project.projectId] ?? null,
          planChoices(project.projectId).frame,
        ),
      ]),
    ),
  });
  const mastersRow =
    projects.length === 0 ? nightMastersRow(t, locale, masters) : null;
  const groups = summary.groups.map((group) => ({ ...group }));
  const addRows = (
    key: "sent" | "neverLeaves",
    titleKey: string,
    rows: typeof multiSummary.sent,
  ) => {
    if (rows.length === 0) return;
    const found = groups.find((group) => group.key === key);
    if (found) found.rows = [...found.rows, ...rows];
    else groups.push({ key, titleKey, rows });
  };
  addRows(
    "sent",
    summary.allowed ? "wizard.summary.sent" : "wizard.summary.toSend",
    multiSummary.sent,
  );
  addRows("neverLeaves", "wizard.summary.neverLeaves", [
    ...multiSummary.never,
    ...(mastersRow ? [mastersRow] : []),
  ]);
  return (
    <div className="as-step">
      <h2 className="as-step__heading">{t("wizard.summary.title")}</h2>
      <p className={helpTextClassName}>
        {oneFinalStop(t(summary.leadKey, summary.leadParams))}
      </p>
      {/* A payment that failed: the space is read-only, nothing will be sent
          -- said before the last step. */}
      <PaymentFailedNotice variant="beforeSending" />
      <Notice tone="error">{error}</Notice>
      <Notice tone="warning">
        {stopped ? t("wizard.sending.stackingStopped") : null}
      </Notice>
      {groups.map((group) => (
        <section key={group.key} className="as-summary-group">
          <h3 className="as-summary-group__title">
            <Icon
              name={
                group.key === "sent"
                  ? "upload"
                  : group.key === "keptOnline"
                    ? "check"
                    : group.key === "setAside"
                      ? "pending"
                      : "folder"
              }
              className="as-line__glyph"
            />
            {t(group.titleKey)}
          </h3>
          <ul className="as-summary-rows">
            {group.rows.map((row, index) => (
              <li
                key={index}
                className={
                  [
                    row.over && "is-over",
                    row.note && "is-note",
                    row.render && "is-render",
                    row.fullResolution && "is-full-resolution",
                  ]
                    .filter(Boolean)
                    .join(" ") || undefined
                }
              >
                <span>
                  <Segs segs={row.segs} />
                </span>
                <span className="as-summary-rows__value">{row.value}</span>
              </li>
            ))}
          </ul>
        </section>
      ))}
      {/* « Liste des incidents »: what completes the session, independent
          of the sending. */}
      <IncidentsPanel
        context={context}
        view={view}
        input={input}
        poses={poses}
        store={incidents}
        onPlan={onPlan}
        onMeasure={onMeasure}
      />
      {/* « Joindre aussi les TIFF »: only when a render of this sending has
          a TIFF to join (the stacking to come, or renders already made). */}
      {(treatments.has("stacking") ||
        (precheck.fullResolution?.count ?? 0) > 0) && (
        <section className="as-card as-join-tiff-card">
          <JoinTiffBox
            checked={includeTiff ?? false}
            disabled={includeTiff === null}
            fullResolution={precheck.fullResolution ?? null}
            onChange={setIncludeTiff}
          />
        </section>
      )}
      {summary.allowed ? (
        <div className="as-check">
          <h3 className="as-check__title">
            <Icon name="check" className="as-line__glyph" />
            {t("wizard.summary.check.title")}
          </h3>
          <ul className="as-lines">
            {summary.checks.map((check) => (
              <li key={check.text} className="as-line as-line--ok">
                <Icon name="check" className="as-line__glyph" />
                <span className="as-line__text">{check.text}</span>
              </li>
            ))}
            {multiSummary.check && (
              <li
                className={
                  multiSummary.check.ok
                    ? "as-line as-line--ok"
                    : "as-line as-line--warning"
                }
              >
                <Icon
                  name={multiSummary.check.ok ? "check" : "warning"}
                  className="as-line__glyph"
                />
                <span className="as-line__text">
                  {multiSummary.check.text}
                  {multiSummary.check.offers && (
                    <>
                      {" "}
                      <button
                        type="button"
                        className="as-button as-button--link"
                        onClick={offers}
                      >
                        <Icon name="external" className="as-button__icon" />
                        {t("wizard.offers")}
                      </button>
                    </>
                  )}
                </span>
              </li>
            )}
          </ul>
          {summary.analysisNote && (
            <p className={helpTextClassName}>
              {t("wizard.summary.analysisNote")}
            </p>
          )}
          <InfoLine
            line={t("wizard.summary.check.service")}
            details={<p>{t("wizard.summary.check.serviceDetails")}</p>}
          />
        </div>
      ) : (
        <div className="as-check as-check--refused" role="alert">
          <h3 className="as-check__title">
            <Icon name="cross" className="as-line__glyph" />
            {t("wizard.summary.impossible")}
          </h3>
          <ul className="as-lines">
            {summary.refusals.map((text) => (
              <li key={text} className="as-line as-line--missing">
                <Icon name="cross" className="as-line__glyph" />
                <span className="as-line__text">{text}</span>
              </li>
            ))}
          </ul>
          <div className="as-check__foot">
            <span>{t("wizard.summary.nothingSent")}</span>
            {!privateServer && (
              <button
                type="button"
                className="as-button as-button--link"
                onClick={offers}
              >
                <Icon name="external" className="as-button__icon" />
                {t("wizard.offers")}
              </button>
            )}
          </div>
          <InfoLine
            line={t("wizard.summary.check.serviceShort")}
            details={<p>{t("wizard.summary.check.serviceDetails")}</p>}
          />
        </div>
      )}
      {summary.allowed && !resume && (
        <div className="as-summary-foot">
          <strong>
            {t("wizard.summary.toSendTotal", { size: summary.toSend })}
          </strong>
          <span className="as-summary-foot__links">
            <button
              type="button"
              className="as-button as-button--link"
              onClick={() => onOpen("dryRun")}
            >
              {t("wizard.summary.simulate")}
            </button>
            <button
              type="button"
              className="as-button as-button--link"
              onClick={() => onOpen("queue")}
            >
              {t("wizard.summary.queue")}
            </button>
          </span>
        </div>
      )}
      <Notice tone="error">{applyError}</Notice>
      <StepFoot
        onBack={onBack}
        line={incidentsLine(
          incidentsPlan.plan
            ? incidentsPlan.plan.adds.length +
                incidentsPlan.plan.replacements.length
            : 0,
          t,
          locale,
        )}
        working={adding ? t("wizard.recap.working.incidents") : null}
        primary={t(summary.primaryKey)}
        icon="upload"
        // The list of incidents is being read: what is ticked is not known
        // yet.
        enabled={
          summary.allowed && !!folder && !incidentsPlan.loading && !adding
        }
        onPrimary={() => void send()}
      />
    </div>
  );
}
