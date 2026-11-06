import { Icon } from "../../components/Icon";
import { useImport } from "../../components/ImportProvider";
import { useSpaceWeb } from "../../components/TabParts";
import { useUploads } from "../../components/UploadsProvider";
import { WizardFoot } from "../../components/WizardFrame";
import { useI18n } from "../../i18n";
import { openInBrowser } from "../../lib/bridge";
import { noneSentReason, type NoneSentReason } from "../../lib/importFlow";
import { resultRows } from "../../lib/detectResult";
import { nightDay } from "../../lib/nights";
import { tidyReportLines } from "../../lib/tidy";
import {
  latestBatch,
  percentOf,
  progressLine,
  skyLine,
  stateLine,
  titleLine,
  type UploadRow,
  type UploadsState,
} from "../../lib/uploads";
import { pluralText } from "../../lib/written";
import {
  buttonClassName,
  helpTextClassName,
  progressClassName,
} from "../../theme";
import { plainPrimaryLabel } from "./importWords";

/** What « Terminé » says of one night's incidents. */
export interface DoneIncidents {
  night: string;
  added: number;
  rejected: number;
  refused: boolean;
}

/** What « Terminé » says of one night's logs. */
export interface DoneLogs {
  night: string;
  /** `null`: sent; else why not. */
  failed: string | null;
}

export interface DoneView {
  made: number;
  updated: number;
  plans: number;
  sent: number;
  /** Of them, the variants of another image. */
  variants: number;
  /** Why no image was sent (when `sent` is 0); `null`: nothing to add. */
  noneReason?: NoneSentReason | null;
  incidents: DoneIncidents[];
  /** The tidying, line by line; `null` when it was not asked. */
  tidy: string[] | null;
  /** The report of each night's logs: sent, or why not (absent in the views
   * drawn before it existed). */
  logs?: DoneLogs[];
  /** The images of this import, as the uploads hold them. */
  batch: UploadRow[];
  sky: UploadsState["sky"];
  web: string | null;
}

/**
 * Step 5 of the import, « Terminé »: what was created, the incidents added
 * to each night and the files tidied, the images that left and, for each,
 * where it is -- the same bar as the row of « Envois » while it leaves,
 * « Envoyée » once done -- and its sky annotations as they happen; the ways on
 * (« Voir les envois », the space on the web). No « Précédent »: one primary,
 * « Retour aux sessions », which closes the import.
 */
export function ImportDone({ onLeave }: { onLeave: () => void }) {
  const { t, locale } = useI18n();
  const imp = useImport();
  const { state } = imp;
  const uploads = useUploads();
  const { web } = useSpaceWeb();
  if (!state.proposal || !state.created) return null;
  const outcome = state.created.outcome;
  const rows = resultRows(state.proposal, outcome);
  const made = rows.filter((row) => row.action === "created").length;
  const sent = state.sent ?? 0;
  const nightOf = (sessionId: string) => {
    const row = rows.find((item) => item.sessionId === sessionId);
    return row
      ? [
          row.designations.join(", "),
          nightDay(`${row.eveningDate}T12:00:00Z`, locale, "UTC"),
        ].join(t("detect.join"))
      : sessionId;
  };
  const view: DoneView = {
    made,
    updated: rows.length - made,
    plans: outcome.multiNight.length,
    sent,
    variants: Math.min(sent, state.sentVariants),
    noneReason: sent === 0 ? noneSentReason(state) : null,
    incidents: rows.flatMap((row) => {
      const result = state.report.incidents[row.sessionId];
      return result
        ? [
            {
              night: nightOf(row.sessionId),
              added: result.added + result.replaced,
              rejected: result.rejected,
              refused: result.refused,
            },
          ]
        : [];
    }),
    tidy: state.report.tidy
      ? tidyReportLines(t, locale, state.report.tidy)
      : null,
    logs: Object.entries(state.report.logs).map(([sessionId, result]) => ({
      night: nightOf(sessionId),
      failed: result.sent ? null : result.reason,
    })),
    // The images of this import: the last group the detection queued.
    batch: sent > 0 ? latestBatch(uploads.state, "detect") : [],
    sky: uploads.state.sky,
    web,
  };
  return (
    <ImportDoneView
      view={view}
      onRetrySky={uploads.retrySky}
      onOpenUploads={uploads.openPage}
      onLeave={() => {
        imp.close();
        onLeave();
      }}
    />
  );
}

