import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

import { NO_IMPORT } from "./importFlow.ts";
import { pngRowOf, summaryImages, tiffKeysOf } from "./detectTreatments.ts";
import type { TreatmentResult } from "./detectTreatments.ts";

// « Images à envoyer » is one card per image: the picture, the title, « À
// envoyer » with a box for the PNG and one for its TIFF (unticked unless
// chosen, a warning when ticked, never without its PNG), the editor's button,
// and the place of the file on one line.

const read = (path: string) =>
  readFileSync(new URL(path, import.meta.url), "utf8");
const screen = read("../screens/detect/SummaryImages.tsx");

function render(stem: string): TreatmentResult[] {
  const png = `/w/M 45/stacked/${stem}.png`;
  const base = {
    targetId: "m45",
    designation: "M 45",
    pipelineId: "raw",
    nights: 1,
  };
  return [
    { ...base, path: png, size: 13e6 },
    {
      ...base,
      path: `/w/M 45/stacked/${stem}.tif`,
      size: 47e6,
      fullResolution: true,
      twinOf: png,
    },
  ];
}

test("the TIFF of a render is the second box of its PNG's card, and the PNG is found from the TIFF", () => {
  const rows = summaryImages(render("raw"), null);
  const [png, tiff] = rows;
  assert.equal(pngRowOf(tiff, rows)?.key, png.key);
  assert.equal(pngRowOf(png, rows), null);
  assert.deepEqual(tiffKeysOf(png, rows), [tiff.key]);
  assert.deepEqual(tiffKeysOf(tiff, rows), []);
});

test("a TIFF is never ticked from the start: the import state has no global TIFF box any more", () => {
  assert.ok(!("joinTiff" in NO_IMPORT));
  assert.ok(
    !read("../screens/detect/ImportSummary.tsx").includes("JoinTiffBox"),
  );
  assert.ok(!read("../components/ImportProvider.tsx").includes("joinTiff"));
});

test("the picture of a card is not a button and carries no badge: « Régler l'image » is the one way into the editor", () => {
  const thumb = screen.slice(
    screen.indexOf("function Thumb("),
    screen.indexOf("/** The name of the Finder"),
  );
  assert.doesNotMatch(thumb, /onClick|<button|role="button"|tabIndex/);
  assert.doesNotMatch(thumb, /Icon|badge/);
  // The editor's button carries the sliders icon, once.
  assert.equal(screen.match(/name="sliders"/g)?.length, 1);
  assert.match(screen, /onClick=\{\(\) => onAdjust\(row, view\.render\)\}/);
});

test("the TIFF box warns when ticked, waits for its PNG, and its words are in the ten languages", () => {
  assert.match(screen, /tiffOn && <TiffWarning/);
  assert.match(screen, /<TiffWarning/);
  assert.match(
    read("../components/TiffWarning.tsx"),
    /detect\.send\.tiffWarning/,
  );
  assert.match(screen, /disabled=\{!tiffOpen\}/);
  assert.match(screen, /detect\.send\.tiffNeedsPng/);
  for (const locale of [
    "fr",
    "en",
    "de",
    "es",
    "it",
    "ja",
    "pt-BR",
    "ru",
    "zh-Hans",
    "zh-Hant",
  ]) {
    const dict = JSON.parse(read(`../i18n/${locale}.json`)) as Record<
      string,
      string
    >;
    for (const key of [
      "detect.send.toSend",
      "detect.send.tiffWarning",
      "detect.send.tiffNeedsPng",
    ])
      assert.ok(dict[key], `${locale}: ${key}`);
    assert.ok(!/[;；]/.test(dict["detect.send.tiffWarning"]), locale);
    assert.ok(!/—/.test(dict["detect.send.tiffWarning"]), locale);
  }
});

test("the place of the file is one grey line cut short, its whole path the tooltip, « Afficher dans le Finder » beside it", () => {
  assert.match(screen, /className="as-send__path"/);
  assert.match(screen, /title=\{path\}/);
  assert.match(screen, /revealKey\(\)/);
  const css = read("../theme/detect-result.css");
  const path = css.slice(css.indexOf(".as-send__path {"));
  assert.match(path.slice(0, path.indexOf("}")), /text-overflow: ellipsis/);
  assert.match(
    path.slice(0, path.indexOf("}")),
    /color: var\(--as-color-muted-text\)/,
  );
});

test("the linear file is a light line, with no picture and no editor button", () => {
  const line = screen.slice(screen.indexOf("function LinearLine("));
  assert.doesNotMatch(line, /Thumb|onAdjust/);
  assert.match(line, /is-file/);
});

test("the card has the same two columns whatever the image, and one column in a narrow window", () => {
  const css = read("../theme/detect-result.css");
  const card = css.slice(css.indexOf(".as-treat__image {"));
  assert.match(
    card.slice(0, card.indexOf("}")),
    /grid-template-columns: 9rem minmax\(0, 1fr\)/,
  );
  assert.match(css, /@media \(max-width: 48rem\)/);
});
