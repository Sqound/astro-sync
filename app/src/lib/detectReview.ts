// « Détecter les sessions », the review page before the creation:
// what is ticked, what the person answered, what the footer and the
// primary button count, the request that carries it all back -- pure
// TypeScript (no React, no Tauri), so it runs under `node --test`. The
// engine gives the lines (one id each, `checked`, `locked`, `ask`,
// `reason`); the window only keeps the person's answers on top of them.

import type {
  BulkCreateRequest,
  DetectedSession,
  DetectedTarget,
  MultiNightGroup,
  PlaceAnswer,
  PlaceProposal,
  Proposal,
  ProposalLine,
  UserData,
} from "./bridge";
import type { Translate } from "../i18n/locale.ts";
import { nightDay } from "./nights.ts";
import { conjunction, pluralText } from "./written.ts";

/** The person's answer for one place of the proposal: one of « Mes lieux »,
 * or a place to create (its name; the coordinates are the poses'). */
export type PlaceChoice =
  { kind: "known"; knownPlaceId: string } | { kind: "create"; name: string };

export interface ReviewState {
  /** The lines left ticked, by line id. */
  ticked: Record<string, boolean>;
  /** The lines that ask (an overlap): `add` or `skip`; none: not answered. */
  answers: Record<string, "add" | "skip">;
  /** The place chosen for each place of the proposal, by its id. */
  places: Record<string, PlaceChoice>;
  /** The whole hours to add to the clock of each session. */
  shifts: Record<string, number>;
  /** The band answered for each night whose poses name no filter, by session
   * id; none: not answered, the night waits. */
  filters: Record<string, NightBand>;
}

/** What the person says of a night whose poses carry no `FILTER`. */
export type NightBand = "broadband" | "dualband";

/** The filter written for a band the person gave (never one they did not). */
const BAND_FILTER: Record<NightBand, string> = {
  broadband: "Broadband",
  dualband: "Dual-band",
};

/** The lines of one night, found once. */
export interface NightLines {
  session: ProposalLine | null;
  targets: Record<string, ProposalLine>;
  clock: ProposalLine | null;
  calibration: ProposalLine[];
  logs: ProposalLine[];
}

/** The lines of every session, by session id. */
export function nightLines(proposal: Proposal): Record<string, NightLines> {
  const out: Record<string, NightLines> = {};
  for (const session of proposal.sessions) {
    out[session.id] = {
      session: null,
      targets: {},
      clock: null,
      calibration: [],
      logs: [],
    };
  }
  for (const line of proposal.lines) {
    const id = line.subject.split("/")[0];
    const night = out[id];
    if (!night) continue;
    if (line.kind === "session") night.session = line;
    else if (line.kind === "target") {
      night.targets[line.subject.slice(id.length + 1)] = line;
    } else if (line.kind === "clock") night.clock = line;
    else if (line.kind === "calibration") night.calibration.push(line);
    else if (line.kind === "log") night.logs.push(line);
  }
  return out;
}

export function placeLine(
  proposal: Proposal,
  place: PlaceProposal,
): ProposalLine | null {
  return (
    proposal.lines.find(
      (line) => line.kind === "place" && line.subject === place.id,
    ) ?? null
  );
}

/** The review once the proposal followed the rights of the account (an offer
 * granted or ended since the detection): what the person ticked or answered
 * stays; a line that appeared is ticked as the engine proposes it; the
 * multi-night plans that just opened (`unlocked`) are ticked, the person never
 * had the choice of them; a line that is locked now is no longer ticked. */
export function followRights(
  proposal: Proposal,
  review: ReviewState,
  unlocked: readonly string[],
): ReviewState {
  const ticked: Record<string, boolean> = {};
  for (const line of proposal.lines) {
    const kept = review.ticked[line.id];
    ticked[line.id] =
      !line.locked &&
      (unlocked.includes(line.id) ||
        (kept === undefined ? line.checked && !line.ask : kept));
  }
  return { ...review, ticked };
}

