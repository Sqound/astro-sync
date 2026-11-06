import { useEffect, useReducer, useState } from "react";

import { onServiceEvent } from "../lib/bridge";
import {
  IDLE,
  elapsedSeconds,
  isWaiting,
  reduceServiceWait,
  type ServiceEvent,
  type ServiceWaitState,
} from "../lib/serviceWait";

/**
 * Whether a call is waiting for the hosted service to start, and for how many
 * seconds -- the state behind the banner every screen shows, for the screens
 * that want to say something of their own while it lasts (the code screen:
 * « Votre code sera vérifié dès que le service répond »).
 */
export function useServiceWaiting(): { waiting: boolean; seconds: number } {
  const [state, dispatch] = useReducer(
    (current: ServiceWaitState, action: { event: ServiceEvent; now: number }) =>
      reduceServiceWait(current, action.event, action.now),
    IDLE,
  );
  const [now, setNow] = useState(() => Date.now());
  const waiting = isWaiting(state);

  useEffect(() => {
    let stopped = false;
    let unlisten: (() => void) | undefined;
    onServiceEvent((event) => dispatch({ event, now: Date.now() }))
      .then((stop) => {
        if (stopped) {
          stop();
        } else {
          unlisten = stop;
        }
      })
      .catch(() => {
        // No event channel (a plain browser preview): nothing to show.
      });
    return () => {
      stopped = true;
      unlisten?.();
    };
  }, []);

  useEffect(() => {
    if (!waiting) {
      return;
    }
    setNow(Date.now());
    const timer = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(timer);
  }, [waiting]);

  return { waiting, seconds: elapsedSeconds(state, now) };
}
