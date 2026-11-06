import { useCallback, useEffect, useRef, useState } from "react";

import {
  localWorkCancelKind,
  onPoseStats,
  posesPlan,
  posesRun,
} from "../lib/bridge";
import {
  NO_POSES,
  familiesOfPlan,
  posesOf,
  startRun,
  statsOf,
  withAllDiscarded,
  withPoseDiscarded,
  withPoseEvent,
  type FamiliesPlan,
  type NightPoses,
  type PoseStats,
  type TargetPoses,
} from "../lib/poseStats";

/** What a measure needs to know of a target of the night. */
export interface PoseJob {
  targetId: string;
  nightFolder: string;
  targetFolder: string;
  eveningDate: string;
  longitude: number | null;
  latitude: number | null;
}

export interface PoseStatsControl {
  poses: NightPoses;
  /** What the families are, said before the measure. */
  loadPlan: (job: PoseJob) => Promise<FamiliesPlan | null>;
  /** « Mesurer les poses », or « Reprendre la mesure » (`resume`). */
  start: (job: PoseJob, resume: boolean) => Promise<void>;
  /** « Arrêter la mesure »: what is measured is kept. */
  stop: () => void;
  /** The case « Écarter les poses signalées » (all of them). */
  discardAll: (targetId: string, discard: boolean) => void;
  /** The case of one pose. */
  discardOne: (targetId: string, path: string, discard: boolean) => void;
}

/**
 * The measures of the poses of a night's targets, kept above the steps so
 * that « Fichiers » (where the measure goes) and « Traitements » (where
 * its poses are left out) see the same thing. One measure at a time: the
 * core's events do not name their target.
 */
export function usePoseStats(): PoseStatsControl {
  const [poses, setPoses] = useState<NightPoses>({});
  const current = useRef<NightPoses>({});
  const running = useRef<string | null>(null);

  const update = useCallback(
    (targetId: string, change: (entry: TargetPoses) => TargetPoses) => {
      const next = {
        ...current.current,
        [targetId]: change(posesOf(current.current, targetId)),
      };
      current.current = next;
      setPoses(next);
    },
    [],
  );

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let gone = false;
    void onPoseStats((event) => {
      const target = running.current;
      if (!target) return;
      update(target, (entry) => ({
        ...entry,
        run: withPoseEvent(entry.run, event),
      }));
    })
      .then((off) => {
        if (gone) off();
        else unlisten = off;
      })
      .catch(() => undefined);
    return () => {
      gone = true;
      unlisten?.();
    };
  }, [update]);

  const loadPlan = useCallback(
    async (job: PoseJob): Promise<FamiliesPlan | null> => {
      const known = posesOf(current.current, job.targetId).plan;
      if (known) return known;
      try {
        const plan = await posesPlan({
          nightFolder: job.nightFolder,
          targetFolder: job.targetFolder,
          eveningDate: job.eveningDate,
          longitude: job.longitude,
        });
        update(job.targetId, (entry) => ({ ...entry, plan }));
        return plan;
      } catch {
        return null;
      }
    },
    [update],
  );

  const start = useCallback(
    async (job: PoseJob, resume: boolean) => {
      if (running.current) return;
      const plan = await loadPlan(job);
      if (!plan) return;
      const before = posesOf(current.current, job.targetId);
      const kept: PoseStats | null = resume ? statsOf(before) : null;
      running.current = job.targetId;
      update(job.targetId, (entry) => ({
        ...entry,
        // A new measure forgets what the person had unticked.
        kept: resume ? entry.kept : [],
        run: startRun(plan, kept, Date.now()),
      }));
      try {
        const stats = await posesRun({
          nightFolder: job.nightFolder,
          targetFolder: job.targetFolder,
          eveningDate: job.eveningDate,
          longitude: job.longitude,
          latitude: job.latitude,
          resume: kept?.families.flatMap((family) => family.measured) ?? [],
        });
        update(job.targetId, (entry) => ({
          ...entry,
          run: { status: stats.stopped ? "stopped" : "done", stats },
        }));
      } catch (error) {
        update(job.targetId, (entry) => {
          const partial =
            entry.run.status === "running" ? statsOf(entry) : null;
          return {
            ...entry,
            run: partial
              ? { status: "failed", error, stats: partial }
              : { status: "failed", error, stats: null },
          };
        });
      } finally {
        running.current = null;
      }
    },
    [loadPlan, update],
  );

  const stop = useCallback(() => {
    void localWorkCancelKind("pose-stats").catch(() => undefined);
  }, []);

  const discardAll = useCallback(
    (targetId: string, discard: boolean) =>
      update(targetId, (entry) => withAllDiscarded(entry, discard)),
    [update],
  );
  const discardOne = useCallback(
    (targetId: string, path: string, discard: boolean) =>
      update(targetId, (entry) => withPoseDiscarded(entry, path, discard)),
    [update],
  );

  return { poses, loadPlan, start, stop, discardAll, discardOne };
}

export { NO_POSES, familiesOfPlan };
