// The storage card at the head of « Envois »: how much of the space is used,
// how many images it keeps against what the free tier allows, read from the
// service (`GET /v1/space/storage`, the same read as the check of
// « Récapitulatif »). Pure: the page gives what it read (or the failure) and
// draws the view.

import type { SpaceStorage } from "./bridge";

/** `Number.MAX_SAFE_INTEGER`: a figure the profile does not cap. */
const UNLIMITED = 9007199254740991;

export type StorageRead =
  | { kind: "loading" }
  /** The service did not answer (or the call failed). */
  | { kind: "unreachable" }
  /** This computer is not attached to a space. */
  | { kind: "unpaired" }
  | { kind: "ok"; storage: SpaceStorage };

export type StorageView =
  | { kind: "loading" }
  | { kind: "unreachable" }
  | { kind: "unpaired" }
  | {
      kind: "ok";
      usedBytes: number;
      /** `null`: no cap on the size. */
      quotaBytes: number | null;
      /** 0 to 1 of the size used; `null` without a cap. */
      usedFraction: number | null;
      /** The largest file the space takes, `null` when nothing caps it. */
      maxFileBytes: number | null;
      /** The images kept against the quota of the free tier; `null` when
       * nothing caps their number. */
      images: { count: number; limit: number; fraction: number } | null;
    };

function capped(value: number): number | null {
  return value > 0 && value < UNLIMITED ? value : null;
}

export function storageView(read: StorageRead): StorageView {
  if (read.kind !== "ok") return read;
  const { storage } = read;
  const quotaBytes = capped(storage.quotaBytes);
  const limit = capped(storage.captureCountLimit);
  return {
    kind: "ok",
    usedBytes: Math.max(0, storage.usedBytes),
    quotaBytes,
    usedFraction:
      quotaBytes === null
        ? null
        : Math.max(0, Math.min(1, storage.usedBytes / quotaBytes)),
    maxFileBytes: capped(storage.maxFileBytes),
    images:
      limit === null
        ? null
        : {
            count: storage.captureCount,
            limit,
            fraction: Math.max(0, Math.min(1, storage.captureCount / limit)),
          },
  };
}

/** What a failed read of the storage means: no space attached, or a service
 * that did not answer. */
export function readOfFailure(error: unknown): StorageRead {
  const code =
    typeof error === "object" && error !== null && "code" in error
      ? (error as { code: unknown }).code
      : null;
  return code === "not_paired" ? { kind: "unpaired" } : { kind: "unreachable" };
}
