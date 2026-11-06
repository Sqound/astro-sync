import assert from "node:assert/strict";
import test from "node:test";

import type {
  EnqueueAnswer,
  ImageCandidate,
  ImagePlan,
  NightPrecheck,
  QuotaCheck,
} from "./bridge";
import { sendNight, type SendCalls } from "./sendNight.ts";

const tick = () => new Promise<void>((done) => setTimeout(done, 0));

function gate<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((done, fail) => {
    resolve = done;
    reject = fail;
  });
  return { promise, resolve, reject };
}

const candidate = (name: string) => ({ name }) as unknown as ImageCandidate;
const quota = (allowed: boolean) =>
  ({ allowed, findings: [] }) as unknown as QuotaCheck;
/** What the check answers: a final image, a variant, and two progression
 * images of IC 1805 -- the history of the capture the variant's first
 * image creates -- and one with no capture to join. */
const checkedWith = (allowed = true) =>
  ({
    plans: [
      { image: { kind: "final" }, decision: { decision: "new_capture" } },
      {
        image: { kind: "stacked", targetId: "ic1805" },
        decision: { decision: "new_capture" },
      },
      ...["p1", "p2"].map((name) => ({
        image: { kind: "progression", targetId: "ic1805", name },
        decision: { decision: "progression_of_new", firstSha256: "e" },
      })),
      {
        image: { kind: "progression", targetId: "m31" },
        decision: {
          decision: "not_sent",
          reason: "no_capture_for_progression",
        },
      },
    ] as unknown as ImagePlan[],
    quota: quota(allowed),
  }) as unknown as NightPrecheck;

/** Calls that write what they do in `log`; the three treatments wait for
 * the test to release them. */
function recorder() {
  const log: string[] = [];
  const stacking = gate<ImageCandidate[]>();
  const progression = gate<{
    candidates: ImageCandidate[];
    frames: Record<string, number>;
    failure: { error: unknown } | null;
  }>();
  const logs = gate<unknown>();
  const prechecked: ImageCandidate[][] = [];
  const enqueued: ImagePlan[][] = [];
  const calls: SendCalls = {
    stacking: async () => {
      log.push("stacking:start");
      const result = await stacking.promise.finally(() =>
        log.push("stacking:end"),
      );
      return result;
    },
    progression: async () => {
      log.push("progression:start");
      return progression.promise.finally(() => log.push("progression:end"));
    },
    logs: async () => {
      log.push("logs:start");
      return logs.promise.finally(() => log.push("logs:end"));
    },
    precheck: async (extra) => {
      log.push("precheck");
      prechecked.push(extra);
      return checkedWith();
    },
    keep: (plan) => plan.image.kind !== "final",
    enqueue: async (plans) => {
      log.push("enqueue");
      enqueued.push(plans);
      return { result: "enqueued" } as unknown as EnqueueAnswer;
    },
    upload: async () => void log.push("upload"),
    integrations: null,
    planImage: null,
    solve: async () => {
      log.push("solve");
      return 3;
    },
    record: async () => void log.push("record"),
  };
  return {
    calls,
    log,
    stacking,
    progression,
    logs,
    prechecked,
    enqueued,
  };
}

test("the report of the logs and the progression images start with the stacking, not after it", async () => {
  const run = recorder();
  const sending = sendNight(run.calls);
  await tick();
  // The three are on their way together; nothing after them has begun.
  assert.deepEqual(run.log, [
    "stacking:start",
    "progression:start",
    "logs:start",
  ]);
  // The logs finish long before the stacking: the stacking is not waited for
  // to send the report.
  run.logs.resolve(null);
  await tick();
  assert.deepEqual(run.log.slice(3), ["logs:end"]);
  run.progression.resolve({
    candidates: [candidate("p1"), candidate("p2")],
    frames: { t1: 40 },
    failure: null,
  });
  await tick();
  assert.ok(!run.log.includes("precheck"), "the check waits for the stacking");
  run.stacking.resolve([candidate("s1")]);
  const result = await sending;
  // The check receives the images of both, then the rest follows in order.
  assert.deepEqual(
    run.prechecked[0].map((item) => (item as unknown as { name: string }).name),
    ["s1", "p1", "p2"],
  );
  assert.deepEqual(run.log.slice(run.log.indexOf("precheck")), [
    "precheck",
    "enqueue",
    "upload",
    "solve",
    "record",
  ]);
  assert.equal(result.kind, "sent");
  if (result.kind === "sent") {
    assert.equal(result.solved, 3);
    // Counted as progression images of their target, never as captures or
    // variants; the one with no capture does not leave.
    assert.equal(result.progressionImages, 2);
    assert.deepEqual(result.progressionByTarget, { ic1805: 2 });
    assert.deepEqual(result.failures, {
      logs: null,
      progression: null,
      multiNight: null,
    });
  }
  // The final image is not ticked: it is not queued.
  assert.equal(run.enqueued[0].length, 4);
});