/** The state when the page opens: what the engine ticked and did not lock;
 * the places it matched; the clock shift it proposed. Nothing that asks is
 * ticked or answered, and a place to create or to choose waits. The band of
 * a night is preselected when its names say it (`filterGuess`). */
export function initialState(proposal: Proposal): ReviewState {
  const ticked: Record<string, boolean> = {};
  for (const line of proposal.lines) {
    ticked[line.id] = line.checked && !line.locked && !line.ask;
  }
  const places: Record<string, PlaceChoice> = {};
  for (const place of proposal.places) {
    const waits = place.action === "create" || place.action === "choose";
    if (!waits && place.chosen) {
      places[place.id] = { kind: "known", knownPlaceId: place.chosen };
    }
    const line = placeLine(proposal, place);
    // A place to create is asked, whatever the engine ticked: its nights wait.
    if (line && waits) ticked[line.id] = false;
  }
  const shifts: Record<string, number> = {};
  for (const session of proposal.sessions) {
    shifts[session.id] = session.clock?.proposedShiftHours ?? 0;
  }
  // A night whose names say its band starts with that band chosen; the
  // person sees where it comes from and can change it.
  const filters: Record<string, NightBand> = {};
  for (const session of proposal.sessions) {
    if (session.filterAsk && session.filterGuess) {
      filters[session.id] = session.filterGuess.band;
    }
  }
  return { ticked, answers: {}, places, shifts, filters };
}

// ------------------------------------------------------------------ updates

export function toggleLine(state: ReviewState, id: string): ReviewState {
  return { ...state, ticked: { ...state.ticked, [id]: !state.ticked[id] } };
}

export function answerLine(
  state: ReviewState,
  id: string,
  answer: "add" | "skip",
): ReviewState {
  return { ...state, answers: { ...state.answers, [id]: answer } };
}

/** The band of a night answered; `null` takes the answer back. */
export function chooseFilter(
  state: ReviewState,
  sessionId: string,
  band: NightBand | null,
): ReviewState {
  const filters = { ...state.filters };
  if (band) filters[sessionId] = band;
  else delete filters[sessionId];
  return { ...state, filters };
}

/** The place answered (the line of the place is ticked with it); `null`
 * takes the answer back. */
export function choosePlace(
  proposal: Proposal,
  state: ReviewState,
  place: PlaceProposal,
  choice: PlaceChoice | null,
): ReviewState {
  const places = { ...state.places };
  if (choice) places[place.id] = choice;
  else delete places[place.id];
  const line = placeLine(proposal, place);
  const ticked = line
    ? { ...state.ticked, [line.id]: choice !== null }
    : state.ticked;
  return { ...state, places, ticked };
}

/** The shift chosen for a suspect clock; 0 is « Aucun décalage » (the clock
 * line is unticked, the hours chosen before are kept). */
export function chooseShift(
  proposal: Proposal,
  state: ReviewState,
  sessionId: string,
  hours: number,
): ReviewState {
  const clock = nightLines(proposal)[sessionId]?.clock;
  const shifts = { ...state.shifts };
  const ticked = { ...state.ticked };
  if (hours === 0) {
    if (clock) ticked[clock.id] = false;
  } else {
    shifts[sessionId] = hours;
    if (clock) ticked[clock.id] = true;
  }
  return { ...state, shifts, ticked };
}

// ------------------------------------------------------------ what depends

/** The place of a place proposal as answered; `null`: the nights wait. A
 * place whose line was unticked is not answered. */
export function placeOf(
  proposal: Proposal,
  state: ReviewState,
  place: PlaceProposal,
): PlaceChoice | null {
  const line = placeLine(proposal, place);
  if (line && !state.ticked[line.id]) return null;
  return state.places[place.id] ?? null;
}

/** Whether the person emptied the name of a place to create. */
export function isBlankName(name: string): boolean {
  return name.trim() === "";
}

/** The places to create whose name was emptied, while a night that uses them
 * would be created: « Continuer » waits for a name (a place with no name is
 * never created). */
