import assert from "node:assert/strict";
import test from "node:test";

import { createTranslator } from "../i18n/locale.ts";
import type { CapabilitiesState, Pipeline } from "./bridge";
import { CAPABILITIES_REFRESH_GAP_MS, offerChanged } from "./capabilities.ts";
import {
  NO_IMPORT,
  footRule,
  importPresence,
  importReducer,
  noneSentReason,
  type ImportAction,
} from "./importFlow.ts";
import {
  eveningOfSession,
  nightCell,
  nightHeader,
  nightSentence,
  oneFinalStop,
} from "./nights.ts";
import { pipelineDialogView } from "./pipelineChoice.ts";
import { PIPELINES, tools } from "./homeFixtures.ts";

// The import as the person lives it: the date of a night, the
// offer announced before the treatments, the reading that says what it does,
// the preview of a pipeline.

const fr = createTranslator("fr");
const en = createTranslator("en");
const play = (actions: ImportAction[]) =>
  actions.reduce(importReducer, NO_IMPORT);

/* ------------------------- SY-12: one date per night --------------------- */

test("a night has one date, the evening, in two forms: « Mer. 7 oct. » in a cell, « 7 oct. » in a sentence -- never a weekday in lower case, never 10/21/26", () => {
  assert.equal(nightCell("2026-10-07", "fr"), "Mer. 7 oct.");
  assert.equal(nightSentence("2026-10-07", "fr"), "7 oct.");
  assert.equal(nightCell("2026-10-07", "en"), "Wed, Oct 7");
  assert.equal(nightSentence("2026-10-07", "en"), "Oct 7");
  assert.equal(
    en("wizard.title", { day: nightSentence("2026-10-07", "en") }),
    "Night of Oct 7",
  );
  assert.equal(
    fr("wizard.title", { day: nightSentence("2026-10-07", "fr") }),
    "Nuit du 7 oct.",
  );
  assert.equal(nightSentence("nope", "fr"), "—");
  // The evening of the service wins; else the rule of the service (the start
  // at the place's longitude, noon to noon) -- not the computer's time zone.
  assert.equal(
    eveningOfSession({
      startedAt: "2026-10-07T19:05:00Z",
      eveningDate: "2026-10-07",
    }),
    "2026-10-07",
  );
  for (const startedAt of ["2026-10-07T19:05:00Z", "2026-10-08T02:40:00Z"]) {
    assert.equal(eveningOfSession({ startedAt }), "2026-10-07", startedAt);
    // Tokyo (+9 h) would read the 8th for the first start: the night is the 7th.
    assert.equal(eveningOfSession({ startedAt }, 139.7), "2026-10-07");
  }
});

test("the title of the wizard and the line of the summary never double the full stop of an abbreviation", () => {
  assert.equal(
    oneFinalStop("Ce qui complétera la session du 7 oct.."),
    "Ce qui complétera la session du 7 oct.",
  );
  assert.equal(
    oneFinalStop("What will complete the session of Oct 7."),
    "What will complete the session of Oct 7.",
  );
  assert.equal(oneFinalStop("Wait…"), "Wait…");
  const header = nightHeader(fr, "fr", {
    session: {
      startedAt: "2026-10-07T19:05:00Z",
      finishedAt: "2026-10-08T02:40:00Z",
      status: "finished",
      planId: null,
      eveningDate: "2026-10-07",
    },
    detail: null,
    online: null,
  });
  assert.equal(header.title, "Nuit du 7 oct.");
});

/* --------------------- SY-21: the reading says what it does -------------- */

test("the search for the files counts what it finds and never shows « 0 sur 0 » nor a « 0 % »", () => {
  const started = play([{ type: "open" }, { type: "scanStart" }]);
  assert.equal(started.scan?.total, 0);
  assert.equal(started.scan?.walked, 0);
  const walking = play([
    { type: "open" },
    { type: "scanStart" },
    { type: "scanWalk", found: 412, folder: "/home/astro/Astro/ASIAIR" },
  ]);
  assert.equal(walking.scan?.walked, 412);
  assert.equal(walking.scan?.folder, "/home/astro/Astro/ASIAIR");
  // No percentage while nothing can be measured: the menu shows none.
  assert.equal(importPresence(walking).percent, null);
  // Then the headers: the total is known, the percentage comes, the walk no
  // longer moves the count.
  const reading = importReducer(walking, {
    type: "scanProgress",
    read: 482,
    total: 1423,
  });
  assert.equal(importPresence(reading).percent, 34);
  assert.equal(
    importReducer(reading, { type: "scanWalk", found: 9999 }).scan?.walked,
    412,
  );
  // A walk event outside a scan is ignored.
  assert.equal(
    importReducer(NO_IMPORT, { type: "scanWalk", found: 3 }).scan,
    null,
  );
});

