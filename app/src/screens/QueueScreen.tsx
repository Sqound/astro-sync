import { useCallback, useEffect, useState } from "react";

import { Notice } from "../components/Notice";
import { PaymentFailedNotice } from "../components/PaymentFailedNotice";
import { useSpaceWeb } from "../components/TabParts";
import { useUploads } from "../components/UploadsProvider";
import { Waiting } from "../components/Waiting";
import { useI18n } from "../i18n";
import {
  openInBrowser,
  queueForgetBeforeLink,
  queueStatus,
  spaceStorageGet,
  type QueueEntry,
  type SkyAnnotation,
} from "../lib/bridge";
import { formatBytes, formatDateTime } from "../lib/format";
import { windowStorage } from "../lib/lastReport";
import { SOURCE_FILE_MISSING, rowReason } from "../lib/queue";
import {
  readOfFailure,
  storageView,
  type StorageRead,
  type StorageView,
} from "../lib/storageCard";
import {
  beforeLinkRows,
  capturePage,
  clearedAfter,
  historyRows,
  loadClearedAt,
  saveClearedAt,
  type HistoryRow,
} from "../lib/uploadHistory";
import {
  liveRows,
  percentOf,
  progressLine,
  skyLine,
  stateLine,
  titleLine,
  type UploadRow,
} from "../lib/uploads";
import {
  badgeClassName,
  buttonClassName,
  helpTextClassName,
  progressClassName,
} from "../theme";

/**
 * « Envois »: the one place for the uploads. At the head, how much of the
 * space is used; then what is leaving now (a bar, the bytes, the speed, the
 * reason of a refusal, « Réessayer » after a network failure); then the
 * history of every upload -- it is the queue's own database, so it outlives
 * the window --, newest first, which can be cleared.
 */
export function QueueScreen() {
  const { t, locale } = useI18n();
  const {
    state,
    running,
    paused,
    start,
    pause,
    retry,
    retrySky,
    resend,
    labels,
    resumePending,
  } = useUploads();
  const { web } = useSpaceWeb();
  const [entries, setEntries] = useState<QueueEntry[] | null>(null);
  // What was sent (or queued) before this computer was linked to the space:
  // shown apart, never this space's.
  const [beforeLink, setBeforeLink] = useState<QueueEntry[]>([]);
  const [kept, setKept] = useState<readonly SkyAnnotation[]>([]);
  const [unread, setUnread] = useState(false);
  const [storage, setStorage] = useState<StorageRead>({ kind: "loading" });
  const [clearedAt, setClearedAt] = useState<string | null>(() =>
    loadClearedAt(windowStorage()),
  );

  const refresh = useCallback(() => {
    queueStatus()
      .then((status) => {
        setEntries(status.entries);
        setBeforeLink(status.beforeLink ?? []);
        setKept(status.sky ?? []);
        setUnread(false);
      })
      .catch(() => setUnread(true));
    spaceStorageGet()
      .then((value) => setStorage({ kind: "ok", storage: value }))
      .catch((caught) => setStorage(readOfFailure(caught)));
  }, []);

  // At the opening, and each time a file reaches an end.
  useEffect(() => {
    refresh();
  }, [refresh, state.revision]);

  // Files an earlier run left in the queue go on by themselves.
  useEffect(() => {
    resumePending();
  }, [resumePending]);

  const live = liveRows(state);
  const history = historyRows(entries ?? [], labels(), clearedAt);
  const earlier = beforeLinkRows(beforeLink, labels());
  const queued = live.filter((row) => row.status === "queued").length;
  // What the events told is newer than the last read of the database.
  const skyOf = (sha256: string): SkyAnnotation | undefined =>
    state.sky[sha256] ?? kept.find((item) => item.sha256 === sha256);

  function clear() {
    const next = clearedAfter(entries ?? [], clearedAt);
    saveClearedAt(windowStorage(), next);
    setClearedAt(next);
    // What came before the link is deleted from the queue: it belongs to no
    // space this computer knows.
    if (beforeLink.length > 0)
      void queueForgetBeforeLink()
        .then(refresh)
        .catch(() => undefined);
  }

  return (
    <div className="as-page as-uploads">
      <h1 className="as-page__title">{t("queue.title")}</h1>
      <p className={helpTextClassName}>{t("queue.description")}</p>

      <StorageCard view={storageView(storage)} onRefresh={refresh} />

      <section className="as-card as-uploads__section">
        <div className="as-uploads__head">
          <h2 className="as-result__heading">{t("uploads.live.title")}</h2>
          <div className="as-uploads__actions">
            {running && (
              <button
                type="button"
                className={buttonClassName("tertiary")}
                onClick={pause}
              >
                {t("queue.pauseButton")}
              </button>
            )}
            {!running && (paused || queued > 0) && (
              <button
                type="button"
                className={buttonClassName("secondary")}
                onClick={start}
              >
                {paused ? t("queue.resumeButton") : t("queue.uploadAllButton")}
              </button>
            )}
          </div>
        </div>
        {live.length === 0 ? (
          <p className={helpTextClassName}>{t("uploads.live.empty")}</p>
        ) : (
          <ul className="as-uploads__list">
            {live.map((row) => (
              <LiveItem
                key={row.sha256}
                row={row}
                web={web}
                onRetry={(sha256) => void retry(sha256).then(refresh)}
                onResend={(sha256) => void resend(sha256).then(refresh)}
              />
            ))}
          </ul>
        )}
      </section>

      <section className="as-card as-uploads__section">
        <div className="as-uploads__head">
          <h2 className="as-result__heading">{t("uploads.history.title")}</h2>
          {(history.length > 0 || earlier.length > 0) && (
            <button
              type="button"
              className={buttonClassName("link")}
              onClick={clear}
            >
              {t("uploads.history.clear")}
            </button>
          )}
        </div>
        {unread && (
          <Notice tone="warning">{t("uploads.history.unread")}</Notice>
        )}
        {entries === null && !unread && (
          <Waiting label={t("common.wait.queue")} />
        )}
        {entries !== null && history.length === 0 && (
          <p className={helpTextClassName}>{t("uploads.history.empty")}</p>
        )}
        <ul className="as-uploads__list">
          {history.map((row) => (
            <HistoryItem
              key={row.sha256}
              row={row}
              web={web}
              locale={locale}
              sky={skyOf(row.sha256)}
              onRetrySky={retrySky}
              onRetry={(sha256) => void retry(sha256).then(refresh)}
            />
          ))}
        </ul>
      </section>

      {earlier.length > 0 && (
        <section
          className="as-card as-uploads__section"
          data-section="before-link"
        >
          <div className="as-uploads__head">
            <h2 className="as-result__heading">
              {t("uploads.beforeLink.title")}
            </h2>
          </div>
          <p className={helpTextClassName}>
            {t("uploads.beforeLink.description")}
          </p>
          <ul className="as-uploads__list">
            {earlier.map((row) => (
              <li
                key={row.sha256}
                className="as-uploads__item"
                data-status={row.state}
              >
                <span className="as-uploads__title">
                  {titleLine(t, locale, row)}
                </span>
                <span className="as-uploads__state">
                  <span className={badgeClassName("neutral")}>
                    {t(`status.${row.state}`)}
                  </span>
                  <span className={helpTextClassName}>
                    {formatDateTime(row.at, locale)}
                  </span>
                </span>
              </li>
            ))}
          </ul>
        </section>
      )}
    </div>
  );
}

