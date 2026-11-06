import { useCallback, useEffect, useState } from "react";

import { Icon, type IconName } from "../components/Icon";
import { Waiting } from "../components/Waiting";
import { InfoLine } from "../components/InfoLine";
import { PipelineDetails } from "../components/PipelineDetails";
import { Notice } from "../components/Notice";
import { PipelinesNote } from "../components/PipelinesNote";
import { TrustDialog } from "../components/TrustDialog";
import { useI18n } from "../i18n";
import {
  downloadReferenceStars,
  localWorkCancel,
  onPythonEnv,
  onToolDownload,
  openInBrowser,
  pipelineSections,
  pipelineTrustSet,
  pythonEnvInstall,
  toolsDetect,
  toolsDownload,
  toolsInstallAll,
  toolsVerify,
  type DownloadAnswer,
  type PipelineSections,
  type ToolRow,
  type TrustState,
} from "../lib/bridge";
import { formatError } from "../lib/format";
import {
  dependenciesHead,
  dependencyRows,
  ENVIRONMENT_ROWS,
  pipelineSectionViews,
  sectionsToInstall,
  type DependencyRow,
  type DownloadState,
  type PipelineSectionView,
  type SectionLineView,
} from "../lib/settingsTab";
import {
  badgeClassName,
  buttonClassName,
  helpTextClassName,
  inactiveButtonClassName,
} from "../theme";

const ICON: Record<string, IconName> = {
  siril: "list",
  astap: "sparkle",
  "astrometry-net": "sparkle",
  "tycho-2": "chart",
  "stacking-pipeline": "sliders",
};

/**
 * Réglages › « Dépendances »: one row per tool as the core reads it from the
 * registry, its role in one line, its state as a badge, « Installer »
 * (tertiary) and « Page de téléchargement » (link); « Tout
 * installer » the page's primary only when something required is missing.
 *
 * The row « Pipeline d'empilement » says its Python environment
 * (made here: « Installer », its progress, « Annuler l'installation »,
 * « Reprendre l'installation » after a failure), and the section
 * « Pipelines de traitement » lists each pipeline found, shipped or added,
 * with one line per dependency it declares (resolved by the registry) and,
 * for a script the person added, their trust in it (« Lui faites-vous
 * confiance ? »). « Vérifier à nouveau » also checks the environment.
 */
