// « Nuits »: what one row of the list says about a night -- its
// date and hours, its place and targets, what is on this computer, what is
// online, its state and what it misses. Pure TypeScript (no React, no
// Tauri): the words are chosen here and tested under `node --test`;
// `screens/SessionsScreen.tsx` draws them.

import { nightTag } from "./multiNight.ts";
import type { TranslateParams } from "../i18n/locale.ts";
import type {
  DetectionInput,
  Missing,
  NightDetail,
  PlanTag,
  NightDetection,
  NightOnline,
  NightView,
  SessionSummary,
} from "./bridge";
import { pluralText } from "./written.ts";

type Translate = (key: string, params?: TranslateParams) => string;

/** « Jeu. 8 oct. »: the evening's weekday, day and month, capitalised as
 * the first word of a cell. */
export function nightDay(
  instant: string,
  locale: string,
  timeZone?: string,
): string {
  const date = new Date(instant);
  if (Number.isNaN(date.getTime())) return "—";
  const text = new Intl.DateTimeFormat(locale, {
    weekday: "short",
    day: "numeric",
    month: "short",
    ...(timeZone ? { timeZone } : {}),
  }).format(date);
  return text.charAt(0).toLocaleUpperCase(locale) + text.slice(1);
}

/** The date a night is named by, **one per night**: the evening of its start
 * (`AAAA-MM-JJ`) as the service keeps it -- its `eveningDate`, the one of the
 * folder and of « Night of 7 Oct » on the web --, else computed with the same
 * rule from the start and the longitude of the place (UTC noon when none is
 * known). Never the calendar day of the computer's time zone: the same night
 * is not « jeu. 8 oct. » in Tokyo and « mer. 7 oct. » in Paris. */
export function eveningOfSession(
  session: Pick<SessionSummary, "startedAt"> & {
    eveningDate?: string | null;
  },
  longitude: number | null = null,
): string {
  if (session.eveningDate && /^\d{4}-\d{2}-\d{2}/.test(session.eveningDate))
    return session.eveningDate.slice(0, 10);
  return eveningDateAtLongitude(session.startedAt, longitude ?? 0);
}

/** The first of the two forms a night's date is written in, for a **cell or
 * a title**: « Mer. 7 oct. » (weekday, day, month; capitalised as the first
 * word of a cell). `eveningDate` is a calendar date, read at noon UTC. */
export function nightCell(eveningDate: string, locale: string): string {
  return nightDay(`${eveningDate.slice(0, 10)}T12:00:00Z`, locale, "UTC");
}

/** The second form, for a **sentence**: « 7 oct. » (day and month, no weekday,
 * no capital): « Nuit du 7 oct. », « la session du 7 oct. ». */
export function nightSentence(eveningDate: string, locale: string): string {
  const date = new Date(`${eveningDate.slice(0, 10)}T12:00:00Z`);
  if (Number.isNaN(date.getTime())) return "—";
  return new Intl.DateTimeFormat(locale, {
    day: "numeric",
    month: "short",
    timeZone: "UTC",
  }).format(date);
}

/** A sentence that ends with an abbreviation does not take a second full
 * stop: « la session du 7 oct.. » is written « la session du 7 oct. ». */
export function oneFinalStop(text: string): string {
  return text.replace(/([^.])\.\.$/, "$1.");
}

/** « 21:05 » on 24 hours. */
export function hourOf(
  instant: string,
  locale: string,
  timeZone?: string,
): string {
  const date = new Date(instant);
  if (Number.isNaN(date.getTime())) return "—";
  return new Intl.DateTimeFormat(locale, {
    hour: "2-digit",
    minute: "2-digit",
    hourCycle: "h23",
    ...(timeZone ? { timeZone } : {}),
  }).format(date);
}

/** « 21:05 – 04:40 », or « 21:10 – en cours » while the night is open. */
export function hoursLine(
  t: Translate,
  session: Pick<SessionSummary, "startedAt" | "finishedAt" | "status">,
  locale: string,
  timeZone?: string,
): string {
  const start = hourOf(session.startedAt, locale, timeZone);
  if (session.status !== "finished" || !session.finishedAt) {
    return t("nights.hoursOpen", { start });
  }
  return t("nights.hours", {
    start,
    end: hourOf(session.finishedAt, locale, timeZone),
  });
}

/** The place of a night as the service holds it, or `null`. */
export function placeOf(detail: NightDetail | null | undefined): string | null {
  return detail?.night?.place?.value?.name ?? null;
}

