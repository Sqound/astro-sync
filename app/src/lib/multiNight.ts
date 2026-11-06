// The multi-night plans in Astro Sync's screens (Treatments, Summary and Nights).
// Pure TypeScript, tested under `node --test`: what « Traitements » says of the
// night's integration and of the image of the plan, what « Récapitulatif »
// adds, the tag of « Nuits », the words of a refusal of the service. The
// right is the stacking's (`analysis.progression`, « Synchronisation
// avancée+ ») on a service that sells it; a private server never restricts
// the multi-night plans: only a service never heard closes them.

import type { TranslateParams } from "../i18n/locale.ts";
import type {
  FamilyPreview,
  IntegrationMaster,
  IntegrationPreview,
  IntegrationsSent,
  MultiNightGate,
  MultiNightView,
  NightProject,
  Pipeline,
  PlanFrame,
  PlanImageRun,
  PlanLine,
  PlanSource,
  PlanTag,
} from "./bridge";
import { loadPlanKept, type KeptStorage } from "./planKept.ts";
import { nightDay } from "./nights.ts";
import {
  inListOrder,
  pipelineName as chosenPipelineName,
  rendersOf,
  type TargetRenders,
} from "./pipelineChoice.ts";
import type { Seg } from "./wizardFiles.ts";
import {
  conjunction,
  formatMinutes,
  formatSize,
  pluralText,
} from "./written.ts";

type Translate = (key: string, params?: TranslateParams) => string;

/** What the person chose for the image of a plan. */
export interface PlanImageChoices {
  /** The option unticked. */
  off: boolean;
  /** The integrations unticked (their keys); all are ticked by default. */
  unticked: ReadonlySet<string>;
  /** The source taken for a night, by line key: the master Astro Sync kept
   * of its poses, or the image its device stacked. A night not named takes
   * its default (the master when one is kept). Kept per plan. */
  sources: Readonly<Record<string, PlanSource>>;
  /** The frame kept, the four integers of a notch;
   * `null`: the whole image. Kept per plan. */
  frame: PlanFrame | null;
}

export const DEFAULT_PLAN_IMAGE: PlanImageChoices = {
  off: false,
  unticked: new Set(),
  sources: {},
  frame: null,
};

/** The choices of a plan: what the window holds for it, else what was kept
 * for the plan on this computer (the source of each night, the frame). */
export function planChoicesOf(
  held: Readonly<Record<string, PlanImageChoices>> | undefined,
  projectId: string,
  storage: KeptStorage | null = null,
): PlanImageChoices {
  const found = held?.[projectId];
  if (found) return found;
  const kept = loadPlanKept(storage, projectId);
  return { ...DEFAULT_PLAN_IMAGE, sources: kept.sources, frame: kept.frame };
}

/** « 4 h 54 », « 4 h 00 », « 45 min »: an integration time, as the screens
 * write it (the minutes always said after the hours). */
export function integrationTime(t: Translate, seconds: number): string {
  const minutes = Math.max(0, Math.round(seconds / 60));
  if (minutes < 60) return formatMinutes(t, minutes);
  return t("common.hoursMinutes", {
    hours: Math.floor(minutes / 60),
    minutes: String(minutes % 60).padStart(2, "0"),
  });
}

/** « Dim. 9 août »: an evening, as the night's pages write it. */
export function eveningDay(eveningDate: string, locale: string): string {
  return nightDay(`${eveningDate}T12:00:00Z`, locale, "UTC");
}

/** « 9 août »: an evening, short. */
export function eveningShort(eveningDate: string, locale: string): string {
  const date = new Date(`${eveningDate}T12:00:00Z`);
  if (Number.isNaN(date.getTime())) return eveningDate;
  return new Intl.DateTimeFormat(locale, {
    day: "numeric",
    month: "long",
    timeZone: "UTC",
  }).format(date);
}

/** Whether the multi-night plans are open here. */
export function multiNightOpen(
  gate: MultiNightGate | null | undefined,
): boolean {
  return gate?.allowed ?? false;
}

/** The projects of the night a ticked target works for. */
export function nightProjects(
  view: MultiNightView | null | undefined,
  ticked: (targetId: string) => boolean = () => true,
): { project: NightProject; lines: PlanLine[] }[] {
  return (view?.projects ?? []).filter((item) => ticked(item.project.targetId));
}

