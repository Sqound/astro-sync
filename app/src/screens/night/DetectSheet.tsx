import { useEffect, useMemo, useRef, useState } from "react";

import { FileModeChoice } from "../../components/FileModeChoice";
import { Icon } from "../../components/Icon";
import { Notice } from "../../components/Notice";
import { ScanHead } from "../../components/ScanHead";
import { useI18n } from "../../i18n";
import {
  isCommandError,
  localWorkCancel,
  localWorkCancelKind,
  onLocalWork,
  sessionsDetect,
  sessionsOrganise,
  sessionsOrganisePlan,
  sessionsUserData,
  targetsSearch,
  type OrganiseMode,
  type OrganiseOutcome,
  type OrganisePlan,
  type Proposal,
  type SessionScope,
  type WorkProgress,
} from "../../lib/bridge";
import { isCancelled } from "../../lib/detectFlow";
import { withUtcOffsets } from "../../lib/detectPlaces";
import { formatBytes, formatError } from "../../lib/format";
import { formatMinutes } from "../../lib/written";
import { displayPath, homeOf } from "../../lib/home";
import {
  emphasise,
  organiseCounts,
  organiseEnd,
  organisePercent,
  organisePrimary,
  phaseKey,
  sourceFolder,
  type TextPart,
} from "../../lib/organise";
import {
  READER_NAME,
  buildRequest,
  calibrationFiles,
  intoNightFolder,
  modeSentence,
  organiseProposal,
  relativeTo,
  scopeKey,
  sheetModel,
  treeFolders,
  utcDay,
  utcTime,
  type CatalogueExtras,
  type DetectAnswer,
  type SendLocations,
  type SheetFile,
  type SheetGroup,
} from "../../lib/wizardDetect";
import { sheetUnreadText, unreadCounts } from "../../lib/missingMessages";
import { pluralText } from "../../lib/written";
import {
  badgeClassName,
  buttonClassName,
  helpTextClassName,
  progressClassName,
} from "../../theme";

type Phase =
  | { kind: "reading" }
  | { kind: "ready"; proposal: Proposal }
  | { kind: "failed"; message: string };

type Running = {
  mode: OrganiseMode;
  plan: OrganisePlan;
  proposal: Proposal;
  progress: WorkProgress | null;
};

/** How many rows of a list show before it scrolls. */
const VISIBLE_ROWS = 7;

function Parts({ parts }: { parts: TextPart[] }) {
  return (
    <>
      {parts.map((part, index) =>
        part.code ? (
          <code key={index} className="as-code">
            {part.text}
          </code>
        ) : (
          <span key={index}>{part.text}</span>
        ),
      )}
    </>
  );
}

/**
 * The detection sheet of the sending of a session, opened over « Fichiers »: the
 * engine's detection bounded to the session (`scope`), the session's targets
 * with their frames, the night's calibration and the logs tied to it, what
 * matches none of the targets, then the mode of the files and « Suivant ».
 * `initial`: the proposal of an earlier answer (« Revoir la détection »),
 * kept while the scope is the same; `nightFolder`: where the tidying goes;
 * `onNext`: the person's answer; `onCancel`: « Annuler »; `onAddTarget`: the
 * target added to the session (the scope then changes and the sheet reads
 * again).
 */
