// What step 4 of the import queues when its primary is pressed. The first image
// ticked of a target leaves as its capture, the others as its variants
// (`sendOrder`): each variant names the digest of the image whose capture it is
// a variant of, and that image goes first in the queue. The new import lost
// this link when it replaced the old window, so two renders of one plan arrived
// as two captures. Pure: the provider queues what this says.

import type { ImageToSend } from "./bridge";
import { sendOrder, type SummaryImage } from "./detectTreatments.ts";

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

/** The file of an image, whatever it is (a night's render or a plan's). */
export function renderOf(send: ImageToSend) {
  return send.kind === "night" ? send.render : send.outcome.render;
}

/**
 * The images ticked, in the order they are queued: for each target, the head
 * (the first ticked of its rows, the natural render when it is ticked, since
 * the rows list it first) as a capture, then the others as its variants. A
 * variant of an image that cannot be found leaves as a capture.
 */
export function importSends(
  rows: readonly SummaryImage[],
  made: readonly MadeImage[],
  ticked: readonly string[],
): ImportSend[] {
  return sendOrder(rows, ticked).map(({ row, variantOf }) => {
    const item = made.find(
      (m) => m.targetId === row.targetId && m.path === row.path,
    );
    if (!item) throw new Error(`No image was made for ${row.path}.`);
    const head =
      variantOf &&
      made.find(
        (m) => m.path === variantOf.path && m.targetId === variantOf.targetId,
      );
    if (!head) return { row, send: item.send, variant: false };
    const variantOfSha256 = renderOf(head.send).sha256;
    return {
      row,
      send: { ...item.send, variantOfSha256 },
      variant: true,
    };
  });
}

/** How many of the images queued are variants of another one queued. */
export function variantCount(sends: readonly ImportSend[]): number {
  return sends.filter((item) => item.variant).length;
}