export function placesWithoutName(
  proposal: Proposal,
  state: ReviewState,
): PlaceProposal[] {
  return proposal.places.filter((place) => {
    const choice = placeOf(proposal, state, place);
    if (choice?.kind !== "create" || !isBlankName(choice.name)) return false;
    return place.sessions.some((id) => {
      const session = proposal.sessions.find((item) => item.id === id);
      return session && nightStatus(proposal, state, session).on;
    });
  });
}

export interface NightStatus {
  /** The place of the night is not chosen: the night waits for it. */
  waitsPlace: boolean;
  /** The poses name no filter and the band of the night is not answered:
   * the night waits for it. */
  waitsFilter: boolean;
  /** Locked by the engine (a plan of that date under the one-plan rule). */
  locked: boolean;
  /** The night will be created or updated. */
  on: boolean;
  /** The hours of the clock correction in force (0: none). */
  shiftHours: number;
}

export function nightStatus(
  proposal: Proposal,
  state: ReviewState,
  session: DetectedSession,
): NightStatus {
  const lines = nightLines(proposal)[session.id];
  const place = proposal.places.find((item) => item.id === session.place);
  const waitsPlace = !place || placeOf(proposal, state, place) === null;
  const waitsFilter = Boolean(session.filterAsk) && !state.filters[session.id];
  const locked = lines?.session?.locked ?? false;
  const on =
    !waitsPlace &&
    !waitsFilter &&
    !locked &&
    Boolean(lines?.session && state.ticked[lines.session.id]);
  const clock = lines?.clock;
  const shiftHours =
    clock && state.ticked[clock.id] ? (state.shifts[session.id] ?? 0) : 0;
  return { waitsPlace, waitsFilter, locked, on, shiftHours };
}

export interface GroupStatus {
  /** One night of the group waits for its place. */
  waitsPlace: boolean;
  /** The first night of the group that waits, for « Attend le lieu du… ». */
  waitingDate: string | null;
  /** Fewer than two nights of the group are left on. */
  short: boolean;
  /** The nights left on, and what they add up to. */
  nights: MultiNightGroup["nights"];
  seconds: number;
  frames: number;
  on: boolean;
}

export function groupStatus(
  proposal: Proposal,
  state: ReviewState,
  group: MultiNightGroup,
): GroupStatus {
  const line = proposal.lines.find(
    (item) => item.kind === "multi-night" && item.subject === group.id,
  );
  let waitingDate: string | null = null;
  const nights = group.nights.filter((night) => {
    const session = proposal.sessions.find((s) => s.id === night.sessionId);
    if (!session) return false;
    const status = nightStatus(proposal, state, session);
    if (status.waitsPlace && waitingDate === null) {
      waitingDate = night.eveningDate;
    }
    return status.on;
  });
  const short = nights.length < 2;
  const waitsPlace = waitingDate !== null;
  return {
    waitsPlace,
    waitingDate,
    short,
    nights,
    seconds: nights.reduce((sum, night) => sum + night.seconds, 0),
    frames: nights.reduce((sum, night) => sum + night.frames, 0),
    on:
      Boolean(line && state.ticked[line.id]) &&
      !group.locked &&
      !waitsPlace &&
      !short,
  };
}

/** Whether a line of a night counts: its night is on, and the line is ticked
 * (a line that asks counts through its answer). */
function targetOn(
  state: ReviewState,
  line: ProposalLine,
  night: NightStatus,
): boolean {
  return night.on && !line.locked && Boolean(state.ticked[line.id]);
}

/** The ids of the lines left ticked, with what depends on them: a night
 * without its place is not ticked, nor are its targets, clock, calibration
 * and logs; a place never answered is not ticked. */
