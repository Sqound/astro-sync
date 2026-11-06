import assert from "node:assert/strict";
import test from "node:test";

import { createTranslator } from "../i18n/locale.ts";
import type { QueueDriveEvent, QueueEntry } from "./bridge";
import {
  NO_UPLOADS,
  bannerLines,
  isLive,
  latestBatch,
  liveRows,
  pendingCount,
  percentOf,
  progressLine,
  reasonOf,
  runTarget,
  stateLine,
  titleLine,
  noticeRows,
  uploadNotices,
  DONE_NOTICE_KEY,
  noticeKey,
  noticeTone,
  uploadsReducer,
  type UploadsAction,
  type UploadsState,
} from "./uploads.ts";

const fr = createTranslator("fr");
const SHA = "a".repeat(64);

function play(actions: UploadsAction[], from: UploadsState = NO_UPLOADS) {
  return actions.reduce(uploadsReducer, from);
}
const event = (e: QueueDriveEvent, now: number): UploadsAction => ({
  type: "event",
  event: e,
  now,
});
const tracked = (extra: Record<string, unknown> = {}): UploadsAction => ({
  type: "track",
  origin: "detect",
  files: [
    {
      sha256: SHA,
      path: "/work/IC1805/stacked/IC1805_sho.tif",
      size: 46_500_000,
      target: "IC 1805",
      rendering: "Bibande",
      ...extra,
    },
  ],
});
const started = (now: number) =>
  event(
    {
      kind: "Started",
      sha256: SHA,
      sourcePath: "/work/IC1805/stacked/IC1805_sho.tif",
      bytes: 46_500_000,
    },
    now,
  );

test("a recorded sequence queued -> progress -> done: the row, the badge and the notification follow it", () => {
  let state = play([tracked()]);
  assert.equal(state.rows.length, 1);
  assert.equal(state.rows[0].status, "queued");
  assert.equal(state.rows[0].fileName, "IC1805_sho.tif");
  assert.equal(pendingCount(state), 1);
  // Queued is not yet a notification: the upload has not started.
  assert.equal(noticeRows(state).length, 0);

  state = play([started(1000)], state);
  assert.equal(state.rows[0].status, "uploading");
  assert.equal(noticeRows(state).length, 1, "the start is announced");
  assert.equal(pendingCount(state), 1);

  state = play(
    [
      event(
        { kind: "Progress", sha256: SHA, sent: 4_650_000, total: 46_500_000 },
        2000,
      ),
      event(
        { kind: "Progress", sha256: SHA, sent: 9_300_000, total: 46_500_000 },
        3000,
      ),
    ],
    state,
  );
  const row = state.rows[0];
  assert.equal(percentOf(row), 20);
  assert.ok(row.speed !== null && row.speed > 4_000_000, "bytes per second");
  assert.match(progressLine(fr, "fr", row), /9,3 Mo sur 46,5 Mo/);
  assert.match(progressLine(fr, "fr", row), /Mo\/s/);

  // The person closes the notification: the upload goes on.
  state = play([{ type: "dismiss", sha256: SHA }], state);
  assert.equal(noticeRows(state).length, 0);
  assert.equal(state.rows[0].status, "uploading");
  assert.equal(pendingCount(state), 1);

  state = play(
    [
      event(
        {
          kind: "Completed",
          sha256: SHA,
          captureId: "cap-1",
          deduplicated: false,
        },
        4000,
      ),
    ],
    state,
  );
  assert.equal(state.rows[0].status, "done");
  assert.equal(state.rows[0].captureId, "cap-1");
  assert.equal(pendingCount(state), 0, "the badge goes");
  assert.equal(noticeRows(state).length, 1, "the end is announced again");
  assert.equal(stateLine(fr, "fr", state.rows[0]), "Envoyée");
  assert.ok(state.revision > 0, "the page reads the queue again");
  assert.equal(isLive(state.rows[0]), false);
});

