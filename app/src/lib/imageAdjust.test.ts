import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

import {
  ADJUST_SPECS,
  NEUTRAL,
  adjustPixel,
  adjustPixels,
  adjustedLine,
  adjustedList,
  canonicalValues,
  cleanValues,
  clampValue,
  importText,
  isChanged,
  isAdjustable,
  isNeutral,
  reachText,
  rowAdjustment,
  spikesGroupText,
  valueText,
  type AdjustValues,
} from "./imageAdjust.ts";
import { SUPPORTED_LOCALES } from "../i18n/locale.ts";

const fr = JSON.parse(
  readFileSync(new URL("../i18n/fr.json", import.meta.url), "utf8"),
) as Record<string, string>;
const t = (key: string, params: Record<string, string | number> = {}) =>
  (fr[key] ?? key).replace(/\{(\w+)\}/g, (_, name) => String(params[name]));

/** A synthetic preview: a grey sky, a bluish nebula, a white star. */
function picture(): {
  base: Uint8ClampedArray;
  stars: Uint8ClampedArray;
  mask: Uint8ClampedArray;
} {
  const px = [
    [20, 20, 23],
    [40, 60, 110],
    [255, 255, 255],
    [0, 0, 0],
  ];
  const starLayer = [
    [0, 0, 0],
    [0, 0, 0],
    [180, 180, 180],
    [0, 0, 0],
  ];
  const weight = [0, 255, 255, 0];
  const base = new Uint8ClampedArray(px.length * 4);
  const stars = new Uint8ClampedArray(px.length * 4);
  const mask = new Uint8ClampedArray(px.length * 4);
  px.forEach((rgb, i) => {
    base.set([...rgb, 255], i * 4);
    stars.set([...starLayer[i], 255], i * 4);
    mask.set([weight[i], weight[i], weight[i], 255], i * 4);
  });
  return { base, stars, mask };
}

function run(values: Partial<AdjustValues>): Uint8ClampedArray {
  const { base, stars, mask } = picture();
  const out = new Uint8ClampedArray(base.length);
  adjustPixels(base, stars, mask, { ...NEUTRAL, ...values }, out);
  return out;
}

const lum = (a: ArrayLike<number>, i: number) =>
  0.2126 * a[i * 4] + 0.7152 * a[i * 4 + 1] + 0.0722 * a[i * 4 + 2];

test("every setting at its neutral value leaves the preview unchanged to the bit", () => {
  const { base } = picture();
  assert.deepEqual([...run({})], [...base]);
  for (const spec of ADJUST_SPECS) {
    assert.deepEqual([...run({ [spec.name]: NEUTRAL[spec.name] })], [...base]);
  }
  assert.equal(isNeutral({ ...NEUTRAL }), true);
  assert.equal(isNeutral({ ...NEUTRAL, colour: 1.7 }), false);
});

test("each setting moves the image the right way", () => {
  const { base } = picture();
  // Point noir: the sky darker.
  assert.ok(lum(run({ blackPoint: 6 }), 0) < lum(base, 0) - 5);
  // Luminosité: the nebula lifted or lowered, the black kept.
  assert.ok(lum(run({ brightness: 12 }), 1) > lum(base, 1));
  assert.ok(lum(run({ brightness: -12 }), 1) < lum(base, 1));
  assert.equal(lum(run({ brightness: 40 }), 3), 0);
  // Couleur: the nebula more coloured, the sky (weight 0) untouched.
  const chroma = (a: ArrayLike<number>, i: number) =>
    Math.max(a[i * 4], a[i * 4 + 1], a[i * 4 + 2]) -
    Math.min(a[i * 4], a[i * 4 + 1], a[i * 4 + 2]);
  assert.ok(chroma(run({ colour: 1.7 }), 1) > 1.4 * chroma(base, 1));
  assert.equal(chroma(run({ colour: 1.7 }), 0), chroma(base, 0));
  // Étoiles: the star reduced or kept, the nebula without stars untouched.
  assert.ok(lum(run({ stars: -40 }), 2) < lum(base, 2));
  assert.equal(lum(run({ stars: -40 }), 1), lum(base, 1));
});