export function tickedIds(proposal: Proposal, state: ReviewState): string[] {
  const ids: string[] = [];
  for (const place of proposal.places) {
    const line = placeLine(proposal, place);
    if (line && placeOf(proposal, state, place)) ids.push(line.id);
  }
  for (const line of proposal.lines) {
    if (line.kind === "equipment" && state.ticked[line.id]) ids.push(line.id);
  }
  const lines = nightLines(proposal);
  for (const session of proposal.sessions) {
    const night = nightStatus(proposal, state, session);
    if (!night.on) continue;
    const of = lines[session.id];
    if (of.session) ids.push(of.session.id);
    for (const line of Object.values(of.targets)) {
      if (!line.ask && targetOn(state, line, night)) ids.push(line.id);
    }
    if (of.clock && night.shiftHours !== 0) ids.push(of.clock.id);
    for (const line of [...of.calibration, ...of.logs]) {
      if (state.ticked[line.id]) ids.push(line.id);
    }
  }
  for (const group of proposal.multiNight) {
    const line = proposal.lines.find(
      (item) => item.kind === "multi-night" && item.subject === group.id,
    );
    if (line && groupStatus(proposal, state, group).on) ids.push(line.id);
  }
  return ids;
}

/** The proposal with the band of each night answered written as the filter
 * of its targets that name none; the treatments read the band from it. */
function withAnsweredFilters(proposal: Proposal, state: ReviewState): Proposal {
  if (Object.keys(state.filters).length === 0) return proposal;
  return {
    ...proposal,
    sessions: proposal.sessions.map((session) => {
      const band = state.filters[session.id];
      if (!session.filterAsk || !band) return session;
      return {
        ...session,
        targets: session.targets.map((target) =>
          target.filters.length > 0
            ? target
            : { ...target, filters: [BAND_FILTER[band]] },
        ),
      };
    }),
  };
}

/** The place a place to create is given by the person: the name they typed is
 * written as the one candidate and the one chosen of the proposal's place
 * (`created:<place>`), so that whatever reads the name of a night's place
 * from the proposal -- the night's folder `AAAA-MM-JJ · Col de l'arrière-pays · id`, the
 * result page -- finds it. The answer (`places`) still decides what is
 * created. */
function withCreatedPlaces(
  proposal: Proposal,
  answers: readonly PlaceAnswer[],
): Proposal {
  const named = answers.filter(
    (answer) => answer.kind === "create" && !isBlankName(answer.name),
  );
  if (named.length === 0) return proposal;
  return {
    ...proposal,
    places: proposal.places.map((place) => {
      const answer = named.find((item) => item.placeId === place.id);
      if (!answer || answer.kind !== "create") return place;
      const id = `created:${place.id}`;
      return {
        ...place,
        candidates: [{ placeId: id, name: answer.name.trim(), distanceKm: 0 }],
        chosen: id,
      };
    }),
  };
}

/** « Créer … »: the request as ticked. Nothing outside it is written. */
export function buildRequest(
  proposal: Proposal,
  state: ReviewState,
): BulkCreateRequest {
  const lines = nightLines(proposal);
  const places: PlaceAnswer[] = [];
  for (const place of proposal.places) {
    const choice = placeOf(proposal, state, place);
    const used = place.sessions.some((id) => {
      const session = proposal.sessions.find((s) => s.id === id);
      return session && nightStatus(proposal, state, session).on;
    });
    if (!choice || !used) continue;
    places.push(
      choice.kind === "known"
        ? {
            kind: "known",
            placeId: place.id,
            knownPlaceId: choice.knownPlaceId,
          }
        : {
            kind: "create",
            placeId: place.id,
            name: choice.name,
            latitude: place.latitude ?? 0,
            longitude: place.longitude ?? 0,
            timeZone: null,
          },
    );
  }
  const answers: BulkCreateRequest["answers"] = [];
  const shifts: BulkCreateRequest["shifts"] = [];
  for (const session of proposal.sessions) {
    const night = nightStatus(proposal, state, session);
    if (!night.on) continue;
    for (const line of Object.values(lines[session.id].targets)) {
      const answer = state.answers[line.id];
      if (line.ask && answer) answers.push({ lineId: line.id, answer });
    }
    if (night.shiftHours !== 0) {
      shifts.push({ sessionId: session.id, hours: night.shiftHours });
    }
  }
  return {
    proposal: withCreatedPlaces(withAnsweredFilters(proposal, state), places),
    ticked: tickedIds(proposal, state),
    places,
    answers,
    shifts,
  };
}

