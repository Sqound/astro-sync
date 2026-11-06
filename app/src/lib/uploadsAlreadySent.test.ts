import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

import { createTranslator, SUPPORTED_LOCALES } from "../i18n/locale.ts";
import type { QueueEntry } from "./bridge";
import {
  ALREADY_SENT_NOTICE_KEY,
  DONE_NOTICE_KEY,
  NO_UPLOADS,
  bannerLines,
  hasSomethingToSend,
  isLive,
  liveRows,
  pendingCount,
  stateLine,
  uploadNotices,
  uploadsReducer,
  type UploadsAction,
  type UploadsState,
} from "./uploads.ts";

// The queue belongs to the linked space: a file this space already has ends
// at once (« Déjà envoyé le … », « Voir la capture », « Renvoyer »), instead of
// staying « En file d'envoi » for a run that has nothing to send (06/11).

const fr = createTranslator("fr");
const SHA = "b".repeat(64);
const OTHER = "c".repeat(64);
const SENT_AT = "2026-11-05T17:53:00Z";

function play(actions: UploadsAction[], from: UploadsState = NO_UPLOADS) {
  return actions.reduce(uploadsReducer, from);
}

const tracked: UploadsAction = {
  type: "track",
  origin: "detect",
  files: [
    {
      sha256: SHA,
      path: "/w/Sh2-103/stacked/Sh2-103_plan_hoo.tif",
      size: 1_000,
      target: "Sh2-103",
      rendering: "HOO",
    },
    { sha256: OTHER, path: "/w/Sh2-103/stacked/raw.tif", size: 2_000 },
  ],
};

const entry = (
  sha256: string,
  state: QueueEntry["state"],
  captureId: string | null = null,
): QueueEntry =>
  ({
    id: 1,
    sourcePath: "/w/x.tif",
    sha256,
    size: 1_000,
    uploadId: null,
    offset: 0,
    state,
    captureId,
    lastError: null,
    lastErrorCode: null,
    createdAt: SENT_AT,
    updatedAt: SENT_AT,
    kind: "stacked",
    target: { sessionId: null, targetIds: [], variantOf: null, source: null },
  }) as QueueEntry;

test("the files the enqueueing found already sent end at once, with their capture and their date", () => {
  const state = play([
    tracked,
    {
      type: "alreadySent",
      files: [{ sha256: SHA, captureId: "cap-1", sentAt: SENT_AT }],
    },
  ]);
  const row = state.rows.find((item) => item.sha256 === SHA)!;
  assert.equal(row.status, "done");
  assert.equal(row.captureId, "cap-1");
  assert.equal(row.alreadySentAt, SENT_AT);
  const line = stateLine(fr, "fr", row);
  assert.match(line, /^Déjà envoyé le /);
  assert.doesNotMatch(line, /file d/);
  // Still on the page (with « Renvoyer »), no longer counted as leaving.
  assert.ok(isLive(row));
  assert.equal(pendingCount(state), 1, "only the other file is to send");
  assert.equal(
    liveRows(state).filter((item) => item.status === "queued").length,
    1,
  );
});

test("a run that found nothing to send ends the rows the queue of the space says sent", () => {
  const state = play([
    tracked,
    {
      type: "reconcile",
      entries: [entry(SHA, "completed", "cap-1"), entry(OTHER, "queued")],
    },
  ]);
  const sent = state.rows.find((item) => item.sha256 === SHA)!;
  assert.equal(sent.status, "done");
  assert.equal(sent.alreadySentAt, SENT_AT);
  assert.equal(
    state.rows.find((item) => item.sha256 === OTHER)!.status,
    "queued",
  );
});

test("a file this run sends is not « already sent », even when the queue was read before its event", () => {
  const state = play([
    tracked,
    { type: "reconcile", entries: [entry(SHA, "completed", "cap-1")] },
    {
      type: "event",
      now: 1,
      event: {
        kind: "Completed",
        sha256: SHA,
        captureId: "cap-1",
        deduplicated: false,
        skipped: [],
        replaced: false,
      },
    } as UploadsAction,
  ]);
  const row = state.rows.find((item) => item.sha256 === SHA)!;
  assert.equal(row.alreadySentAt, null);
  assert.equal(stateLine(fr, "fr", row), fr("uploads.row.done"));
});