test("queued -> refused: the service's reason is said, with the size against the limit when it is known", () => {
  let state = play([tracked(), started(1000)]);
  state = play(
    [
      event(
        {
          kind: "Refused",
          sha256: SHA,
          code: "file_too_large",
          message: "413",
        },
        2000,
      ),
    ],
    state,
  );
  const row = state.rows[0];
  assert.equal(row.status, "refused");
  assert.equal(pendingCount(state), 0);
  assert.equal(noticeRows(state).length, 1);
  assert.match(stateLine(fr, "fr", row), /^Refusée : /);
  assert.match(reasonOf(fr, "fr", row), /trop volumineux/);
  state = play([{ type: "limit", sha256: SHA, bytes: 25_000_000 }], state);
  assert.equal(
    reasonOf(fr, "fr", state.rows[0]),
    "46,5 Mo > 25,0 Mo pour votre espace",
  );

  const quota = play([
    tracked(),
    event(
      {
        kind: "Refused",
        sha256: SHA,
        code: "capture_limit_reached",
        message: "",
      },
      1,
    ),
  ]);
  assert.match(reasonOf(fr, "fr", quota.rows[0]), /nombre de captures/);
});

test("a network failure stays on the page with « Réessayer » when the file is still in the queue, and goes to the history when it is not", () => {
  const entry = (state: QueueEntry["state"]): QueueEntry =>
    ({
      id: 1,
      sourcePath: "/work/x.tif",
      sha256: SHA,
      size: 46_500_000,
      uploadId: null,
      offset: 0,
      state,
      captureId: null,
      lastError: null,
      lastErrorCode: null,
      createdAt: "",
      updatedAt: "",
      kind: "stacked",
      target: { sessionId: null, targetIds: [], variantOf: null, source: null },
    }) as QueueEntry;
  let state = play([
    tracked(),
    started(1),
    event(
      { kind: "Failed", sha256: SHA, code: "network_error", message: "x" },
      2,
    ),
  ]);
  assert.equal(state.rows[0].status, "failed");
  assert.equal(stateLine(fr, "fr", state.rows[0]), "Échec réseau");
  const kept = play([{ type: "reconcile", entries: [entry("queued")] }], state);
  assert.equal(kept.rows[0].retryable, true);
  assert.equal(liveRows(kept).length, 1);
  const again = play([{ type: "requeue", sha256: SHA }], kept);
  assert.equal(again.rows[0].status, "queued");
  assert.equal(pendingCount(again), 1);

  const set = play([{ type: "reconcile", entries: [entry("failed")] }], state);
  assert.equal(set.rows[0].retryable, false);
  assert.equal(liveRows(set).length, 0);
});

test("a wait, a pause and a stop are said without losing the file", () => {
  let state = play([tracked(), started(1)]);
  state = play(
    [
      event(
        {
          kind: "Waiting",
          sha256: SHA,
          code: "network_error",
          message: "",
          retryInSeconds: 8,
          attempt: 1,
        },
        2,
      ),
    ],
    state,
  );
  assert.equal(state.rows[0].status, "waiting");
  assert.equal(pendingCount(state), 1);
  assert.match(stateLine(fr, "fr", state.rows[0]), /nouvel essai dans/);
  state = play(
    [event({ kind: "Stopped", sha256: null, reason: "paused" }, 3)],
    state,
  );
  assert.equal(state.rows[0].status, "queued");
  state = play(
    [
      started(4),
      event({ kind: "Stopped", sha256: SHA, reason: "no_progress" }, 5),
    ],
    state,
  );
  assert.equal(state.rows[0].status, "failed");
  assert.equal(
    stateLine(fr, "fr", state.rows[0]),
    "Échec : L’envoi n’avance plus.",
  );
  const held = play([
    tracked(),
    event(
      { kind: "Held", sha256: SHA, code: "storage_quota_reached", message: "" },
      1,
    ),
  ]);
  assert.equal(held.rows[0].status, "held");
  assert.equal(
    pendingCount(held),
    0,
    "a file that waits for the person is not counted",
  );
  assert.equal(liveRows(held).length, 1);
});

