// « Ranger les poses dans le dossier de travail » as a ticked option of the
// import's « Récapitulatif »: what is already in place, what would change,
// and what a run did -- never a bare « 0 lien créé » when every file was
// already linked by the treatments. Pure TypeScript, tested under
// `node --test`; the plan and the outcome are the engine's.

import type {
  BulkCreateOutcome,
  NightNaming,
  OrganiseMode,
  OrganiseOutcome,
  OrganisePlan,
  Proposal,
} from "./bridge";
import { resultRows } from "./detectResult.ts";
import { reasonKey } from "./organise.ts";
import { pluralText } from "./written.ts";

/** The option as the person left it. `on`: the primary tidies; `mode`: how
 * (« Déplacer » unless the sheet chose another); `destination`: `null` is the
 * working folder. */
export interface TidyChoice {
  on: boolean;
  mode: Exclude<OrganiseMode, "leave-in-place">;
  destination: string | null;
}

export const NO_TIDY: TidyChoice = {
  on: false,
  mode: "move",
  destination: null,
};

/** What the plan of the working folder says of the files, read before the
 * option is shown. */
export interface TidyState {
  /** `done`: every file is already there; `partly`: some are; `todo`: none is;
   * `nothing`: the plan holds no file. */
  kind: "done" | "partly" | "todo" | "nothing";
  /** Files already at their place. */
  inPlace: number;
  /** Files the run would put there. */
  toDo: number;
  /** Files left where they are (another file bears their name). */
  skipped: number;
  /** How the files in place are there. */
  inPlaceAs: "link" | "file" | "mixed" | null;
}

export function tidyStateOf(plan: OrganisePlan): TidyState {
  let inPlace = 0;
  let toDo = 0;
  let links = 0;
  let files = 0;
  for (const action of plan.actions) {
    if (action.kind === "already-there") {
      inPlace += 1;
      if (action.inPlaceAs === "link") links += 1;
      else files += 1;
    } else {
      toDo += 1;
    }
  }
  const skipped = plan.conflicts.length;
  const kind: TidyState["kind"] =
    inPlace === 0 && toDo === 0
      ? "nothing"
      : toDo === 0 && skipped === 0
        ? "done"
        : inPlace > 0
          ? "partly"
          : "todo";
  return {
    kind,
    inPlace,
    toDo,
    skipped,
    inPlaceAs:
      inPlace === 0
        ? null
        : links > 0 && files > 0
          ? "mixed"
          : links > 0
            ? "link"
            : "file",
  };
}

/** The plan as the sheet « Voir ce qui sera rangé » shows it: only what
 * would change (the files in place are counted, not listed). */
export function planToChange(plan: OrganisePlan): OrganisePlan {
  const actions = plan.actions.filter(
    (action) => action.kind !== "already-there",
  );
  return {
    ...plan,
    actions,
    files: actions.length,
    bytes: actions.reduce((total, action) => total + action.bytes, 0),
  };
}

/** What a run did, for « Terminé »: created / already in place / left where
 * they are. */
export interface TidyResult {
  mode: Exclude<OrganiseMode, "leave-in-place">;
  /** Files moved, copied or linked by this run. */
  created: number;
  /** Files that were already at their place. */
  inPlace: number;
  /** How the files in place are there (`null` when none). */
  inPlaceAs: TidyState["inPlaceAs"];
  /** Files left where they are, with the engine's reason. */
  skipped: { name: string; reason: string }[];
  /** Files that failed. */
  failed: number;
  stopped: boolean;
}

/** The result of a run of `plan` (`outcome` is `null` when nothing was left
 * to do and the engine was not asked). */
export function tidyResultOf(
  mode: TidyResult["mode"],
  plan: OrganisePlan,
  outcome: OrganiseOutcome | null,
): TidyResult {
  const state = tidyStateOf(plan);
  return {
    mode,
    created: outcome?.done ?? 0,
    inPlace: outcome ? outcome.alreadyThere : state.inPlace,
    inPlaceAs: state.inPlaceAs,
    skipped: [
      ...plan.conflicts.map((c) => ({ name: c.source, reason: c.reason })),
      ...(outcome?.failed ?? []).map((c) => ({
        name: c.source,
        reason: c.reason,
      })),
    ],
    failed: outcome?.failed.length ?? 0,
    stopped: outcome?.stopped ?? false,
  };
}

/** The folders of the nights tidied: their date, place and session, by
 * detected session. */
export function tidyNames(
  proposal: Proposal,
  outcome: BulkCreateOutcome,
): Record<string, NightNaming> {
  return Object.fromEntries(
    resultRows(proposal, outcome).map((row) => [
      outcome.sessions.find((s) => s.sessionId === row.sessionId)?.detectedId ??
        row.sessionId,
      { place: row.place || null, sessionId: row.sessionId },
    ]),
  );
}

type Translate = (
  key: string,
  params?: Record<string, string | number>,
) => string;

/** How the files in place are there, in words (« liens symboliques »). */
export function howWords(t: Translate, kind: TidyState["inPlaceAs"]): string {
  return t(`import.tidy.how.${kind ?? "file"}`);
}

const CREATED_KEY: Record<TidyResult["mode"], string> = {
  move: "organise.moved",
  copy: "organise.copied",
  symlink: "organise.linked",
};

/**
 * What « Terminé » says of the tidying: the files put in place, the ones
 * that already were, the ones left where they are -- never a bare « 0 lien
 * créé » when every file was in place already.
 */
export function tidyReportLines(
  t: Translate,
  locale: string,
  result: TidyResult,
): string[] {
  const lines: string[] = [];
  const how = howWords(t, result.inPlaceAs);
  const count = new Intl.NumberFormat(locale);
  if (result.created > 0) {
    lines.push(
      pluralText(t, locale, CREATED_KEY[result.mode], result.created, {
        count: count.format(result.created),
      }),
    );
    if (result.inPlace > 0)
      lines.push(
        t("import.done.tidy.inPlace", {
          count: count.format(result.inPlace),
          how,
        }),
      );
  } else if (result.inPlace > 0) {
    lines.push(
      t("import.done.tidy.allInPlace", {
        count: count.format(result.inPlace),
        how,
      }),
    );
  }
  if (result.skipped.length > 0) {
    const reasons = [...new Set(result.skipped.map((s) => s.reason))].map(
      (reason) => t(reasonKey(reason)),
    );
    lines.push(
      pluralText(t, locale, "import.done.tidy.skipped", result.skipped.length, {
        count: count.format(result.skipped.length),
        reason: reasons.join(", "),
      }),
    );
  }
  if (result.stopped) lines.push(t("organise.stopped"));
  if (lines.length === 0) lines.push(t("import.done.tidy.nothing"));
  return lines;
}
