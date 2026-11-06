import { useCallback, useEffect, useMemo, useState } from "react";

import { useI18n } from "../../i18n";
import { nightDetail, type Proposal, type WizardStep } from "../../lib/bridge";
import type { NightPoses } from "../../lib/poseStats";
import {
  importedSessions,
  sessionSegment,
  type ImportedSession,
} from "../../lib/incidentsImport";
import type { IncidentsDraft, DraftAction } from "../../lib/incidentsDraft";
import { IncidentsBlock } from "../night/IncidentsPanel";
import type { Created } from "../../lib/importFlow";

/** The result has no measure of the poses. */
const NO_POSES: NightPoses = {};

/**
 * « Liste des incidents » at the result of « Détecter les sessions »: the same card as
 * in the night's « Récapitulatif », one per session the import made or added
 * poses to, with the choice of session under the title when there are
 * several. A session the import made has no signalement declared; one it
 * added poses to may have some (they are listed, and paired with what is
 * detected, as in « Récapitulatif »). Neither has a « Fichiers » step here:
 * the card says the poses were not measured. Every card keeps its own
 * choices while another one is shown, and in the import's store: no card
 * writes by itself, the step's primary writes what is ticked on all of them.
 */
export function ResultIncidents({
  proposal,
  created,
  drafts,
  withStep,
  onDraft,
  onOpenNight,
}: {
  proposal: Proposal;
  created: Created;
  /** The choices on each session's list, kept by the import (its primary
   * writes them; no card writes by itself). */
  drafts: Record<string, IncidentsDraft>;
  /** The step's primary, named in each card's foot. */
  withStep: "send" | "finish" | "create";
  onDraft: (sessionId: string, action: DraftAction) => void;
  /** Opens a session's wizard; `null` while the sessions are not
   * created: « Mesurer les poses » waits for them. */
  onOpenNight:
    | ((
        sessionId: string,
        step: WizardStep | null,
        targetId?: string | null,
      ) => void)
    | null;
}) {
  // The sessions that already existed: a finished one lets a detection
  // replace a signalement declared on it (the others do not).
  const [finished, setFinished] = useState<ReadonlySet<string>>(
    () => new Set(),
  );
  useEffect(() => {
    let gone = false;
    for (const made of created.outcome.sessions) {
      if (made.action !== "updated") continue;
      void nightDetail(made.sessionId)
        .then((detail) => {
          if (gone || detail.session.status !== "finished") return;
          setFinished((held) => new Set([...held, made.sessionId]));
        })
        .catch(() => undefined);
    }
    return () => {
      gone = true;
    };
  }, [created]);
  const sessions = useMemo(
    () =>
      importedSessions(proposal, created.outcome.sessions, finished).sort(
        (a, b) => b.eveningDate.localeCompare(a.eveningDate),
      ),
    [proposal, created, finished],
  );
  const [chosen, setChosen] = useState<string | null>(null);
  const [counts, setCounts] = useState<Record<string, number | null>>({});
  const setCount = useCallback(
    (id: string, count: number | null) =>
      // The count stays once it is known: a card that moved on (« Ajoutés »)
      // keeps the number it had in the selector.
      setCounts((held) =>
        held[id] === count || (count === null && held[id] != null)
          ? held
          : { ...held, [id]: count },
      ),
    [],
  );
  if (sessions.length === 0) return null;
  const shown = sessions.some((s) => s.sessionId === chosen)
    ? chosen
    : sessions[0].sessionId;
  const selector =
    sessions.length > 1 ? (
      <SessionSelector
        sessions={sessions}
        shown={shown}
        counts={counts}
        onChoose={setChosen}
      />
    ) : null;
  return (
    <div className="as-result__incidents">
      {sessions.map((session) => (
        <SessionCard
          key={session.sessionId}
          session={session}
          hidden={session.sessionId !== shown}
          selector={selector}
          draft={drafts[session.sessionId]}
          withStep={withStep}
          onDraft={onDraft}
          onCount={setCount}
          onOpenNight={onOpenNight}
        />
      ))}
    </div>
  );
}

function SessionCard({
  session,
  hidden,
  selector,
  draft,
  withStep,
  onDraft,
  onCount,
  onOpenNight,
}: {
  session: ImportedSession;
  draft: IncidentsDraft | undefined;
  withStep: "send" | "finish" | "create";
  onDraft: (sessionId: string, action: DraftAction) => void;
  hidden: boolean;
  selector: React.ReactNode;
  onCount: (id: string, count: number | null) => void;
  onOpenNight:
    | ((
        sessionId: string,
        step: WizardStep | null,
        targetId?: string | null,
      ) => void)
    | null;
}) {
  const id = session.sessionId;
  const count = useCallback(
    (value: number | null) => onCount(id, value),
    [id, onCount],
  );
  const store = useMemo(
    () => ({ draft, dispatch: (action: DraftAction) => onDraft(id, action) }),
    [draft, id, onDraft],
  );
  return (
    <IncidentsBlock
      night={session.night}
      poses={NO_POSES}
      placeName={session.place}
      targetName={session.targetName}
      hidden={hidden}
      store={store}
      withStep={withStep}
      selector={selector}
      onCount={count}
      // « Mesurer les poses » is a step of the session's wizard: it opens
      // there, at the card of the target.
      onMeasure={
        onOpenNight ? (targetId) => onOpenNight(id, "fichiers", targetId) : null
      }
    />
  );
}

/** One segment per session created: « M45 · 10 oct. · 3 ». */
function SessionSelector({
  sessions,
  shown,
  counts,
  onChoose,
}: {
  sessions: ImportedSession[];
  shown: string | null;
  counts: Record<string, number | null>;
  onChoose: (sessionId: string) => void;
}) {
  const { t, locale } = useI18n();
  return (
    <div
      className="as-incidents__sessions as-segments"
      role="group"
      aria-label={t("incidents.sessions.label")}
    >
      {sessions.map((session) => (
        <button
          key={session.sessionId}
          type="button"
          className={
            session.sessionId === shown ? "as-segment is-chosen" : "as-segment"
          }
          aria-pressed={session.sessionId === shown}
          onClick={() => onChoose(session.sessionId)}
        >
          {sessionSegment(
            locale,
            session,
            counts[session.sessionId] ?? null,
            t("detect.join"),
          )}
        </button>
      ))}
    </div>
  );
}
