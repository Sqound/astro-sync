import { useEffect, useRef, useState } from "react";

import { holdTargetCard } from "./holdTargetCard";

import { Icon } from "../../components/Icon";
import { InfoLine } from "../../components/InfoLine";
import { Notice } from "../../components/Notice";
import { PoseStatsCard } from "../../components/PoseStatsCard";
import { usePromptWindow } from "../../components/PromptWindow";
import { Waiting } from "../../components/Waiting";
import { useI18n } from "../../i18n";
import {
  assistantPrompt,
  folderOpen,
  nightPrecheck,
  type DetectionInput,
  type ImagePlan,
  type NightView,
} from "../../lib/bridge";
import { displayPath, homeOf } from "../../lib/home";
import { eveningOfSession, longitudeOf, nightSentence } from "../../lib/nights";
import { posesOf } from "../../lib/poseStats";
import type { PoseJob, PoseStatsControl } from "../../hooks/usePoseStats";
import { promptNames } from "../../lib/promptWindow";
import {
  filesView,
  findingSegs,
  type FileBox,
  type TargetCard,
} from "../../lib/wizardFiles";
import {
  buildScope,
  hasLocation,
  inPlaceTargets,
  opensByItself,
  type DetectAnswer,
  type SendLocations,
} from "../../lib/wizardDetect";
import { DeviceBoxView } from "./DeviceMaster";
import { DetectSheet } from "./DetectSheet";
import { resumeNotice } from "../../lib/wizardSummary";
import { baseName, parentOf } from "../../lib/wizardSteps";
import { targetCardId } from "../../lib/wizardAnchor";
import { badgeClassName, helpTextClassName } from "../../theme";
import type { NightContext } from "./NightWizard";
import { Lines, StepFoot, StepWait } from "./parts";

/**
 * « Fichiers »: what the night's folder holds, read from its folders and its
 * FITS headers -- nothing is moved. Per target, its final image (with its
 * SHA-256 compared to what is online), its frames and what is misfiled; then
 * the calibration, the logs, the files outside the structure.
 */
