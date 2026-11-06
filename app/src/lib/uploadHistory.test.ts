import assert from "node:assert/strict";
import test from "node:test";

import type { QueueEntry } from "./bridge";
import {
  LABELS_MAX,
  capturePage,
  clearedAfter,
  historyRows,
  loadClearedAt,
  loadLabels,
  rememberLabels,
  saveClearedAt,
  saveLabels,
} from "./uploadHistory.ts";

function memory() {
  const kept = new Map<string, string>();
  return {
    getItem: (key: string) => kept.get(key) ?? null,
    setItem: (key: string, value: string) => void kept.set(key, value),
    kept,
  };
}

const entry = (
  id: number,
  state: QueueEntry["state"],
  updatedAt: string,
  extra: Partial<QueueEntry> = {},
): QueueEntry =>
  ({
    id,
    sourcePath: `/w/${id}.tif`,
    sha256: `sha${id}`,
    size: 1000 * id,
    uploadId: null,
    offset: 0,
    state,
    captureId: state === "completed" ? `cap${id}` : null,
    lastError: null,
    lastErrorCode: null,
    createdAt: updatedAt,
    updatedAt,
    kind: "stacked",
    target: { sessionId: null, targetIds: [], variantOf: null, source: null },
    ...extra,
  }) as QueueEntry;

test("the history is the finished uploads of the queue, newest first, with what the gesture said of them", () => {
  const labels = rememberLabels(
    {},
    [{ sha256: "sha2", target: "IC 1805", rendering: "Bibande" }],
    1,
  );
  const rows = historyRows(
    [
      entry(1, "completed", "2026-10-22T10:00:00Z"),
      entry(2, "refused", "2026-10-23T07:00:00Z", {
        lastErrorCode: "file_too_large",
      }),
      entry(3, "queued", "2026-10-23T08:00:00Z"),
      entry(4, "held", "2026-10-23T08:00:00Z"),
      entry(5, "completed", "2026-10-23T06:00:00Z"),
      entry(6, "failed", "2026-10-21T06:00:00Z"),
    ],
    labels,
    null,
  );
  assert.deepEqual(
    rows.map((row) => row.sha256),
    ["sha2", "sha5", "sha1", "sha6"],
  );
  assert.equal(rows[0].target, "IC 1805");
  assert.equal(rows[0].rendering, "Bibande");
  assert.equal(rows[0].state, "refused");
  assert.equal(rows[1].captureId, "cap5");
  assert.equal(rows[1].fileName, "5.tif");
});

test("clearing hides what is finished up to now, keeps what ends later, and survives a reopening", () => {
  const storage = memory();
  const entries = [
    entry(1, "completed", "2026-10-22T10:00:00Z"),
    entry(2, "refused", "2026-10-23T07:00:00Z"),
    entry(3, "queued", "2026-10-23T09:00:00Z"),
  ];
  const cleared = clearedAfter(entries, loadClearedAt(storage));
  assert.equal(
    cleared,
    "2026-10-23T07:00:00Z",
    "the queued file is not history",
  );
  saveClearedAt(storage, cleared);
  // Window reopened.
  const reopened = loadClearedAt(storage);
  assert.equal(historyRows(entries, {}, reopened).length, 0);
  const later = [...entries, entry(4, "completed", "2026-10-23T09:30:00Z")];
  assert.deepEqual(
    historyRows(later, {}, reopened).map((row) => row.sha256),
    ["sha4"],
  );
  // Nothing finished: nothing to clear, the earlier line stays.
  assert.equal(clearedAfter([entries[2]], reopened), reopened);
});

test("the labels are kept per computer, the oldest forgotten, and an empty storage is not an error", () => {
  const storage = memory();
  assert.deepEqual(loadLabels(storage), {});
  assert.deepEqual(loadLabels(null), {});
  let labels = rememberLabels({}, [{ sha256: "a", target: "M 31" }], 1);
  saveLabels(storage, labels);
  assert.equal(loadLabels(storage).a.target, "M 31");
  // A file with nothing to say adds nothing.
  assert.deepEqual(rememberLabels(labels, [{ sha256: "b" }], 2), labels);
  const many = Array.from({ length: LABELS_MAX + 5 }, (_, i) => ({
    sha256: `k${i}`,
    target: "T",
  }));
  const spread = Object.fromEntries(
    many.map((item, i) => [
      item.sha256,
      { target: "T", rendering: null, at: i },
    ]),
  );
  const trimmed = rememberLabels(
    spread,
    [{ sha256: "new", target: "N" }],
    99999,
  );
  assert.equal(Object.keys(trimmed).length, LABELS_MAX);
  assert.ok(trimmed.new);
  assert.ok(!trimmed.k0, "the oldest went");
  storage.setItem("astro-sync.upload-labels", "not json");
  assert.deepEqual(loadLabels(storage), {});
});

test("the capture link opens the capture in the space, in the language of the window", () => {
  assert.equal(
    capturePage("https://portal.example/", "fr", "abc"),
    "https://portal.example/fr/manage/captures/abc",
  );
});
