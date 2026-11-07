import assert from "node:assert/strict";
import test from "node:test";

import { SUPPORTED_LOCALES, createTranslator } from "../i18n/locale.ts";
import type { ImagePlan, Treatment } from "./bridge";
import {
  M31,
  NIGHT_FOLDER,
  detectionMixed,
  onlineMixed,
  precheck,
} from "./homeFixtures.ts";
import {
  isFullResolutionTwin,
  joinTiffLabel,
  renderFormatText,
  withoutTiffText,
} from "./renderFormat.ts";
import type { Seg } from "./wizardFiles.ts";
import { summaryView } from "./wizardSummary.ts";

const fr = createTranslator("fr");
const text = (segs: readonly Seg[]) =>
  segs
    .map((s) => (typeof s === "string" ? s : "strong" in s ? s.strong : s.code))
    .join("");
const MB = 1_000_000;

/** A file of a render of this sending's stacking. */
function file(
  name: string,
  size: number,
  twin: string | null,
  decision: ImagePlan["decision"],
): ImagePlan {
  return {
    image: {
      targetId: M31,
      path: `${NIGHT_FOLDER}/M31/stacked/M31_2026-10-07_${name}`,
      sha256: name.padEnd(64, "0"),
      size,
      kind: "stacked",
      render: name.split(".")[0],
      twinSha256: twin,
    },
    decision,
  };
}

const NATURAL_PNG = file("raw.png", 13 * MB, "t".repeat(64), {
  decision: "new_capture",
});
const NATURAL_TIFF = file("raw.tif", 47 * MB, "raw.png".padEnd(64, "0"), {
  decision: "variant_of_new",
  firstSha256: "raw.png".padEnd(64, "0"),
});
const LINEAR = file("linear.tif", 20 * MB, null, {
  decision: "variant_of_new",
  firstSha256: "raw.png".padEnd(64, "0"),
});

function sentRows(
  plans: ImagePlan[],
  extra: Parameters<typeof precheck>[1] = {},
) {
  const view = summaryView(fr, "fr", {
    precheck: precheck(plans, extra),
    detection: detectionMixed(),
    online: onlineMixed(),
    treatments: new Set<Treatment>(["logs"]),
    stackTargets: new Set(),
    resume: false,
    privateServer: false,
    date: "7 oct.",
    renderName: (id) => ({ raw: "Rendu naturel" })[id] ?? id,
  });
  return {
    view,
    rows: view.groups
      .find((g) => g.key === "sent")!
      .rows.map((r) => [text(r.segs).replace(/ · SHA-256 .*$/, ""), r.value]),
  };
}

test("the format of a render: its PNG, its TIFF in full resolution, a TIFF alone", () => {
  assert.equal(renderFormatText(fr, NATURAL_PNG.image), "PNG");
  assert.equal(
    renderFormatText(fr, NATURAL_TIFF.image),
    "TIFF pleine résolution",
  );
  assert.equal(renderFormatText(fr, LINEAR.image), "TIFF seul");
  assert.ok(isFullResolutionTwin(NATURAL_TIFF.image));
  assert.ok(!isFullResolutionTwin(NATURAL_PNG.image));
  assert.ok(!isFullResolutionTwin(LINEAR.image), "the linear is no twin");
});

test("PNG only: the target says its images and their weight, each render its format", () => {
  const { rows } = sentRows([NATURAL_PNG]);
  assert.deepEqual(rows.slice(0, 2), [
    ["Image empilée de M31", "1 image · 13 Mo"],
    ["Rendu naturel · PNG", "13 Mo"],
  ]);
});

test("PNG and TIFF: the TIFF follows its PNG, the total of the target counts both", () => {
  const { rows } = sentRows([NATURAL_PNG, NATURAL_TIFF]);
  assert.deepEqual(rows.slice(0, 3), [
    ["Image empilée de M31", "2 images · 60 Mo"],
    ["Rendu naturel · PNG", "13 Mo"],
    ["Rendu naturel · TIFF pleine résolution", "47 Mo"],
  ]);
});

test("the linear render leaves as its TIFF, said « TIFF seul », beside the PNG of the others", () => {
  const { rows } = sentRows([NATURAL_PNG, LINEAR]);
  assert.deepEqual(rows.slice(0, 3), [
    ["Image empilée de M31", "2 images · 33 Mo"],
    ["Rendu naturel · PNG", "13 Mo"],
    ["Sans traitement · TIFF seul", "20 Mo"],
  ]);
});

