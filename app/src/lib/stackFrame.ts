// « Cadre de l'empilement »: the frame of the stacking of a target, in the
// head of the window « Pipelines pour {cible} ». The field rotation and the
// notches are measured by the stacking script at the alignment of the poses
// (`night_framing.py`) and read back from its records (`stackingFrame`);
// nothing is recomputed here. Before the first stacking the block is greyed,
// « Automatique » alone; after it, a slider of five notches from the area
// every pose covers to the whole field. One frame per target, for all its
// pipelines; a frame moved by hand is kept on this computer beside the
// pipelines chosen. Pure TypeScript, tested under `node --test`.

import type { TranslateParams } from "../i18n/locale.ts";
import type { FrameNotch, TargetFrame } from "./bridge";
import { integrationTime } from "./multiNight.ts";
import { percentOf } from "./planFraming.ts";

type Translate = (key: string, params?: TranslateParams) => string;

/** The notches of the slider: the common area, three between, the whole field. */
export const FRAME_NOTCHES = 5;

/** A frame moved by hand: the rank of its notch and, when the target was
 * measured, the share of the field it keeps (what its card says). */
export interface FrameChoice {
  rank: number;
  share?: number;
}

/** The frames moved by hand, by target; a target not named is Automatic. */
export type TargetFrames = Readonly<Record<string, FrameChoice>>;

/** What the block shows: before the first stacking, a field that turned, one
 * that did not, one that turned so far no area every pose covers. */
export type FrameState = "before" | "turned" | "still" | "noCommon";

export function frameState(measured: TargetFrame | null): FrameState {
  if (!measured) return "before";
  if (measured.measure.notches.length >= 2) return "turned";
  const degrees = measured.fieldRotationDegrees;
  return degrees !== null && degrees >= 3 ? "noCommon" : "still";
}

/** The rank the slider stands on: the one moved by hand, else Automatic. */
export function frameRank(
  measured: TargetFrame | null,
  choice: FrameChoice | undefined,
): number {
  const automatic = measured?.measure.automatic ?? FRAME_NOTCHES - 1;
  const count = measured?.measure.notches.length ?? 0;
  if (!choice || count < 2) return automatic;
  return Math.min(Math.max(0, choice.rank), count - 1);
}

/** The choice once the slider moved: Automatic again (`null`) on its notch. */
export function frameChoiceOf(
  measured: TargetFrame | null,
  rank: number,
): FrameChoice | null {
  const notch = measured?.measure.notches[rank];
  if (!notch || rank === measured?.measure.automatic) return null;
  return { rank, share: notch.shareOfFrame };
}

/** One stop of the slider, with what is written under it. */
export interface FrameStopView {
  /** « 60 % »: the share of the whole field, under each notch; « – » before
   * the alignment. */
  percent: string;
  /** « Zone commune », « Tout le champ »: the first and the last. */
  name: string | null;
  automatic: boolean;
}

/** What the block says. */
export interface FrameView {
  state: FrameState;
  /** The slider can move (a field that turned). */
  enabled: boolean;
  line: string;
  name: string;
  /** « Automatique » beside the name. */
  automatic: boolean;
  /** « Mesuré à l'alignement » beside the name, before the first stacking. */
  pending: boolean;
  size: string | null;
  exposure: string;
  /** Before the first stacking: what the alignment will show. */
  after: string | null;
  stops: FrameStopView[];
  rank: number;
  note: { tone: "ok" | "warn" | "muted"; text: string } | null;
  foot: string;
}

function whole(value: number, locale: string): string {
  return new Intl.NumberFormat(locale).format(value);
}

function notchName(t: Translate, rank: number, count: number): string {
  if (rank === 0) return t("wizard.frame.nameCommon");
  if (rank === count - 1) return t("wizard.frame.nameWhole");
  return t("wizard.frame.nameBetween");
}

/** « 11 % du cadre à 20 min au lieu de 2 h », in time when the exposure of
 * the target is known, in poses otherwise. */
