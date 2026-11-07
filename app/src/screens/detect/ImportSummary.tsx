import { useEffect, useState } from "react";

import { Icon } from "../../components/Icon";
import { useImport } from "../../components/ImportProvider";
import { InfoLine } from "../../components/InfoLine";
import { JoinTiffBox } from "../../components/JoinTiffBox";
import { Notice } from "../../components/Notice";
import { PaymentFailedNotice } from "../../components/PaymentFailedNotice";
import { useSnackbar } from "../../components/Snackbar";
import { offersOpener, useSpaceWeb } from "../../components/TabParts";
import { WizardFoot } from "../../components/WizardFrame";
import { useI18n } from "../../i18n";
import {
  detectTreatmentsPrepare,
  sessionsOrganisePlan,
  workingFolderGet,
  type Proposal,
  type WizardStep,
} from "../../lib/bridge";
import {
  addedTargets,
  attachedFrames,
  resultRows,
  sessionPreparation,
  shiftText,
  skippedRows,
} from "../../lib/detectResult";
import { imageRoom, summaryImages } from "../../lib/detectTreatments";
import { formatError } from "../../lib/format";
import { displayPath, homeOf } from "../../lib/home";
import {
  footRule,
  recapLine,
  recapPlan,
  recapProgress,
  workingOutcome,
  type ImportState,
} from "../../lib/importFlow";
import { tidyNames, tidyStateOf } from "../../lib/tidy";
import { nightDay } from "../../lib/nights";
import { summaryImageView } from "../../lib/summaryImages";
import { formatMinutes, pluralText } from "../../lib/written";
import {
  badgeClassName,
  buttonClassName,
  helpTextClassName,
} from "../../theme";
import { usePipelines } from "./ImportTreatments";
import { OrganiseDialog } from "./OrganiseDialog";
import { ResultIncidents } from "./ResultIncidents";
import { SummaryImages } from "./SummaryImages";
import { plainPrimaryLabel, secondaryLabel } from "./importWords";

type Translate = ReturnType<typeof useI18n>["t"];

/** « Récapitulatif » and its line: the sessions created -- or to create,
 * before the primary --, the plans, the frames attached, the targets added. */
export function summaryHead(
  t: Translate,
  locale: string,
  state: ImportState,
): { title: string; line: string | null } {
  const title = t("wizard.summary.title");
  const working = workingOutcome(state);
  if (!state.proposal || !working) return { title, line: null };
  const outcome = working.outcome;
  const rows = resultRows(state.proposal, outcome);
  const made = rows.filter((row) => row.action === "created").length;
  const lead = state.created
    ? "detect.result.title"
    : "import.summary.toCreate";
  const attached = attachedFrames(rows);
  const plans = outcome.multiNight;
  const day = (date: string) =>
    new Intl.DateTimeFormat(locale, {
      day: "numeric",
      month: "long",
      timeZone: "UTC",
    }).format(new Date(`${date}T12:00:00Z`));
  const line = [
    pluralText(t, locale, lead, made),
    plans.length > 0
      ? pluralText(t, locale, "detect.result.plansNamed", plans.length, {
          names: plans.map((plan) => plan.designation).join(", "),
        })
      : null,
    attached.sessions > 0
      ? t("detect.result.attached", {
          frames: attached.frames.toLocaleString(locale),
          sessions: attached.sessions,
        })
      : null,
    ...addedTargets(outcome).map((added) =>
      t("detect.result.added", {
        target: added.designation,
        date: day(added.eveningDate),
      }),
    ),
  ]
    .filter(Boolean)
    .join(t("detect.join"));
  return { title, line };
}

/**
 * Step 4 of the import, « Récapitulatif » (6 a and 6 c): what will be created or
 * updated (once created, a row opens the session's wizard), what will not and why,
 * the incidents of each session, « Vos fichiers » (the option « Ranger les poses
 * dans le dossier de travail »), then « Images à envoyer »: every image the
 * treatments made, ticked unless the space cannot take it, in the place left. One
 * primary: « Créer et envoyer », or « Créer » with none ticked -- nothing was
 * written to the space before it -- and it applies everything, in order: the
 * sessions and the plans, the incidents of every night, the tidying, the images (all
 * applied when the person goes to the next step), each stage said on the line above
 * it. No card has a button of its own that writes.
 */
