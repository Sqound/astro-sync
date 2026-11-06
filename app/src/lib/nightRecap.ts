// What the night's wizard applies when a step is left (the options are chosen on a
// step, and it is when the person goes to the next step that they are applied). The
// person chooses on the step -- a target from the catalogue, a form for a new one, a plan
// to join or to leave, a master to measure, the incidents ticked -- and the step's primary
// writes them, in a safe order:
//
//   « Cibles »       creations -> attachments -> plans   (« Continuer »)
//   « Traitements »  removals of a measure -> measures   (« Continuer »)
//   « Récapitulatif » the incidents, then the sending     (« Envoyer »)
//
// The choices are plain values with a reducer, kept by the wizard above its
// steps so that « Précédent » finds them again. A choice that is not applied
// is simply a row of the list that is not ticked: unticking is the undo.
// What was applied is kept as a report, which « Nuit envoyée » shows. Pure
// TypeScript, tested under `node --test`; `lib/nightApply.ts` runs it.

import type { TranslateParams } from "../i18n/locale.ts";
import type { NewPlanTarget, PersonalTargetCreate } from "./bridge";
import { pluralText } from "./written.ts";

type Translate = (key: string, params?: TranslateParams) => string;

/* ------------------------------- The choices ------------------------------ */

/** A target of « Cibles » that is not in the night yet: one of the catalogue,
 * or one the form of « Nouvelle cible » describes. A row of the list, with
 * « nouveau », whose case is the undo. */
export interface PendingTarget {
  /** `catalogue:<id>` or `new:<n>`. */
  key: string;
  kind: "catalogue" | "create";
  /** The id in the catalogue (`catalogue`); `null` for a new target. */
  targetId: string | null;
  designation: string;
  /** The form as it will be sent (`create`). */
  create: PersonalTargetCreate | null;
  ticked: boolean;
}

/** The plan « Plan multi-nuits » will put the night in: an existing one (a
 * night of another plan is moved), or a blank one made for a target of the
 * night. */
export type PlanPick = (
  | {
      kind: "attach";
      projectId: string;
      title: string;
      /** « 3 nuits · 11 h 08 · objectif 12 h ». */
      facts: string;
      /** The plan the night leaves by it, when it moves. */
      movesFromId: string | null;
      movesFromName: string | null;
    }
  | {
      kind: "create";
      target: NewPlanTarget;
      /** The plan's name in the list (the target's). */
      title: string;
    }
) & { ticked: boolean };

/** A measure of a master, to make when « Traitements » is left: the file a
 * person stacked elsewhere, or the image the device stacked. One per
 * target (the figures kept in the target's folder are one set). */
export interface MeasurePick {
  targetId: string;
  folderName: string;
  designation: string;
  source: "master" | "device";
  /** The file to measure. */
  file: string;
  fileName: string;
  ticked: boolean;
}

/** The measure kept for a target that the person asks to forget. */
export interface ForgetPick {
  targetId: string;
  folderName: string;
  designation: string;
}

/** What the wizard did when a step was left, for « Nuit envoyée ». */
export type ReportItem =
  | { kind: "target"; how: "added" | "created"; name: string }
  | { kind: "plan"; how: "attach" | "create"; name: string }
  | { kind: "detach"; name: string }
  | {
      kind: "measure";
      source: "master" | "device";
      target: string;
      /** The figures the measure came to, said. */
      figures: string;
    }
  | { kind: "forget"; target: string }
  | {
      kind: "incidents";
      /** Incidents written: added, or replacing a signalement. */
      added: number;
      rejected: number;
      refused: boolean;
    };

export interface NightChoices {
  targets: PendingTarget[];
  plan: PlanPick | null;
  /** The plans the night leaves (only the ticked ones are kept). */
  detach: { projectId: string; title: string }[];
  /** By target. */
  measures: Record<string, MeasurePick>;
  forget: ForgetPick[];
  report: ReportItem[];
}

export const NO_CHOICES: NightChoices = {
  targets: [],
  plan: null,
  detach: [],
  measures: {},
  forget: [],
  report: [],
};

