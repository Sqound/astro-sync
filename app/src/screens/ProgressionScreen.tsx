import { useEffect, useMemo, useState } from "react";

import { Icon } from "../components/Icon";
import { InfoLine } from "../components/InfoLine";
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
  isCommandError,
  nightDetect,
  nightEnqueue,
  nightFoldersList,
  nightOnline,
  nightPrecheck,
  nightRecordSending,
  progressionFamilies,
  progressionImagesCandidates,
  progressionImagesRun,
  toolsDetect,
  uploadAll,
  type AcquisitionFamily,
  type CapabilitiesState,
  type DetectionInput,
  type ProgressionImages,
  type TargetDetection,
  type ToolRow,
} from "../lib/bridge";
import { formatError } from "../lib/format";
import { detectionInput } from "../lib/nights";
import {
  canGenerate,
  failureText,
  milestones,
  progressionState,
  sirilLine,
  sirilText,
} from "../lib/progressionTab";
import { isReserved, nightChoices, type NightChoice } from "../lib/tabs";
import { refusalText } from "../lib/wizardSummary";
import {
  buttonClassName,
  fieldClassName,
  fieldLabelClassName,
  helpTextClassName,
  inactiveButtonClassName,
} from "../theme";
import type { ScreenId } from "../lib/menu";

/**
 * « Progression »: the message at the head (today's paragraph behind ⓘ), Siril
 * in one line with the link to Réglages › Dépendances, a night and a target of
 * the working folder, the families listed by themselves, the images in Astro
 * Sync's own data, « Envoyer ces images de progression ». Reserved without
 * `analysis.progression` (never an offer read).
 */