/** The targets' names of a night, in the service's order. */
export function targetNames(online: NightOnline | null | undefined): string[] {
  return (online?.targets ?? [])
    .map((target) => target.name ?? target.targetId ?? "")
    .filter((name) => name !== "");
}

/** The header of a night's wizard, as the night's page on the web draws it
 * (`SessionDetailPage.js`): the eyebrow above (the name the night carries --
 * the plan's -- in small gold, or « Nuit sans plan »), the title (« Nuit du
 * mer. 7 oct. », always), and the line under it (« Col des Étoiles · 21:05 →
 * 04:40 UTC · 3 cibles »). The names of the targets are in the « Cibles »
 * step, not here. */
export interface NightHeader {
  eyebrow: string | null;
  title: string;
  line: string;
}

export function nightHeader(
  t: Translate,
  locale: string,
  night: {
    session: Pick<
      SessionSummary,
      "startedAt" | "finishedAt" | "status" | "planId"
    >;
    detail: NightDetail | null;
    online: NightOnline | null;
  },
): NightHeader {
  const { session } = night;
  const name = night.detail?.night?.title?.value?.trim() ?? "";
  const eyebrow =
    name !== "" ? name : session.planId ? null : t("wizard.header.noPlan");
  const title = t("wizard.title", {
    day: nightSentence(
      eveningOfSession(session, longitudeOf(night.detail)),
      locale,
    ),
  });
  const start = hourOf(session.startedAt, locale, "UTC");
  const hours =
    session.status === "finished" && session.finishedAt
      ? `${start} → ${hourOf(session.finishedAt, locale, "UTC")} UTC`
      : `${start} UTC`;
  const count = (night.online?.targets ?? []).filter(
    (target) => target.targetId,
  ).length;
  const line = [
    placeOf(night.detail),
    hours,
    count > 0 ? pluralText(t, locale, "wizard.header.targets", count) : null,
  ]
    .filter(Boolean)
    .join(" · ");
  return { eyebrow, title, line };
}

/** « Col des Étoiles · NGC 7000, IC 1396, M31 ». */
export function placeAndTargets(
  place: string | null,
  targets: readonly string[],
): string {
  const names = targets.join(", ");
  return [place, names || null].filter(Boolean).join(" · ");
}

/** How many frames a night's folder holds: lights and calibration. */
export function frameCount(detection: NightDetection): number {
  const lights = detection.targets
    .filter((target) => target.inNight)
    .reduce((sum, target) => sum + target.lightFrames, 0);
  const c = detection.calibration;
  return lights + c.darks + c.flats + c.darkFlats + c.biases;
}

/** « Sur cet ordinateur »: the night's folder, said in one line. */
export function localLine(
  t: Translate,
  locale: string,
  view: Pick<NightView, "location" | "detection" | "resume" | "integrations">,
): string {
  if (view.location.state === "missing") return t("nights.local.noFolder");
  if (view.location.state === "duplicated") return t("nights.local.twoFolders");
  const detection = view.detection;
  if (!detection) return t("nights.local.noFolder");
  const inNight = detection.targets.filter((target) => target.inNight);
  const frames = frameCount(detection);
  const finals = inNight.filter((target) => target.finalImage !== null).length;
  if (frames === 0 && finals === 0 && detection.logs.length === 0) {
    return t("nights.local.empty");
  }
  const parts = [pluralText(t, locale, "nights.local.targets", inNight.length)];
  if (view.resume.missing.some((m) => m.key === "logsAdded")) {
    parts.push(
      pluralText(t, locale, "nights.local.logsAdded", detection.logs.length),
    );
  } else {
    if (frames > 0)
      parts.push(pluralText(t, locale, "nights.local.frames", frames));
    if (finals > 0)
      parts.push(pluralText(t, locale, "nights.local.finals", finals));
  }
  // The masters kept in the folder: « 1 intégration ».
  const integrations = view.integrations?.length ?? 0;
  if (integrations > 0) {
    parts.push(
      pluralText(t, locale, "nights.local.integrations", integrations),
    );
  }
  return parts.join(" · ");
}

const SOURCE_KEY: Record<string, string> = {
  phone: "nights.online.fromPhone",
  web: "nights.online.fromWeb",
};