export type NightAction =
  /** « Ajouter X à la nuit » on a card of the catalogue. */
  | { type: "pickCatalogue"; targetId: string; designation: string }
  /** « Nouvelle cible » recorded from its form. */
  | { type: "pickNew"; create: PersonalTargetCreate }
  | { type: "tickTarget"; key: string; on: boolean }
  | { type: "pickPlan"; pick: DistributiveOmit<PlanPick, "ticked"> }
  | { type: "tickPlan"; on: boolean }
  | { type: "tickDetach"; projectId: string; title: string; on: boolean }
  | { type: "pickMeasure"; pick: Omit<MeasurePick, "ticked"> }
  | { type: "tickMeasure"; targetId: string; on: boolean }
  | { type: "tickForget"; pick: ForgetPick; on: boolean }
  /** What the primary applied: the choice leaves the lists, the report
   * keeps it. */
  | { type: "targetApplied"; key: string }
  | { type: "planApplied" }
  | { type: "detachApplied"; projectId: string; name: string }
  | { type: "measureApplied"; targetId: string; figures: string }
  | { type: "forgetApplied"; targetId: string }
  | {
      type: "incidentsApplied";
      item: Extract<ReportItem, { kind: "incidents" }>;
    };

type DistributiveOmit<T, K extends PropertyKey> = T extends unknown
  ? Omit<T, K>
  : never;

export function nightReducer(
  state: NightChoices,
  action: NightAction,
): NightChoices {
  switch (action.type) {
    case "pickCatalogue": {
      const key = `catalogue:${action.targetId}`;
      return state.targets.some((target) => target.key === key)
        ? withTarget(state, key, true)
        : {
            ...state,
            targets: [
              ...state.targets,
              {
                key,
                kind: "catalogue",
                targetId: action.targetId,
                designation: action.designation,
                create: null,
                ticked: true,
              },
            ],
          };
    }
    case "pickNew": {
      const used = state.targets
        .filter((target) => target.kind === "create")
        .map((target) => Number(target.key.slice("new:".length)));
      const n = used.length === 0 ? 1 : Math.max(...used) + 1;
      return {
        ...state,
        targets: [
          ...state.targets,
          {
            key: `new:${n}`,
            kind: "create",
            targetId: null,
            designation: action.create.name,
            create: action.create,
            ticked: true,
          },
        ],
      };
    }
    case "tickTarget":
      return withTarget(state, action.key, action.on);
    case "pickPlan":
      return { ...state, plan: { ...action.pick, ticked: true } as PlanPick };
    case "tickPlan":
      return state.plan
        ? { ...state, plan: { ...state.plan, ticked: action.on } }
        : state;
    case "tickDetach": {
      const rest = state.detach.filter(
        (item) => item.projectId !== action.projectId,
      );
      return {
        ...state,
        detach: action.on
          ? [...rest, { projectId: action.projectId, title: action.title }]
          : rest,
      };
    }
    case "pickMeasure":
      return {
        ...state,
        measures: {
          ...state.measures,
          [action.pick.targetId]: { ...action.pick, ticked: true },
        },
      };
    case "tickMeasure": {
      const held = state.measures[action.targetId];
      return held
        ? {
            ...state,
            measures: {
              ...state.measures,
              [action.targetId]: { ...held, ticked: action.on },
            },
          }
        : state;
    }
    case "tickForget": {
      const rest = state.forget.filter(
        (item) => item.targetId !== action.pick.targetId,
      );
      return { ...state, forget: action.on ? [...rest, action.pick] : rest };
    }
    case "targetApplied": {
      const done = state.targets.find((target) => target.key === action.key);
      if (!done) return state;
      return {
        ...state,
        targets: state.targets.filter((target) => target.key !== action.key),
        report: [
          ...state.report,
          {
            kind: "target",
            how: done.kind === "create" ? "created" : "added",
            name: done.designation,
          },
        ],
      };
    }
    case "planApplied": {
      const done = state.plan;
      if (!done) return state;
      return {
        ...state,
        plan: null,
        report: [
          ...state.report,
          { kind: "plan", how: done.kind, name: done.title },
        ],
      };
    }
    case "detachApplied":
      return {
        ...state,
        detach: state.detach.filter(
          (item) => item.projectId !== action.projectId,
        ),
        report: [...state.report, { kind: "detach", name: action.name }],
      };
    case "measureApplied": {
      const done = state.measures[action.targetId];
      if (!done) return state;
      const { [action.targetId]: _gone, ...rest } = state.measures;
      void _gone;
      return {
        ...state,
        measures: rest,
        report: [
          ...state.report,
          {
            kind: "measure",
            source: done.source,
            target: done.designation,
            figures: action.figures,
          },
        ],
      };
    }
    case "forgetApplied": {
      const done = state.forget.find(
        (item) => item.targetId === action.targetId,
      );
      if (!done) return state;
      return {
        ...state,
        forget: state.forget.filter(
          (item) => item.targetId !== action.targetId,
        ),
        report: [...state.report, { kind: "forget", target: done.designation }],
      };
    }
    case "incidentsApplied":
      return { ...state, report: [...state.report, action.item] };
  }
}

