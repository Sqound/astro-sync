import { useEffect, useMemo, useState, type ReactNode } from "react";

import { IncidentDialog, IncidentsCard } from "../../components/IncidentsCard";
import { usePlaceZone } from "../../components/PoseStatsCard";
import {
  useIncidents,
  type DraftStore,
  type IncidentsNight,
} from "../../hooks/useIncidents";
import type { DetectionInput, NightView } from "../../lib/bridge";
import { utcOffsetMinutes } from "../../lib/detectPlaces";
import { loadImportClock } from "../../lib/importClock";
import { windowStorage } from "../../lib/lastReport";
import type { Genre, Line } from "../../lib/incidents";
import type { IncidentsPlan } from "../../lib/incidentsDraft";
import { placeOffsetMinutes, type NightPoses } from "../../lib/poseStats";
import type { NightContext } from "./NightWizard";

/**
 * « Liste des incidents » at « Récapitulatif »: the card, its dialog
 * « Modifier l'incident » / « Ajouter un incident », and what it writes --
 * independent of « Envoyer cette nuit ». Shown for a night of the space
 * whose folder is read.
 */
export function IncidentsPanel({
  context,
  view,
  input,
  poses,
  store,
  onPlan,
  onMeasure,
}: {
  context: NightContext;
  view: NightView | null;
  input: DetectionInput;
  poses: NightPoses;
  /** The choices on the list, kept by the wizard (« Précédent » finds them)
   * and written by the primary of the step. */
  store: DraftStore;
  /** What the primary would write, and whether the list is still being read. */
  onPlan: (plan: IncidentsPlan, loading: boolean) => void;
  /** Back to « Fichiers », to the card of the target whose poses are
   * measured there. */
  onMeasure: (targetId: string | null) => void;
}) {
  const detection = view?.detection ?? null;
  const session = context.session;
  // What is found in the files moves onto the session's clock when the
  // import corrected it (the window of the night is already drawn on
  // the files' clock by `input`).
  const shiftSeconds = useMemo(
    () =>
      (loadImportClock(windowStorage(), context.sessionId)?.shiftHours ?? 0) *
      3600,
    [context.sessionId],
  );
  const night = useMemo(() => {
    if (!detection || !session) return null;
    return {
      sessionId: context.sessionId,
      finished: session.status === "finished",
      nightFolder: detection.folder,
      targets: detection.targets
        .filter((target) => target.folderName && target.lightFrames > 0)
        .map((target) => ({
          targetId: target.targetId,
          folderName: target.folderName as string,
        })),
      eveningDate: input.eveningDate ?? "",
      longitude: input.longitude ?? null,
      startedAt: session.startedAt,
      shiftSeconds,
    };
  }, [
    detection,
    session,
    context.sessionId,
    input.eveningDate,
    input.longitude,
    shiftSeconds,
  ]);
  const targetName =
    night && night.targets.length === 1
      ? (detection?.targets.find(
          (x) => x.targetId === night.targets[0].targetId,
        )?.designation ?? null)
      : null;
  return (
    <IncidentsBlock
      night={night}
      poses={poses}
      placeName={context.detail?.night?.place?.value?.name ?? null}
      targetName={targetName}
      store={store}
      withStep="send"
      onPlan={onPlan}
      onMeasure={onMeasure}
    />
  );
}

/**
 * The card of one night and what it needs: the list, its dialog, the writing.
 * Shared by « Récapitulatif » (`IncidentsPanel`) and by the result of
 * « Détecter les sessions » (`ResultIncidents`), where the
 * night is a session the import created.
 */
export function IncidentsBlock({
  night,
  poses,
  placeName,
  targetName,
  selector = null,
  hidden = false,
  store,
  withStep,
  onMeasure,
  onCount,
  onPlan,
}: {
  night: IncidentsNight | null;
  poses: NightPoses;
  placeName: string | null;
  targetName: string | null;
  /** The choice of session under the title. */
  selector?: ReactNode;
  /** Another session's card is the one shown: this one keeps its state. */
  hidden?: boolean;
  /** The choices are kept above the card (by the import, by the night's
   * wizard): they survive « Précédent », and the primary of the step writes
   * them. */
  store: DraftStore;
  /** Which button of the step writes them; the card says so. */
  withStep: "send" | "finish" | "create";
  /** « Mesurer les poses »; `null` where it cannot be done yet. */
  onMeasure: ((targetId: string | null) => void) | null;
  /** How many lines the list holds, once it is read (`null` while it is
   * not). */
  onCount?: (count: number | null) => void;
  /** What the step's primary would write, whenever it changes. */
  onPlan?: (plan: IncidentsPlan, loading: boolean) => void;
}) {
  const incidents = useIncidents(night, poses, store);
  const zone = usePlaceZone(placeName);
  const lines = incidents.view.kind === "list" ? incidents.view.lines : [];
  const count =
    incidents.view.kind === "list"
      ? incidents.view.lines.length
      : incidents.view.kind === "empty"
        ? 0
        : null;
  useEffect(() => {
    onCount?.(count);
  }, [count, onCount]);
  const { plan, loading } = incidents;
  useEffect(() => {
    onPlan?.(plan, loading);
  }, [plan, loading, onPlan]);
  const reference =
    lines[0]?.start ?? Date.parse(night?.startedAt ?? "") / 1000;
  const offset = placeOffsetMinutes(
    zone,
    night?.longitude ?? null,
    Number.isFinite(reference) ? reference : Date.now() / 1000,
    utcOffsetMinutes,
  );
  const [dialog, setDialog] = useState<{ line: Line; adding: boolean } | null>(
    null,
  );
  if (!night) return null;
  const nightStart = Date.parse(night.startedAt) / 1000;
  // The poses to measure first: the first target not measured yet.
  const toMeasure =
    night.targets.find((x) => poses[x.targetId]?.run.status !== "done")
      ?.targetId ??
    night.targets[0]?.targetId ??
    null;

  function blank(genre: Genre): Line {
    const start = Number.isFinite(nightStart) ? nightStart : Date.now() / 1000;
    return {
      id: `manual:${Date.now()}`,
      origin: "detected",
      genre,
      start,
      end: null,
      text: "",
      payload: {},
      evidence: null,
      eventId: null,
      recordedAt: null,
      manual: true,
    };
  }

  return (
    <div hidden={hidden || undefined}>
      <IncidentsCard
        view={incidents.view}
        offset={offset}
        targetName={targetName}
        imported={night.imported}
        noMeasure={night.noMeasure}
        selector={selector}
        withStep={withStep}
        onTick={incidents.tick}
        onQuick={incidents.setQuick}
        onEdit={(line) => setDialog({ line, adding: false })}
        onRemove={incidents.remove}
        onAdd={() => setDialog({ line: blank("note"), adding: true })}
        onRetry={incidents.reload}
        onMeasure={onMeasure ? () => onMeasure(toMeasure) : null}
      />
      {dialog && (
        <IncidentDialog
          line={dialog.line}
          adding={dialog.adding}
          offset={offset}
          base={dialog.line.start}
          lockGenre={dialog.line.origin === "declared"}
          onSave={(line) => {
            if (dialog.adding) incidents.addManual(line);
            else incidents.edit(line);
            setDialog(null);
          }}
          onRemove={() => {
            incidents.remove(dialog.line.id);
            setDialog(null);
          }}
          onClose={() => setDialog(null)}
        />
      )}
    </div>
  );
}
