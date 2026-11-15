import assert from "node:assert/strict";
import test from "node:test";

import { createLimiter, latestOnly, mapWithLimit } from "./concurrency.ts";

/** A promise settled by hand. */
function gate<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((done, fail) => {
    resolve = done;
    reject = fail;
  });
  return { promise, resolve, reject };
}

const tick = () => new Promise<void>((done) => setTimeout(done, 0));

test("mapWithLimit never runs more than the limit at once and keeps the order of the results", async () => {
  const gates = Array.from({ length: 7 }, () => gate<number>());
  let running = 0;
  let peak = 0;
  const started: number[] = [];
  const all = mapWithLimit([0, 1, 2, 3, 4, 5, 6], 3, async (item) => {
    started.push(item);
    running += 1;
    peak = Math.max(peak, running);
    try {
      return await gates[item].promise;
    } finally {
      running -= 1;
    }
  });
  await tick();
  assert.deepEqual(started, [0, 1, 2]);
  // The answers come back out of order: 2, then 0, then the rest.
  gates[2].resolve(20);
  await tick();
  assert.deepEqual(started, [0, 1, 2, 3]);
  gates[0].resolve(0);
  gates[1].resolve(10);
  gates[3].resolve(30);
  await tick();
  assert.deepEqual(started, [0, 1, 2, 3, 4, 5, 6]);
  for (const index of [6, 5, 4]) gates[index].resolve(index * 10);
  assert.deepEqual(await all, [0, 10, 20, 30, 40, 50, 60]);
  assert.equal(peak, 3);
});

test("mapWithLimit of nothing answers at once, and a failure rejects", async () => {
  assert.deepEqual(await mapWithLimit([], 4, async () => 1), []);
  await assert.rejects(
    mapWithLimit([1, 2], 1, async (item) => {
      if (item === 2) throw new Error("boom");
      return item;
    }),
    /boom/,
  );
});

test("the limiter serves the lowest priority first, then the oldest request", async () => {
  const limiter = createLimiter(1);
  const first = gate<void>();
  const order: string[] = [];
  const held = limiter.run(() => first.promise, 0);
  const late = limiter.run(async () => void order.push("late"), 9);
  const urgent = limiter.run(async () => void order.push("urgent"), 1);
  const same = limiter.run(async () => void order.push("same"), 9);
  first.resolve();
  await Promise.all([held, late, urgent, same]);
  assert.deepEqual(order, ["urgent", "late", "same"]);
});

test("a task that throws or rejects frees its place", async () => {
  const limiter = createLimiter(1);
  await assert.rejects(
    limiter.run(() => {
      throw new Error("sync");
    }),
    /sync/,
  );
  await assert.rejects(limiter.run(() => Promise.reject(new Error("async"))));
  assert.equal(await limiter.run(async () => "still runs"), "still runs");
});

test("the latest read wins: an older answer that comes back later is dropped (« Vérifier à nouveau » shows its own read, not the one before)", async () => {
  const reads = latestOnly();
  const shown: string[] = [];
  const read = async (answer: string, delay: number) => {
    const ticket = reads.begin();
    await new Promise((resolve) => setTimeout(resolve, delay));
    if (reads.isLatest(ticket)) shown.push(answer);
  };
  // The opening read (before the image was dropped) is slow; the check
  // started after it answers first.
  await Promise.all([read("no final image", 30), read("final image", 5)]);
  assert.deepEqual(shown, ["final image"]);
  // A lone read is always the latest.
  await read("again", 0);
  assert.deepEqual(shown, ["final image", "again"]);
});

test("« Vérifier à nouveau » of « Fichiers »: its read is the latest, and the button waits for it", async () => {
  const { readFile } = await import("node:fs/promises");
  const wizard = await readFile(
    new URL("../screens/night/NightWizard.tsx", import.meta.url),
    "utf8",
  );
  const refresh = wizard.slice(wizard.indexOf("const refreshView"));
  const body = refresh.slice(0, refresh.indexOf("}, [input"));
  assert.match(body, /const ticket = viewReads\.begin\(\);/);
  assert.match(
    body,
    /if \(!viewReads\.isLatest\(ticket\)\) return null;\s+setView\(next\)/,
  );
  const files = await readFile(
    new URL("../screens/night/FilesStep.tsx", import.meta.url),
    "utf8",
  );
  assert.match(files, /disabled=\{checking\}/);
  assert.match(files, /"wizard\.files\.checking"/);
});