/** « IC 1805 — Nébuleuse du Cœur ». */
export function planName(project: NightProject): string {
  return project.commonName
    ? `${project.name} — ${project.commonName}`
    : project.name;
}

/* ------------------------- « Intégration de la nuit » --------------------- */

function number(value: number, locale: string): string {
  return new Intl.NumberFormat(locale, { maximumFractionDigits: 1 }).format(
    value,
  );
}

/** « UV/IR-cut · 120 × 120 s · gain 100 · bin 1 · −10 °C ». */
export function familyText(
  t: Translate,
  locale: string,
  family: Pick<
    FamilyPreview,
    | "filter"
    | "exposureSeconds"
    | "gain"
    | "binning"
    | "temperatureC"
    | "frameCount"
  >,
): string {
  const parts = [family.filter ?? t("multiNight.noFilter")];
  if (family.exposureSeconds !== null) {
    parts.push(
      t("multiNight.framesTimes", {
        count: family.frameCount,
        seconds: number(family.exposureSeconds, locale),
      }),
    );
  }
  if (family.gain !== null) {
    parts.push(t("multiNight.gain", { gain: number(family.gain, locale) }));
  }
  if (family.binning !== null) {
    parts.push(
      t("multiNight.binning", {
        binning: String(family.binning).split(/x/i)[0],
      }),
    );
  }
  if (family.temperatureC !== null) {
    parts.push(
      t("multiNight.temperature", {
        value: number(family.temperatureC, locale).replace("-", "−"),
      }),
    );
  }
  return parts.join(" · ");
}

/** One target's « Intégration de la nuit, par famille ». */
export interface TargetIntegrationView {
  targetId: string;
  families: { text: string; duration: string; master: string | null }[];
  /** « 1 pose hors de la nuit du 12 août, écartée de l'intégration … ». */
  outside: string | null;
}

/** What « Intégration de la nuit » says under each target. */
export function targetIntegrationView(
  t: Translate,
  locale: string,
  input: {
    targetId: string;
    targetFolder: string;
    preview: IntegrationPreview;
    eveningDate: string | null;
  },
): TargetIntegrationView {
  const { preview } = input;
  return {
    targetId: input.targetId,
    families: preview.families.map((family) => ({
      text: familyText(t, locale, family),
      duration: integrationTime(t, family.totalSeconds),
      master: family.master
        ? t("multiNight.masterKept", {
            path: `${input.targetFolder}/stacked/${family.label}_integration.fit`,
            size: formatSize(t, family.master.bytes, locale),
          })
        : t("multiNight.masterWillBeKept", {
            path: `${input.targetFolder}/stacked/`,
          }),
    })),
    outside:
      preview.outside > 0
        ? pluralText(t, locale, "multiNight.outside", preview.outside, {
            date: input.eveningDate
              ? eveningShort(input.eveningDate, locale)
              : "—",
          })
        : null,
  };
}

/* ------------------------- « Image du plan multi-nuits » ------------------ */

/** The source column of a night. */
export interface PlanSourceView {
  /** `choice`: the master and the image of the device are both here (two
   * buttons); `deviceOnly`: the device's image is all there is; `here`: the
   * poses only, the night is stacked here. */
  kind: "choice" | "deviceOnly" | "here";
  /** What the job takes for the night. */
  taken: PlanSource;
  /** « Les poses sont ici : on repart d'elles, par défaut. » */
  note: string;
}

export interface PlanLineView {
  key: string;
  segs: Seg[];
  duration: string;
  ticked: boolean;
  /** On another computer: listed, never ticked, never combined. */
  disabled: boolean;
  place: PlanLine["place"];
  /** `null` while no night of the plan has an image of its device: the
   * lines are what they were. */
  source: PlanSourceView | null;
}

export interface PlanGroupView {
  label: string;
  lines: PlanLineView[];
}

