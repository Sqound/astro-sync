import assert from "node:assert/strict";
import test from "node:test";

import {
  NO_SNACKS,
  SNACKBAR_ACTION_DURATION_MS,
  SNACKBAR_DURATION_MS,
  SNACKBAR_MAX,
  clampPercent,
  dismissSnack,
  dismissSnackKey,
  durationOf,
  modeOf,
  pushSnack,
  roleOf,
  visibleSnack,
} from "./snackbar.ts";

// The snackbars of the window.

test("an error waits for its cross; the rest closes by itself", () => {
  assert.equal(modeOf({ tone: "error" }), "persistent");
  assert.equal(modeOf({ tone: "success" }), "timed");
  assert.equal(modeOf({ tone: "info" }), "timed");
  assert.equal(modeOf({ tone: "warning" }), "timed");
  // The owner may say otherwise (the service's wait stays until it ends).
  assert.equal(modeOf({ tone: "info", mode: "persistent" }), "persistent");
  assert.equal(modeOf({ tone: "error", mode: "timed" }), "timed");
});

test("a timed snackbar stays its fixed duration, longer with a gesture", () => {
  assert.equal(durationOf({ mode: "persistent" }), null);
  assert.equal(durationOf({ mode: "timed" }), SNACKBAR_DURATION_MS);
  assert.equal(
    durationOf({ mode: "timed", action: { label: "Voir", run: () => {} } }),
    SNACKBAR_ACTION_DURATION_MS,
  );
  assert.ok(SNACKBAR_DURATION_MS >= 4000, "time to read a line");
  assert.ok(SNACKBAR_ACTION_DURATION_MS > SNACKBAR_DURATION_MS);
});

test("snackbars queue in the order they came, the oldest going past the maximum", () => {
  let state = NO_SNACKS;
  const ids: number[] = [];
  for (let n = 0; n < SNACKBAR_MAX + 1; n += 1) {
    const pushed = pushSnack(state, { tone: "success", text: `n${n}` });
    state = pushed.state;
    ids.push(pushed.id);
  }
  assert.equal(new Set(ids).size, ids.length, "each its own id");
  assert.equal(state.snacks.length, SNACKBAR_MAX);
  assert.deepEqual(
    state.snacks.map((snack) => snack.text),
    Array.from({ length: SNACKBAR_MAX }, (_, n) => `n${n + 1}`),
  );
});

test("one snackbar per key: the new one takes its place and keeps its id", () => {
  let state = pushSnack(NO_SNACKS, {
    tone: "info",
    text: "Attente : 1 s…",
    key: "service",
  }).state;
  state = pushSnack(state, { tone: "success", text: "Copié" }).state;
  const again = pushSnack(state, {
    tone: "info",
    text: "Attente : 2 s…",
    key: "service",
  });
  assert.equal(again.id, state.snacks[0].id);
  assert.deepEqual(
    again.state.snacks.map((snack) => snack.text),
    ["Attente : 2 s…", "Copié"],
  );
});

test("a snackbar goes by its id or by its key, the others stay", () => {
  let state = pushSnack(NO_SNACKS, { tone: "info", text: "a", key: "k" }).state;
  const second = pushSnack(state, { tone: "error", text: "b" });
  state = second.state;
  assert.deepEqual(
    dismissSnack(state, second.id).snacks.map((snack) => snack.text),
    ["a"],
  );
  assert.deepEqual(
    dismissSnackKey(state, "k").snacks.map((snack) => snack.text),
    ["b"],
  );
  // Closing one already gone changes nothing.
  assert.equal(dismissSnack(state, 999).snacks.length, 2);
});

test("an error interrupts a screen reader, the rest is polite", () => {
  assert.equal(roleOf("error"), "alert");
  assert.equal(roleOf("warning"), "status");
  assert.equal(roleOf("success"), "status");
  assert.equal(roleOf("info"), "status");
});

