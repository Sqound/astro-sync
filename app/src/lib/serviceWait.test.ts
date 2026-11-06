// The banner that says "the service is starting" is driven by a pure
// reducer and a few lookups, so it can be tested under `node --test` like
// `capabilities.ts` (JSX cannot be type-stripped; this module has none).

import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

import { SUPPORTED_LOCALES, type Locale } from "../i18n/locale.ts";
import {
  IDLE,
  PLAIN_FAILURE_CODES,
  elapsedSeconds,
  isWaiting,
  reduceServiceWait,
} from "./serviceWait.ts";

async function loadDictionary(locale: Locale): Promise<Record<string, string>> {
  const raw = await readFile(
    new URL(`../i18n/${locale}.json`, import.meta.url),
    "utf8",
  );
  return JSON.parse(raw) as Record<string, string>;
}

test("the banner appears when the service starts and disappears when it answers", () => {
  assert.equal(isWaiting(IDLE), false);
  const waiting = reduceServiceWait(IDLE, { kind: "starting" }, 1_000);
  assert.equal(isWaiting(waiting), true);
  const done = reduceServiceWait(waiting, { kind: "ready" }, 5_000);
  assert.equal(isWaiting(done), false);
});

test("a wait that ends without an answer (gave up, cancelled) also clears the banner", () => {
  const waiting = reduceServiceWait(IDLE, { kind: "starting" }, 0);
  assert.equal(
    isWaiting(reduceServiceWait(waiting, { kind: "stopped" }, 9)),
    false,
  );
});

test("two calls waiting at once keep the banner until both are over", () => {
  // The startup refresh and a screen's own request can both be waiting on
  // the same slow service: the first to be answered must not hide the
  // banner while the other still waits.
  let state = reduceServiceWait(IDLE, { kind: "starting" }, 1_000);
  state = reduceServiceWait(state, { kind: "starting" }, 3_000);
  state = reduceServiceWait(state, { kind: "ready" }, 4_000);
  assert.equal(isWaiting(state), true);
  assert.equal(
    state.since,
    1_000,
    "the elapsed time counts from the first call",
  );
  state = reduceServiceWait(state, { kind: "ready" }, 5_000);
  assert.equal(isWaiting(state), false);
  assert.equal(state.since, null);
});

test("an answer nobody announced never drives the count below zero", () => {
  const state = reduceServiceWait(IDLE, { kind: "ready" }, 100);
  assert.equal(isWaiting(state), false);
  assert.equal(
    isWaiting(reduceServiceWait(state, { kind: "starting" }, 200)),
    true,
    "the next real wait still shows the banner",
  );
});

test("elapsed time is whole seconds since the wait began", () => {
  const waiting = reduceServiceWait(IDLE, { kind: "starting" }, 10_000);
  assert.equal(elapsedSeconds(waiting, 10_000), 0);
  assert.equal(elapsedSeconds(waiting, 10_999), 0);
  assert.equal(elapsedSeconds(waiting, 12_000), 2);
  assert.equal(elapsedSeconds(waiting, 9_000), 0, "never negative");
  assert.equal(elapsedSeconds(IDLE, 50_000), 0);
});

test("every message the wait can end with exists in every language", async () => {
  const keys = [
    "service.starting",
    "service.startingElapsed",
    "errors.cancelled",
    "errors.network_error",
    "queue.stoppedCancelled",
    "queue.failedNoticeKnown",
    ...[...PLAIN_FAILURE_CODES].map((code) => `errors.${code}`),
  ];
  for (const locale of SUPPORTED_LOCALES) {
    const dictionary = await loadDictionary(locale);
    for (const key of keys) {
      assert.ok(
        typeof dictionary[key] === "string" && dictionary[key].length > 0,
        `${locale}.json is missing "${key}"`,
      );
    }
  }
});

test("the waiting messages are plain words: no jargon, no raw technical text, polite in French", async () => {
  const fr = await loadDictionary("fr");
  const technical =
    /http|timeout|timed out|reqwest|5\d\d|ingress|conteneur|container|serveur/i;
  for (const locale of SUPPORTED_LOCALES) {
    const dictionary = await loadDictionary(locale);
    for (const key of [
      "service.starting",
      "errors.service_start_timeout",
      "errors.cancelled",
      "errors.service_write_unconfirmed",
      "queue.stoppedCancelled",
    ]) {
      assert.doesNotMatch(dictionary[key], technical, `${locale} ${key}`);
    }
  }
  // Vouvoiement: the texts shown to the person never use the informal "tu".
  for (const key of [
    "service.starting",
    "errors.service_start_timeout",
    "errors.cancelled",
    "errors.service_write_unconfirmed",
    "queue.stoppedCancelled",
  ]) {
    assert.doesNotMatch(fr[key], /\b(tu|te|ton|ta|tes|toi)\b/i, key);
  }
  assert.match(
    fr["service.starting"],
    /^Le service démarre\. Cela peut prendre une minute\.$/,
  );
});