function exposureText(
  t: Translate,
  notch: FrameNotch,
  registered: number,
  seconds: number | null,
): string {
  if (notch.fullShare >= 1) return t("wizard.frame.exposureFull");
  const share = percentOf(1 - notch.fullShare);
  if (seconds && seconds > 0 && registered > 0) {
    return t("wizard.frame.exposureLow", {
      share,
      low: integrationTime(t, (seconds * notch.lowestFrames) / registered),
      total: integrationTime(t, seconds),
    });
  }
  return t("wizard.frame.exposureLowPoses", {
    share,
    low: notch.lowestFrames,
    total: registered,
  });
}

/** What the block says for the frame chosen, from the numbers measured.
 * ``seconds``: the exposure of the target's poses (its card's duration). */
export function frameView(
  t: Translate,
  locale: string,
  measured: TargetFrame | null,
  choice: FrameChoice | undefined,
  seconds: number | null,
): FrameView {
  const state = frameState(measured);
  const blank = Array.from({ length: FRAME_NOTCHES }, (_, position) => ({
    percent: "–",
    name: null,
    automatic: position === FRAME_NOTCHES - 1,
  }));
  if (state === "before" || !measured) {
    return {
      state,
      enabled: false,
      line: t("wizard.frame.lineBefore"),
      name: t("wizard.frame.automatic"),
      automatic: false,
      pending: true,
      size: null,
      exposure: t("wizard.frame.automaticHelp"),
      after: t("wizard.frame.afterStacking"),
      stops: blank.map((stop, position) => ({
        ...stop,
        automatic: false,
        name:
          position === 0
            ? t("wizard.frame.stopCommon")
            : position === FRAME_NOTCHES - 1
              ? t("wizard.frame.stopWhole")
              : null,
      })),
      rank: FRAME_NOTCHES - 1,
      note: null,
      foot: t("wizard.frame.footBefore"),
    };
  }
  const { measure } = measured;
  const width = measure.frameWidth ?? 0;
  const height = measure.frameHeight ?? 0;
  // A frame other than the one the last stacking cut: the next treatment cuts
  // it from the whole field, without stacking again.
  const foot =
    state === "turned" &&
    frameRank(measured, choice) !== (measure.rank ?? measure.automatic)
      ? t("wizard.frame.footChanged")
      : t("wizard.frame.foot");
  if (state !== "turned") {
    return {
      state,
      enabled: false,
      line:
        state === "still"
          ? t("wizard.frame.lineStill")
          : t("wizard.frame.lineNoCommon", {
              degrees: Math.round(measured.fieldRotationDegrees ?? 0),
            }),
      name: t("wizard.frame.nameWhole"),
      automatic: true,
      pending: false,
      size:
        width && height
          ? t("wizard.frame.size", {
              width: whole(width, locale),
              height: whole(height, locale),
              share: 100,
            })
          : null,
      exposure:
        state === "still"
          ? t("wizard.frame.exposureFull")
          : t("wizard.frame.exposureNoCommon"),
      after: null,
      stops: blank.map((_, position) => ({
        percent:
          position === FRAME_NOTCHES - 1
            ? t("wizard.frame.percent", { value: 100 })
            : "",
        name: position === FRAME_NOTCHES - 1 ? t("wizard.frame.stopAll") : null,
        automatic: false,
      })),
      rank: FRAME_NOTCHES - 1,
      note:
        state === "still"
          ? { tone: "ok", text: t("wizard.frame.noteStill") }
          : { tone: "muted", text: t("wizard.frame.noteNoCommon") },
      foot,
    };
  }
  const count = measure.notches.length;
  const rank = frameRank(measured, choice);
  const notch = measure.notches[rank];
  const lost = 100 - percentOf(notch.shareOfFrame, true);
  const low = percentOf(1 - notch.fullShare);
  let note: FrameView["note"];
  if (rank === 0) {
    note = { tone: "muted", text: t("wizard.frame.noteCommon", { lost }) };
  } else if (rank === count - 1) {
    note = { tone: "muted", text: t("wizard.frame.noteWhole", { share: low }) };
  } else {
    note = { tone: "muted", text: t("wizard.frame.noteBetween", { lost }) };
  }
  return {
    state,
    enabled: true,
    line: t("wizard.frame.lineTurned", {
      degrees: Math.round(measured.fieldRotationDegrees ?? 0),
    }),
    name: notchName(t, rank, count),
    automatic: rank === measure.automatic,
    pending: false,
    size: t("wizard.frame.size", {
      width: whole(notch.width, locale),
      height: whole(notch.height, locale),
      share: percentOf(notch.shareOfFrame, true),
    }),
    exposure: exposureText(t, notch, measured.registered, seconds),
    after: null,
    stops: measure.notches.map((item, position) => ({
      percent: t("wizard.frame.percent", {
        value: percentOf(item.shareOfFrame, true),
      }),
      name:
        position === 0
          ? t("wizard.frame.stopCommon")
          : position === count - 1
            ? t("wizard.frame.stopWhole")
            : null,
      automatic: position === measure.automatic,
    })),
    rank,
    note,
    foot,
  };
}