/** The header card: the size used, the images kept against the quota of the
 * free tier, and the states a read can end in. */
export function StorageCard({
  view,
  onRefresh,
}: {
  view: StorageView;
  onRefresh: () => void;
}) {
  const { t, locale } = useI18n();
  return (
    <section className="as-card as-uploads__storage" data-state={view.kind}>
      <div className="as-uploads__head">
        <h2 className="as-result__heading">{t("uploads.storage.title")}</h2>
        <button
          type="button"
          className={buttonClassName("link")}
          onClick={onRefresh}
        >
          {t("common.refresh")}
        </button>
      </div>
      {/* A payment that failed: the space is read-only, what leaves is held. */}
      <PaymentFailedNotice variant="state" />
      {view.kind === "loading" && (
        <p className={helpTextClassName}>{t("uploads.storage.loading")}</p>
      )}
      {view.kind === "unreachable" && (
        <Notice tone="warning">{t("uploads.storage.unreachable")}</Notice>
      )}
      {view.kind === "unpaired" && (
        <Notice tone="warning">{t("queue.notPairedNotice")}</Notice>
      )}
      {view.kind === "ok" && (
        <>
          <p className="as-uploads__figure">
            {view.quotaBytes === null
              ? t("uploads.storage.usedOnly", {
                  used: formatBytes(t, view.usedBytes, locale),
                })
              : t("uploads.storage.used", {
                  used: formatBytes(t, view.usedBytes, locale),
                  quota: formatBytes(t, view.quotaBytes, locale),
                })}
          </p>
          {view.usedFraction !== null && (
            <Bar
              fraction={view.usedFraction}
              label={t("uploads.storage.title")}
            />
          )}
          {view.images && (
            <>
              <p className="as-uploads__figure">
                {t("uploads.storage.images", {
                  count: view.images.count,
                  limit: view.images.limit,
                })}
              </p>
              <Bar
                fraction={view.images.fraction}
                label={t("uploads.storage.images", {
                  count: view.images.count,
                  limit: view.images.limit,
                })}
              />
            </>
          )}
          {view.maxFileBytes !== null && (
            <p className={helpTextClassName}>
              {t("uploads.storage.maxFile", {
                size: formatBytes(t, view.maxFileBytes, locale),
              })}
            </p>
          )}
        </>
      )}
    </section>
  );
}

