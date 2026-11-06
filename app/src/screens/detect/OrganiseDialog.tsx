import { useEffect, useMemo, useRef, useState } from "react";

import { FileModeChoice } from "../../components/FileModeChoice";
import { Icon } from "../../components/Icon";
import { Notice } from "../../components/Notice";
import { useI18n } from "../../i18n";
import {
  chooseFolder,
  sessionsOrganisePlan,
  type NightNaming,
  workingFolderGet,
  type OrganiseMode,
  type OrganisePlan,
  type Proposal,
} from "../../lib/bridge";
import { formatBytes, formatError } from "../../lib/format";
import { formatSize } from "../../lib/written";
import {
  emphasise,
  fileName,
  modeSentenceKey,
  modeSentenceTone,
  organiseCounts,
  organiseTreeText,
  reasonKey,
  sourceFolder,
  type TextPart,
} from "../../lib/organise";
import { planToChange, tidyStateOf, type TidyChoice } from "../../lib/tidy";
import { pluralText } from "../../lib/written";
import { buttonClassName, helpTextClassName } from "../../theme";

function Parts({ parts }: { parts: TextPart[] }) {
  return (
    <>
      {parts.map((part, index) =>
        part.code ? (
          <code key={index} className="as-code">
            {part.text}
          </code>
        ) : (
          <span key={index}>{part.text}</span>
        ),
      )}
    </>
  );
}

/**
 * « Voir ce qui sera rangé »: the sheet of the option « Ranger
 * les poses dans le dossier de travail » of « Récapitulatif ». It explains
 * what moves -- the four modes, the summary and the tree of the files that
 * would change, the ones already in place only counted -- and records the
 * choice; nothing is moved here: the step's primary tidies (it is applied when
 * the person goes to the next step). `proposal`: what the detection found;
 * `sessionIds`: the sessions created or updated, whose files are tidied (all
 * when empty); `choice`: the option as it stands; `onKeep`: « Valider » with
 * the mode and the folder; `onClose`: « Annuler » or a dismissal.
 */
