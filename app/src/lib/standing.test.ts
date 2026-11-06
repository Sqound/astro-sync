import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

import { SUPPORTED_LOCALES, createTranslator } from "../i18n/locale.ts";
import type { CapabilitiesState, Standing } from "./bridge";
import {
  NO_UPLOADS,
  bannerLines,
  reasonOf,
  stateLine,
  uploadsReducer,
  type UploadsAction,
  type UploadsState,
} from "./uploads.ts";
import {
  READ_ONLY_CODE,
  beforeSendingText,
  heldWindow,
  paymentFailedLine,
  paymentFailedNotice,
  paymentFailedWindow,
  untilText,
  windowOfState,
} from "./standing.ts";

// The exchange `entitlements-payment-failed` of the contract. A subscription
// whose payment failed, ten days into its hold: read-only until 29/12/2026 (90
// days after the hold started on 30/09/2026).
const RECORDED: Standing = {
  kind: "paymentFailed",
  since: "2026-09-30T12:00:00.000Z",
  readOnlyUntil: "2026-12-29T12:00:00.000Z",
};
const TEN_DAYS_IN = Date.parse("2026-10-10T12:00:00.000Z");
const AFTER_WINDOW = Date.parse("2026-12-30T00:00:00.000Z");

const fr = createTranslator("fr");
const SHA = "a".repeat(64);

test("the window of a payment that failed lasts until readOnlyUntil, then says nothing", () => {
  const window = paymentFailedWindow(RECORDED, TEN_DAYS_IN);
  assert.equal(window?.until, "2026-12-29T12:00:00.000Z");
  assert.equal(window?.untilMs, Date.parse("2026-12-29T12:00:00.000Z"));
  assert.equal(paymentFailedWindow(RECORDED, AFTER_WINDOW), null);
  assert.equal(paymentFailedWindow(null, TEN_DAYS_IN), null);
  assert.equal(paymentFailedWindow(undefined, TEN_DAYS_IN), null);
  assert.equal(
    paymentFailedWindow({ ...RECORDED, readOnlyUntil: "bientôt" }, TEN_DAYS_IN),
    null,
    "a date that is not an instant is not shown as one",
  );
});

test("only a paired computer carries the standing of its space", () => {
  const paired: CapabilitiesState = {
    paired: true,
    capabilities: null,
    standing: RECORDED,
  };
  assert.ok(windowOfState(paired, TEN_DAYS_IN));
  assert.equal(windowOfState({ ...paired, paired: false }, TEN_DAYS_IN), null);
  assert.equal(windowOfState({ ...paired, standing: null }, TEN_DAYS_IN), null);
  // A core older than the field does not send it.
  const { standing: _standing, ...older } = paired;
  assert.equal(windowOfState(older, TEN_DAYS_IN), null);
  assert.equal(windowOfState(null, TEN_DAYS_IN), null);
});

test("the state is said in the words of the phone, with the day, in every language", () => {
  const window = paymentFailedWindow(RECORDED, TEN_DAYS_IN)!;
  for (const locale of SUPPORTED_LOCALES) {
    const t = createTranslator(locale);
    const day = untilText(locale, window);
    assert.match(day, /2026|２０２６/u, `${locale}: ${day}`);
    for (const text of [
      paymentFailedLine(t, locale, window),
      paymentFailedNotice(t, locale, window),
      beforeSendingText(t, locale, window),
    ]) {
      assert.ok(text.includes(day), `${locale}: « ${text} » lacks ${day}`);
      assert.doesNotMatch(text, /\{date\}|standing\./u, `${locale}: ${text}`);
    }
  }
});

test("in French: « Paiement en échec · lecture seule jusqu’au … », the sentence of the shell, and the line before the last step", () => {
  const window = paymentFailedWindow(RECORDED, TEN_DAYS_IN)!;
  const day = untilText("fr", window);
  assert.match(day, /^29 décembre 2026$/);
  assert.equal(
    paymentFailedLine(fr, "fr", window),
    "Paiement en échec · lecture seule jusqu’au 29 décembre 2026",
  );
  assert.equal(
    paymentFailedNotice(fr, "fr", window),
    "Votre paiement n’a pas abouti : votre espace est en lecture seule jusqu’au 29 décembre 2026, puis il redevient gratuit. Mettez votre moyen de paiement à jour pour retrouver aussitôt tous vos droits.",
  );
  assert.equal(
    beforeSendingText(fr, "fr", window),
    "Votre espace est en lecture seule jusqu’au 29 décembre 2026 : rien ne sera envoyé.",
  );
});