// --------------------------------------------------------------- the counts

export interface PlanChange {
  planName: string;
  date: string;
  targets: string[];
}

export interface AddedTarget {
  designation: string;
  date: string;
}

export interface Counts {
  /** Sessions made: a new night (a new plan, a reused one, a replaced one). */
  created: number;
  /** Sessions of the person that receive poses or a target. */
  updated: number;
  /** Poses attached to the sessions that exist. */
  attachedFrames: number;
  attachedSessions: number;
  reused: PlanChange[];
  replaced: PlanChange[];
  added: AddedTarget[];
  /** The goals (seconds) of the multi-night plans made. */
  multiGoals: number[];
  hasCalibration: boolean;
  hasLogs: boolean;
  /** The targets that ask and were not answered: not added. */
  unanswered: AddedTarget[];
}

const designations = (session: DetectedSession): string[] =>
  session.targets.map((target) => target.designation);

export function counts(proposal: Proposal, state: ReviewState): Counts {
  const out: Counts = {
    created: 0,
    updated: 0,
    attachedFrames: 0,
    attachedSessions: 0,
    reused: [],
    replaced: [],
    added: [],
    multiGoals: [],
    hasCalibration: false,
    hasLogs: false,
    unanswered: [],
  };
  const lines = nightLines(proposal);
  for (const session of proposal.sessions) {
    const night = nightStatus(proposal, state, session);
    if (!night.on) continue;
    const of = lines[session.id];
    const existing = session.existing;
    if (existing.kind === "existing") {
      out.updated += 1;
      const inside = session.targets.filter(
        (target) => target.existing === "in-session",
      );
      if (inside.length > 0) out.attachedSessions += 1;
      out.attachedFrames += inside.reduce((sum, t) => sum + t.frames, 0);
    } else {
      out.created += 1;
      const change = {
        planName: "planName" in existing ? existing.planName : "",
        date: session.eveningDate,
        targets: designations(session),
      };
      if (existing.kind === "reuse-plan") out.reused.push(change);
      if (existing.kind === "replace-plan") out.replaced.push(change);
    }
    for (const target of session.targets) {
      const line = of.targets[target.key];
      if (!line) continue;
      const add = {
        designation: target.designation,
        date: session.eveningDate,
      };
      if (line.ask) {
        const answer = state.answers[line.id];
        if (answer === "add") out.added.push(add);
        else if (answer !== "skip") out.unanswered.push(add);
      } else if (
        target.existing === "add-to-session" &&
        targetOn(state, line, night)
      ) {
        out.added.push(add);
      }
    }
    if (of.calibration.some((line) => state.ticked[line.id])) {
      out.hasCalibration = true;
    }
    if (of.logs.some((line) => state.ticked[line.id])) out.hasLogs = true;
  }
  for (const group of proposal.multiNight) {
    const status = groupStatus(proposal, state, group);
    if (status.on) out.multiGoals.push(status.seconds);
  }
  return out;
}

/** The primary's label: what is created, or the sessions updated when
 * nothing is. */
export type CreateLabel =
  | { kind: "both"; sessions: number; plans: number }
  | { kind: "sessions"; sessions: number }
  | { kind: "plans"; plans: number }
  | { kind: "update"; sessions: number }
  | { kind: "none" };

export function createLabel(
  proposal: Proposal,
  state: ReviewState,
): CreateLabel {
  const found = counts(proposal, state);
  const sessions = found.created;
  const plans = found.multiGoals.length;
  if (sessions > 0 && plans > 0) return { kind: "both", sessions, plans };
  if (sessions > 0) return { kind: "sessions", sessions };
  if (plans > 0) return { kind: "plans", plans };
  if (found.updated > 0) return { kind: "update", sessions: found.updated };
  return { kind: "none" };
}

/** The nights that wait for their band, oldest first: when the primary is
 * greyed they are why, and the page names them and leads to the first. */
