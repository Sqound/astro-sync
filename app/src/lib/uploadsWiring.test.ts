import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

import { createTranslator, SUPPORTED_LOCALES } from "../i18n/locale.ts";
import { MENU } from "./menu.ts";

// What the pure tests cannot see: that the windows are wired to them. The
// captures show the same in a real window.

const CLEARANCE_NAME = "--as-snackbar-clearance";

const source = (relative: string) =>
  readFileSync(new URL(`../${relative}`, import.meta.url), "utf8");

test("the detection flow drives the upload queue right after it queues the images; the night wizard and the progression still do their own run", () => {
  const flow = source("components/ImportProvider.tsx");
  const queued = flow.indexOf("await detectImagesSend(");
  const started = flow.indexOf("submit(", queued);
  assert.ok(queued > 0, "the images are queued");
  assert.ok(started > queued, "...and the run starts after them, not before");
  assert.match(flow.slice(started, started + 200), /"detect"/);
  // The provider is what runs the queue: one run, whoever asked.
  const provider = source("components/UploadsProvider.tsx");
  assert.match(provider, /await uploadAll\(targetRef\.current\)/);
  assert.match(provider, /dispatch\(\{ type: "track", origin, files \}\)/);
  assert.match(provider, /void drain\(\);/);
  // The night wizard's path is the one it always had.
  const wizard = source("screens/night/SummaryStep.tsx");
  assert.equal((wizard.match(/uploadAll\(\{/g) ?? []).length, 2);
  assert.match(
    wizard,
    /upload: \(\) =>\s+uploadAll\(\{\s+sessionId: context\.sessionId/,
  );
  assert.match(source("screens/ProgressionScreen.tsx"), /await uploadAll\(\{/);
  // Nothing in the flow calls the queue's run on its own: one owner.
  assert.doesNotMatch(flow, /uploadAll\(/);
});

test("files left in the queue by an earlier run are resumed at the start and when the page opens, unless the person paused", () => {
  const provider = source("components/UploadsProvider.tsx");
  assert.match(provider, /resumePending\(status\.entries\)/);
  assert.match(
    provider,
    /if \(pausedRef\.current \|\| runningRef\.current\) return;/,
  );
  assert.match(provider, /\(await capabilities\(\)\)\.paired/);
  assert.match(provider, /"uploads\.resumed"/);
  assert.match(source("screens/QueueScreen.tsx"), /resumePending\(\);/);
});

test("« Envois » is a menu entry with the count of the uploads on its way as a badge, not a separate icon", () => {
  assert.ok(MENU.some((entry) => entry.id === "queue"));
  const app = source("App.tsx");
  assert.match(app, /tab\.id === "queue" && pending > 0/);
  assert.match(app, /className="as-tab__badge"/);
  assert.match(app, /pendingCount\(uploads\.state\)/);
  assert.doesNotMatch(app, /UploadToasts/);
  assert.match(app, /registerOpener\(\(\) => setScreen\("queue"\)\)/);
  assert.match(app, /<UploadsProvider>/);
});

test("the notifications of the uploads are the window's snackbar at the bottom: no card of their own, the provider emits to it", () => {
  assert.throws(() => source("components/UploadToasts.tsx"), /ENOENT/);
  const provider = source("components/UploadsProvider.tsx");
  assert.match(provider, /const \{ notify, dismissKey \} = useSnackbar\(\)/);
  assert.match(provider, /uploadNotices\(t, locale, state, \{/);
  assert.match(provider, /notify\(notice\)/);
  assert.match(provider, /dismissKey\(key\)/);
  // The cross closes the notification, never the upload.
  assert.match(provider, /send\(\{ type: "dismiss", sha256 \}\)/);
  const emitting = provider.slice(
    provider.indexOf("uploadNotices(t, locale"),
    provider.indexOf("const keys = new Set"),
  );
  assert.doesNotMatch(emitting, /uploadPause|uploadAll|cancel/i);
  const css = source("theme/uploads.css");
  assert.doesNotMatch(css, /as-upload-toast/);
  assert.doesNotMatch(css, /position: fixed/);
  assert.ok(!/#[0-9a-fA-F]{3,8}\b/.test(css), "a hex colour in uploads.css");
  assert.ok(
    !/\b(?:rgb|rgba|hsl|hsla)\(/i.test(css),
    "a colour function in uploads.css",
  );
  for (const locale of SUPPORTED_LOCALES) {
    assert.equal(
      createTranslator(locale)("uploads.toasts"),
      "uploads.toasts",
      `${locale} still has the key of the old card`,
    );
  }
});

test("the snackbar shows one card at a time, with an optional bar, a muted line and a cross that tells its owner, and the content stops above it", () => {
  const snackbar = source("components/Snackbar.tsx");
  assert.match(snackbar, /visibleSnack\(state\.snacks\)/);
  assert.doesNotMatch(snackbar, /snacks\.map\(/, "no stack of cards");
  assert.match(snackbar, /role="progressbar"/);
  assert.match(snackbar, /snack\.progress !== undefined/);
  assert.match(snackbar, /aria-valuenow=\{snack\.progress\}/);
  assert.match(snackbar, /width: `\$\{snack\.progress\}%`/);
  assert.match(snackbar, /className="as-snackbar__detail"/);
  assert.match(snackbar, /gone\?\.onClose\?\.\(\)/);
  assert.match(snackbar, /setProperty\(CLEARANCE_VAR/);
  const css = source("theme/shell.css");
  assert.match(css, /margin-bottom: var\(--as-snackbar-clearance, 0\)/);
  assert.match(css, /\.as-snackbar__progress/);
  assert.equal(CLEARANCE_NAME, "--as-snackbar-clearance");
});

test("an avis has no primary: the notifications, the banner and the page's live zone use none", () => {
  for (const file of [
    "components/Snackbar.tsx",
    "components/UploadsProvider.tsx",
    "screens/QueueScreen.tsx",
  ]) {
    assert.doesNotMatch(source(file), /buttonClassName\("primary"\)/, file);
  }
  // « Terminé » of the import: what each image is doing has no primary;
  // the step's one primary is « Retour aux sessions », in the foot.
  const done = source("screens/detect/ImportDone.tsx");
  const images = done.slice(
    done.indexOf("as-import-done__images"),
    done.indexOf("<WizardFoot"),
  );
  assert.doesNotMatch(images, /primary|secondary/);
  assert.match(images, /buttonClassName\("link"\)/);
  assert.match(images, /t\("uploads\.open"\)/);
});

test("« Terminé » reads the live state of each image sent and its sky annotations, not a count of what was queued", () => {
  const done = source("screens/detect/ImportDone.tsx");
  assert.match(done, /latestBatch\(uploads\.state, "detect"\)/);
  // ... with the sky annotations that follow each upload.
  assert.match(done, /stateLine\(t, locale, row\)/);
  assert.match(done, /uploads\.state\.sky/);
  assert.match(done, /view\.sky\[row\.sha256\]/);
  assert.match(done, /skyLine\(t, locale, sky\)/);
  assert.match(done, /onRetrySky=\{uploads\.retrySky\}/);
  // ... and the bar of the row of « Envois » while an image leaves.
  assert.match(done, /progressLine\(t, locale, row\)/);
  assert.match(done, /role="progressbar"/);
});

test("the page « Envois » has the storage card, the live section and the history, and says when the service is unreachable", () => {
  const page = source("screens/QueueScreen.tsx");
  assert.match(page, /<StorageCard view=\{storageView\(storage\)\}/);
  assert.match(page, /t\("uploads\.live\.title"\)/);
  assert.match(page, /t\("uploads\.history\.title"\)/);
  assert.match(page, /t\("uploads\.history\.clear"\)/);
  assert.match(page, /view\.kind === "unreachable"/);
  assert.match(page, /t\("uploads\.storage\.unreachable"\)/);
  assert.match(page, /t\("uploads\.row\.retry"\)/);
  assert.match(page, /t\("uploads\.row\.capture"\)/);
});

test("« Envois » and its words exist in the ten languages; the old name is gone", () => {
  const names = SUPPORTED_LOCALES.map((locale) =>
    createTranslator(locale)("nav.queue"),
  );
  assert.equal(new Set(names).size >= 8, true, names.join(" | "));
  assert.equal(createTranslator("fr")("nav.queue"), "Envois");
  assert.equal(createTranslator("en")("nav.queue"), "Uploads");
  for (const locale of SUPPORTED_LOCALES) {
    const t = createTranslator(locale);
    for (const key of [
      "uploads.storage.unreachable",
      "uploads.row.retry",
      "uploads.history.clear",
      "uploads.open",
      "detect.result.active.other",
      "import.done.sent.other",
    ]) {
      assert.notEqual(t(key), key, `${locale} lacks ${key}`);
    }
  }
});

test("the sky annotations are wired end to end: the event, the retry, the page and the stylesheet", () => {
  const provider = source("components/UploadsProvider.tsx");
  assert.match(
    provider,
    /onSkyEvent\(\(annotation\) => dispatch\(\{ type: "sky", annotation \}\)\)/,
  );
  assert.match(provider, /skyRetry\(sha256\)/);
  assert.match(provider, /type: "skySeed", annotations: status\.sky/);
  const page = source("screens/QueueScreen.tsx");
  assert.match(page, /skyLine\(t, locale, sky\)/);
  assert.match(page, /onClick=\{\(\) => onRetrySky\(row\.sha256\)\}/);
  assert.match(page, /buttonClassName\("link"\)/);
  assert.match(source("theme/uploads.css"), /\.as-uploads__sky/);
});
