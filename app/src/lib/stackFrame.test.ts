import assert from "node:assert/strict";
import test from "node:test";

import type { FrameNotch, TargetFrame } from "./bridge";
import {
  applyFrameToAll,
  frameChoiceOf,
  frameLine,
  frameRank,
  frameState,
  frameToRun,
  frameView,
  loadTargetFrames,
  saveTargetFrames,
  withTargetFrame,
} from "./stackFrame.ts";
import { createTranslator, SUPPORTED_LOCALES } from "../i18n/locale.ts";

const fr = createTranslator("fr");
const NBSP = " ";
const plain = (text: string) => text.replaceAll(/[  ]/g, " ");

/** The five notches of the planche (2 160 × 3 840, turned by 45°, 120 poses). */
function notch(
  rank: number,
  width: number,
  height: number,
  share: number,
  full: number,
  lowest: number,
): FrameNotch {
  return {
    rank,
    top: (3840 - height) / 2,
    left: (2160 - width) / 2,
    height,
    width,
    shareOfFrame: share,
    fullShare: full,
    lowestFrames: lowest,
  };
}

const TURNED: TargetFrame = {
  fieldRotationDegrees: 45.2,
  registered: 120,
  measure: {
    frameHeight: 3840,
    frameWidth: 2160,
    notches: [
      notch(0, 1520, 1536, 0.28, 1, 120),
      notch(1, 1680, 2112, 0.43, 0.961, 40),
      notch(2, 1840, 2688, 0.6, 0.89, 20),
      notch(3, 2000, 3264, 0.79, 0.81, 8),
      notch(4, 2160, 3840, 1, 0.705, 1),
    ],
    automatic: 4,
    rank: null,
  },
};

const STILL: TargetFrame = {
  fieldRotationDegrees: 0.3,
  registered: 40,
  measure: {
    frameHeight: 3840,
    frameWidth: 2160,
    notches: [],
    automatic: null,
    rank: null,
  },
};

test("before the first stacking the block is greyed on « Automatique », measured at the alignment", () => {
  assert.equal(frameState(null), "before");
  const view = frameView(fr, "fr", null, undefined, 7200);
  assert.equal(view.enabled, false);
  assert.equal(view.name, "Automatique");
  assert.equal(view.pending, true);
  assert.equal(view.automatic, false);
  assert.equal(
    plain(view.exposure),
    "Zone commune si le champ n’a pas tourné ; tout le champ s’il a tourné.",
  );
  assert.ok(view.after?.includes("le curseur devient réglable"));
  assert.deepEqual(
    view.stops.map((stop) => stop.percent),
    ["–", "–", "–", "–", "–"],
  );
  assert.equal(view.stops[0].name, "Zone commune");
  assert.equal(view.stops[4].name, "Tout le champ");
  assert.equal(view.note, null);
  assert.ok(view.foot.startsWith("Valeur de départ"));
});

test("after the alignment, a field that turned: Automatic is the whole field, its notches measured", () => {
  assert.equal(frameState(TURNED), "turned");
  const view = frameView(fr, "fr", TURNED, undefined, 7200);
  assert.equal(view.enabled, true);
  assert.equal(
    plain(view.line),
    "Le champ a tourné de 45° cette nuit : les bords sont couverts par moins de poses que le centre. Choisissez la taille du cadre que vous gardez.",
  );
  assert.equal(view.rank, 4);
  assert.equal(view.name, "Tout le champ");
  assert.equal(view.automatic, true);
  assert.equal(
    plain(view.size ?? ""),
    "2 160 × 3 840 px · 100 % de l’image entière",
  );
  assert.equal(plain(view.exposure), "30 % du cadre à 1 min au lieu de 2 h 00");
  assert.deepEqual(
    view.stops.map((stop) => plain(stop.percent)),
    ["28 %", "43 %", "60 %", "79 %", "100 %"],
  );
  assert.deepEqual(
    view.stops.map((stop) => stop.automatic),
    [false, false, false, false, true],
  );
  assert.ok(view.note?.text.includes("Rien n’est perdu"));
  assert.ok(view.foot.includes("stacked/"));
});

test("a notch moved by hand says what it loses or gains, and the next treatment cuts it", () => {
  const between = frameView(fr, "fr", TURNED, { rank: 2, share: 0.6 }, 7200);
  assert.equal(between.name, "Cadre intermédiaire");
  assert.equal(between.automatic, false);
  assert.equal(
    plain(between.size ?? ""),
    "1 840 × 2 688 px · 60 % de l’image entière",
  );
  assert.equal(
    plain(between.exposure),
    "11 % du cadre à 20 min au lieu de 2 h 00",
  );
  assert.equal(
    plain(between.note?.text ?? ""),
    "Un compromis : 40 % du champ est perdu ; les bords gardés restent plus bruités que le centre.",
  );
  assert.ok(between.foot.includes("recoupe le champ entier"));
  const common = frameView(fr, "fr", TURNED, { rank: 0 }, 7200);
  assert.equal(common.name, "Zone commune");
  assert.equal(
    common.exposure,
    "Toutes les poses couvrent chaque point du cadre.",
  );
  assert.equal(
    plain(common.note?.text ?? ""),
    "Le plus petit cadre : toutes les poses couvrent chaque point, mais 72 % du champ est perdu.",
  );
  // Without the exposure of the target, in poses.
  assert.equal(
    plain(frameView(fr, "fr", TURNED, { rank: 2 }, null).exposure),
    "11 % du cadre couvert par 20 poses au lieu de 120",
  );
});

