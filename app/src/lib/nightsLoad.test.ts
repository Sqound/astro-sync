import assert from "node:assert/strict";
import test from "node:test";

import type {
  NightDetail,
  NightOnline,
  NightView,
  SessionSummary,
} from "./bridge";
import {
  FOLDER_READS,
  SERVICE_READS,
  readNights,
  type NightRead,
  type NightReaders,
} from "./nightsLoad.ts";

const tick = () => new Promise<void>((done) => setTimeout(done, 0));

function gate<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => {
    resolve = done;
  });
  return { promise, resolve };
}

const sessionsOf = (count: number) =>
  Array.from({ length: count }, (_, index) => ({
    id: `night-${index}`,
    startedAt: `2026-10-${String(20 - index).padStart(2, "0")}T20:00:00Z`,
  })) as unknown as SessionSummary[];

/** Readers whose answers are released by hand, which count the calls. */
function manualReaders() {
  const detail = new Map<string, ReturnType<typeof gate<NightDetail>>>();
  const online = new Map<string, ReturnType<typeof gate<NightOnline>>>();
  const view = new Map<string, ReturnType<typeof gate<NightView>>>();
  const calls: string[] = [];
  const running = { service: 0, folder: 0 };
  const peak = { service: 0, folder: 0 };
  const track = async <T>(
    kind: "service" | "folder",
    label: string,
    from: Map<string, ReturnType<typeof gate<T>>>,
    id: string,
  ): Promise<T> => {
    calls.push(`${label}:${id}`);
    running[kind] += 1;
    peak[kind] = Math.max(peak[kind], running[kind]);
    const entry = gate<T>();
    from.set(id, entry);
    try {
      return await entry.promise;
    } finally {
      running[kind] -= 1;
    }
  };
  const readers: NightReaders = {
    detail: (id) => track("service", "detail", detail, id),
    online: (id) => track("service", "online", online, id),
    view: (session) => track("folder", "view", view, session.id),
  };
  const answer = async (id: string) => {
    // The three answers of a night, one after the other.
    for (const [map, value] of [
      [detail, { id } as unknown as NightDetail],
      [online, { targets: [] } as unknown as NightOnline],
      [view, { id } as unknown as NightView],
    ] as const) {
      while (!map.has(id)) await tick();
      (map.get(id) as ReturnType<typeof gate<never>>).resolve(value as never);
      await tick();
    }
  };
  return { readers, calls, peak, answer, detail, online, view };
}

test("the reads start together under the two limits", async () => {
  const manual = manualReaders();
  const sessions = sessionsOf(10);
  const reading = readNights({
    sessions,
    readInFull: 12,
    readers: manual.readers,
    isCurrent: () => true,
    onRead: () => undefined,
  });
  await tick();
  // Not one after the other: four calls to the service are on their way at
  // once, the most recent nights first; the folder is not read yet.
  assert.equal(manual.peak.service, SERVICE_READS);
  assert.deepEqual(manual.calls, [
    "detail:night-0",
    "online:night-0",
    "detail:night-1",
    "online:night-1",
  ]);
  for (const session of sessions) await manual.answer(session.id);
  const reads = await reading;
  assert.equal(reads?.length, 10);
  assert.ok(manual.peak.service <= SERVICE_READS);
  assert.ok(manual.peak.folder <= FOLDER_READS);
});

test("a night that answers before an older one is filled before it and the rows keep their order", async () => {
  const manual = manualReaders();
  const sessions = sessionsOf(4);
  const filled: number[] = [];
  const reading = readNights({
    sessions,
    readInFull: 12,
    readers: manual.readers,
    isCurrent: () => true,
    onRead: (read: NightRead) => filled.push(read.index),
  });
  await tick();
  // Four places for the service: nights 0 and 1 hold them. Night 0 answers,
  // then night 2, then night 3, and night 1 -- started before them -- is the
  // last to answer.
  await manual.answer("night-0");
  await manual.answer("night-2");
  await manual.answer("night-3");
  await manual.answer("night-1");
  const reads = await reading;
  assert.deepEqual(filled, [0, 2, 3, 1]);
  // The result is in the order of the list, each night with its own answers.
  assert.deepEqual(
    reads?.map((read) => read.index),
    [0, 1, 2, 3],
  );
  assert.deepEqual(
    reads?.map((read) => (read.detail as unknown as { id: string }).id),
    ["night-0", "night-1", "night-2", "night-3"],
  );
});

test("only the most recent nights have their folder read (READ_IN_FULL)", async () => {
  const manual = manualReaders();
  const sessions = sessionsOf(3);
  const reading = readNights({
    sessions,
    readInFull: 1,
    readers: manual.readers,
    isCurrent: () => true,
    onRead: () => undefined,
  });
  await manual.answer("night-0");
  for (const id of ["night-1", "night-2"]) {
    while (!manual.detail.has(id)) await tick();
    manual.detail.get(id)!.resolve({ id } as unknown as NightDetail);
  }
  const reads = await reading;
  assert.equal(reads?.[1].view, null);
  assert.ok(!manual.calls.includes("online:night-1"));
  assert.ok(!manual.calls.includes("view:night-2"));
});

test("a reading replaced in the meantime starts no more calls and fills no more rows (generation)", async () => {
  const manual = manualReaders();
  const sessions = sessionsOf(10);
  let current = true;
  const filled: number[] = [];
  const reading = readNights({
    sessions,
    readInFull: 12,
    readers: manual.readers,
    isCurrent: () => current,
    onRead: (read) => filled.push(read.index),
  });
  await tick();
  await manual.answer("night-0");
  assert.deepEqual(filled, [0]);
  const before = manual.calls.length;
  current = false;
  // The calls already on their way finish; nothing new starts.
  for (const map of [manual.detail, manual.online, manual.view]) {
    for (const entry of map.values()) entry.resolve({} as never);
  }
  for (let round = 0; round < 5; round += 1) await tick();
  assert.equal(await reading, null);
  assert.deepEqual(filled, [0]);
  assert.equal(manual.calls.length, before);
});

test("a night that cannot be read still fills its row (detail or folder failed)", async () => {
  const sessions = sessionsOf(2);
  const reads = await readNights({
    sessions,
    readInFull: 12,
    readers: {
      detail: async (id) =>
        id === "night-0"
          ? Promise.reject(new Error("offline"))
          : ({ id } as unknown as NightDetail),
      online: async () => Promise.reject(new Error("offline")),
      view: async () => ({}) as NightView,
    },
    isCurrent: () => true,
    onRead: () => undefined,
  });
  assert.equal(reads?.[0].detail, null);
  assert.equal(reads?.[0].view, null);
  assert.equal(reads?.[1].view, null);
});