test("the files an earlier run left in the queue are seeded; the finished ones are not", () => {
  const base = {
    sourcePath: "/w/a.tif",
    size: 10,
    uploadId: null,
    offset: 4,
    captureId: null,
    lastError: null,
    lastErrorCode: null,
    createdAt: "",
    updatedAt: "",
    kind: "stacked",
    target: { sessionId: null, targetIds: [], variantOf: null, source: null },
  };
  const state = play([
    {
      type: "seed",
      entries: [
        { ...base, id: 1, sha256: "1", state: "queued" },
        { ...base, id: 2, sha256: "2", state: "completed" },
        {
          ...base,
          id: 3,
          sha256: "3",
          state: "held",
          lastErrorCode: "storage_quota_reached",
        },
      ] as QueueEntry[],
    },
  ]);
  assert.deepEqual(
    state.rows.map((row) => [row.sha256, row.status, row.sent]),
    [
      ["1", "queued", 4],
      ["3", "held", 4],
    ],
  );
});

test("notifications: one per upload, dismissed all at once on demand (the snackbar queue shows them one at a time)", () => {
  const files = Array.from({ length: 6 }, (_, i) => ({
    sha256: `s${i}`,
    path: `/w/${i}.tif`,
    size: 10,
  }));
  let state = play([{ type: "track", origin: "other", files }]);
  for (const file of files)
    state = play(
      [
        event(
          {
            kind: "Started",
            sha256: file.sha256,
            sourcePath: file.path,
            bytes: 10,
          },
          1,
        ),
      ],
      state,
    );
  assert.deepEqual(
    noticeRows(state).map((row) => row.sha256),
    ["s0", "s1", "s2", "s3", "s4", "s5"],
  );
  state = play([{ type: "dismissAll" }], state);
  assert.equal(noticeRows(state).length, 0);
  assert.equal(pendingCount(state), 6);
});

test("the banner of the result tells the truth: leaving with its percent, then sent, then refused", () => {
  const lines = (state: UploadsState) =>
    bannerLines(fr, "fr", latestBatch(state, "detect")).map(
      (line) => line.text,
    );
  let state = play([tracked()]);
  assert.deepEqual(lines(state), ["1 image mise en file d’envoi"]);
  state = play(
    [
      started(1000),
      event(
        { kind: "Progress", sha256: SHA, sent: 5_580_000, total: 46_500_000 },
        2000,
      ),
    ],
    state,
  );
  assert.deepEqual(lines(state), ["1 image en cours d’envoi · 12 %"]);
  const sent = play(
    [
      event(
        { kind: "Completed", sha256: SHA, captureId: "c", deduplicated: false },
        3,
      ),
    ],
    state,
  );
  assert.deepEqual(lines(sent), ["1 image envoyée"]);
  assert.equal(
    bannerLines(fr, "fr", latestBatch(sent, "detect"))[0].tone,
    "success",
  );
  const refused = play(
    [
      event(
        {
          kind: "Refused",
          sha256: SHA,
          code: "capture_limit_reached",
          message: "",
        },
        3,
      ),
    ],
    state,
  );
  const [line] = bannerLines(fr, "fr", latestBatch(refused, "detect"));
  assert.equal(line.tone, "error");
  assert.match(line.text, /^1 image refusée : Pas ajoutée/);
});

test("a row is « cible · rendu · taille », or the name of the file", () => {
  const state = play([tracked()]);
  assert.equal(
    titleLine(fr, "fr", state.rows[0]),
    "IC 1805 · Bibande · 46,5 Mo",
  );
  const bare = play([
    {
      type: "track",
      origin: "other",
      files: [{ sha256: "z", path: "C:\\w\\a.tif", size: 2000 }],
    },
  ]);
  assert.equal(titleLine(fr, "fr", bare.rows[0]), "a.tif · 2,0 ko");
});

