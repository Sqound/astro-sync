import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useReducer,
  useRef,
  useState,
  type ReactNode,
} from "react";

import { useI18n } from "../i18n";
import { useCapabilities } from "./CapabilitiesProvider";
import {
  capabilities,
  isCommandError,
  onSkyEvent,
  onUploadEvent,
  queueStatus,
  skyRetry,
  spaceStorageGet,
  uploadAll,
  uploadPause,
  uploadRequeueMissing,
  uploadResend,
  uploadResume,
  type AlreadySent,
  type QueueEntry,
  type UploadTargetInput,
} from "../lib/bridge";
import { windowStorage } from "../lib/lastReport";
import { READ_ONLY_CODE } from "../lib/standing";
import {
  loadLabels,
  rememberLabels,
  saveLabels,
  type UploadLabels,
} from "../lib/uploadHistory";
import { pluralText } from "../lib/written";
import { useSnackbar } from "./Snackbar";
import {
  NO_UPLOADS,
  hasSomethingToSend,
  uploadNotices,
  uploadsReducer,
  type TrackedFile,
  type UploadOrigin,
  type UploadsAction,
  type UploadsState,
} from "../lib/uploads";

interface UploadsApi {
  state: UploadsState;
  /** The uploads asked for by a gesture (« Envoyer 2 images »): they enter
   * the list, are remembered for the history, and leave right away. The
   * files the enqueueing found already sent to the linked space
   * (`EnqueueOutcome.alreadySent`) end at once: « Déjà envoyé le … ». */
  submit: (
    origin: UploadOrigin,
    files: readonly TrackedFile[],
    target?: UploadTargetInput,
    alreadySent?: readonly AlreadySent[],
  ) => void;
  /** « Renvoyer » a file already sent to the linked space. */
  resend: (sha256: string) => Promise<void>;
  /** Tries again a file that failed (all of them with `null`). */
  retry: (sha256: string | null) => Promise<void>;
  /** Tries again the sky annotations of a file sent: solved here and sent
   * again, the answer comes as an event. */
  retrySky: (sha256: string) => void;
  /** The run is going (a file is leaving or the queue is being walked). */
  running: boolean;
  paused: boolean;
  /** « Tout envoyer » / « Reprendre »: runs the queue. */
  start: () => void;
  /** Files left `queued` or `uploading` by an earlier run (the window was
   * closed, the computer slept): runs them again by itself, unless the person
   * paused, and says so. Does nothing when this computer is not attached. */
  resumePending: () => void;
  pause: () => void;
  dismiss: (sha256: string) => void;
  dismissAll: () => void;
  /** What the person said of the files (for the history), read at once. */
  labels: () => UploadLabels;
  /** Opens the « Envois » page (the shell registers how). */
  openPage: () => void;
  registerOpener: (open: (() => void) | null) => () => void;
}

const UploadsContext = createContext<UploadsApi>({
  state: NO_UPLOADS,
  submit: () => undefined,
  resend: () => Promise.resolve(),
  retry: () => Promise.resolve(),
  retrySky: () => undefined,
  running: false,
  paused: false,
  start: () => undefined,
  resumePending: () => undefined,
  pause: () => undefined,
  dismiss: () => undefined,
  dismissAll: () => undefined,
  labels: () => ({}),
  openPage: () => undefined,
  registerOpener: () => () => undefined,
});

export function useUploads(): UploadsApi {
  return useContext(UploadsContext);
}

/**
 * Listens once to the uploader's events (`astro-sync://upload-event`, emitted
 * by every run, whoever started it) and keeps the list of the uploads of the
 * window: the page « Envois », the badge on its menu entry and the banner of
 * the result read it; the notifications go through the window's snackbar (one
 * at the bottom, the upload on its way first). It also starts the run for the
 * files a gesture queues -- queuing a file never sent it by itself.
 */