export function DependenciesTab() {
  const { t, locale } = useI18n();
  const [tools, setTools] = useState<ToolRow[] | null>(null);
  const [sections, setSections] = useState<PipelineSections | null>(null);
  const [downloads, setDownloads] = useState<Record<string, DownloadState>>({});
  const [trust, setTrust] = useState<Record<string, TrustState>>({});
  const [asking, setAsking] = useState<PipelineSectionView | null>(null);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(
    async (verify = false) => {
      try {
        setTools(await (verify ? toolsVerify() : toolsDetect()));
      } catch (caught) {
        setTools([]);
        setError(formatError(t, caught));
      }
      try {
        setSections(await pipelineSections(locale));
        setTrust({});
      } catch {
        setSections({ sections: [], ignored: [] });
      }
    },
    [t, locale],
  );

  useEffect(() => {
    void refresh();
    const stops: (() => void)[] = [];
    let stopped = false;
    const keep = (unlisten: () => void) => {
      if (stopped) unlisten();
      else stops.push(unlisten);
    };
    onToolDownload((id, progress) =>
      setDownloads((current) => ({
        ...current,
        [id]: { state: "downloading", ...progress },
      })),
    )
      .then(keep)
      .catch(() => undefined);
    onPythonEnv((progress) =>
      setDownloads((current) => ({
        ...current,
        "stacking-pipeline": { state: "installing", progress },
      })),
    )
      .then(keep)
      .catch(() => undefined);
    return () => {
      stopped = true;
      for (const stop of stops) stop();
    };
  }, [refresh]);

  function settle(answers: readonly DownloadAnswer[]) {
    setDownloads((current) => {
      const next = { ...current };
      for (const answer of answers) {
        const id = ENVIRONMENT_ROWS.includes(answer.id)
          ? "stacking-pipeline"
          : answer.id;
        // No file for this system is not a failure: the row keeps its page.
        if (answer.result === "done") delete next[id];
        else if (
          answer.result === "failed" &&
          answer.failure.code === "no_automatic_download"
        )
          delete next[id];
        else if (answer.result === "env_failed")
          next[id] = { state: "envFailed", failure: answer.failure };
        else next[id] = { state: "failed", failure: answer.failure };
      }
      return next;
    });
  }

  async function installEnvironment() {
    setError(null);
    setDownloads((current) => ({
      ...current,
      "stacking-pipeline": { state: "installing", progress: null },
    }));
    try {
      const answer = await pythonEnvInstall();
      settle([
        answer.result === "done"
          ? { result: "done", id: "stacking-pipeline" }
          : {
              result: "env_failed",
              id: "stacking-pipeline",
              failure: answer.failure,
            },
      ]);
    } catch (caught) {
      settle([{ result: "done", id: "stacking-pipeline" }]);
      setError(formatError(t, caught));
    }
    await refresh();
  }

  async function install(id: string) {
    if (ENVIRONMENT_ROWS.includes(id)) {
      await installEnvironment();
      return;
    }
    setError(null);
    setDownloads((current) => ({
      ...current,
      [id]: { state: "downloading", bytes: 0, total: null },
    }));
    try {
      if (id === "tycho-2") {
        await downloadReferenceStars();
        settle([{ result: "done", id }]);
      } else {
        settle([await toolsDownload(id)]);
      }
    } catch (caught) {
      setDownloads((current) => {
        const next = { ...current };
        delete next[id];
        return next;
      });
      setError(formatError(t, caught));
    }
    await refresh();
  }

  async function installAll() {
    setError(null);
    if (sectionsToInstall(sections).includes("python-env")) {
      setDownloads((current) => ({
        ...current,
        "stacking-pipeline": { state: "installing", progress: null },
      }));
    }
    try {
      const answers = await toolsInstallAll();
      settle(answers);
      // A required tool the registry has no file for: its page only.
      const answered = new Set(answers.map((answer) => answer.id));
      setDownloads((current) => {
        const next = { ...current };
        if (!answered.has("python-env")) {
          if (next["stacking-pipeline"]?.state === "installing") {
            delete next["stacking-pipeline"];
          }
        }
        return next;
      });
    } catch (caught) {
      setError(formatError(t, caught));
    }
    await refresh();
  }

  async function answerTrust(section: PipelineSectionView, trusted: boolean) {
    const sha256 = sections?.sections.find(
      (s) => s.id === section.id,
    )?.scriptSha256;
    setAsking(null);
    if (!sha256) return;
    try {
      await pipelineTrustSet(section.id, sha256, trusted);
      setTrust((current) => ({
        ...current,
        [section.id]: trusted ? "trusted" : "refused",
      }));
    } catch (caught) {
      setError(formatError(t, caught));
    }
  }

  if (tools === null) {
    return <Waiting label={t("common.wait.tools")} />;
  }
  const rows = dependencyRows(t, locale, tools, downloads);
  const toInstall = sectionsToInstall(sections);
  const head = dependenciesHead(t, locale, rows, tools, toInstall);
  const views = sections
    ? pipelineSectionViews(t, locale, sections, tools, downloads, trust)
    : [];

  return (
    <div className="as-form-card">
      <InfoLine
        line={t("deps.lead")}
        details={<p>{t("deps.leadDetails")}</p>}
      />
      <Notice tone="error">{error}</Notice>
      <div className="as-list-head">
        <p>{head.line}</p>
        <div className="as-actions">
          <button
            type="button"
            className={buttonClassName("link")}
            onClick={() => void refresh(true)}
          >
            {t("deps.check")}
          </button>
          {head.installAll &&
            (head.downloading ? (
              <button
                type="button"
                className={inactiveButtonClassName}
                disabled
              >
                {t("deps.installAll")}
              </button>
            ) : (
              <button
                type="button"
                className={buttonClassName("primary")}
                onClick={() => void installAll()}
              >
                {t("deps.installAll")}
              </button>
            ))}
        </div>
      </div>
      {rows.map((row) => (
        <ToolLine
          key={row.id}
          row={row}
          busy={head.downloading}
          onInstall={() => void install(row.id)}
        />
      ))}

      {sections === null && (
        // The pipelines are read after the tools: their place and their title
        // are there at once, so the page does not jump when they arrive and
        // nobody takes the list above for the whole.
        <section
          className="as-pipelines"
          aria-labelledby="deps-pipelines"
          aria-busy="true"
        >
          <h2 id="deps-pipelines" className="as-pipelines__title">
            {t("deps.pipelines.title")}
          </h2>
          <Waiting label={t("common.wait.pipelines")} size="inline" />
        </section>
      )}
      {views.length > 0 && (
        <section className="as-pipelines" aria-labelledby="deps-pipelines">
          <h2 id="deps-pipelines" className="as-pipelines__title">
            {t("deps.pipelines.title")}
          </h2>
          <PipelinesNote />
          <InfoLine
            line={t("deps.pipelines.lead")}
            details={<p>{t("deps.pipelines.leadDetails")}</p>}
          />
          {views.map((view) => (
            <div key={view.id} className="as-pipeline">
              <h3 className="as-pipeline__name">{view.name}</h3>
              <p className={helpTextClassName}>
                {view.origin} · {view.description}
              </p>
              <PipelineDetails
                label={t("deps.pipelines.details")}
                steps={view.details}
              />
              {view.dependencies.map((line) => (
                <SectionLine
                  key={line.id}
                  line={line}
                  busy={head.downloading}
                  onInstall={() => void install(line.id)}
                />
              ))}
              {view.trust && (
                <div className="as-tool as-tool--sub">
                  <span className="as-tool__icon" aria-hidden="true">
                    <Icon name="lock" />
                  </span>
                  <div className="as-tool__body">
                    <h4 className="as-tool__name">{t("deps.trust.name")}</h4>
                    <span>{t("deps.trust.role")}</span>
                    <span className="as-tool__line">{view.trust.line}</span>
                  </div>
                  <div className="as-tool__aside">
                    <span className={badgeClassName(view.trust.badge.tone)}>
                      {view.trust.badge.text}
                    </span>
                    <div className="as-tool__actions">
                      {view.trust.action === "decide" ? (
                        <button
                          type="button"
                          className={buttonClassName("tertiary")}
                          onClick={() => setAsking(view)}
                        >
                          {t("deps.trust.decide")}
                        </button>
                      ) : (
                        <button
                          type="button"
                          className={buttonClassName("link")}
                          onClick={() => void answerTrust(view, false)}
                        >
                          {t("deps.trust.revoke")}
                        </button>
                      )}
                    </div>
                  </div>
                </div>
              )}
            </div>
          ))}
          {(sections?.ignored ?? []).map((ignored) => (
            <p key={ignored.folder} className={helpTextClassName}>
              {t("deps.pipelines.ignored", {
                folder: ignored.folder,
                reason: ignored.reason,
              })}
            </p>
          ))}
        </section>
      )}

      {asking && (
        <TrustDialog
          name={asking.name}
          sha256={
            sections?.sections.find((s) => s.id === asking.id)?.scriptSha256 ??
            null
          }
          changed={asking.trust?.state === "changed"}
          onClose={() => setAsking(null)}
          onAnswer={(trusted) => void answerTrust(asking, trusted)}
        />
      )}
    </div>
  );
}

