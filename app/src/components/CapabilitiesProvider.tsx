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
import { capabilities, type CapabilitiesState } from "../lib/bridge";
import { CAPABILITIES_REFRESH_GAP_MS, offerChanged } from "../lib/capabilities";
import { windowOfState, type PaymentFailedWindow } from "../lib/standing";
import { useSnackbar } from "./Snackbar";

interface CapabilitiesApi {
  /** What the paired service grants and says of the billing of the space
   * (`capabilities()`): `null` while it is not read, or unpaired. */
  state: CapabilitiesState | null;
  setState: (state: CapabilitiesState | null) => void;
  /** Reads it again (the network first, the cache offline); answers what it
   * read, `null` when it could not (what was there stays). */
  refresh: () => Promise<CapabilitiesState | null>;
}

const CapabilitiesContext = createContext<CapabilitiesApi>({
  state: null,
  setState: () => undefined,
  refresh: () => Promise.resolve(null),
});

/**
 * The one reading of the capabilities of the window: the profile the screens
 * gate on, and the standing of the space. It is read at the start and each
 * time the connection changes; a file held for the read-only space asks for
 * it again, so a payment that failed since the last reading is said. A
 * failure leaves what was there (the core serves its cache offline).
 *
 * It is also read again when the window comes back to the front (the person
 * bought an offer on the web and returns): no need to quit the application.
 * When the offer turns out to have changed, a snackbar says « Offre mise à
 * jour ».
 */
export function CapabilitiesProvider({ children }: { children: ReactNode }) {
  const { t } = useI18n();
  const { notify } = useSnackbar();
  const [state, setState] = useState<CapabilitiesState | null>(null);
  const held = useRef<CapabilitiesState | null>(null);
  held.current = state;
  const lastRead = useRef(0);
  const refresh = useCallback(() => {
    lastRead.current = Date.now();
    return capabilities()
      .then((next) => {
        if (offerChanged(held.current, next)) {
          notify({
            tone: "success",
            text: t("capabilities.refreshed"),
            key: "capabilities-refreshed",
          });
        }
        held.current = next;
        setState(next);
        return next;
      })
      .catch(() => {
        // Left as it was.
        return null;
      });
  }, [notify, t]);
  useEffect(() => {
    void refresh();
    // Once, at the start: the others come from the window coming back.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  useEffect(() => {
    // The window comes back to the front: read again, at most every few
    // seconds.
    const back = () => {
      if (document.visibilityState === "hidden") return;
      if (Date.now() - lastRead.current < CAPABILITIES_REFRESH_GAP_MS) return;
      void refresh();
    };
    window.addEventListener("focus", back);
    document.addEventListener("visibilitychange", back);
    return () => {
      window.removeEventListener("focus", back);
      document.removeEventListener("visibilitychange", back);
    };
  }, [refresh]);
  const api = useMemo(
    () => ({ state, setState, refresh }),
    [state, setState, refresh],
  );
  return (
    <CapabilitiesContext.Provider value={api}>
      {children}
    </CapabilitiesContext.Provider>
  );
}

export function useCapabilities(): CapabilitiesApi {
  return useContext(CapabilitiesContext);
}

/** The window of a payment that failed while it lasts (the clock moves on a
 * minute at a time), `null` when the space is not in one. */
export function usePaymentFailed(): PaymentFailedWindow | null {
  const { state } = useCapabilities();
  const watching = state?.standing != null;
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (!watching) return;
    setNow(Date.now());
    const timer = setInterval(() => setNow(Date.now()), 60_000);
    return () => clearInterval(timer);
  }, [watching]);
  return windowOfState(state, now);
}