/** « En ligne »: the captures, their sky annotations and the analysis. */
export function onlineLine(
  t: Translate,
  locale: string,
  online: NightOnline,
): string {
  const captures = online.targets.flatMap((target) =>
    target.captures.map((capture) => ({ target, capture })),
  );
  const count = captures.length;
  const analysis = online.analysis.present;
  if (count === 0 && !analysis) return t("nights.online.nothing");
  const annotated =
    count > 0 &&
    captures.every(({ capture }) =>
      (capture.mainImage ? [capture.mainImage] : capture.images).every(
        (image) => image.skySolution === "present",
      ),
    );
  const images = pluralText(t, locale, "nights.online.images", count);
  if (count === 1 && !annotated && !analysis) {
    const { target, capture } = captures[0];
    const source =
      SOURCE_KEY[capture.mainImage?.source ?? capture.source ?? ""];
    if (source) {
      return t("nights.online.oneFrom", {
        target: target.name ?? target.targetId ?? "",
        source: t(source),
      });
    }
  }
  if (count === 1 && annotated && !analysis) {
    return t("nights.online.oneAnnotated");
  }
  const parts = count > 0 ? [images] : [];
  if (annotated) parts.push(t("nights.online.annotations"));
  if (analysis) parts.push(t("nights.online.analysis"));
  return parts.join(" · ");
}

export type NightState = "not_started" | "to_complete" | "complete";

/** What a night misses, in the order the sheet lists it: the final
 * images first, then the sky annotations, then the analysis. `context`
 * picks the family of words: the list says « après l'envoi », the home's
 * « À reprendre » row « depuis l'envoi ». */
export function missingText(
  t: Translate,
  locale: string,
  missing: readonly Missing[],
  names: ReadonlyMap<string, string> = new Map(),
  context: "nights" | "home" = "nights",
): string {
  const parts: string[] = [];
  const finals = missing.filter((m) => m.key === "finalImage");
  if (finals.length === 1) {
    const only = finals[0] as Extract<Missing, { key: "finalImage" }>;
    parts.push(
      t("nights.missing.finalOf", {
        target: names.get(only.targetId) ?? only.name,
      }),
    );
  } else if (finals.length > 1) {
    parts.push(pluralText(t, locale, "nights.missing.finals", finals.length));
  }
  for (const item of missing) {
    switch (item.key) {
      case "noTarget":
        parts.push(t("nights.missing.targets"));
        break;
      case "folder":
        parts.push(t("nights.missing.folder"));
        break;
      case "targetFolder":
        parts.push(
          t("nights.missing.targetFolder", {
            target: names.get(item.targetId) ?? item.targetId,
          }),
        );
        break;
      case "skyAnnotations":
        parts.push(pluralText(t, locale, "nights.missing.sky", item.count));
        break;
      default:
        break;
    }
  }
  if (missing.some((m) => m.key === "progression")) {
    parts.push(t("nights.missing.progression"));
  }
  if (missing.some((m) => m.key === "analysis")) {
    parts.push(t("nights.missing.analysis"));
  }
  if (missing.some((m) => m.key === "logsAdded")) {
    parts.push(
      t(
        context === "home"
          ? "home.missing.logsAdded"
          : "nights.missing.logsAdded",
      ),
    );
  }
  for (const item of missing) {
    if (item.key === "imagesToSend") {
      parts.push(pluralText(t, locale, "nights.missing.toSend", item.count));
    } else if (item.key === "queuePending") {
      parts.push(pluralText(t, locale, "nights.missing.queue", item.count));
    }
  }
  return parts.join(", ");
}

/** Why the treatment of a session is held back:
 * `to_debrief` (the night is over and its debriefing is still to be done) or
 * `abandoned` (the plan was not carried out). Both are the contract's own
 * states of a line of the Journal (`journal.schema.json`, `state`), told by
 * the fields the service already sends on a session -- never a second
 * reading of the night:
 *
 * - `abandoned` is the session's `outcome`;
 * - `to_debrief` is a **finished** session of a **plan** whose `outcome` is
 *   still `null` (an absent `outcome` is `null`, as the contract says of an
 *   earlier answer). A session without a plan has no promise to be debriefed
 *   against (`no_plan`) and a night still open or paused is not listed by the
 *   Journal: both stay open to the treatment, as an `executed` or `partial`
 *   session does.
 *
 * It becomes `null` -- the row is selectable again -- as soon as the service
 * gives the session a debriefing. */
export type TreatmentHold = "to_debrief" | "abandoned";

export function treatmentHold(
  session: Pick<SessionSummary, "planId" | "status" | "outcome">,
): TreatmentHold | null {
  if (session.outcome === "abandoned") return "abandoned";
  if (
    session.planId &&
    session.status === "finished" &&
    (session.outcome ?? null) === null
  ) {
    return "to_debrief";
  }
  return null;
}

/** One row of « Nuits », every cell already in words. `null` cells are
 * not known yet (the row fills in as each night is read). */
