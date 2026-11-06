import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

import type { Capabilities, CapabilitiesState } from "./bridge.ts";
import {
  analysisSendingNotice,
  analysisSendingNoticeKey,
  isAnalysisSendingHeld,
} from "./capabilities.ts";

const BASE_CAPABILITIES: Capabilities = {
  "planner.multiple": true,
  "planner.mosaic": true,
  "pointing.solve": true,
  "sync.plan_limit": 1000,
  "gallery.private": true,
  "capture.variant_limit": 4,
  "analysis.logs": false,
  "analysis.progression": false,
  "storage.max_file_bytes": 104_857_600,
  "storage.quota_bytes": 53_687_091_200,
  "portal.invitations": false,
  "capture.count_limit": 9007199254740991,
  "capture.public": true,
  "history.retention_days": 90,
};

const ENTITLED: CapabilitiesState = {
  paired: true,
  capabilities: {
    ...BASE_CAPABILITIES,
    "analysis.logs": true,
    "analysis.progression": true,
  },
};

const STANDARD: CapabilitiesState = {
  paired: true,
  capabilities: BASE_CAPABILITIES,
};

const UNKNOWN: CapabilitiesState = {
  paired: true,
  capabilities: null,
};

const NOT_PAIRED: CapabilitiesState = {
  paired: false,
  capabilities: null,
};

test("the sending of a report is held only when the paired service says it keeps none", () => {
  assert.equal(isAnalysisSendingHeld(STANDARD), true);
  assert.equal(isAnalysisSendingHeld(ENTITLED), false);
  // Not known yet, or not paired: nothing is held back -- the service answers
  // (`analysis_not_entitled`) when a report is sent.
  assert.equal(isAnalysisSendingHeld(UNKNOWN), false);
  assert.equal(isAnalysisSendingHeld(NOT_PAIRED), false);
  assert.equal(isAnalysisSendingHeld(null), false);
  assert.equal(isAnalysisSendingHeld(undefined), false);
});

test("the sentence that says the analysis is here and free is the hosted service's or a private server's", () => {
  assert.equal(analysisSendingNotice(STANDARD, true), "hosted");
  assert.equal(analysisSendingNotice(STANDARD, false), "private");
  for (const hosted of [true, false]) {
    assert.equal(analysisSendingNotice(ENTITLED, hosted), null);
    assert.equal(analysisSendingNotice(UNKNOWN, hosted), null);
    assert.equal(analysisSendingNotice(NOT_PAIRED, hosted), null);
    assert.equal(analysisSendingNotice(null, hosted), null);
  }
  assert.equal(
    analysisSendingNoticeKey("hosted"),
    "analysis.sendingHeldHosted",
  );
  assert.equal(
    analysisSendingNoticeKey("private"),
    "analysis.sendingHeldPrivate",
  );
});

/** The application never hides a screen or a tab on a capability of the
 * account: every entry of the menu is always there. « Progression » and
 * « Journaux » stay visible without
 * `analysis.progression` / `analysis.logs`: their page opens and says the
 * reserve itself. */
test("no screen of Astro Sync is gated on the account's capabilities", async () => {
  const read = (relative: string) =>
    readFile(new URL(relative, import.meta.url), "utf8");
  const app = await read("../App.tsx");
  for (const gone of [
    "visibleScreens",
    "isAnalysisEntitled",
    "CAPABILITY_GATED_SCREEN_IDS",
    "shouldShowAnalysisAdvancedOfferNotice",
  ]) {
    assert.ok(!app.includes(gone), `App.tsx still says ${gone}`);
  }
  // Every tab is listed, and the two reserved tabs are rendered without
  // asking what the account holds: the page reads the capability itself.
  assert.ok(app.includes('{screen === "progression" && ('));
  assert.ok(app.includes('{screen === "logs" && ('));
  assert.ok(app.includes("MENU_TOP.map") && app.includes("MENU_GROUPS.map"));
  const capabilities = await read("./capabilities.ts");
  assert.ok(!capabilities.includes("visibleScreens"));
  const en = JSON.parse(await read("../i18n/en.json")) as Record<
    string,
    string
  >;
  assert.equal(en["settings.analysisAdvancedOfferNotice"], undefined);
});
