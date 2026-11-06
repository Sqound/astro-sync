// The sky annotations that follow each upload: what the « Envois » page says
// under a file sent, what the banner of the result adds, and how the events
// and the queue's database feed the state. Pure TypeScript, under
// `node --test`.

import assert from "node:assert/strict";
import test from "node:test";

import { createTranslator, SUPPORTED_LOCALES } from "../i18n/locale.ts";
import type { QueueDriveEvent, SkyAnnotation } from "./bridge";
import {
  NO_UPLOADS,
  afterColon,
  bannerLines,
  latestBatch,
  skyLine,
  uploadsReducer,
  type UploadsAction,
  type UploadsState,
} from "./uploads.ts";

const fr = createTranslator("fr");
const plain = (text: string) => text.replaceAll("\u00a0", " ");
const SHA = "b".repeat(64);

const annotation = (extra: Partial<SkyAnnotation> = {}): SkyAnnotation => ({
  sha256: SHA,
  captureId: "cap-plan",
  state: "sent",
  updatedAt: "2026-10-23T17:10:00Z",
  ...extra,
});

function play(actions: UploadsAction[], from: UploadsState = NO_UPLOADS) {
  return actions.reduce(uploadsReducer, from);
}
const completed: QueueDriveEvent = {
  kind: "Completed",
  sha256: SHA,
  captureId: "cap-plan",
  deduplicated: false,
};
const sentState = () =>
  play([
    {
      type: "track",
      origin: "detect",
      files: [
        {
          sha256: SHA,
          path: "/work/IC1805/stacked/IC_1805_plan_raw.tif",
          size: 46_500_000,
          target: "IC 1805",
          rendering: "Naturel",
        },
      ],
    },
    { type: "event", event: completed, now: 1 },
  ]);

test("the Envois row says what became of the sky annotations: sent and how long, already there, no solver, not solved", () => {
  const line = (extra: Partial<SkyAnnotation>) =>
    skyLine(fr, "fr", annotation(extra));

  const running = line({ state: "running" });
  assert.equal(plain(running.text), plain("Annotations du ciel : en cours"));
  assert.deepEqual([running.tone, running.retry], ["neutral", false]);

  const sent = line({ elapsedMs: 4_200, source: "astap" });
  assert.equal(
    plain(sent.text),
    plain("Annotations du ciel : envoyées · résolues en 4 s"),
  );
  assert.deepEqual([sent.tone, sent.retry], ["success", false]);
  // A solution so quick it is under a second still says one second.
  assert.match(line({ elapsedMs: 120 }).text, /résolues en 1 s$/);
  // The header of the image gave it: nothing was computed.
  assert.match(
    line({ source: "header", elapsedMs: 3 }).text,
    /lue dans l’image/,
  );
  assert.equal(
    plain(line({ state: "already" }).text),
    plain("Annotations du ciel : déjà présentes"),
  );

  const missing = line({
    state: "failed",
    failure: { code: "solver_missing" },
  });
  assert.equal(
    plain(missing.text),
    plain("Annotations du ciel : solveur absent"),
  );
  assert.deepEqual([missing.tone, missing.retry], ["warning", true]);

  const failed = line({ state: "failed", failure: { code: "too_few_stars" } });
  assert.equal(
    plain(failed.text),
    plain(
      "Annotations du ciel non résolues : trop peu d’étoiles reconnues dans l’image.",
    ),
  );
  assert.equal(failed.retry, true);
  const timeout = line({
    state: "failed",
    failure: { code: "timeout", seconds: 180 },
  });
  assert.match(plain(timeout.text), /après 180 s/);

  // The service could not be asked: its code is said in words.
  const network = line({ state: "failed", errorCode: "network_error" });
  assert.match(plain(network.text), /^Annotations du ciel non résolues\s: /);
  assert.doesNotMatch(network.text, /network_error/);
  const interrupted = line({ state: "failed", errorCode: "sky_interrupted" });
  assert.match(plain(interrupted.text), /interrompues/);
});

test("the events and the queue's database feed the state; a newer event is not overwritten by an older read", () => {
  const running = annotation({
    state: "running",
    updatedAt: "2026-10-23T17:10:00Z",
  });
  const sent = annotation({
    state: "sent",
    updatedAt: "2026-10-23T17:10:05Z",
    elapsedMs: 4000,
  });
  let state = play([{ type: "sky", annotation: running }]);
  assert.equal(state.sky[SHA].state, "running");
  state = play([{ type: "sky", annotation: sent }], state);
  assert.equal(state.sky[SHA].state, "sent");
  // The database was read before the end: it still says « running ».
  state = play([{ type: "skySeed", annotations: [running] }], state);
  assert.equal(state.sky[SHA].state, "sent");
  // A file the window never saw is learned from the database.
  const other = annotation({ sha256: "c".repeat(64), state: "already" });
  state = play([{ type: "skySeed", annotations: [other] }], state);
  assert.equal(state.sky["c".repeat(64)].state, "already");
});

test("« Réessayer » shows the annotations running again at once, without the old reason", () => {
  const failed = annotation({
    state: "failed",
    failure: { code: "not_solved" },
    message:
      "Aucune correspondance entre les étoiles de l’image et le catalogue.",
  });
  let state = play([{ type: "sky", annotation: failed }]);
  state = play([{ type: "skyRetry", sha256: SHA }], state);
  const row = state.sky[SHA];
  assert.equal(row.state, "running");
  assert.equal(row.failure, undefined);
  assert.equal(row.message, undefined);
  // A file with no annotations known has nothing to show running.
  assert.equal(
    play([{ type: "skyRetry", sha256: "d".repeat(64) }]).sky["d".repeat(64)],
    undefined,
  );
});

