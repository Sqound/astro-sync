import { useState } from "react";

import { Notice } from "../components/Notice";
import { useSnackbar } from "../components/Snackbar";
import { Waiting } from "../components/Waiting";
import { useI18n } from "../i18n";
import {
  chooseFolder,
  dryRunFolder,
  enqueueFolder,
  producePackage,
  type DryRunReport,
} from "../lib/bridge";
import { formatBytes, formatError } from "../lib/format";
import {
  buttonClassName,
  confirmClassName,
  fieldClassName,
  fieldGroupClassName,
  fieldLabelClassName,
  helpTextClassName,
  panelClassName,
  tableClassName,
  tableWrapperClassName,
} from "../theme";

export function DryRunScreen() {
  const { t, locale } = useI18n();

  const [folder, setFolder] = useState<string | null>(null);
  const [report, setReport] = useState<DryRunReport | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // What passes is said by a snackbar.
  const { notify } = useSnackbar();
  const saySuccess = (text: string) => notify({ tone: "success", text });

  const [outputParent, setOutputParent] = useState<string | null>(null);
  const [sessionLabel, setSessionLabel] = useState("");
  const [confirming, setConfirming] = useState(false);
  const [packaging, setPackaging] = useState(false);

  async function handleChooseFolder() {
    const chosen = await chooseFolder();
    if (chosen) {
      setFolder(chosen);
      setReport(null);
    }
  }

  async function handleChooseOutput() {
    const chosen = await chooseFolder();
    if (chosen) setOutputParent(chosen);
  }

  async function handleRunDryRun() {
    if (!folder) return;
    setError(null);
    setLoading(true);
    try {
      setReport(await dryRunFolder(folder));
    } catch (caught) {
      setError(formatError(t, caught));
    } finally {
      setLoading(false);
    }
  }

  async function handleEnqueue() {
    if (!folder) return;
    setError(null);
    try {
      const outcome = await enqueueFolder(folder);
      saySuccess(
        t("dryRun.enqueueSuccessNotice", {
          inserted: outcome.inserted,
          alreadyKnown: outcome.alreadyKnown,
        }),
      );
    } catch (caught) {
      setError(formatError(t, caught));
    }
  }

  async function handleConfirmPackage() {
    if (!folder || !outputParent) return;
    setConfirming(false);
    setError(null);
    setPackaging(true);
    try {
      const outcome = await producePackage(folder, outputParent, {
        label: sessionLabel.trim() || null,
      });
      saySuccess(
        t("dryRun.packageSuccessBody", {
          count: outcome.assetCount,
          dir: outcome.outputDir,
        }),
      );
    } catch (caught) {
      setError(formatError(t, caught));
    } finally {
      setPackaging(false);
    }
  }

  return (
    <div className={panelClassName}>
      <h2>{t("dryRun.title")}</h2>
      <p>{t("dryRun.description")}</p>

      <Notice tone="error">{error}</Notice>

      <div className={fieldGroupClassName}>
        <span className={fieldLabelClassName}>{t("dryRun.folderLabel")}</span>
        <p>
          <button
            type="button"
            className={buttonClassName("tertiary")}
            onClick={() => void handleChooseFolder()}
          >
            {t("dryRun.chooseFolderButton")}
          </button>
        </p>
        <p className={helpTextClassName}>
          {folder ?? t("associate.noFolderSelected")}
        </p>
        <p>
          <button
            type="button"
            className={buttonClassName("primary")}
            disabled={!folder || loading}
            onClick={() => void handleRunDryRun()}
          >
            {t("dryRun.runButton")}
          </button>
        </p>
      </div>

      {loading && <Waiting label={t("common.wait.dryRun")} />}

      {!loading && report && report.entries.length === 0 && (
        <p>{t("dryRun.emptyState")}</p>
      )}

      {!loading && report && report.entries.length > 0 && (
        <>
          <p>
            {t("dryRun.totalFiles", { count: report.entries.length })} —{" "}
            {t("dryRun.totalBytes", {
              value: formatBytes(t, report.totalBytes, locale),
            })}
          </p>
          <div className={tableWrapperClassName}>
            <table className={tableClassName}>
              <thead>
                <tr>
                  <th scope="col">{t("dryRun.columnPath")}</th>
                  <th scope="col">{t("dryRun.columnClass")}</th>
                  <th scope="col">{t("dryRun.columnDestination")}</th>
                  <th scope="col">{t("dryRun.columnBytes")}</th>
                  <th scope="col">{t("dryRun.columnSha256")}</th>
                </tr>
              </thead>
              <tbody>
                {report.entries.map((entry) => (
                  <tr key={entry.sourceRelativePath}>
                    <td>{entry.sourceRelativePath}</td>
                    <td>{entry.class}</td>
                    <td>{entry.packageRelativePath}</td>
                    <td>{formatBytes(t, entry.bytes, locale)}</td>
                    <td>
                      <code>{entry.sha256.slice(0, 12)}…</code>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>

          <p>
            <button
              type="button"
              className={buttonClassName("tertiary")}
              onClick={() => void handleEnqueue()}
            >
              {t("dryRun.enqueueButton")}
            </button>
          </p>

          <section>
            <h3>{t("dryRun.packageSectionTitle")}</h3>
            <div className={fieldGroupClassName}>
              <span className={fieldLabelClassName}>
                {t("dryRun.outputParentLabel")}
              </span>
              <p>
                <button
                  type="button"
                  className={buttonClassName("tertiary")}
                  onClick={() => void handleChooseOutput()}
                >
                  {t("dryRun.chooseOutputButton")}
                </button>
              </p>
              <p className={helpTextClassName}>
                {outputParent ?? t("associate.noFolderSelected")}
              </p>
            </div>
            <div className={fieldGroupClassName}>
              <label
                className={fieldLabelClassName}
                htmlFor="dry-run-session-label"
              >
                {t("dryRun.sessionLabelLabel")}
              </label>
              <input
                id="dry-run-session-label"
                className={fieldClassName}
                type="text"
                value={sessionLabel}
                placeholder={t("dryRun.sessionLabelPlaceholder")}
                onChange={(event) => setSessionLabel(event.target.value)}
              />
            </div>
            <p>
              <button
                type="button"
                className={buttonClassName("primary")}
                disabled={!outputParent || packaging}
                onClick={() => setConfirming(true)}
              >
                {t("dryRun.confirmButton")}
              </button>
            </p>

            {confirming && (
              <div
                className={confirmClassName(false)}
                role="alertdialog"
                aria-modal="true"
              >
                <h4>{t("dryRun.confirmDialogTitle")}</h4>
                <p>
                  {t("dryRun.confirmDialogBody", {
                    count: report.entries.length,
                    bytes: formatBytes(t, report.totalBytes, locale),
                  })}
                </p>
                <p>
                  <button
                    type="button"
                    className={buttonClassName("primary")}
                    onClick={() => void handleConfirmPackage()}
                  >
                    {t("dryRun.confirmDialogConfirm")}
                  </button>{" "}
                  <button
                    type="button"
                    className={buttonClassName("link")}
                    onClick={() => setConfirming(false)}
                  >
                    {t("common.cancel")}
                  </button>
                </p>
              </div>
            )}
          </section>
        </>
      )}
    </div>
  );
}