export interface PlanOptionView {
  projectId: string;
  /** `available`, `reserved` (no right), `private` (a private server). */
  state: "available" | "reserved" | "unknown";
  ticked: boolean;
  line: string;
  groups: PlanGroupView[];
  /** « Combinées : Bibande 4 h 54 · RGB 2 h 14 · 7 h 08 sur 2 nuits ». */
  combined: string | null;
  /** « Non combinée : UV/IR-cut 4 h 00 (décochée). » */
  notCombined: string | null;
  /** One night only: the image of the plan is that night's. */
  oneNight: string | null;
  /** « L'intégration du 10 août est sur un autre ordinateur. » */
  otherComputer: { line: string; details: string } | null;
  /** Without the right: « Connues : … · 3 nuits · 11 h 08 ». */
  known: string | null;
  /** « L'image est faite avec les pipelines choisis pour IC 1805 : … »:
   * the pipelines of the plan's target; `null` when no image is
   * made. */
  pipelinesLine: string | null;
  /** « La nuit du 11 octobre entre avec l'image de l'appareil, telle quelle
   * … »: the nights taken from their device's image; `null` when none is. */
  deviceWarning: string | null;
  /** « Pourquoi pas l'image de l'appareil partout ? … »: said with the
   * warning. */
  deviceWhy: string | null;
  whereItGoes: string;
  /** Without the right: why, in one line. */
  closedLine: string | null;
  offers: boolean;
}

/** The label of a group: « Bibande (Hα + OIII) », else the filter as the
 * frames name it. */
export function groupLabel(
  t: Translate,
  line: Pick<PlanLine, "band" | "filter">,
): string {
  if (line.band === "dualband") return t("multiNight.band.dualband");
  return line.filter ?? t("multiNight.noFilter");
}

/** The source a line is taken from: the one asked when it is here, else the
 * master when one is kept, else the image of the device. */
export function sourceOf(
  line: PlanLine,
  choices: Pick<PlanImageChoices, "sources">,
): PlanSource {
  const asked = choices.sources[line.key];
  if (asked === "device" && line.device) return "device";
  if (asked === "master" && line.master !== null) return "master";
  return line.source;
}

/** Whether something here can be combined for the line: a master kept, or the
 * image of the device. */
export function combinable(line: PlanLine): boolean {
  return (
    line.place !== "otherComputer" &&
    (line.master !== null || line.device !== undefined)
  );
}

/** The line as it is taken: the totals of the device's image when it is the
 * source (its header says its poses and its time). */
function taking(line: PlanLine, choices: PlanImageChoices): PlanLine {
  if (sourceOf(line, choices) !== "device" || !line.device) return line;
  return {
    ...line,
    source: "device",
    frameCount: line.device.frameCount,
    totalSeconds: line.device.totalSeconds,
    bytes: line.device.bytes,
  };
}

/** The lines ticked that can be combined: what is combined, each as its
 * source gives it. */
export function combinedLines(
  lines: readonly PlanLine[],
  choices: PlanImageChoices,
): PlanLine[] {
  return lines
    .filter((line) => combinable(line) && !choices.unticked.has(line.key))
    .map((line) => taking(line, choices));
}

/** The nights of some lines, once each. */
function nightsOf(lines: readonly PlanLine[]): string[] {
  return [...new Set(lines.map((line) => line.nightId))];
}

/** The reference: the longest integration among the lines, and when the plan
 * mixes dual-band and broadband ones the longest **without** a dual-band
 * filter (its frame is the plan's: `plan_image::reference_of`); a tie goes
 * to the earliest evening. */
export function referenceLine(lines: readonly PlanLine[]): PlanLine | null {
  const mixed =
    lines.some((line) => line.band === "dualband") &&
    lines.some((line) => line.band !== "dualband");
  return lines
    .filter((line) => !mixed || line.band !== "dualband")
    .reduce<PlanLine | null>((best, line) => {
      if (!best || line.totalSeconds > best.totalSeconds) return line;
      if (
        line.totalSeconds === best.totalSeconds &&
        line.eveningDate < best.eveningDate
      ) {
        return line;
      }
      return best;
    }, null);
}

/** The source asked for each night that has both (what `plan_image_run`
 * takes): only the nights the person chose, and where the source is here. */
export function chosenSources(
  lines: readonly PlanLine[],
  choices: PlanImageChoices,
): Record<string, PlanSource> {
  const out: Record<string, PlanSource> = {};
  for (const line of lines) {
    const asked = choices.sources[line.key];
    if (!asked || !combinable(line)) continue;
    if (sourceOf(line, choices) === asked) out[line.key] = asked;
  }
  return out;
}

