// The lines of « Liste des incidents »: what the core detected and what the
// session holds, said in the person's language and turned into the `Line`s
// `incidents.ts` merges. Pure TypeScript, tested under `node --test`.

import type { Translate } from "../i18n/locale.ts";
import type {
  DeclaredIncident,
  DetectedIncident,
  IncidentKind,
  UnproposedIncident,
} from "./bridge";
import {
  TEXT_LIMITS,
  cut,
  payloadFor,
  type Genre,
  type Line,
} from "./incidents.ts";

const seconds = (iso: string): number => Date.parse(iso) / 1000;

/** « 12 min », « 11 min 47 s », « 47 s »: a length in seconds. */
export function lengthText(t: Translate, total: number): string {
  const whole = Math.max(0, Math.round(total));
  if (whole < 60)
    return t("wizard.sending.durationSeconds", { seconds: whole });
  const minutes = Math.floor(whole / 60);
  const rest = whole % 60;
  if (rest === 0) return t("common.minutesShort", { count: minutes });
  return t("wizard.sending.durationMinutes", {
    minutes,
    seconds: String(rest).padStart(2, "0"),
  });
}

function numberParam(params: Record<string, unknown>, name: string): number {
  const value = params[name];
  return typeof value === "number" ? value : Number(value ?? 0);
}

/** What a detected incident says, from its code and its parameters. */
export function detectedText(
  t: Translate,
  locale: string,
  incident: Pick<DetectedIncident, "code" | "params">,
): string {
  const p = incident.params as Record<string, unknown>;
  const one = (count: number) =>
    new Intl.PluralRules(locale).select(count) === "one" ? ".one" : ".other";
  switch (incident.code) {
    case "meridian_flip":
      return t("incidents.text.meridian_flip");
    case "pose_gap":
      return t("incidents.text.pose_gap", {
        gap: lengthText(t, numberParam(p, "gapSeconds")),
        after: numberParam(p, "afterPose"),
        before: numberParam(p, "beforePose"),
      });
    case "guide_star_lost": {
      const count = numberParam(p, "count");
      const notFound = numberParam(p, "notFound");
      const base = t(`incidents.text.guide_star_lost${one(count)}`, { count });
      return notFound > 0
        ? base +
            t(`incidents.text.guide_star_lost_not_found${one(notFound)}`, {
              count: notFound,
            })
        : base;
    }
    case "guiding_timeouts": {
      const count = numberParam(p, "count");
      return t(`incidents.text.guiding_timeouts${one(count)}`, { count });
    }
    case "guiding_spikes":
      return t("incidents.text.guiding_spikes", {
        count: numberParam(p, "count"),
        threshold: numberParam(p, "thresholdArcSeconds"),
        max: numberParam(p, "maxArcSeconds"),
      });
    case "exposure_change":
      return t("incidents.text.exposure_change", {
        from: numberParam(p, "fromSeconds"),
        to: numberParam(p, "toSeconds"),
      });
    case "sensor_temperature_drift":
      return t("incidents.text.sensor_temperature_drift", {
        from: numberParam(p, "fromCelsius"),
        to: numberParam(p, "toCelsius"),
      });
    default:
      return "";
  }
}

