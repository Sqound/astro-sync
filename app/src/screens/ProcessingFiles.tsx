import { useCallback, useEffect, useRef, useState } from "react";

import { Icon } from "../components/Icon";
import { InfoLine } from "../components/InfoLine";
import { Notice } from "../components/Notice";
import { useSnackbar } from "../components/Snackbar";
import { useI18n } from "../i18n";
import {
  processingFilesClean,
  processingFilesScan,
  pythonEnvReinstall,
  type ProcessingScan,
} from "../lib/bridge";
import { formatError } from "../lib/format";
import { displayPath, homeOf } from "../lib/home";
import {
  cleanBoxes,
  defaultTicks,
  filesView,
  kindsOf,
  processingSize,
  tickedBytes,
  type BoxId,
  type FileRowView,
} from "../lib/processingFiles";
import {
  buttonClassName,
  helpTextClassName,
  inactiveButtonClassName,
} from "../theme";

type Sheet = "clean" | "reinstall" | null;

/**
 * « Fichiers de traitement »: what the pipelines produced in the working folder
 * -- masters, results, plan renders, temporaries --, its size, and
 * « Nettoyer… ». The person's own files (copied, moved or linked in) are said
 * apart and never touched; the Python environment is said apart and
 * reinstalled, not cleaned. The sizes are read when the tab opens and at
 * « Recalculer », never in the background.
 */