test("« Renvoyer » queues the file again; the service then says whether it still had it", () => {
  let state = play([
    tracked,
    {
      type: "alreadySent",
      files: [{ sha256: SHA, captureId: "cap-1", sentAt: SENT_AT }],
    },
    { type: "resend", sha256: SHA },
  ]);
  let row = state.rows.find((item) => item.sha256 === SHA)!;
  assert.equal(row.status, "queued");
  assert.equal(row.alreadySentAt, null);
  state = play(
    [
      {
        type: "event",
        now: 2,
        event: {
          kind: "Completed",
          sha256: SHA,
          captureId: "cap-1",
          deduplicated: true,
          skipped: [],
          replaced: false,
        },
      } as UploadsAction,
    ],
    state,
  );
  row = state.rows.find((item) => item.sha256 === SHA)!;
  assert.equal(stateLine(fr, "fr", row), "Déjà dans votre espace");
});

test("the files already sent have one notice and one banner line of their own, never « envoyée »", () => {
  const state = play([
    tracked,
    {
      type: "alreadySent",
      files: [
        { sha256: SHA, captureId: "cap-1", sentAt: SENT_AT },
        { sha256: OTHER, captureId: "cap-2", sentAt: SENT_AT },
      ],
    },
  ]);
  const notices = uploadNotices(fr, "fr", state, {
    open: () => undefined,
    retry: () => undefined,
    close: () => undefined,
  });
  assert.ok(notices.some((notice) => notice.key === ALREADY_SENT_NOTICE_KEY));
  assert.ok(!notices.some((notice) => notice.key === DONE_NOTICE_KEY));
  const lines = bannerLines(fr, "fr", state.rows).map((line) => line.text);
  assert.deepEqual(lines, ["2 fichiers déjà envoyés à cet espace"]);
});

test("« Tout envoyer » has something to start only when the queue of the space holds a file to send", () => {
  assert.equal(hasSomethingToSend([]), false);
  assert.equal(hasSomethingToSend([entry(SHA, "completed")]), false);
  assert.equal(
    hasSomethingToSend([entry(SHA, "completed"), entry(OTHER, "refused")]),
    false,
  );
  assert.equal(hasSomethingToSend([entry(SHA, "queued")]), true);
  assert.equal(hasSomethingToSend([entry(SHA, "uploading")]), true);
  assert.equal(hasSomethingToSend([entry(SHA, "held")]), true);
});

test("the button says why nothing starts, and the page wires « Renvoyer » and « Avant la liaison »", () => {
  const source = (relative: string) =>
    readFileSync(new URL(`../${relative}`, import.meta.url), "utf8");
  const provider = source("components/UploadsProvider.tsx");
  const start = provider.slice(provider.indexOf("const start = useCallback"));
  assert.match(start, /hasSomethingToSend\(status\.entries\)/);
  assert.match(start, /say\("uploads\.nothingToSend"\)/);
  // Nothing to send: the run is not started (the button does not flicker).
  assert.ok(
    start.indexOf("return;") < start.indexOf("void drain();"),
    "the notice returns before the run",
  );
  const screen = source("screens/QueueScreen.tsx");
  assert.match(screen, /t\("uploads\.row\.resend"\)/);
  assert.match(screen, /t\("uploads\.beforeLink\.title"\)/);
  // A row still queued before the link is « to import again », never « waiting ».
  assert.match(screen, /t\("uploads\.beforeLink\.reimport"\)/);
  assert.match(screen, /queueForgetBeforeLink\(\)/);
  const flow = source("components/ImportProvider.tsx");
  assert.match(flow, /enqueued\?\.alreadySent/);
});

test("the new words exist in every language", () => {
  for (const locale of SUPPORTED_LOCALES) {
    const t = createTranslator(locale);
    for (const key of [
      "uploads.row.alreadySent",
      "uploads.row.resend",
      "uploads.nothingToSend",
      "uploads.alreadySent.one",
      "uploads.alreadySent.other",
      "uploads.beforeLink.title",
      "uploads.beforeLink.description",
      "uploads.beforeLink.reimport",
      "errors.project_not_found",
      "errors.variant_capture_refused",
    ])
      assert.notEqual(t(key), key, `${locale}: ${key}`);
    assert.match(t("uploads.row.alreadySent", { date: "X" }), /X/);
  }
});
