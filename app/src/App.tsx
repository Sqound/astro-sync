import { useCallback, useEffect, useState } from "react";

import {
  I18nProvider,
  SUPPORTED_LOCALES,
  detectSystemLocale,
  useI18n,
  type Locale,
} from "./i18n";
import {
  onSpaceEvent,
  settingsGet,
  settingsSet,
  workingFolderGet,
  workingFolderPrepare,
} from "./lib/bridge";
import { pluralText } from "./lib/written";
import { BuildVersion } from "./components/BuildVersion";
import {
  CapabilitiesProvider,
  useCapabilities,
} from "./components/CapabilitiesProvider";
import { ServiceWaitBanner } from "./components/ServiceWaitBanner";
import { SnackbarProvider } from "./components/Snackbar";
import { ImportProvider, useImport } from "./components/ImportProvider";
import { UploadsProvider, useUploads } from "./components/UploadsProvider";
import { useAssistantFileNotice } from "./components/useAssistantFileNotice";
import {
  ensureAssistantFile,
  shouldPrepareAtStartup,
} from "./lib/promptWindow";
import {
  MENU_GROUPS,
  MENU_TOP,
  groupEntries,
  menuEntryOf,
  type MenuEntry,
  type ScreenId,
} from "./lib/menu";
import { pendingCount } from "./lib/uploads";
import type { WizardStep } from "./lib/bridge";
import {
  AnnotationsScreen,
  DryRunScreen,
  GuideScreen,
  HomeScreen,
  LogsScreen,
  NightWizard,
  ProgressionScreen,
  ImportScreen,
  QueueScreen,
  SessionsScreen,
  SettingsScreen,
  SpaceScreen,
} from "./screens";
import {
  applyTheme,
  appShellClassName,
  contentClassName,
  sidebarClassName,
  tabClassName,
  tabGroupClassName,
  tabListClassName,
  tabSectionTitleClassName,
} from "./theme";

/** The night whose wizard is open, and the step it opens at (« Nuits »
 * chooses or creates the night; « À reprendre » opens the missing step). */
interface OpenNight {
  sessionId: string;
  step: WizardStep | null;
  /** The target whose card the step scrolls to (« Mesurer les poses »). */
  targetId: string | null;
}

function isSupportedLocale(value: string | null): value is Locale {
  return (
    value !== null && (SUPPORTED_LOCALES as readonly string[]).includes(value)
  );
}

