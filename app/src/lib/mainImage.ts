// « Images à envoyer »: which image of a capture is its main image. One choice
// per target and series of poses (the images of a series are one capture), made
// on the cards, and what the space says of the capture when it holds one
// already. Pure TypeScript (no React, no Tauri): the window draws it, the
// import queues it.
//
// The rules, all of them a decision of this computer's screen -- the one that
// counts is taken again when the image really leaves (`main_choice.rs`):
//
// - nothing is chosen: the main image is the one that would lead today, the
//   first image ticked;
// - the capture exists already on the space and is private: « Garder l'image
//   principale de votre espace » is the choice, the images all leave as
//   variants of that capture; another image ticked as the main one is a
//   deliberate gesture, written on the screen;
// - the capture is public: the service only changes its main image from the
//   web, the images leave as variants and the choice is greyed;
// - the space cannot be read: as today, with a line that says so.

import type { NightOnline, OnlineCapture } from "./bridge";
import { seriesKeyOfRow, type SummaryImage } from "./detectTreatments.ts";
import { seriesKey } from "./seriesOfPoses.ts";

/** The choice that keeps the main image the space holds. */
export const KEEP = "keep";

/** What the space said of a session: its night, or that it could not be read. */
export type OnlineRead = NightOnline | "failed";

/** What the space holds for one capture to be (a target, a series of poses). */
export type GroupOnline =
  /** Nothing: a session still to create, or no capture of this series. */
  | { kind: "none" }
  /** Being asked. */
  | { kind: "pending" }
  /** Asked, and the space could not say (offline, an answer that does not
   * tell which capture is this series'). */
  | { kind: "unread" }
  | {
      kind: "capture";
      captureId: string;
      visibility: "private" | "public";
      /** The main image of the capture, `null` when it has none. */
      main: { sha256: string; name: string; since: string } | null;
      /** The digests of the variants it holds. */
      variants: string[];
    };

/** What the window knows around the rows. */
export interface MainContext {
  /** The session a row's capture belongs to (an image of a night), by row key;
   * `null` for the image of a plan. */
  sessionOf: Readonly<Record<string, string | null>>;
  /** The sessions the space holds already (the others are still to create). */
  existing: ReadonlySet<string>;
  /** What the space said of each of them. */
  reads: Readonly<Record<string, OnlineRead>>;
  /** The digest of the file of each row. */
  shaOf: Readonly<Record<string, string>>;
  /** Whether the space keeps variants (`storage.variantLimit` above 0, or not
   * known): without them nothing can be kept beside another image. */
  variantsAllowed: boolean;
}

/** The key of a group: a target and a series of poses (a series not known is
 * one), the key `sendOrder` leads a capture with. */
export function groupKeyOfRow(
  row: Pick<SummaryImage, "targetId" | "series">,
): string {
  return `${row.targetId}\u0000${seriesKeyOfRow(row) ?? ""}`;
}

/** The rows that have a card of their own (the TIFF of a render is the second
 * box of its PNG's card; the linear file is a line, not an image). */
export function cardsOf(rows: readonly SummaryImage[]): SummaryImage[] {
  return rows.filter(
    (row) =>
      row.role !== "file" &&
      !(
        row.fullResolution &&
        rows.some(
          (other) =>
            other.targetId === row.targetId &&
            other.path === row.twinOf &&
            !other.fullResolution,
        )
      ),
  );
}

/** The series key of a capture online (`filter|exposure|gain|binning`). */
function onlineSeriesKey(capture: OnlineCapture): string | null {
  const series = capture.series;
  if (!series || typeof series.exposureSeconds !== "number") return null;
  return seriesKey({
    filter: series.filter ?? null,
    exposureSeconds: series.exposureSeconds,
    gain: series.gain ?? null,
    binning: series.binning ?? 1,
    frames: null,
    totalSeconds: null,
  });
}

