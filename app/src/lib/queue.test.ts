import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

import {
  SUPPORTED_LOCALES,
  createTranslator,
  type Locale,
} from "../i18n/locale.ts";
import type { QueueDriveEvent, QueueEntry } from "./bridge.ts";
import { noticeOfSetAside, reasonText, rowReason } from "./queue.ts";
import { PLAIN_FAILURE_CODES, REFUSAL_CODES } from "./serviceWait.ts";

function entry(overrides: Partial<QueueEntry>): QueueEntry {
  return {
    id: 1,
    sourcePath: "/night/M31/final/m31.tif",
    sha256: "a".repeat(64),
    size: 1000,
    uploadId: null,
    offset: 0,
    state: "queued",
    captureId: null,
    lastError: null,
    lastErrorCode: null,
    createdAt: "2026-09-29T21:00:00Z",
    updatedAt: "2026-09-29T21:00:00Z",
    kind: "final",
    target: { sessionId: null, targetIds: [], variantOf: null, source: null },
    ...overrides,
  };
}

async function dictionary(locale: Locale): Promise<Record<string, string>> {
  return JSON.parse(
    await readFile(new URL(`../i18n/${locale}.json`, import.meta.url), "utf8"),
  ) as Record<string, string>;
}

/** The width of a sentence on screen: a Chinese, Japanese or Korean character
 * is two columns wide, a letter of any other script one. The floor below asks
 * for a real sentence (not a bare code or a stub) in every language, and a
 * count of characters would ask the CJK languages for twice the sentence the
 * others write. */
function displayWidth(sentence: string): number {
  const wide =
    /[\u1100-\u115f\u2e80-\ua4cf\uac00-\ud7a3\uf900-\ufaff\ufe30-\ufe6f\uff00-\uff60\uffe0-\uffe6]/u;
  return [...sentence].reduce(
    (width, character) => width + (wide.test(character) ? 2 : 1),
    0,
  );
}

/** The codes of the core's catalogue, read from its source
 * (`astro_sync_core::refusal`). */