test("a file held for the read-only space is held with the payment's reason while the window lasts", () => {
  assert.ok(heldWindow(READ_ONLY_CODE, RECORDED.readOnlyUntil, TEN_DAYS_IN));
  assert.equal(
    heldWindow("storage_quota_reached", RECORDED.readOnlyUntil, TEN_DAYS_IN),
    null,
  );
  assert.equal(heldWindow(READ_ONLY_CODE, null, TEN_DAYS_IN), null);
  assert.equal(
    heldWindow(READ_ONLY_CODE, RECORDED.readOnlyUntil, AFTER_WINDOW),
    null,
  );
});

const play = (actions: UploadsAction[], from: UploadsState = NO_UPLOADS) =>
  actions.reduce(uploadsReducer, from);
/** A French sentence with its no-break spaces read as plain ones. */
const plain = (text: string) => text.replaceAll("\u00a0", " ");
const tracked: UploadsAction = {
  type: "track",
  origin: "detect",
  files: [
    { sha256: SHA, path: "/w/IC1805.tif", size: 1000, target: "IC 1805" },
  ],
};
const held = (code: string): UploadsAction => ({
  type: "event",
  event: { kind: "Held", sha256: SHA, code, message: "read only" },
  now: 1,
});

test("the service holds a file back (403 entitlement_read_only) in a space whose payment failed: held, with the payment's reason, no « Réessayer »", () => {
  const state = play([
    { type: "standing", until: RECORDED.readOnlyUntil },
    tracked,
    held(READ_ONLY_CODE),
  ]);
  const row = state.rows[0];
  assert.equal(row.status, "held", "held, not refused or failed");
  assert.equal(row.retryable, false, "nothing to try again");
  assert.equal(row.heldUntil, RECORDED.readOnlyUntil);
  const line = plain(stateLine(fr, "fr", row));
  assert.match(
    line,
    /^En pause\s: votre paiement n’a pas abouti\s: votre espace est en lecture seule jusqu’au 29 décembre 2026, puis il redevient gratuit\./,
  );
  assert.doesNotMatch(line, /pris fin/, "never « l'abonnement a pris fin »");
  const banner = bannerLines(fr, "fr", state.rows);
  assert.deepEqual(
    banner.map((item) => item.tone),
    ["warning"],
  );
  assert.match(
    plain(banner[0].text),
    /image en pause\s: votre paiement n’a pas abouti/,
  );
});

test("the standing read after the hold still explains it, and the end of the window gives the ordinary sentence back", () => {
  let state = play([tracked, held(READ_ONLY_CODE)]);
  assert.equal(state.rows[0].heldUntil, null);
  assert.match(stateLine(fr, "fr", state.rows[0]), /pris fin/);
  state = play([{ type: "standing", until: RECORDED.readOnlyUntil }], state);
  assert.equal(state.rows[0].heldUntil, RECORDED.readOnlyUntil);
  assert.match(
    reasonOf(fr, "fr", state.rows[0], TEN_DAYS_IN),
    /^votre paiement n’a pas abouti/,
  );
  // The window is over on the clock: the ordinary sentence of the refusal.
  assert.match(reasonOf(fr, "fr", state.rows[0], AFTER_WINDOW), /pris fin/);
  // The payment resumed: the standing is gone.
  state = play([{ type: "standing", until: null }], state);
  assert.equal(state.rows[0].heldUntil, null);
});

test("another hold does not take the payment's reason", () => {
  const state = play([
    { type: "standing", until: RECORDED.readOnlyUntil },
    tracked,
    held("storage_quota_reached"),
  ]);
  assert.equal(state.rows[0].heldUntil, null);
  assert.doesNotMatch(stateLine(fr, "fr", state.rows[0]), /paiement/);
});

test("the wiring: the state is shown on the Espace page, the card of Envois and before the last step of a night and of an import", () => {
  const source = (relative: string) =>
    readFileSync(new URL(`../${relative}`, import.meta.url), "utf8");
  assert.match(
    source("screens/space/ConnectedSpace.tsx"),
    /<PaymentFailedNotice variant="state" \/>/,
  );
  assert.match(
    source("screens/QueueScreen.tsx"),
    /<PaymentFailedNotice variant="state" \/>/,
  );
  assert.match(
    source("screens/night/SummaryStep.tsx"),
    /<PaymentFailedNotice variant="beforeSending" \/>/,
  );
  assert.match(
    source("screens/detect/ImportSummary.tsx"),
    /<PaymentFailedNotice variant="beforeSending" \/>/,
  );
  // The provider that reads it once is above the uploads, which say it.
  const app = source("App.tsx");
  assert.ok(
    app.indexOf("<CapabilitiesProvider>") < app.indexOf("<UploadsProvider>"),
  );
  assert.match(
    source("components/UploadsProvider.tsx"),
    /type: "standing", until: failedUntil/,
  );
});