test("a field that did not turn has nothing to choose", () => {
  assert.equal(frameState(STILL), "still");
  const view = frameView(fr, "fr", STILL, { rank: 1 }, 7200);
  assert.equal(view.enabled, false);
  assert.equal(view.name, "Tout le champ");
  assert.equal(view.automatic, true);
  assert.equal(view.rank, 4);
  assert.ok(view.line.startsWith("Le champ n’a pas tourné"));
  assert.equal(view.note?.tone, "ok");
  assert.equal(plain(view.stops[4].percent), "100 %");
  assert.equal(view.stops[4].name, "Tout le cadre");
  // A field that turned with no area every pose covers: the whole field, said so.
  const far = { ...STILL, fieldRotationDegrees: 170 };
  assert.equal(frameState(far), "noCommon");
  assert.ok(frameView(fr, "fr", far, undefined, null).line.includes("170°"));
});

test("the notch of Automatic brings Automatic back; another is kept with its share", () => {
  assert.equal(frameChoiceOf(TURNED, 4), null);
  assert.deepEqual(frameChoiceOf(TURNED, 1), { rank: 1, share: 0.43 });
  assert.equal(frameChoiceOf(null, 1), null);
  assert.equal(frameRank(TURNED, undefined), 4);
  assert.equal(frameRank(TURNED, { rank: 9 }), 4);
  assert.equal(frameRank(STILL, { rank: 0 }), 4);
});

test("the card says the frame only when it was moved by hand", () => {
  assert.equal(frameLine(fr, undefined), null);
  assert.equal(
    plain(frameLine(fr, { rank: 2, share: 0.6 }) ?? ""),
    "Cadre : Cadre intermédiaire, 60 % du champ",
  );
  assert.equal(plain(frameLine(fr, { rank: 0 }) ?? ""), "Cadre : Zone commune");
});

test("« Appliquer à toutes les cibles » copies a frame moved by hand by its rank; Automatic stays Automatic", () => {
  const before = { "ic-1396": { rank: 1, share: 0.5 } };
  // Moved by hand: every target gets the rank, recomputed on its own field.
  const copied = applyFrameToAll(
    before,
    "ngc-7000",
    ["ngc-7000", "ic-1396", "m-31"],
    {
      rank: 2,
      share: 0.6,
    },
  );
  assert.deepEqual(copied, {
    "ngc-7000": { rank: 2, share: 0.6 },
    "ic-1396": { rank: 2 },
    "m-31": { rank: 2 },
  });
  // Automatic: the others keep their own frame.
  assert.deepEqual(
    applyFrameToAll(before, "ngc-7000", ["ic-1396", "m-31"], null),
    before,
  );
  assert.deepEqual(withTargetFrame(copied, "m-31", null), {
    "ngc-7000": { rank: 2, share: 0.6 },
    "ic-1396": { rank: 2 },
  });
  assert.equal(frameToRun(copied, "ic-1396"), 2);
  assert.equal(frameToRun(copied, "unknown"), null);
});

test("the frames survive the window closed, and what cannot be read is Automatic", () => {
  const held = new Map<string, string>();
  const storage = {
    getItem: (key: string) => held.get(key) ?? null,
    setItem: (key: string, value: string) => void held.set(key, value),
  };
  saveTargetFrames(storage, "night-1", {
    "ngc-7000": { rank: 1, share: 0.43 },
  });
  assert.deepEqual(loadTargetFrames(storage, "night-1"), {
    "ngc-7000": { rank: 1, share: 0.43 },
  });
  assert.deepEqual(loadTargetFrames(storage, "night-2"), {});
  held.set(
    "astro-sync.frames:night-3",
    JSON.stringify({
      a: { rank: 7 },
      b: { rank: "1" },
      c: { rank: 3, share: 4 },
      d: null,
    }),
  );
  assert.deepEqual(loadTargetFrames(storage, "night-3"), { c: { rank: 3 } });
  held.set("astro-sync.frames:night-4", "{not json");
  assert.deepEqual(loadTargetFrames(storage, "night-4"), {});
});

test("every language says the frame, with the same words as the plan's slider", () => {
  for (const locale of SUPPORTED_LOCALES) {
    const t = createTranslator(locale);
    const view = frameView(t, locale, TURNED, { rank: 0 }, 7200);
    for (const text of [
      view.line,
      view.name,
      view.size ?? "",
      view.exposure,
      view.note?.text ?? "",
      view.foot,
    ]) {
      assert.ok(!text.includes("wizard.frame."), `${locale}: ${text}`);
      assert.ok(!/\{\w+\}/.test(text), `${locale}: ${text}`);
    }
    // One vocabulary (question 6): the plan's notches are named as the night's.
    assert.equal(
      t("multiNight.framing.nameCropped"),
      t("wizard.frame.nameCommon"),
      locale,
    );
    assert.equal(
      t("multiNight.framing.nameWhole"),
      t("wizard.frame.nameWhole"),
      locale,
    );
    assert.equal(
      t("multiNight.framing.stopCropped"),
      t("wizard.frame.stopCommon"),
      locale,
    );
    assert.equal(
      t("multiNight.framing.stopWhole"),
      t("wizard.frame.stopWhole"),
      locale,
    );
    assert.equal(
      t("multiNight.framing.default"),
      t("wizard.frame.automatic"),
      locale,
    );
    assert.equal(
      t("multiNight.framing.slider"),
      t("wizard.frame.slider"),
      locale,
    );
  }
  assert.ok(fr("wizard.frame.cardLine", { frame: "x" }).includes(`${NBSP}:`));
});
