import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useReducer,
  useRef,
  type ReactNode,
} from "react";

import { useI18n } from "../i18n";
import { useCapabilities } from "./CapabilitiesProvider";
import {
  detectImagesSend,
  detectImportCreate,
  detectImportDiscard,
  detectImportPlans,
  detectTreatmentsPrepare,
  incidentsApply,
  isCommandError,
  localSettingsGet,
  localSettingsSetLastFolders,
  localWorkCancel,
  nightSendLogs,
  onDetectFound,
  onLocalWork,
  onPlanImage,
  onStacking,
  planImageRunLocal,
  sessionsDetect,
  sessionsOrganise,
  sessionsOrganisePlan,
  sessionsProvisional,
  sessionsRefreshRights,
  sessionsUserData,
  spaceStorageGet,
  stackingRun,
  workingFolderGet,
  type PlanImageOutcome,
  type PlanProgressionStep,
  type StackedRender,
  type LastFolders,
} from "../lib/bridge";
import {
  buildRequest as buildDetectRequest,
  isCancelled,
  lastFoldersOf,
  windowFromLast,
  type DetectWindowState,
} from "../lib/detectFlow";
import { withUtcOffsets } from "../lib/detectPlaces";
import {
  buildRequest as buildCreateRequest,
  followRights,
  initialState,
  type ReviewState,
} from "../lib/detectReview";
import {
  summaryImages,
  treatmentCards,
  treatmentFolders,
  treatmentNightFolders,
  type SummaryImage,
} from "../lib/detectTreatments";
import { contrastValue } from "../lib/contrast";
import { colourOf, pipelinesToRun, softStarsOf } from "../lib/finish";
import { NO_PROCESSING_ID } from "../lib/pipelineChoice";
import { formatError } from "../lib/format";
import { importSends, renderOf, variantCount } from "../lib/importSend";
import { nightDay } from "../lib/nights";
import { applyRecap, PlanNotCreated, type ApplyEnd } from "../lib/importApply";
import { tidyNames, type TidyChoice, type TidyState } from "../lib/tidy";
import type { DraftAction } from "../lib/incidentsDraft";
import { clocksOf, saveImportClocks } from "../lib/importClock";
import {
  NO_IMPORT,
  importPresence,
  importReducer,
  provisionalIds,
  recapPlan,
  type ImportPresence,
  type ImportState,
  type ImportStep,
  type Made,
} from "../lib/importFlow";
import { windowStorage } from "../lib/lastReport";
import {
  beginProgress,
  isOfferRefusal,
  isStackingCancelled,
  nightUnitLabel,
  startUnit,
  withStackEvent,
  withUnits,
  type TreatmentProgress,
} from "../lib/treatmentProgress";
import { isAnalysisSendingHeld } from "../lib/capabilities";
import { runTarget } from "../lib/uploads";
import { useUploads } from "./UploadsProvider";