export function FilesStep({
  context,
  view,
  viewFailed,
  input,
  locations,
  answer,
  onAnswer,
  onAddTarget,
  onRefresh,
  onRetry,
  onMakeFolder,
  poses,
  anchor,
  onBack,
  onContinue,
}: {
  context: NightContext;
  view: NightView | null;
  viewFailed: boolean;
  input: DetectionInput;
  /** Where the session's frames and logs are (empty: the night's folder). */
  locations: SendLocations;
  /** The answer of the detection sheet, until the locations change. */
  answer: DetectAnswer | null;
  onAnswer: (answer: DetectAnswer | null) => void;
  onAddTarget: (targetId: string) => Promise<void>;
  onRefresh: () => void;
  onRetry: () => void;
  /** Makes the night's folder (without a question); its failure is said
   * in a snackbar. */
  onMakeFolder?: () => Promise<void>;
  /** « Mesure des poses »: kept above the steps. */
  poses: PoseStatsControl;
  /** « Mesurer les poses » came here for a target: its card is brought to
   * the top and focused, once (`nonce` tells one request from the next). */
  anchor?: { targetId: string; nonce: number } | null;
  onBack?: () => void;
  onContinue: () => void;
}) {
  const { t, locale } = useI18n();
  const [plans, setPlans] = useState<ImagePlan[]>([]);
  const promptWindow = usePromptWindow();
  // The sheet opens by itself when a location is given; « Revoir la
  // détection » opens it again.
  const [reviewing, setReviewing] = useState(false);

  // What each final image becomes (its SHA-256 against what is online):
  // nothing is sent.
  useEffect(() => {
    let cancelled = false;
    nightPrecheck(context.sessionId, input, [])
      .then((precheck) => {
        if (!cancelled) setPlans(precheck.plans);
      })
      .catch(() => {
        if (!cancelled) setPlans([]);
      });
    return () => {
      cancelled = true;
    };
  }, [context.sessionId, input, view]);

  // The card the person came for (« Mesurer les poses »): once its cards are
  // drawn, it goes to the top of the window and takes the focus; it stays
  // there while the cards above it settle, until the person scrolls.
  const revealed = useRef<number | null>(null);
  const hold = useRef<(() => void) | null>(null);
  const answered = answer !== null;
  useEffect(() => {
    if (!anchor || revealed.current === anchor.nonce || !view?.detection)
      return;
    const card = document.getElementById(targetCardId(anchor.targetId));
    if (!card) return;
    revealed.current = anchor.nonce;
    hold.current?.();
    hold.current = holdTargetCard(card);
  }, [anchor, view, reviewing, answered]);
  useEffect(
    () => () => {
      // Leaving the step stops the hold; the request is open again (a
      // remount in development runs the effect above once more).
      hold.current?.();
      hold.current = null;
      revealed.current = null;
    },
    [],
  );

  // The night reached « Fichiers » without its folder: Astro Sync makes it,
  // once, then reads the night again.
  const made = useRef<"no" | "making" | "done">("no");
  const [making, setMaking] = useState(false);
  useEffect(() => {
    if (!view || view.detection || !onMakeFolder || made.current !== "no")
      return;
    made.current = "making";
    setMaking(true);
    void onMakeFolder().finally(() => {
      made.current = "done";
      setMaking(false);
    });
  }, [view, onMakeFolder]);

  if (!view) {
    return (
      <StepWait
        label={t("wizard.wait.folder")}
        failed={viewFailed}
        onRetry={onRetry}
        onBack={onBack}
      />
    );
  }
  if (!view.detection && (making || made.current === "making")) {
    return <Waiting label={t("wizard.folder.creating")} />;
  }
  if (!view.detection) {
    return (
      <div className="as-step">
        <Notice tone="warning">{t("wizard.files.noFolder")}</Notice>
        <StepFoot
          onBack={onBack}
          primary={t("wizard.files.continue")}
          onPrimary={onContinue}
        />
      </div>
    );
  }
  const detection = view.detection;
  if (
    context.session &&
    (reviewing || opensByItself(locations, answer !== null))
  ) {
    return (
      <DetectSheet
        locations={locations}
        scope={buildScope(context.session, context.online)}
        nightFolder={detection.folder}
        initial={answer?.proposal ?? null}
        onNext={(next) => {
          onAnswer(next);
          setReviewing(false);
          onRefresh();
        }}
        onCancel={() => {
          setReviewing(false);
          onBack?.();
        }}
        onAddTarget={onAddTarget}
      />
    );
  }
  const inPlace =
    answer && !answer.organised ? inPlaceTargets(answer.proposal) : {};
  const added = view.resume.missing.some((m) => m.key === "logsAdded");
  const files = filesView(t, locale, {
    detection,
    online: view.online,
    plans,
    added,
    eveningDay: context.session
      ? nightSentence(
          eveningOfSession(context.session, longitudeOf(context.detail)),
          locale,
        )
      : "",
  });
  const notice = resumeNotice(t, locale, {
    resume: view.resume,
    online: view.online,
    detection,
  });
  const root = context.root ?? parentOf(detection.folder);
  const home = context.root ? homeOf(context.root) : null;

  async function ask(card: TargetCard) {
    const messages = detection.findings
      .filter(
        (finding) =>
          finding.targetId === card.targetId ||
          String(finding.params.from ?? "").startsWith(`${card.folderName}/`),
      )
      .map((finding) =>
        findingSegs(t, locale, finding)
          .map((seg) =>
            typeof seg === "string"
              ? seg
              : "strong" in seg
                ? seg.strong
                : seg.code,
          )
          .join(""),
      );
    // Copied and said in a snackbar, « Voir le prompt » opens it.
    await promptWindow.ask(() =>
      assistantPrompt(locale, {
        root: displayPath(root, home),
        nightFolder: baseName(detection.folder),
        targets: [card.folderName],
        focus: card.folderName,
        messages,
        names: promptNames([card], context.online),
      }),
    );
  }

  return (
    <div className="as-step">
      {notice && (
        <div className="as-notice as-notice--neutral as-notice--resume">
          <Icon name="refresh" className="as-line__glyph" />
          <span>
            <strong>{t("wizard.resume.title")}</strong> {notice}
          </span>
          <InfoLine line="" details={<p>{t("wizard.resume.details")}</p>} />
        </div>
      )}
      <div className="as-step__head-row">
        <div>
          <h2 className="as-step__heading">{t("wizard.files.title")}</h2>
          <p className={helpTextClassName}>{files.summary}</p>
        </div>
        <div className="as-actions">
          {/* « Revoir la détection » beside « Vérifier à
              nouveau ». */}
          {hasLocation(locations) && answer && (
            <button
              type="button"
              className="as-button as-button--link as-files__review"
              onClick={() => setReviewing(true)}
            >
              <Icon name="search" className="as-button__icon" />
              {t("wizard.files.reviewDetection")}
            </button>
          )}
          <button
            type="button"
            className="as-button as-button--link"
            onClick={onRefresh}
          >
            <Icon name="refresh" className="as-button__icon" />
            {t("wizard.files.checkAgain")}
          </button>
        </div>
      </div>
      {promptWindow.window}
      {hasLocation(locations) && answer && (
        <p className={`${helpTextClassName} as-files__read-from`}>
          {t("wizard.files.readFrom", {
            path: displayPath(
              locations.fits ?? detection.folder,
              homeOf(locations.fits ?? detection.folder),
            ),
          })}
        </p>
      )}
      <div className="as-cards">
        {files.targets.map((card) => (
          <section
            key={card.targetId}
            id={targetCardId(card.targetId)}
            className="as-file-card"
            tabIndex={-1}
            aria-labelledby={`${targetCardId(card.targetId)}-title`}
          >
            <header className="as-file-card__head">
              <h3 id={`${targetCardId(card.targetId)}-title`}>
                {card.designation}
              </h3>
              <TargetBadge card={card} />
              {/* The frames read where they are open their folder. */}
              <button
                type="button"
                className="as-button as-button--link as-file-card__open"
                onClick={() =>
                  void folderOpen(
                    inPlace[card.targetId]?.folder ??
                      `${detection.folder}/${card.folderName}`,
                  )
                }
              >
                <Icon name="folder" className="as-button__icon" />
                {inPlace[card.targetId]
                  ? t("wizard.files.openFrames")
                  : t("wizard.files.openTarget", { target: card.folderName })}
              </button>
            </header>
            <FileBoxView
              title={t("wizard.files.box.frames")}
              box={
                inPlace[card.targetId]
                  ? readInPlace(
                      card.frames,
                      `${displayPath(
                        inPlace[card.targetId].folder,
                        homeOf(inPlace[card.targetId].folder),
                      )}/`,
                      t("wizard.files.readInPlace"),
                    )
                  : card.frames
              }
            />
            {card.device && <DeviceBoxView box={card.device} />}
            <FileBoxView title={t("wizard.files.box.final")} box={card.final} />
            {card.lines.length > 0 && <Lines lines={card.lines} />}
            {card.folderName && (
              <TargetPoseStats
                poses={poses}
                job={{
                  targetId: card.targetId,
                  nightFolder: detection.folder,
                  targetFolder: card.folderName,
                  eveningDate: input.eveningDate ?? "",
                  longitude: input.longitude ?? null,
                  latitude:
                    context.detail?.night?.place?.value?.latitude ?? null,
                }}
                placeName={context.detail?.night?.place?.value?.name ?? null}
              />
            )}
            {card.askAssistant && (
              <div className="as-actions as-actions--start">
                <button
                  type="button"
                  className="as-button as-button--tertiary"
                  onClick={() => void ask(card)}
                >
                  <Icon name="sparkle" className="as-button__icon" />
                  {t("wizard.folder.askAssistant")}
                </button>
              </div>
            )}
          </section>
        ))}
        {files.calibration && (
          <section className="as-file-card">
            <header className="as-file-card__head">
              <h3>{t("wizard.files.calibration")}</h3>
            </header>
            <Lines
              lines={files.calibration}
              onAction={(action) =>
                void folderOpen(
                  `${detection.folder}/calibration/${action === "openDarks" ? "darks" : "flats"}`,
                )
              }
            />
          </section>
        )}
        <section className="as-file-card">
          <header className="as-file-card__head">
            <h3>{t("wizard.files.logs")}</h3>
          </header>
          {files.logs.empty ? (
            <>
              <Lines
                lines={[{ glyph: "none", segs: [t("wizard.files.noLogs")] }]}
              />
              <InfoLine
                line={t("wizard.files.noLogsHelp")}
                details={<p>{t("wizard.files.noLogsDetails")}</p>}
              />
            </>
          ) : (
            <Lines lines={files.logs.lines} />
          )}
        </section>
        {files.outside && (
          <section className="as-file-card">
            <header className="as-file-card__head">
              <h3>{t("wizard.files.outside")}</h3>
            </header>
            <Lines lines={[{ glyph: "warning", segs: files.outside }]} />
          </section>
        )}
      </div>
      {files.footer && (
        <InfoLine
          line={files.footer}
          details={<p>{t("wizard.files.footerDetails")}</p>}
        />
      )}
      <StepFoot
        onBack={onBack}
        primary={t("wizard.files.continue")}
        onPrimary={onContinue}
      />
    </div>
  );
}

