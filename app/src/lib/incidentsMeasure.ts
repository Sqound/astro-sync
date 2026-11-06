// Whether the card of the incidents has to say that the poses are not
// measured. Pure; tested under `node --test`.

import type { IncidentsNight } from "../hooks/useIncidents.ts";
import type { NightPoses } from "./poseStats";

export interface MeasureState {
  /** No target of the night has its poses measured, and the measure can be
   * made on this screen's session (« Fichiers »). */
  noneMeasured: boolean;
  /** The same, and the night has a target whose poses could be measured:
   * the list says the lines of the measure are not in it. */
  unmeasured: boolean;
}

/** At the result of « Détecter les sessions » (`noMeasure`) the poses are
 * measured elsewhere and the card says so on its own; in a session's
 * « Récapitulatif » it is the measure itself that decides. */
export function measureState(
  night: Pick<IncidentsNight, "noMeasure" | "targets"> | null,
  poses: NightPoses,
): MeasureState {
  if (!night || night.noMeasure)
    return { noneMeasured: false, unmeasured: false };
  const noneMeasured = !night.targets.some(
    (target) => poses[target.targetId]?.run.status === "done",
  );
  return { noneMeasured, unmeasured: noneMeasured && night.targets.length > 0 };
}
