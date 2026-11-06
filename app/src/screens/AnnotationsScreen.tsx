import { useEffect, useRef, useState } from "react";

import { Icon } from "../components/Icon";
import { Waiting } from "../components/Waiting";
import { InfoLine } from "../components/InfoLine";
import { Notice } from "../components/Notice";
import { PageTop } from "../components/PageTop";
import { useI18n } from "../i18n";
import {
  isCommandError,
  localWorkCancel,
  nightFoldersList,
  nightOnline,
  nightPrecheck,
  skyMissing,
  skySolveCapture,
  toolsDetect,
  type MissingCapture,
  type ToolRow,
} from "../lib/bridge";
import {
  annotationsState,
  captureLine,
  captureTitle,
  countText,
  failureText,
  rowsToSolve,
  runningText,
  solverPresent,
  sourceText,
  stateText,
  type RowProgress,
} from "../lib/annotationsTab";
import { formatError } from "../lib/format";
import type { ScreenId } from "../lib/menu";
import { detectionInput } from "../lib/nights";
import {
  badgeClassName,
  buttonClassName,
  helpTextClassName,
  inactiveButtonClassName,
} from "../theme";

/** The night images on this computer, by SHA-256: the final images of the
 * nights whose folder is here (a capture solved from its own file is never
 * downloaded again). */
async function imagesHere(
  captures: readonly MissingCapture[],
): Promise<Record<string, string>> {
  const sessions = new Set(
    captures.map((capture) => capture.sessionId).filter(Boolean) as string[],
  );
  if (sessions.size === 0) return {};
  const folders = await nightFoldersList().catch(() => []);
  const here: Record<string, string> = {};
  for (const folder of folders) {
    if (!sessions.has(folder.nightId)) continue;
    try {
      const online = await nightOnline(folder.nightId);
      const date = /^(\d{4}-\d{2}-\d{2})/.exec(folder.name)?.[1];
      const input = detectionInput(
        { startedAt: `${date ?? "1970-01-01"}T18:00:00Z` },
        online,
      );
      const precheck = await nightPrecheck(
        folder.nightId,
        { ...input, eveningDate: date ?? null },
        [],
      );
      for (const plan of precheck.plans) {
        here[plan.image.sha256] = plan.image.path;
      }
    } catch {
      // That night's images are fetched from the space instead.
    }
  }
  return here;
}

/**
 * « Annotations »: the captures already sent without a sky solution, from the
 * phone, the web or a night; each row says where its image is. « Tout
 * résoudre » the one primary, « Résoudre et envoyer » tertiary per row,
 * « Arrêter la résolution » a link.
 */