/** « Mesure des poses » under a target's frames: nothing for a
 * target with no light frame. */
function TargetPoseStats({
  poses,
  job,
  placeName,
}: {
  poses: PoseStatsControl;
  job: PoseJob;
  placeName: string | null;
}) {
  const entry = posesOf(poses.poses, job.targetId);
  const { loadPlan } = poses;
  const { targetId, nightFolder, targetFolder, eveningDate, longitude } = job;
  useEffect(() => {
    void loadPlan({
      targetId,
      nightFolder,
      targetFolder,
      eveningDate,
      longitude,
      latitude: null,
    });
  }, [loadPlan, targetId, nightFolder, targetFolder, eveningDate, longitude]);
  const running = Object.values(poses.poses).some(
    (other) => other.run.status === "running",
  );
  if (entry.plan && entry.plan.families.length === 0) return null;
  if (!entry.plan && entry.run.status === "idle") return null;
  return (
    <PoseStatsCard
      entry={entry}
      place={{
        name: placeName,
        latitude: job.latitude,
        longitude: job.longitude,
      }}
      busyElsewhere={running && entry.run.status !== "running"}
      onStart={(resume) => void poses.start(job, resume)}
      onStop={poses.stop}
    />
  );
}

/** The frames' line of a target read where its frames are:
 * the folder of the frames first, « lues sur place » last. */
