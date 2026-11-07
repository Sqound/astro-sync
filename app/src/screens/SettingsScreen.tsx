import { useEffect, useState } from "react";

import { Icon } from "../components/Icon";
import { useBuildInfo } from "../components/BuildVersion";
import { InfoLine } from "../components/InfoLine";
import { Notice } from "../components/Notice";
import { useSnackbar } from "../components/Snackbar";
import { PageTop } from "../components/PageTop";
import {
  SUPPORTED_LOCALES,
  detectSystemLocale,
  localeName,
  useI18n,
  type Locale,
} from "../i18n";
import {
  chooseFolder,
  folderOpen,
  localSettingsGet,
  localSettingsSetPlanField,
  localSettingsSetPolishedEngine,
  localSettingsSetRendersUpload,
  localSettingsSetUtcOffsets,
  settingsGet,
  settingsSet,
  spaceStatus,
  workingFolderGet,
  workingFolderSet,
  type DetachOutcome,
  type LocalSettings,
  type PlanField,
  type PolishedEngine,
  type RendersUpload,
  type SpaceStatus,
} from "../lib/bridge";
import { settingsVersionLine } from "../lib/buildInfo";
import { formatError } from "../lib/format";
import { displayPath, homeOf } from "../lib/home";
import {
  OFFSET_FIELDS,
  offsetDraft,
  offsetsSavable,
  readOffset,
  type OffsetDraft,
} from "../lib/settingsTab";
import { DependenciesTab } from "./DependenciesTab";
import { ProcessingFiles } from "./ProcessingFiles";
import { DetachDevice, DeviceName } from "./space/DeviceSettings";
import {
  buttonClassName,
  fieldClassName,
  fieldGroupClassName,
  fieldLabelClassName,
  helpTextClassName,
  inactiveButtonClassName,
} from "../theme";

export type SettingsTab = "general" | "dependencies";

/** « Ouvrir dans le Finder », « l'Explorateur », or « le dossier ». */
function openFolderKey(): string {
  return navigator.userAgent.includes("Windows")
    ? "wizard.folder.openExplorer"
    : navigator.userAgent.includes("Mac")
      ? "wizard.folder.openFinder"
      : "wizard.folder.openFolder";
}

/**
 * « Réglages »: two tabs. « Général » opens on « Dossier de travail » --
 * the only place it changes, never asked in the wizard -- then today's
 * settings: the language, the name of this computer, the clocks of the logs,
 * « Détacher cet ordinateur » in its own zone. The second tab is
 * « Dépendances ».
 */
export function SettingsScreen({
  initialTab = "general",
  onConnectionChanged,
}: {
  initialTab?: SettingsTab;
  /** The connection changed (detached): the capabilities of the account are
   * to be read again. */
  onConnectionChanged: () => void;
}) {
  const { t } = useI18n();
  const [tab, setTab] = useState<SettingsTab>(initialTab);

  return (
    <div className="as-page">
      <PageTop>
        <h1 className="as-page__title">{t("settings.title")}</h1>
        <div className="as-segments" role="tablist">
          {(["general", "dependencies"] as const).map((item) => (
            <button
              key={item}
              type="button"
              role="tab"
              aria-selected={tab === item}
              className={tab === item ? "as-segment is-chosen" : "as-segment"}
              onClick={() => setTab(item)}
            >
              {t(
                item === "general"
                  ? "settings.tab.general"
                  : "settings.tab.dependencies",
              )}
            </button>
          ))}
        </div>
      </PageTop>
      {tab === "general" ? (
        <GeneralTab onConnectionChanged={onConnectionChanged} />
      ) : (
        <DependenciesTab />
      )}
    </div>
  );
}