test("a refusal for room says what the sending weighs without the TIFF when that fits, never otherwise", () => {
  const finding = {
    limit: "storage_quota" as const,
    toSendBytes: 667 * MB,
    remainingBytes: 560 * MB,
    quotaBytes: 1000 * MB,
  };
  const refused = (fullResolutionBytes: number) => {
    const check = precheck([NATURAL_PNG, NATURAL_TIFF], {
      allowed: false,
      findings: [finding],
      toSend: 667 * MB,
    });
    check.quota.fullResolutionBytes = fullResolutionBytes;
    return summaryView(fr, "fr", {
      precheck: check,
      detection: detectionMixed(),
      online: onlineMixed(),
      treatments: new Set<Treatment>(["logs"]),
      stackTargets: new Set(),
      resume: false,
      privateServer: false,
      date: "7 oct.",
    }).refusals;
  };
  assert.deepEqual(refused(188 * MB), [
    "À envoyer : 667 Mo ; place restante : 560 Mo sur 1 Go.",
    "Sans les TIFF : 479 Mo, cela tient. Décochez « Joindre aussi les TIFF » pour envoyer les PNG.",
  ]);
  // Without the TIFF it still does not fit, or there are none: nothing more.
  assert.equal(refused(50 * MB).length, 1);
  assert.equal(refused(0).length, 1);
  assert.equal(
    withoutTiffText(fr, "fr", { toSendBytes: 10, fullResolutionBytes: 4 }, 6),
    "Sans les TIFF : 6 octets, cela tient. Décochez « Joindre aussi les TIFF » pour envoyer les PNG.",
  );
});

test("the stacking to come says what it will send: PNG, or PNG + TIFF when the box is ticked", () => {
  const line = (includeTiff: boolean) =>
    summaryView(fr, "fr", {
      precheck: precheck([]),
      detection: detectionMixed(),
      online: onlineMixed(),
      treatments: new Set<Treatment>(["stacking"]),
      stackTargets: new Set([M31]),
      resume: false,
      privateServer: false,
      date: "7 oct.",
      includeTiff,
    })
      .groups.find((g) => g.key === "sent")!
      .rows.map((r) => [text(r.segs), r.value])[0];
  assert.deepEqual(line(false), ["Image empilée de M31 · PNG", "calculé ici"]);
  assert.deepEqual(line(true), [
    "Image empilée de M31 · PNG + TIFF",
    "calculé ici",
  ]);
});

test("the box says the weight of one TIFF when the TIFF are known, and why it is a choice, in the ten languages", () => {
  assert.equal(
    joinTiffLabel(fr, "fr", { count: 4, bytes: 188 * MB }),
    "Joindre aussi les TIFF (pleine résolution, ≈ 47 Mo chacun)",
  );
  assert.equal(
    joinTiffLabel(fr, "fr", null),
    "Joindre aussi les TIFF (pleine résolution)",
  );
  assert.equal(
    fr("uploadFormat.help"),
    "Le TIFF ne sert qu’au téléchargement en pleine résolution ; la galerie affiche le PNG.",
  );
  for (const locale of SUPPORTED_LOCALES) {
    const t = createTranslator(locale);
    for (const key of [
      "uploadFormat.joinTiff",
      "uploadFormat.help",
      "uploadFormat.tiffFullResolution",
      "uploadFormat.tiffOnly",
      "uploadFormat.pngAndTiff",
      "settings.rendersUpload.title",
      "settings.rendersUpload.png",
      "settings.rendersUpload.both",
      "settings.rendersUpload.line",
    ]) {
      assert.notEqual(t(key), key, `${locale}: ${key}`);
    }
    const sized = joinTiffLabel(t, locale, { count: 1, bytes: 47 * MB });
    assert.match(sized, /47/, `${locale}: ${sized}`);
    assert.match(sized, /TIFF/, `${locale}: ${sized}`);
    const without = t("uploadFormat.withoutTiff", { size: "479 Mo" });
    assert.ok(without.includes("479 Mo"), `${locale}: ${without}`);
  }
  // The register of each language: « du » in German, 你 in simplified Chinese.
  assert.match(createTranslator("de")("settings.rendersUpload.line"), /\bdu\b/);
  assert.match(
    createTranslator("zh-Hans")("settings.rendersUpload.line"),
    /你/,
  );
});