/** What the space holds for the group `row` stands for. */
export function groupOnline(
  context: MainContext,
  row: SummaryImage,
): GroupOnline {
  const session = context.sessionOf[row.key] ?? null;
  // A session the import still has to create holds nothing; an image of a
  // plan has no night to read.
  if (!session || !context.existing.has(session)) return { kind: "none" };
  const read = context.reads[session];
  if (read === undefined) return { kind: "pending" };
  if (read === "failed") return { kind: "unread" };
  const target = read.targets.find((item) => item.targetId === row.targetId);
  const captures = (target?.captures ?? []).filter(
    (capture) => capture.mainImage !== null,
  );
  if (captures.length === 0) return { kind: "none" };
  const wanted = seriesKeyOfRow(row);
  let capture: OnlineCapture | undefined;
  if (wanted === null) {
    // No series: the main capture of the target, which the service lists first.
    capture = captures[0];
  } else {
    capture = captures.find((item) => onlineSeriesKey(item) === wanted);
    if (!capture) {
      // A capture without a series may take this one (the service decides it
      // by the name of its image): that is not told here.
      if (captures.some((item) => onlineSeriesKey(item) === null))
        return { kind: "unread" };
      return { kind: "none" };
    }
  }
  const main = capture.mainImage;
  return {
    kind: "capture",
    captureId: capture.id,
    visibility: capture.visibility === "public" ? "public" : "private",
    main: main
      ? {
          sha256: main.sha256,
          name: main.originalName,
          since: main.createdAt,
        }
      : null,
    variants: capture.images
      .filter((image) => !image.main)
      .map((image) => image.sha256),
  };
}

/** Why the choice of a card is closed. */
export type ChoiceBlock =
  /** The image is not ticked: it is not sent. */
  | "notTicked"
  /** Its bytes are online as a variant of the capture: only the web changes
   * the main image to a variant. */
  | "onlineVariant"
  /** Its bytes are the main image already. */
  | "onlineMain";

/** One group of « Images à envoyer »: what the window draws above its cards. */
export interface MainGroup {
  key: string;
  targetId: string;
  /** The cards of the group, in the order of the list. */
  cards: SummaryImage[];
  online: GroupOnline;
  /** The capture is public: every choice is closed, the images are variants. */
  locked: boolean;
  /** « Garder l'image principale de votre espace » is offered. */
  keepOffered: boolean;
  /** The key of the card that is the main image, `KEEP`, or `null` (nothing
   * ticked, or locked). */
  selected: string | null;
  /** Why a card's choice is closed. */
  blocks: Record<string, ChoiceBlock>;
}

/** The groups of the list, in order, with their choice resolved. `choices` is
 * what the person chose, by group key: a card's key or `KEEP`. */
export function mainGroups(
  rows: readonly SummaryImage[],
  ticked: readonly string[],
  choices: Readonly<Record<string, string>>,
  context: MainContext,
): MainGroup[] {
  const order: string[] = [];
  const byKey = new Map<string, SummaryImage[]>();
  for (const card of cardsOf(rows)) {
    const key = groupKeyOfRow(card);
    if (!byKey.has(key)) {
      byKey.set(key, []);
      order.push(key);
    }
    byKey.get(key)!.push(card);
  }
  return order.map((key) => {
    const cards = byKey.get(key)!;
    const online = groupOnline(context, cards[0]);
    const capture = online.kind === "capture" ? online : null;
    const locked = capture?.visibility === "public";
    const keepOffered =
      !!capture && !locked && capture.main !== null && context.variantsAllowed;
    const blocks: Record<string, ChoiceBlock> = {};
    for (const card of cards) {
      if (!ticked.includes(card.key)) blocks[card.key] = "notTicked";
      else if (keepOffered && capture) {
        const sha = context.shaOf[card.key];
        if (sha && capture.main?.sha256.toLowerCase() === sha.toLowerCase())
          blocks[card.key] = "onlineMain";
        else if (
          sha &&
          capture.variants.some((v) => v.toLowerCase() === sha.toLowerCase())
        )
          blocks[card.key] = "onlineVariant";
      }
    }
    const open = (cardKey: string) => !(cardKey in blocks);
    const picked = choices[key];
    let selected: string | null;
    if (locked) selected = null;
    else if (keepOffered) {
      selected =
        picked !== undefined && picked !== KEEP && open(picked) ? picked : KEEP;
    } else if (picked !== undefined && picked !== KEEP && open(picked)) {
      selected = picked;
    } else {
      selected = cards.find((card) => open(card.key))?.key ?? null;
    }
    return {
      key,
      targetId: cards[0].targetId,
      cards,
      online,
      locked,
      keepOffered,
      selected,
      blocks,
    };
  });
}