/**
 * The pipelines of the image of a plan: those chosen for the
 * plan's target in « Traitements » (`targetRenders`), else the target's
 * defaults -- « Rendu naturel », and « Correction bibande » as soon as a
 * dual-band integration is ticked. One image is made with each.
 */
export function planPipelines(
  lines: readonly PlanLine[],
  choices: PlanImageChoices,
  targetRenders: TargetRenders,
  targetId: string,
  pipelines: readonly Pipeline[],
): string[] {
  const band = combinedLines(lines, choices).some(
    (line) => line.band === "dualband",
  )
    ? "dualband"
    : "broadband";
  return inListOrder(
    rendersOf(targetRenders, targetId, band, pipelines),
    pipelines,
  );
}

/** « Rendu naturel et Correction bibande »: the pipelines, named. */
function pipelinesText(
  t: Translate,
  locale: string,
  pipelines: readonly Pipeline[],
  ids: readonly string[],
): string {
  const names = ids.map((id) => chosenPipelineName(t, pipelines, id, locale));
  return conjunction(locale, names);
}

/** « Bibande 4 h 54 · RGB 2 h 14 »: each group's total. */
function byGroup(t: Translate, lines: readonly PlanLine[]): string {
  const totals = new Map<string, number>();
  for (const line of lines) {
    const label = groupLabel(t, line).replace(/ \(.*\)$/, "");
    totals.set(label, (totals.get(label) ?? 0) + line.totalSeconds);
  }
  return [...totals]
    .map(([label, seconds]) => `${label} ${integrationTime(t, seconds)}`)
    .join(" · ");
}