export interface NightRow {
  sessionId: string;
  day: string;
  hours: string;
  open: boolean;
  placeAndTargets: string;
  local: string | null;
  online: string | null;
  state: NightState | null;
  missing: string | null;
  /** Its detail is being read: the row says so, the star beside. */
  reading: boolean;
  /** The treatment of this session is held back: the row is not
   * selectable and `heldMessage` says what to do. Absent in a snapshot saved
   * by an earlier build, which reads as « not held ». */
  held?: TreatmentHold | null;
  /** « Débriefez cette session dans l'app pour commencer le traitement. » */
  heldMessage?: string | null;
  /** « Plan multi-nuits IC 1805 · 3 nuits · 11 h 08 ». */
  planTag?: string | null;
}

export function nightRow(
  t: Translate,
  locale: string,
  input: {
    session: SessionSummary;
    detail?: NightDetail | null;
    view?: NightView | null;
    timeZone?: string;
    /** Its read is not back yet. */
    reading?: boolean;
    /** The tags of its multi-night plans. */
    tags?: readonly PlanTag[];
  },
): NightRow {
  const { session, detail, view, timeZone } = input;
  const names = new Map(
    (view?.online.targets ?? [])
      .filter((target) => target.targetId && target.name)
      .map((target) => [target.targetId as string, target.name as string]),
  );
  const held = treatmentHold(session);
  return {
    sessionId: session.id,
    day: nightCell(eveningOfSession(session, longitudeOf(detail)), locale),
    hours: hoursLine(t, session, locale, timeZone),
    open: session.status !== "finished",
    placeAndTargets: placeAndTargets(
      placeOf(detail),
      targetNames(view?.online),
    ),
    local: view ? localLine(t, locale, view) : null,
    online: view ? onlineLine(t, locale, view.online) : null,
    // A held row does not announce a state of its own (« Complète », « À
    // compléter »): its message is the state.
    state: held ? null : (view?.resume.status ?? null),
    missing:
      !held && view && view.resume.status === "to_complete"
        ? missingText(t, locale, view.resume.missing, names)
        : null,
    reading: input.reading ?? false,
    held,
    heldMessage: held ? t("nights.held.debrief") : null,
    planTag: nightTag(t, locale, input.tags),
  };
}

/** The newest night first (`startedAt`), as the description says. */
export function newestFirst<T extends { startedAt: string }>(
  sessions: readonly T[],
): T[] {
  return [...sessions].sort((a, b) => b.startedAt.localeCompare(a.startedAt));
}

/** « 8 oct. à 10:20 »: when the online state shown offline was read. */
export function offlineWhen(
  t: Translate,
  locale: string,
  iso: string,
  timeZone?: string,
): string {
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return "—";
  return t("nights.offlineAt", {
    day: new Intl.DateTimeFormat(locale, {
      day: "numeric",
      month: "short",
      ...(timeZone ? { timeZone } : {}),
    }).format(date),
    time: hourOf(iso, locale, timeZone),
  });
}

/** The last rows read online, kept for the offline state (« l'état en
 * ligne est celui du 8 oct. à 10:20 »). */
export interface NightsSnapshot {
  savedAt: string;
  rows: NightRow[];
}

const SNAPSHOT_KEY = "astro-sync.nights";

export interface SnapshotStorage {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
}

export function saveSnapshot(
  storage: SnapshotStorage | null,
  snapshot: NightsSnapshot,
): void {
  try {
    storage?.setItem(SNAPSHOT_KEY, JSON.stringify(snapshot));
  } catch {
    // A full or blocked storage only loses the offline view.
  }
}

export function loadSnapshot(
  storage: SnapshotStorage | null,
): NightsSnapshot | null {
  try {
    const text = storage?.getItem(SNAPSHOT_KEY);
    if (!text) return null;
    const parsed = JSON.parse(text) as NightsSnapshot;
    return Array.isArray(parsed.rows) && typeof parsed.savedAt === "string"
      ? parsed
      : null;
  } catch {
    return null;
  }
}

/** Whether a failure of the list means « no network » (the offline state)
 * rather than a refusal to say. */
export function isOffline(code: string | null | undefined): boolean {
  return (
    code === "network_error" ||
    code === "service_unreachable" ||
    code === "timeout" ||
    code === "cancelled"
  );
}

/** The creation window's fields, checked before « Créer cette nuit »: a
 * date and a place are needed; hours, when given, read « 21:00 ». */
export interface CreateForm {
  date: string;
  place: string;
  start: string;
  end: string;
  title: string;
}