export function ImportSummary({
  onOpenNight,
}: {
  onOpenNight: (
    sessionId: string,
    step: WizardStep | null,
    targetId?: string | null,
  ) => void;
}) {
  const { t, locale } = useI18n();
  const { notify } = useSnackbar();
  const { web } = useSpaceWeb();
  const offers = offersOpener(web, locale);
  const imp = useImport();
  const { state } = imp;
  const { pipelineName } = usePipelines();
  const [organising, setOrganising] = useState(false);
  const setOverview = imp.setTidyOverview;
  const tidyMode = state.tidy.mode;
  const tidyDestination = state.tidy.destination;
  // What the working folder holds already: the treatments linked every frame
  // into it, so "Ranger" may have nothing left to do.
  const working = workingOutcome(state);
  useEffect(() => {
    const held = state.proposal;
    const made = working;
    if (!held || !made) return;
    let current = true;
    const ids = made.outcome.sessions.map((s) => s.detectedId);
    void (async () => {
      try {
        const destination = tidyDestination ?? (await workingFolderGet()).root;
        const plan = await sessionsOrganisePlan(
          held,
          destination,
          tidyMode,
          ids,
          tidyNames(held, made.outcome),
        );
        if (current) setOverview(tidyStateOf(plan));
      } catch {
        // Unknown: the option stays offered, the primary asks again.
        if (current) setOverview(null);
      }
    })();
    return () => {
      current = false;
    };
  }, [state.proposal, working, tidyMode, tidyDestination, setOverview]);
  // The session whose night folder is being made before its wizard opens.
  const [opening, setOpening] = useState<string | null>(null);
  const proposal = state.proposal;
  // Before the primary, what will be created (provisional identifiers); a row
  // opens a session's wizard only once it exists.
  const created = working;
  const exists = state.created !== null;
  if (!proposal || !created) return null;
  const rows = resultRows(proposal, created.outcome);
  const skipped = skippedRows(proposal, created.outcome);
  const images = summaryImages(state.made, state.storage);
  const room = imageRoom(state.storage);
  const roomText = room
    ? pluralText(t, locale, "detect.treat.room", room.remaining, {
        quota: room.quota,
      })
    : null;
  const folder = state.choices.fits ?? commonFolder(proposal);
  const rule = footRule(state);
  const names = tidyNames(proposal, created.outcome);
  const plan = recapPlan(state, "");
  const overview = state.tidyOverview;
  const how = (kind: "link" | "file" | "mixed" | null) =>
    t(`import.tidy.how.${kind ?? "file"}`);

  /** Opens a session's wizard (a row, « Mesurer les poses »). The session the
   * import made has no folder on this computer yet: its frames, calibration
   * and logs are linked into one first (never moved), as the treatments do,
   * so that « Fichiers », the measure of the poses and « Récapitulatif » read
   * it as any night. A session whose folder cannot be made still opens, and
   * the reason is said. The import stays open: the menu's « Import en cours »
   * comes back to this step. */
  async function openSession(
    sessionId: string,
    step: WizardStep | null,
    targetId?: string | null,
  ) {
    if (opening || !proposal || !created || !exists) return;
    const preparation = sessionPreparation(
      proposal,
      created.outcome,
      sessionId,
    );
    if (preparation) {
      setOpening(sessionId);
      try {
        await detectTreatmentsPrepare(
          proposal,
          preparation.outcome,
          preparation.targetIds,
        );
      } catch (caught) {
        notify({
          tone: "error",
          text: `${t("wizard.folder.failed")} ${formatError(t, caught)}`,
        });
      } finally {
        setOpening(null);
      }
    }
    onOpenNight(sessionId, step, targetId);
  }

  return (
    <>
      {/* A payment that failed: the space is read-only, nothing will be sent
          -- said before the primary of the last step. */}
      {!exists && <PaymentFailedNotice variant="beforeSending" />}
      {exists && created.outcome.replayed && (
        <Notice tone="neutral">{t("detect.result.replayed")}</Notice>
      )}
      {opening !== null && (
        <p className={helpTextClassName} role="status">
          {t("wizard.folder.creating")}
        </p>
      )}
      <div className="as-result__list" role="table">
        {rows.map((row) => (
          <button
            key={row.sessionId}
            type="button"
            role="row"
            className="as-result__row"
            disabled={opening !== null || !exists}
            onClick={() => void openSession(row.sessionId, null)}
          >
            <span className="as-result__day" role="cell">
              <strong>
                {nightDay(`${row.eveningDate}T12:00:00Z`, locale, "UTC")}
              </strong>
              <span className="as-nights__sub">{row.place}</span>
            </span>
            <span className="as-result__what" role="cell">
              <span>{row.designations.join(", ")}</span>
              {row.multiNight && (
                <span
                  className={`${badgeClassName("outline-gold")} as-nights__plan-tag`}
                >
                  {t("multiNight.tag", {
                    name: row.multiNight.designation,
                    nights: pluralText(
                      t,
                      locale,
                      "multiNight.nights",
                      row.multiNight.nights,
                    ),
                    total: formatMinutes(t, row.multiNight.goalSeconds / 60),
                  })}
                </span>
              )}
              {row.planName && row.planAction === "reused" && (
                <span className="as-nights__sub">
                  {t("detect.result.planReused", { plan: row.planName })}
                </span>
              )}
              {row.planName && row.planAction === "replaced" && (
                <span className="as-nights__sub">
                  {t("detect.result.planReplaced", { plan: row.planName })}
                </span>
              )}
              {row.shiftHours !== 0 && (
                <span className="as-nights__sub">
                  {t("detect.result.shifted", {
                    shift: shiftText(row.shiftHours),
                  })}
                </span>
              )}
            </span>
            <span className="as-result__state" role="cell">
              <span className="as-result__origin">
                {row.action === "created"
                  ? t(
                      exists
                        ? "detect.result.imported"
                        : "import.summary.toImport",
                    )
                  : t(
                      exists
                        ? "detect.result.updated"
                        : "import.summary.toUpdate",
                    )}
              </span>
              <span className={badgeClassName("outline")}>
                {t("detect.result.executed")}
              </span>
            </span>
            {exists && <Icon name="chevron" className="as-nights__chevron" />}
          </button>
        ))}
      </div>

      {skipped.length > 0 && (
        <section className="as-card as-result__card as-result__skipped">
          <p>
            <strong>
              {pluralText(t, locale, "detect.result.skipped", skipped.length)}
            </strong>
          </p>
          <ul>
            {skipped.map((item) => (
              <li key={`${item.kind}:${item.id}`}>
                {item.label}
                {t("detect.join")}
                {t(`detect.skip.${item.reason}`)}
              </li>
            ))}
          </ul>
        </section>
      )}

      {/* The list of the incidents of each session
          created. */}
      <ResultIncidents
        proposal={proposal}
        created={created}
        drafts={state.incidents}
        withStep={
          rule.primary?.kind === "send" || rule.primary?.kind === "createSend"
            ? "send"
            : exists
              ? "finish"
              : "create"
        }
        onDraft={imp.changeIncidents}
        onOpenNight={
          exists
            ? (sessionId, step, targetId) =>
                void openSession(sessionId, step, targetId)
            : null
        }
      />

      {overview?.kind !== "nothing" && (
        <section className="as-card as-result__card as-tidy">
          <h2 className="as-result__heading">{t("detect.files.title")}</h2>
          {overview === null ? (
            <p className={helpTextClassName} role="status">
              {t("import.tidy.checking")}
            </p>
          ) : overview.kind === "done" ? (
            // Every file is in the working folder already: said, with nothing
            // to tick.
            <p className="as-tidy__state">
              <Icon name="check" className="as-tidy__glyph" />
              <span>
                {t("import.tidy.done", { how: how(overview.inPlaceAs) })}
              </span>
            </p>
          ) : (
            <>
              <p className={helpTextClassName}>
                {t("detect.files.lead", {
                  path: displayPath(folder, homeOf(folder)),
                })}
              </p>
              {overview.kind === "partly" && (
                <p className={helpTextClassName}>
                  {t("import.tidy.partly", {
                    inPlace: overview.inPlace,
                    how: how(overview.inPlaceAs),
                    todo: overview.toDo,
                  })}
                </p>
              )}
              <label className="as-tidy__option">
                <input
                  type="checkbox"
                  checked={state.tidy.on}
                  disabled={state.sending}
                  onChange={(event) => imp.tickTidy(event.target.checked)}
                />
                <span>{t("import.tidy.label")}</span>
              </label>
              <div className="as-actions">
                <button
                  type="button"
                  className={buttonClassName("link")}
                  onClick={() => setOrganising(true)}
                >
                  {t("import.tidy.see")}
                </button>
              </div>
            </>
          )}
        </section>
      )}

      <section className="as-card as-result__card as-treat__send">
        <h2 className="as-result__heading">{t("detect.send.title")}</h2>
        <InfoLine
          line={
            roomText
              ? t("detect.send.room", { room: roomText })
              : t("detect.send.noRoom")
          }
          details={<p>{t("detect.send.details")}</p>}
        />
        {/* « Joindre aussi les TIFF »: the same box as the night's
            « Récapitulatif », over the list, when a render has a TIFF. */}
        {images.some((row) => row.fullResolution) && (
          <JoinTiffBox
            checked={state.joinTiff}
            disabled={state.sending}
            fullResolution={{
              count: images.filter((row) => row.fullResolution).length,
              bytes: images
                .filter((row) => row.fullResolution)
                .reduce((sum, row) => sum + row.size, 0),
            }}
            onChange={imp.joinTiff}
          />
        )}
        {images.length === 0 && (
          <p className={helpTextClassName}>
            {state.treated
              ? t("detect.send.nothing")
              : t("import.send.untreated")}
          </p>
        )}
        <SummaryImages
          rows={images}
          ticked={state.ticked}
          room={room}
          pipelineName={pipelineName}
          onToggle={imp.toggleImage}
        />
        {room && (
          <div className="as-result__why">
            <span className={helpTextClassName}>
              {t("detect.send.whyCount", { quota: room.quota })}
            </span>
            <button
              type="button"
              className={buttonClassName("link")}
              onClick={offers}
            >
              <Icon name="external" className="as-button__icon" />
              {t("detect.offersWeb")}
            </button>
          </div>
        )}
      </section>
      {/* The offer refused the stacking: one sentence for every target, in
          amber -- not an error of the person nor of the application. */}
      <Notice tone="warning">
        {state.treatRefused > 0
          ? pluralText(
              t,
              locale,
              "errors.stacking.not_in_subscription",
              state.treatRefused,
            )
          : null}
      </Notice>
      <Notice tone="error">
        {[...state.treatErrors, state.error].filter(Boolean).join(" ")}
      </Notice>
      <WizardFoot
        message={
          <p className={helpTextClassName} role="status">
            {(state.working || state.error
              ? recapProgress(state, plan, t)
              : null) ?? recapLine(plan, t, locale)}
          </p>
        }
        previous={rule.previous !== null ? () => imp.goTo(3) : null}
        secondary={
          rule.secondary && {
            label: secondaryLabel(t, rule.secondary),
            onClick: imp.finish,
            className: "as-treat__secondary",
          }
        }
        primary={
          rule.primary && {
            label:
              rule.primary.kind === "send"
                ? pluralText(t, locale, "detect.send.send", state.ticked.length)
                : plainPrimaryLabel(
                    t,
                    rule.primary.kind === "createSend" ||
                      rule.primary.kind === "createOnly"
                      ? rule.primary.kind
                      : "finish",
                  ),
            onClick:
              rule.primary.kind === "send" || rule.primary.kind === "createSend"
                ? () =>
                    imp.send(
                      (row) =>
                        summaryImageView(t, locale, row, pipelineName).render,
                    )
                : imp.finish,
            enabled: rule.primary.enabled,
            icon:
              rule.primary.kind === "send" || rule.primary.kind === "createSend"
                ? "upload"
                : undefined,
            className: "as-treat__primary",
          }
        }
      />
      {organising && (
        <OrganiseDialog
          proposal={proposal}
          sessionIds={created.outcome.sessions.map((s) => s.detectedId)}
          names={names}
          choice={state.tidy}
          onKeep={(choice) => {
            imp.chooseTidy(choice);
            setOrganising(false);
          }}
          onClose={() => setOrganising(false)}
        />
      )}
    </>
  );
}

/** Where the poses are: the folder the first frames share. */
function commonFolder(proposal: Proposal): string {
  const file = proposal.sessions[0]?.targets[0]?.files[0];
  if (!file) return "";
  const parts = file.replace(/\\/g, "/").split("/");
  return parts.slice(0, Math.max(1, parts.length - 3)).join("/");
}
