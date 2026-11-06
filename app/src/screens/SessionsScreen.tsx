import { useCallback, useEffect, useRef, useState } from "react";

import { Icon } from "../components/Icon";
import { Waiting } from "../components/Waiting";
import { InfoLine } from "../components/InfoLine";
import { Notice } from "../components/Notice";
import { PageTop } from "../components/PageTop";
import { useI18n } from "../i18n";
import {
  isCommandError,
  listSessions,
  multiNightTags,
  nightCreate,
  nightDetail,
  nightOnline,
  nightView,
  workingFolderGet,
  type CapabilitiesState,
  type SessionSummary,
  type WizardStep,
  type PlanTag,
} from "../lib/bridge";
import { displayPath, homeOf } from "../lib/home";
import { loadImportClock, withImportClock } from "../lib/importClock";
import { windowStorage } from "../lib/lastReport";
import {
  createFormReady,
  detectionInput,
  isOffline,
  knownPlaces,
  loadSnapshot,
  loadUnticked,
  newestFirst,
  nightRow,
  offlineWhen,
  saveSnapshot,
  type CreateForm,
  type NightRow,
  type NightsSnapshot,
} from "../lib/nights";
import { formatError } from "../lib/format";
import { readNights } from "../lib/nightsLoad";
import {
  buttonClassName,
  fieldClassName,
  fieldLabelClassName,
  helpTextClassName,
  inactiveButtonClassName,
} from "../theme";
import { ImportCard } from "./detect/ImportCard";
import { NightRowButton } from "./NightRowButton";

/** The nights whose row is read in full (folder, online state): the most
 * recent ones; older rows keep their date and place. */
const READ_IN_FULL = 12;

/**
 * « Nuits »: one clickable row per night of the space -- its date and hours,
 * its place and targets, what is on this computer, what is online, its state
 * --, « Créer une nuit » and its window, the empty and offline states.
 * Clicking a row opens the night's wizard at the step where something is
 * missing.
 */