export interface ImportApi {
  state: ImportState;
  presence: ImportPresence;
  /** « Détecter les sessions »: opens an import, or the one already open. */
  open: () => void;
  /** Forgets the import (« Retour aux sessions » from « Terminé »). */
  close: () => void;
  /** « Abandonner l'import », once confirmed: stops what runs, removes the
   * provisional folders of the treatments (links and Astro Sync's results,
   * never a file of the person's), forgets the import; nothing created is
   * undone. */
  abandon: () => void;
  setChoices: (choices: DetectWindowState) => void;
  /** « Lancer la détection ». */
  startScan: () => void;
  /** « Arrêter la détection »: nothing is written, back to the folders. */
  stopScan: () => void;
  changeReview: (update: (review: ReviewState) => ReviewState) => void;
  /** « Continuer » of step 2: what would be created, computed here (nothing
   * is written). */
  create: () => void;
  goTo: (step: ImportStep) => void;
  toggleTarget: (targetId: string) => void;
  setRenders: (renders: ImportState["renders"]) => void;
  setFinishes: (finishes: NonNullable<ImportState["finishes"]>) => void;
  /** « Lancer N traitements »: the pipelines of each target ticked. */
  runTreatments: (pipelines: Record<string, string[]>) => void;
  /** « Arrêter les traitements »: the run ends, the import stays on step 3. */
  stopTreatments: () => void;
  /** « Continuer sans traitement » / « Continuer avec les images faites ». */
  skipTreatments: () => void;
  toggleImage: (key: string) => void;
  /** « Joindre aussi les TIFF »: the TIFF rows ticked or unticked. */
  joinTiff: (on: boolean) => void;
  /** A choice on the list of incidents of a session (a tick, an edit, a line
   * added by hand): kept in the store, written by the primary. */
  changeIncidents: (sessionId: string, action: DraftAction) => void;
  /** The option « Ranger les poses dans le dossier de travail ». */
  tickTidy: (on: boolean) => void;
  chooseTidy: (choice: TidyChoice) => void;
  setTidyOverview: (overview: TidyState | null) => void;
  /** « Créer et envoyer » / « Envoyer N images »: everything ticked applied,
   * in order; `rendering` names the image of a row. */
  send: (rendering: (row: SummaryImage) => string) => void;
  /** « Créer » / « Créer sans envoyer » / « Terminer »: the same, no image
   * leaves. */
  finish: () => void;
}

const ImportContext = createContext<ImportApi | null>(null);

export function useImport(): ImportApi {
  const api = useContext(ImportContext);
  if (!api) throw new Error("useImport outside ImportProvider");
  return api;
}

type Translate = (
  key: string,
  params?: Record<string, string | number>,
) => string;

/** Why step 4's primary stopped, in words, by its stage. */
function failureText(
  t: Translate,
  locale: string,
  end: Extract<ApplyEnd, { ok: false }>,
  state: ImportState,
): string {
  const reason = formatError(t, end.error);
  switch (end.stage) {
    case "sessions": {
      if (isCommandError(end.error) && end.error.code === "not_yet_available")
        return t("detect.create.unavailable");
      // The sessions were created; their folders could not take their names.
      const stage = isCommandError(end.error)
        ? (end.error.details as { stage?: string } | null)?.stage
        : undefined;
      return stage === "nights"
        ? t("import.recap.foldersFailed", { reason })
        : t("import.recap.createFailed", { reason });
    }
    case "plan":
      return end.error instanceof PlanNotCreated
        ? t("import.recap.planNotCreated", { plan: end.error.designation })
        : t("import.recap.planFailed", { reason });
    case "incidents": {
      const made = state.created?.outcome.sessions.find(
        (s) => s.sessionId === end.sessionId,
      );
      return t("import.recap.incidentsFailed", {
        night: made
          ? [
              made.designations.join(", "),
              nightDay(`${made.eveningDate}T12:00:00Z`, locale, "UTC"),
            ].join(t("detect.join"))
          : (end.sessionId ?? ""),
        reason,
      });
    }
    case "tidy":
      return t("import.recap.tidyFailed", { reason });
    // The report of the logs never stops the primary (« Terminé » says it).
    case "send":
    case "logs":
      return reason;
  }
}

/**
 * The import of sessions above the screens, as `UploadsProvider` holds the
 * uploads: the state of the five steps AND the jobs (the scan, the creation,
 * the treatments, the sending). A click in the menu unmounts the page, never
 * the import: the scan and the treatments go on, their result lands here, the
 * page reopens at the same step. Closing the application forgets it (the
 * sessions created stay in the space).
 */