/* ----------------- SY-08 / SY-11: the offer, announced in advance -------- */

const STATE = (progression: boolean | undefined, paired = true) =>
  ({
    paired,
    capabilities:
      progression === undefined
        ? null
        : ({ "analysis.progression": progression } as never),
  }) as CapabilitiesState;

test("an offer that changed is found by comparing two known readings; the first reading is never a change", () => {
  assert.equal(offerChanged(STATE(false), STATE(true)), true);
  assert.equal(offerChanged(STATE(true), STATE(true)), false);
  assert.equal(offerChanged(null, STATE(true)), false);
  assert.equal(offerChanged(STATE(undefined), STATE(true)), false);
  assert.equal(offerChanged(STATE(true), STATE(undefined)), false);
  assert.ok(CAPABILITIES_REFRESH_GAP_MS >= 1000);
});

test("the stacking is local and never gated: step 3 always offers to launch it, whatever the offer", () => {
  const onStep3 = play([{ type: "open" }, { type: "scanStart" }]);
  const step3 = { ...onStep3, step: 3 as const, scan: null };
  assert.equal(footRule(step3).primary?.kind, "treat");
  assert.equal(footRule(step3).secondary, "skipTreatments");
  assert.equal(footRule({ ...step3, treated: true }).primary?.kind, "treat");
});

/* ------------------------- SY-09: the reason of « aucune image » --------- */

test("« Aucune image envoyée » gives its reason: the offer, nothing ticked, the treatments passed, or failed -- else nothing", () => {
  const base = { ...NO_IMPORT, treated: true };
  assert.equal(noneSentReason({ ...base, treatRefused: 4 }), "offer");
  assert.equal(noneSentReason({ ...NO_IMPORT, treated: false }), "skipped");
  assert.equal(
    noneSentReason({
      ...base,
      made: [{ targetId: "m31" } as never],
      ticked: [],
    }),
    "unticked",
  );
  assert.equal(noneSentReason({ ...base, treatErrors: ["boom"] }), "failed");
  assert.equal(noneSentReason(base), null);
  // The offer comes first.
  assert.equal(
    noneSentReason({ ...base, treatRefused: 1, treatErrors: ["boom"] }),
    "offer",
  );
  const result = importReducer(importReducer(NO_IMPORT, { type: "open" }), {
    type: "treatDone",
    made: [],
    errors: [],
    refused: 4,
  });
  assert.equal(result.treatRefused, 4);
  assert.deepEqual(result.treatErrors, []);
  const sentence = (count: number) =>
    fr(`errors.stacking.not_in_subscription.${count === 1 ? "one" : "other"}`, {
      count,
    });
  assert.equal(
    sentence(4),
    "L’empilement n’est pas compris dans votre offre : aucune des 4 cibles n’a été empilée, rien n’a été téléchargé.",
  );
  assert.equal(
    fr("import.done.noneSentWhy", { reason: fr("import.done.reason.offer") }),
    "Aucune image envoyée : l’empilement n’est pas compris dans votre offre.",
  );
});

/* ---------------------- planche 09: the preview of a pipeline ------------ */

test("a pipeline with a preview shows it under its description; one without shows nothing; « Sans traitement » always has its own", () => {
  const withPreview: Pipeline[] = PIPELINES.map((pipeline, index) =>
    index === 0
      ? {
          ...pipeline,
          preview: "preview.jpg",
          previewData: "data:image/jpeg;base64,/9j/AAAA",
        }
      : pipeline,
  );
  const view = pipelineDialogView(fr, "fr", {
    target: {
      targetId: "ngc7000",
      designation: "NGC 7000",
      frames: 40,
      duration: "2 h 00",
      filter: "L-eXtreme",
      band: "dualband",
    },
    pipelines: withPreview,
    tools: tools({}),
    selection: ["raw"],
    several: true,
    engine: "graxpert-ai",
    progress: {},
  });
  const [first, second] = view.cards;
  assert.deepEqual(first.preview, {
    kind: "image",
    src: "data:image/jpeg;base64,/9j/AAAA",
    alt: "Aperçu du rendu « Rendu naturel »",
  });
  assert.equal(second.preview, null);
  const last = view.cards[view.cards.length - 1];
  assert.equal(last.id, "linear");
  assert.equal(last.preview?.kind, "linear");
  assert.equal(last.preview?.alt, "Aperçu du rendu « Sans traitement »");
});
