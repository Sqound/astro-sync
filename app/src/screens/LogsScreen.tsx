import { useEffect, useState } from "react";

import { Icon } from "../components/Icon";
import { Notice } from "../components/Notice";
import { PageTop } from "../components/PageTop";
import {
  NightSelect,
  ReservedLine,
  TabHead,
  offersOpener,
  useSpaceWeb,
} from "../components/TabParts";
import { useI18n } from "../i18n";
import {
  folderOpen,
  nightAnalysis,
  nightDetect,
  nightFoldersList,
  nightOnline,
  nightSendLogs,
  openInBrowser,
  type CapabilitiesState,
  type DetectedLog,
  type NightAnalysis,
  type NightAnalysisSent,
} from "../lib/bridge";
import { formatError } from "../lib/format";
import {
  logLine,
  logsState,
  orderedLogs,
  replacesLine,
  sentLines,
  sentNightLine,
} from "../lib/logsTab";
import { isReserved, nightChoices, type NightChoice } from "../lib/tabs";
import {
  buttonClassName,
  helpTextClassName,
  inactiveButtonClassName,
} from "../theme";
import { nightPage } from "./night/NightWizard";

/**
 * « Journaux »: the same form as « Progression » -- a message
 * at the head, the night selector, the logs found in `logs/` with their
 * size, one primary that names its object. It sends a night's report alone,
 * without reopening the wizard: the report goes, the raw logs stay, an
 * analysis online is replaced. Reserved without `analysis.logs` (never
 * an offer read: a private server grants it).
 */