test("the preview's formulas give what the script computes on the same pixels", () => {
  const data = JSON.parse(
    readFileSync(
      new URL("./imageAdjust.fixture.json", import.meta.url),
      "utf8",
    ),
  ) as {
    values: AdjustValues;
    input: number[];
    stars: number[];
    mask: number[];
    output: number[];
  };
  const v = data.values;
  let worst = 0;
  for (let i = 0; i < data.mask.length; i += 1) {
    const p = data.input.slice(i * 3, i * 3 + 3);
    const s = data.stars.slice(i * 3, i * 3 + 3);
    adjustPixel(
      p,
      v.stars / 100,
      s[0],
      s[1],
      s[2],
      v.blackPoint / 100,
      2 ** (-v.brightness / 50),
      v.colour,
      data.mask[i],
    );
    for (let c = 0; c < 3; c += 1) {
      worst = Math.max(worst, Math.abs(p[c] - data.output[i * 3 + c]));
    }
  }
  assert.ok(worst < 1e-4, `largest difference ${worst}`);
});

test("a bright coloured pixel lifted past 1 is never darkened (no ring around a star)", () => {
  const lum = (p: number[]) => 0.2126 * p[0] + 0.7152 * p[1] + 0.0722 * p[2];
  // The blue halo of a bright star: brightness +50 (gamma 0.5) and colour × 3
  // push its blue past 1; its luminance follows the curve, its hue stays.
  const halo = [0.55, 0.6, 1.0];
  const lifted = [...halo];
  adjustPixel(lifted, 0, 0, 0, 0, 0, 0.5, 1, 1);
  assert.ok(Math.abs(lum(lifted) - lum(halo) ** 0.5) < 1e-6, `${lifted}`);
  const stronger = [...halo];
  adjustPixel(stronger, 0, 0, 0, 0, 0, 1, 3, 1);
  assert.ok(Math.abs(lum(stronger) - lum(halo)) < 1e-6, `${stronger}`);
  assert.ok(stronger[2] >= stronger[1], `the halo stays blue: ${stronger}`);
});

test("the values under the sliders and the line of an image set read as designed", () => {
  assert.equal(valueText(t, "fr", "blackPoint", 0), "Comme le rendu");
  assert.equal(valueText(t, "fr", "blackPoint", 8), "8 %");
  assert.equal(valueText(t, "fr", "brightness", 12), "+12");
  assert.equal(valueText(t, "fr", "brightness", -12), "−12");
  assert.equal(valueText(t, "fr", "colour", 1.7), "× 1,7");
  assert.equal(valueText(t, "fr", "smoothing", 0), "Sans");
  assert.equal(valueText(t, "fr", "smoothing", 40), "40 %");
  assert.equal(valueText(t, "fr", "stars", -40), "Réduites 40 %");
  assert.equal(valueText(t, "fr", "stars", 30), "Renforcées 30 %");
  assert.equal(
    adjustedLine(t, "fr", {
      ...NEUTRAL,
      blackPoint: 8,
      brightness: 12,
      colour: 1.7,
    }),
    "Réglée : point noir 8 %, luminosité +12, couleur × 1,7",
  );
});

test("values read back are brought into their range, a missing one is neutral", () => {
  assert.deepEqual(cleanValues({ blackPoint: 99, colour: "x" }), {
    ...NEUTRAL,
    blackPoint: 20,
  });
  assert.deepEqual(cleanValues(null), NEUTRAL);
  assert.equal(cleanValues({ colour: 1.73 }).colour, 1.7);
});

test("no setting on « Sans traitement » nor on a TIFF, one on each PNG image", () => {
  assert.equal(isAdjustable({ path: "/n/M31/stacked/M31_natural.png" }), true);
  assert.equal(
    isAdjustable({
      path: "/n/M31/stacked/M31_natural.tif",
      fullResolution: true,
    }),
    false,
  );
  assert.equal(
    isAdjustable({ path: "/n/M31/stacked/M31_linear.tif", file: "linear" }),
    false,
  );
  assert.equal(
    isAdjustable({ path: "/n/M31/stacked/M31_linear.png", file: "linear" }),
    false,
  );
});