export function DetectSheet({
  locations,
  scope,
  nightFolder,
  initial,
  onNext,
  onCancel,
  onAddTarget,
}: {
  locations: SendLocations;
  scope: SessionScope;
  nightFolder: string | null;
  initial: Proposal | null;
  onNext: (answer: DetectAnswer) => void;
  onCancel: () => void;
  onAddTarget: (targetId: string) => Promise<void>;
}) {
  const { t, locale } = useI18n();
  const key = scopeKey(scope);
  const [phase, setPhase] = useState<Phase>(
    initial ? { kind: "ready", proposal: initial } : { kind: "reading" },
  );
  const [ran, setRan] = useState<string | null>(initial ? key : null);
  const [attempt, setAttempt] = useState(0);
  const [progress, setProgress] = useState({
    read: 0,
    total: 0,
    walked: 0,
    folder: null as string | null,
  });
  const [mode, setMode] = useState<OrganiseMode>("leave-in-place");
  const [plans, setPlans] = useState<
    Partial<Record<OrganiseMode, OrganisePlan>>
  >({});
  const [windowsNoLink, setWindowsNoLink] = useState(false);
  const [open, setOpen] = useState<ReadonlySet<string>>(
    () => new Set(scope.targets.slice(0, 1).map((target) => `t:${target.id}`)),
  );
  const [confirming, setConfirming] = useState(false);
  const [running, setRunning] = useState<Running | null>(null);
  const [finished, setFinished] = useState<{
    mode: OrganiseMode;
    plan: OrganisePlan;
    outcome: OrganiseOutcome;
    proposal: Proposal;
  } | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [adding, setAdding] = useState<string | null>(null);
  const alive = useRef(true);
  const run = useRef(0);
  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
    };
  }, []);

  // The detection, bounded to the session: at opening and each time its
  // scope changes (a target added). The headers' progress is on the bridge's
  // event; the places' offsets are the window's to compute.
  useEffect(() => {
    if (ran === key && attempt === 0) return;
    const mine = ++run.current;
    setPhase({ kind: "reading" });
    setProgress({ read: 0, total: 0, walked: 0, folder: null });
    setPlans({});
    let off: (() => void) | null = null;
    let gone = false;
    void onLocalWork((work) => {
      if (work.kind === "scan") {
        // The search for the files, before any header (no total yet).
        setProgress((current) =>
          current.total > 0
            ? current
            : { ...current, walked: work.done, folder: work.current ?? null },
        );
      }
      if (work.kind === "headers") {
        setProgress((current) => ({
          ...current,
          read: work.done,
          total: work.total,
        }));
      }
    })
      .then((unlisten) => {
        if (gone) unlisten();
        else off = unlisten;
      })
      .catch(() => undefined);
    void (async () => {
      try {
        const [user, extras] = await Promise.all([
          sessionsUserData().then((data) => withUtcOffsets(data, new Date())),
          catalogueExtras(scope, locale),
        ]);
        const bounded: SessionScope = {
          ...scope,
          targets: scope.targets.map((target) => ({
            ...target,
            aliases: extras[target.id]?.aliases ?? target.aliases,
            raHours: extras[target.id]?.raHours ?? target.raHours ?? null,
            decDegrees:
              extras[target.id]?.decDegrees ?? target.decDegrees ?? null,
          })),
        };
        const proposal = await sessionsDetect(
          buildRequest(locations, bounded),
          user,
          locale,
        );
        if (mine !== run.current || !alive.current) return;
        setPhase({ kind: "ready", proposal });
        setRan(key);
      } catch (caught) {
        if (mine !== run.current || !alive.current) return;
        setRan(key);
        if (isCommandError(caught) && isCancelled(caught.code)) {
          onCancel();
          return;
        }
        setPhase({ kind: "failed", message: formatError(t, caught) });
      }
    })();
    return () => {
      gone = true;
      off?.();
    };
    // `scope` is read through its key: a new object with the same content
    // does not read again.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key, attempt, locations]);

  const proposal = phase.kind === "ready" ? phase.proposal : null;
  const model = useMemo(
    () => (proposal ? sheetModel(proposal, locations.fits) : null),
    [proposal, locations.fits],
  );
  const tidy = useMemo(
    () => (proposal ? organiseProposal(proposal) : null),
    [proposal],
  );

  // The plan of the mode chosen, for this night's folder; « Lien
  // symbolique » is asked at the start too, for the platform's flag.
  useEffect(() => {
    if (!tidy || !nightFolder) return;
    let current = true;
    const wanted: OrganiseMode[] = ["symlink"];
    if (mode !== "leave-in-place" && mode !== "symlink") wanted.push(mode);
    for (const asked of wanted) {
      if (plans[asked]) continue;
      sessionsOrganisePlan(tidy, nightFolder, asked, null)
        .then((plan) => {
          if (!current) return;
          setPlans((known) => ({ ...known, [asked]: plan }));
          if (asked === "symlink") {
            setWindowsNoLink(plan.windowsNoLink);
            if (plan.windowsNoLink) {
              setMode((chosen) =>
                chosen === "symlink" ? "leave-in-place" : chosen,
              );
            }
          }
        })
        .catch((caught) => current && setError(formatError(t, caught)));
    }
    return () => {
      current = false;
    };
    // `plans` only gates what is asked.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [tidy, nightFolder, mode]);

  const fmt = (value: number) => new Intl.NumberFormat(locale).format(value);
  const seconds = (values: number[]) =>
    values
      .map((value) => t("common.secondsShort", { count: fmt(value) }))
      .join(", ");
  const toggle = (id: string) =>
    setOpen((current) => {
      const next = new Set(current);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });

  const plan = mode === "leave-in-place" ? null : (plans[mode] ?? null);
  const counts = plan ? organiseCounts(plan) : null;
  const ready = mode === "leave-in-place" || Boolean(plan && nightFolder);
  const primary = organisePrimary(mode);
  const unassignedFrames = model
    ? model.unassigned.reduce((sum, group) => sum + group.frames, 0) +
      (model.withoutObject?.frames ?? 0)
    : 0;

  function answer(organised: boolean, outcomeProposal: Proposal) {
    onNext({ mode, proposal: outcomeProposal, organised, scopeKey: key });
  }

  async function startTidying() {
    if (!proposal || !plan || !nightFolder) return;
    const moved = intoNightFolder(plan, nightFolder);
    setError(null);
    setConfirming(false);
    setRunning({ mode, plan: moved, proposal, progress: null });
    const listener = onLocalWork((work) => {
      if (work.kind === "organise" && alive.current) {
        setRunning((current) =>
          current ? { ...current, progress: work } : current,
        );
      }
    }).catch(() => null);
    try {
      const outcome = await sessionsOrganise(moved);
      if (!alive.current) return;
      setRunning(null);
      if (outcome.stopped || outcome.failed.length > 0) {
        setFinished({ mode, plan: moved, outcome, proposal });
      } else {
        onNext({ mode, proposal, organised: true, scopeKey: key });
      }
    } catch (caught) {
      if (!alive.current) return;
      setRunning(null);
      setError(formatError(t, caught));
    } finally {
      void listener.then((unlisten) => unlisten?.());
    }
  }

  function next() {
    if (!proposal) return;
    if (mode === "leave-in-place") answer(false, proposal);
    else if (mode === "move") setConfirming(true);
    else void startTidying();
  }

  async function add(group: SheetGroup) {
    setAdding(group.key);
    setError(null);
    try {
      const id =
        group.targetId ?? (await resolveTarget(group.designation, locale));
      if (!id) return;
      await onAddTarget(id);
    } catch (caught) {
      if (alive.current) setError(formatError(t, caught));
    } finally {
      if (alive.current) setAdding(null);
    }
  }

  function cancel() {
    if (phase.kind === "reading") void localWorkCancel().catch(() => undefined);
    run.current += 1;
    onCancel();
  }

  // --- the tidying runs ---------------------------------------------------
  if (running) {
    const percent = running.progress ? organisePercent(running.progress) : 0;
    return (
      <div className="as-dialog__scrim" role="presentation">
        <section
          className="as-dialog as-organise"
          role="dialog"
          aria-modal="true"
          aria-labelledby="send-organise-title"
        >
          <h2 id="send-organise-title" className="as-dialog__title">
            {t("organise.progress")}
          </h2>
          <div className="as-organise__phase">
            <span>{t(phaseKey(running.mode))}</span>
            <span className="as-organise__count">
              {t("organise.count", {
                done: fmt(running.progress?.done ?? 0),
                total: fmt(running.plan.files),
                size: formatBytes(t, running.progress?.bytesDone ?? 0, locale),
                sizeTotal: formatBytes(t, running.plan.bytes, locale),
              })}
            </span>
          </div>
          <div
            className={progressClassName.track}
            role="progressbar"
            aria-valuemin={0}
            aria-valuemax={100}
            aria-valuenow={percent}
            aria-label={t("organise.progress")}
          >
            <div
              className={progressClassName.fill}
              style={{ width: `${percent}%` }}
            />
          </div>
          {running.mode === "move" && (
            <p className={helpTextClassName}>{t("organise.progressNote")}</p>
          )}
          <div className="as-organise__foot as-organise__foot--end">
            <button
              type="button"
              className={buttonClassName("link")}
              onClick={() => {
                void localWorkCancelKind("organise").catch(() => {});
              }}
            >
              {t("organise.stop")}
            </button>
          </div>
        </section>
      </div>
    );
  }

  // --- the tidying stopped or left files behind ---------------------------
  if (finished) {
    const end = organiseEnd(finished.mode, finished.outcome);
    const source = sourceFolder(finished.plan);
    return (
      <div className="as-dialog__scrim" role="presentation">
        <section
          className="as-dialog as-organise"
          role="dialog"
          aria-modal="true"
          aria-labelledby="send-organise-title"
        >
          <h2 id="send-organise-title" className="as-dialog__title">
            {end.kind === "stopped"
              ? t("organise.stopped")
              : pluralText(
                  t,
                  locale,
                  {
                    moved: "organise.moved",
                    copied: "organise.copied",
                    linked: "organise.linked",
                  }[end.kind],
                  end.count,
                  { count: fmt(end.count) },
                )}
          </h2>
          {end.kind === "stopped" && (
            <p className={helpTextClassName}>
              {t("organise.stoppedAt", {
                done: fmt(end.count),
                total: fmt(finished.plan.files),
              })}
            </p>
          )}
          {end.failed > 0 && (
            <div className="as-organise__line as-organise__line--warning">
              <Icon name="warning" className="as-organise__glyph" />
              <span>
                <Parts
                  parts={emphasise(
                    pluralText(t, locale, "organise.failed", end.failed, {
                      count: fmt(end.failed),
                      path: source,
                    }),
                    [source],
                  )}
                />
              </span>
            </div>
          )}
          <div className="as-organise__foot as-organise__foot--end">
            <button
              type="button"
              className={buttonClassName("primary")}
              onClick={() =>
                onNext({
                  mode: finished.mode,
                  proposal: finished.proposal,
                  organised: true,
                  scopeKey: key,
                })
              }
            >
              {t("organise.done")}
            </button>
          </div>
        </section>
      </div>
    );
  }

  // --- « Déplacer »: the summary before acting ------------------------------
  if (confirming && plan && counts && nightFolder) {
    const source = sourceFolder(plan);
    const home = homeOf(nightFolder);
    const targets = model?.targets.map((target) => target.designation) ?? [];
    return (
      <div className="as-dialog__scrim" role="presentation">
        <section
          className="as-dialog as-send-move"
          role="dialog"
          aria-modal="true"
          aria-labelledby="send-move-title"
        >
          <h2 id="send-move-title" className="as-dialog__title">
            {t("wizard.detect.moveTitle", { count: fmt(plan.files) })}
          </h2>
          <p className="as-send-move__summary">
            {t("wizard.detect.moveSummary", {
              frames: fmt(counts.frames),
              targets: targets.join(", "),
              calibration: fmt(counts.calibration),
              logs: fmt(counts.logs),
              size: formatBytes(t, plan.bytes, locale),
            })}
            <br />
            <code className="as-code">
              {displayPath(source, homeOf(source))}
            </code>
            {" → "}
            <code className="as-code">{displayPath(nightFolder, home)}</code>
          </p>
          <ol className="as-send-move__steps">
            {[1, 2, 3].map((step) => (
              <li key={step}>{t(`wizard.detect.moveStep${step}`)}</li>
            ))}
          </ol>
          <p className={helpTextClassName}>
            {unassignedFrames > 0
              ? pluralText(
                  t,
                  locale,
                  "wizard.detect.moveStays",
                  unassignedFrames,
                  {
                    count: fmt(unassignedFrames),
                  },
                )
              : t("wizard.detect.moveStaysNone")}
          </p>
          <div className="as-organise__foot as-organise__foot--end">
            <button
              type="button"
              className={buttonClassName("link")}
              onClick={() => setConfirming(false)}
            >
              {t("wizard.back")}
            </button>
            <button
              type="button"
              className={buttonClassName("primary", { destructive: true })}
              onClick={() => void startTidying()}
            >
              {pluralText(t, locale, primary.key, plan.files, {
                count: fmt(plan.files),
              })}
            </button>
          </div>
        </section>
      </div>
    );
  }

  // --- the sheet ------------------------------------------------------------
  const sentence = modeSentence(mode);
  const folders = plan ? treeFolders(plan) : [];
  const filterless = folders.find((folder) => /^[^/]+\/lights\/$/.test(folder));
  const camera =
    proposal?.sessions
      .flatMap((session) => session.targets)
      .map((target) => target.instrument.split(" · ")[0])
      .find((name) => name && name !== "?") ?? "";
  const sentenceText = t(sentence.key, {
    frames: fmt(counts?.frames ?? 0),
    folders: folders.join(", "),
    files: fmt(plan?.files ?? 0),
    size: plan ? formatBytes(t, plan.bytes, locale) : "",
    path: locations.fits
      ? displayPath(locations.fits, homeOf(locations.fits))
      : "",
  });

  return (
    <div className="as-dialog__scrim" role="presentation">
      <section
        className="as-dialog as-send-sheet"
        role="dialog"
        aria-modal="true"
        aria-labelledby="send-sheet-title"
      >
        <h2 id="send-sheet-title" className="as-dialog__title">
          {t("wizard.detect.title")}
        </h2>
        {phase.kind === "reading" && (
          <div className="as-send-sheet__reading">
            <ScanHead
              read={progress.read}
              total={progress.total}
              walked={progress.walked}
              folder={progress.folder}
            />
            <div className="as-actions">
              <button
                type="button"
                className={buttonClassName("link")}
                onClick={cancel}
              >
                {t("detect.stop")}
              </button>
            </div>
          </div>
        )}
        {phase.kind === "failed" && (
          <>
            <Notice tone="error">{phase.message}</Notice>
            <div className="as-actions">
              <button
                type="button"
                className={buttonClassName("link")}
                onClick={cancel}
              >
                {t("common.cancel")}
              </button>
              <button
                type="button"
                className={buttonClassName("secondary")}
                onClick={() => setAttempt((current) => current + 1)}
              >
                {t("wizard.retry")}
              </button>
            </div>
          </>
        )}
        {proposal && model && (
          <>
            <p className="as-send-sheet__lead">
              {locations.fits && (
                <>
                  <code className="as-code">
                    {displayPath(locations.fits, homeOf(locations.fits))}
                  </code>
                  {" · "}
                </>
              )}
              {t("wizard.detect.lead", {
                date: utcDay(scope.from, locale),
                start: utcTime(scope.from, locale),
                end: utcTime(scope.to, locale),
              })}
            </p>
            <div className="as-send-sheet__body">
              {model.targets.map((target) => (
                <Fold
                  key={target.id}
                  id={`t:${target.id}`}
                  open={open}
                  onToggle={toggle}
                  head={
                    <>
                      <strong className="as-send-fold__name">
                        {target.designation}
                      </strong>
                      <span className={badgeClassName("success")}>
                        <Icon name="check" />
                        {pluralText(
                          t,
                          locale,
                          "wizard.files.frames",
                          target.frames,
                          {
                            count: fmt(target.frames),
                          },
                        )}
                      </span>
                      <span className={helpTextClassName}>
                        {[
                          target.exposures.length > 0
                            ? seconds(target.exposures)
                            : null,
                          target.totalSeconds > 0
                            ? formatMinutes(t, target.totalSeconds / 60)
                            : null,
                        ]
                          .filter(Boolean)
                          .join(" · ")}
                      </span>
                    </>
                  }
                >
                  <FileList
                    files={target.files}
                    withDetails
                    seconds={seconds}
                  />
                </Fold>
              ))}
              {model.calibration.length > 0 && (
                <Fold
                  id="calibration"
                  open={open}
                  onToggle={toggle}
                  head={
                    <>
                      <strong className="as-send-fold__name">
                        {t("wizard.detect.calibration")}
                      </strong>
                      <span className={helpTextClassName}>
                        {model.calibration
                          .map((item) =>
                            pluralText(
                              t,
                              locale,
                              `wizard.detect.cal.${item.kind}`,
                              item.frames,
                              { count: fmt(item.frames) },
                            ),
                          )
                          .join(" · ")}
                      </span>
                    </>
                  }
                >
                  <FileList
                    files={calibrationFiles(proposal).map((path) => ({
                      name: relativeTo(path, null),
                      path,
                      exposureSeconds: null,
                      filter: null,
                      utc: null,
                    }))}
                    seconds={seconds}
                  />
                </Fold>
              )}
              {model.logs.files.length > 0 && (
                <Fold
                  id="logs"
                  open={open}
                  onToggle={toggle}
                  head={
                    <>
                      <strong className="as-send-fold__name">
                        {t("wizard.detect.logs")}
                      </strong>
                      <span className={helpTextClassName}>
                        {pluralText(
                          t,
                          locale,
                          "wizard.detect.logsLine",
                          model.logs.files.length,
                          {
                            count: fmt(model.logs.files.length),
                            readers: model.logs.readers
                              .map((kind) => READER_NAME[kind] ?? kind)
                              .join(", "),
                            date: utcDay(scope.from, locale),
                          },
                        )}
                      </span>
                    </>
                  }
                >
                  <FileList
                    files={model.logs.files.map((file) => ({
                      name: file.name,
                      path: file.path,
                      exposureSeconds: null,
                      filter: null,
                      utc: null,
                    }))}
                    seconds={seconds}
                  />
                </Fold>
              )}
              {unassignedFrames > 0 && (
                <section className="as-send-unassigned">
                  <p className="as-send-unassigned__head">
                    <strong>{t("wizard.detect.unassigned")}</strong>
                    <span className={helpTextClassName}>
                      {" · "}
                      {pluralText(
                        t,
                        locale,
                        "wizard.detect.unassignedLead",
                        unassignedFrames,
                        { count: fmt(unassignedFrames) },
                      )}
                    </span>
                  </p>
                  {model.unassigned.map((group) => (
                    <Fold
                      key={group.key}
                      id={`u:${group.key}`}
                      open={open}
                      onToggle={toggle}
                      action={
                        <button
                          type="button"
                          className={buttonClassName("tertiary")}
                          disabled={adding !== null}
                          onClick={() => void add(group)}
                        >
                          <Icon name="plus" className="as-button__icon" />
                          {t("wizard.detect.add", {
                            target: group.designation,
                          })}
                        </button>
                      }
                      head={
                        <>
                          <strong className="as-send-fold__name">
                            {group.designation}
                          </strong>
                          <span className={helpTextClassName}>
                            {groupLine(t, locale, group, seconds)}
                          </span>
                        </>
                      }
                    >
                      <FileList
                        files={group.files}
                        withDetails
                        seconds={seconds}
                      />
                    </Fold>
                  ))}
                  {model.withoutObject && (
                    <Fold
                      id="u:without-object"
                      open={open}
                      onToggle={toggle}
                      head={
                        <>
                          <strong className="as-send-fold__name">
                            {t("wizard.detect.noObject")}
                          </strong>
                          <span className={helpTextClassName}>
                            {groupLine(t, locale, model.withoutObject, seconds)}
                          </span>
                        </>
                      }
                    >
                      <FileList
                        files={model.withoutObject.files}
                        withDetails
                        seconds={seconds}
                      />
                    </Fold>
                  )}
                </section>
              )}
              {model.otherNights > 0 && (
                <p className={helpTextClassName}>
                  {pluralText(
                    t,
                    locale,
                    "wizard.detect.otherNights",
                    model.otherNights,
                    { count: fmt(model.otherNights) },
                  )}
                </p>
              )}
              {/* What of this folder is not read (the
                  videos, the RAW files), an information under the line of the
                  poses of other nights. */}
              {proposal &&
                sheetUnreadText(t, locale, unreadCounts(proposal)) && (
                  <p className={helpTextClassName} data-unread="information">
                    {sheetUnreadText(t, locale, unreadCounts(proposal))}
                  </p>
                )}
            </div>
          </>
        )}
        {error && <Notice tone="error">{error}</Notice>}
        {proposal && model && (
          <div className="as-send-sheet__foot">
            <div className="as-send-sheet__mode">
              <FileModeChoice
                mode={mode}
                onChange={(next) => {
                  setError(null);
                  setMode(next);
                }}
                windowsNoLink={windowsNoLink}
              />
              <div
                className={`as-organise__line as-organise__line--${sentence.tone}`}
              >
                {sentence.tone === "warning" && (
                  <Icon name="warning" className="as-organise__glyph" />
                )}
                <span>
                  <Parts
                    parts={emphasise(sentenceText, [
                      locations.fits
                        ? displayPath(locations.fits, homeOf(locations.fits))
                        : "",
                    ])}
                  />
                </span>
              </div>
              {mode !== "leave-in-place" && filterless && (
                <p className={`${helpTextClassName} as-send-sheet__note`}>
                  {t("wizard.detect.noFilter", {
                    camera,
                    example: filterless,
                  })}
                </p>
              )}
            </div>
            <div className="as-actions as-send-sheet__actions">
              <button
                type="button"
                className={buttonClassName("link")}
                onClick={cancel}
              >
                {t("common.cancel")}
              </button>
              <button
                type="button"
                className={buttonClassName("primary")}
                disabled={!ready}
                onClick={next}
              >
                {t("wizard.detect.next")}
              </button>
            </div>
          </div>
        )}
      </section>
    </div>
  );
}

/** « 18 poses · 120 s · 16:10 → 16:46 UTC ». */
function groupLine(
  t: (key: string, params?: Record<string, string | number>) => string,
  locale: string,
  group: SheetGroup,
  seconds: (values: number[]) => string,
): string {
  const parts = [
    pluralText(t, locale, "wizard.files.frames", group.frames, {
      count: new Intl.NumberFormat(locale).format(group.frames),
    }),
  ];
  if (group.exposures.length > 0) parts.push(seconds(group.exposures));
  if (group.span) {
    parts.push(
      `${utcTime(group.span.start, locale)} → ${utcTime(group.span.end, locale)} UTC`,
    );
  }
  return parts.join(" · ");
}

function Fold({
  id,
  open,
  onToggle,
  head,
  action,
  children,
}: {
  id: string;
  open: ReadonlySet<string>;
  onToggle: (id: string) => void;
  head: React.ReactNode;
  action?: React.ReactNode;
  children: React.ReactNode;
}) {
  const isOpen = open.has(id);
  return (
    <div className="as-send-fold">
      <div className="as-send-fold__row">
        <button
          type="button"
          className="as-send-fold__toggle"
          aria-expanded={isOpen}
          onClick={() => onToggle(id)}
        >
          <Icon
            name="chevron"
            className={isOpen ? "as-how__chevron is-open" : "as-how__chevron"}
          />
          {head}
        </button>
        {action}
      </div>
      {isOpen && <div className="as-send-fold__body">{children}</div>}
    </div>
  );
}

function FileList({
  files,
  withDetails = false,
  seconds,
}: {
  files: SheetFile[];
  withDetails?: boolean;
  seconds: (values: number[]) => string;
}) {
  const { t, locale } = useI18n();
  const more = Math.max(0, files.length - VISIBLE_ROWS);
  return (
    <div className="as-send-files">
      <table className="as-send-files__table">
        {withDetails && (
          <thead>
            <tr>
              <th scope="col">{t("wizard.detect.col.file")}</th>
              <th scope="col">{t("wizard.detect.col.utc")}</th>
              <th scope="col">{t("wizard.detect.col.exposure")}</th>
              <th scope="col">{t("wizard.detect.col.filter")}</th>
            </tr>
          </thead>
        )}
        <tbody>
          {files.map((file) => (
            <tr key={file.path}>
              <td title={file.path}>{file.name}</td>
              {withDetails && (
                <>
                  <td>{file.utc ? utcTime(file.utc, locale) : "—"}</td>
                  <td>
                    {file.exposureSeconds !== null
                      ? seconds([file.exposureSeconds])
                      : "—"}
                  </td>
                  <td>{file.filter ?? "—"}</td>
                </>
              )}
            </tr>
          ))}
        </tbody>
      </table>
      {more > 0 && (
        <p className="as-send-files__more">
          {t("wizard.detect.more", {
            count: new Intl.NumberFormat(locale).format(more),
          })}
        </p>
      )}
    </div>
  );
}

/** The aliases and the coordinates the catalogue knows for the session's
 * targets; a target it does not answer for keeps none (the engine then
 * matches by id and by name). */
async function catalogueExtras(
  scope: SessionScope,
  locale: string,
): Promise<CatalogueExtras> {
  const out: CatalogueExtras = {};
  await Promise.allSettled(
    scope.targets
      .filter((target) => target.raHours == null)
      .map(async (target) => {
        const found = await targetsSearch(target.designation, locale, 0, 6);
        const same = found.targets.find((item) => item.id === target.id);
        if (same) {
          out[target.id] = {
            aliases: same.aliases,
            raHours: same.raHours,
            decDegrees: same.decDegrees,
          };
        }
      }),
  );
  return out;
}

/** The catalogue's id of a target the detection named (« NGC 7000 ») when
 * the engine did not know it. */
async function resolveTarget(
  designation: string,
  locale: string,
): Promise<string | null> {
  const found = await targetsSearch(designation, locale, 0, 6);
  const loose = (text: string) => text.toLowerCase().replace(/[\s_-]+/g, "");
  const same = found.targets.find(
    (item) =>
      loose(item.designation) === loose(designation) ||
      item.aliases.some((alias) => loose(alias) === loose(designation)),
  );
  return same?.id ?? null;
}
