import { useCallback, useEffect, useMemo, useState } from "react";

import { BrandMark } from "../components/BrandMark";
import { Waiting } from "../components/Waiting";
import { Icon } from "../components/Icon";
import { InfoLine } from "../components/InfoLine";
import { useSnackbar } from "../components/Snackbar";
import { useI18n } from "../i18n";
import {
  listSessions,
  nightDetail,
  nightFoldersList,
  nightOnline,
  nightView,
  queueStatus,
  spaceStatus,
  workingFolderGet,
  type CapabilitiesState,
  type SpaceStatus,
  type WizardStep,
} from "../lib/bridge";
import {
  displayPath,
  homeOf,
  homeState,
  HOW_STEPS,
  type ResumeRow,
} from "../lib/home";
import { loadImportClock, withImportClock } from "../lib/importClock";
import { windowStorage } from "../lib/lastReport";
import type { ScreenId } from "../lib/menu";
import {
  detectionInput,
  loadUnticked,
  missingText,
  newestFirst,
  eveningOfSession,
  nightSentence,
  placeOf,
  treatmentHold,
} from "../lib/nights";
import { hostOf } from "../lib/space";
import {
  buttonClassName,
  cardAvatarClassName,
  cardBodyClassName,
  cardClassName,
  cardSubClassName,
  cardTitleClassName,
  helpTextClassName,
  homeClassName,
  homeHeaderClassName,
  homeMarkClassName,
  homeTitleClassName,
  inactiveButtonClassName,
  leadClassName,
  rowChevronClassName,
  rowClassName,
  rowIconClassName,
  rowLineClassName,
  rowTextClassName,
  rowTitleClassName,
  sectionTitleClassName,
  badgeClassName,
} from "../theme";
import { SpaceScreen } from "./SpaceScreen";

/** The nights read for « À reprendre »: the most recent ones that have a
 * folder on this computer. */
const RESUME_SCAN = 5;

/**
 * « Accueil »: the connection, the nights to resume, « Comment ça marche »
 * in four steps, and « Commencer », the one primary, which opens « Nuits »
 * -- greyed while signed out. What is shown is `lib/home.ts`; this file
 * draws it.
 */