function Bar({ fraction, label }: { fraction: number; label: string }) {
  const percent = Math.round(fraction * 100);
  return (
    <div
      className={progressClassName.track}
      role="progressbar"
      aria-label={label}
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={percent}
    >
      <div
        className={progressClassName.fill}
        style={{ width: `${percent}%` }}
      />
    </div>
  );
}

function LiveItem({
  row,
  web,
  onRetry,
  onResend,
}: {
  row: UploadRow;
  web: string | null;
  onRetry: (sha256: string | null) => void;
  onResend: (sha256: string) => void;
}) {
  const { t, locale } = useI18n();
  return (
    <li className="as-uploads__item" data-status={row.status}>
      <span className="as-uploads__title">{titleLine(t, locale, row)}</span>
      <span className="as-uploads__state">
        {row.status === "uploading"
          ? progressLine(t, locale, row)
          : stateLine(t, locale, row)}
      </span>
      {row.status === "uploading" && (
        <div
          className={progressClassName.track}
          role="progressbar"
          aria-valuemin={0}
          aria-valuemax={100}
          aria-valuenow={percentOf(row)}
        >
          <div
            className={progressClassName.fill}
            style={{ width: `${percentOf(row)}%` }}
          />
        </div>
      )}
      {row.status === "failed" && row.retryable && (
        <button
          type="button"
          className={`${buttonClassName("link")} as-uploads__action`}
          onClick={() => onRetry(row.sha256)}
        >
          {t("uploads.row.retry")}
        </button>
      )}
      {/* Already in this space: its capture, and « Renvoyer » (its capture
          may have been deleted on the web since). */}
      {Boolean(row.alreadySentAt) && row.captureId && web && (
        <button
          type="button"
          className={`${buttonClassName("link")} as-uploads__action`}
          onClick={() =>
            void openInBrowser(capturePage(web, locale, row.captureId!))
          }
        >
          {t("uploads.row.capture")}
        </button>
      )}
      {Boolean(row.alreadySentAt) && (
        <button
          type="button"
          className={`${buttonClassName("link")} as-uploads__action`}
          onClick={() => onResend(row.sha256)}
        >
          {t("uploads.row.resend")}
        </button>
      )}
    </li>
  );
}

function HistoryItem({
  row,
  web,
  locale,
  sky,
  onRetrySky,
  onRetry,
}: {
  row: HistoryRow;
  web: string | null;
  locale: string;
  sky: SkyAnnotation | undefined;
  onRetrySky: (sha256: string) => void;
  onRetry: (sha256: string) => void;
}) {
  const { t } = useI18n();
  const reason = rowReason(t, row.entry);
  const skyText =
    row.state === "completed" && sky !== undefined
      ? skyLine(t, locale, sky)
      : null;
  return (
    <li className="as-uploads__item" data-status={row.state}>
      <span className="as-uploads__title">{titleLine(t, locale, row)}</span>
      <span className="as-uploads__state">
        <span
          className={badgeClassName(
            row.state === "completed" ? "success" : "error",
          )}
        >
          {t(`status.${row.state}`)}
        </span>
        {reason && <span>{reason}</span>}
        <span className={helpTextClassName}>
          {formatDateTime(row.at, locale)}
        </span>
      </span>
      {skyText && (
        <span
          className="as-uploads__sky"
          data-tone={skyText.tone}
          data-sky={sky?.state}
        >
          <span>{skyText.text}</span>
          {skyText.retry && (
            <button
              type="button"
              className={buttonClassName("link")}
              onClick={() => onRetrySky(row.sha256)}
            >
              {t("uploads.row.retry")}
            </button>
          )}
        </span>
      )}
      {row.state === "failed" &&
        row.entry.lastErrorCode === SOURCE_FILE_MISSING && (
          <button
            type="button"
            className={`${buttonClassName("link")} as-uploads__action`}
            onClick={() => onRetry(row.sha256)}
          >
            {t("uploads.row.retry")}
          </button>
        )}
      {row.state === "completed" && row.captureId && web && (
        <button
          type="button"
          className={`${buttonClassName("link")} as-uploads__action`}
          onClick={() =>
            void openInBrowser(capturePage(web, locale, row.captureId!))
          }
        >
          {t("uploads.row.capture")}
        </button>
      )}
    </li>
  );
}