/** One row of the tools. */
function ToolLine({
  row,
  busy,
  onInstall,
}: {
  row: DependencyRow;
  busy: boolean;
  onInstall: () => void;
}) {
  const { t } = useI18n();
  return (
    <div className="as-tool">
      <span className="as-tool__icon" aria-hidden="true">
        <Icon name={ICON[row.id] ?? "gear"} />
      </span>
      <div className="as-tool__body">
        <h2 className="as-tool__name">
          {row.name}{" "}
          {row.optional && (
            <span className={badgeClassName("neutral")}>
              {t("deps.optional")}
            </span>
          )}
        </h2>
        <span>{row.role}</span>
        {row.lines.map((line) => (
          <span key={line} className="as-tool__line">
            {line}
          </span>
        ))}
        {row.progress && row.progress.total ? (
          <div className="as-progress">
            <div
              className="as-progress__fill"
              style={{
                width: `${Math.min(100, (100 * row.progress.bytes) / row.progress.total)}%`,
              }}
            />
          </div>
        ) : null}
      </div>
      <div className="as-tool__aside">
        <span className={badgeClassName(row.badge.tone)}>
          {row.badge.tone === "success" && (
            <Icon name="check" strokeWidth={2.4} />
          )}
          {row.badge.text}
        </span>
        <div className="as-tool__actions">
          {(row.action === "install" ||
            row.action === "resume" ||
            row.action === "resumeInstall") && (
            <button
              type="button"
              className={buttonClassName("tertiary")}
              disabled={busy}
              onClick={onInstall}
            >
              {t(
                row.action === "install"
                  ? "deps.install"
                  : row.action === "resume"
                    ? "deps.resume"
                    : "deps.resumeInstall",
              )}
            </button>
          )}
          {(row.action === "cancel" || row.action === "cancelInstall") && (
            <button
              type="button"
              className={buttonClassName("link")}
              onClick={() => void localWorkCancel()}
            >
              {t(
                row.action === "cancel" ? "deps.cancel" : "deps.cancelInstall",
              )}
            </button>
          )}
          {row.page &&
            row.action !== "cancel" &&
            row.action !== "cancelInstall" && (
              <button
                type="button"
                className={buttonClassName("link")}
                onClick={() => row.page && void openInBrowser(row.page)}
              >
                <Icon name="external" className="as-button__icon" />
                {t("deps.page")}
              </button>
            )}
        </div>
      </div>
    </div>
  );
}