function withTarget(
  state: NightChoices,
  key: string,
  ticked: boolean,
): NightChoices {
  return {
    ...state,
    targets: state.targets.map((target) =>
      target.key === key ? { ...target, ticked } : target,
    ),
  };
}

/* ----------------------------- What « Cibles » applies --------------------- */

export interface CiblesPlan {
  /** Targets of the catalogue to add to the night. */
  adds: PendingTarget[];
  /** Targets to create, then add. */
  creates: PendingTarget[];
  /** The plan to join or to make; `null` when none is ticked. */
  plan: PlanPick | null;
  /** The plans to leave. */
  detach: { projectId: string; title: string }[];
}

/** What the primary of « Cibles » applies, in the order it applies it
 * (creations, then the additions of the catalogue, the plan, the plans left).
 * A plan the night leaves by joining another is not left a second time. */
export function ciblesPlan(choices: NightChoices): CiblesPlan {
  const plan = choices.plan?.ticked ? choices.plan : null;
  const moved = plan?.kind === "attach" ? plan.movesFromId : null;
  return {
    adds: choices.targets.filter((t) => t.ticked && t.kind === "catalogue"),
    creates: choices.targets.filter((t) => t.ticked && t.kind === "create"),
    plan,
    detach: choices.detach.filter((item) => item.projectId !== moved),
  };
}

export function ciblesWrite(plan: CiblesPlan): boolean {
  return (
    plan.adds.length > 0 ||
    plan.creates.length > 0 ||
    plan.plan !== null ||
    plan.detach.length > 0
  );
}

/** How many targets of the night « Continuer avec N cibles » counts: the
 * ticked rows of the night and the ticked ones that will join it. */
export function targetsCount(ticked: number, plan: CiblesPlan): number {
  return ticked + plan.adds.length + plan.creates.length;
}

/* --------------------------- What « Traitements » applies ------------------ */

export interface MeasuresPlan {
  /** The kept measures to forget (before the new ones). */
  forget: ForgetPick[];
  measures: MeasurePick[];
}

/** The measures asked: a measure kept and forgotten for a target that gets a
 * new one is not forgotten first (the new figures replace it). */
export function measuresPlan(choices: NightChoices): MeasuresPlan {
  const measures = Object.values(choices.measures).filter((m) => m.ticked);
  const measured = new Set(measures.map((m) => m.targetId));
  return {
    forget: choices.forget.filter((item) => !measured.has(item.targetId)),
    measures,
  };
}

export function measuresWrite(plan: MeasuresPlan): boolean {
  return plan.forget.length > 0 || plan.measures.length > 0;
}

/* --------------------------------- The sentences --------------------------- */

const upperFirst = (text: string, locale: string) =>
  text.charAt(0).toLocaleUpperCase(locale) + text.slice(1);

/** The parts of the sentence above the foot of « Cibles », in the order they
 * are applied. */