/** « Image du plan multi-nuits » for one plan of the night. */
export function planOptionView(
  t: Translate,
  locale: string,
  input: {
    project: NightProject;
    lines: readonly PlanLine[];
    gate: MultiNightGate;
    choices: PlanImageChoices;
    pipelines: readonly Pipeline[];
    /** The pipelines chosen for each target of the night. */
    targetRenders: TargetRenders;
  },
): PlanOptionView {
  const { project, lines, gate, choices } = input;
  const state: PlanOptionView["state"] = gate.allowed
    ? "available"
    : gate.closed === "unknown"
      ? "unknown"
      : "reserved";
  const open = state === "available";
  const combined = combinedLines(lines, choices);
  const reference = referenceLine(combined);
  const nights = nightsOf(lines);
  const groups: PlanGroupView[] = [];
  for (const line of lines) {
    const label = groupLabel(t, line);
    let group = groups.find((item) => item.label === label);
    if (!group) {
      group = { label, lines: [] };
      groups.push(group);
    }
    const disabled = line.place === "otherComputer" || !open;
    const taken = sourceOf(line, choices);
    const withDevice = lines.some((item) => item.device !== undefined);
    const source: PlanSourceView | null =
      withDevice && line.place !== "otherComputer"
        ? line.master !== null && line.device
          ? {
              kind: "choice",
              taken,
              note: t(
                taken === "master"
                  ? "multiNight.source.restackNote"
                  : "multiNight.source.deviceNote",
              ),
            }
          : line.device
            ? {
                kind: "deviceOnly",
                taken: "device",
                note: t("multiNight.source.deviceOnlyNote"),
              }
            : {
                kind: "here",
                taken: "master",
                note: t("multiNight.source.posesOnlyNote"),
              }
        : null;
    const detail = [
      eveningDay(line.eveningDate, locale),
      line.exposureSeconds !== null
        ? t("multiNight.framesTimes", {
            count: line.frameCount,
            seconds: number(line.exposureSeconds, locale),
          })
        : null,
      open && reference && line.key === reference.key
        ? t("multiNight.reference")
        : null,
    ]
      .filter(Boolean)
      .join(" · ");
    const place =
      line.place === "thisNight"
        ? t("multiNight.place.thisNight")
        : source?.kind === "deviceOnly"
          ? t("multiNight.source.deviceHere")
          : line.place === "thisComputer"
            ? t("multiNight.place.thisComputer")
            : line.computerLabel
              ? t("multiNight.place.otherComputer", {
                  computer: line.computerLabel,
                })
              : t("multiNight.place.otherComputerUnnamed");
    group.lines.push({
      key: line.key,
      segs:
        source?.kind === "deviceOnly" && line.place !== "thisNight"
          ? [
              detail,
              " ",
              { strong: place },
              " · ",
              t("multiNight.source.deviceHeader"),
            ]
          : [detail, " ", { strong: place }],
      duration: integrationTime(t, line.totalSeconds),
      ticked:
        open &&
        line.place !== "otherComputer" &&
        !choices.unticked.has(line.key),
      disabled,
      place: line.place,
      source,
    });
  }
  const combinedNights = nightsOf(combined);
  const fromDevice = combined.filter((line) => line.source === "device");
  const unticked = lines.filter(
    (line) => line.place !== "otherComputer" && choices.unticked.has(line.key),
  );
  const elsewhere = lines.find((line) => line.place === "otherComputer");
  const pipelineIds = planPipelines(
    lines,
    choices,
    input.targetRenders,
    project.targetId,
    input.pipelines,
  );
  const total = combined.reduce((sum, line) => sum + line.totalSeconds, 0);
  return {
    projectId: project.projectId,
    state,
    ticked: open && !choices.off,
    line: t("multiNight.planImage.line", {
      name: planName(project),
      nights: pluralText(t, locale, "multiNight.nights", nights.length),
    }),
    groups,
    combined:
      open && combinedNights.length > 1
        ? t("multiNight.combined", {
            list: byGroup(t, combined),
            total: integrationTime(t, total),
            nights: pluralText(
              t,
              locale,
              "multiNight.nights",
              combinedNights.length,
            ),
          })
        : null,
    notCombined:
      open && unticked.length > 0
        ? t("multiNight.notCombined", { list: byGroup(t, unticked) })
        : null,
    oneNight:
      open && combinedNights.length === 1
        ? t("multiNight.oneNight", {
            list: byGroup(t, combined),
            date: eveningShort(combined[0].eveningDate, locale),
          })
        : null,
    otherComputer:
      open && elsewhere
        ? {
            line: t("multiNight.otherComputer", {
              date: eveningShort(elsewhere.eveningDate, locale),
            }),
            details: t("multiNight.howToCombineDetails", {
              date: eveningShort(elsewhere.eveningDate, locale),
            }),
          }
        : null,
    known: open
      ? null
      : t("multiNight.known", {
          list: byGroup(t, lines),
          nights: pluralText(t, locale, "multiNight.nights", nights.length),
          total: integrationTime(
            t,
            lines.reduce((sum, line) => sum + line.totalSeconds, 0),
          ),
        }),
    // The pipelines of an image that will be made: two nights or more.
    pipelinesLine:
      open && combinedNights.length > 1
        ? t("multiNight.planImage.pipelines", {
            name: project.name,
            list: pipelinesText(t, locale, input.pipelines, pipelineIds),
          })
        : null,
    deviceWarning:
      open && fromDevice.length > 0 && combinedNights.length > 1
        ? pluralText(t, locale, "multiNight.source.warn", fromDevice.length, {
            dates: conjunction(
              locale,
              fromDevice.map((line) => eveningShort(line.eveningDate, locale)),
            ),
          })
        : null,
    deviceWhy:
      open && fromDevice.length > 0 && combinedNights.length > 1
        ? t("multiNight.source.why")
        : null,
    whereItGoes: t("multiNight.whereItGoes", { name: project.name }),
    closedLine:
      state === "reserved"
        ? t("multiNight.reserved")
        : state === "unknown"
          ? t("multiNight.unknownService")
          : null,
    offers: state === "reserved",
  };
}

/** The keys of the integrations combined (what `plan_image_run` takes). */
export function chosenKeys(
  lines: readonly PlanLine[],
  choices: PlanImageChoices,
): string[] {
  return combinedLines(lines, choices).map((line) => line.key);
}

/** Whether the image of the plan will be made: open, ticked, two nights. */
export function planImageWanted(
  gate: MultiNightGate,
  lines: readonly PlanLine[],
  choices: PlanImageChoices,
): boolean {
  return (
    gate.allowed &&
    !choices.off &&
    nightsOf(combinedLines(lines, choices)).length > 1
  );
}

/* ------------------------------ « Récapitulatif » -------------------------- */

export interface MultiNightSummary {
  /** Rows of « Envoyé à votre espace ». */
  sent: { segs: Seg[]; value: string }[];
  /** Rows of « Ne quitte jamais cet ordinateur ». */
  never: { segs: Seg[]; value: string }[];
  /** The line of « Vérification avant l'envoi »: the right, or why not. */
  check: { ok: boolean; text: string; offers: boolean } | null;
}