test("every language has the words of « Régler l'image »", () => {
  const keys = Object.keys(fr).filter((key) => key.startsWith("adjust."));
  assert.ok(keys.length > 20);
  for (const locale of SUPPORTED_LOCALES) {
    const dict = JSON.parse(
      readFileSync(new URL(`../i18n/${locale}.json`, import.meta.url), "utf8"),
    ) as Record<string, string>;
    for (const key of keys) assert.ok(dict[key], `${locale}: ${key}`);
  }
});

test("a saved setting reads its values in the words of the planche", () => {
  const nebulae: AdjustValues = {
    ...NEUTRAL,
    blackPoint: 8,
    brightness: 12,
    colour: 1.7,
  };
  assert.equal(
    adjustedList(t, "fr", nebulae),
    "point noir 8 %, luminosité +12, couleur × 1,7",
  );
  assert.equal(
    t("adjust.saved.values", { list: adjustedList(t, "fr", nebulae) }),
    "Les valeurs ci-dessous, sans l’image : point noir 8 %, luminosité +12, couleur × 1,7.",
  );
  assert.equal(
    adjustedList(t, "fr", { ...NEUTRAL, smoothing: 20, stars: -40 }),
    "lissage 20 %, étoiles réduites 40 %",
  );
  assert.equal(adjustedList(t, "fr", NEUTRAL), "Comme le rendu");
});

test("an import says what it added and names what it left out, never overwritten", () => {
  const plural = (key: string, params: Record<string, string | number> = {}) =>
    t(key, params);
  assert.equal(
    importText(plural, "fr", { added: ["Amas"], skipped: [] }),
    "1 réglage importé.",
  );
  assert.equal(
    importText(plural, "fr", {
      added: ["Amas", "Galaxies"],
      skipped: ["Nébuleuses en émission"],
    }),
    "2 réglages importés. Laissés de côté, nom déjà pris : « Nébuleuses en émission ».",
  );
  assert.equal(
    importText(plural, "fr", { added: [], skipped: ["A", "B"] }),
    "0 réglage importé. Laissés de côté, nom déjà pris : « A » et « B ».",
  );
});

test("the words of the saved settings are the planche's, in the ten languages", () => {
  const planche: Record<string, string> = {
    "adjust.saved.apply": "Appliquer un réglage enregistré",
    "adjust.saved.save": "Enregistrer ce réglage",
    "adjust.saved.empty":
      "Aucun réglage enregistré. Réglez une image, puis « Enregistrer ce réglage » : vous pourrez l’appliquer à d’autres images.",
    "adjust.saved.nameLabel": "Nom du réglage",
    "adjust.saved.confirmSave": "Enregistrer le réglage",
    "adjust.saved.title": "Réglages enregistrés",
    "adjust.saved.rename": "Renommer",
    "adjust.saved.remove": "Supprimer",
    "adjust.saved.removeKeeps":
      "Les images déjà réglées avec lui ne changent pas.",
    "adjust.saved.listHelp":
      "Choisir un réglage le montre sur l’aperçu. Rien n’est enregistré tant que vous n’enregistrez pas l’image.",
    "adjust.saved.export": "Exporter…",
    "adjust.saved.import": "Importer…",
  };
  for (const [key, text] of Object.entries(planche))
    assert.equal(fr[key], text, key);
  assert.equal(
    t("adjust.saved.applied", { name: "Nébuleuses en émission" }),
    "«\u00a0Nébuleuses en émission\u00a0» appliqué à l’aperçu.",
  );
  assert.equal(
    t("adjust.saved.removeAsk", { name: "Amas, étoiles réduites" }),
    "Supprimer « Amas, étoiles réduites » ?",
  );
  const keys = Object.keys(fr).filter(
    (key) =>
      key.startsWith("adjust.saved.") ||
      key.startsWith("errors.adjust_") ||
      key === "wizard.summary.replacesMain" ||
      key === "wizard.summary.setAsideAdjustedVariant",
  );
  assert.ok(keys.length >= 30);
  for (const locale of SUPPORTED_LOCALES) {
    const dict = JSON.parse(
      readFileSync(new URL(`../i18n/${locale}.json`, import.meta.url), "utf8"),
    ) as Record<string, string>;
    for (const key of keys) {
      assert.ok(dict[key], `${locale}: ${key}`);
      // The same placeholders as the French.
      const names = (text: string) =>
        [...text.matchAll(/\{(\w+)\}/g)].map((m) => m[1]).sort();
      assert.deepEqual(names(dict[key]), names(fr[key]), `${locale}: ${key}`);
    }
  }
});

