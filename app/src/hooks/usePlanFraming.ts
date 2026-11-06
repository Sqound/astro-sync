import { useEffect, useRef, useState } from "react";

import {
  planFramingRead,
  type PlanFraming,
  type PlanLine,
  type PlanSource,
} from "../lib/bridge";

/**
 * The frames on offer for the image of a plan: read
 * from the record of the last image made with the same integrations, again
 * whenever the integrations ticked or the source of a night change. `null`
 * while nothing is known (before the first image of that set, which is always
 * made whole, or when the window cannot read). Reads only here, never asks
 * the service.
 */
export function usePlanFraming(input: {
  projectId: string;
  lines: readonly PlanLine[];
  chosen: readonly string[];
  sources: Readonly<Record<string, PlanSource>>;
  /** Read at all: the plan's image is wanted and its right is open. */
  enabled: boolean;
}): PlanFraming | null {
  const [framing, setFraming] = useState<PlanFraming | null>(null);
  const latest = useRef(input);
  latest.current = input;
  const key = JSON.stringify([
    input.enabled,
    input.projectId,
    input.chosen,
    input.sources,
    input.lines.map((line) => [
      line.key,
      line.source,
      line.master,
      line.device?.path,
    ]),
  ]);
  useEffect(() => {
    let gone = false;
    const now = latest.current;
    if (!now.enabled || now.chosen.length < 2) {
      setFraming(null);
      return undefined;
    }
    void planFramingRead({
      projectId: now.projectId,
      lines: [...now.lines],
      chosen: [...now.chosen],
      sources: { ...now.sources },
    })
      .then((found) => !gone && setFraming(found))
      .catch(() => !gone && setFraming(null));
    return () => {
      gone = true;
    };
  }, [key]);
  return framing;
}