/** What « Récapitulatif » adds for the multi-night plans of the night. */
export function multiNightSummary(
  t: Translate,
  locale: string,
  input: {
    view: MultiNightView | null;
    /** The masters kept in this night's folder. */
    masters: readonly IntegrationMaster[];
    /** The night's integration runs (the stacking is ticked). */
    integrating: boolean;
    planImage: Readonly<Record<string, PlanImageChoices>>;
    pipelines: readonly Pipeline[];
    targetRenders: TargetRenders;
    /** The size of the frame kept for the image of each plan, by plan, when it
     * is not the whole image. */
    framed?: Readonly<Record<string, { width: number; height: number } | null>>;
  },
): MultiNightSummary {
  const summary: MultiNightSummary = { sent: [], never: [], check: null };
  const projects = input.view?.projects ?? [];
  if (!input.view || projects.length === 0) return summary;
  const gate = input.view.gate;
  const here = t("wizard.summary.computedHere");
  const herePlural = t("wizard.summary.computedHerePlural");
  for (const { project, lines } of projects) {
    const choices = planChoicesOf(input.planImage, project.projectId);
    const dates = [...new Set(lines.map((line) => line.eveningDate))]
      .sort()
      .map((date) => eveningShort(date, locale));
    if (gate.allowed) {
      const thisNight = lines.filter((line) => line.place === "thisNight");
      if (input.integrating || thisNight.length > 0) {
        summary.sent.push({
          segs: [
            t("multiNight.summary.nightIntegration", {
              target: project.name,
              list:
                thisNight.length > 0
                  ? thisNight
                      .map((line) =>
                        [
                          groupLabel(t, line),
                          line.exposureSeconds !== null
                            ? t("multiNight.framesTimes", {
                                count: line.frameCount,
                                seconds: number(line.exposureSeconds, locale),
                              })
                            : null,
                          integrationTime(t, line.totalSeconds),
                        ]
                          .filter(Boolean)
                          .join(" · "),
                      )
                      .join(" ; ")
                  : t("multiNight.summary.afterStacking"),
            }),
          ],
          value: here,
        });
      }
      summary.sent.push({
        segs: [
          pluralText(
            t,
            locale,
            "multiNight.summary.planIntegrations",
            lines.length,
            { name: project.name, dates: dates.join(", ") },
          ),
        ],
        value: herePlural,
      });
      if (planImageWanted(gate, lines, choices)) {
        const combined = combinedLines(lines, choices);
        const ids = planPipelines(
          lines,
          choices,
          input.targetRenders,
          project.targetId,
          input.pipelines,
        );
        const framed = input.framed?.[project.projectId] ?? null;
        summary.sent.push({
          segs: [
            t("multiNight.summary.planImage", {
              name: project.name,
              render: pipelinesText(t, locale, input.pipelines, ids),
              list: byGroup(t, combined),
            }),
            ...(framed
              ? [
                  " · ",
                  {
                    strong: t("multiNight.summary.cropped", {
                      width: new Intl.NumberFormat(locale).format(framed.width),
                      height: new Intl.NumberFormat(locale).format(
                        framed.height,
                      ),
                    }),
                  },
                ]
              : []),
          ],
          value: here,
        });
      }
      summary.check = {
        ok: true,
        text: t("multiNight.summary.checkActive", { name: project.name }),
        offers: false,
      };
    } else {
      summary.sent.push(
        {
          segs: [
            t("multiNight.summary.notSent", {
              name: project.name,
            }),
          ],
          value: "—",
        },
        {
          segs: [t("multiNight.summary.notMade", { name: project.name })],
          value: "—",
        },
      );
      summary.check = {
        ok: false,
        text:
          gate.closed === "unknown"
            ? t("multiNight.summary.checkUnknown")
            : t("multiNight.summary.checkReserved"),
        offers: gate.closed !== "unknown",
      };
    }
    const kept = lines.filter((line) => line.master !== null);
    if (kept.length > 0) {
      const keptDates = [...new Set(kept.map((line) => line.eveningDate))]
        .sort()
        .map((date) => eveningShort(date, locale));
      summary.never.push({
        segs: [
          pluralText(t, locale, "multiNight.summary.masters", kept.length, {
            dates: keptDates.join(", "),
          }),
        ],
        value: formatSize(
          t,
          kept.reduce((sum, line) => sum + line.bytes, 0),
          locale,
        ),
      });
    }
  }
  if (summary.never.length === 0 && input.masters.length > 0) {
    summary.never.push({
      segs: [
        pluralText(
          t,
          locale,
          "multiNight.summary.mastersNight",
          input.masters.length,
        ),
      ],
      value: formatSize(
        t,
        input.masters.reduce((sum, master) => sum + master.bytes, 0),
        locale,
      ),
    });
  }
  return summary;
}

