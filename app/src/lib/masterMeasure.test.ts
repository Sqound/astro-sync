import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

import { SUPPORTED_LOCALES, createTranslator } from "../i18n/locale.ts";
import type { Capabilities, ImageQuality } from "./bridge";
import {
  CAPABILITIES_ADVANCED,
  CAPABILITIES_PLUS,
  detectionAllFound,
  detectionMixed,
} from "./homeFixtures.ts";
import {
  MASTER_REFUSALS,
  masterCurrentText,
  masterEntryState,
  masterQualityText,
  masterRefusalText,
  masterTargets,
} from "./masterMeasure.ts";
import { imageQualityText } from "./wizardSummary.ts";

const fr = createTranslator("fr");

const QUALITY: ImageQuality = {
  fwhmPixels: 3.8,
  fwhmArcsec: 2.14,
  eccentricity: 0.31,
  starCount: 212,
  measuredOn: "linear",
  measuredAt: "2026-10-14T09:00:00Z",
  tool: { name: "Astro Sync", version: "0.1.0" },
};

const present = [{ id: "python-env", state: "present" as const }];

test("the entry follows the right first, then the Python environment (the stacking's order)", () => {
  const state = (
    capabilities: Capabilities | null | undefined,
    tools: { id: string; state: "present" | "missing" | "downloaded" }[],
  ) => masterEntryState({ capabilities, tools });
  assert.equal(state(CAPABILITIES_PLUS, present), "available");
  assert.equal(
    state(CAPABILITIES_PLUS, [{ id: "python-env", state: "downloaded" }]),
    "available",
  );
  // Without analysis.logs: greyed with the reserved sentence, whatever the tool.
  assert.equal(state(CAPABILITIES_ADVANCED, present), "reserved");
  assert.equal(state(CAPABILITIES_ADVANCED, []), "reserved");
  assert.equal(state(null, present), "reserved");
  assert.equal(state(undefined, present), "reserved");
  // The right is there, the environment is not.
  assert.equal(state(CAPABILITIES_PLUS, []), "toolMissing");
  assert.equal(
    state(CAPABILITIES_PLUS, [{ id: "python-env", state: "missing" }]),
    "toolMissing",
  );
});

test("a master is offered for the targets of the night that have a folder", () => {
  const targets = masterTargets(detectionAllFound());
  assert.deepEqual(
    targets.map((target) => target.designation),
    ["NGC 7000", "IC 1396", "M31"],
  );
  assert.ok(targets.every((target) => target.folderName.length > 0));
  assert.deepEqual(masterTargets(null), []);
  const none = detectionMixed();
  none.targets = none.targets.map((target) => ({
    ...target,
    folderName: null,
  }));
  assert.deepEqual(masterTargets(none), []);
  const outside = detectionAllFound();
  outside.targets = outside.targets.map((target) => ({
    ...target,
    inNight: false,
  }));
  assert.deepEqual(masterTargets(outside), []);
});

test("every refusal has its sentence in the ten languages, and an unknown word reads as an unreadable file", async () => {
  for (const locale of SUPPORTED_LOCALES) {
    const t = createTranslator(locale);
    const seen = new Set<string>();
    for (const code of MASTER_REFUSALS) {
      const text = masterRefusalText(t, code);
      assert.ok(text.length > 8, `${locale} ${code}`);
      assert.ok(!text.startsWith("master."), `${locale} ${code} has no text`);
      seen.add(text);
    }
    assert.equal(
      seen.size,
      MASTER_REFUSALS.length,
      `${locale}: two words, one sentence`,
    );
    assert.equal(
      masterRefusalText(t, "something-new"),
      masterRefusalText(t, "unreadable"),
    );
  }
  assert.match(masterRefusalText(fr, "stretched"), /master linéaire/);
});

test("the window says what to give, in the ten languages: the linear master, before any stretch, FITS or XISF, and that a stretched image or a JPEG cannot be measured", async () => {
  const dictionaries = await Promise.all(
    SUPPORTED_LOCALES.map(async (locale) => ({
      locale,
      dictionary: JSON.parse(
        await readFile(
          new URL(`../i18n/${locale}.json`, import.meta.url),
          "utf8",
        ),
      ) as Record<string, string>,
    })),
  );
  for (const { locale, dictionary } of dictionaries) {
    for (const key of ["master.give", "master.refusal.stretched"]) {
      assert.match(dictionary[key], /FITS/, `${locale} ${key}`);
      assert.match(dictionary[key], /XISF/, `${locale} ${key}`);
    }
    assert.match(dictionary["master.give"], /JPEG/, locale);
    assert.match(
      dictionary["master.refusal.unsupported_format"],
      /JPEG/,
      locale,
    );
    // The sentences that carry a value keep its placeholder.
    assert.match(dictionary["master.pending.measure"], /\{file\}/, locale);
    assert.match(dictionary["master.current"], /\{file\}/, locale);
    assert.match(
      dictionary["wizard.summary.imageQualityMaster"],
      /\{fwhm\}.*\{eccentricity\}.*\{count\}/,
      locale,
    );
    assert.match(
      dictionary["wizard.summary.imageQualityMasterShort"],
      /\{fwhm\}.*\{count\}/,
      locale,
    );
    // The reserved entry says what the stacking's options say.
    assert.ok(dictionary["wizard.treatments.reserved"].length > 0);
  }
  assert.equal(
    createTranslator("fr")("master.give"),
    "Donnez le master linéaire, tel qu’il sort de l’empilement, avant tout étirement ou traitement (FITS ou XISF). Une image étirée ou un JPEG ne se mesure pas.",
  );
});

test("the figures of a measured master read as the summary's, on the master's own sentence", () => {
  assert.equal(
    masterQualityText(fr, "fr", QUALITY),
    "Qualité mesurée sur votre master\u00a0: FWHM 2,1″ · excentricité 0,31 · 212 étoiles",
  );
  assert.equal(
    masterQualityText(fr, "fr", {
      ...QUALITY,
      eccentricity: null,
      fwhmArcsec: null,
    }),
    "Qualité mesurée sur votre master\u00a0: FWHM 3,8 px · 212 étoiles",
  );
  // The stacking keeps its own sentence.
  assert.match(
    imageQualityText(fr, "fr", QUALITY),
    /^Qualité de l’image empilée/,
  );
  assert.equal(
    masterCurrentText(fr, { master: "M31_integration.xisf" }),
    "Mesure gardée, faite sur M31_integration.xisf.",
  );
  for (const locale of SUPPORTED_LOCALES) {
    const t = createTranslator(locale);
    const text = masterQualityText(t, locale, QUALITY);
    assert.ok(
      !text.includes("{") && text.includes("212"),
      `${locale}: ${text}`,
    );
  }
});