function AppShell() {
  const { t, locale } = useI18n();
  const [screen, setScreen] = useState<ScreenId>("home");
  const [night, setNight] = useState<OpenNight | null>(null);
  // The uploads of the window: the count on the « Envois » entry, and the
  // notifications' way to the page.
  const uploads = useUploads();
  const pending = pendingCount(uploads.state);
  const { registerOpener } = uploads;
  // The import of sessions: it outlives its page; the menu shows « Import
  // en cours » while it is alive.
  const imp = useImport();
  const presence = imp.presence;
  const openImport = useCallback(() => {
    imp.open();
    setScreen("import");
    // `open` is stable; the rest of `imp` is not needed here.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [imp.open]);
  useEffect(() => registerOpener(() => setScreen("queue")), [registerOpener]);
  const openNight = useCallback(
    (
      sessionId: string,
      step: WizardStep | null,
      targetId: string | null = null,
    ) => {
      setNight({ sessionId, step, targetId });
      setScreen("night");
    },
    [],
  );
  // What the paired service grants (`analysis.logs`, `analysis.progression`,
  // the storage limits) and says of the billing of the space (a payment that
  // failed): every screen is always there; « Progression » and « Journaux »
  // read it to say their reserve themselves. `null` (not yet fetched, or
  // unpaired) holds nothing back. Read once by `CapabilitiesProvider`: at
  // startup and whenever the connection changes (signed in, detached, paired)
  // -- `gui_bridge::refresh_capabilities` tries the network, falls back to
  // the local cache offline, and never blocks the UI.
  const {
    state: capabilitiesState,
    setState: setCapabilitiesState,
    refresh: refreshCapabilities,
  } = useCapabilities();
  // Bumped when the service ends a sign-in: the « Espace Astro Portal »
  // screen starts over from what is now on disk.
  const [spaceVersion, setSpaceVersion] = useState(0);

  // `ASSISTANT-IA.md` is kept up to date without a gesture: made ready at
  // startup once the working folder is known, and again once the person is
  // connected (the first connected launch creates the folder). A file the
  // person changed is kept, and said.
  const noticeAssistantFile = useAssistantFileNotice();
  const paired = capabilitiesState?.paired === true;
  useEffect(() => {
    let stopped = false;
    (async () => {
      const folder = await workingFolderGet().catch(() => null);
      if (stopped || !folder || !shouldPrepareAtStartup(folder, paired)) return;
      const state = await ensureAssistantFile(() =>
        workingFolderPrepare(locale),
      );
      if (!stopped) noticeAssistantFile(state);
    })();
    return () => {
      stopped = true;
    };
  }, [paired, locale, noticeAssistantFile]);

  // The service says this computer is no longer attached to its space
  // (detached from elsewhere, or the space deleted): the core has already
  // forgotten the token and kept the reason. Back to « Accueil », whose
  // sign-in card says why -- the folders associated with nights and the
  // pending uploads are untouched.
  useEffect(() => {
    let stopped = false;
    let unlisten: (() => void) | undefined;
    onSpaceEvent((event) => {
      if (event.kind === "signed_out") {
        setCapabilitiesState({
          paired: false,
          capabilities: null,
          standing: null,
        });
        setSpaceVersion((version) => version + 1);
        setScreen("home");
      }
    })
      .then((stop) => {
        if (stopped) {
          stop();
        } else {
          unlisten = stop;
        }
      })
      .catch(() => {
        // No event channel (a plain browser preview): nothing to react to.
      });
    return () => {
      stopped = true;
      unlisten?.();
    };
  }, [setCapabilitiesState]);

  const renderEntry = (tab: MenuEntry, nested = false) => {
    const active = menuEntryOf(screen, presence.shown) === tab.id;
    return (
      <li key={tab.id}>
        <button
          type="button"
          data-menu={tab.id}
          aria-current={active ? "page" : undefined}
          className={tabClassName(active, nested)}
          onClick={() => setScreen(tab.id)}
        >
          <span
            className={
              tab.id === "import" && !presence.done
                ? "as-tab__label as-tab__label--stacked"
                : "as-tab__label"
            }
          >
            {t(tab.labelKey)}
          </span>
          {tab.id === "queue" && pending > 0 && (
            <span
              className="as-tab__badge"
              role="status"
              aria-label={pluralText(t, locale, "uploads.badge", pending)}
            >
              {pending}
            </span>
          )}
          {tab.id === "import" &&
            !presence.done &&
            !(presence.job === "scan" && presence.percent === null) && (
              <span
                className="as-tab__badge as-tab__badge--import"
                role="status"
              >
                {presence.waiting
                  ? t("import.badge.waiting")
                  : presence.percent !== null
                    ? t("wizard.sending.percent", { percent: presence.percent })
                    : "…"}
              </span>
            )}
        </button>
      </li>
    );
  };

  return (
    <div className={appShellClassName}>
      <nav className={sidebarClassName} aria-label={t("app.title")}>
        <strong>{t("app.title")}</strong>
        <div className={tabListClassName}>
          <ul className={tabGroupClassName}>
            {MENU_TOP.map((entry) => renderEntry(entry))}
          </ul>
          {MENU_GROUPS.map((group) => (
            <div key={group.section} className={tabGroupClassName}>
              <p
                id={`as-menu-${group.section}`}
                className={tabSectionTitleClassName}
              >
                {t(group.titleKey)}
              </p>
              <ul aria-labelledby={`as-menu-${group.section}`}>
                {groupEntries(group, presence.shown).map((entry) =>
                  renderEntry(entry, true),
                )}
              </ul>
            </div>
          ))}
        </div>
        <BuildVersion />
      </nav>
      <main className={contentClassName}>
        {/* Whatever the screen, a call waiting on a slow service says
            so in a snackbar and can be cancelled. */}
        <ServiceWaitBanner />
        {screen === "home" && (
          <HomeScreen
            key={spaceVersion}
            capabilitiesState={capabilitiesState}
            onConnectionChanged={refreshCapabilities}
            onOpen={setScreen}
            onOpenNight={openNight}
          />
        )}
        {screen === "space" && (
          <SpaceScreen
            key={spaceVersion}
            capabilitiesState={capabilitiesState}
            onConnectionChanged={refreshCapabilities}
          />
        )}
        {screen === "sessions" && (
          <SessionsScreen
            capabilitiesState={capabilitiesState}
            onOpenNight={openNight}
            onImport={openImport}
          />
        )}
        {screen === "import" && (
          <ImportScreen
            onLeave={() => setScreen("sessions")}
            onOpenNight={openNight}
            onOpen={setScreen}
          />
        )}
        {screen === "night" && night && (
          <NightWizard
            key={night.sessionId}
            sessionId={night.sessionId}
            initialStep={night.step}
            initialTarget={night.targetId}
            capabilitiesState={capabilitiesState}
            onBack={() => setScreen("sessions")}
            onOpen={setScreen}
          />
        )}
        {screen === "logs" && (
          <LogsScreen capabilitiesState={capabilitiesState} />
        )}
        {screen === "annotations" && <AnnotationsScreen onOpen={setScreen} />}
        {screen === "guide" && <GuideScreen />}
        {screen === "dryRun" && <DryRunScreen />}
        {screen === "queue" && <QueueScreen />}
        {screen === "progression" && (
          <ProgressionScreen
            capabilitiesState={capabilitiesState}
            onOpen={setScreen}
          />
        )}
        {(screen === "settings" || screen === "dependencies") && (
          <SettingsScreen
            key={screen}
            initialTab={screen === "dependencies" ? "dependencies" : "general"}
            onConnectionChanged={refreshCapabilities}
          />
        )}
      </main>
    </div>
  );
}

/**
 * Resolves the locale to boot with (a saved override, or the system's own
 * language) once, then persists any later change back through
 * `settings_set` -- always re-reading current settings first, so a locale
 * change never clobbers a portal URL/device label saved moments earlier by
 * the Settings screen's own, independent save action.
 */
export default function App() {
  const [bootLocale, setBootLocale] = useState<Locale | null>(null);

  useEffect(() => {
    applyTheme(document.documentElement);
  }, []);

  useEffect(() => {
    let cancelled = false;
    settingsGet()
      .then((settings) => {
        if (!cancelled) {
          setBootLocale(
            isSupportedLocale(settings.locale)
              ? settings.locale
              : detectSystemLocale(),
          );
        }
      })
      .catch(() => {
        if (!cancelled) {
          setBootLocale(detectSystemLocale());
        }
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const handleLocaleChange = useCallback((locale: Locale) => {
    void (async () => {
      try {
        const current = await settingsGet();
        await settingsSet({ ...current, locale });
      } catch {
        // The language switch itself already took effect in the UI; a
        // failure to persist it is not worth blocking on here, and the
        // next screen that saves settings will carry the current locale
        // forward regardless.
      }
    })();
  }, []);

  if (bootLocale === null) {
    // Deliberately untranslated: the locale is not known yet, and this
    // resolves within a single local IPC round-trip.
    return null;
  }

  return (
    <I18nProvider locale={bootLocale} onLocaleChange={handleLocaleChange}>
      <SnackbarProvider>
        <CapabilitiesProvider>
          <UploadsProvider>
            <ImportProvider>
              <AppShell />
            </ImportProvider>
          </UploadsProvider>
        </CapabilitiesProvider>
      </SnackbarProvider>
    </I18nProvider>
  );
}