/** One dependency of a pipeline, under its block. */
function SectionLine({
  line,
  busy,
  onInstall,
}: {
  line: SectionLineView;
  busy: boolean;
  onInstall: () => void;
}) {
  const { t } = useI18n();
  return (
    <div className="as-tool as-tool--sub">
      <span className="as-tool__icon" aria-hidden="true">
        <Icon name="gear" />
      </span>
      <div className="as-tool__body">
        <h4 className="as-tool__name">
          {line.name}{" "}
          {line.optional && (
            <span className={badgeClassName("neutral")}>
              {t("deps.optional")}
            </span>
          )}
        </h4>
        <span>{line.reason}</span>
        {line.lines.map((text) => (
          <span key={text} className="as-tool__line">
            {text}
          </span>
        ))}
      </div>
      <div className="as-tool__aside">
        <span className={badgeClassName(line.badge.tone)}>
          {line.badge.tone === "success" && (
            <Icon name="check" strokeWidth={2.4} />
          )}
          {line.badge.text}
        </span>
        <div className="as-tool__actions">
          {line.action && (
            <button
              type="button"
              className={buttonClassName("tertiary")}
              disabled={busy}
              onClick={onInstall}
            >
              {t(
                line.action === "install"
                  ? "deps.install"
                  : "deps.resumeInstall",
              )}
            </button>
          )}
          {line.page && (
            <button
              type="button"
              className={buttonClassName("link")}
              onClick={() => void openInBrowser(line.page!)}
            >
              <Icon name="external" className="as-button__icon" />
              {t("deps.page")}
            </button>
          )}
        </div>
      </div>
    </div>
  );
}
