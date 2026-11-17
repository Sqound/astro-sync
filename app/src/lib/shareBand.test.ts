import assert from "node:assert/strict";
import { readFile, readdir } from "node:fs/promises";
import test from "node:test";

import { SUPPORTED_LOCALES, createTranslator } from "../i18n/locale.ts";
import {
  BAND_SHARE,
  bandHeight,
  bandInputFromRow,
  bandText,
  copyLayout,
  parseShareOptions,
  serialiseShareOptions,
  shareCloseKey,
  shareFileName,
  stripHeight,
  type BandInput,
} from "./shareBand.ts";

const fr = createTranslator("fr");
const NB = " ";

const PLEIADES: BandInput = {
  name: "Les Pléiades",
  designation: "M45",
  night: { count: 1, dates: ["2026-10-24"] },
  frames: 430,
  exposureSeconds: 30,
  totalSeconds: 12_900,
};

test("the band is 8 % of the width, never thinner than 40 px nor thicker than 240", () => {
  assert.equal(BAND_SHARE, 0.08);
  assert.equal(bandHeight(600), 48);
  assert.equal(bandHeight(1080), 86);
  assert.equal(bandHeight(2160), 173);
  assert.equal(bandHeight(4000), 240);
  assert.equal(bandHeight(200), 40);
});

test("the strip of the mark alone is thinner than the band", () => {
  for (const width of [600, 1080, 2160, 4000]) {
    assert.ok(stripHeight(width) < bandHeight(width), String(width));
  }
});

test("the full band says the target, the night, the frames and the time", () => {
  const text = bandText(fr, "fr", PLEIADES);
  assert.equal(text.top, "Les Pléiades");
  assert.equal(text.bottom, "M45 · Nuit du 24 octobre 2026");
  assert.equal(
    text.integration,
    `430${NB}×${NB}30${NB}s · 3${NB}h${NB}35${NB}min`,
  );
});

test("a missing name leaves the designation on the first line, nothing invented", () => {
  const text = bandText(fr, "fr", { ...PLEIADES, name: null });
  assert.equal(text.top, "M45");
  assert.equal(text.bottom, "Nuit du 24 octobre 2026");
});

test("a long exposure stays in seconds, a very long one goes to minutes", () => {
  const long = bandText(fr, "fr", {
    ...PLEIADES,
    frames: 40,
    exposureSeconds: 180,
    totalSeconds: 7_200,
  });
  assert.equal(
    long.integration,
    `40${NB}×${NB}180${NB}s · 2${NB}h${NB}00${NB}min`,
  );
  const half = bandText(fr, "fr", { ...PLEIADES, exposureSeconds: 0.5 });
  assert.ok(half.integration?.startsWith(`430${NB}×${NB}0,5${NB}s`));
  const huge = bandText(fr, "fr", { ...PLEIADES, exposureSeconds: 900 });
  assert.ok(huge.integration?.includes(`15${NB}min${NB}00${NB}s`));
});

test("missing frames leave the time alone; nothing known leaves the right side empty", () => {
  const timeOnly = bandText(fr, "fr", {
    ...PLEIADES,
    frames: null,
    exposureSeconds: null,
  });
  assert.equal(timeOnly.integration, `3${NB}h${NB}35${NB}min`);
  const none = bandText(fr, "fr", {
    ...PLEIADES,
    frames: null,
    exposureSeconds: null,
    totalSeconds: null,
  });
  assert.equal(none.integration, null);
  // Frames and exposure without a total: the product, exact.
  const product = bandText(fr, "fr", { ...PLEIADES, totalSeconds: null });
  assert.equal(
    product.integration,
    `430${NB}×${NB}30${NB}s · 3${NB}h${NB}35${NB}min`,
  );
});

test("an unknown night is left out, not replaced by a word", () => {
  const text = bandText(fr, "fr", { ...PLEIADES, night: null });
  assert.equal(text.bottom, "M45");
  const noName = bandText(fr, "fr", { ...PLEIADES, night: null, name: null });
  assert.equal(noName.bottom, null);
});

