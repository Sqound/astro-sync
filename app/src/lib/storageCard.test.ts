import assert from "node:assert/strict";
import test from "node:test";

import type { SpaceStorage } from "./bridge";
import { readOfFailure, storageView } from "./storageCard.ts";

const storage = (extra: Partial<SpaceStorage> = {}): SpaceStorage =>
  ({
    usedBytes: 50_000_000,
    quotaBytes: 200_000_000,
    remainingBytes: 150_000_000,
    maxFileBytes: 25_000_000,
    captureCount: 1,
    captureCountLimit: 3,
    variantLimit: 0,
    capabilities: {},
    ...extra,
  }) as SpaceStorage;

test("the card of the free tier: size used over the quota, images kept over their limit", () => {
  const view = storageView({ kind: "ok", storage: storage() });
  assert.equal(view.kind, "ok");
  if (view.kind !== "ok") return;
  assert.equal(view.usedFraction, 0.25);
  assert.equal(view.quotaBytes, 200_000_000);
  assert.equal(view.maxFileBytes, 25_000_000);
  assert.deepEqual(view.images, { count: 1, limit: 3, fraction: 1 / 3 });
});

test("a subscription or a private server caps nothing: no bar, no image count", () => {
  const view = storageView({
    kind: "ok",
    storage: storage({
      quotaBytes: Number.MAX_SAFE_INTEGER,
      captureCountLimit: Number.MAX_SAFE_INTEGER,
      maxFileBytes: Number.MAX_SAFE_INTEGER,
    }),
  });
  assert.equal(view.kind, "ok");
  if (view.kind !== "ok") return;
  assert.equal(view.quotaBytes, null);
  assert.equal(view.usedFraction, null);
  assert.equal(view.images, null);
  assert.equal(view.maxFileBytes, null);
});

test("a full space reads as full, never past it", () => {
  const view = storageView({
    kind: "ok",
    storage: storage({ usedBytes: 300_000_000, captureCount: 5 }),
  });
  assert.equal(view.kind === "ok" && view.usedFraction, 1);
  assert.equal(view.kind === "ok" && view.images?.fraction, 1);
});

test("loading, unreachable and unpaired are states of their own", () => {
  assert.deepEqual(storageView({ kind: "loading" }), { kind: "loading" });
  assert.deepEqual(readOfFailure({ code: "not_paired", message: "" }), {
    kind: "unpaired",
  });
  assert.deepEqual(readOfFailure({ code: "network_error", message: "" }), {
    kind: "unreachable",
  });
  assert.deepEqual(readOfFailure(new Error("down")), { kind: "unreachable" });
  assert.deepEqual(storageView(readOfFailure(new Error("x"))), {
    kind: "unreachable",
  });
});
