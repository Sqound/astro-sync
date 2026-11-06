// Attaching a night to a multi-night plan, in the wizard's « Cibles » step:
// the « Plan multi-nuits » block, the window of
// the plans of the night's target with its search, the confirmation bar and
// its « Annuler ». Pure TypeScript (no React, no Tauri), tested under
// `node --test`; `screens/night/PlanAttach.tsx` draws it.
//
// The right is the plan's own (`planner.mosaic`, « Planification avancée »,
// bought once or held by either subscription) on the
// hosted service (a private server never restricts it); held
// back, the block says so and offers « Voir les offres ». « Synchronisation avancée+ » alone does not
// attach; it keeps the integrations and the image of the plan. Reading never
// needs the right: a night already in a plan always says so.

import type { TranslateParams } from "../i18n/locale.ts";
import type {
  AttachedPlan,
  NewPlanTarget,
  NightAttachment,
  PlanChoice,
  PlanChoices,
} from "./bridge";
import { integrationTime, refusalWords } from "./multiNight.ts";
import type { ReportStorage } from "./lastReport.ts";
import { formatMinutes, pluralText } from "./written.ts";

type Translate = (key: string, params?: TranslateParams) => string;

/** « IC 1805 — Nébuleuse du Cœur ». */
export function planTitle(plan: {
  name: string;
  commonName: string | null;
}): string {
  return plan.commonName ? `${plan.name} — ${plan.commonName}` : plan.name;
}

/** « 3 nuits · 11 h 08 · objectif 12 h »: what a plan holds. */
export function planFacts(
  t: Translate,
  locale: string,
  plan: {
    nightCount: number;
    totalSeconds: number;
    goalMinutes?: number | null;
  },
): string {
  const parts = [pluralText(t, locale, "multiNight.nights", plan.nightCount)];
  if (plan.totalSeconds > 0) parts.push(integrationTime(t, plan.totalSeconds));
  if (plan.goalMinutes) {
    parts.push(
      t("planAttach.goal", { goal: formatMinutes(t, plan.goalMinutes) }),
    );
  }
  return parts.join(" · ");
}

/* --------------------------------- The block ------------------------------- */

/** What the block reads: the answer, a failed read, the last answer kept. */
export interface BlockInput {
  attachment: NightAttachment | null;
  failed: boolean;
  /** The last answer read for this night (the offline state). */
  cached: NightAttachment | null;
}

/** The sentence that replaces the link when the right is missing, and
 * whether « Voir les offres » follows. */
export interface ClosedLine {
  text: string;
  offers: boolean;
}

export type BlockView =
  | { kind: "loading" }
  /** The read failed and nothing is known of the night: the phrase and
   * « Réessayer ». */
  | { kind: "failed"; text: string }
  /** The night is in no plan. */
  | { kind: "none"; text: string; attach: boolean; closed: ClosedLine | null }
  /** The night is in a plan (at least one); `detach` when it can be left. */
  | {
      kind: "attached";
      plans: { projectId: string; title: string; facts: string }[];
      detach: boolean;
      closed: ClosedLine | null;
      /** « Hors ligne … »: what is shown is the last answer. */
      offline: string | null;
    };

/** The line that stands for the link when attaching is closed here. */
export function closedLine(
  t: Translate,
  closed: NightAttachment["gate"]["closed"],
): ClosedLine | null {
  if (closed === null) return null;
  if (closed === "unknown") {
    return { text: t("multiNight.unknownService"), offers: false };
  }
  return { text: t("planAttach.reserved"), offers: true };
}

/** The « Plan multi-nuits » block of « Cibles ». */
export function blockView(
  t: Translate,
  locale: string,
  input: BlockInput,
): BlockView {
  const shown = input.attachment ?? (input.failed ? input.cached : null);
  if (!shown) {
    return input.failed
      ? { kind: "failed", text: t("planAttach.readFailed") }
      : { kind: "loading" };
  }
  const offline = input.attachment === null;
  const closed = offline ? null : closedLine(t, shown.gate.closed);
  if (shown.plans.length === 0) {
    return {
      kind: "none",
      text: t("planAttach.none"),
      attach: !offline && closed === null,
      closed,
    };
  }
  return {
    kind: "attached",
    plans: shown.plans.map((plan: AttachedPlan) => ({
      projectId: plan.projectId,
      title: planTitle(plan),
      facts: planFacts(t, locale, plan),
    })),
    detach: !offline && closed === null,
    closed,
    offline: offline ? t("planAttach.offline.block") : null,
  };
}

/* -------------------------------- The window ------------------------------- */

/** One plan of the window. */
export interface ChoiceView {
  projectId: string;
  title: string;
  facts: string;
  /** « Déplace la nuit du plan IC 1848 » */
  note: string | null;
  /** The tag at the right of the title. */
  tag: { tone: "neutral" | "success"; text: string } | null;
  /** The night is in it already: nothing to click. */
  disabled: boolean;
}

