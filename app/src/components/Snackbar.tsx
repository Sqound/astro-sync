import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";

import { useI18n } from "../i18n";
import {
  NO_SNACKS,
  dismissSnack,
  dismissSnackKey,
  durationOf,
  pushSnack,
  roleOf,
  visibleSnack,
  type Snack,
  type SnackInput,
  type SnackState,
} from "../lib/snackbar";
import { buttonClassName, progressClassName } from "../theme";
import { BrandMark } from "./BrandMark";
import { Icon } from "./Icon";

/** The custom property the content reads to stop above the snackbar. */
export const CLEARANCE_VAR = "--as-snackbar-clearance";
const CLEARANCE_GAP_PX = 8;

interface SnackbarApi {
  /** Shows a snackbar; its id, to close it later. */
  notify: (input: SnackInput) => number;
  dismiss: (id: number) => void;
  dismissKey: (key: string) => void;
}

const SnackbarContext = createContext<SnackbarApi>({
  notify: () => 0,
  dismiss: () => undefined,
  dismissKey: () => undefined,
});

/** What passes is said by a snackbar. */
export function useSnackbar(): SnackbarApi {
  return useContext(SnackbarContext);
}

/**
 * The snackbars of the window: one small card at the bottom centre, above
 * everything, with the small mark of Astro Sync, its tone said by a thin edge
 * and the colour of the mark (never a solid block), its text, a muted line
 * and a progress bar when it talks about something running, the gesture it
 * offers (a link) and, when it stays, its cross « Fermer ». Several never
 * overlap: the others wait in the queue, what is running first. The region
 * is announced to screen readers (`aria-live`), and the content of the
 * window stops above it (`--as-snackbar-clearance`).
 */
export function SnackbarProvider({ children }: { children: ReactNode }) {
  const [state, setState] = useState<SnackState>(NO_SNACKS);
  const stateRef = useRef(state);
  stateRef.current = state;

  const notify = useCallback((input: SnackInput) => {
    const pushed = pushSnack(stateRef.current, input);
    // The same message again changes nothing, and does not render.
    if (pushed.state !== stateRef.current) {
      stateRef.current = pushed.state;
      setState(pushed.state);
    }
    return pushed.id;
  }, []);
  // The person's cross or the end of a snackbar's time: its owner is told
  // (it may have been closed already -- nothing then).
  const dismiss = useCallback((id: number) => {
    const gone = stateRef.current.snacks.find((snack) => snack.id === id);
    stateRef.current = dismissSnack(stateRef.current, id);
    setState(stateRef.current);
    gone?.onClose?.();
  }, []);
  const dismissKey = useCallback((key: string) => {
    stateRef.current = dismissSnackKey(stateRef.current, key);
    setState(stateRef.current);
  }, []);
  const api = useMemo(
    () => ({ notify, dismiss, dismissKey }),
    [notify, dismiss, dismissKey],
  );

  return (
    <SnackbarContext.Provider value={api}>
      {children}
      <Snackbars snack={visibleSnack(state.snacks)} onDismiss={dismiss} />
    </SnackbarContext.Provider>
  );
}

function Snackbars({
  snack,
  onDismiss,
}: {
  snack: Snack | null;
  onDismiss: (id: number) => void;
}) {
  const { t } = useI18n();
  const region = useRef<HTMLElement | null>(null);
  // The content of the window ends above the snackbar, so that nothing is
  // hidden behind it, whatever the height of the window.
  useEffect(() => {
    const root = document.documentElement;
    const element = region.current;
    if (!snack || !element) {
      root.style.removeProperty(CLEARANCE_VAR);
      return;
    }
    const measure = () => {
      const top = element.getBoundingClientRect().top;
      const room = Math.max(0, window.innerHeight - top) + CLEARANCE_GAP_PX;
      root.style.setProperty(CLEARANCE_VAR, `${Math.ceil(room)}px`);
    };
    measure();
    if (typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(measure);
    observer.observe(element);
    window.addEventListener("resize", measure);
    return () => {
      observer.disconnect();
      window.removeEventListener("resize", measure);
      root.style.removeProperty(CLEARANCE_VAR);
    };
  }, [snack !== null]); // eslint-disable-line react-hooks/exhaustive-deps
  return (
    <section
      ref={region}
      className="as-snackbars"
      aria-label={t("snackbar.region")}
      aria-live="polite"
    >
      {snack && (
        <SnackCard
          key={snack.id}
          snack={snack}
          onDismiss={() => onDismiss(snack.id)}
        />
      )}
    </section>
  );
}

function SnackCard({
  snack,
  onDismiss,
}: {
  snack: Snack;
  onDismiss: () => void;
}) {
  const { t } = useI18n();
  // A timed snackbar waits while the pointer or the keyboard is on it.
  const [held, setHeld] = useState(false);
  const duration = durationOf(snack);
  // The latest way to close, so that the time does not start again whenever
  // the window renders.
  const close = useRef(onDismiss);
  close.current = onDismiss;
  useEffect(() => {
    if (duration === null || held) return;
    const timer = setTimeout(() => close.current(), duration);
    return () => clearTimeout(timer);
  }, [duration, held, snack.text]);

  return (
    <div
      className={`as-snackbar as-snackbar--${snack.tone}`}
      role={roleOf(snack.tone)}
      data-mode={snack.mode}
      onMouseEnter={() => setHeld(true)}
      onMouseLeave={() => setHeld(false)}
      onFocus={() => setHeld(true)}
      onBlur={() => setHeld(false)}
    >
      <BrandMark className="as-snackbar__mark" />
      <div className="as-snackbar__body">
        <p className="as-snackbar__text">{snack.text}</p>
        {snack.detail && <p className="as-snackbar__detail">{snack.detail}</p>}
        {snack.progress !== undefined && (
          <div
            className={`${progressClassName.track} as-snackbar__progress`}
            role="progressbar"
            aria-valuemin={0}
            aria-valuemax={100}
            aria-valuenow={snack.progress}
            aria-label={snack.text}
          >
            <div
              className={progressClassName.fill}
              style={{ width: `${snack.progress}%` }}
            />
          </div>
        )}
      </div>
      {snack.action && (
        <button
          type="button"
          className={`${buttonClassName("link")} as-snackbar__action`}
          onClick={() => snack.action?.run()}
        >
          {snack.action.label}
        </button>
      )}
      {snack.mode === "persistent" && (
        <button
          type="button"
          className="as-snackbar__close"
          aria-label={t("common.close")}
          title={t("common.close")}
          onClick={onDismiss}
        >
          <Icon name="close" className="as-snackbar__close-icon" />
        </button>
      )}
    </div>
  );
}