function ciblesParts(plan: CiblesPlan, t: Translate, locale: string): string[] {
  return [
    plan.creates.length > 0
      ? pluralText(
          t,
          locale,
          "wizard.recap.targets.created",
          plan.creates.length,
        )
      : null,
    plan.adds.length > 0
      ? pluralText(t, locale, "wizard.recap.targets.added", plan.adds.length)
      : null,
    plan.plan
      ? plan.plan.kind === "attach"
        ? t("wizard.recap.plan.attach", { name: plan.plan.title })
        : t("wizard.recap.plan.create", { target: plan.plan.title })
      : null,
    plan.detach.length > 0
      ? pluralText(t, locale, "wizard.recap.plan.detach", plan.detach.length, {
          list: plan.detach.map((item) => item.title).join(t("detect.join")),
        })
      : null,
  ].filter((part): part is string => part !== null);
}

/** « En continuant : 2 cibles ajoutées à la nuit, nuit rattachée au plan IC
 * 1805. », or `null` when the primary only goes on. */
export function ciblesLine(
  plan: CiblesPlan,
  t: Translate,
  locale: string,
): string | null {
  const parts = ciblesParts(plan, t, locale);
  return parts.length === 0
    ? null
    : t("wizard.recap.lead", { list: parts.join(t("detect.join")) });
}

function measureParts(plan: MeasuresPlan, t: Translate): string[] {
  return [
    ...plan.forget.map((item) =>
      t("wizard.recap.measure.forget", { target: item.designation }),
    ),
    ...plan.measures.map((item) =>
      t(
        item.source === "device"
          ? "wizard.recap.measure.device"
          : "wizard.recap.measure.master",
        { target: item.designation },
      ),
    ),
  ];
}

/** « En continuant : master de NGC 7000 mesuré. », or `null`. */
export function measuresLine(plan: MeasuresPlan, t: Translate): string | null {
  const parts = measureParts(plan, t);
  return parts.length === 0
    ? null
    : t("wizard.recap.lead", { list: parts.join(t("detect.join")) });
}

/** « En envoyant : 3 incidents ajoutés à la session. », or `null`. */
export function incidentsLine(
  count: number,
  t: Translate,
  locale: string,
): string | null {
  return count === 0
    ? null
    : t("wizard.recap.sendLead", {
        list: pluralText(t, locale, "wizard.recap.incidents", count),
      });
}

/* ---------------------------------- The report ----------------------------- */

/** « Cible NGC 7000 ajoutée à la nuit. »: what « Nuit envoyée » says was done
 * on the way, one line per thing, in the order it was done. */
export function reportLines(
  report: readonly ReportItem[],
  t: Translate,
  locale: string,
): string[] {
  return report.flatMap((item) => {
    switch (item.kind) {
      case "target":
        return [
          t(
            item.how === "created"
              ? "wizard.recap.report.created"
              : "wizard.recap.report.added",
            { name: item.name },
          ),
        ];
      case "plan":
        return [
          upperFirst(
            item.how === "attach"
              ? t("wizard.recap.plan.attach", { name: item.name })
              : t("wizard.recap.plan.create", { target: item.name }),
            locale,
          ),
        ];
      case "detach":
        return [
          upperFirst(
            pluralText(t, locale, "wizard.recap.plan.detach", 1, {
              list: item.name,
            }),
            locale,
          ),
        ];
      case "measure":
        return [
          [
            upperFirst(
              t(
                item.source === "device"
                  ? "wizard.recap.measure.device"
                  : "wizard.recap.measure.master",
                { target: item.target },
              ),
              locale,
            ),
            item.figures,
          ]
            .filter(Boolean)
            .join(t("detect.join")),
        ];
      case "forget":
        return [
          upperFirst(
            t("wizard.recap.measure.forget", { target: item.target }),
            locale,
          ),
        ];
      case "incidents": {
        const lines = [
          upperFirst(
            pluralText(t, locale, "wizard.recap.incidents", item.added),
            locale,
          ),
        ];
        if (item.rejected > 0) {
          lines[0] += ` ${pluralText(
            t,
            locale,
            "import.done.incidentsRejected",
            item.rejected,
          )}`;
        }
        if (item.refused) lines.push(t("import.done.incidentsRefused"));
        return lines;
      }
    }
  });
}