test("the run is attributed to the session of the images when they all share one", () => {
  const sessions = [
    { sessionId: "s1", planId: "p1" },
    { sessionId: "s2", planId: null },
  ];
  assert.deepEqual(runTarget(sessions, ["s1", "s1", null]), {
    sessionId: "s1",
    planId: "p1",
    targetIds: [],
  });
  assert.deepEqual(runTarget(sessions, ["s1", "s2"]), {
    sessionId: null,
    planId: null,
    targetIds: [],
  });
  // A plan's image carries its plan itself: no session to give.
  assert.deepEqual(runTarget(sessions, [null]), {
    sessionId: null,
    planId: null,
    targetIds: [],
  });
});

test("the batches: the banner reads the latest gesture of the detection only", () => {
  const state = play([
    tracked(),
    {
      type: "track",
      origin: "detect",
      files: [{ sha256: "second", path: "/w/b.tif", size: 5 }],
    },
    {
      type: "track",
      origin: "other",
      files: [{ sha256: "third", path: "/w/c.tif", size: 5 }],
    },
  ]);
  assert.deepEqual(
    latestBatch(state, "detect").map((row) => row.sha256),
    ["second"],
  );
});

const handlers = () => {
  const calls = {
    opened: 0,
    retried: [] as string[],
    closed: [] as string[][],
  };
  return {
    calls,
    handlers: {
      open: () => {
        calls.opened += 1;
      },
      retry: (sha: string) => calls.retried.push(sha),
      close: (shas: string[]) => calls.closed.push(shas),
    },
  };
};

test("the snackbars of the uploads: the one on its way has a bar, persistent; sent is timed and says it once; refused and failed stay, with « Réessayer » when possible", () => {
  const { calls, handlers: h } = handlers();
  // On its way.
  let state = play(
    [
      tracked(),
      started(1000),
      event(
        { kind: "Progress", sha256: SHA, sent: 9_300_000, total: 46_500_000 },
        2000,
      ),
    ],
    NO_UPLOADS,
  );
  let notices = uploadNotices(fr, "fr", state, h);
  assert.equal(notices.length, 1);
  const running = notices[0];
  assert.equal(running.key, noticeKey(SHA));
  assert.equal(running.mode, "persistent");
  assert.equal(running.progress, 20);
  assert.equal(running.tone, "info");
  assert.match(running.detail ?? "", /9,3 Mo sur 46,5 Mo/);
  assert.equal(running.action?.label, "Voir les envois");
  running.action?.run();
  assert.equal(calls.opened, 1);
  // The cross is the owner's to hear: the upload row only loses its notice.
  running.onClose?.();
  assert.deepEqual(calls.closed, [[SHA]]);

  // Sent: one timed snackbar for the whole group, with the link.
  state = play(
    [
      event(
        { kind: "Completed", sha256: SHA, captureId: "c", deduplicated: false },
        3000,
      ),
    ],
    state,
  );
  notices = uploadNotices(fr, "fr", state, h);
  assert.equal(notices.length, 1);
  assert.equal(notices[0].key, DONE_NOTICE_KEY);
  assert.equal(notices[0].tone, "success");
  assert.equal(notices[0].progress, undefined);
  assert.equal(notices[0].mode, "timed");
  assert.equal(notices[0].detail, "Envoyée");
  assert.equal(notices[0].action?.label, "Voir les envois");

  // Dismissed: nothing is asked for.
  state = play([{ type: "dismiss", sha256: SHA }], state);
  assert.deepEqual(uploadNotices(fr, "fr", state, h), []);
});