export function nightsWaitingBand(
  proposal: Proposal,
  state: ReviewState,
): DetectedSession[] {
  return proposal.sessions
    .filter((session) => nightStatus(proposal, state, session).waitsFilter)
    .sort((a, b) => a.eveningDate.localeCompare(b.eveningDate));
}

// ---------------------------------------------------------------- the words

/** `5 h 20`, `11 h 09`, `3 h 00`, `6 min`: the length of the acquisition,
 * the minutes on two digits after an hour. */
export function formatHours(seconds: number): string {
  const minutes = Math.round(seconds / 60);
  const h = Math.floor(minutes / 60);
  const m = minutes % 60;
  if (h === 0) return `${m} min`;
  return `${h} h ${String(m).padStart(2, "0")}`;
}

/** `1,4` / `1.4`: a distance in km to one decimal, in the language's own
 * way (the unit stays in the sentence's key). */
export function formatKm(km: number, locale: string): string {
  return new Intl.NumberFormat(locale, {
    minimumFractionDigits: 1,
    maximumFractionDigits: 1,
  }).format(km);
}

/** `43,8731 N · 7,4312 E`. */
export function formatCoordinates(
  latitude: number,
  longitude: number,
  locale: string,
): string {
  const number = new Intl.NumberFormat(locale, {
    minimumFractionDigits: 4,
    maximumFractionDigits: 4,
  });
  return `${number.format(Math.abs(latitude))} ${latitude < 0 ? "S" : "N"} · ${number.format(Math.abs(longitude))} ${longitude < 0 ? "W" : "E"}`;
}

/** `−9 h`, `+3 h`: a clock shift, with the true minus sign. */
export function formatShift(hours: number): string {
  if (hours === 0) return "0 h";
  return `${hours < 0 ? "−" : "+"}${Math.abs(hours)} h`;
}

/** The whole hours a clock may be shifted by, −12 to +12 without 0. */
export const SHIFT_CHOICES: readonly number[] = Array.from(
  { length: 25 },
  (_, index) => index - 12,
).filter((hours) => hours !== 0);

/** `12:10` of an ISO instant in UTC. */
export function utcHour(instant: string): string {
  return instant.slice(11, 16);
}

/** An ISO instant moved by whole hours (`2026-08-03T12:20:00Z`, +9). */
export function shiftInstant(instant: string, hours: number): string {
  const moved = new Date(Date.parse(instant) + hours * 3_600_000);
  return moved.toISOString().replace(/\.\d{3}Z$/, "Z");
}

/** `Ven. 31 juil.`: the evening of a night, as a cell opens. */
export function eveningLabel(date: string, locale: string): string {
  return nightDay(`${date}T12:00:00Z`, locale, "UTC");
}

/** `7 août`: a date inside a sentence. */
export function dayMonth(date: string, locale: string): string {
  return new Intl.DateTimeFormat(locale, {
    day: "numeric",
    month: "long",
    timeZone: "UTC",
  }).format(new Date(`${date}T12:00:00Z`));
}

/** What follows the day in a date of the language (` août`), when the day
 * comes first and nothing else is in front; `null` otherwise (the dates are
 * then written one by one). */
function monthTail(date: string, locale: string): string | null {
  const parts = new Intl.DateTimeFormat(locale, {
    day: "numeric",
    month: "long",
    timeZone: "UTC",
  }).formatToParts(new Date(`${date}T12:00:00Z`));
  const day = parts.findIndex((part) => part.type === "day");
  const month = parts.findIndex((part) => part.type === "month");
  if (day !== 0 || month < 0) return null;
  return parts
    .slice(1)
    .map((part) => part.value)
    .join("");
}

/** `9, 10 et 11 août`, `3 et 15 août`, `31 juillet et 3 août`: dates that
 * share a month are written with it once. */