export function createFormReady(form: CreateForm): boolean {
  const hour = (value: string) =>
    value.trim() === "" || /^([01]\d|2[0-3]):[0-5]\d$/.test(value.trim());
  return (
    /^\d{4}-\d{2}-\d{2}$/.test(form.date) &&
    form.place.trim() !== "" &&
    hour(form.start) &&
    hour(form.end)
  );
}

/** The places a night can be created at: the places of the space's nights,
 * the most used first (the service lends no list of saved places to a
 * device yet). */
export function knownPlaces(
  details: readonly (NightDetail | null | undefined)[],
): string[] {
  const counts = new Map<string, number>();
  for (const detail of details) {
    const place = placeOf(detail);
    if (place) counts.set(place, (counts.get(place) ?? 0) + 1);
  }
  return [...counts.entries()]
    .sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]))
    .map(([place]) => place);
}

/** `AAAA-MM-JJ` of the evening a night starts, in the computer's zone. */
export function eveningDateOf(startedAt: string, timeZone?: string): string {
  const date = new Date(startedAt);
  if (Number.isNaN(date.getTime())) return startedAt.slice(0, 10);
  const parts = new Intl.DateTimeFormat("en-CA", {
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
    ...(timeZone ? { timeZone } : {}),
  }).formatToParts(date);
  const part = (type: string) =>
    parts.find((item) => item.type === type)?.value ?? "";
  return `${part("year")}-${part("month")}-${part("day")}`;
}

/** The evening date of a night from its start at the place's longitude
 * (degrees east): the date of the start in local mean solar time, moved
 * back by twelve hours -- the core's `evening_date_of`, the service's rule
 * for a night without a plan (never a time zone: the place rule). */
export function eveningDateAtLongitude(
  startedAt: string,
  longitude: number,
): string {
  const ms = new Date(startedAt).getTime();
  if (Number.isNaN(ms)) return startedAt.slice(0, 10);
  const solar = new Date(ms + (longitude / 15) * 3_600_000 - 12 * 3_600_000);
  return solar.toISOString().slice(0, 10);
}

/** The longitude of the night's place (`GET /v1/sessions/{id}`), if known. */
export function longitudeOf(
  detail: NightDetail | null | undefined,
): number | null {
  const longitude = detail?.night?.place?.value?.longitude;
  return typeof longitude === "number" && Number.isFinite(longitude)
    ? longitude
    : null;
}

/** What the detection of a night needs: its targets as the service names
 * them, which are ticked (all, until « Cibles » says otherwise), which have
 * a final image online, and the night's window -- its evening and the
 * place's longitude (local noon to local noon). */
export function detectionInput(
  session: Pick<SessionSummary, "startedAt"> & {
    eveningDate?: string | null;
  },
  online: NightOnline | null,
  unticked: ReadonlySet<string> = new Set(),
  stacking: ReadonlySet<string> = new Set(),
  timeZone?: string,
  longitude: number | null = null,
): DetectionInput {
  return {
    eveningDate: session.eveningDate
      ? session.eveningDate.slice(0, 10)
      : longitude === null
        ? eveningDateOf(session.startedAt, timeZone)
        : eveningDateAtLongitude(session.startedAt, longitude),
    nightStartedAt: session.startedAt,
    longitude,
    targets: (online?.targets ?? [])
      .filter((target) => target.targetId)
      .map((target) => ({
        targetId: target.targetId as string,
        designation: target.name ?? (target.targetId as string),
        aliases: [],
        ticked: !unticked.has(target.targetId as string),
        onlineFinal: target.captures.some((capture) => capture.mainImage),
        stackingPlanned: stacking.has(target.targetId as string),
      })),
  };
}

/** The targets left unticked at « Cibles », kept per night on this computer. */
export function loadUnticked(
  storage: SnapshotStorage | null,
  sessionId: string,
): Set<string> {
  try {
    const text = storage?.getItem(`astro-sync.unticked:${sessionId}`);
    const parsed = text ? (JSON.parse(text) as unknown) : [];
    return new Set(
      Array.isArray(parsed)
        ? parsed.filter((v): v is string => typeof v === "string")
        : [],
    );
  } catch {
    return new Set();
  }
}

export function saveUnticked(
  storage: SnapshotStorage | null,
  sessionId: string,
  unticked: ReadonlySet<string>,
): void {
  try {
    storage?.setItem(
      `astro-sync.unticked:${sessionId}`,
      JSON.stringify([...unticked]),
    );
  } catch {
    // Lost ticks only bring every target back ticked.
  }
}