export function ProcessingFiles() {
  const { t, locale } = useI18n();
  const { notify } = useSnackbar();
  const [scan, setScan] = useState<ProcessingScan | null>(null);
  const [computing, setComputing] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [sheet, setSheet] = useState<Sheet>(null);
  const [ticked, setTicked] = useState<BoxId[]>([]);
  const [working, setWorking] = useState(false);
  const [reinstalling, setReinstalling] = useState(false);
  const alive = useRef(true);

  const compute = useCallback(async () => {
    setComputing(true);
    try {
      const measured = await processingFilesScan();
      if (alive.current) {
        setScan(measured);
        setError(null);
      }
    } catch (caught) {
      if (alive.current) setError(formatError(t, caught));
    } finally {
      if (alive.current) setComputing(false);
    }
    // `t` changes with the language only: the sizes are not read again for it.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    alive.current = true;
    void compute();
    return () => {
      alive.current = false;
    };
  }, [compute]);

  const view = scan ? filesView(t, locale, scan) : null;
  const boxes = scan ? cleanBoxes(t, locale, scan) : [];
  const toDelete = tickedBytes(boxes, ticked);

  function openSheet() {
    setTicked(defaultTicks(boxes));
    setSheet("clean");
  }

  function toggle(id: BoxId) {
    setTicked((current) =>
      current.includes(id)
        ? current.filter((item) => item !== id)
        : [...current, id],
    );
  }

  async function clean() {
    setWorking(true);
    setError(null);
    try {
      const outcome = await processingFilesClean(kindsOf(boxes, ticked));
      setSheet(null);
      notify({
        tone: "success",
        text: t("settings.processing.freed", {
          size: processingSize(t, outcome.freedBytes, locale),
        }),
      });
      if (outcome.failed.length > 0) {
        setError(
          t(
            outcome.failed.length === 1
              ? "settings.processing.failed.one"
              : "settings.processing.failed.other",
            { count: outcome.failed.length },
          ),
        );
      }
      await compute();
    } catch (caught) {
      setSheet(null);
      setError(formatError(t, caught));
    } finally {
      setWorking(false);
    }
  }

  async function reinstall() {
    setSheet(null);
    setReinstalling(true);
    setError(null);
    try {
      const answer = await pythonEnvReinstall();
      if (answer.result === "done") {
        notify({ tone: "success", text: t("settings.processing.python.done") });
      } else {
        setError(t("settings.processing.python.failed"));
      }
    } catch (caught) {
      setError(formatError(t, caught));
    } finally {
      setReinstalling(false);
      await compute();
    }
  }

  const renderRow = (row: FileRowView) => (
    <div className="as-files__row" key={row.kind}>
      <div className="as-files__name">
        <span>{row.name}</span>
        {row.note && <span className="as-files__note">{row.note}</span>}
      </div>
      <span className="as-files__size">
        {processingSize(t, row.bytes, locale)}
      </span>
    </div>
  );

  return (
    <section className="as-zone as-files-zone">
      <h2 className="as-zone__title">{t("settings.processing.title")}</h2>
      <InfoLine
        line={t("settings.processing.line")}
        details={<p>{t("settings.processing.details")}</p>}
      />
      <Notice tone="error">{error}</Notice>
      {scan && !scan.reachable && (
        <Notice tone="warning">
          {t("settings.processing.unreachable", {
            path: displayPath(scan.root, homeOf(scan.root)),
          })}
        </Notice>
      )}
      {view && (
        <>
          <div className="as-files">
            {view.rows.map(renderRow)}
            <div className="as-files__row as-files__row--total">
              <div className="as-files__name">
                <strong>{t("settings.processing.total.name")}</strong>
                {view.total.note && (
                  <span className="as-files__note">{view.total.note}</span>
                )}
              </div>
              <strong className="as-files__size">
                {processingSize(t, view.total.bytes, locale)}
              </strong>
            </div>
            <div className="as-files__row">
              <div className="as-files__name">
                <span>{t("settings.processing.user.name")}</span>
                {view.reachable && (
                  <span className="as-files__note">
                    {t("settings.processing.user.note")}
                  </span>
                )}
              </div>
              <span className="as-files__size">
                {processingSize(t, view.user.bytes, locale)}
              </span>
            </div>
          </div>
          <div className="as-files as-files--python">
            <div className="as-files__row">
              <div className="as-files__name">
                <span>{t("settings.processing.python.name")}</span>
                <span className="as-files__note">
                  {t("settings.processing.python.note")}
                </span>
              </div>
              <span className="as-files__size">
                {reinstalling
                  ? t("settings.processing.python.running")
                  : processingSize(t, view.python.bytes, locale)}
              </span>
              <button
                type="button"
                className={buttonClassName("link")}
                disabled={reinstalling}
                onClick={() => setSheet("reinstall")}
              >
                {t("settings.processing.python.reinstall")}
              </button>
            </div>
          </div>
        </>
      )}
      <div className="as-files__foot">
        <span className={`${helpTextClassName} as-files__stamp`}>
          {computing ? t("settings.processing.computing") : (view?.stamp ?? "")}
        </span>
        <button
          type="button"
          className={buttonClassName("tertiary")}
          disabled={computing || working}
          onClick={() => void compute()}
        >
          <Icon name="refresh" className="as-button__icon" />
          {t("settings.processing.recompute")}
        </button>
        {view?.cleanable && !computing && !reinstalling ? (
          <button
            type="button"
            className={buttonClassName("secondary")}
            onClick={openSheet}
          >
            {t("settings.processing.clean")}
          </button>
        ) : (
          <button type="button" className={inactiveButtonClassName} disabled>
            {t("settings.processing.clean")}
          </button>
        )}
      </div>

      {sheet === "clean" && (
        <div className="as-dialog__scrim" role="presentation">
          <div
            className="as-dialog"
            role="dialog"
            aria-modal="true"
            aria-labelledby="processing-clean-title"
          >
            <h2 id="processing-clean-title" className="as-dialog__title">
              {t("settings.processing.sheet.title")}
            </h2>
            <p className={helpTextClassName}>
              {t("settings.processing.sheet.body")}
            </p>
            <div className="as-options">
              {boxes.map((box) => (
                <section className="as-option" key={box.id}>
                  <label className="as-option__head">
                    <input
                      type="checkbox"
                      checked={ticked.includes(box.id)}
                      onChange={() => toggle(box.id)}
                    />
                    <strong className="as-option__title">{box.title}</strong>
                    <span className="as-option__size">
                      {processingSize(t, box.bytes, locale)}
                    </span>
                  </label>
                  <p className="as-option__line">{box.note}</p>
                  {box.warning && (
                    <p className="as-option__line as-files__warn">
                      <Icon name="warning" className="as-files__warn-icon" />
                      {box.warning}
                    </p>
                  )}
                </section>
              ))}
            </div>
            <p className="as-files__kept">
              {t("settings.processing.sheet.kept")}
            </p>
            <div className="as-actions">
              <button
                type="button"
                className={buttonClassName("link")}
                onClick={() => setSheet(null)}
              >
                {t("common.cancel")}
              </button>
              {toDelete > 0 && !working ? (
                <button
                  type="button"
                  className={buttonClassName("primary", { destructive: true })}
                  onClick={() => void clean()}
                >
                  {t("settings.processing.sheet.delete", {
                    size: processingSize(t, toDelete, locale),
                  })}
                </button>
              ) : (
                <button
                  type="button"
                  className={inactiveButtonClassName}
                  disabled
                >
                  {t("settings.processing.sheet.delete", {
                    size: processingSize(t, toDelete, locale),
                  })}
                </button>
              )}
            </div>
          </div>
        </div>
      )}

      {sheet === "reinstall" && (
        <div className="as-dialog__scrim" role="presentation">
          <div
            className="as-dialog"
            role="dialog"
            aria-modal="true"
            aria-labelledby="processing-reinstall-title"
          >
            <h2 id="processing-reinstall-title" className="as-dialog__title">
              {t("settings.processing.python.confirmTitle")}
            </h2>
            <p className={helpTextClassName}>
              {t("settings.processing.python.confirmBody", {
                size: processingSize(t, scan?.pythonEnvBytes ?? 0, locale),
              })}
            </p>
            <div className="as-actions">
              <button
                type="button"
                className={buttonClassName("link")}
                onClick={() => setSheet(null)}
              >
                {t("common.cancel")}
              </button>
              <button
                type="button"
                className={buttonClassName("primary", { destructive: true })}
                onClick={() => void reinstall()}
              >
                {t("settings.processing.python.confirm")}
              </button>
            </div>
          </div>
        </div>
      )}
    </section>
  );
}
