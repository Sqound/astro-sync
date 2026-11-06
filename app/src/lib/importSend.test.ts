import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

import type { ImageToSend, StackedRender } from "./bridge.ts";
import { imageKey, summaryImages } from "./detectTreatments.ts";
import type { TreatmentResult } from "./detectTreatments.ts";
import { importSends, renderOf, variantCount } from "./importSend.ts";

// Two renders of one plan ticked in step 4 are one capture and
// one variant, the capture first. An earlier import sent them as two
// captures because the new import no longer named the capture of the variant.

const render = (sha: string): StackedRender =>
  ({ sha256: sha, path: `/stacked/${sha}.tif`, size: 1 }) as StackedRender;

function planImage(
  targetId: string,
  pipelineId: string,
  sha: string,
  contrast?: number,
) {
  const result: TreatmentResult = {
    targetId,
    designation: "IC 1805",
    pipelineId,
    contrast,
    nights: 3,
    path: `/stacked/${sha}.tif`,
    size: 46_500_000,
  };
  const send: ImageToSend = {
    kind: "plan",
    outcome: { render: render(sha), targetId } as never,
  };
  return { ...result, send };
}

function nightImage(targetId: string, pipelineId: string, sha: string) {
  const result: TreatmentResult = {
    targetId,
    designation: "M31",
    pipelineId,
    nights: 1,
    path: `/stacked/${sha}.tif`,
    size: 10_000_000,
  };
  const send: ImageToSend = {
    kind: "night",
    sessionId: "night-1",
    targetId,
    render: render(sha),
  };
  return { ...result, send };
}

test("two renders of one plan ticked: one capture, then one variant that names it", () => {
  const made = [
    planImage("ic1805", "raw", "a".repeat(64)),
    planImage("ic1805", "dualband", "b".repeat(64)),
  ];
  const rows = summaryImages(made, null);
  const sends = importSends(
    rows,
    made,
    rows.map((row) => row.key),
  );
  assert.equal(sends.length, 2);
  assert.equal(sends[0].variant, false);
  assert.equal(
    (sends[0].send as { variantOfSha256?: string }).variantOfSha256,
    undefined,
    "the first image is the capture",
  );
  assert.equal(sends[1].variant, true);
  assert.equal(sends[1].send.kind, "plan");
  assert.equal(
    (sends[1].send as { variantOfSha256?: string }).variantOfSha256,
    "a".repeat(64),
  );
  assert.equal(renderOf(sends[0].send).sha256, "a".repeat(64));
  assert.equal(variantCount(sends), 1);
});

test("the natural render is the head when it is ticked, whatever the order they were made in", () => {
  const made = [
    planImage("ic1805", "dualband", "b".repeat(64)),
    planImage("ic1805", "raw", "a".repeat(64)),
  ];
  const rows = summaryImages(made, null);
  const sends = importSends(
    rows,
    made,
    rows.map((row) => row.key),
  );
  assert.equal(renderOf(sends[0].send).sha256, "a".repeat(64));
  assert.equal(sends[1].variant, true);
  assert.equal(
    (sends[1].send as { variantOfSha256?: string }).variantOfSha256,
    "a".repeat(64),
  );
});

test("only the second one ticked: it leaves as the capture, alone", () => {
  const made = [
    planImage("ic1805", "raw", "a".repeat(64)),
    planImage("ic1805", "dualband", "b".repeat(64)),
  ];
  const rows = summaryImages(made, null);
  const sends = importSends(rows, made, [rows[1].key]);
  assert.equal(sends.length, 1);
  assert.equal(sends[0].variant, false);
  assert.equal(
    (sends[0].send as { variantOfSha256?: string }).variantOfSha256,
    undefined,
  );
});

test("each target has its own head; the images of the night's wizard keep their relation", () => {
  const made = [
    nightImage("m31", "raw", "1".repeat(64)),
    nightImage("m31", "dualband", "2".repeat(64)),
    planImage("ic1805", "raw", "a".repeat(64)),
  ];
  const rows = summaryImages(made, null);
  const sends = importSends(
    rows,
    made,
    rows.map((row) => row.key),
  );
  assert.deepEqual(
    sends.map((item) => [renderOf(item.send).sha256[0], item.variant]),
    [
      ["1", false],
      ["2", true],
      ["a", false],
    ],
  );
  const second = sends[1].send;
  assert.equal(second.kind, "night");
  assert.equal(
    (second as { variantOfSha256?: string }).variantOfSha256,
    "1".repeat(64),
  );
  assert.equal(variantCount(sends), 1);
  // The key of a row is its target and its file, as the page ticks it.
  assert.equal(imageKey(made[0]), rows[0].key);
});

test("nothing ticked, nothing queued", () => {
  const made = [planImage("ic1805", "raw", "a".repeat(64))];
  assert.deepEqual(importSends(summaryImages(made, null), made, []), []);
});

function source(relative: string) {
  return readFileSync(new URL(`../${relative}`, import.meta.url), "utf8");
}

test("the primary of step 4 queues through importSends and step 5 is told how many variants left", () => {
  const provider = source("components/ImportProvider.tsx");
  assert.match(provider, /importSends\(rows, made, held\.ticked\)/);
  assert.match(provider, /variantCount\(chosen\)/);
  assert.match(
    provider,
    /dispatch\(\{ type: "sent", count: chosen\.length, variants \}\)/,
  );
  const done = source("screens/detect/ImportDone.tsx");
  assert.match(done, /import\.done\.sentSplit/);
  // Both kinds of image carry the digest of the capture they are a variant of
  // across the bridge, and the Tauri side reads it for both.
  assert.match(source("lib/bridge.ts"), /kind: "plan";[\s\S]*variantOfSha256/);
  const rust = readFileSync(
    new URL("../../src-tauri/src/commands_detect.rs", import.meta.url),
    "utf8",
  );
  assert.match(rust, /Plan \{[\s\S]*variant_of_sha256: Option<String>/);
});

test("the night's wizard does not go through this path", () => {
  // It queues with `precheck::enqueue_checked` (Decision::VariantOfNew), not
  // with the images of the import.
  assert.doesNotMatch(source("screens/night/NightWizard.tsx"), /importSends/);
});

test("the progression of a plan travels with the image that is its capture", () => {
  const step = {
    nightId: "n-1",
    eveningDate: "2026-08-09",
    nights: 1,
    frames: 98,
    seconds: 17640,
    jpeg: "/plan/progression/a.jpg",
    sha256: "c".repeat(64),
    size: 3,
  };
  const made = [
    planImage("ic1805", "raw", "a".repeat(64)),
    planImage("ic1805", "dualband", "b".repeat(64)),
  ].map((item) => ({
    ...item,
    send: { ...item.send, progression: [step] } as ImageToSend,
  }));
  const rows = summaryImages(made, null);
  const sends = importSends(
    rows,
    made,
    rows.map((row) => row.key),
  );
  const carried = sends.map(
    (item) =>
      item.send as { progression?: unknown[]; variantOfSha256?: string },
  );
  // Both carry it; the service side keeps it only for the capture (no
  // `variantOfSha256`), so the history is queued once, with the capture.
  assert.equal(carried[0].variantOfSha256, undefined);
  assert.equal(carried[0].progression?.length, 1);
  assert.equal(carried[1].variantOfSha256, "a".repeat(64));
  assert.equal(carried[1].progression?.length, 1);
});
