// What the person chose for the image of a multi-night plan and that stays with
// the plan: the frame kept, and the
// source taken for each night (the master Astro Sync kept of its poses, or the
// image its device stacked). Kept on this computer like the pipelines are, per
// plan -- not per night: the next night of the same plan finds the choice
// again. Pure TypeScript, tested under `node --test`.

import type { PlanFrame, PlanSource } from "./bridge";

export interface PlanKept {
  /** The frame (the four integers of a notch); `null`: the whole image. */
  frame: PlanFrame | null;
  /** The source taken, by line key; a night not named takes its default. */
  sources: Readonly<Record<string, PlanSource>>;
}

export const NOTHING_KEPT: PlanKept = { frame: null, sources: {} };

/** What `localStorage` offers (the tests give their own). */
export interface KeptStorage {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
}

const storageKey = (projectId: string) => `astro-sync.planImage:${projectId}`;

function isFrame(value: unknown): value is PlanFrame {
  if (!value || typeof value !== "object") return false;
  const frame = value as Record<string, unknown>;
  return ["top", "left", "height", "width"].every(
    (key) => Number.isInteger(frame[key]) && (frame[key] as number) >= 0,
  );
}

/** What was kept for a plan; anything that cannot be read is nothing kept. */
export function loadPlanKept(
  storage: KeptStorage | null,
  projectId: string,
): PlanKept {
  try {
    const text = storage?.getItem(storageKey(projectId));
    const parsed = text ? (JSON.parse(text) as unknown) : null;
    if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) {
      return NOTHING_KEPT;
    }
    const { frame, sources } = parsed as Record<string, unknown>;
    const kept: Record<string, PlanSource> = {};
    if (sources && typeof sources === "object" && !Array.isArray(sources)) {
      for (const [key, source] of Object.entries(sources)) {
        if (source === "master" || source === "device") kept[key] = source;
      }
    }
    return {
      frame:
        isFrame(frame) && frame.height > 0 && frame.width > 0 ? frame : null,
      sources: kept,
    };
  } catch {
    return NOTHING_KEPT;
  }
}

export function savePlanKept(
  storage: KeptStorage | null,
  projectId: string,
  kept: PlanKept,
): void {
  try {
    storage?.setItem(storageKey(projectId), JSON.stringify(kept));
  } catch {
    // A choice lost only brings the defaults back.
  }
}