function GeneralTab({
  onConnectionChanged,
}: {
  onConnectionChanged: () => void;
}) {
  const { t, locale, setLocale } = useI18n();
  const [status, setStatus] = useState<SpaceStatus | null>(null);
  // What passes is said by a snackbar.
  const { notify } = useSnackbar();
  const saySuccess = (text: string) => notify({ tone: "success", text });
  const [storedLocale, setStoredLocale] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [root, setRoot] = useState<string | null>(null);
  const [candidate, setCandidate] = useState<string | null>(null);
  const [local, setLocal] = useState<LocalSettings | null>(null);
  const [draft, setDraft] = useState<OffsetDraft>(offsetDraft(null));
  const buildInfo = useBuildInfo();

  useEffect(() => {
    let cancelled = false;
    settingsGet()
      .then((settings) => !cancelled && setStoredLocale(settings.locale))
      .catch((caught: unknown) => {
        if (!cancelled) setError(formatError(t, caught));
      });
    spaceStatus()
      .then((loaded) => !cancelled && setStatus(loaded))
      .catch((caught: unknown) => {
        if (!cancelled) setError(formatError(t, caught));
      });
    workingFolderGet()
      .then((state) => !cancelled && setRoot(state.root))
      .catch(() => undefined);
    localSettingsGet()
      .then((loaded) => {
        if (cancelled) return;
        setLocal(loaded);
        setDraft(offsetDraft(loaded));
      })
      .catch(() => undefined);
    return () => {
      cancelled = true;
    };
    // Only ever runs once: this tab owns its own fetch/save cycle.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  async function handleLocaleSelect(value: string) {
    if (value === "system") {
      setLocale(detectSystemLocale());
      setStoredLocale(null);
      try {
        const current = await settingsGet();
        await settingsSet({ ...current, locale: null });
      } catch (caught) {
        setError(formatError(t, caught));
      }
      return;
    }
    const next = value as Locale;
    setLocale(next);
    setStoredLocale(next);
  }

  function handleDetached(outcome: DetachOutcome) {
    setStatus(outcome.status);
    // Signed in to a private server by e-mail code: « votre portail ».
    const portal = !outcome.status.hosted;
    saySuccess(
      outcome.serviceNotified
        ? t(portal ? "space.portal.detach.done" : "space.detach.done")
        : t(
            portal
              ? "space.portal.detach.notNotified"
              : "space.detach.notNotified",
          ),
    );
    onConnectionChanged();
  }

  async function handleUnpaired() {
    saySuccess(t("settings.unpairSuccessNotice"));
    try {
      setStatus(await spaceStatus());
    } catch (caught) {
      setError(formatError(t, caught));
    }
    onConnectionChanged();
  }

  async function changeFolder() {
    setError(null);
    try {
      const chosen = await chooseFolder();
      if (chosen) setCandidate(chosen);
    } catch (caught) {
      setError(formatError(t, caught));
    }
  }

  async function useCandidate() {
    if (!candidate) return;
    try {
      const prepared = await workingFolderSet(candidate, locale);
      setRoot(prepared.root);
      saySuccess(
        t("settings.folder.changed", {
          path: displayPath(prepared.root, homeOf(prepared.root)),
        }),
      );
    } catch (caught) {
      setError(formatError(t, caught));
    } finally {
      setCandidate(null);
    }
  }

  async function saveOffsets() {
    const value = (key: keyof OffsetDraft) => {
      const read = readOffset(draft[key]);
      return read.ok ? read.value : null;
    };
    try {
      const saved = await localSettingsSetUtcOffsets({
        ninaUtcOffset: value("ninaUtcOffset"),
        phd2UtcOffset: value("phd2UtcOffset"),
        asiairUtcOffset: value("asiairUtcOffset"),
      });
      setLocal(saved);
      setDraft(offsetDraft(saved));
      saySuccess(t("settings.utc.saved"));
    } catch (caught) {
      setError(formatError(t, caught));
    }
  }

  // The engine of the « Rendu soigné »: kept at once, read by
  // the stacking when it runs.
  async function changeEngine(engine: PolishedEngine) {
    try {
      setLocal(await localSettingsSetPolishedEngine(engine));
    } catch (caught) {
      setError(formatError(t, caught));
    }
  }

  // « Image du plan multi-nuits »: kept at once, read when the image of a
  // multi-night plan is made.
  async function changePlanField(field: PlanField) {
    try {
      setLocal(await localSettingsSetPlanField(field));
    } catch (caught) {
      setError(formatError(t, caught));
    }
  }

  // « Envoi des rendus »: kept at once, the starting value of the box
  // « Joindre aussi les TIFF » of the next sending.
  async function changeRendersUpload(renders: RendersUpload) {
    try {
      setLocal(await localSettingsSetRendersUpload(renders));
    } catch (caught) {
      setError(formatError(t, caught));
    }
  }

  const shown = root ? displayPath(root, homeOf(root)) : "";
  const savable = offsetsSavable(local, draft);

  return (
    <div className="as-form-card as-settings-general">
      <Notice tone="error">{error}</Notice>

      <section className="as-zone">
        <h2 className="as-zone__title">{t("settings.folder.title")}</h2>
        <div className="as-folder-row">
          <span className="as-folder-row__path">
            <Icon name="folder" />
            {shown}
          </span>
          <button
            type="button"
            className={buttonClassName("link")}
            disabled={!root}
            onClick={() =>
              root &&
              void folderOpen(root).catch((caught: unknown) =>
                setError(formatError(t, caught)),
              )
            }
          >
            <Icon name="external" className="as-button__icon" />
            {t(openFolderKey())}
          </button>
          <button
            type="button"
            className={buttonClassName("tertiary")}
            onClick={() => void changeFolder()}
          >
            <Icon name="folder" className="as-button__icon" />
            {t("settings.folder.change")}
          </button>
        </div>
        <InfoLine
          line={t("settings.folder.line")}
          details={<p>{t("wizard.folder.neverMovesDetails")}</p>}
        />
      </section>

      <ProcessingFiles />

      <section className="as-zone">
        <h2 className="as-zone__title">{t("settings.languageLabel")}</h2>
        <div className={fieldGroupClassName}>
          <select
            className={fieldClassName}
            aria-label={t("settings.languageLabel")}
            value={storedLocale ?? "system"}
            onChange={(event) => void handleLocaleSelect(event.target.value)}
          >
            <option value="system">{t("settings.languageSystem")}</option>
            {SUPPORTED_LOCALES.map((item) => (
              <option key={item} value={item}>
                {localeName(item)}
              </option>
            ))}
          </select>
          <p className={helpTextClassName}>{localeName(locale)}</p>
        </div>
      </section>

      <section className="as-zone">
        <h2 className="as-zone__title">{t("settings.polished.title")}</h2>
        <div className={fieldGroupClassName}>
          <select
            className={fieldClassName}
            aria-label={t("settings.polished.title")}
            value={local?.polishedEngine ?? "graxpert-ai"}
            disabled={!local}
            onChange={(event) =>
              void changeEngine(event.target.value as PolishedEngine)
            }
          >
            <option value="graxpert-ai">
              {t("settings.polished.graxpert")}
            </option>
            <option value="siril">{t("settings.polished.siril")}</option>
          </select>
          <p className={helpTextClassName}>{t("settings.polished.line")}</p>
        </div>
      </section>

      <section className="as-zone">
        <h2 className="as-zone__title">{t("settings.rendersUpload.title")}</h2>
        <div className={fieldGroupClassName}>
          <select
            className={fieldClassName}
            aria-label={t("settings.rendersUpload.title")}
            value={local?.rendersUpload ?? "png"}
            disabled={!local}
            onChange={(event) =>
              void changeRendersUpload(event.target.value as RendersUpload)
            }
          >
            <option value="png">{t("settings.rendersUpload.png")}</option>
            <option value="both">{t("settings.rendersUpload.both")}</option>
          </select>
          <p className={helpTextClassName}>
            {t("settings.rendersUpload.line")}
          </p>
        </div>
      </section>

      <section className="as-zone">
        <h2 className="as-zone__title">{t("settings.planField.title")}</h2>
        <div className={fieldGroupClassName}>
          <select
            className={fieldClassName}
            aria-label={t("settings.planField.title")}
            value={local?.planField ?? "clean"}
            disabled={!local}
            onChange={(event) =>
              void changePlanField(event.target.value as PlanField)
            }
          >
            <option value="clean">{t("settings.planField.clean")}</option>
            <option value="common">{t("settings.planField.common")}</option>
            <option value="whole">{t("settings.planField.whole")}</option>
          </select>
          <p className={helpTextClassName}>{t("settings.planField.line")}</p>
        </div>
      </section>

      {status?.paired && status.space && (
        <DeviceName status={status} onStatus={setStatus} />
      )}

      <section className="as-zone">
        <h2 className="as-zone__title">{t("settings.utc.title")}</h2>
        <p className={helpTextClassName}>{t("settings.utc.line")}</p>
        <div className="as-utc-grid">
          {OFFSET_FIELDS.map((field) => (
            <label key={field.key}>
              <span className={fieldLabelClassName}>{t(field.labelKey)}</span>
              <input
                className={fieldClassName}
                value={draft[field.key]}
                placeholder={t("settings.utc.placeholder")}
                aria-invalid={!readOffset(draft[field.key]).ok}
                onChange={(event) =>
                  setDraft((current) => ({
                    ...current,
                    [field.key]: event.target.value,
                  }))
                }
              />
            </label>
          ))}
        </div>
        <div className="as-actions">
          {savable ? (
            <button
              type="button"
              className={buttonClassName("primary")}
              onClick={() => void saveOffsets()}
            >
              {t("settings.utc.save")}
            </button>
          ) : (
            <button type="button" className={inactiveButtonClassName} disabled>
              {t("settings.utc.save")}
            </button>
          )}
        </div>
      </section>

      {status?.paired && (
        <DetachDevice
          status={status}
          onDetached={handleDetached}
          onUnpaired={() => void handleUnpaired()}
        />
      )}

      {buildInfo && (
        <p className={helpTextClassName}>
          {settingsVersionLine(t, buildInfo, locale)}
        </p>
      )}

      {candidate && (
        <div className="as-dialog__scrim" role="presentation">
          <div
            className="as-dialog"
            role="dialog"
            aria-modal="true"
            aria-labelledby="working-folder-title"
          >
            <h2 id="working-folder-title" className="as-dialog__title">
              {t("settings.folder.confirmTitle")}
            </h2>
            <p className="as-folder-row__path">
              {displayPath(candidate, homeOf(candidate))}
            </p>
            <p className={helpTextClassName}>
              {t("settings.folder.confirmBody", { old: shown })}
            </p>
            <div className="as-actions">
              <button
                type="button"
                className={buttonClassName("link")}
                onClick={() => setCandidate(null)}
              >
                {t("common.cancel")}
              </button>
              <button
                type="button"
                className={buttonClassName("primary")}
                onClick={() => void useCandidate()}
              >
                {t("settings.folder.use")}
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