export function AnnotationsScreen({
  onOpen,
}: {
  onOpen: (screen: ScreenId) => void;
}) {
  const { t, locale } = useI18n();
  const [captures, setCaptures] = useState<MissingCapture[] | null>(null);
  const [tools, setTools] = useState<ToolRow[] | null>(null);
  const [here, setHere] = useState<Record<string, string>>({});
  const [progress, setProgress] = useState<Record<string, RowProgress>>({});
  const [running, setRunning] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notPaired, setNotPaired] = useState(false);
  const stopped = useRef(false);

  useEffect(() => {
    let cancelled = false;
    toolsDetect()
      .then((rows) => !cancelled && setTools(rows))
      .catch(() => undefined);
    skyMissing()
      .then(async (list) => {
        if (cancelled) return;
        setCaptures(list);
        const found = await imagesHere(list);
        if (!cancelled) setHere(found);
      })
      .catch((caught: unknown) => {
        if (cancelled) return;
        setCaptures([]);
        if (
          isCommandError(caught) &&
          (caught.code === "not_paired" || caught.code === "portal_url_missing")
        ) {
          setNotPaired(true);
        } else {
          setError(formatError(t, caught));
        }
      });
    return () => {
      cancelled = true;
    };
  }, [t]);

  async function solve(list: readonly MissingCapture[]) {
    setRunning(true);
    setError(null);
    stopped.current = false;
    for (const capture of list) {
      if (stopped.current) break;
      setProgress((current) => ({
        ...current,
        [capture.id]: { state: "running" },
      }));
      try {
        const local = here[capture.sha256];
        const outcome = await skySolveCapture(
          capture,
          local ? { [capture.sha256]: local } : null,
        );
        setProgress((current) => ({
          ...current,
          [capture.id]:
            outcome.outcome === "sent"
              ? { state: "done" }
              : { state: "failed", failure: outcome.failure },
        }));
      } catch (caught) {
        setProgress((current) => ({
          ...current,
          [capture.id]: { state: "todo" },
        }));
        setError(formatError(t, caught));
        break;
      }
    }
    setRunning(false);
  }

  function stop() {
    stopped.current = true;
    void localWorkCancel();
  }

  const list = captures ?? [];
  const solver = solverPresent(tools);
  const state = annotationsState({
    captures: list,
    solverPresent: solver,
    running,
  });
  const toSolve = rowsToSolve(list, progress);

  return (
    <div className="as-page">
      <PageTop>
        <h1 className="as-page__title">{t("nav.annotations")}</h1>
      </PageTop>
      <InfoLine
        line={t("annotations.help")}
        details={<p>{t("annotations.helpDetails")}</p>}
      />
      <Notice tone="error">{error}</Notice>
      {notPaired && (
        <Notice tone="warning">{t("sessions.notPairedNotice")}</Notice>
      )}
      {captures === null && <Waiting label={t("common.wait.annotations")} />}

      {state === "noSolver" && (
        <div className="as-notice as-notice--warning as-notice--inline">
          <span>{t("annotations.noSolver")}</span>
          <button
            type="button"
            className={buttonClassName("link")}
            onClick={() => onOpen("dependencies")}
          >
            {t("deps.open")}
          </button>
        </div>
      )}

      {state === "empty" && captures !== null && !notPaired && !error && (
        <p className="as-tool-line">
          <Icon name="check" />
          <span>{t("annotations.empty")}</span>
        </p>
      )}

      {list.length > 0 && (
        <>
          <div className="as-list-head">
            <p className={helpTextClassName}>
              {countText(t, locale, list.length)}
            </p>
            <div className="as-actions">
              {running && (
                <button
                  type="button"
                  className={buttonClassName("link")}
                  onClick={stop}
                >
                  {t("annotations.stop")}
                </button>
              )}
              {state === "list" && toSolve.length > 0 ? (
                <button
                  type="button"
                  className={buttonClassName("primary")}
                  onClick={() => void solve(toSolve)}
                >
                  {t("annotations.solveAll")}
                </button>
              ) : (
                <button
                  type="button"
                  className={inactiveButtonClassName}
                  disabled
                >
                  {running
                    ? t("annotations.running")
                    : t("annotations.solveAll")}
                </button>
              )}
            </div>
          </div>
          {list.map((capture) => {
            const row = progress[capture.id] ?? { state: "todo" as const };
            const isHere = Boolean(here[capture.sha256]);
            return (
              <div key={capture.id} className="as-capture">
                <span className="as-capture__thumb" aria-hidden="true">
                  <Icon name="sparkle" />
                </span>
                <div className="as-capture__body">
                  <h2 className="as-capture__title">{captureTitle(capture)}</h2>
                  <span className="as-capture__line">
                    {captureLine(t, locale, capture)}
                  </span>
                  <span className="as-capture__line">
                    <Icon
                      name={
                        isHere
                          ? "folder"
                          : capture.mainImageSource === "phone"
                            ? "phone"
                            : "external"
                      }
                    />{" "}
                    {sourceText(t, capture, isHere)}
                  </span>
                  {row.state === "running" && (
                    <span className="as-capture__line">
                      {runningText(t, locale, capture, isHere)}
                    </span>
                  )}
                  {row.state === "failed" && row.failure && (
                    <span className="as-capture__failure">
                      {failureText(t, row.failure)}
                    </span>
                  )}
                </div>
                <div className="as-capture__aside">
                  <span
                    className={badgeClassName(
                      row.state === "done"
                        ? "success"
                        : row.state === "failed"
                          ? "error"
                          : row.state === "running"
                            ? "neutral"
                            : "warning",
                    )}
                  >
                    {row.state === "failed" && <Icon name="warning" />}
                    {stateText(t, row.state)}
                  </span>
                </div>
                {row.state !== "done" && (
                  <button
                    type="button"
                    className={buttonClassName("tertiary")}
                    disabled={running || state === "noSolver"}
                    onClick={() => void solve([capture])}
                  >
                    {t("annotations.solve")}
                  </button>
                )}
              </div>
            );
          })}
        </>
      )}
    </div>
  );
}