/** What the card of the target says of its frame: one line, only when it was
 * moved by hand (Automatic says nothing). */
export function frameLine(
  t: Translate,
  choice: FrameChoice | undefined,
): string | null {
  if (!choice) return null;
  const name =
    choice.share !== undefined
      ? t("wizard.frame.cardShare", {
          name: notchName(t, choice.rank, FRAME_NOTCHES),
          share: percentOf(choice.share, true),
        })
      : notchName(t, choice.rank, FRAME_NOTCHES);
  return t("wizard.frame.cardLine", { frame: name });
}

/** One target's frames after the window: its choice, or Automatic. */
export function withTargetFrame(
  frames: TargetFrames | undefined,
  targetId: string,
  choice: FrameChoice | null,
): Record<string, FrameChoice> {
  const next: Record<string, FrameChoice> = { ...frames };
  if (choice) next[targetId] = { ...choice };
  else delete next[targetId];
  return next;
}

/** « Appliquer à toutes les cibles »: a frame moved by hand goes to every
 * target by the rank of its notch (each target measures its own notches; the
 * share it keeps is its own, so it is not copied); Automatic copies nothing,
 * every other target keeps its own frame. */
export function applyFrameToAll(
  frames: TargetFrames | undefined,
  sourceId: string,
  targetIds: Iterable<string>,
  choice: FrameChoice | null,
): Record<string, FrameChoice> {
  let next = withTargetFrame(frames, sourceId, choice);
  if (!choice) return next;
  for (const targetId of targetIds) {
    if (targetId === sourceId) continue;
    next = withTargetFrame(next, targetId, { rank: choice.rank });
  }
  return next;
}

/** The rank the stacking of a target is given; `null`: Automatic. */
export function frameToRun(
  frames: TargetFrames | undefined,
  targetId: string,
): number | null {
  const choice = frames?.[targetId];
  return choice && choice.rank >= 0 && choice.rank < FRAME_NOTCHES
    ? choice.rank
    : null;
}

/* ------------------------ Kept on this computer --------------------------- */

interface FrameStorage {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
}

const framesKey = (sessionId: string) => `astro-sync.frames:${sessionId}`;

/** The frames moved by hand for the targets of a night, kept on this computer
 * beside the pipelines chosen and their finishes: they survive the window
 * closed and the next day. What cannot be read is Automatic. */
export function loadTargetFrames(
  storage: FrameStorage | null,
  sessionId: string,
): Record<string, FrameChoice> {
  const out: Record<string, FrameChoice> = {};
  try {
    const text = storage?.getItem(framesKey(sessionId));
    const parsed = text ? (JSON.parse(text) as unknown) : null;
    if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) {
      return out;
    }
    for (const [targetId, value] of Object.entries(
      parsed as Record<string, unknown>,
    )) {
      if (!value || typeof value !== "object") continue;
      const held = value as Record<string, unknown>;
      const rank = held.rank;
      if (
        typeof rank !== "number" ||
        !Number.isInteger(rank) ||
        rank < 0 ||
        rank >= FRAME_NOTCHES
      ) {
        continue;
      }
      out[targetId] =
        typeof held.share === "number" && held.share > 0 && held.share <= 1
          ? { rank, share: held.share }
          : { rank };
    }
  } catch {
    // Frames that cannot be read: every target is Automatic.
  }
  return out;
}

export function saveTargetFrames(
  storage: FrameStorage | null,
  sessionId: string,
  frames: TargetFrames,
): void {
  try {
    storage?.setItem(framesKey(sessionId), JSON.stringify(frames));
  } catch {
    // A frame lost only brings Automatic back.
  }
}