/** « Masters des intégrations : 1, gardé dans stacked/ » on a night of no
 * plan (the masters are always kept). */
export function nightMastersRow(
  t: Translate,
  locale: string,
  masters: readonly IntegrationMaster[],
): { segs: Seg[]; value: string } | null {
  if (masters.length === 0) return null;
  return {
    segs: [
      pluralText(t, locale, "multiNight.summary.mastersNight", masters.length),
    ],
    value: formatSize(
      t,
      masters.reduce((sum, master) => sum + master.bytes, 0),
      locale,
    ),
  };
}

/* --------------------------------- The sending ---------------------------- */

/** The words of a refusal of the service, in the person's language. */
export function refusalWords(
  t: Translate,
  code: string,
  message: string,
): string {
  switch (code) {
    case "multi_night_not_active":
      return t("multiNight.refusal.notActive");
    case "integration_outside_night":
      return t("multiNight.refusal.outsideNight");
    case "project_not_found":
      return t("multiNight.refusal.projectNotFound");
    // Attaching a night, detaching it, creating a plan.
    case "target_mismatch":
      return t("multiNight.refusal.targetMismatch");
    case "capability_required":
      return t("multiNight.refusal.capabilityRequired");
    case "session_not_found":
    case "plan_not_found":
    case "night_not_found":
      return t("multiNight.refusal.nightNotFound");
    default:
      return t("multiNight.refusal.other", { message });
  }
}

/** What the sending says of the multi-night plans: one line each, a
 * refusal said in clear (never hidden, never an error of the night). */
export function sentMultiNightLines(
  t: Translate,
  locale: string,
  input: {
    projects: readonly NightProject[];
    integrations: readonly IntegrationsSent[];
    /** The images made of each plan (one per pipeline of its target). */
    planImage: Readonly<Record<string, PlanImageRun>>;
  },
): { ok: string[]; notes: string[] } {
  const name = (projectId: string) =>
    input.projects.find((project) => project.projectId === projectId)?.name ??
    projectId;
  const ok: string[] = [];
  const notes: string[] = [];
  for (const sent of input.integrations) {
    if (sent.result === "sent") {
      ok.push(
        pluralText(t, locale, "multiNight.sent.integrations", sent.lines, {
          name: name(sent.projectId),
        }),
      );
    } else if (sent.result === "refused") {
      notes.push(
        t("multiNight.refusedLine", {
          name: name(sent.projectId),
          words: refusalWords(t, sent.code, sent.message),
        }),
      );
    }
  }
  for (const [projectId, run] of Object.entries(input.planImage)) {
    if (run.kind === "made") {
      ok.push(
        t("multiNight.sent.planImage", {
          name: name(projectId),
          nights: pluralText(
            t,
            locale,
            "multiNight.nights",
            run.outcomes[0]?.combinedNightIds.length ?? 0,
          ),
        }),
      );
    }
  }
  return { ok, notes };
}

/* ----------------------------------- « Nuits » ---------------------------- */

/** « Plan multi-nuits IC 1805 · 3 nuits · 11 h 08 », or `null`. */
export function nightTag(
  t: Translate,
  locale: string,
  tags: readonly PlanTag[] | undefined,
): string | null {
  const tag = tags?.[0];
  if (!tag) return null;
  // A night attached by itself has no integration yet: no time to say.
  if (tag.totalSeconds <= 0) {
    return t("multiNight.tagNoTime", {
      name: tag.name,
      nights: pluralText(t, locale, "multiNight.nights", tag.nightCount),
    });
  }
  return t("multiNight.tag", {
    name: tag.name,
    nights: pluralText(t, locale, "multiNight.nights", tag.nightCount),
    total: integrationTime(t, tag.totalSeconds),
  });
}