export function datesText(dates: string[], locale: string): string {
  const groups: {
    month: string;
    days: string[];
    dates: string[];
    last: string;
  }[] = [];
  for (const date of [...dates].sort()) {
    const month = date.slice(0, 7);
    const day = new Intl.DateTimeFormat(locale, {
      day: "numeric",
      timeZone: "UTC",
    }).format(new Date(`${date}T12:00:00Z`));
    const last = groups[groups.length - 1];
    if (last && last.month === month) {
      last.days.push(day);
      last.dates.push(date);
      last.last = date;
    } else {
      groups.push({ month, days: [day], dates: [date], last: date });
    }
  }
  const words = groups.flatMap((group) => {
    const tail = monthTail(group.last, locale);
    if (group.days.length === 1 || tail === null) {
      return group.days.length === 1
        ? [dayMonth(group.last, locale)]
        : group.dates.map((date) => dayMonth(date, locale));
    }
    return [`${conjunction(locale, group.days)}${tail}`];
  });
  return conjunction(locale, words);
}

/** `300 s` or `180 s, 300 s`: the exposures of a target's frames. */
export function exposures(target: DetectedTarget): number[] {
  return [
    ...new Set(
      target.acquisition
        .map((row) => row.exposureSeconds)
        .filter((value): value is number => value !== null),
    ),
  ];
}

/** A list in the language's way (`A, B et C`). */
export function listText(items: string[], locale: string): string {
  return conjunction(locale, items);
}

// ------------------------------------------------- calibration and the logs

export interface CalibrationItem {
  setId: string;
  kind: string;
  frames: number;
  exposureSeconds: number | null;
  gain: number | null;
  temperatureC: number | null;
  filter: string | null;
  /** The day the set was taken, `AAAA-MM-JJ`, and its first hour. */
  date: string | null;
  hour: string | null;
  daysAway: number;
  matchedOn: string[];
}

const KIND_ORDER = ["dark", "flat", "dark-flat", "bias"];

/** The sets attached to a night, darks first. */
export function calibrationItems(
  proposal: Proposal,
  session: DetectedSession,
): CalibrationItem[] {
  const items: CalibrationItem[] = [];
  for (const attached of session.calibration) {
    const set = proposal.calibrationSets.find((s) => s.id === attached.setId);
    if (!set) continue;
    items.push({
      setId: set.id,
      kind: attached.kind,
      frames: set.frames,
      exposureSeconds: set.exposureSeconds,
      gain: set.gain,
      temperatureC: set.temperatureC,
      filter: set.filter,
      date: set.start ? set.start.slice(0, 10) : null,
      hour: set.start ? utcHour(set.start) : null,
      daysAway: attached.daysAway,
      matchedOn: attached.matchedOn,
    });
  }
  return items.sort(
    (a, b) => KIND_ORDER.indexOf(a.kind) - KIND_ORDER.indexOf(b.kind),
  );
}

/** Whether the detail of the calibration is worth its box: a set of another
 * day serves the night. */
export function borrowsCalibration(items: CalibrationItem[]): boolean {
  return items.some((item) => item.daysAway > 0);
}

/** The criteria the sets were matched on, in the order they are said. */
export function matchCriteria(items: CalibrationItem[]): string[] {
  const order = ["date", "exposure", "temperature", "gain", "filter"];
  const found = new Set(items.flatMap((item) => item.matchedOn));
  return order.filter((name) => found.has(name));
}

export interface LogItem {
  kind: string;
  count: number;
  /** The first day of the first file, `AAAA-MM-JJ`. */
  date: string | null;
}

/** The logs of a night, one entry per kind, in the order they were found. */
export function logItems(
  proposal: Proposal,
  session: DetectedSession,
): LogItem[] {
  const items: LogItem[] = [];
  for (const attached of session.logs) {
    const file = proposal.logs.find((log) => log.id === attached.logId);
    if (!file) continue;
    const found = items.find((item) => item.kind === file.kind);
    if (found) found.count += 1;
    else {
      items.push({
        kind: file.kind,
        count: 1,
        date: file.start ? file.start.slice(0, 10) : null,
      });
    }
  }
  return items;
}

