import {
  useCallback,
  useEffect,
  useMemo,
  useReducer,
  useRef,
  useState,
} from "react";

import { Icon } from "../../components/Icon";
import { InfoLine } from "../../components/InfoLine";
import { Notice } from "../../components/Notice";
import { WizardFrame } from "../../components/WizardFrame";
import { usePromptWindow } from "../../components/PromptWindow";
import { useSnackbar } from "../../components/Snackbar";
import { Waiting } from "../../components/Waiting";
import { useI18n } from "../../i18n";
import {
  assistantPrompt,
  chooseFolder,
  folderOpen,
  listSessions,
  nightDetail,
  nightFolderPlan,
  nightFolderPrepare,
  nightFollow,
  nightFoundAgain,
  nightMultiNight,
  nightOnline,
  nightTargetsChange,
  nightView,
  pipelinesList,
  spaceStatus,
  toolsDetect,
  workingFolderGet,
  type CapabilitiesState,
  type MultiNightView,
  type NightDetail,
  type NightFolderOutcome,
  type NightFolderSpec,
  type NightOnline,
  type NightView,
  type Pipeline,
  type SessionSummary,
  type SpaceStatus,
  type ToolRow,
  type WizardStep,
} from "../../lib/bridge";
import { formatError } from "../../lib/format";
import { promptNames } from "../../lib/promptWindow";
import { displayPath, homeOf } from "../../lib/home";
import { windowStorage } from "../../lib/lastReport";
import { loadImportClock, withImportClock } from "../../lib/importClock";
import { loadDeviceUse, saveDeviceUse } from "../../lib/deviceMaster";
import {
  loadLocations,
  saveLocations,
  type DetectAnswer,
  type SendLocations,
} from "../../lib/wizardDetect";
import { SessionLocations } from "./SessionLocations";
import { loadTargetRenders, saveTargetRenders } from "../../lib/pipelineChoice";
import { loadTargetFinishes, saveTargetFinishes } from "../../lib/finish";
import { loadTargetFrames, saveTargetFrames } from "../../lib/stackFrame";
import type { ScreenId } from "../../lib/menu";
import {
  detectionInput,
  eveningOfSession,
  loadUnticked,
  nightHeader,
  placeOf,
  longitudeOf,
  saveUnticked,
} from "../../lib/nights";
import { waitingKey } from "../../lib/waiting";
import {
  baseName,
  folderLine,
  folderStepAction,
  folderWasKnown,
  folderTree,
  nextStep,
  openingState,
  openingStep,
  parentOf,
  previousStep,
  targetChoices,
  thread,
  type Phase,
} from "../../lib/wizardSteps";
import { EMPTY_DRAFT, draftReducer } from "../../lib/incidentsDraft";
import {
  applyCibles,
  applyMeasures,
  type ApplyContext,
  type ApplyEnd,
  type ApplyHooks,
  type ApplyWork,
} from "../../lib/nightApply";
import {
  NO_CHOICES,
  ciblesPlan,
  ciblesWrite,
  measuresPlan,
  measuresWrite,
  nightReducer,
} from "../../lib/nightRecap";
import {
  DEFAULT_CHOICES,
  type TreatmentChoices,
} from "../../lib/wizardTreatments";
import { helpTextClassName } from "../../theme";
import { usePoseStats } from "../../hooks/usePoseStats";
import { NIGHT_APPLY_DEPS, failureText } from "./applyDeps";
import { FilesStep } from "./FilesStep";
import { copyText, Segs, StepBack, StepFoot, Stalled } from "./parts";
import { SummaryStep } from "./SummaryStep";
import { TargetsStep } from "./TargetsStep";
import { TreatmentsStep } from "./TreatmentsStep";

/** What the wizard knows of the night, each part once its read is back. */
export interface NightContext {
  sessionId: string;
  session: SessionSummary | null;
  online: NightOnline | null;
  detail: NightDetail | null;
  root: string | null;
  status: SpaceStatus | null;
  tools: ToolRow[];
  pipelines: Pipeline[];
  /** The multi-night plans the night's targets work for, read when
   * « Traitements » opens; `null` until then or on a failure. */
  multiNight: MultiNightView | null;
}

function emptyContext(sessionId: string): NightContext {
  return {
    sessionId,
    session: null,
    online: null,
    detail: null,
    root: null,
    status: null,
    tools: [],
    pipelines: [],
    multiNight: null,
  };
}

/**
 * The wizard of a night, under « Nuits »: « ← Nuits », the night's title,
 * the thread of five steps (Cibles, Dossier, Fichiers, Traitements,
 * Récapitulatif), then the sending and « Nuit envoyée ». The page of a
 * night is this wizard reopened at the step where something is missing.
 */
