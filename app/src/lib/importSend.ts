// What step 4 of the import queues when its primary is pressed. The first image
// ticked of a target leaves as its capture, the others as its variants
// (`sendOrder`): each variant names the digest of the image whose capture it is
// a variant of, and that image goes first in the queue. The new import lost
// this link when it replaced the old window, so two renders of one plan arrived
// as two captures. Pure: the provider queues what this says.

import type { ImageToSend } from "./bridge";
import { sendOrder, type SummaryImage } from "./detectTreatments.ts";
import type { MainPick } from "./mainImage.ts";

/** An image made by the treatments, how it would leave and the row of step 4
 * it stands for. */
interface MadeImage {
  targetId: string;
  path: string;
  send: ImageToSend;
}

/** One image to queue: the row of step 4 and what leaves. */
export interface ImportSend {
  row: SummaryImage;
  send: ImageToSend;
  /** Whether it leaves as a variant of the capture another image creates. */
  variant: boolean;
}

/** The file that leaves for an image, whatever it is (a night's render or a
 * plan's): its PNG, or its TIFF for the row of the TIFF. */
export function renderOf(send: ImageToSend): {
  path: string;
  sha256: string;
  size: number;
} {
  const render = send.kind === "night" ? send.render : send.outcome.render;
  return (send.fullResolution && render.fullResolution) || render;
}

/**
 * The images ticked, in the order they are queued: for each target and
 * series, the head (the image chosen as the main one, else the first ticked of
 * its rows, the natural render when it is ticked, since the rows list it
 * first) as a capture, then the others as its variants. A variant of an image
 * that cannot be found leaves as a capture. `picks` (`mainPlan`): a series
 * whose images are all variants of a capture the space holds leaves with that
 * capture (`variantOfCapture`), and a head says what the space must confirm
 * as it leaves (`mainWish`).
 */
export function importSends(
  rows: readonly SummaryImage[],
  made: readonly MadeImage[],
  ticked: readonly string[],
  picks: Readonly<Record<string, MainPick>> = {},
): ImportSend[] {
  return sendOrder(rows, ticked, picks).map((item) => {
    const { row, variantOf } = item;
    const madeImage = made.find(
      (m) => m.targetId === row.targetId && m.path === row.path,
    );
    if (!madeImage) throw new Error(`No image was made for ${row.path}.`);
    const head =
      variantOf &&
      made.find(
        (m) => m.path === variantOf.path && m.targetId === variantOf.targetId,
      );
    if (item.variantOfCapture) {
      // Every image of the series is a variant of a capture the space holds:
      // its main image is never touched. The linear file is attached to it.
      return {
        row,
        send: { ...madeImage.send, variantOfCapture: item.variantOfCapture },
        variant: row.role !== "file",
      };
    }
    if (!head) {
      return {
        row,
        send: item.mainWish
          ? { ...madeImage.send, mainWish: item.mainWish }
          : madeImage.send,
        variant: false,
      };
    }
    const variantOfSha256 = renderOf(head.send).sha256;
    return {
      row,
      send: { ...madeImage.send, variantOfSha256 },
      // The linear file is attached to the capture, not a variant of it.
      variant: row.role !== "file",
    };
  });
}

/** How many images are queued: the linear file is a file, not an image. */
export function imageCount(sends: readonly ImportSend[]): number {
  return sends.filter((item) => item.row.role !== "file").length;
}

/** How many of the images queued are variants of another one queued. */
export function variantCount(sends: readonly ImportSend[]): number {
  return sends.filter((item) => item.variant).length;
}