test("one snackbar shows at a time: the running one first, the others after it in their order", () => {
  assert.equal(visibleSnack([]), null);
  let state = pushSnack(NO_SNACKS, { tone: "success", text: "Copié" }).state;
  state = pushSnack(state, { tone: "error", text: "Dossier" }).state;
  assert.equal(visibleSnack(state.snacks)?.text, "Copié", "first come");
  const upload = pushSnack(state, {
    tone: "info",
    mode: "persistent",
    text: "M 31",
    progress: 20,
    key: "upload:a",
  });
  state = upload.state;
  assert.equal(visibleSnack(state.snacks)?.text, "M 31", "running first");
  // Done: the running one goes, the others come back in their order.
  state = dismissSnackKey(state, "upload:a");
  assert.equal(visibleSnack(state.snacks)?.text, "Copié");
  state = pushSnack(state, {
    tone: "success",
    text: "1 image envoyée",
    key: "upload:done",
  }).state;
  state = dismissSnack(state, state.snacks[0].id);
  assert.equal(visibleSnack(state.snacks)?.text, "Dossier");
});

test("a running upload stays (persistent, no time); a finished one closes by itself; the cross closes only that snackbar and tells its owner", () => {
  const running = pushSnack(NO_SNACKS, {
    tone: "info",
    text: "M 31",
    mode: "persistent",
    progress: 40,
    key: "upload:a",
  });
  const snack = running.state.snacks[0];
  assert.equal(durationOf(snack), null, "waits for its cross");
  // It turns into the finished message in place: same turn, now timed.
  const done = pushSnack(running.state, {
    tone: "success",
    text: "M 31",
    key: "upload:a",
    action: { label: "Voir les envois", run: () => {} },
  });
  assert.equal(done.id, running.id);
  assert.equal(done.state.snacks[0].progress, undefined);
  assert.equal(durationOf(done.state.snacks[0]), SNACKBAR_ACTION_DURATION_MS);
  // The cross removes the one snackbar, the queue behind it stays.
  let state = pushSnack(running.state, { tone: "info", text: "Copié" }).state;
  state = dismissSnack(state, snack.id);
  assert.deepEqual(
    state.snacks.map((item) => item.text),
    ["Copié"],
  );
});

test("the same message again with the same key changes nothing, so its time does not start again", () => {
  const first = pushSnack(NO_SNACKS, {
    tone: "success",
    text: "1 image envoyée",
    key: "upload:done",
    action: { label: "Voir les envois", run: () => {} },
  });
  let ran = 0;
  const again = pushSnack(first.state, {
    tone: "success",
    text: "1 image envoyée",
    key: "upload:done",
    action: { label: "Voir les envois", run: () => (ran += 1) },
  });
  assert.equal(again.state, first.state, "the very same state");
  assert.equal(again.id, first.id);
  // ... with the newest gesture kept.
  again.state.snacks[0].action?.run();
  assert.equal(ran, 1);
  // A new percent is a change.
  const bar = pushSnack(NO_SNACKS, {
    tone: "info",
    text: "a",
    key: "k",
    progress: 10,
  });
  assert.notEqual(
    pushSnack(bar.state, { tone: "info", text: "a", key: "k", progress: 11 })
      .state,
    bar.state,
  );
});

test("the percent is kept to 0-100 and the running snackbar is never the one dropped past the maximum", () => {
  assert.equal(clampPercent(-3), 0);
  assert.equal(clampPercent(250), 100);
  assert.equal(clampPercent(33.9), 33);
  assert.equal(clampPercent(Number.NaN), 0);
  let state = pushSnack(NO_SNACKS, {
    tone: "info",
    text: "run",
    progress: 5,
    mode: "persistent",
  }).state;
  for (let n = 0; n < SNACKBAR_MAX + 2; n += 1) {
    state = pushSnack(state, { tone: "success", text: `n${n}` }).state;
  }
  assert.equal(state.snacks.length, SNACKBAR_MAX);
  assert.equal(state.snacks[0].text, "run");
});