export function NightWizard({
  sessionId,
  initialStep,
  initialTarget = null,
  capabilitiesState,
  onBack,
  onOpen,
}: {
  sessionId: string;
  initialStep: WizardStep | null;
  /** The target whose card « Fichiers » brings to the top when it opens
   * (« Mesurer les poses » of the result of « Détecter les sessions »). */
  initialTarget?: string | null;
  capabilitiesState: CapabilitiesState | null;
  onBack: () => void;
  onOpen: (screen: ScreenId) => void;
}) {
  const { t, locale } = useI18n();
  const { notify } = useSnackbar();
  // What the wizard knows of the night, filled as each read comes back: the
  // header as soon as the night is known, a step as soon as what it reads is
  // there (the frame is drawn at once, the wait is inside it).
  const [context, setContext] = useState<NightContext>(() =>
    emptyContext(sessionId),
  );
  // The night itself (the list or its detail) and what is online are read:
  // « Cibles » can be drawn. Each failed read leaves its part empty.
  const [nightRead, setNightRead] = useState(false);
  const [onlineRead, setOnlineRead] = useState(false);
  // The installed tools and the pipelines, read when « Traitements » opens.
  const [toolsRead, setToolsRead] = useState<string | null>(null);
  const [view, setView] = useState<NightView | null>(null);
  const [phase, setPhase] = useState<Phase | null>(initialStep);
  // « Mesurer les poses » asks « Fichiers » for a target's card: a step is
  // not enough, the card is brought to the top and focused.
  const [anchor, setAnchor] = useState<{
    targetId: string;
    nonce: number;
  } | null>(() =>
    initialStep === "fichiers" && initialTarget
      ? { targetId: initialTarget, nonce: 1 }
      : null,
  );
  const [unticked, setUnticked] = useState<Set<string>>(() =>
    loadUnticked(windowStorage(), sessionId),
  );
  // Where the frames and the logs of the session are, when not in the
  // night's folder, kept by session; the answer of the detection sheet
  // lasts while the wizard is open.
  const [locations, setLocationsState] = useState<SendLocations>(() =>
    loadLocations(windowStorage(), sessionId),
  );
  const setLocations = useCallback(
    (next: SendLocations) => {
      setLocationsState(next);
      saveLocations(windowStorage(), sessionId, next);
      setDetectAnswer(null);
    },
    [sessionId],
  );
  const [detectAnswer, setDetectAnswer] = useState<DetectAnswer | null>(null);
  // « Mesure des poses »: measured in « Fichiers », the poses flagged left
  // out in « Traitements », read again when the stacking runs.
  const poses = usePoseStats();
  const [plan, setPlan] = useState<NightFolderOutcome | null>(null);
  const [folder, setFolder] = useState<NightFolderOutcome | null>(null);
  // The pipelines chosen target by target are kept on this computer, by
  // night.
  const [choices, setChoicesState] = useState<TreatmentChoices>(() => ({
    ...DEFAULT_CHOICES,
    targetRenders: loadTargetRenders(windowStorage(), sessionId),
    // The finish of each pipeline set by hand, beside them.
    targetFinishes: loadTargetFinishes(windowStorage(), sessionId),
    // The frame of each target moved by hand.
    targetFrames: loadTargetFrames(windowStorage(), sessionId),
    // The image of a device or a new stacking, family by family.
    deviceUse: loadDeviceUse(windowStorage(), sessionId),
  }));
  const setChoices = useCallback(
    (next: TreatmentChoices) => {
      setChoicesState(next);
      saveTargetRenders(windowStorage(), sessionId, next.targetRenders);
      saveTargetFinishes(windowStorage(), sessionId, next.targetFinishes ?? {});
      saveTargetFrames(windowStorage(), sessionId, next.targetFrames ?? {});
      saveDeviceUse(windowStorage(), sessionId, next.deviceUse ?? {});
    },
    [sessionId],
  );
  const [error, setError] = useState<string | null>(null);
  // What the person chose on the steps and their primaries apply: the targets and the plan of « Cibles », the masters of
  // « Traitements », the incidents of « Récapitulatif ». Kept here, above the
  // steps, so that « Précédent » finds them again; what was applied is the
  // report of « Nuit envoyée ».
  const [recap, dispatchRecap] = useReducer(nightReducer, NO_CHOICES);
  const [incidentsDraft, dispatchIncidents] = useReducer(
    draftReducer,
    EMPTY_DRAFT,
  );
  const incidents = useMemo(
    () => ({ draft: incidentsDraft, dispatch: dispatchIncidents }),
    [incidentsDraft],
  );
  // The part of a step that its primary is writing, and why it stopped.
  const [working, setWorking] = useState<ApplyWork | null>(null);
  const [applyError, setApplyError] = useState<string | null>(null);
  // Bumped when a primary wrote: the plans of the night are read again.
  const [planReads, setPlanReads] = useState(0);
  // The reading of the night's page failed: the steps that wait for it say
  // so, with « Réessayer » (never a wait without an end).
  const [viewFailed, setViewFailed] = useState(false);
  // Bumped by « Réessayer »: the night's page is read again even when it was
  // already null.
  const [viewTry, setViewTry] = useState(0);
  // The « Dossier » step runs at each opening (`attempt`), once per opening.
  const [attempt, setAttempt] = useState(0);
  const handled = useRef(-1);
  const [folderPanel, setFolderPanel] = useState<FolderPanel>({
    kind: "working",
  });
  // The night had a folder here that is gone: the new one was made all the
  // same and the step says so.
  const [relocated, setRelocated] = useState(false);
  // Bumped to read the night again from the start (« Réessayer »).
  const [readVersion, setReadVersion] = useState(0);
  const phaseRef = useRef<Phase | null>(initialStep);
  const targetKeyRef = useRef("");

  // What the night's page reads at once, in parallel; each read fills its
  // part as it comes back, and a failed one leaves its part empty (the step
  // that needs it says so). The tools and the pipelines are read later, when
  // « Traitements » opens.
  useEffect(() => {
    let cancelled = false;
    const patch = (change: (current: NightContext) => NightContext) => {
      if (!cancelled) setContext(change);
    };
    setContext(emptyContext(sessionId));
    setNightRead(false);
    setOnlineRead(false);
    const nightReads = Promise.allSettled([
      listSessions().then((page) => {
        const found = page.sessions.find((item) => item.id === sessionId);
        patch((current) => ({
          ...current,
          session: current.session ?? found ?? null,
        }));
      }),
      nightDetail(sessionId).then((detail) =>
        patch((current) => ({
          ...current,
          detail,
          session: current.session ?? detail.session,
        })),
      ),
    ]).then(() => {
      if (!cancelled) setNightRead(true);
    });
    // `night_view` takes the targets' names, so what is online is read first
    // (the page of the night then reads it again with the rest).
    nightOnline(sessionId)
      .then((online) => patch((current) => ({ ...current, online })))
      .catch((caught) => {
        if (!cancelled) setError(formatError(t, caught));
      })
      .finally(() => {
        if (!cancelled) setOnlineRead(true);
      });
    workingFolderGet()
      .then((state) =>
        patch((current) => ({ ...current, root: state.root ?? null })),
      )
      .catch(() => undefined);
    spaceStatus()
      .then((status) => patch((current) => ({ ...current, status })))
      .catch(() => undefined);
    void nightReads;
    return () => {
      cancelled = true;
    };
  }, [sessionId, t, readVersion]);

  useEffect(() => {
    const needsTools =
      phase === "traitements" ||
      phase === "recapitulatif" ||
      phase === "sending" ||
      phase === "sent";
    if (!needsTools || toolsRead === locale) return;
    let cancelled = false;
    void Promise.allSettled([toolsDetect(), pipelinesList(locale)]).then(
      ([tools, pipelines]) => {
        if (cancelled) return;
        setContext((current) => ({
          ...current,
          tools: tools.status === "fulfilled" ? tools.value : [],
          pipelines:
            pipelines.status === "fulfilled" ? pipelines.value.pipelines : [],
        }));
        setToolsRead(locale);
      },
    );
    return () => {
      cancelled = true;
    };
  }, [phase, toolsRead, locale]);

  // The multi-night plans of the night, with « Traitements »: read again at
  // each opening of the step (a night sent elsewhere since).
  const planId = context.session?.planId ?? null;
  const startedAt = context.session?.startedAt ?? null;
  const sessionEvening = context.session
    ? eveningOfSession(context.session, longitudeOf(context.detail))
    : null;
  useEffect(() => {
    if (phase !== "traitements" || !startedAt || !sessionEvening) return;
    let cancelled = false;
    nightMultiNight(sessionId, planId, sessionEvening)
      .then((multiNight) => {
        if (!cancelled) setContext((current) => ({ ...current, multiNight }));
      })
      .catch(() => undefined);
    return () => {
      cancelled = true;
    };
  }, [phase, sessionId, planId, startedAt, sessionEvening]);

  // The clock correction « Détecter les sessions » applied to the session:
  // the files keep the hours the camera wrote, so the night's window is
  // drawn on the evening they were written on.
  const importClock = useMemo(
    () => loadImportClock(windowStorage(), sessionId),
    [sessionId],
  );
  const input = useMemo(
    () =>
      context.session && onlineRead
        ? withImportClock(
            detectionInput(
              context.session,
              context.online,
              unticked,
              choices.unticked.has("stacking")
                ? new Set()
                : choices.stackTargets,
              undefined,
              longitudeOf(context.detail),
            ),
            importClock,
          )
        : null,
    [
      context.session,
      context.online,
      context.detail,
      onlineRead,
      unticked,
      choices,
      importClock,
    ],
  );

  const refreshView = useCallback(async () => {
    if (!input) return null;
    try {
      const next = await nightView(sessionId, input);
      setView(next);
      setViewFailed(false);
      return next;
    } catch (caught) {
      setError(formatError(t, caught));
      setViewFailed(true);
      return null;
    }
  }, [input, sessionId, t]);

  // The view once the night is known; the phase it opens at.
  useEffect(() => {
    if (!input || view) return;
    void refreshView().then((next) => {
      if (next) {
        setPhase((current) => current ?? openingStep(next.resume.step));
      }
      if (next?.location.state === "found") {
        setFolder(
          (current) =>
            current ?? {
              folder: (next.location as { folder: string }).folder,
              created: false,
              createdPaths: [],
              targets: (next.detection?.targets ?? []).map((target) => ({
                targetId: target.targetId,
                designation: target.designation,
                folderName: target.folderName ?? target.designation,
                state: target.inNight ? "existing" : "out-of-night",
              })),
              applied: false,
            },
        );
      }
    });
  }, [input, view, refreshView, viewTry]);

  const spec = useMemo<NightFolderSpec | null>(() => {
    if (!context?.session) return null;
    const location = view?.location;
    return {
      nightId: sessionId,
      serviceUrl: context.status?.serviceUrl ?? "",
      eveningDate: eveningOfSession(
        context.session,
        longitudeOf(context.detail),
      ),
      label:
        context.detail?.night?.title?.value ??
        placeOf(context.detail) ??
        t("wizard.folder.defaultLabel"),
      targets: (context.online?.targets ?? [])
        .filter((target) => target.targetId)
        .map((target) => ({
          targetId: target.targetId as string,
          designation: target.name ?? (target.targetId as string),
          ticked: !unticked.has(target.targetId as string),
        })),
      follow: location?.state === "found" ? location.folder : null,
    };
  }, [context, view, sessionId, unticked, t]);

  // The folder names « Cibles » shows in grey (nothing is written).
  useEffect(() => {
    if (!spec) return;
    nightFolderPlan(spec)
      .then(setPlan)
      .catch(() => setPlan(null));
  }, [spec]);

  phaseRef.current = phase;
  targetKeyRef.current = (context?.online?.targets ?? [])
    .map((target) => target.targetId)
    .join("|");

  /** Creates what is missing of the night's folder (the core keeps what is
   * there: a target ticked since gets its folder, nothing else is touched),
   * then reads the night again, whose folder is there now. */
  const runPrepare = useCallback(
    async (follow: string | null) => {
      if (!spec) return;
      setFolderPanel({ kind: "working" });
      try {
        const outcome = await nightFolderPrepare(
          follow ? { ...spec, follow } : spec,
        );
        setFolder(outcome);
        setFolderPanel({ kind: "ready" });
        setError(null);
        setView(null);
      } catch (caught) {
        // Said in words in a snackbar; the step keeps « Réessayer ».
        notify({
          tone: "error",
          text: `${t("wizard.folder.failed")} ${formatError(t, caught)}`,
        });
        setFolderPanel({ kind: "failed" });
      }
    },
    [spec, t, notify],
  );

  // « Dossier » does its work each time the step opens, without a question:
  // the night that reopens here without a folder gets it, whatever button
  // led here, even when the folder it had is gone.
  useEffect(() => {
    if (phase !== "dossier" || handled.current === attempt) return;
    if (!view || !spec) return; // reading the night: the step says so
    handled.current = attempt;
    if (
      folderStepAction(view.location) === "choose" &&
      view.location.state === "duplicated"
    ) {
      setFolderPanel({ kind: "choose", folders: view.location.folders });
    } else {
      setRelocated(folderWasKnown(view.location, view.folderTrace));
      void runPrepare(null);
    }
  }, [phase, attempt, view, spec, runPrepare]);

  /** The night changed elsewhere (a target added on the web): read again,
   * and run « Dossier » again when its targets are not the same. */
  const reloadNight = useCallback(async () => {
    const [onlineReply, detailReply] = await Promise.allSettled([
      nightOnline(sessionId),
      nightDetail(sessionId),
    ]);
    if (onlineReply.status !== "fulfilled") return;
    const changed =
      onlineReply.value.targets.map((target) => target.targetId).join("|") !==
      targetKeyRef.current;
    setContext((current) => ({
      ...current,
      online: onlineReply.value,
      detail:
        detailReply.status === "fulfilled" ? detailReply.value : current.detail,
    }));
    setView(null);
    if (changed && phaseRef.current === "dossier") {
      setAttempt((current) => current + 1);
    }
  }, [sessionId]);

  // The window comes back to the front while « Cibles » or « Dossier » is
  // open: a target added on the web appears, ticked.
  useEffect(() => {
    if (phase !== "cibles" && phase !== "dossier") return;
    const onFocus = () => void reloadNight();
    window.addEventListener("focus", onFocus);
    return () => window.removeEventListener("focus", onFocus);
  }, [phase, reloadNight]);

  const opening = openingState({
    contextRead: nightRead && onlineRead,
    hasSession: Boolean(context.session),
    phase,
    viewFailed,
  });

  const session = context.session;
  // The header of the night's page on the web: the eyebrow, « Nuit du … »,
  // the line with the place, the hours (UTC) and the number of targets.
  const header = session
    ? nightHeader(t, locale, {
        session,
        detail: context.detail,
        online: context.online,
      })
    : null;
  const goTo = (next: Phase, target: string | null = null) => {
    setError(null);
    setApplyError(null);
    setAnchor((held) =>
      next === "fichiers" && target
        ? { targetId: target, nonce: (held?.nonce ?? 0) + 1 }
        : null,
    );
    if (next === "dossier") {
      // Each opening of « Dossier » does its work again.
      setFolderPanel({ kind: "working" });
      setAttempt((current) => current + 1);
    }
    setPhase(next);
  };
  const retryView = () => {
    setError(null);
    setViewFailed(false);
    setView(null);
    setViewTry((current) => current + 1);
  };
  const applyContext: ApplyContext = {
    t,
    locale,
    reason: (caught) => formatError(t, caught),
  };
  const applyHooks: ApplyHooks = {
    work: setWorking,
    targetApplied: (key) => dispatchRecap({ type: "targetApplied", key }),
    planApplied: () => dispatchRecap({ type: "planApplied" }),
    detachApplied: (projectId, name) =>
      dispatchRecap({ type: "detachApplied", projectId, name }),
    measureApplied: (targetId, figures) =>
      dispatchRecap({ type: "measureApplied", targetId, figures }),
    forgetApplied: (targetId) =>
      dispatchRecap({ type: "forgetApplied", targetId }),
  };
  /** Ends a primary that wrote: the words of what stopped it, or the next
   * step (what is ticked is applied when the person goes on). */
  const finishApply = (end: ApplyEnd, next: Phase) => {
    setWorking(null);
    if (end.ok) goTo(next);
    else setApplyError(failureText(t, end.failure));
  };
  /** « Continuer » on « Cibles »: the targets to create, the targets of the
   * catalogue, the plan; then the night is read again, whose folders the next
   * step makes. */
  async function continueFromTargets() {
    const plan = ciblesPlan(recap);
    if (!ciblesWrite(plan)) {
      goTo("dossier");
      return;
    }
    setApplyError(null);
    const end = await applyCibles(
      NIGHT_APPLY_DEPS,
      sessionId,
      plan,
      applyContext,
      applyHooks,
    );
    await reloadNight();
    setPlanReads((current) => current + 1);
    finishApply(end, "dossier");
  }
  /** « Continuer » on « Traitements »: the measures asked. */
  async function continueFromTreatments() {
    const plan = measuresPlan(recap);
    if (!measuresWrite(plan)) {
      goTo("recapitulatif");
      return;
    }
    setApplyError(null);
    const end = await applyMeasures(
      NIGHT_APPLY_DEPS,
      sessionId,
      plan,
      applyContext,
      applyHooks,
    );
    finishApply(end, "recapitulatif");
  }
  const step = phase === "sending" || phase === "sent" ? null : phase;
  const back = step ? previousStep(step) : null;
  const privateServer =
    context.status && !context.status.hosted ? context.status.serviceUrl : null;

  /** « Suivre ce dossier »: the copy every step then reads. */
  async function followFolder(chosen: string) {
    try {
      await nightFollow(sessionId, chosen);
      setError(null);
      await runPrepare(chosen);
    } catch (caught) {
      setError(formatError(t, caught));
    }
  }

  /** « Retrouver le dossier »: the person points at the moved folder, which
   * must carry the night; the step then runs again on it. */
  async function findFolder() {
    try {
      const chosen = await chooseFolder();
      if (!chosen) return;
      await nightFoundAgain(sessionId, chosen);
      setError(null);
      setRelocated(false);
      setView(null);
      setFolderPanel({ kind: "working" });
      setAttempt((current) => current + 1);
    } catch (caught) {
      setError(formatError(t, caught));
    }
  }

  return (
    <WizardFrame
      back={{ label: t("nav.sessions"), onClick: onBack }}
      eyebrow={header?.eyebrow}
      title={header?.title}
      line={header?.line}
      stepKey={phase}
      steps={thread(phase).map((item) => ({
        ...item,
        // A step done opens again, as « Précédent » does -- not while the
        // night is leaving or once it left.
        go:
          phase !== "sending" && phase !== "sent"
            ? () => goTo(item.step as WizardStep)
            : undefined,
      }))}
    >
      <Notice tone="error">{error}</Notice>
      {opening === "loading" && (
        <Waiting
          label={t(
            waitingKey({
              phase,
              viewReady: view !== null,
              creatingFolder: false,
              toolsReady: toolsRead !== null,
            }),
          )}
        />
      )}
      {opening === "not-found" && (
        <Stalled
          message={t("wizard.open.notFound")}
          onRetry={() => {
            setError(null);
            setReadVersion((version) => version + 1);
          }}
        />
      )}
      {opening === "failed" && (
        <Stalled message={t("wizard.view.failed")} onRetry={retryView} />
      )}
      {opening === "ready" && phase === "cibles" && (
        <TargetsStep
          context={context}
          choices={targetChoices(
            t,
            context.online,
            context.detail,
            unticked,
            plan,
          )}
          recap={recap}
          onRecap={dispatchRecap}
          planReads={planReads}
          working={working ? t(`wizard.recap.working.${working}`) : null}
          applyError={applyError}
          onToggle={(targetId) => {
            const next = new Set(unticked);
            if (next.has(targetId)) next.delete(targetId);
            else next.add(targetId);
            setUnticked(next);
            saveUnticked(windowStorage(), sessionId, next);
          }}
          onOffers={() => onOpen("dependencies")}
          onContinue={() => void continueFromTargets()}
        />
      )}
      {opening === "ready" && phase === "dossier" && (
        <FolderStep
          context={context}
          folder={folder}
          panel={folderPanel}
          relocated={relocated}
          reading={view === null}
          readingFailed={viewFailed}
          onRetryReading={retryView}
          onFollow={(chosen) => void followFolder(chosen)}
          onFind={() => void findFolder()}
          onRetry={() => {
            handled.current = -1;
            setFolderPanel({ kind: "working" });
            setAttempt((current) => current + 1);
          }}
          locations={locations}
          onLocations={setLocations}
          onGuide={() => onOpen("guide")}
          onBack={() => goTo("cibles")}
          onContinue={() => goTo("fichiers")}
        />
      )}
      {opening === "ready" && phase === "fichiers" && input && (
        <FilesStep
          context={context}
          view={view}
          viewFailed={viewFailed}
          input={input}
          locations={locations}
          answer={detectAnswer}
          onAnswer={setDetectAnswer}
          onAddTarget={async (targetId) => {
            await nightTargetsChange(sessionId, { kind: "add", targetId });
            await reloadNight();
          }}
          onRefresh={() => void refreshView()}
          onRetry={retryView}
          onMakeFolder={() => runPrepare(null)}
          poses={poses}
          anchor={anchor}
          onBack={back ? () => goTo(back) : undefined}
          onContinue={() => goTo(nextStep("fichiers") ?? "traitements")}
        />
      )}
      {opening === "ready" && phase === "traitements" && (
        <TreatmentsStep
          context={context}
          view={view}
          viewFailed={viewFailed}
          toolsReady={toolsRead !== null}
          onRetry={retryView}
          input={input}
          capabilitiesState={capabilitiesState}
          privateServer={privateServer}
          choices={choices}
          onChoices={setChoices}
          poses={poses}
          recap={recap}
          onRecap={dispatchRecap}
          working={working ? t(`wizard.recap.working.${working}`) : null}
          applyError={applyError}
          onTools={(tools) => setContext((current) => ({ ...current, tools }))}
          onTrust={(id, trust) =>
            setContext((current) => ({
              ...current,
              pipelines: current.pipelines.map((pipeline) =>
                pipeline.id === id ? { ...pipeline, trust } : pipeline,
              ),
            }))
          }
          onOffers={() => onOpen("dependencies")}
          onBack={() => goTo("fichiers")}
          onMeasure={(targetId) => goTo("fichiers", targetId)}
          onContinue={() => void continueFromTreatments()}
        />
      )}
      {opening === "ready" &&
        (phase === "recapitulatif" ||
          phase === "sending" ||
          phase === "sent") &&
        input && (
          <SummaryStep
            context={context}
            view={view}
            input={input}
            folder={folder}
            phase={phase}
            privateServer={privateServer}
            choices={choices}
            poses={poses.poses}
            incidents={incidents}
            report={recap.report}
            onReport={(item) =>
              dispatchRecap({ type: "incidentsApplied", item })
            }
            onPhase={goTo}
            onMeasure={(targetId) => goTo("fichiers", targetId)}
            onBack={() => goTo("traitements")}
            onNights={onBack}
            onOpen={onOpen}
          />
        )}
    </WizardFrame>
  );
}

