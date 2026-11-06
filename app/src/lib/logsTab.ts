// « Journaux »: what the tab says about a night's logs -- the
// files found in `logs/` with their reader and size, the analysis online
// the report would replace, and once sent, what was read and what was
// replaced. Pure TypeScript, tested under `node --test`;
// `screens/LogsScreen.tsx` draws it.

import type { TranslateParams } from "../i18n/locale.ts";
import type {
  DetectedLog,
  LogsUtcOffset,
  NightAnalysis,
  NightAnalysisSent,
  SessionReport,
} from "./bridge";
import { dayInSentence } from "./tabs.ts";
import { listText } from "./wizardFiles.ts";
import { formatSize, pluralText } from "./written.ts";

type Translate = (key: string, params?: TranslateParams) => string;

/** The four states the tab shows, and the one between them. */
export type LogsState =
  "reserved" | "noNight" | "found" | "none" | "sending" | "sent";

export function logsState(input: {
  reserved: boolean;
  hasNight: boolean;
  logs: readonly DetectedLog[] | null;
  sending: boolean;
  sent: boolean;
}): LogsState {
  if (input.reserved) return "reserved";
  if (!input.hasNight) return "noNight";
  if (input.sent) return "sent";
  if (input.sending) return "sending";
  return input.logs && input.logs.some((log) => log.kind !== "unknown")
    ? "found"
    : "none";
}

function fileName(relativePath: string): string {
  return relativePath.split("/").pop() ?? relativePath;
}

/** One line of « Journaux trouvés »: `logs/` (in the code face, drawn by
 * the screen) then « NINA · journal de séquence du 5 oct. : paliers, mises
 * au point · 1,4 Mo ». */
export function logLine(
  t: Translate,
  locale: string,
  log: DetectedLog,
  eveningDate: string | null,
): string {
  const file = fileName(log.relativePath);
  const what =
    log.kind === "nina"
      ? t("logs.kind.nina", {
          date: eveningDate ? dayWithoutWeekday(eveningDate, locale) : file,
        })
      : t(`logs.kind.${log.kind}`, { file });
  return t("logs.line", { what, size: formatSize(t, log.bytes, locale) });
}

/** « 5 oct. »: the evening without its weekday. */
function dayWithoutWeekday(eveningDate: string, locale: string): string {
  return new Intl.DateTimeFormat(locale, {
    day: "numeric",
    month: "short",
    timeZone: "UTC",
  }).format(new Date(`${eveningDate}T12:00:00Z`));
}

/** The known logs first (NINA, then PHD2, then ASIAIR), the unknown last,
 * each group in the folder's order. */
export function orderedLogs(logs: readonly DetectedLog[]): DetectedLog[] {
  const rank = { nina: 0, phd2: 1, asiair: 2, unknown: 3 } as const;
  return [...logs].sort((a, b) => rank[a.kind] - rank[b.kind]);
}

/** « Une analyse du 3 oct. est en ligne pour cette nuit : le nouveau
 * rapport la remplacerait. » -- the amber line; nothing when the night has
 * no analysis online. */
export function replacesLine(
  t: Translate,
  locale: string,
  sentAt: string | null | undefined,
): string | null {
  if (!sentAt) return null;
  return t("logs.replaces", { date: shortDay(sentAt, locale) });
}

function shortDay(instant: string, locale: string): string {
  const date = new Date(instant);
  if (Number.isNaN(date.getTime())) return instant;
  return new Intl.DateTimeFormat(locale, {
    day: "numeric",
    month: "short",
  }).format(date);
}

/** The guiding of the whole night: the root mean square of every slice's
 * total, weighted by its samples (« 0,82″ »). */
export function nightGuidingRms(report: SessionReport): number | null {
  let squares = 0;
  let samples = 0;
  for (const slice of report.guidingSeries) {
    if (slice.rmsTotal === null || slice.samples <= 0) continue;
    squares += slice.rmsTotal * slice.rmsTotal * slice.samples;
    samples += slice.samples;
  }
  return samples > 0 ? Math.sqrt(squares / samples) : null;
}

function twoDecimals(value: number, locale: string): string {
  return new Intl.NumberFormat(locale, {
    minimumFractionDigits: 2,
    maximumFractionDigits: 2,
  }).format(value);
}

/** « Heure des journaux : PHD2 +02:00, ASIAIR +02:00, trouvée d'après vos
 * poses »: the clocks the report was read with; nothing without a log. */
export function logsClockLine(
  t: Translate,
  clock: LogsUtcOffset | null | undefined,
): string | null {
  if (!clock) return null;
  const kinds: [string, string | null][] = [
    ["PHD2", clock.phd2],
    ["ASIAIR", clock.asiair],
    ["NINA", clock.nina],
  ];
  const what = kinds
    .filter((kind): kind is [string, string] => kind[1] !== null)
    .map(([name, offset]) => `${name} ${offset}`)
    .join(", ");
  if (what === "") return null;
  return t(clock.detected ? "logs.clockFound" : "logs.clockGiven", { what });
}

/** The lines of « Rapport envoyé », each with its tick: what was read, the
 * quality and the guiding, what it replaces, the night's sky value. */
export function sentLines(
  t: Translate,
  locale: string,
  analysis: NightAnalysis,
  sent: NightAnalysisSent,
  previousAnalysisAt: string | null | undefined,
): string[] {
  const report = analysis.report;
  const lines: string[] = [];
  const read: string[] = [];
  if (report.acquisitionSteps.length > 0) {
    read.push(
      pluralText(t, locale, "logs.steps", report.acquisitionSteps.length),
    );
  }
  const types = new Set(report.timeline.events.map((event) => event.type));
  if (types.has("autofocus") || types.has("autofocus-result")) {
    read.push(t("logs.focus"));
  }
  if (types.has("meridian-flip")) read.push(t("logs.flip"));
  if (read.length > 0) {
    lines.push(
      t(report.source.ninaFilesParsed > 0 ? "logs.readNina" : "logs.readLogs", {
        what: listText(t, read),
      }),
    );
  }
  const rms = nightGuidingRms(report);
  const quality =
    report.quality.length > 0
      ? pluralText(t, locale, "logs.quality", report.quality.length)
      : null;
  if (quality && rms !== null) {
    lines.push(
      t("logs.qualityAndGuiding", {
        quality,
        guiding: t("logs.guiding", { rms: twoDecimals(rms, locale) }),
      }),
    );
  } else if (quality) {
    lines.push(quality);
  } else if (rms !== null) {
    lines.push(t("logs.guidingAlone", { rms: twoDecimals(rms, locale) }));
  }
  if (sent.analysis?.replaced) {
    lines.push(
      t("logs.replaced", {
        date: previousAnalysisAt ? shortDay(previousAnalysisAt, locale) : "—",
      }),
    );
  } else if (sent.analysis) {
    lines.push(t("logs.first"));
  }
  const clock = logsClockLine(t, sent.logsUtcOffset ?? analysis.logsUtcOffset);
  if (clock) lines.push(clock);
  if (sent.skyMeasure.kind === "sent") {
    lines.push(
      t("logs.sky", {
        sqm: twoDecimals(sent.skyMeasure.stored.measure.sqm, locale),
      }),
    );
  }
  return lines;
}

/** « Nuit du lun. 5 oct. · Col des Étoiles ». */
export function sentNightLine(
  t: Translate,
  locale: string,
  eveningDate: string | null,
  place: string | null,
): string {
  return t("logs.sentNight", {
    day: eveningDate ? dayInSentence(eveningDate, locale) : "—",
    place: place ?? "—",
  });
}