/** `ASIAIR`, `PHD2`, `N.I.N.A.`: how a log's kind is named. */
export function logKindName(kind: string): string {
  if (kind === "nina") return "N.I.N.A.";
  return kind.toUpperCase();
}

/** The files set aside, for « Voir les 25 fichiers ». */
export function asideFiles(proposal: Proposal): {
  noDate: string[];
  calibration: string[];
  logs: string[];
  all: string[];
} {
  const noDate = proposal.setAside.withoutDate;
  const calibration = proposal.setAside.calibrationUnattached;
  const logs = proposal.setAside.logsUnattached;
  // The videos, their capture files and the RAW files are listed too:
  // their extensions say what each is.
  const unread = [
    ...(proposal.setAside.videos ?? []),
    ...(proposal.setAside.videoCaptures ?? []),
    ...(proposal.setAside.rawFiles ?? []),
  ];
  return {
    noDate,
    calibration,
    logs,
    all: [...noDate, ...calibration, ...logs, ...unread],
  };
}

/** The targets seen in the sessions that no multi-night group holds. */
export function singleNightTargets(proposal: Proposal): string[] {
  const grouped = new Set(proposal.multiNight.map((group) => group.targetKey));
  const out: string[] = [];
  for (const session of proposal.sessions) {
    for (const target of session.targets) {
      if (!grouped.has(target.key) && !out.includes(target.designation)) {
        out.push(target.designation);
      }
    }
  }
  return out;
}

/** The name of the place of a night's place proposal as answered: one of
 * « Mes lieux », the name given to a place to create; `null`: not chosen. */
export function placeNameOf(
  proposal: Proposal,
  user: UserData,
  state: ReviewState,
  placeId: string,
): string | null {
  const place = proposal.places.find((item) => item.id === placeId);
  const choice = place ? placeOf(proposal, state, place) : null;
  if (!choice) return null;
  if (choice.kind === "create") return choice.name;
  return (
    user.places.find((item) => item.id === choice.knownPlaceId)?.name ??
    place?.candidates.find((c) => c.placeId === choice.knownPlaceId)?.name ??
    null
  );
}

/** The parts of « Sera créé : … » in the language of the window, from the
 * counts: the sessions and their plans (with the plans reused or replaced),
 * the multi-night plans, the poses attached, the targets added, the
 * calibration and logs. */
export function summaryParts(
  t: Translate,
  locale: string,
  found: Counts,
): string[] {
  const parts: string[] = [];
  if (found.created > 0) {
    const sessions = pluralText(
      t,
      locale,
      "detect.summary.sessions",
      found.created,
    );
    const details = [
      ...found.reused.map((item) =>
        t("detect.summary.reuse", {
          plan: item.planName,
          date: dayMonth(item.date, locale),
        }),
      ),
      ...found.replaced.map((item) =>
        t("detect.summary.replace", {
          target: listText(item.targets, locale),
          plan: item.planName,
          date: dayMonth(item.date, locale),
        }),
      ),
    ];
    parts.push(
      details.length > 0
        ? t("detect.summary.dont", {
            sessions,
            details: conjunction(locale, details),
          })
        : sessions,
    );
  }
  if (found.multiGoals.length > 0) {
    parts.push(
      pluralText(t, locale, "detect.summary.multi", found.multiGoals.length, {
        goals: conjunction(locale, found.multiGoals.map(formatHours)),
      }),
    );
  }
  if (found.attachedSessions > 0) {
    parts.push(
      pluralText(t, locale, "detect.summary.frames", found.attachedSessions, {
        frames: found.attachedFrames,
      }),
    );
  }
  for (const item of found.added) {
    parts.push(
      t("detect.summary.added", {
        target: item.designation,
        date: dayMonth(item.date, locale),
      }),
    );
  }
  if (found.hasCalibration || found.hasLogs) {
    parts.push(
      t(
        found.hasCalibration && found.hasLogs
          ? "detect.summary.calibLogs"
          : found.hasCalibration
            ? "detect.summary.calib"
            : "detect.summary.logs",
      ),
    );
  }
  return parts;
}