export function ProgressionScreen({
  capabilitiesState,
  onOpen,
}: {
  capabilitiesState: CapabilitiesState | null;
  onOpen: (screen: ScreenId) => void;
}) {
  const { t, locale } = useI18n();
  const { web, privateServer } = useSpaceWeb();
  const offers = offersOpener(web, locale);
  const reserved = isReserved(capabilitiesState, "analysis.progression");
  const [tools, setTools] = useState<ToolRow[] | null>(null);
  const [choices, setChoices] = useState<NightChoice[] | null>(null);
  const [nightId, setNightId] = useState<string | null>(null);
  const [input, setInput] = useState<DetectionInput | null>(null);
  const [targets, setTargets] = useState<TargetDetection[] | null>(null);
  const [targetId, setTargetId] = useState<string | null>(null);
  const [families, setFamilies] = useState<AcquisitionFamily[] | null>(null);
  const [familyKey, setFamilyKey] = useState<string | null>(null);
  const [running, setRunning] = useState(false);
  const [images, setImages] = useState<ProgressionImages | null>(null);
  const [showScript, setShowScript] = useState(false);
  const [skipped, setSkipped] = useState(false);
  const [sending, setSending] = useState(false);
  const [sentNotice, setSentNotice] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    toolsDetect()
      .then((rows) => !cancelled && setTools(rows))
      .catch(() => !cancelled && setTools([]));
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

  // The night's targets with frames: their names come from the space, their
  // folders and frames from the night's folder.
  useEffect(() => {
    if (!night) return;
    let cancelled = false;
    setTargets(null);
    setTargetId(null);
    setImages(null);
    setSentNotice(false);
    void (async () => {
      try {
        const online = await nightOnline(night.nightId).catch(() => null);
        const built = detectionInput(
          { startedAt: `${night.eveningDate ?? "1970-01-01"}T18:00:00Z` },
          online,
        );
        const asked = { ...built, eveningDate: night.eveningDate };
        const detection = await nightDetect(night.folder, asked);
        if (cancelled) return;
        const withFrames = detection.targets.filter(
          (target) => target.lightFrames > 0 && target.folderName,
        );
        setInput(asked);
        setTargets(withFrames);
        setTargetId(withFrames[0]?.targetId ?? null);
      } catch (caught) {
        if (!cancelled) {
          setTargets([]);
          setError(formatError(t, caught));
        }
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [night, t]);

  const target = targets?.find((item) => item.targetId === targetId) ?? null;

  useEffect(() => {
    if (!night || !target?.folderName) {
      setFamilies(null);
      return;
    }
    let cancelled = false;
    setFamilies(null);
    progressionFamilies(night.folder, target.folderName)
      .then((found) => {
        if (cancelled) return;
        setFamilies(found);
        setFamilyKey(found[0]?.key ?? null);
      })
      .catch((caught: unknown) => {
        if (!cancelled) {
          setFamilies([]);
          setError(formatError(t, caught));
        }
      });
    return () => {
      cancelled = true;
    };
  }, [night, target, t]);

  const siril = useMemo(() => sirilLine(tools), [tools]);
  const state = progressionState({
    reserved,
    siril,
    generated: images !== null,
  });
  const family = families?.find((item) => item.key === familyKey) ?? null;
  const generate = canGenerate({ state, family: familyKey, running });

  async function handleGenerate() {
    if (!night || !target?.folderName || !family) return;
    setRunning(true);
    setError(null);
    try {
      setImages(
        await progressionImagesRun({
          nightId: night.nightId,
          nightFolder: night.folder,
          targetFolder: target.folderName,
          family,
        }),
      );
      setShowScript(false);
    } catch (caught) {
      setError(
        isCommandError(caught) && caught.code === "progression_failed"
          ? failureText(t, caught.details)
          : formatError(t, caught),
      );
    } finally {
      setRunning(false);
    }
  }

  async function handleSend() {
    if (!night || !target || !images || !input) return;
    setSending(true);
    setError(null);
    try {
      const candidates = await progressionImagesCandidates(
        images,
        target.targetId,
      );
      const checked = await nightPrecheck(
        night.nightId,
        input,
        candidates,
        locale,
      );
      if (!checked.quota.allowed) {
        setError(
          checked.quota.findings
            .map((finding) =>
              refusalText(
                t,
                locale,
                finding,
                () => target.designation,
                privateServer,
              ),
            )
            .join(" "),
        );
        return;
      }
      const plans = checked.plans.filter(
        (plan) => plan.image.kind === "progression",
      );
      const answer = await nightEnqueue(night.nightId, plans, checked.quota);
      if (answer.result === "refused") {
        setError(
          answer.findings
            .map((finding) =>
              refusalText(
                t,
                locale,
                finding,
                () => target.designation,
                privateServer,
              ),
            )
            .join(" "),
        );
        return;
      }
      await uploadAll({
        sessionId: night.nightId,
        planId: null,
        targetIds: [],
      });
      await nightRecordSending(night.nightId, {
        analysisFingerprint: null,
        progressionFrames: { [target.targetId]: target.lightFrames },
      }).catch(() => undefined);
      setSentNotice(true);
    } catch (caught) {
      setError(formatError(t, caught));
    } finally {
      setSending(false);
    }
  }

  const greyed = state === "reserved";
  const script = (images as { script?: string } | null)?.script ?? "";

  return (
    <div className="as-page">
      <PageTop>
        <h1 className="as-page__title">{t("progression.title")}</h1>
      </PageTop>
      <TabHead
        title={t("progression.headTitle")}
        line={t("progression.headLine")}
        note={t("progression.headNote")}
        details={<p>{t("progression.description")}</p>}
      />
      {state === "reserved" && (
        <ReservedLine text={t("progression.reserved")} onOffers={offers} />
      )}
      {state === "sirilMissing" && (
        <div className="as-notice as-notice--warning as-notice--inline">
          <span>{t("progression.sirilMissing")}</span>
          <button
            type="button"
            className={buttonClassName("link")}
            onClick={() => onOpen("dependencies")}
          >
            {t("deps.open")}
          </button>
        </div>
      )}
      {siril && siril.state !== "missing" && !greyed && (
        <p className="as-tool-line">
          <Icon name="check" />
          <span>{sirilText(t, siril)}</span>
          <button
            type="button"
            className={buttonClassName("link")}
            onClick={() => onOpen("dependencies")}
          >
            {t("wizard.treatments.dependencies")}
          </button>
        </p>
      )}
      <Notice tone="error">{error}</Notice>
      {choices !== null && choices.length === 0 && (
        <p className={helpTextClassName}>{t("tabs.noNight")}</p>
      )}

      {state === "generated" && images && target ? (
        <div className="as-form-card">
          <h2 className="as-form-card__title">
            {t("progression.availableTitle")}
          </h2>
          <p className={helpTextClassName}>
            {t("progression.generatedFor", {
              target: target.designation,
              family: familyKey ?? "",
            })}
          </p>
          <ul className="as-milestones">
            {milestones(images).map((milestone) => (
              <li key={milestone.index}>
                <span className="as-milestones__frame" aria-hidden="true">
                  <Icon name="sparkle" />
                </span>
                {t("progression.milestoneLabel", {
                  index: milestone.index,
                  count: milestone.frameCount,
                })}
              </li>
            ))}
          </ul>
          <p>
            <button
              type="button"
              className={buttonClassName("link")}
              onClick={() => setShowScript((current) => !current)}
            >
              {showScript
                ? t("progression.hideScriptButton")
                : t("progression.viewScriptButton")}
            </button>
          </p>
          {showScript && (
            <pre>
              <code>{script}</code>
            </pre>
          )}
          <Notice tone="success">
            {sentNotice ? t("progression.sentNotice") : null}
          </Notice>
          {sending && (
            <p className={helpTextClassName}>
              {t("progression.sendingNotice")}
            </p>
          )}
          <div className="as-actions">
            {sending || sentNotice ? (
              <button
                type="button"
                className={inactiveButtonClassName}
                disabled
              >
                <Icon name="upload" className="as-button__icon" />
                {t("progression.send")}
              </button>
            ) : (
              <button
                type="button"
                className={buttonClassName("primary")}
                onClick={() => void handleSend()}
              >
                <Icon name="upload" className="as-button__icon" />
                {t("progression.send")}
              </button>
            )}
          </div>
        </div>
      ) : skipped ? (
        <Notice tone="neutral">
          {t("progression.skippedNotice")}{" "}
          <button
            type="button"
            className={buttonClassName("link")}
            onClick={() => setSkipped(false)}
          >
            {t("progression.undoSkipButton")}
          </button>
        </Notice>
      ) : (
        (choices?.length ?? 0) > 0 && (
          <div className={greyed ? "as-form-card is-greyed" : "as-form-card"}>
            <h2 className="as-form-card__title">
              {t("progression.formTitle")}
            </h2>
            {!greyed && target?.folderName && (
              <p className={helpTextClassName}>
                {t("progression.readFrom", {
                  path: `${target.folderName}/lights/`,
                })}
              </p>
            )}
            <div className="as-select-grid">
              <NightSelect
                choices={choices ?? []}
                value={nightId}
                disabled={greyed || running}
                onChange={setNightId}
              />
              <label>
                <span className={fieldLabelClassName}>
                  {t("progression.target")}
                </span>
                <select
                  className={fieldClassName}
                  value={targetId ?? ""}
                  disabled={greyed || running || !targets?.length}
                  onChange={(event) => setTargetId(event.target.value)}
                >
                  {(targets ?? []).map((item) => (
                    <option key={item.targetId} value={item.targetId}>
                      {item.designation}
                    </option>
                  ))}
                </select>
              </label>
            </div>
            {targets !== null && targets.length === 0 && !greyed && (
              <p className={helpTextClassName}>{t("progression.noLights")}</p>
            )}
            <label className="as-field-group">
              <span className={fieldLabelClassName}>
                {t("progression.familyLabel")}
              </span>
              <select
                className={fieldClassName}
                value={familyKey ?? ""}
                disabled={greyed || running || !families?.length}
                onChange={(event) => setFamilyKey(event.target.value)}
              >
                {(families ?? []).map((item) => (
                  <option key={item.key} value={item.key}>
                    {t("progression.familyOptionLabel", {
                      key: item.key,
                      count: item.framePaths.length,
                    })}
                  </option>
                ))}
              </select>
            </label>
            {families !== null && families.length === 0 && !greyed && (
              <p className={helpTextClassName}>
                {t("progression.noFamiliesFound")}
              </p>
            )}
            <InfoLine
              line={t("progression.output")}
              details={<p>{t("progression.outputDetails")}</p>}
            />
            {running && (
              <p className={helpTextClassName}>
                {t("progression.generatingNotice")}
              </p>
            )}
            <div className="as-actions">
              <button
                type="button"
                className={buttonClassName("link")}
                disabled={greyed}
                onClick={() => setSkipped(true)}
              >
                {t("progression.skipButton")}
              </button>
              {generate ? (
                <button
                  type="button"
                  className={buttonClassName("primary")}
                  onClick={() => void handleGenerate()}
                >
                  {t("progression.generateButton")}
                </button>
              ) : (
                <button
                  type="button"
                  className={inactiveButtonClassName}
                  disabled
                >
                  {t("progression.generateButton")}
                </button>
              )}
            </div>
          </div>
        )
      )}
    </div>
  );
}