export function HomeScreen({
  capabilitiesState,
  onConnectionChanged,
  onOpen,
  onOpenNight,
}: {
  capabilitiesState: CapabilitiesState | null;
  /** The connection changed (signed in, detached): the capabilities of the
   * space are to be read again. */
  onConnectionChanged: () => void;
  onOpen: (target: ScreenId) => void;
  onOpenNight: (sessionId: string, step: WizardStep | null) => void;
}) {
  const { t, locale } = useI18n();
  const [status, setStatus] = useState<SpaceStatus | null>(null);
  const [loaded, setLoaded] = useState(false);
  const [created, setCreated] = useState(false);
  const [root, setRoot] = useState<string | null>(null);
  const [sentOnce, setSentOnce] = useState(false);
  const [toResume, setToResume] = useState<ResumeRow[]>([]);
  const [howOpen, setHowOpen] = useState<boolean | null>(null);

  const refreshLocal = useCallback(async () => {
    const [space, folder, queue] = await Promise.allSettled([
      spaceStatus(),
      workingFolderGet(),
      queueStatus(),
    ]);
    if (space.status === "fulfilled") setStatus(space.value);
    if (folder.status === "fulfilled") setRoot(folder.value.root);
    if (queue.status === "fulfilled") {
      setSentOnce(queue.value.summary.completed > 0);
    }
    setLoaded(true);
    return space.status === "fulfilled" ? space.value : null;
  }, []);

  // The nights « à compléter » that have a folder here: read in the
  // background, never in the way of the rest of the home.
  const refreshResume = useCallback(
    async (connected: boolean) => {
      if (!connected) {
        setToResume([]);
        return;
      }
      try {
        const [page, folders] = await Promise.all([
          listSessions(),
          nightFoldersList(),
        ]);
        const here = new Set(folders.map((folder) => folder.nightId));
        const candidates = newestFirst(page.sessions)
          // A session still to debrief (or abandoned) is not offered
          // for the treatment here any more than in « Sessions ».
          .filter(
            (session) =>
              here.has(session.id) && treatmentHold(session) === null,
          )
          .slice(0, RESUME_SCAN);
        const rows: ResumeRow[] = [];
        for (const session of candidates) {
          try {
            const [online, detail] = await Promise.all([
              nightOnline(session.id),
              nightDetail(session.id).catch(() => null),
            ]);
            const unticked = loadUnticked(windowStorage(), session.id);
            const view = await nightView(
              session.id,
              withImportClock(
                detectionInput(session, online, unticked),
                loadImportClock(windowStorage(), session.id),
              ),
            );
            if (view.resume.status !== "to_complete") continue;
            const place = placeOf(detail);
            const names = new Map(
              online.targets
                .filter((target) => target.targetId && target.name)
                .map((target) => [
                  target.targetId as string,
                  target.name as string,
                ]),
            );
            rows.push({
              sessionId: session.id,
              title: [
                t("home.resume.night", {
                  day: nightSentence(eveningOfSession(session), locale),
                }),
                place,
              ]
                .filter(Boolean)
                .join(" · "),
              line: t("home.resume.line", {
                what: missingText(
                  t,
                  locale,
                  view.resume.missing,
                  names,
                  "home",
                ),
              }),
              step: view.resume.step,
            });
          } catch {
            // A night that cannot be read is simply not proposed here.
          }
        }
        setToResume(rows);
      } catch {
        setToResume([]);
      }
    },
    [t, locale],
  );

  useEffect(() => {
    let cancelled = false;
    void refreshLocal().then((space) => {
      if (!cancelled) void refreshResume(!!space?.paired);
    });
    return () => {
      cancelled = true;
    };
  }, [refreshLocal, refreshResume]);

  const connected = !!status?.paired;
  const state = useMemo(
    () => homeState({ connected, toResume, sentOnce }),
    [connected, toResume, sentOnce],
  );
  const showHow = howOpen ?? !state.howFolded;

  async function handleConnectionChanged() {
    onConnectionChanged();
    const space = await refreshLocal();
    void refreshResume(!!space?.paired);
  }

  const who = status?.space
    ? status.space.email
    : status
      ? hostOf(status.serviceUrl)
      : "";
  const rootLine = root ? displayPath(root, homeOf(root)) : null;

  return (
    <div className={homeClassName}>
      <SpaceReady ready={created && connected} />
      <header className={`${homeHeaderClassName} as-page__top`}>
        <h1 className={homeTitleClassName}>
          <BrandMark className={homeMarkClassName} />
          {t("app.title")}
        </h1>
        <p className={leadClassName}>{t("app.tagline")}</p>
      </header>

      {!loaded && <Waiting label={t("common.wait.space")} />}

      {connected && status ? (
        <section
          className={cardClassName("connected")}
          aria-label={t("space.title")}
        >
          <button
            type="button"
            className="as-card__open"
            onClick={() => onOpen("space")}
          >
            <span className={cardAvatarClassName}>
              <Icon name="person" className="as-row__icon" />
            </span>
            <span className={cardBodyClassName}>
              <span className={cardTitleClassName}>{t("space.title")}</span>
              <span className={cardSubClassName}>
                <Icon name="check" strokeWidth={2.4} />
                {t("home.connection.connected", { who })}
              </span>
            </span>
          </button>
          <button
            type="button"
            className={buttonClassName("link")}
            onClick={() => onOpen("settings")}
          >
            {t("nav.settings")}
          </button>
        </section>
      ) : (
        loaded && (
          <SpaceScreen
            variant="card"
            onSignedIn={setCreated}
            capabilitiesState={capabilitiesState}
            onConnectionChanged={() => void handleConnectionChanged()}
          />
        )
      )}

      {state.resume.length > 0 && (
        <section aria-label={t("home.resume.title")} className="as-stack">
          <h2 className={sectionTitleClassName}>{t("home.resume.title")}</h2>
          {state.resume.map((row) => (
            <button
              key={row.sessionId}
              type="button"
              className={rowClassName}
              onClick={() => onOpenNight(row.sessionId, row.step)}
            >
              <Icon name="clock" className={rowIconClassName} />
              <span className={rowTextClassName}>
                <span className={rowTitleClassName}>{row.title}</span>
                <span className={rowLineClassName}>{row.line}</span>
              </span>
              <span className={badgeClassName("neutral")}>
                {t("nights.state.toComplete")}
              </span>
              <Icon name="chevron" className={rowChevronClassName} />
            </button>
          ))}
        </section>
      )}

      <section className="as-how" aria-label={t("home.how.title")}>
        <button
          type="button"
          className="as-how__toggle"
          aria-expanded={showHow}
          onClick={() => setHowOpen(!showHow)}
        >
          <Icon
            name="chevron"
            className={showHow ? "as-how__chevron is-open" : "as-how__chevron"}
          />
          <span className="as-how__title">{t("home.how.title")}</span>
          {!showHow && (
            <span className="as-how__folded">{t("home.how.folded")}</span>
          )}
        </button>
        {showHow && (
          <>
            <ol className="as-how__steps">
              {HOW_STEPS.map((step, index) => (
                <li key={step.titleKey} className="as-how__step">
                  <span className="as-how__step-title">
                    <span className="as-how__number">{index + 1}</span>
                    {t(step.titleKey)}
                  </span>
                  <span className="as-how__step-line">{t(step.lineKey)}</span>
                </li>
              ))}
            </ol>
            <InfoLine
              line={t("home.how.folder", { path: rootLine ?? "" })}
              details={<p>{t("home.how.folderDetails")}</p>}
            />
          </>
        )}
      </section>

      <div className="as-actions">
        {!state.startEnabled && state.startReasonKey && (
          <span className={helpTextClassName}>{t(state.startReasonKey)}</span>
        )}
        <button
          type="button"
          className={buttonClassName("link")}
          onClick={() => onOpen("guide")}
        >
          <Icon name="book" className="as-button__icon" />
          {t("home.readGuide")}
        </button>
        {state.startEnabled ? (
          <button
            type="button"
            className={buttonClassName("primary")}
            onClick={() => onOpen("sessions")}
          >
            {t("space.connected.start")}
          </button>
        ) : (
          <button type="button" className={inactiveButtonClassName} disabled>
            {t("space.connected.start")}
          </button>
        )}
      </div>
    </div>
  );
}

/** « Votre espace est prêt », once, in a snackbar. */
function SpaceReady({ ready }: { ready: boolean }) {
  const { t } = useI18n();
  const { notify } = useSnackbar();
  useEffect(() => {
    if (ready) notify({ tone: "success", text: t("space.connected.ready") });
  }, [ready, notify, t]);
  return null;
}