async function catalogueOfTheCore(): Promise<string[]> {
  const source = await readFile(
    new URL("../../../crates/astro-sync-core/src/refusal.rs", import.meta.url),
    "utf8",
  );
  const body = source.slice(
    source.indexOf("const KNOWN"),
    source.indexOf("/// Every code of the catalogue"),
  );
  return [...body.matchAll(/\(\s*"([a-z_]+)",\s*"/g)].map((match) => match[1]);
}

test("the refusals of the core each have a sentence in every language, and the window knows them all", async () => {
  const codes = await catalogueOfTheCore();
  assert.equal(codes.length, 20, "the catalogue of the core");
  assert.deepEqual([...REFUSAL_CODES].sort(), [...codes].sort());
  for (const locale of SUPPORTED_LOCALES) {
    const words = await dictionary(locale);
    for (const code of codes) {
      const sentence = words[`errors.${code}`];
      assert.ok(
        sentence && displayWidth(sentence) > 20,
        `${locale}: errors.${code}`,
      );
      assert.ok(
        !sentence.includes(code) && !sentence.includes("{"),
        `${locale}: ${code} says its own code or leaves a placeholder`,
      );
      assert.ok(PLAIN_FAILURE_CODES.has(code), code);
    }
  }
});

test("a refusal is said in the person's language, never with its raw code or the service's own words", () => {
  for (const locale of SUPPORTED_LOCALES) {
    const t = createTranslator(locale);
    for (const code of REFUSAL_CODES) {
      const text = reasonText(t, code, "The service's own English text.");
      assert.ok(!text.includes("English"), `${locale}: ${code}: ${text}`);
      assert.notEqual(text, `errors.${code}`);
    }
    // A code this window has no sentence for falls on the core's own text,
    // which is French and plain, never on `errors.<code>`.
    assert.equal(
      reasonText(t, "a_code_from_a_newer_service", "Texte du noyau."),
      "Texte du noyau.",
    );
  }
});

test("a file the service refused, or that waits for the person, says why under its state", () => {
  for (const locale of SUPPORTED_LOCALES) {
    const t = createTranslator(locale);
    const refused = rowReason(
      t,
      entry({
        state: "refused",
        lastErrorCode: "capture_content_refused",
        lastError: "refused by the service (415 capture_content_refused): ...",
      }),
    );
    assert.equal(refused, t("errors.capture_content_refused"), locale);
    const held = rowReason(
      t,
      entry({
        state: "held",
        lastErrorCode: "storage_quota_reached",
        lastError: "refused by the service (409 storage_quota_reached): ...",
      }),
    );
    assert.equal(held, t("errors.storage_quota_reached"), locale);
    assert.ok(!(refused ?? "").includes("415"), `${locale}: ${refused}`);
    // Nothing under a file that is on its way, done or waiting.
    for (const state of ["queued", "uploading", "completed"] as const) {
      assert.equal(rowReason(t, entry({ state })), null, `${locale}: ${state}`);
    }
    // A done file whose image replaced the phone's photo says so.
    const replaced = rowReason(
      t,
      entry({ state: "completed", lastErrorCode: "replaced" }),
    );
    assert.equal(replaced, t("queue.replacedPhoto"), locale);
    assert.ok(!(replaced ?? "").startsWith("queue."), locale);
  }
});

test("a run that waits, or stops for the person, or sets a file aside, says it in plain words with the wait", () => {
  const sha256 = "b".repeat(64);
  const events: QueueDriveEvent[] = [
    {
      kind: "Refused",
      sha256,
      code: "capture_content_refused",
      message: "Texte du noyau.",
    },
    {
      kind: "Held",
      sha256,
      code: "storage_quota_reached",
      message: "Texte du noyau.",
    },
    {
      kind: "Waiting",
      sha256,
      code: "upload_sessions_limit_reached",
      message: "Texte du noyau.",
      retryInSeconds: 30,
      attempt: 2,
    },
  ];
  for (const locale of SUPPORTED_LOCALES) {
    const t = createTranslator(locale);
    const notices = events.map((event) => noticeOfSetAside(t, event));
    assert.deepEqual(
      notices.map((notice) => notice?.tone),
      ["error", "warning", "warning"],
      locale,
    );
    for (const notice of notices) {
      assert.ok(notice, locale);
      assert.ok(
        !notice.text.includes("{") &&
          !notice.text.includes("errors.") &&
          !notice.text.includes("Texte du noyau"),
        `${locale}: ${notice.text}`,
      );
    }
    assert.ok(notices[0]!.text.includes(sha256.slice(0, 12)), locale);
    assert.ok(
      notices[2]!.text.includes("30"),
      `${locale}: ${notices[2]!.text}`,
    );
  }
  const fr = createTranslator("fr");
  assert.equal(
    noticeOfSetAside(fr, events[2]!)?.text,
    "Trop d’envois sont en cours dans votre espace. Ce fichier n’est pas en échec : il reprendra tout seul dès qu’un envoi se termine. Astro Sync réessaie tout seul dans 30 s. Vous pouvez mettre l’envoi en pause.",
  );
  const held = noticeOfSetAside(fr, events[1]!)?.text ?? "";
  assert.ok(held.startsWith("L’envoi est arrêté."), held);
  assert.ok(held.includes("Vos fichiers restent dans la file"), held);
  // The other events are not this module's business.
  assert.equal(
    noticeOfSetAside(fr, { kind: "Progress", sha256, sent: 1, total: 2 }),
    null,
  );
});

test("a file that is gone says which file and which folder, in every language", () => {
  for (const locale of SUPPORTED_LOCALES) {
    const t = createTranslator(locale);
    const sentence = rowReason(
      t,
      entry({
        state: "failed",
        sourcePath:
          "/data/AstroSync/Sh2-103 · local-fea0a6/progression/Sh2-103_plan-2_night-01_120-frames.jpg",
        lastErrorCode: "source_file_missing",
        lastError: "could not read the source file ...",
      }),
    );
    assert.ok(sentence, locale);
    assert.ok(
      sentence.includes("Sh2-103_plan-2_night-01_120-frames.jpg"),
      `${locale}: the file`,
    );
    assert.ok(
      sentence.includes("/data/AstroSync/Sh2-103 · local-fea0a6/progression"),
      `${locale}: the folder`,
    );
    assert.ok(!sentence.includes("{"), `${locale}: ${sentence}`);
    assert.ok(!sentence.startsWith("errors."), locale);
    assert.ok(!sentence.includes("could not read"), locale);
  }
  // The French sentence, as asked.
  assert.equal(
    rowReason(
      createTranslator("fr"),
      entry({
        state: "failed",
        sourcePath: "/w/Cible/progression/a.jpg",
        lastErrorCode: "source_file_missing",
      }),
    ),
    "Fichier introuvable\u00a0: a.jpg, il était dans /w/Cible/progression.",
  );
  // A failure without a code keeps the sentence of every other failure.
  const t = createTranslator("fr");
  assert.equal(
    rowReason(t, entry({ state: "failed", lastError: "boom" })),
    t("errors.upload_error", { message: "boom" }),
  );
});