export function SessionsScreen({
  onOpenNight,
  onImport,
}: {
  capabilitiesState: CapabilitiesState | null;
  onOpenNight: (
    sessionId: string,
    step: WizardStep | null,
    targetId?: string | null,
  ) => void;
  /** « Détecter les sessions », « Reprendre »: the import's page, the
   * one already open when there is one. */
  onImport: () => void;
}) {
  const { t, locale } = useI18n();
  const [rows, setRows] = useState<NightRow[] | null>(null);
  const [steps, setSteps] = useState<Record<string, WizardStep | null>>({});
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notPaired, setNotPaired] = useState(false);
  const [offline, setOffline] = useState<NightsSnapshot | null>(null);
  const [root, setRoot] = useState<string | null>(null);
  const [places, setPlaces] = useState<string[]>([]);
  const [creating, setCreating] = useState(false);
  const generation = useRef(0);

  const refresh = useCallback(async () => {
    const run = ++generation.current;
    setLoading(true);
    setError(null);
    setNotPaired(false);
    setOffline(null);
    workingFolderGet()
      .then((state) => setRoot(state.root))
      .catch(() => setRoot(null));
    let sessions: SessionSummary[];
    try {
      sessions = newestFirst((await listSessions()).sessions);
    } catch (caught) {
      setLoading(false);
      if (
        isCommandError(caught) &&
        (caught.code === "not_paired" || caught.code === "portal_url_missing")
      ) {
        setNotPaired(true);
        setRows(null);
      } else if (isCommandError(caught) && isOffline(caught.code)) {
        const snapshot = loadSnapshot(windowStorage());
        setOffline(snapshot ?? { savedAt: "", rows: [] });
        setRows(snapshot?.rows ?? []);
      } else {
        setError(formatError(t, caught));
      }
      return;
    }
    // The tags of the multi-night plans, read alongside; a failed read
    // tags nothing.
    let tags: Record<string, PlanTag[]> = {};
    const tagsRead = multiNightTags()
      .then((found) => {
        tags = found;
      })
      .catch(() => undefined);
    // Every row says it is being read until its read is back.
    const current: NightRow[] = sessions.map((session) =>
      nightRow(t, locale, { session, reading: true }),
    );
    setRows(current);
    setLoading(false);
    // The nights are read together, the most recent first, under two limits
    // (the service, the folders on disk); each row fills in when its own
    // read is back, and keeps its place in the list.
    const reads = await readNights({
      sessions,
      readInFull: READ_IN_FULL,
      isCurrent: () => run === generation.current,
      readers: {
        detail: nightDetail,
        online: nightOnline,
        view: (session, online) =>
          nightView(
            session.id,
            // A session whose clock the import corrected is read on the
            // clock of its files.
            withImportClock(
              detectionInput(
                session,
                online,
                loadUnticked(windowStorage(), session.id),
              ),
              loadImportClock(windowStorage(), session.id),
            ),
          ),
      },
      onRead: async ({ index, session, detail, view }) => {
        await tagsRead;
        if (view) {
          setSteps((previous) => ({
            ...previous,
            [session.id]: view.resume.step ?? null,
          }));
        }
        current[index] = nightRow(t, locale, {
          session,
          detail,
          view,
          tags: tags[session.id],
        });
        setRows([...current]);
      },
    });
    if (!reads) return;
    setPlaces(knownPlaces(reads.map((read) => read.detail)));
    saveSnapshot(windowStorage(), {
      savedAt: new Date().toISOString(),
      rows: current,
    });
  }, [t, locale]);

  useEffect(() => {
    void refresh();
    return () => {
      generation.current += 1;
    };
  }, [refresh]);

  const rootLine = root ? displayPath(root, homeOf(root)) : "";
  const empty = !loading && !notPaired && !offline && rows?.length === 0;
  const serviceDown = offline !== null;

  return (
    <div className="as-page as-page--list">
      <PageTop>
        <div className="as-page__head">
          <div>
            <h1 className="as-page__title">{t("sessions.title")}</h1>
            <p className={helpTextClassName}>{t("sessions.description")}</p>
          </div>
          <div className="as-page__actions">
            <button
              type="button"
              className={buttonClassName("tertiary")}
              onClick={() => void refresh()}
              disabled={loading || serviceDown}
            >
              {t("common.refresh")}
            </button>
            {!empty && (
              <button
                type="button"
                className={`${buttonClassName("tertiary")} as-detect-open`}
                disabled={serviceDown || notPaired}
                onClick={onImport}
              >
                <Icon name="folder" className="as-button__icon" />
                {t("sessions.detect")}
              </button>
            )}
            {!empty && (
              <button
                type="button"
                className={buttonClassName("secondary")}
                disabled={serviceDown || notPaired}
                onClick={() => setCreating(true)}
              >
                <Icon name="plus" className="as-button__icon" />
                {t("nights.create")}
              </button>
            )}
          </div>
        </div>
      </PageTop>
      <InfoLine
        line={t("nights.workingFolder", { path: rootLine })}
        details={<p>{t("nights.workingFolderDetails")}</p>}
      />
      {/* An import running or waiting is always within reach:
            « Reprendre ». */}
      <ImportCard onResume={onImport} />

      <Notice tone="error">{error}</Notice>
      {notPaired && (
        <Notice tone="warning">{t("sessions.notPairedNotice")}</Notice>
      )}
      {offline && (
        <div className="as-notice as-notice--neutral as-notice--inline">
          <span>
            {offline.savedAt
              ? t("nights.offline", {
                  date: offlineWhen(t, locale, offline.savedAt),
                })
              : t("nights.offlineNever")}
          </span>
          <button
            type="button"
            className={buttonClassName("link")}
            onClick={() => void refresh()}
          >
            {t("nights.retry")}
          </button>
        </div>
      )}
      {loading && rows === null && <Waiting label={t("common.wait.nights")} />}

      {empty && (
        <div className="as-empty">
          <svg
            className="as-empty__star"
            viewBox="0 0 100 100"
            aria-hidden="true"
          >
            <path d="M50 2 56 40 94 26 62 50 94 74 56 60 50 98 44 60 6 74 38 50 6 26 44 40Z" />
          </svg>
          <p>{t("nights.empty")}</p>
          <p className={helpTextClassName}>{t("nights.emptyDetect")}</p>
          <div className="as-actions as-empty__actions">
            <button
              type="button"
              className={buttonClassName("primary")}
              disabled={serviceDown || notPaired}
              onClick={onImport}
            >
              <Icon name="folder" className="as-button__icon" />
              {t("sessions.detect")}
            </button>
            <button
              type="button"
              className={buttonClassName("secondary")}
              disabled={serviceDown || notPaired}
              onClick={() => setCreating(true)}
            >
              <Icon name="plus" className="as-button__icon" />
              {t("nights.create")}
            </button>
          </div>
        </div>
      )}

      {rows && rows.length > 0 && (
        <div
          className="as-nights"
          role="table"
          aria-label={t("sessions.title")}
        >
          <div className="as-nights__head" role="row">
            <span role="columnheader">{t("nights.column.night")}</span>
            <span role="columnheader">{t("nights.column.targets")}</span>
            <span role="columnheader">{t("nights.column.local")}</span>
            <span role="columnheader">{t("nights.column.online")}</span>
            <span role="columnheader">{t("nights.column.state")}</span>
          </div>
          <div className="as-nights__rows">
            {rows.map((row) => (
              <NightRowButton
                key={row.sessionId}
                row={row}
                serviceDown={serviceDown}
                onOpen={() =>
                  onOpenNight(row.sessionId, steps[row.sessionId] ?? null)
                }
              />
            ))}
          </div>
        </div>
      )}

      {creating && (
        <CreateNightDialog
          places={places}
          rootLine={rootLine}
          onCancel={() => setCreating(false)}
          onCreated={(sessionId) => {
            setCreating(false);
            onOpenNight(sessionId, "cibles");
          }}
        />
      )}
    </div>
  );
}