test("several nights say their count and the span, and the total time", () => {
  const text = bandText(fr, "fr", {
    ...PLEIADES,
    night: { count: 3, dates: ["2026-10-22", "2026-10-23", "2026-10-24"] },
    frames: null,
    exposureSeconds: null,
    totalSeconds: 41_400,
  });
  assert.match(text.bottom ?? "", /^M45 · 3\snuits, 22.24 octobre 2026$/);
  assert.equal(text.integration, `3${NB}nuits · 11${NB}h${NB}30${NB}min`);
  const noSpan = bandText(fr, "fr", {
    ...PLEIADES,
    night: { count: 3, dates: [] },
    frames: null,
    exposureSeconds: null,
    totalSeconds: 41_400,
  });
  assert.equal(noSpan.bottom, `M45 · 3${NB}nuits`);
});

test("the band reads in each of the ten languages, with no empty word", () => {
  for (const locale of SUPPORTED_LOCALES) {
    const t = createTranslator(locale);
    const one = bandText(t, locale, PLEIADES);
    assert.ok(
      one.bottom && one.bottom.includes("2026"),
      `${locale}: ${one.bottom}`,
    );
    assert.ok(one.integration && one.integration.includes("430"), locale);
    const many = bandText(t, locale, {
      ...PLEIADES,
      night: { count: 3, dates: ["2026-10-22", "2026-10-24"] },
    });
    assert.ok(
      many.bottom && many.bottom.includes("3"),
      `${locale}: ${many.bottom}`,
    );
    for (const value of [one.bottom, many.bottom, one.integration]) {
      assert.ok(!/\{|undefined|NaN/.test(value ?? ""), `${locale}: ${value}`);
    }
  }
});

test("the data of a row: the designation, the poses, the time; nothing of the place", () => {
  const input = bandInputFromRow(
    {
      designation: "M45",
      nights: 1,
      series: {
        filter: "IRCUT",
        totalSeconds: 12_900,
        exposureSeconds: 30,
        frames: 430,
      },
      // A row also carries the path of the file, which may name a place.
      path: "/data/place-name/M45/M45_stack.png",
    } as never,
    "Les Pléiades",
    ["2026-10-24"],
  );
  assert.deepEqual(input, PLEIADES);
  const everything = JSON.stringify(bandText(fr, "fr", input));
  assert.ok(!/place|data|IRCUT/i.test(everything), everything);
});

test("a night is told only when it is known: one night of a card with several is not guessed", () => {
  const row = { designation: "M45", nights: 1, series: undefined } as never;
  assert.deepEqual(bandInputFromRow(row, null, ["2026-10-24"]).night, {
    count: 1,
    dates: ["2026-10-24"],
  });
  assert.equal(
    bandInputFromRow(row, null, ["2026-10-23", "2026-10-24"]).night,
    null,
  );
  const stacked = { designation: "M45", nights: 2, series: undefined } as never;
  assert.deepEqual(
    bandInputFromRow(stacked, null, ["2026-10-23", "2026-10-24"]).night,
    { count: 2, dates: ["2026-10-23", "2026-10-24"] },
  );
  assert.deepEqual(bandInputFromRow(stacked, null, ["2026-10-24"]).night, {
    count: 2,
    dates: [],
  });
});

test("the copy is the image with the band beneath: the image keeps every pixel", () => {
  const text = bandText(fr, "fr", PLEIADES);
  const both = copyLayout(2160, 3991, text, { data: true, mark: true });
  assert.equal(both.kind, "band");
  assert.equal(both.imageHeight, 3991);
  assert.equal(both.height, 3991 + both.bandHeight);
  assert.equal(both.bandHeight, bandHeight(2160));
  assert.equal(both.width, 2160);
});

test("the mark alone is a strip under the image, never a corner on it", () => {
  const text = bandText(fr, "fr", PLEIADES);
  const markOnly = copyLayout(1080, 1620, text, { data: false, mark: true });
  assert.equal(markOnly.kind, "strip");
  assert.equal(markOnly.bandHeight, stripHeight(1080));
  assert.equal(markOnly.height, 1620 + stripHeight(1080));
  // Nothing known and the mark wanted: the mark alone too.
  const empty = bandText(fr, "fr", {
    ...PLEIADES,
    name: null,
    designation: "",
    night: null,
    frames: null,
    exposureSeconds: null,
    totalSeconds: null,
  });
  assert.equal(
    copyLayout(1080, 1620, empty, { data: true, mark: true }).kind,
    "strip",
  );
});

test("both switches off: the image alone, no band; data without the mark: the band", () => {
  const text = bandText(fr, "fr", PLEIADES);
  const none = copyLayout(1080, 1620, text, { data: false, mark: false });
  assert.deepEqual(
    [none.kind, none.bandHeight, none.height],
    ["none", 0, 1620],
  );
  assert.equal(
    copyLayout(1080, 1620, text, { data: true, mark: false }).kind,
    "band",
  );
});

test("the two switches start on, and a choice is remembered", () => {
  assert.deepEqual(parseShareOptions(null), { data: true, mark: true });
  assert.deepEqual(parseShareOptions("not json"), { data: true, mark: true });
  const kept = serialiseShareOptions({ data: true, mark: false });
  assert.deepEqual(parseShareOptions(kept), { data: true, mark: false });
  assert.deepEqual(parseShareOptions('{"data":false}'), {
    data: false,
    mark: true,
  });
});

test("the file proposed is named from the designation, safe on every system", () => {
  assert.equal(shareFileName("M45"), "M45-astro-sync.png");
  assert.equal(shareFileName("Sh2-190"), "Sh2-190-astro-sync.png");
  assert.equal(
    shareFileName("NGC 7000/North: America"),
    "NGC-7000-North-America-astro-sync.png",
  );
  assert.equal(shareFileName(""), "astro-sync.png");
});

test("the band never reaches the files that are sent or the originals", async () => {
  // Only the export sheet and the card button may use the band's code; the
  // sending, the adjustment and the uploads never import it.
  const allowed = new Set([
    "lib/bridge.ts",
    "lib/shareBand.ts",
    "lib/shareBandDraw.ts",
    "lib/shareBand.test.ts",
    "lib/shareBandDraw.test.ts",
    "lib/shareCopy.ts",
    "components/ShareSheet.tsx",
    "screens/detect/SummaryImages.tsx",
    "screens/detect/ImportSummary.tsx",
  ]);
  const root = new URL("../", import.meta.url);
  const found: string[] = [];
  const walk = async (dir: URL, prefix: string) => {
    for (const entry of await readdir(dir, { withFileTypes: true })) {
      if (entry.name === "i18n" || entry.name === "node_modules") continue;
      if (entry.isDirectory()) {
        await walk(new URL(`${entry.name}/`, dir), `${prefix}${entry.name}/`);
      } else if (/\.(ts|tsx)$/.test(entry.name)) {
        const source = await readFile(new URL(entry.name, dir), "utf8");
        if (/shareBand|shareCopy|share_copy/.test(source)) {
          found.push(`${prefix}${entry.name}`);
        }
      }
    }
  };
  await walk(root, "");
  for (const file of found)
    assert.ok(allowed.has(file), `${file} uses the band`);
  // The core's own writers of a render, of the sending, of the adjusted
  // image never call the share copy writer.
  const core = new URL("../../../crates/astro-sync-core/src/", import.meta.url);
  for (const name of [
    "image_adjust.rs",
    "render_files.rs",
    "package.rs",
    "uploader.rs",
    "upload_queue.rs",
  ]) {
    let source = "";
    try {
      source = await readFile(new URL(name, core), "utf8");
    } catch {
      continue;
    }
    assert.ok(!source.includes("share_copy"), `${name} calls the share copy`);
  }
});

test("once the copy is saved the second button says « Fermer », in the ten languages", () => {
  assert.equal(shareCloseKey(false), "share.cancel");
  assert.equal(shareCloseKey(true), "common.close");
  assert.equal(createTranslator("fr")(shareCloseKey(true)), "Fermer");
  assert.equal(createTranslator("fr")(shareCloseKey(false)), "Annuler");
  for (const locale of SUPPORTED_LOCALES) {
    const t = createTranslator(locale);
    assert.notEqual(t(shareCloseKey(true)), "common.close", locale);
    assert.notEqual(t(shareCloseKey(true)), t(shareCloseKey(false)), locale);
  }
});