/* --------------------------------- Cibles -------------------------------- */

/** The night's page on the web (the targets are edited there). */
export function nightPage(
  web: string,
  locale: string,
  sessionId: string,
): string {
  return `${web.replace(/\/$/, "")}/${locale}/sessions/${sessionId}`;
}

/* -------------------------------- Dossier -------------------------------- */

/** What the « Dossier » step shows when it has no folder to show yet. */
type FolderPanel =
  | { kind: "working" }
  | { kind: "ready" }
  | { kind: "choose"; folders: string[] }
  | { kind: "failed" };

function FolderStep({
  context,
  folder,
  panel,
  relocated,
  reading,
  readingFailed,
  onRetryReading,
  onFollow,
  onFind,
  onRetry,
  locations,
  onLocations,
  onGuide,
  onBack,
  onContinue,
}: {
  context: NightContext;
  locations: SendLocations;
  onLocations: (next: SendLocations) => void;
  folder: NightFolderOutcome | null;
  panel: FolderPanel;
  /** The folder the night had here was gone: this one is new. */
  relocated: boolean;
  /** The night's page is not read yet. */
  reading: boolean;
  /** … and its reading failed. */
  readingFailed: boolean;
  onRetryReading: () => void;
  onFollow: (folder: string) => void;
  onFind: () => void;
  onRetry: () => void;
  onGuide: () => void;
  onBack: () => void;
  onContinue: () => void;
}) {
  const { t, locale } = useI18n();
  const [frameOpen, setFrameOpen] = useState(false);
  const { notify } = useSnackbar();
  const promptWindow = usePromptWindow();
  const home = context.root ? homeOf(context.root) : null;
  if (panel.kind === "choose") {
    // Two folders carry the night: the person says which one to follow.
    return (
      <div className="as-step">
        <h2 className="as-step__heading">{t("wizard.folder.title")}</h2>
        <div className="as-stalled">
          <p>
            <strong>{t("wizard.folder.duplicated.title")}</strong>
          </p>
          <p className={helpTextClassName}>
            {t("wizard.folder.duplicated.lead")}
          </p>
        </div>
        <ul className="as-folder-copies">
          {panel.folders.map((copy) => (
            <li key={copy} className="as-path">
              <Icon name="folder" className="as-path__icon" />
              <code className="as-path__text">{displayPath(copy, home)}</code>
              <button
                type="button"
                className="as-button as-button--tertiary"
                onClick={() => onFollow(copy)}
              >
                {t("wizard.folder.follow")}
              </button>
            </li>
          ))}
        </ul>
        <StepBack onBack={onBack} />
      </div>
    );
  }
  if (!folder) {
    // Reading the night, creating the folder, or stuck with the reason.
    return (
      <div className="as-step">
        <h2 className="as-step__heading">{t("wizard.folder.title")}</h2>
        {panel.kind === "failed" ? (
          <Stalled
            message={t("wizard.folder.failed")}
            onRetry={onRetry}
            onBack={onBack}
          />
        ) : reading && readingFailed ? (
          <Stalled
            message={t("wizard.view.failed")}
            onRetry={onRetryReading}
            onBack={onBack}
          />
        ) : (
          <Waiting
            label={t(
              waitingKey({
                phase: "dossier",
                viewReady: !reading,
                creatingFolder: true,
                toolsReady: true,
              }),
            )}
          />
        )}
      </div>
    );
  }
  const path = displayPath(folder.folder, home);
  const nightName = baseName(folder.folder);
  const root = context.root ?? parentOf(folder.folder);
  const targetFolders = folder.targets
    .filter((target) => target.state !== "out-of-night")
    .map((target) => target.folderName);
  const finder = navigator.userAgent.includes("Windows")
    ? "wizard.folder.openExplorer"
    : navigator.userAgent.includes("Mac")
      ? "wizard.folder.openFinder"
      : "wizard.folder.openFolder";

  // The prompt is copied and said in a snackbar, « Voir le prompt » opens
  // it; its words are the core's.
  const ask = () =>
    promptWindow.ask(() =>
      assistantPrompt(locale, {
        root: displayPath(root, home),
        nightFolder: nightName,
        targets: targetFolders,
        names: promptNames(
          folder.targets.filter((target) => target.state !== "out-of-night"),
          context.online,
        ),
      }),
    );

  return (
    <div className="as-step">
      <h2 className="as-step__heading">{t("wizard.folder.title")}</h2>
      <p className="as-step__ok">
        <Icon name="check" className="as-line__glyph" />
        {folderLine(t, locale, folder)}
      </p>
      {relocated && (
        <div className="as-notice as-notice--neutral as-notice--inline">
          <span>{t("wizard.folder.relocated")}</span>
          <button
            type="button"
            className="as-button as-button--tertiary"
            onClick={onFind}
          >
            <Icon name="folder" className="as-button__icon" />
            {t("wizard.folder.findOld")}
          </button>
        </div>
      )}
      <div className="as-path">
        <Icon name="folder" className="as-path__icon" />
        <code className="as-path__text">{path}</code>
        <button
          type="button"
          className="as-button as-button--link"
          onClick={() => void folderOpen(folder.folder)}
        >
          <Icon name="external" className="as-button__icon" />
          {t(finder)}
        </button>
        <button
          type="button"
          className="as-button as-button--link"
          onClick={() =>
            void copyText(folder.folder).then((ok) =>
              notify(
                ok
                  ? { tone: "success", text: t("wizard.folder.pathCopied") }
                  : { tone: "warning", text: t("wizard.folder.pathRefused") },
              ),
            )
          }
        >
          <Icon name="copy" className="as-button__icon" />
          {t("wizard.folder.copyPath")}
        </button>
      </div>
      <div className="as-fold">
        <button
          type="button"
          className="as-fold__toggle"
          aria-expanded={frameOpen}
          onClick={() => setFrameOpen(!frameOpen)}
        >
          <Icon
            name="chevron"
            className={
              frameOpen ? "as-how__chevron is-open" : "as-how__chevron"
            }
          />
          <strong>{t("wizard.folder.howTitle")}</strong>
          {!frameOpen && (
            <span className={helpTextClassName}>
              {t("wizard.folder.howFolded")}
            </span>
          )}
        </button>
        {frameOpen && (
          <button
            type="button"
            className="as-button as-button--link as-fold__guide"
            onClick={onGuide}
          >
            <Icon name="book" className="as-button__icon" />
            {t("home.readGuide")}
          </button>
        )}
        {frameOpen && (
          <div className="as-fold__body">
            <ul className="as-tree">
              {folderTree(baseName(root), nightName, targetFolders).map(
                (row, index) => (
                  <li
                    key={index}
                    style={{ paddingLeft: `${row.depth * 1.25}rem` }}
                  >
                    <Icon
                      name={row.file ? "book" : "folder"}
                      className="as-tree__icon"
                    />
                    <code>{row.name}</code>
                    {(row.noteKey || row.note) && (
                      <span className="as-tree__note">
                        {row.noteKey ? t(row.noteKey) : row.note}
                      </span>
                    )}
                  </li>
                ),
              )}
            </ul>
            <div className="as-tree__aside">
              <p>
                <Segs
                  segs={[{ strong: "final/" }, t("wizard.folder.finalNote")]}
                />
              </p>
              <p>
                <Segs
                  segs={[
                    { strong: "lights/" },
                    ", ",
                    { strong: "calibration/" },
                    ", ",
                    { strong: "logs/" },
                    t("wizard.folder.framesNote"),
                  ]}
                />
              </p>
              <p>{t("wizard.folder.englishNames")}</p>
            </div>
          </div>
        )}
      </div>
      <SessionLocations locations={locations} onChange={onLocations} />
      <div className="as-assist">
        <p>{t("wizard.folder.assistNote")}</p>
        <button
          type="button"
          className="as-button as-button--tertiary"
          onClick={() => void ask()}
        >
          <Icon name="sparkle" className="as-button__icon" />
          {t("wizard.folder.askAssistant")}
        </button>
      </div>
      <InfoLine
        line={t("wizard.folder.neverMoves")}
        details={<p>{t("wizard.folder.neverMovesDetails")}</p>}
      />
      {promptWindow.window}
      <StepFoot
        onBack={onBack}
        primary={t("wizard.folder.continue")}
        onPrimary={onContinue}
      />
    </div>
  );
}
