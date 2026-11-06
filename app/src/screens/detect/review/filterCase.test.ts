import assert from "node:assert/strict";
import { mkdir, writeFile } from "node:fs/promises";
import test, { before } from "node:test";
import { fileURLToPath } from "node:url";

import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { rolldown } from "rolldown";

import { createTranslator, type Locale } from "../../../i18n/locale.ts";
import type { DetectedSession, Proposal } from "../../../lib/bridge.ts";
import {
  chooseFilter,
  initialState,
  nightStatus,
  tickedIds,
  type ReviewState,
} from "../../../lib/detectReview.ts";

// « Filtre de cette nuit ? » with the band read from the names: the guess
// is chosen from the start and says where it comes from; the limit to two
// bands is said under the question; without a guess the question is
// today's. The card is bundled on the fly (the test runner reads no JSX)
// and drawn to a string.

type Harness = typeof import("./filterCaseHarness.tsx");
let harness: Harness;

before(async () => {
  const stubCss = {
    name: "stub-css",
    load(id: string) {
      return id.endsWith(".css")
        ? { code: "export default '';", moduleType: "js" as const }
        : null;
    },
  };
  const bundle = await rolldown({
    input: fileURLToPath(new URL("./filterCaseHarness.tsx", import.meta.url)),
    platform: "node",
    plugins: [stubCss],
    external: ["react", "react/jsx-runtime", "react-dom"],
    logLevel: "silent",
  });
  const { output } = await bundle.generate({ format: "esm" });
  const folder = new URL(
    "../../../../node_modules/.cache/astro-sync-tests/",
    import.meta.url,
  );
  await mkdir(folder, { recursive: true });
  const file = new URL("filter-case.mjs", folder);
  await writeFile(file, output[0].code);
  harness = await import(file.href);
});

function night(partial: Partial<DetectedSession>): DetectedSession {
  return {
    id: "night-1",
    eveningDate: "2026-08-09",
    writtenEveningDate: "2026-08-09",
    place: "place-1",
    equipment: [],
    start: "2026-08-09T21:00:00Z",
    end: "2026-08-09T23:00:00Z",
    targets: [],
    shiftHours: 0,
    calibration: [],
    logs: [],
    existing: { kind: "create" },
    filterAsk: true,
    ...partial,
  } as DetectedSession;
}

function proposalOf(session: DetectedSession): Proposal {
  return {
    sessions: [session],
    lines: [],
    places: [],
    equipment: [],
    multiNight: [],
  } as unknown as Proposal;
}

function drawn(
  locale: Locale,
  session: DetectedSession,
  state: ReviewState,
): string {
  return renderToStaticMarkup(
    createElement(
      harness.I18nProvider,
      { locale, onLocaleChange: () => {} },
      createElement(harness.FilterCase, {
        session,
        state,
        change: () => {},
      }),
    ),
  );
}

const dual = night({
  filterGuess: {
    band: "dualband",
    origin: "folder",
    source: "180s_dualband",
  },
});

test("a guessed band is chosen from the start and the night does not wait", () => {
  const proposal = proposalOf(dual);
  const state = initialState(proposal);
  assert.equal(state.filters[dual.id], "dualband");
  assert.equal(nightStatus(proposal, state, dual).waitsFilter, false);
  // Changing it is one click, and un-choosing asks again.
  const changed = chooseFilter(state, dual.id, "broadband");
  assert.equal(changed.filters[dual.id], "broadband");
  assert.equal(
    nightStatus(proposal, chooseFilter(state, dual.id, null), dual).waitsFilter,
    true,
  );
  assert.ok(Array.isArray(tickedIds(proposal, state)));
});

test("no guess: the question is today's, nothing is chosen", () => {
  const asked = night({});
  const proposal = proposalOf(asked);
  const state = initialState(proposal);
  assert.equal(state.filters[asked.id], undefined);
  assert.equal(nightStatus(proposal, state, asked).waitsFilter, true);
  // A guess on a night that asks nothing is not used.
  const declared = night({ ...dual, filterAsk: false });
  assert.equal(
    initialState(proposalOf(declared)).filters[declared.id],
    undefined,
  );
});

test("the card shows the chosen band, where it was read, and the two-band limit", () => {
  const fr = createTranslator("fr");
  const state = initialState(proposalOf(dual));
  const html = drawn("fr", dual, state);
  assert.ok(
    html.includes("Déduit du nom « 180s_dualband » — changez si c’est faux."),
  );
  assert.ok(!html.includes(fr("detect.night.filterBody")));
  assert.ok(html.includes(fr("detect.night.filterLimit")));
  assert.ok(
    html.includes("Astro Sync distingue seulement large bande et bibande"),
  );
  // Answered: a statement (« Filtre de cette nuit : bibande. »), no longer
  // the amber question.
  assert.ok(html.includes("Filtre de cette nuit : bibande."));
  assert.ok(html.includes("as-review__ask is-answered"));
  assert.ok(!html.includes("is-unanswered"));
  // The choice is a radio group: the chosen one is checked and carries a tick.
  assert.match(html, /role="radiogroup"/);
  assert.match(
    html,
    /aria-checked="true"[^>]*is-chosen[^>]*>.*?Bibande<\/button>/,
  );
  assert.match(html, /aria-checked="false"[^>]*>Large bande<\/button>/);
  // Changed by the person: the line of the guess is gone, the statement
  // says the answer is theirs.
  const changed = chooseFilter(state, dual.id, "broadband");
  const after = drawn("fr", dual, changed);
  assert.ok(!after.includes("Déduit du nom"));
  assert.ok(after.includes("Filtre de cette nuit : large bande."));
  assert.ok(after.includes(fr("detect.night.filterAnsweredBody")));
  assert.ok(after.includes(fr("detect.night.filterLimit")));
  // Never answered: the amber question, nothing checked.
  const asked = drawn("fr", dual, chooseFilter(state, dual.id, null));
  assert.ok(asked.includes(fr("detect.night.filterTitle")));
  assert.ok(asked.includes(fr("detect.night.filterBody")));
  assert.ok(asked.includes("is-unanswered"));
  assert.ok(!asked.includes('aria-checked="true"'));
});

test("a heading of a note names the note, and every language says it", () => {
  const note = night({
    filterGuess: {
      band: "broadband",
      origin: "note",
      source: "SH2-190 — RGB 90 s",
      noteFile: "SESSION.md",
    },
  });
  const state = initialState(proposalOf(note));
  assert.ok(
    drawn("en", note, state).includes(
      "Deduced from the heading of SESSION.md, “SH2-190 — RGB 90 s” — change it if that is wrong.",
    ),
  );
  for (const locale of [
    "en",
    "fr",
    "de",
    "es",
    "it",
    "ja",
    "pt-BR",
    "ru",
    "zh-Hans",
    "zh-Hant",
  ] as const) {
    const t = createTranslator(locale);
    const html = drawn(locale, dual, initialState(proposalOf(dual)));
    assert.ok(html.includes(t("detect.night.filterLimit")), locale);
    const line = t("detect.night.filterGuess", { source: "180s_dualband" });
    assert.ok(line.includes("180s_dualband"), locale);
    assert.ok(html.includes(line.replaceAll("&", "&amp;")), locale);
  }
});