/** The page itself, from plain data (drawn to a string by the tests). */
export function ImportDoneView({
  view,
  onRetrySky,
  onOpenUploads,
  onLeave,
}: {
  view: DoneView;
  onRetrySky: (sha256: string) => void;
  onOpenUploads: () => void;
  onLeave: () => void;
}) {
  const { t, locale } = useI18n();
  const { web, sent, variants } = view;
  return (
    <>
      <section className="as-card as-import-done">
        <h2 className="as-result__heading">{t("import.done.made")}</h2>
        <ul className="as-import-done__lines">
          <li>
            <Icon name="check" />
            <span>
              {pluralText(t, locale, "detect.result.title", view.made)}
            </span>
          </li>
          {view.updated > 0 && (
            <li>
              <Icon name="check" />
              <span>
                {pluralText(t, locale, "import.done.updated", view.updated)}
              </span>
            </li>
          )}
          {view.plans > 0 && (
            <li>
              <Icon name="check" />
              <span>
                {pluralText(t, locale, "import.done.plans", view.plans)}
              </span>
            </li>
          )}
          {view.incidents.length === 0 ? (
            <li className="is-later" data-done="incidents">
              <Icon name="pending" />
              <span>{t("import.done.noIncidents")}</span>
            </li>
          ) : (
            view.incidents.map((night) => (
              <li key={night.night} data-done="incidents">
                <Icon name="check" />
                <span>
                  {pluralText(t, locale, "import.done.incidents", night.added, {
                    night: night.night,
                  })}
                  {night.rejected > 0 && (
                    <>
                      {" "}
                      {pluralText(
                        t,
                        locale,
                        "import.done.incidentsRejected",
                        night.rejected,
                      )}
                    </>
                  )}
                  {night.refused && <> {t("import.done.incidentsRefused")}</>}
                </span>
              </li>
            ))
          )}
          {view.tidy === null ? (
            <li className="is-later" data-done="tidy">
              <Icon name="pending" />
              <span>{t("import.done.noTidy")}</span>
            </li>
          ) : (
            view.tidy.map((line) => (
              <li key={line} data-done="tidy">
                <Icon name="check" />
                <span>{line}</span>
              </li>
            ))
          )}
          {(view.logs ?? []).map((night) => (
            <li
              key={`logs-${night.night}`}
              data-done="logs"
              className={night.failed === null ? undefined : "is-later is-warn"}
            >
              <Icon name={night.failed === null ? "check" : "pending"} />
              <span>
                {night.failed === null
                  ? t("import.done.logsSent", { night: night.night })
                  : t("import.done.logsFailed", {
                      night: night.night,
                      reason: night.failed,
                    })}
              </span>
            </li>
          ))}
          <li
            className={
              sent > 0
                ? undefined
                : view.noneReason === "offer" || view.noneReason === "failed"
                  ? "is-later is-warn"
                  : "is-later"
            }
          >
            <Icon name={sent > 0 ? "check" : "pending"} />
            <span>
              {sent === 0
                ? view.noneReason
                  ? t("import.done.noneSentWhy", {
                      reason: t(`import.done.reason.${view.noneReason}`),
                    })
                  : t("import.done.noneSent")
                : variants === 0
                  ? pluralText(t, locale, "import.done.sent", sent)
                  : t("import.done.sentSplit", {
                      captures: pluralText(
                        t,
                        locale,
                        "import.done.sentCaptures",
                        sent - variants,
                      ),
                      variants: pluralText(
                        t,
                        locale,
                        "import.done.sentVariants",
                        variants,
                      ),
                    })}
            </span>
          </li>
        </ul>
        {view.batch.length > 0 && (
          <ul className="as-import-done__images">
            {view.batch.map((row) => {
              const sky = view.sky[row.sha256];
              const skyText = sky ? skyLine(t, locale, sky) : null;
              const leaving = row.status === "uploading";
              return (
                <li key={row.sha256} data-status={row.status}>
                  <strong>{titleLine(t, locale, row)}</strong>
                  <span className={helpTextClassName}>
                    {leaving
                      ? progressLine(t, locale, row)
                      : stateLine(t, locale, row)}
                  </span>
                  {leaving && (
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
                  <span
                    className={`as-import-done__sky is-${skyText?.tone ?? "neutral"}`}
                  >
                    {skyText?.text ?? t("import.done.skyLater")}
                    {skyText?.retry && (
                      <button
                        type="button"
                        className={buttonClassName("link")}
                        onClick={() => onRetrySky(row.sha256)}
                      >
                        {t("wizard.retry")}
                      </button>
                    )}
                  </span>
                </li>
              );
            })}
          </ul>
        )}
      </section>
      <section className="as-card as-import-done">
        <h2 className="as-result__heading">{t("import.done.next")}</h2>
        <p className={helpTextClassName}>{t("import.done.leave")}</p>
        <div className="as-import-done__links">
          <button
            type="button"
            className={buttonClassName("link")}
            onClick={onOpenUploads}
          >
            {t("uploads.open")}
          </button>
          {web && (
            <button
              type="button"
              className={buttonClassName("link")}
              onClick={() =>
                void openInBrowser(
                  `${web.replace(/\/$/, "")}/${locale}/sessions`,
                ).catch(() => undefined)
              }
            >
              <Icon name="external" className="as-button__icon" />
              {t("import.done.space")}
            </button>
          )}
        </div>
      </section>
      <WizardFoot
        primary={{
          label: plainPrimaryLabel(t, "backToSessions"),
          onClick: onLeave,
          className: "as-import-done__back",
        }}
      />
    </>
  );
}