export function UploadsProvider({ children }: { children: ReactNode }) {
  const { t, locale } = useI18n();
  const { notify, dismissKey } = useSnackbar();
  const tRef = useRef({ t, locale, notify });
  tRef.current = { t, locale, notify };
  const [state, dispatch] = useReducer(uploadsReducer, NO_UPLOADS);
  const stateRef = useRef(state);
  stateRef.current = state;
  // A payment that failed keeps the space read-only: the files held for
  // `entitlement_read_only` say so, with the day the window ends.
  const { state: capabilitiesState, refresh: refreshCapabilities } =
    useCapabilities();
  const failedUntil =
    capabilitiesState?.paired &&
    capabilitiesState.standing?.kind === "paymentFailed"
      ? capabilitiesState.standing.readOnlyUntil
      : null;
  useEffect(() => {
    dispatch({ type: "standing", until: failedUntil });
  }, [failedUntil]);
  const refreshRef = useRef(refreshCapabilities);
  refreshRef.current = refreshCapabilities;
  const lastStandingRead = useRef(0);
  const [running, setRunning] = useState(false);
  const [paused, setPausedState] = useState(false);
  const setPaused = useCallback((value: boolean) => {
    pausedRef.current = value;
    setPausedState(value);
  }, []);
  const runningRef = useRef(false);
  const again = useRef(false);
  const afterOther = useRef(false);
  const pausedRef = useRef(false);
  const targetRef = useRef<UploadTargetInput>({ targetIds: [] });
  const drainRef = useRef<() => Promise<void>>(async () => undefined);
  const labels = useRef<UploadLabels | null>(null);
  const opener = useRef<(() => void) | null>(null);
  const openRef = useRef<() => void>(() => undefined);
  const noticeKeys = useRef<Set<string>>(new Set());

  const readLabels = useCallback((): UploadLabels => {
    if (labels.current === null) labels.current = loadLabels(windowStorage());
    return labels.current;
  }, []);

  const reconcile = useCallback(() => {
    queueStatus()
      .then((status) => {
        dispatch({ type: "reconcile", entries: status.entries });
        dispatch({ type: "skySeed", annotations: status.sky ?? [] });
      })
      .catch(() => undefined);
  }, []);

  const drain = useCallback(
    async (quiet = false) => {
      if (runningRef.current) {
        again.current = true;
        return;
      }
      // A run another screen started (a night's summary) is walking the queue:
      // two runs would take the same file. This one starts when that one stops.
      if (stateRef.current.rows.some((row) => row.status === "uploading")) {
        afterOther.current = true;
        return;
      }
      runningRef.current = true;
      setRunning(true);
      setPaused(false);
      try {
        do {
          again.current = false;
          try {
            await uploadAll(targetRef.current);
          } catch (caught) {
            // A computer that is not attached has nothing to say about files it
            // resumed by itself.
            if (quiet && isCommandError(caught) && caught.code === "not_paired")
              break;
            dispatch({
              type: "runFailed",
              code: isCommandError(caught) ? caught.code : "unknown_error",
              message: isCommandError(caught) ? caught.message : String(caught),
            });
          }
        } while (again.current);
      } finally {
        runningRef.current = false;
        setRunning(false);
        // A run that found nothing to send emits nothing about the files the
        // window follows: what the queue says of them ends their rows.
        reconcile();
      }
    },
    [reconcile],
  );

  drainRef.current = drain;

  const resumePending = useCallback(
    (known?: readonly QueueEntry[]) => {
      void (async () => {
        if (pausedRef.current || runningRef.current) return;
        try {
          const entries = known ?? (await queueStatus()).entries;
          const pending = entries.filter(
            (entry) => entry.state === "queued" || entry.state === "uploading",
          );
          if (pending.length === 0) return;
          // Not attached to a space: nothing can leave, and nothing is said.
          if (!(await capabilities()).paired) return;
          if (pausedRef.current || runningRef.current) return;
          dispatch({ type: "seed", entries });
          const { t: say, locale: language, notify: tell } = tRef.current;
          tell({
            tone: "info",
            text: pluralText(say, language, "uploads.resumed", pending.length),
            action: {
              label: say("uploads.open"),
              run: () => openRef.current(),
            },
          });
          void drain(true);
        } catch {
          // The queue or the service cannot be read: the page says it.
        }
      })();
    },
    [drain],
  );

  useEffect(() => {
    let stopped = false;
    let unlisten: (() => void) | undefined;
    // What an earlier run of the window left in the queue.
    queueStatus()
      .then((status) => {
        if (stopped) return;
        dispatch({ type: "seed", entries: status.entries });
        dispatch({ type: "skySeed", annotations: status.sky ?? [] });
        // ... and goes on with them, unless the person paused.
        resumePending(status.entries);
      })
      .catch(() => undefined);
    onUploadEvent((event) => {
      dispatch({ type: "event", event, now: Date.now() });
      // A file the service holds back because the space is read-only: read the
      // standing again (at most once a minute), so that a payment that failed
      // since the last reading explains it.
      if (
        (event.kind === "Held" || event.kind === "Refused") &&
        event.code === READ_ONLY_CODE &&
        Date.now() - lastStandingRead.current > 60_000
      ) {
        lastStandingRead.current = Date.now();
        refreshRef.current();
      }
      if (event.kind === "Refused" && event.code === "file_too_large") {
        spaceStorageGet()
          .then((storage) =>
            dispatch({
              type: "limit",
              sha256: event.sha256,
              bytes: storage.maxFileBytes,
            }),
          )
          .catch(() => undefined);
      }
      if (event.kind === "Stopped") {
        if (event.reason === "paused" || event.reason === "cancelled")
          setPaused(true);
        else if (afterOther.current && !runningRef.current) {
          afterOther.current = false;
          setTimeout(() => void drainRef.current(), 50);
        }
      }
    })
      .then((stop) => {
        if (stopped) stop();
        else unlisten = stop;
      })
      .catch(() => {
        // No event channel (a plain browser preview): nothing to follow.
      });
    // The sky annotations that follow each upload.
    let unlistenSky: (() => void) | undefined;
    onSkyEvent((annotation) => dispatch({ type: "sky", annotation }))
      .then((stop) => {
        if (stopped) stop();
        else unlistenSky = stop;
      })
      .catch(() => undefined);
    return () => {
      stopped = true;
      unlisten?.();
      unlistenSky?.();
    };
  }, [resumePending, setPaused]);

  // A file reached an end: what the queue now says of the failures settles
  // whether trying again is possible.
  useEffect(() => {
    if (state.revision > 0) reconcile();
  }, [state.revision, reconcile]);

  const submit = useCallback(
    (
      origin: UploadOrigin,
      files: readonly TrackedFile[],
      target?: UploadTargetInput,
      alreadySent?: readonly AlreadySent[],
    ) => {
      // The files carry their own attribution; the run's target is what they
      // lack (the session's plan).
      targetRef.current = target ?? { targetIds: [] };
      dispatch({ type: "track", origin, files });
      if (alreadySent && alreadySent.length > 0)
        dispatch({ type: "alreadySent", files: alreadySent });
      const next = rememberLabels(readLabels(), files, Date.now());
      labels.current = next;
      saveLabels(windowStorage(), next);
      void uploadResume().catch(() => undefined);
      void drain();
    },
    [drain, readLabels],
  );
  const retry = useCallback(
    async (sha256: string | null) => {
      dispatch({ type: "requeue", sha256 });
      // A file the queue could not find goes back in the queue first (the
      // others are queued already).
      await uploadRequeueMissing(sha256).catch(() => 0);
      void drain();
    },
    [drain],
  );
  const retrySky = useCallback((sha256: string) => {
    dispatch({ type: "skyRetry", sha256 });
    skyRetry(sha256).catch(() => undefined);
  }, []);
  const resend = useCallback(
    async (sha256: string) => {
      const requeued = await uploadResend(sha256).catch(() => false);
      if (!requeued) return;
      dispatch({ type: "resend", sha256 });
      void uploadResume().catch(() => undefined);
      void drain();
    },
    [drain],
  );
  // « Tout envoyer »: when the queue of the linked space holds nothing to
  // send, nothing starts -- the button does not flicker -- and a notice says
  // why; the rows the window followed end with what the queue says of them.
  const start = useCallback(() => {
    void (async () => {
      if (!pausedRef.current) {
        const status = await queueStatus().catch(() => null);
        if (status !== null && !hasSomethingToSend(status.entries)) {
          dispatch({ type: "reconcile", entries: status.entries });
          const { t: say, notify: tell } = tRef.current;
          tell({ tone: "info", text: say("uploads.nothingToSend") });
          return;
        }
      }
      void uploadResume().catch(() => undefined);
      void drain();
    })();
  }, [drain]);
  const pause = useCallback(() => {
    void uploadPause().catch(() => undefined);
  }, []);
  const resumeNow = useCallback(() => resumePending(), [resumePending]);
  const send = useCallback((action: UploadsAction) => dispatch(action), []);
  const dismiss = useCallback(
    (sha256: string) => send({ type: "dismiss", sha256 }),
    [send],
  );
  const dismissAll = useCallback(() => send({ type: "dismissAll" }), [send]);
  const registerOpener = useCallback((open: (() => void) | null) => {
    opener.current = open;
    return () => {
      if (opener.current === open) opener.current = null;
    };
  }, []);
  const openPage = useCallback(() => opener.current?.(), []);
  openRef.current = openPage;

  // The notifications of the uploads are the window's snackbar: the one on
  // its way (bar and speed), what was refused or failed (with « Réessayer »),
  // and the files sent. What no longer needs saying goes.
  useEffect(() => {
    const wanted = uploadNotices(t, locale, state, {
      open: openPage,
      retry: (sha256) => void retry(sha256),
      close: (sha256s) => {
        for (const sha256 of sha256s) send({ type: "dismiss", sha256 });
      },
    });
    const keys = new Set<string>();
    for (const notice of wanted) {
      if (notice.key) keys.add(notice.key);
      notify(notice);
    }
    for (const key of noticeKeys.current) if (!keys.has(key)) dismissKey(key);
    noticeKeys.current = keys;
  }, [state, t, locale, notify, dismissKey, openPage, retry, send]);

  const api = useMemo(
    () => ({
      state,
      submit,
      resend,
      retry,
      retrySky,
      running,
      paused,
      start,
      resumePending: resumeNow,
      pause,
      dismiss,
      dismissAll,
      labels: readLabels,
      openPage,
      registerOpener,
    }),
    [
      state,
      submit,
      resend,
      retry,
      retrySky,
      running,
      paused,
      start,
      resumeNow,
      pause,
      dismiss,
      dismissAll,
      readLabels,
      openPage,
      registerOpener,
    ],
  );
  return (
    <UploadsContext.Provider value={api}>{children}</UploadsContext.Provider>
  );
}
