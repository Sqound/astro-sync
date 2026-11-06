// « Annotations »: the captures already sent without a sky solution -- from the
// phone, the web or a night --, where each image is (in a night's folder,
// solved there, or only online, its original downloaded first), each row's
// state and its failure in words. Pure TypeScript, tested under `node --test`;
// `screens/AnnotationsScreen.tsx` draws it.

import type { TranslateParams } from "../i18n/locale.ts";
import type { MissingCapture, SolveFailure, ToolRow } from "./bridge";
import { dayInSentence } from "./tabs.ts";
import { formatSize, pluralText } from "./written.ts";

type Translate = (key: string, params?: TranslateParams) => string;

export type RowState = "todo" | "running" | "done" | "failed";

export interface RowProgress {
  state: RowState;
  failure?: SolveFailure;
}

/** The four states of the tab. */
export type AnnotationsState = "list" | "running" | "empty" | "noSolver";

export function annotationsState(input: {
  captures: readonly MissingCapture[];
  solverPresent: boolean;
  running: boolean;
}): AnnotationsState {
  if (input.captures.length === 0) return "empty";
  if (!input.solverPresent) return "noSolver";
  return input.running ? "running" : "list";
}

/** A solver is there: ASTAP with its star database, or astrometry.net. */
export function solverPresent(tools: readonly ToolRow[] | null): boolean {
  if (tools === null) return true;
  const present = (id: string) =>
    tools.some((row) => row.id === id && row.state === "present");
  return (
    (present("astap") && present("astap-database")) || present("astrometry-net")
  );
}

/** « Galaxie d'Andromède »: the common name of the first target, else its
 * designation, else the file. */
export function captureTitle(capture: MissingCapture): string {
  const target = capture.targets[0];
  return target?.name ?? target?.designation ?? capture.originalName;
}

/** « M31 · Nuit du lun. 5 oct. · M31_final.jpg ». */
export function captureLine(
  t: Translate,
  locale: string,
  capture: MissingCapture,
): string {
  const parts: string[] = [];
  const designation = capture.targets[0]?.designation;
  if (designation) parts.push(designation);
  if (capture.nightDate) {
    parts.push(
      t("annotations.nightOf", {
        day: dayInSentence(capture.nightDate.slice(0, 10), locale),
      }),
    );
  }
  parts.push(capture.originalName);
  return parts.join(" · ");
}

/** Where the image is: in a night's folder on this computer (`here`), or
 * only online, with where it came from. */
export function sourceText(
  t: Translate,
  capture: MissingCapture,
  here: boolean,
): string {
  if (here) return t("annotations.source.here");
  switch (capture.mainImageSource) {
    case "phone":
      return t("annotations.source.phone");
    case "web":
      return t("annotations.source.web");
    default:
      return t("annotations.source.other");
  }
}

/** What the running row says: the download of the original first when the
 * image is not here, then the resolution. */
export function runningText(
  t: Translate,
  locale: string,
  capture: MissingCapture,
  here: boolean,
): string {
  return here
    ? t("annotations.solvingHere")
    : t("annotations.downloading", {
        size: formatSize(t, capture.byteSize, locale),
      });
}

/** The failure in words, never its code. */
export function failureText(t: Translate, failure: SolveFailure): string {
  return t(`annotations.failure.${failure.code}`, {
    seconds: failure.seconds ?? 0,
  });
}

export function stateText(t: Translate, state: RowState): string {
  return t(`annotations.state.${state}`);
}

export function countText(t: Translate, locale: string, count: number): string {
  return pluralText(t, locale, "annotations.count", count);
}

/** The rows « Tout résoudre » goes through: the ones still to solve and the
 * failed ones, in the list's order. */
export function rowsToSolve(
  captures: readonly MissingCapture[],
  progress: Readonly<Record<string, RowProgress>>,
): MissingCapture[] {
  return captures.filter((capture) => {
    const state = progress[capture.id]?.state ?? "todo";
    return state === "todo" || state === "failed";
  });
}