test("a stacking that fails lets the others finish, then the sending stops with its error", async () => {
  const run = recorder();
  const sending = sendNight(run.calls);
  const settled = sending.then(
    () => "sent",
    (error: Error) => error.message,
  );
  await tick();
  run.stacking.reject(new Error("siril failed"));
  await tick();
  // Not answered yet: the report and the progression are still running.
  run.logs.resolve(null);
  await tick();
  assert.ok(run.log.includes("logs:end"));
  run.progression.resolve({ candidates: [], frames: {}, failure: null });
  assert.equal(await settled, "siril failed");
  assert.ok(!run.log.includes("precheck"));
  assert.ok(!run.log.includes("enqueue"));
});

test("a report or progression images that fail are returned and do not stop the images", async () => {
  const run = recorder();
  const sending = sendNight(run.calls);
  await tick();
  run.logs.reject(new Error("report refused"));
  run.progression.resolve({
    candidates: [candidate("p1")],
    frames: { t1: 12 },
    failure: { error: new Error("siril died on t2") },
  });
  run.stacking.resolve([]);
  const result = await sending;
  assert.equal(result.kind, "sent");
  if (result.kind === "sent") {
    assert.equal(
      (result.failures.logs?.error as Error).message,
      "report refused",
    );
    assert.equal(
      (result.failures.progression?.error as Error).message,
      "siril died on t2",
    );
    // What the progression made before failing still goes to the check.
    assert.equal(run.prechecked[0].length, 1);
  }
  assert.ok(run.log.includes("upload"));
});

test("a treatment that is not ticked is not run, and a refusal queues nothing", async () => {
  const log: string[] = [];
  const result = await sendNight({
    stacking: null,
    progression: null,
    logs: null,
    precheck: async (extra) => {
      log.push(`precheck:${extra.length}`);
      return checkedWith(false);
    },
    keep: () => true,
    enqueue: async () => {
      log.push("enqueue");
      return { result: "enqueued" } as unknown as EnqueueAnswer;
    },
    upload: async () => void log.push("upload"),
    solve: null,
    record: async () => void log.push("record"),
  });
  assert.equal(result.kind, "refused");
  assert.deepEqual(log, ["precheck:0"]);
});

test("a queue that refuses is returned as a refusal with its findings", async () => {
  const result = await sendNight({
    stacking: null,
    progression: null,
    logs: null,
    precheck: async () => checkedWith(true),
    keep: () => true,
    enqueue: async () =>
      ({
        result: "refused",
        findings: ["too big"],
      }) as unknown as EnqueueAnswer,
    upload: async () => assert.fail("nothing is uploaded"),
    solve: null,
    record: async () => assert.fail("nothing is recorded"),
  });
  assert.equal(result.kind, "refused");
  if (result.kind === "refused") {
    assert.equal(result.precheck.quota.allowed, false);
    assert.deepEqual(result.precheck.quota.findings, ["too big"]);
  }
});

test("the lines of a multi-night plan and its image come after the night's images; a refusal is returned, never an error", async () => {
  const run = recorder();
  run.calls.integrations = async () => {
    run.log.push("integrations");
    return [
      {
        result: "refused",
        projectId: "p-heart",
        code: "integration_outside_night",
        message: "not dated by its night",
      },
    ];
  };
  run.calls.planImage = async () => {
    run.log.push("planImage");
    return { "p-heart": { kind: "oneNight", nightId: "n-09" } };
  };
  const sending = sendNight(run.calls);
  await tick();
  run.logs.resolve(null);
  run.progression.resolve({ candidates: [], frames: {}, failure: null });
  run.stacking.resolve([]);
  const result = await sending;
  assert.deepEqual(run.log.slice(run.log.indexOf("upload")), [
    "upload",
    "integrations",
    "planImage",
    "solve",
    "record",
  ]);
  assert.equal(result.kind, "sent");
  if (result.kind === "sent") {
    assert.equal(result.multiNight.integrations[0].result, "refused");
    assert.deepEqual(result.multiNight.planImage, {
      "p-heart": { kind: "oneNight", nightId: "n-09" },
    });
    assert.equal(result.failures.multiNight, null);
  }
});

test("a failure of the image of a plan is said and the night is still sent", async () => {
  const run = recorder();
  run.calls.integrations = async () => [];
  run.calls.planImage = async () => {
    throw new Error("siril could not align the masters");
  };
  const sending = sendNight(run.calls);
  await tick();
  run.logs.resolve(null);
  run.progression.resolve({ candidates: [], frames: {}, failure: null });
  run.stacking.resolve([]);
  const result = await sending;
  assert.equal(result.kind, "sent");
  if (result.kind === "sent") {
    assert.equal(
      (result.failures.multiNight?.error as Error).message,
      "siril could not align the masters",
    );
  }
  assert.ok(run.log.includes("solve") && run.log.includes("record"));
});