test("the star separation's answer is kept: the effect that starts it does not run again when it marks itself running", () => {
  const screen = readFileSync(
    new URL("../screens/adjust/AdjustScreen.tsx", import.meta.url),
    "utf8",
  );
  const call = screen.indexOf("imageAdjustStars(path)");
  assert.ok(call > 0, "the screen asks for the star layer");
  const start = screen.lastIndexOf("useEffect(", call);
  const end = screen.indexOf("}, [", call);
  const deps = screen.slice(end + 3, screen.indexOf("]", end) + 1);
  // `setStars("running")` inside an effect that lists `stars` runs it again:
  // its cleanup marks the first call dead, and the line « Séparation des
  // étoiles… » stays once the layer is written.
  assert.match(screen.slice(start, call), /setStars\("running"\)/);
  assert.doesNotMatch(deps, /\bstars\b/, `deps ${deps}`);
});

test("the row of a set render's TIFF says the image set's TIFF, and the render's own TIFF once brought back", () => {
  const png = "/n/M31/stacked/M31_natural.png";
  const tif = "/n/M31/stacked/M31_natural.tif";
  const values: AdjustValues = { ...NEUTRAL, blackPoint: 8, colour: 1.7 };
  const set = {
    values,
    path: "/n/M31/stacked/adjusted/M31_natural_adjusted.png",
    sha256: "png-set",
    size: 101,
    tiff: {
      path: "/n/M31/stacked/adjusted/M31_natural_adjusted.tif",
      sha256: "tiff-set",
      size: 4096,
    },
  };
  const tiffRow = { path: tif, fullResolution: true, twinOf: png };
  assert.deepEqual(rowAdjustment({ path: png }, { [png]: set }), {
    path: set.path,
    sha256: "png-set",
    size: 101,
    values,
    quarterTurns: 0,
  });
  assert.deepEqual(rowAdjustment(tiffRow, { [png]: set }), {
    ...set.tiff,
    values,
    quarterTurns: 0,
  });
  // Brought back to the render (no image set): both rows are the render's.
  assert.equal(rowAdjustment({ path: png }, { [png]: null }), null);
  assert.equal(rowAdjustment(tiffRow, { [png]: null }), null);
  // An image set without a TIFF (a render written as PNG only) says nothing
  // on a TIFF row, and the TIFF row never takes the PNG's image set.
  assert.equal(rowAdjustment(tiffRow, { [png]: { ...set, tiff: null } }), null);
  assert.equal(rowAdjustment({ path: tif }, { [png]: set }), null);
});

const withSpikes = (spikes: number, spikesReach = 50): AdjustValues => ({
  ...NEUTRAL,
  spikes,
  spikesReach,
});

test("the spikes are off by default, and the reach alone is not a change", () => {
  assert.equal(NEUTRAL.spikes, 0);
  assert.equal(NEUTRAL.spikesReach, 50);
  assert.equal(isNeutral(withSpikes(0, 90)), true);
  assert.equal(isChanged(withSpikes(0, 90), "spikesReach"), false);
  assert.equal(isNeutral(withSpikes(5)), false);
  assert.deepEqual(canonicalValues(withSpikes(0, 90)), { ...NEUTRAL });
  assert.deepEqual(canonicalValues(withSpikes(20, 90)), withSpikes(20, 90));
  // Read back from a record of before the spikes: neutral.
  assert.deepEqual(cleanValues({ blackPoint: 8 }), {
    ...NEUTRAL,
    blackPoint: 8,
  });
  assert.equal(cleanValues({ spikes: 400, spikesReach: -3 }).spikes, 100);
  assert.equal(cleanValues({ spikes: 40, spikesReach: -3 }).spikesReach, -5);
  // The range goes down to the brightest star alone, and no further; a reach saved
  // before (0 and more) is read back as it was.
  assert.equal(cleanValues({ spikes: 40, spikesReach: -300 }).spikesReach, -30);
  for (const reach of [0, 5, 50, 95, 100]) {
    assert.equal(
      cleanValues({ spikes: 40, spikesReach: reach }).spikesReach,
      reach,
    );
  }
  assert.equal(clampValue("spikesReach", -28), -30);
});