/** « Créer une nuit »: the form, word for word. Nothing is written on disk: the
 * folder comes at the « Dossier » step. */
function CreateNightDialog({
  places,
  rootLine,
  onCancel,
  onCreated,
}: {
  places: string[];
  rootLine: string;
  onCancel: () => void;
  onCreated: (sessionId: string) => void;
}) {
  const { t } = useI18n();
  const [form, setForm] = useState<CreateForm>({
    date: new Date().toISOString().slice(0, 10),
    place: places[0] ?? "",
    start: "",
    end: "",
    title: "",
  });
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const set = (field: keyof CreateForm) => (value: string) =>
    setForm((current) => ({ ...current, [field]: value }));

  async function submit() {
    setBusy(true);
    setError(null);
    try {
      const detail = await nightCreate({
        eveningDate: form.date,
        start: form.start.trim() || null,
        end: form.end.trim() || null,
        place: form.place.trim(),
        title: form.title.trim() || null,
      });
      onCreated(detail.session.id);
    } catch (caught) {
      setError(formatError(t, caught));
      setBusy(false);
    }
  }

  return (
    <div className="as-dialog__scrim" role="presentation">
      <div
        className="as-dialog"
        role="dialog"
        aria-modal="true"
        aria-labelledby="create-night-title"
      >
        <h2 id="create-night-title" className="as-dialog__title">
          {t("nights.create")}
        </h2>
        <p className={helpTextClassName}>{t("nights.createForm.lead")}</p>
        <div className="as-dialog__grid">
          <label className="as-dialog__field">
            <span className={fieldLabelClassName}>
              {t("nights.createForm.date")}
            </span>
            <input
              className={fieldClassName}
              type="date"
              value={form.date}
              onChange={(event) => set("date")(event.target.value)}
            />
          </label>
          <label className="as-dialog__field">
            <span className={fieldLabelClassName}>
              {t("nights.createForm.place")}
            </span>
            <input
              className={fieldClassName}
              list="create-night-places"
              value={form.place}
              onChange={(event) => set("place")(event.target.value)}
            />
            <datalist id="create-night-places">
              {places.map((place) => (
                <option key={place} value={place} />
              ))}
            </datalist>
            <span className={helpTextClassName}>
              {t("nights.createForm.placeHelp")}
            </span>
          </label>
        </div>
        <div className="as-dialog__grid as-dialog__grid--three">
          <label className="as-dialog__field">
            <span className={fieldLabelClassName}>
              {t("nights.createForm.start")}
            </span>
            <input
              className={fieldClassName}
              placeholder="21:00"
              value={form.start}
              onChange={(event) => set("start")(event.target.value)}
            />
          </label>
          <label className="as-dialog__field">
            <span className={fieldLabelClassName}>
              {t("nights.createForm.end")}
            </span>
            <input
              className={fieldClassName}
              placeholder="04:00"
              value={form.end}
              onChange={(event) => set("end")(event.target.value)}
            />
          </label>
          <label className="as-dialog__field">
            <span className={fieldLabelClassName}>
              {t("nights.createForm.name")}
            </span>
            <input
              className={fieldClassName}
              placeholder={t("nights.createForm.namePlaceholder")}
              value={form.title}
              onChange={(event) => set("title")(event.target.value)}
            />
          </label>
        </div>
        <p className={helpTextClassName}>{t("nights.createForm.utc")}</p>
        <InfoLine
          line={t("nights.createForm.folderLater", { path: rootLine })}
          details={<p>{t("nights.createForm.folderLaterDetails")}</p>}
        />
        <Notice tone="error">{error}</Notice>
        <div className="as-actions">
          <button
            type="button"
            className={buttonClassName("link")}
            onClick={onCancel}
          >
            {t("common.cancel")}
          </button>
          {createFormReady(form) && !busy ? (
            <button
              type="button"
              className={buttonClassName("primary")}
              onClick={() => void submit()}
            >
              {t("nights.createForm.submit")}
            </button>
          ) : (
            <button type="button" className={inactiveButtonClassName} disabled>
              {t("nights.createForm.submit")}
            </button>
          )}
        </div>
      </div>
    </div>
  );
}
