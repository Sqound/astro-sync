import assert from "node:assert/strict";
import test from "node:test";

import {
  SAME_ERROR_GAP_MS,
  describeError,
  installErrorReporting,
  type ErrorEventLike,
  type ErrorTarget,
} from "./errorReport.ts";

// What the window hands to the core when it meets an uncaught error.

function fakeWindow() {
  const listeners: Record<string, (event: ErrorEventLike) => void> = {};
  const target: ErrorTarget = {
    addEventListener(type, listener) {
      listeners[type] = listener;
    },
  };
  return { target, listeners };
}

test("an Error gives its message and its stack, a string its text, anything else a fixed phrase", () => {
  const error = new Error("boom");
  assert.deepEqual(describeError(error), {
    message: "boom",
    stack: error.stack ?? null,
  });
  assert.deepEqual(describeError("plain"), { message: "plain", stack: null });
  assert.deepEqual(describeError({ path: "/Us\u0065rs/camille/x" }), {
    message: "Uncaught value",
    stack: null,
  });
  assert.deepEqual(describeError(undefined), {
    message: "Uncaught value",
    stack: null,
  });
  assert.equal(describeError("x".repeat(9000)).message.length, 4000);
});

test("an uncaught error and an unhandled rejection are handed over, the same one once per gap", () => {
  const { target, listeners } = fakeWindow();
  const handed: Array<[string, string | null]> = [];
  let clock = 1_000;
  installErrorReporting(
    target,
    async (message, stack) => {
      handed.push([message, stack]);
    },
    () => clock,
  );
  listeners.error({ error: new Error("first"), message: "ignored" });
  listeners.error({ message: "from the message" });
  listeners.unhandledrejection({ reason: "rejected" });
  assert.deepEqual(
    handed.map(([message]) => message),
    ["first", "from the message", "rejected"],
  );
  // The same error again, at once: not handed over twice.
  listeners.unhandledrejection({ reason: "rejected" });
  assert.equal(handed.length, 3);
  clock += SAME_ERROR_GAP_MS;
  listeners.unhandledrejection({ reason: "rejected" });
  assert.equal(handed.length, 4);
});

test("a failure of the hand-over, asynchronous or not, is swallowed", async () => {
  const { target, listeners } = fakeWindow();
  installErrorReporting(target, () => Promise.reject(new Error("no bridge")));
  assert.doesNotThrow(() => listeners.error({ error: new Error("one") }));
  const synchronous = fakeWindow();
  installErrorReporting(synchronous.target, () => {
    throw new Error("synchronous");
  });
  assert.doesNotThrow(() =>
    synchronous.listeners.error({ error: new Error("two") }),
  );
  await new Promise((resolve) => setTimeout(resolve, 5));
});