test("the slider of the reach goes from the brightest star alone to the medium stars in steps of 5", () => {
  const spec = ADJUST_SPECS.find((item) => item.name === "spikesReach")!;
  assert.equal(spec.min, -30);
  assert.equal(spec.max, 100);
  assert.equal(spec.step, 5);
  // The default stays where it was.
  assert.equal(NEUTRAL.spikesReach, 50);
  assert.equal(clampValue("spikesReach", -30), -30);
  assert.equal(clampValue("spikesReach", -27), -25);
  assert.equal(clampValue("spikesReach", 3), 5);
});

test("the spikes say what they are in the words of the planche, last in the line", () => {
  assert.equal(valueText(t, "fr", "spikes", 0), "Sans");
  assert.equal(valueText(t, "fr", "spikes", 60), "60 %");
  assert.equal(spikesGroupText(t, "fr", withSpikes(0)), "Sans");
  assert.equal(spikesGroupText(t, "fr", withSpikes(60)), "Ajoutées, 60 %");
  assert.equal(reachText(t, "fr", 50, 63), "Brillantes · 63 étoiles");
  assert.equal(reachText(t, "fr", 0, 1), "Les plus brillantes · 1 étoile");
  assert.equal(reachText(t, "fr", -30, 1), "Les plus brillantes · 1 étoile");
  assert.equal(reachText(t, "fr", -20, 3), "Les plus brillantes · 3 étoiles");
  assert.equal(reachText(t, "fr", -30, 1), "Les plus brillantes · 1 étoile");
  assert.equal(reachText(t, "fr", -20, 3), "Les plus brillantes · 3 étoiles");
  assert.equal(
    reachText(t, "fr", 100, 220),
    "Jusqu’aux moyennes · 220 étoiles",
  );
  assert.equal(
    adjustedList(t, "fr", { ...withSpikes(60, 20), stars: -10, blackPoint: 8 }),
    "point noir 8 %, étoiles réduites 10 %, aigrettes ajoutées 60 %",
  );
  assert.equal(
    adjustedLine(t, "fr", { ...withSpikes(60), stars: -10 }, 1),
    t("adjust.applied", {
      list: "tournée de 90° à droite, étoiles réduites 10 %, aigrettes ajoutées 60 %",
    }),
  );
  // The reach alone is never a term of the line.
  assert.equal(adjustedList(t, "fr", withSpikes(0, 90)), "Comme le rendu");
});

test("every language says the spikes, in the register of its file", () => {
  const keys = [
    "adjust.spikes.title",
    "adjust.spikes.note",
    "adjust.spikes.intro",
    "adjust.spikes.group.added",
    "adjust.spikes.strength.title",
    "adjust.spikes.strength.help",
    "adjust.spikes.reach.title",
    "adjust.spikes.reach.help",
    "adjust.spikes.reach.first",
    "adjust.spikes.reach.brightest",
    "adjust.spikes.reach.bright",
    "adjust.spikes.reach.medium",
    "adjust.spikes.reach.value",
    "adjust.spikes.reach.count.one",
    "adjust.spikes.reach.count.other",
    "adjust.applied.spikes",
  ];
  for (const locale of SUPPORTED_LOCALES) {
    const dict = JSON.parse(
      readFileSync(new URL(`../i18n/${locale}.json`, import.meta.url), "utf8"),
    ) as Record<string, string>;
    for (const key of keys) {
      assert.ok(dict[key], `${locale}: ${key}`);
      assert.ok(!/[;；—]/.test(dict[key]), `${locale}: ${key} juxtaposes`);
    }
    // The help says plainly that the spikes are not in the data.
    assert.ok(dict["adjust.spikes.intro"].length > 30, locale);
  }
});