function readInPlace(box: FileBox, folder: string, words: string): FileBox {
  const [first, ...rest] = box.lines;
  if (!first) return box;
  const segs = first.segs.map((seg) =>
    typeof seg === "object" && "code" in seg ? { code: folder } : seg,
  );
  return {
    ...box,
    lines: [{ ...first, segs: [...segs, ` · ${words}`] }, ...rest],
  };
}

function FileBoxView({ title, box }: { title: string; box: FileBox }) {
  const { t } = useI18n();
  return (
    <div className="as-file-box">
      <div className="as-file-box__head">
        <h4>{title}</h4>
        <span className={badgeClassName(box.state.tone)}>
          <Icon name={box.state.tone === "success" ? "check" : "cross"} />
          {t(`wizard.files.box.state.${box.state.key}`)}
        </span>
      </div>
      <Lines lines={box.lines} />
    </div>
  );
}

function TargetBadge({ card }: { card: TargetCard }) {
  const { t, locale } = useI18n();
  const badge = card.badge;
  if (badge.tone === "error") {
    return (
      <span className={badgeClassName("error")}>
        <Icon name="cross" />
        {t("wizard.files.badge.empty")}
      </span>
    );
  }
  if (badge.tone === "warning" && badge.key !== "tidy") {
    return (
      <span className={badgeClassName("warning")}>
        {t(`wizard.files.badge.${badge.key}`)}
      </span>
    );
  }
  if (badge.tone === "warning") {
    const one = new Intl.PluralRules(locale).select(badge.count) === "one";
    return (
      <span className={badgeClassName("warning")}>
        {t(`wizard.files.badge.tidy.${one ? "one" : "other"}`, {
          count: badge.count,
        })}
      </span>
    );
  }
  return (
    <span className={badgeClassName("success")}>
      <Icon name={badge.key === "online" && badge.phone ? "phone" : "check"} />
      {t(
        badge.key === "online"
          ? "wizard.files.badge.online"
          : "wizard.files.badge.ready",
      )}
    </span>
  );
}