/** What leaves for a group, by group key. */
export type MainPick =
  /** The image leads: it leaves first, without a capture of its own, and the
   * others are variants of it. `wish`: what the space must confirm when it
   * leaves (`main_choice.rs`) -- `over`, the image was chosen to take the main
   * place of a capture the space holds; `keep`, nothing was chosen and the
   * space could not be read, so the main image it holds is not touched;
   * `null`, nothing to confirm (no capture online). */
  | { kind: "row"; key: string; wish: MainWish | null }
  /** Every image is a variant of this capture of the space; its main image is
   * never touched. */
  | { kind: "capture"; captureId: string };

/** What the space must confirm as an image leaves (`main_choice.rs`). */
export type MainWish = "keep" | "over";

/** What each group sends: `mainGroups` turned into what the queue is told. A
 * group with no image ticked has no entry. */
export function mainPlan(
  groups: readonly MainGroup[],
  choices: Readonly<Record<string, string>> = {},
): Record<string, MainPick> {
  const plan: Record<string, MainPick> = {};
  for (const group of groups) {
    const online = group.online;
    if (online.kind === "capture" && (group.locked || group.selected === KEEP))
      plan[group.key] = { kind: "capture", captureId: online.captureId };
    else if (group.selected !== null && group.selected !== KEEP) {
      const picked = choices[group.key] === group.selected;
      plan[group.key] = {
        kind: "row",
        key: group.selected,
        wish:
          online.kind === "capture"
            ? group.keepOffered
              ? "over"
              : null
            : online.kind === "unread" || online.kind === "pending"
              ? picked
                ? "over"
                : "keep"
              : null,
      };
    }
  }
  return plan;
}

/** The choices that still stand once `ticked` changed: a choice of an image
 * that is not ticked any more is dropped, and the main image falls back to the
 * first image ticked of the group (the one that leads today). */
export function keepChoices(
  rows: readonly SummaryImage[],
  ticked: readonly string[],
  choices: Readonly<Record<string, string>>,
): Record<string, string> {
  const kept: Record<string, string> = {};
  const cards = cardsOf(rows);
  for (const [key, value] of Object.entries(choices)) {
    if (value === KEEP) {
      kept[key] = value;
      continue;
    }
    const card = cards.find((item) => item.key === value);
    if (card && ticked.includes(card.key)) kept[key] = value;
  }
  return kept;
}

/** The date the main image is online since, « 12 nov. » (the year too when it
 * is not this one). */
export function sinceDate(
  locale: string,
  iso: string,
  now: Date = new Date(),
): string {
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return "";
  const sameYear = date.getUTCFullYear() === now.getUTCFullYear();
  return new Intl.DateTimeFormat(locale, {
    day: "numeric",
    month: "short",
    ...(sameYear ? {} : { year: "numeric" }),
    timeZone: "UTC",
  }).format(date);
}

/** How the main image the space holds is named: the render of the card whose
 * bytes it is, else the name of its file. */
export function onlineMainLabel(
  group: MainGroup,
  context: MainContext,
  main: { sha256: string; name: string },
  renderOf: (card: SummaryImage) => string,
): string {
  const card = group.cards.find((item) => {
    const sha = context.shaOf[item.key];
    return !!sha && sha.toLowerCase() === main.sha256.toLowerCase();
  });
  return card ? renderOf(card) : main.name;
}