export function OrganiseDialog({
  proposal,
  sessionIds,
  names,
  choice,
  onKeep,
  onClose,
}: {
  proposal: Proposal;
  sessionIds: string[];
  /** Each night's folder `AAAA-MM-JJ · lieu · id`, by detected session (the
   * sessions created name them). */
  names?: Record<string, NightNaming>;
  choice: TidyChoice;
  onKeep: (choice: TidyChoice) => void;
  onClose: () => void;
}) {
  const { t, locale } = useI18n();
  const [mode, setMode] = useState<OrganiseMode>(choice.mode);
  const [destination, setDestination] = useState<string | null>(
    choice.destination,
  );
  // The plan of each mode asked, for this destination. « Lien symbolique »
  // is also asked at the start: its answer carries the platform's flag and
  // the numbers of the files (« Laisser en place » plans nothing).
  const [plans, setPlans] = useState<
    Partial<Record<OrganiseMode, OrganisePlan>>
  >({});
  const [windowsNoLink, setWindowsNoLink] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // The folder the person picked; else the working folder (`null` is kept).
  const [picked, setPicked] = useState(choice.destination !== null);
  const sessions = useMemo(
    () => (sessionIds.length > 0 ? sessionIds : null),
    [sessionIds],
  );
  const alive = useRef(true);
  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
    };
  }, []);

  useEffect(() => {
    workingFolderGet()
      .then((state) => alive.current && setDestination(state.root))
      .catch((reason) => alive.current && setError(formatError(t, reason)));
    // Once, at opening.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // The plan of the choice, and the overview once: a new destination or a
  // new mode asks the engine again, nothing is computed here.
  useEffect(() => {
    if (destination === null) return;
    let current = true;
    const wanted: OrganiseMode[] = ["symlink"];
    if (mode !== "leave-in-place" && mode !== "symlink") wanted.push(mode);
    for (const asked of wanted) {
      if (plans[asked]) continue;
      sessionsOrganisePlan(proposal, destination, asked, sessions, names)
        .then((plan) => {
          if (!current) return;
          setPlans((known) => ({ ...known, [asked]: plan }));
          if (asked === "symlink") {
            setWindowsNoLink(plan.windowsNoLink);
            if (plan.windowsNoLink) {
              setMode((chosen) =>
                chosen === "symlink" ? "leave-in-place" : chosen,
              );
            }
          }
        })
        .catch((reason) => current && setError(formatError(t, reason)));
    }
    return () => {
      current = false;
    };
    // `plans` only gates what is asked; the key is the destination and mode.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [destination, mode, proposal, sessions]);

  async function pickFolder() {
    const chosen = await chooseFolder().catch(() => null);
    if (chosen && chosen !== destination) {
      setPlans({});
      setDestination(chosen);
      setPicked(true);
    }
  }

  function pickMode(next: OrganiseMode) {
    setError(null);
    setMode(next);
  }

  // Only what would change is shown; the files already in place are counted.
  const overview = plans.symlink ? planToChange(plans.symlink) : undefined;
  const there = plans.symlink ? tidyStateOf(plans.symlink) : null;
  const inPlace = there?.inPlace ?? 0;
  const rawPlan = mode === "leave-in-place" ? plans.symlink : plans[mode];
  const plan = rawPlan ? planToChange(rawPlan) : undefined;
  const countText = (base: string, count: number) =>
    pluralText(t, locale, base, count, {
      count: new Intl.NumberFormat(locale).format(count),
    });

  function keep() {
    onKeep({
      on: mode !== "leave-in-place",
      mode: mode === "leave-in-place" ? choice.mode : mode,
      destination: picked ? destination : null,
    });
  }

  const counts = overview ? organiseCounts(overview) : null;
  const source = overview ? sourceFolder(overview) : "";
  const summary =
    overview && counts
      ? emphasise(
          t("organise.summary", {
            count: new Intl.NumberFormat(locale).format(overview.files),
            frames: new Intl.NumberFormat(locale).format(counts.frames),
            calibration: new Intl.NumberFormat(locale).format(
              counts.calibration,
            ),
            logs: new Intl.NumberFormat(locale).format(counts.logs),
            size: formatBytes(t, overview.bytes, locale),
            path: source,
          }),
          [source],
        )
      : null;
  const files = plan?.files ?? 0;
  const sentenceTone = modeSentenceTone(mode);
  // « Copie de 1 001 fichiers ; 412 Go libres sur ce disque. »
  const free = plan?.freeBytes;
  const sentenceText =
    mode === "copy" && typeof free === "number"
      ? t("organise.copyLineFree", {
          count: new Intl.NumberFormat(locale).format(files),
          free: formatSize(t, free, locale),
        })
      : t(modeSentenceKey(mode), {
          count: new Intl.NumberFormat(locale).format(files),
          path: source,
        });
  const treeText =
    mode !== "leave-in-place" && plan
      ? organiseTreeText(plan, {
          poses: (count) => countText("organise.tree.poses", count),
          moreNights: (count) => countText("organise.tree.moreNights", count),
        })
      : null;
  const ready = mode === "leave-in-place" || Boolean(plan);

  return (
    <div className="as-dialog__scrim" role="presentation">
      <section
        className="as-dialog as-organise as-organise--wide"
        role="dialog"
        aria-modal="true"
        aria-labelledby="organise-title"
      >
        <h2 id="organise-title" className="as-dialog__title">
          {t("organise.title")}
        </h2>
        {summary && overview && overview.files > 0 && (
          <p className={`${helpTextClassName} as-organise__summary`}>
            <Parts parts={summary} />
          </p>
        )}
        {there && overview && overview.files === 0 && inPlace > 0 && (
          <div className="as-organise__line as-organise__line--success">
            <Icon name="check" className="as-organise__glyph" />
            <span>
              {t("import.tidy.done", {
                how: t(`import.tidy.how.${there.inPlaceAs ?? "file"}`),
              })}
            </span>
          </div>
        )}
        <FileModeChoice
          mode={mode}
          onChange={pickMode}
          windowsNoLink={windowsNoLink}
        />
        {mode !== "leave-in-place" && (
          <>
            <div className="as-organise__to">
              <span className="as-organise__label">{t("organise.to")}</span>
              <div className="as-organise__row">
                <div className="as-field as-organise__folder">
                  <Icon name="folder" className="as-organise__glyph" />
                  <span>{destination ?? ""}</span>
                </div>
                <button
                  type="button"
                  className={buttonClassName("tertiary")}
                  onClick={() => void pickFolder()}
                >
                  {t("detect.chooseFolder")}
                </button>
              </div>
            </div>
            {treeText && (
              <pre className="as-organise__tree" aria-label={t("organise.to")}>
                {treeText}
              </pre>
            )}
            {inPlace > 0 && overview && overview.files > 0 && (
              <p className={helpTextClassName}>
                {t("import.tidy.sheetInPlace", {
                  count: new Intl.NumberFormat(locale).format(inPlace),
                })}
              </p>
            )}
            <p className={helpTextClassName}>
              <Parts
                parts={emphasise(t("organise.treeNote"), [
                  "calibration/",
                  "logs/",
                ])}
              />
            </p>
            {plan && plan.conflicts.length > 0 && (
              <div className="as-organise__conflicts">
                <p className={helpTextClassName}>
                  {countText("organise.conflicts", plan.conflicts.length)}
                </p>
                <ul className="as-organise__failures">
                  {plan.conflicts.map((conflict) => (
                    <li key={`${conflict.source}>${conflict.destination}`}>
                      <code className="as-code">
                        {fileName(conflict.source)}
                      </code>
                      <span className={helpTextClassName}>
                        {t(reasonKey(conflict.reason))}
                      </span>
                    </li>
                  ))}
                </ul>
              </div>
            )}
          </>
        )}
        {ready && (
          <div
            className={`as-organise__line as-organise__line--${sentenceTone}`}
          >
            <Icon
              name={sentenceTone === "warning" ? "warning" : "check"}
              className="as-organise__glyph"
            />
            <span>
              <Parts parts={emphasise(sentenceText, [source])} />
            </span>
          </div>
        )}
        {error && <Notice tone="error">{error}</Notice>}
        <div className="as-organise__foot as-organise__foot--end">
          <button
            type="button"
            className={buttonClassName("link")}
            onClick={onClose}
          >
            {t("common.cancel")}
          </button>
          <button
            type="button"
            className={buttonClassName("primary")}
            disabled={!ready}
            onClick={keep}
          >
            {t("import.tidy.keep")}
          </button>
        </div>
      </section>
    </div>
  );
}