export function choiceView(
  t: Translate,
  locale: string,
  choice: PlanChoice,
): ChoiceView {
  let note: string | null = null;
  let tag: ChoiceView["tag"] = null;
  if (choice.state === "moves") {
    note = choice.fromName
      ? t("planAttach.moves", { name: choice.fromName })
      : t("planAttach.movesUnnamed");
  } else if (choice.state === "current") {
    tag = { tone: "success", text: t("planAttach.current") };
  } else if (choice.state === "other_target") {
    tag = { tone: "neutral", text: t("planAttach.otherTarget") };
  }
  return {
    projectId: choice.projectId,
    title: planTitle(choice),
    facts: planFacts(t, locale, choice),
    note,
    tag,
    disabled: choice.state === "current",
  };
}

/** « IC 1805, NGC 7000 ». */
export function targetsLabel(names: readonly string[]): string {
  return names.join(", ");
}

/** What the window says above the list. */
export function windowIntro(
  t: Translate,
  choices: PlanChoices | null,
  targets: readonly string[],
): string | null {
  if (!choices) return null;
  if (choices.noTarget) return t("planAttach.noTarget");
  if (choices.plans.length === 0) {
    return choices.total === 0
      ? t("planAttach.noPlansAtAll")
      : targets.length > 0
        ? t("planAttach.noPlans", { targets: targetsLabel(targets) })
        : null;
  }
  return targets.length > 0
    ? t("planAttach.sub", { targets: targetsLabel(targets) })
    : null;
}

/** Said when a search finds nothing. */
export function searchNone(t: Translate, query: string): string {
  return t("planAttach.searchNone", { query: query.trim() });
}

/** The plans a window lists: the proposed ones, or the search's results. */
export function listed(
  choices: PlanChoices | null,
  found: readonly PlanChoice[] | null,
): readonly PlanChoice[] {
  return found ?? choices?.plans ?? [];
}

/** The text of a search is worth asking the service for. */
export function searchReady(query: string): boolean {
  return query.trim().length >= 1;
}

/** The pause before a search is asked, ms. */
export const SEARCH_DELAY_MS = 300;

/** The rows « Créer un plan multi-nuits » of the window: one per target of
 * the night. */
export interface CreateRow {
  target: NewPlanTarget;
  label: string;
  body: string;
}

export function createRows(
  t: Translate,
  targets: readonly { targetId: string | null; name: string | null }[],
): CreateRow[] {
  const known = targets.filter(
    (target): target is { targetId: string; name: string | null } =>
      target.targetId !== null && target.targetId !== "",
  );
  return known.map((target) => {
    const name = target.name ?? target.targetId;
    return {
      target: { targetId: target.targetId, name, commonName: null },
      label:
        known.length > 1
          ? t("planAttach.createFor", { target: name })
          : t("planAttach.create"),
      body: t("planAttach.createBody", { target: name }),
    };
  });
}

/* ------------------------------- What was refused -------------------------- */

/** The words of a refusal of the service on a gesture on the plan itself:
 * `multi_night_not_active` here is « Planification avancée », where the
 * integrations' refusal says « Synchronisation avancée+ ». The step
 * « Cibles » says them when its primary could not attach, create or leave a
 * plan (`lib/nightApply.ts`). */
export function attachRefusalWords(
  t: Translate,
  code: string,
  message: string,
): string {
  return code === "multi_night_not_active"
    ? t("planAttach.refusal.notActive")
    : refusalWords(t, code, message);
}

/* ----------------------------- The last answers ---------------------------- */

const ATTACHMENT_KEY = "astro-sync.nightAttachment";
const CHOICES_KEY = "astro-sync.nightPlanChoices";

function readAll(
  storage: ReportStorage | null,
  key: string,
): Record<string, unknown> {
  try {
    const raw = storage?.getItem(key);
    const parsed: unknown = raw ? JSON.parse(raw) : {};
    return typeof parsed === "object" && parsed !== null
      ? (parsed as Record<string, unknown>)
      : {};
  } catch {
    return {};
  }
}

function writeOne(
  storage: ReportStorage | null,
  key: string,
  sessionId: string,
  value: unknown,
): void {
  try {
    const all = readAll(storage, key);
    all[sessionId] = value;
    // A computer keeps the last few nights read, no more.
    const kept = Object.entries(all).slice(-30);
    storage?.setItem(key, JSON.stringify(Object.fromEntries(kept)));
  } catch {
    // A storage that refuses just means reading again next time.
  }
}

export function saveAttachment(
  storage: ReportStorage | null,
  sessionId: string,
  attachment: NightAttachment,
): void {
  writeOne(storage, ATTACHMENT_KEY, sessionId, attachment);
}

export function loadAttachment(
  storage: ReportStorage | null,
  sessionId: string,
): NightAttachment | null {
  const found = readAll(storage, ATTACHMENT_KEY)[sessionId];
  return found && typeof found === "object" && "plans" in found
    ? (found as NightAttachment)
    : null;
}

export function saveChoices(
  storage: ReportStorage | null,
  sessionId: string,
  choices: PlanChoices,
): void {
  writeOne(storage, CHOICES_KEY, sessionId, choices);
}

export function loadChoices(
  storage: ReportStorage | null,
  sessionId: string,
): PlanChoices | null {
  const found = readAll(storage, CHOICES_KEY)[sessionId];
  return found && typeof found === "object" && "plans" in found
    ? (found as PlanChoices)
    : null;
}
