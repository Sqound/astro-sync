// The calls the primaries of the night's wizard make: what
// `lib/nightApply.ts` runs, with the application's bridge behind them. A
// test hands in its own.

import {
  incidentsApply,
  masterMeasure,
  masterMeasureForget,
  nightPlanAttach,
  nightPlanCreate,
  nightPlanDetach,
  nightTargetsChange,
  personalTargetCreate,
} from "../../lib/bridge";
import type { ApplyFailure, NightApplyDeps } from "../../lib/nightApply";

export const NIGHT_APPLY_DEPS: NightApplyDeps = {
  personalTargetCreate,
  nightTargetsChange,
  nightPlanCreate,
  nightPlanAttach,
  nightPlanDetach,
  masterMeasure,
  masterMeasureForget,
  incidentsApply,
};

/** What a step says when its primary stopped: which choice could not be
 * applied, why, and that unticking it lets the person go on without it. */
export function failureText(
  t: (key: string, params?: Record<string, string | number>) => string,
  failure: ApplyFailure,
): string {
  return t(`wizard.recap.failed.${failure.kind}`, {
    name: failure.name,
    reason: failure.reason,
  });
}