test("the banner of the result adds the annotations to the file sent", () => {
  const sent = sentState();
  const lines = (sky: Record<string, SkyAnnotation>, reasons = true) =>
    bannerLines(fr, "fr", latestBatch(sent, "detect"), reasons, sky).map(
      (line) => plain(line.text),
    );
  // Nothing known yet: the plain sentence.
  assert.deepEqual(lines({}), ["1 image envoyée"]);
  assert.deepEqual(lines({ [SHA]: annotation({ state: "running" }) }), [
    "1 image envoyée · annotations du ciel en cours",
  ]);
  assert.deepEqual(lines({ [SHA]: annotation() }), [
    "1 image envoyée · annotations du ciel envoyées",
  ]);
  assert.deepEqual(lines({ [SHA]: annotation({ state: "already" }) }), [
    "1 image envoyée · annotations du ciel envoyées",
  ]);
  const failed = annotation({
    state: "failed",
    failure: { code: "solver_missing" },
  });
  const [done, warning] = bannerLines(
    fr,
    "fr",
    latestBatch(sent, "detect"),
    true,
    { [SHA]: failed },
  );
  assert.equal(plain(done.text), "1 image envoyée");
  assert.equal(warning.tone, "warning");
  assert.match(
    plain(warning.text),
    /^1 image sans annotations du ciel\s: aucun solveur n’est installé/,
  );
});

test("the words exist in the ten languages, and the banner reads in English too", () => {
  const keys = [
    "uploads.sky.running",
    "uploads.sky.sent",
    "uploads.sky.sentIn",
    "uploads.sky.sentHeader",
    "uploads.sky.already",
    "uploads.sky.solverMissing",
    "uploads.sky.failed",
    "detect.result.sky.sent",
    "detect.result.sky.running",
    "detect.result.sky.failed.one",
    "detect.result.sky.failed.other",
    "errors.sky_interrupted",
  ];
  assert.equal(SUPPORTED_LOCALES.length, 10);
  for (const locale of SUPPORTED_LOCALES) {
    const t = createTranslator(locale);
    for (const key of keys) {
      const said = t(key, { seconds: 4, reason: "x", count: 2 });
      assert.notEqual(said, key, `${locale}: ${key}`);
    }
  }
  const en = createTranslator("en");
  assert.equal(
    skyLine(en, "en", annotation({ elapsedMs: 4000 })).text,
    "Sky annotations: sent · solved in 4 s",
  );
  const sent = sentState();
  assert.deepEqual(
    bannerLines(en, "en", latestBatch(sent, "detect"), true, {
      [SHA]: annotation(),
    }).map((line) => line.text),
    ["1 image sent · sky annotations sent"],
  );
});

test("the reason after the colon starts in lowercase where the language does, names and German nouns excepted", () => {
  const reason = (locale: string, extra: Partial<SkyAnnotation>) => {
    const t = createTranslator(locale);
    return plain(
      skyLine(t, locale, annotation({ state: "failed", ...extra })).text,
    );
  };
  const colon = /[:：]\s*(.)/u;
  const initial = (locale: string, extra: Partial<SkyAnnotation>) =>
    colon.exec(reason(locale, extra))?.[1] ?? "";
  const unreadable = { failure: { code: "unreadable_image" as const } };
  const outside = { failure: { code: "outside_database" as const } };
  // The six languages that lowercase: every solver failure starts in lowercase.
  for (const locale of ["fr", "en", "es", "it", "pt-BR", "ru"]) {
    for (const code of [
      "unreadable_image",
      "too_few_stars",
      "not_solved",
      "outside_database",
      "cancelled",
      "failed",
    ] as const) {
      const letter = initial(locale, { failure: { code } });
      assert.equal(
        letter,
        letter.toLowerCase(),
        `${locale}/${code}: « ${reason(locale, { failure: { code } })} »`,
      );
    }
  }
  assert.match(
    reason("fr", unreadable),
    /non résolues\s: le solveur ne peut pas lire/,
  );
  assert.match(
    reason("en", outside),
    /not solved: field outside the installed/,
  );
  // German nouns keep their capital; Japanese and Chinese have no case.
  assert.match(reason("de", outside), /gelöst: [A-ZÄÖÜ]/);
  for (const locale of ["de", "ja", "zh-Hans", "zh-Hant"]) {
    const t = createTranslator(locale);
    assert.equal(
      afterColon(t("annotations.failure.outside_database"), locale),
      t("annotations.failure.outside_database"),
    );
  }
  // A name or an acronym keeps its capitals; a reason already in lowercase is
  // left alone; the service's own message is lowered like the others.
  assert.equal(
    afterColon("ASTAP est introuvable", "fr"),
    "ASTAP est introuvable",
  );
  assert.equal(afterColon("Siril a échoué", "fr"), "Siril a échoué");
  assert.equal(afterColon("StarNet2 failed", "en"), "StarNet2 failed");
  assert.equal(
    afterColon("Astro Portal is down", "en"),
    "Astro Portal is down",
  );
  assert.equal(
    afterColon("astrometry.net n’a aucun index", "fr"),
    "astrometry.net n’a aucun index",
  );
  assert.equal(afterColon("È introvabile", "it"), "è introvabile");
  assert.equal(afterColon("Что-то пошло не так", "ru"), "что-то пошло не так");
  assert.equal(afterColon("", "fr"), "");
  // The banner of the result reads the same way.
  const sent = sentState();
  const [, warning] = bannerLines(
    createTranslator("en"),
    "en",
    latestBatch(sent, "detect"),
    true,
    { [SHA]: annotation({ state: "failed", ...unreadable }) },
  );
  assert.match(warning.text, /: the solver cannot read this image/);
});