export function ImportProvider({ children }: { children: ReactNode }) {
  const { t, locale } = useI18n();
  const { submit } = useUploads();
  const [state, dispatch] = useReducer(importReducer, NO_IMPORT);
  const stateRef = useRef(state);
  stateRef.current = state;
  const words = useRef({ t, locale });
  words.current = { t, locale };
  /** Bumped when a job is stopped or the import forgotten: a job whose
   * generation changed drops its result. */
  const generation = useRef(0);
  const last = useRef<LastFolders | undefined>(undefined);

  // The folders open on the last ones chosen.
  useEffect(() => {
    void localSettingsGet()
      .then((settings) => {
        last.current = settings.lastFolders;
        dispatch({
          type: "choicesRead",
          choices: windowFromLast(settings.lastFolders),
        });
      })
      .catch(() => undefined);
  }, []);

  // The offer may change while the proposal is held (a code redeemed, a plan
  // taken on the web): the proposal follows the rights of the account, so the
  // multi-night plans the detection showed locked open, ticked by default.
  const { state: capabilitiesState } = useCapabilities();
  // Read when step 4's primary runs: whether the space keeps the report of
  // the logs.
  const rights = useRef(capabilitiesState);
  rights.current = capabilitiesState;
  const rightsSignature = JSON.stringify(
    capabilitiesState?.capabilities ?? null,
  );
  const signature = useRef(rightsSignature);
  useEffect(() => {
    if (signature.current === rightsSignature) return;
    signature.current = rightsSignature;
    const held = stateRef.current;
    if (!held.proposal || !held.review || held.created) return;
    const { proposal } = held;
    void sessionsRefreshRights(proposal)
      .then((followed) => {
        const now = stateRef.current;
        if (!followed.changed || followed.stale) return;
        if (now.proposal !== proposal || !now.review) return;
        dispatch({
          type: "rightsFollowed",
          proposal: followed.proposal,
          review: followRights(
            followed.proposal,
            now.review,
            followed.unlocked,
          ),
        });
      })
      .catch(() => undefined);
  }, [rightsSignature]);

  const refreshStorage = useCallback(async () => {
    try {
      dispatch({ type: "storage", storage: await spaceStorageGet() });
    } catch {
      // Unknown: no guard, the service still refuses what it cannot keep.
    }
  }, []);

  const open = useCallback(() => dispatch({ type: "open" }), []);

  const close = useCallback(() => {
    generation.current += 1;
    dispatch({ type: "close" });
  }, []);

  const abandon = useCallback(() => {
    const held = stateRef.current;
    generation.current += 1;
    if (held.scan || held.running)
      void localWorkCancel().catch(() => undefined);
    // The provisional folders the treatments made: nothing of the space is
    // touched, nor any file of the person's.
    const ids = provisionalIds(held);
    if (ids.length > 0) void detectImportDiscard(ids).catch(() => undefined);
    dispatch({ type: "close" });
  }, []);

  const startScan = useCallback(() => {
    const held = stateRef.current;
    if (held.scan) return;
    const mine = ++generation.current;
    const choices = held.choices;
    dispatch({ type: "scanStart" });
    const kept = lastFoldersOf(choices, last.current);
    last.current = kept;
    void localSettingsSetLastFolders(kept).catch(() => undefined);
    void (async () => {
      const off: Array<() => void> = [];
      try {
        off.push(
          await onLocalWork((work) => {
            if (work.kind === "scan" && mine === generation.current)
              dispatch({
                type: "scanWalk",
                found: work.done,
                folder: work.current ?? undefined,
              });
            if (work.kind === "headers" && mine === generation.current)
              dispatch({
                type: "scanProgress",
                read: work.done,
                total: work.total,
                current: work.current ?? undefined,
              });
          }),
        );
        off.push(
          await onDetectFound((found) => {
            if (mine === generation.current)
              dispatch({ type: "scanFound", found });
          }),
        );
        // The person's data come first, with the places' offsets from UTC;
        // when they cannot be read the step says why and nothing runs.
        const user = withUtcOffsets(await sessionsUserData(), new Date());
        const proposal = await sessionsDetect(
          buildDetectRequest(choices),
          user,
          words.current.locale,
        );
        if (mine !== generation.current) return;
        dispatch({
          type: "scanDone",
          proposal,
          user,
          review: initialState(proposal),
        });
      } catch (caught) {
        if (mine !== generation.current) return;
        dispatch({
          type: "scanEnd",
          error:
            isCommandError(caught) && isCancelled(caught.code)
              ? null
              : formatError(words.current.t, caught),
        });
      } finally {
        for (const stop of off) stop();
      }
    })();
  }, []);

  const stopScan = useCallback(() => {
    generation.current += 1;
    void localWorkCancel().catch(() => undefined);
    dispatch({ type: "scanEnd", error: null });
  }, []);

  const create = useCallback(() => {
    const held = stateRef.current;
    if (!held.proposal || !held.review || held.creating || held.created) return;
    const { proposal, review } = held;
    dispatch({ type: "createStart" });
    void (async () => {
      try {
        // The offer may have changed since the detection: what is created
        // follows the rights of the account now, and the person sees it
        // before anything is made.
        const followed = await sessionsRefreshRights(proposal).catch(
          () => null,
        );
        if (followed?.changed) {
          if (!followed.stale) {
            dispatch({
              type: "rightsFollowed",
              proposal: followed.proposal,
              review: followRights(
                followed.proposal,
                review,
                followed.unlocked,
              ),
            });
          }
          dispatch({
            type: "createFailed",
            error: words.current.t(
              followed.stale ? "detect.rights.stale" : "detect.rights.followed",
            ),
          });
          return;
        }
        const request = buildCreateRequest(proposal, review);
        // Nothing is written to the space before step 4: what « Créer »
        // would make of the review, computed here, with a provisional
        // identifier for each session and plan to create.
        const outcome = await sessionsProvisional(request);
        dispatch({ type: "planned", local: { request, outcome } });
        void refreshStorage();
      } catch (failure) {
        dispatch({
          type: "createFailed",
          error: formatError(words.current.t, failure),
        });
      }
    })();
  }, [refreshStorage]);

  const runTreatments = useCallback(
    (pipelines: Record<string, string[]>) => {
      const held = stateRef.current;
      if (!held.local || held.created || !held.proposal || held.running) return;
      const proposal = held.proposal;
      // The sessions are not created yet: the treatments run on provisional
      // nights and plans.
      const local = held.local;
      const chosen = held.chosen ?? [];
      const wanted = treatmentCards(proposal, local.outcome).filter((card) =>
        chosen.includes(card.targetId),
      );
      if (wanted.length === 0) return;
      const mine = ++generation.current;
      const live = () => mine === generation.current;
      const update = (
        change: (frame: TreatmentProgress) => TreatmentProgress,
      ) => {
        if (live()) dispatch({ type: "frame", change });
      };
      dispatch({ type: "treatStart", target: wanted[0].designation });
      void (async () => {
        const { t, locale } = words.current;
        const results: Made[] = [];
        const failures: string[] = [];
        // The targets the offer refused: one sentence for all of them.
        let refused = 0;
        const unlisten: Array<() => void> = [];
        try {
          // The stacking and the image of a plan say where they are; the run
          // draws one bar for all of it.
          for (const listen of [onStacking, onPlanImage]) {
            unlisten.push(
              await listen((event) =>
                update((frame) => withStackEvent(frame, event)),
              ),
            );
          }
          dispatch({ type: "frameStart", frame: beginProgress(Date.now()) });
          // The proposal as the request sends it: the band of the nights
          // answered, and the name given to a place created at the import
          // (it names the night's folder).
          const nights = await detectTreatmentsPrepare(
            local.request.proposal,
            local.outcome,
            wanted.map((card) => card.targetId),
          );
          dispatch({
            type: "treatFolders",
            folders: treatmentFolders(nights),
            nights: treatmentNightFolders(nights),
          });
          // The units of the run, in the order they will run: each night of a
          // target weighs its frames, the image of its plan a tenth of them.
          const starts = new Map<string, number>();
          const weights: number[] = [];
          for (const card of wanted) {
            starts.set(card.targetId, weights.length);
            const mineNights = nights.filter((night) =>
              night.targets.some((target) => target.targetId === card.targetId),
            );
            let frames = 0;
            for (const night of mineNights) {
              const count =
                night.targets.find((item) => item.targetId === card.targetId)
                  ?.frames ?? 0;
              frames += count;
              weights.push(Math.max(1, count));
            }
            if (card.multiNight && mineNights.length > 1)
              weights.push(Math.max(1, Math.round(frames / 10)));
          }
          update((frame) => withUnits(frame, weights));
          for (const card of wanted) {
            if (!live()) break;
            let unit = starts.get(card.targetId) ?? 0;
            dispatch({ type: "treatTarget", target: card.designation });
            // Each pipeline with its own finish when it has one.
            const choice = pipelinesToRun(
              pipelines[card.targetId] ?? [],
              held.finishes,
              card.targetId,
              NO_PROCESSING_ID,
            );
            const mineNights = nights.filter((night) =>
              night.targets.some((target) => target.targetId === card.targetId),
            );
            // Every render of the first night (one per pipeline, and the ones
            // with the curve of presentation): each is listed in « Images à
            // envoyer ».
            let first: { sessionId: string; renders: StackedRender[] } | null =
              null;
            try {
              for (const night of mineNights) {
                const target = night.targets.find(
                  (item) => item.targetId === card.targetId,
                )!;
                const label = nightUnitLabel(t, locale, night.eveningDate);
                const at = unit;
                unit += 1;
                update((frame) => startUnit(frame, at, label, Date.now()));
                const stacked = await stackingRun({
                  nightFolder: night.nightFolder,
                  targetFolder: target.folderName,
                  targetId: card.targetId,
                  eveningDate: night.eveningDate,
                  longitude: night.longitude,
                  pipelines: choice,
                });
                const renders = stacked.outcome?.renders ?? [];
                if (renders.length > 0 && !first)
                  first = { sessionId: night.sessionId, renders };
              }
              let plans: PlanImageOutcome[] = [];
              let history: PlanProgressionStep[] = [];
              if (card.multiNight && mineNights.length > 1) {
                const at = unit;
                update((frame) =>
                  startUnit(
                    frame,
                    at,
                    t("detect.treat.runningPlan"),
                    Date.now(),
                  ),
                );
                // The plan the review proposed, from the masters of the
                // import's own nights: nothing is asked of the space.
                const imaged = await planImageRunLocal(
                  {
                    projectId: card.multiNight.planId,
                    targetId: card.targetId,
                    name: card.designation,
                    nights: card.multiNight.sessionIds,
                  },
                  choice,
                  true,
                );
                if (imaged.kind === "made") {
                  plans = imaged.outcomes;
                  history = imaged.progression;
                }
              }
              if (plans.length > 0) {
                // One image of the plan per pipeline, as the night's wizard
                // makes them: its PNG, and beside it its TIFF, ticked only
                // with « Joindre aussi les TIFF ».
                for (const plan of plans) {
                  const row = {
                    targetId: card.targetId,
                    designation: card.designation,
                    pipelineId: plan.render.pipelineId,
                    nights: plan.combinedNightIds.length || mineNights.length,
                  };
                  results.push({
                    ...row,
                    path: plan.render.path,
                    size: plan.render.size,
                    send: {
                      kind: "plan",
                      outcome: plan,
                      progression: history,
                    },
                  });
                  const tiff = plan.render.fullResolution;
                  if (tiff)
                    results.push({
                      ...row,
                      path: tiff.path,
                      size: tiff.size,
                      fullResolution: true,
                      twinOf: plan.render.path,
                      send: {
                        kind: "plan",
                        outcome: plan,
                        progression: history,
                        fullResolution: true,
                      },
                    });
                }
              } else if (first) {
                for (const render of first.renders) {
                  const row = {
                    targetId: card.targetId,
                    designation: card.designation,
                    pipelineId: render.pipelineId,
                    contrast: contrastValue(render.contrast),
                    colour: colourOf(render.colour),
                    softStars: softStarsOf(render.softStars),
                    nights: 1,
                  };
                  const send = {
                    kind: "night" as const,
                    sessionId: first.sessionId,
                    targetId: card.targetId,
                    render,
                  };
                  results.push({
                    ...row,
                    path: render.path,
                    size: render.size,
                    send,
                  });
                  const tiff = render.fullResolution;
                  if (tiff)
                    results.push({
                      ...row,
                      path: tiff.path,
                      size: tiff.size,
                      fullResolution: true,
                      twinOf: render.path,
                      send: { ...send, fullResolution: true },
                    });
                }
              }
            } catch (caught) {
              // « Arrêter les traitements » is not a failure to report.
              if (isOfferRefusal(caught)) refused += 1;
              else if (
                !isStackingCancelled(caught) &&
                !stateRef.current.stopping
              )
                failures.push(
                  t("detect.treat.failed", {
                    target: card.designation,
                    reason: formatError(t, caught),
                  }),
                );
              if (stateRef.current.stopping) break;
            }
          }
        } catch (caught) {
          if (isOfferRefusal(caught)) refused += 1;
          else if (!isStackingCancelled(caught) && !stateRef.current.stopping)
            failures.push(formatError(t, caught));
        } finally {
          for (const off of unlisten) off();
        }
        if (!live()) return;
        // The place left decides which images open ticked; « Envoi des
        // rendus » of Réglages whether their TIFF are.
        await refreshStorage();
        const kept = await localSettingsGet().catch(() => null);
        if (!live()) return;
        dispatch({ type: "joinTiff", on: kept?.rendersUpload === "both" });
        dispatch({
          type: "treatDone",
          made: results,
          errors: failures,
          refused,
        });
      })();
    },
    [refreshStorage],
  );

  const stopTreatments = useCallback(() => {
    if (!stateRef.current.running) return;
    dispatch({ type: "treatStop" });
    void localWorkCancel().catch(() => undefined);
  }, []);

  /** The primary of step 4: the sessions and plans created, the plan folders,
   * the incidents ticked of every night, the tidying, the images ticked (none
   * from « Créer » / « Terminer »), in that order, each stage said; a failure
   * stops there with its reason, what went through stays and the next try
   * skips it. */
  const apply = useCallback(
    (rendering: ((row: SummaryImage) => string) | null) => {
      const held = stateRef.current;
      if (held.sending || !held.local || !held.proposal) return;
      const local = held.local;
      const { t, locale } = words.current;
      const made = held.made;
      const rows = summaryImages(made, held.storage);
      // The first image ticked of a target is its capture; the others leave as
      // its variants (the capture goes first, the queue sends them in order).
      const chosen = rendering ? importSends(rows, made, held.ticked) : [];
      const variants = variantCount(chosen);
      const plan = recapPlan(held, t("incidents.mergeSeparator"));
      dispatch({ type: "sendStart" });
      void (async () => {
        const end = await applyRecap(
          {
            create: () => detectImportCreate(local.request, local.outcome),
            settlePlans: (nights) => detectImportPlans(nights),
            incidentsApply,
            workingFolder: async () => (await workingFolderGet()).root,
            organisePlan: (destination, mode, created) =>
              sessionsOrganisePlan(
                created.request.proposal,
                destination,
                mode,
                created.outcome.sessions.map((s) => s.detectedId),
                tidyNames(created.request.proposal, created.outcome),
              ),
            organise: sessionsOrganise,
            // The report of each night's logs, as « Journaux » of the night's
            // wizard (ticked by default): not when the space keeps none.
            ...(isAnalysisSendingHeld(rights.current)
              ? {}
              : {
                  sendLogs: (sessionId: string, folder: string) =>
                    nightSendLogs(sessionId, folder),
                }),
            enqueue: async (images, created) => {
              const enqueued = await detectImagesSend(images);
              // Queuing never sent anything by itself: the uploads leave now,
              // each with its notification and the page « Envois ».
              submit(
                "detect",
                chosen.map(({ row }, index) => {
                  const render = renderOf(images[index]);
                  return {
                    sha256: render.sha256,
                    path: render.path,
                    size: render.size,
                    target: row.designation,
                    rendering: rendering!(row),
                  };
                }),
                runTarget(
                  created.outcome.sessions,
                  images.map((image) =>
                    image.kind === "night" ? image.sessionId : null,
                  ),
                ),
                // What the linked space already has ends at once (« Déjà
                // envoyé le … »), instead of waiting for a run with nothing
                // to send.
                enqueued?.alreadySent,
              );
            },
          },
          {
            local,
            created: held.created,
            plan,
            images: chosen.map((item) => item.send),
            report: held.report,
            sent: false,
          },
          {
            phase: (working) => dispatch({ type: "working", working }),
            created: (created) => {
              // The wizard of a session whose clock was corrected reads its
              // files on the clock they were written with.
              const { proposal } = created.request;
              const outcome = created.outcome;
              saveImportClocks(windowStorage(), clocksOf(proposal, outcome));
              dispatch({ type: "created", created });
            },
            plansDone: (plans) => dispatch({ type: "plansSettled", plans }),
            incidentsDone: (sessionId, result) =>
              dispatch({ type: "incidentsApplied", sessionId, result }),
            tidyDone: (result) => dispatch({ type: "tidyApplied", result }),
            logsDone: (sessionId, result) =>
              dispatch({ type: "logsSent", sessionId, result }),
          },
        );
        if (end.ok) {
          dispatch({ type: "sent", count: chosen.length, variants });
          return;
        }
        dispatch({
          type: "sendFailed",
          error: failureText(t, locale, end, stateRef.current),
        });
      })();
    },
    [submit],
  );

  const send = useCallback(
    (rendering: (row: SummaryImage) => string) => apply(rendering),
    [apply],
  );

  /** « Créer » / « Créer sans envoyer » / « Terminer »: no image leaves,
   * what else is ticked is still applied. */
  const finish = useCallback(() => apply(null), [apply]);

  const api = useMemo<ImportApi>(
    () => ({
      state,
      presence: importPresence(state),
      open,
      close,
      abandon,
      setChoices: (choices) => dispatch({ type: "choices", choices }),
      startScan,
      stopScan,
      changeReview: (update) => dispatch({ type: "review", update }),
      create,
      goTo: (step) => dispatch({ type: "goTo", step }),
      toggleTarget: (targetId) => dispatch({ type: "toggleTarget", targetId }),
      setRenders: (renders) => dispatch({ type: "renders", renders }),
      setFinishes: (finishes) => dispatch({ type: "finishes", finishes }),
      runTreatments,
      stopTreatments,
      skipTreatments: () => dispatch({ type: "skip" }),
      toggleImage: (key) => dispatch({ type: "toggleImage", key }),
      joinTiff: (on) => dispatch({ type: "joinTiff", on }),
      changeIncidents: (sessionId, action) =>
        dispatch({ type: "incidents", sessionId, action }),
      tickTidy: (on) => dispatch({ type: "tidyTick", on }),
      chooseTidy: (choice) => dispatch({ type: "tidyChoice", choice }),
      setTidyOverview: (overview) =>
        dispatch({ type: "tidyOverview", overview }),
      send,
      finish,
    }),
    [
      state,
      open,
      close,
      abandon,
      startScan,
      stopScan,
      create,
      runTreatments,
      stopTreatments,
      send,
      finish,
    ],
  );

  return (
    <ImportContext.Provider value={api}>{children}</ImportContext.Provider>
  );
}