export function LogsScreen({
  capabilitiesState,
}: {
  capabilitiesState: CapabilitiesState | null;
}) {
  const { t, locale } = useI18n();
  const { web } = useSpaceWeb();
  const offers = offersOpener(web, locale);
  const reserved = isReserved(capabilitiesState, "analysis.logs");
  const [choices, setChoices] = useState<NightChoice[] | null>(null);
  const [nightId, setNightId] = useState<string | null>(null);
  const [logs, setLogs] = useState<DetectedLog[] | null>(null);
  const [previousAt, setPreviousAt] = useState<string | null>(null);
  const [sending, setSending] = useState(false);
  const [sent, setSent] = useState<{
    analysis: NightAnalysis;
    sent: NightAnalysisSent;
  } | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    nightFoldersList()
      .then((folders) => {
        if (cancelled) return;
        const list = nightChoices(folders, locale);
        setChoices(list);
        setNightId(list[0]?.nightId ?? null);
      })
      .catch((caught: unknown) => {
        if (!cancelled) {
          setChoices([]);
          setError(formatError(t, caught));
        }
      });
    return () => {
      cancelled = true;
    };
  }, [t, locale]);

  const night = choices?.find((choice) => choice.nightId === nightId) ?? null;

  useEffect(() => {
    if (!night || reserved) return;
    let cancelled = false;
    setLogs(null);
    setPreviousAt(null);
    setSent(null);
    setError(null);
    nightDetect(night.folder, {
      eveningDate: night.eveningDate,
      nightStartedAt: null,
      targets: [],
    })
      .then((detection) => {
        if (!cancelled) setLogs(orderedLogs(detection.logs));
      })
      .catch((caught: unknown) => {
        if (!cancelled) {
          setLogs([]);
          setError(formatError(t, caught));
        }
      });
    nightOnline(night.nightId)
      .then((online) => {
        if (!cancelled) setPreviousAt(online.analysis?.sentAt ?? null);
      })
      .catch(() => undefined);
    return () => {
      cancelled = true;
    };
  }, [night, reserved, t]);

  async function send() {
    if (!night) return;
    setSending(true);
    setError(null);
    try {
      const analysis = await nightAnalysis(night.folder);
      const answer = await nightSendLogs(night.nightId, night.folder);
      setSent({ analysis, sent: answer });
    } catch (caught) {
      setError(formatError(t, caught));
    } finally {
      setSending(false);
    }
  }

  const state = logsState({
    reserved,
    hasNight: (choices?.length ?? 0) > 0,
    logs,
    sending,
    sent: sent !== null,
  });
  const known = (logs ?? []).filter((log) => log.kind !== "unknown");
  const replaces = replacesLine(t, locale, previousAt);
  const ready = state === "found" && logs !== null;

  return (
    <div className="as-page">
      <PageTop>
        <h1 className="as-page__title">{t("nav.logs")}</h1>
      </PageTop>
      <TabHead
        title={t("logs.headTitle")}
        line={t("logs.headLine")}
        note={t("logs.headNote")}
        details={<p>{t("logs.headDetails")}</p>}
      />
      {reserved && <ReservedLine text={t("logs.reserved")} onOffers={offers} />}
      <Notice tone="error">{error}</Notice>

      {state === "noNight" && choices !== null && (
        <p className={helpTextClassName}>{t("tabs.noNight")}</p>
      )}

      {state === "sent" && sent && night ? (
        <div className="as-form-card">
          <h2 className="as-form-card__title">{t("logs.sent")}</h2>
          <p className={helpTextClassName}>
            {sentNightLine(t, locale, night.eveningDate, night.place)}
          </p>
          <ul className="as-lines">
            {sentLines(t, locale, sent.analysis, sent.sent, previousAt).map(
              (line) => (
                <li key={line} className="as-line as-line--ok">
                  <Icon name="check" className="as-line__glyph" />
                  <span className="as-line__text">{line}</span>
                </li>
              ),
            )}
          </ul>
          <p className={helpTextClassName}>
            {t("logs.stay", { path: "logs/" })}
          </p>
          <div className="as-actions">
            <button
              type="button"
              className={buttonClassName("primary")}
              disabled={!web}
              onClick={() =>
                web && void openInBrowser(nightPage(web, locale, night.nightId))
              }
            >
              <Icon name="external" className="as-button__icon" />
              {t("wizard.sent.openNight")}
            </button>
          </div>
        </div>
      ) : (
        (choices?.length ?? 0) > 0 && (
          <div className={reserved ? "as-form-card is-greyed" : "as-form-card"}>
            <h2 className="as-form-card__title">{t("logs.formTitle")}</h2>
            {!reserved && (
              <p className={helpTextClassName}>
                {t("logs.readFrom", { path: "logs/" })}
              </p>
            )}
            <div className="as-select-grid">
              <NightSelect
                choices={choices ?? []}
                value={nightId}
                disabled={reserved || sending}
                onChange={setNightId}
              />
            </div>
            {!reserved && logs !== null && night && (
              <div className="as-found">
                <div className="as-found__head">
                  <h3 className="as-found__title">{t("logs.found")}</h3>
                  <button
                    type="button"
                    className={buttonClassName("link")}
                    onClick={() => void folderOpen(`${night.folder}/logs`)}
                  >
                    <Icon name="folder" className="as-button__icon" />
                    {t("logs.openFolder")}
                  </button>
                </div>
                {known.length > 0 ? (
                  <ul className="as-lines">
                    {logs.map((log) => (
                      <li
                        key={log.relativePath}
                        className={
                          log.kind === "unknown"
                            ? "as-line as-line--none"
                            : "as-line as-line--ok"
                        }
                      >
                        <Icon
                          name={log.kind === "unknown" ? "pending" : "check"}
                          className="as-line__glyph"
                        />
                        <span className="as-line__text">
                          <code className="as-code">logs/</code>
                          {" · "}
                          {logLine(t, locale, log, night.eveningDate)}
                        </span>
                      </li>
                    ))}
                  </ul>
                ) : (
                  <p className={helpTextClassName}>
                    {t("logs.none", { path: "logs/" })}
                  </p>
                )}
              </div>
            )}
            {ready && replaces && (
              <p className="as-amber">
                <Icon name="warning" />
                {replaces}
              </p>
            )}
            {sending && (
              <p className={helpTextClassName}>{t("logs.sendingNotice")}</p>
            )}
            <div className="as-actions">
              {ready && !sending ? (
                <button
                  type="button"
                  className={buttonClassName("primary")}
                  onClick={() => void send()}
                >
                  <Icon name="upload" className="as-button__icon" />
                  {t("logs.send")}
                </button>
              ) : (
                <button
                  type="button"
                  className={inactiveButtonClassName}
                  disabled
                >
                  <Icon name="upload" className="as-button__icon" />
                  {t("logs.send")}
                </button>
              )}
            </div>
          </div>
        )
      )}
    </div>
  );
}