test("several files sent make one message; a network failure offers « Réessayer », a refusal the page", () => {
  const { calls, handlers: h } = handlers();
  const files = ["s0", "s1", "s2"].map((sha256) => ({
    sha256,
    path: `/w/${sha256}.tif`,
    size: 10,
  }));
  let state = play([{ type: "track", origin: "other", files }]);
  for (const file of files) {
    state = play(
      [
        event(
          {
            kind: "Started",
            sha256: file.sha256,
            sourcePath: file.path,
            bytes: 10,
          },
          1,
        ),
      ],
      state,
    );
  }
  state = play(
    [
      event(
        {
          kind: "Completed",
          sha256: "s0",
          captureId: "c0",
          deduplicated: false,
        },
        2,
      ),
      event(
        {
          kind: "Completed",
          sha256: "s1",
          captureId: "c1",
          deduplicated: false,
        },
        3,
      ),
      event(
        { kind: "Failed", sha256: "s2", code: "network_error", message: "x" },
        4,
      ),
    ],
    state,
  );
  const notices = uploadNotices(fr, "fr", state, h);
  const sent = notices.find((notice) => notice.key === DONE_NOTICE_KEY);
  assert.equal(sent?.text, "2 images envoyées");
  assert.equal(sent?.detail, undefined);
  sent?.onClose?.();
  assert.deepEqual(calls.closed, [["s0", "s1"]]);
  const failed = notices.find((notice) => notice.key === noticeKey("s2"));
  assert.equal(failed?.tone, "error");
  assert.equal(failed?.mode, "persistent");
  assert.equal(failed?.detail, "Échec réseau");
  assert.equal(failed?.action?.label, "Réessayer");
  failed?.action?.run();
  assert.deepEqual(calls.retried, ["s2"]);

  // Refused: a reason, no retry, the page.
  state = play(
    [
      { type: "track", origin: "other", files: [files[0]] },
      event(
        { kind: "Started", sha256: "s0", sourcePath: "/w/s0.tif", bytes: 10 },
        5,
      ),
      event(
        {
          kind: "Refused",
          sha256: "s0",
          code: "file_too_large",
          message: "413",
        },
        6,
      ),
    ],
    state,
  );
  const refused = uploadNotices(fr, "fr", state, h).find(
    (notice) => notice.key === noticeKey("s0"),
  );
  assert.match(refused?.detail ?? "", /^Refusée\s:\s/);
  assert.equal(refused?.action?.label, "Voir les envois");
  assert.equal(noticeTone({ status: "held" }), "warning");
});

test("a file that is gone says its name and folder, and « Réessayer » stays possible", () => {
  const path = "/w/Sh2-103 · 80340a/progression/plan_night-01_120-frames.jpg";
  const failed = play([
    tracked({ path }),
    event(
      { kind: "Started", sha256: SHA, sourcePath: path, bytes: 800 },
      1_000,
    ),
    event(
      {
        kind: "Failed",
        sha256: SHA,
        code: "source_file_missing",
        message: "File not found: plan_night-01_120-frames.jpg, it was in ...",
      },
      2_000,
    ),
  ]);
  const row = failed.rows[0];
  assert.equal(row.status, "failed");
  assert.equal(row.path, path);
  assert.equal(
    reasonOf(fr, "fr", row),
    "Fichier introuvable\u00a0: plan_night-01_120-frames.jpg, il était dans /w/Sh2-103 · 80340a/progression.",
  );
  assert.match(stateLine(fr, "fr", row), /^Échec\u00a0: Fichier introuvable/);
  // The queue keeps such a file as `failed`: the retry stays possible.
  const entries: QueueEntry[] = [
    {
      id: 1,
      sourcePath: path,
      sha256: SHA,
      size: 800,
      uploadId: null,
      offset: 0,
      state: "failed",
      captureId: null,
      lastError: "could not read the source file ...",
      lastErrorCode: "source_file_missing",
      createdAt: "2026-11-02T11:03:00Z",
      updatedAt: "2026-11-02T11:03:00Z",
      kind: "progression",
      target: { sessionId: null, targetIds: [], variantOf: null, source: null },
    },
  ];
  const settled = play([{ type: "reconcile", entries }], failed);
  assert.equal(settled.rows[0].retryable, true);
  // Another failure the queue set aside for good is not retryable.
  const other = play(
    [
      {
        type: "reconcile",
        entries: [{ ...entries[0], lastErrorCode: null }],
      },
    ],
    failed,
  );
  assert.equal(other.rows[0].retryable, false);
  const again = play([{ type: "requeue", sha256: SHA }], settled);
  assert.equal(again.rows[0].status, "queued");
});