/** The grey line of a detected incident: its parameters said in words. */
export function detectedEvidence(
  t: Translate,
  locale: string,
  incident: Pick<DetectedIncident, "evidence" | "params" | "end" | "start">,
): {
  source: "asiair" | "phd2" | "nina" | "fits";
  code: string;
  params: Record<string, string | number>;
} {
  const { evidence } = incident;
  const p = { ...(evidence.params as Record<string, unknown>) };
  const params: Record<string, string | number> = {
    source: t(`incidents.source.${evidence.source}`),
  };
  for (const [key, value] of Object.entries(p)) {
    if (typeof value === "number" || typeof value === "string") {
      params[key] = value;
    }
  }
  if ("durationSeconds" in p) {
    params.duration = lengthText(t, numberParam(p, "durationSeconds"));
  }
  if ("gapSeconds" in p)
    params.gap = lengthText(t, numberParam(p, "gapSeconds"));
  if ("typicalSeconds" in p) {
    params.typical = lengthText(t, numberParam(p, "typicalSeconds"));
  }
  if (evidence.code === "meridian_flip") {
    const outcome =
      p.succeeded === true
        ? "true"
        : p.succeeded === false
          ? "false"
          : "unknown";
    params.outcome = t(`incidents.flipOutcome.${outcome}`);
    params.gapPart =
      "afterPose" in p
        ? t("incidents.flipGap", {
            after: numberParam(p, "afterPose"),
            before: numberParam(p, "beforePose"),
          })
        : "";
  }
  if (evidence.code === "guide_star_lost") {
    const notFound = numberParam(p, "notFound");
    const one = new Intl.PluralRules(locale).select(notFound) === "one";
    params.notFoundPart =
      notFound > 0
        ? t(`incidents.lostNotFound.${one ? "one" : "other"}`, {
            count: notFound,
          })
        : "";
  }
  return { source: evidence.source, code: evidence.code, params };
}

/** One detected incident as a line of the list. The text is in the language of
 * the window; the payload is the core's, its words replaced by it. */
export function detectedLine(
  t: Translate,
  locale: string,
  incident: DetectedIncident,
): Line {
  const text = detectedText(t, locale, incident);
  const genre = incident.eventType as Genre;
  const editable =
    genre === "pause" ||
    genre === "condition" ||
    genre === "equipment_issue" ||
    genre === "note";
  return {
    id: incident.id,
    origin: "detected",
    genre,
    start: seconds(incident.start),
    end: incident.end ? seconds(incident.end) : null,
    text,
    payload: editable
      ? payloadFor(genre, text, incident.payload)
      : { ...incident.payload, description: cut(text, TEXT_LIMITS[genre]) },
    evidence: detectedEvidence(t, locale, incident),
    eventId: null,
    recordedAt: null,
  };
}

/** What a declared event says: its own words, else its genre (a sky chosen
 * with no word, a pause with its cause). */
export function declaredText(t: Translate, event: DeclaredIncident): string {
  const payload = event.payload;
  const word = (name: string) =>
    typeof payload[name] === "string" ? (payload[name] as string).trim() : "";
  const own =
    word("text") || word("description") || word("reason") || word("notes");
  if (own) return own;
  if (event.eventType === "condition" && word("sky")) {
    return t(`incidents.sky.${word("sky")}`);
  }
  if (event.eventType === "pause" && word("cause")) {
    return t(`incidents.pauseCause.${word("cause")}`);
  }
  return t(`incidents.genre.${event.eventType}`);
}

/** A signal the session holds as a line, or `null` for what is not an incident
 * (a note that only gives a cause to a pause, or says the application did not
 * run). A signal Astro Sync added before is `stored`: it stays. */
export function declaredLine(
  t: Translate,
  event: DeclaredIncident,
): Line | null {
  const payload = event.payload;
  if (
    event.eventType === "note" &&
    typeof payload.text !== "string" &&
    (payload.pauseCause !== undefined || payload.appStopped !== undefined)
  ) {
    return null;
  }
  const byAstroSync = event.source === "astro-sync";
  const recorded = seconds(event.recordedAt);
  const genre = event.eventType as Genre;
  // The new genres say their own words (their payload carries a description).
  return {
    id: event.eventId,
    origin: byAstroSync ? "detected" : "declared",
    genre,
    start: recorded,
    end: event.until ? seconds(event.until) : null,
    text: declaredText(t, event),
    payload,
    evidence: null,
    eventId: event.eventId,
    recordedAt: recorded,
    stored: byAstroSync,
  };
}

/** What is detected and not proposed, said in words (`incidents.unproposed.<code>`). */
export function unproposedText(t: Translate, item: UnproposedIncident): string {
  return t(`incidents.unproposed.${item.code}`, {
    count: item.count,
    ...(item.params as Record<string, string | number>),
  });
}

/** The lines of the genres a person can edit in the window. */
export function isEditable(kind: IncidentKind): boolean {
  return (
    kind === "pause" ||
    kind === "condition" ||
    kind === "equipment_issue" ||
    kind === "note"
  );
}
