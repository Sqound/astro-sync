import assert from "node:assert/strict";
import test from "node:test";

import { createTranslator } from "../i18n/locale.ts";
import {
  EMPTY_DRAFT,
  PERSONAL_LABELS,
  cardLines,
  counterText,
  draftFromGuess,
  draftToCreate,
  fitsView,
  formatField,
  formatPosition,
  imageAddress,
  isOfflineFailure,
  moreAction,
  parseDeclination,
  parseRightAscension,
  searchReady,
} from "./targetSearch.ts";
import {
  CARD,
  FITS_FIELD,
  FITS_FOUND,
  FITS_MUTE,
  SEARCH_NEBULA,
} from "./wizardTargetsFixtures.ts";

const fr = createTranslator("fr");
const en = createTranslator("en");
const plain = (text: string) => text.replace(/ /g, " ");

test("the catalogue is asked from two characters; a network failure is « hors ligne », a refusal is not", () => {
  assert.equal(searchReady("m"), false);
  assert.equal(searchReady(" m "), false);
  assert.equal(searchReady("m3"), true);
  assert.equal(isOfflineFailure({ code: "network_error", message: "" }), true);
  assert.equal(
    isOfflineFailure({ code: "service_start_timeout", message: "" }),
    true,
  );
  assert.equal(
    isOfflineFailure({ code: "portal_refused", message: "" }),
    false,
  );
  assert.equal(isOfflineFailure(new Error("x")), false);
});

test("a card writes its lines: designation, common name, type · constellation, magnitude · size", () => {
  assert.deepEqual(cardLines(fr, "fr", CARD.ngc7000), {
    designation: "NGC 7000",
    name: "Nébuleuse de l’Amérique du Nord",
    kind: "Nébuleuse en émission · Cygne",
    details: "magnitude 4,0 · 120 × 100′",
  });
  assert.equal(
    cardLines(en, "en", CARD.m31).details,
    "magnitude 3.4 · 190 × 60′",
  );
  const bare = {
    ...CARD.m27,
    magnitude: null,
    sizeLabel: null,
    typeLabel: null,
  };
  assert.deepEqual(cardLines(fr, "fr", bare).kind, "Petit Renard");
  assert.equal(cardLines(fr, "fr", bare).details, "");
});

test("the preview is read on the service; a drawn one or an absolute address stays as it is", () => {
  assert.equal(
    imageAddress("https://s.example.test/", "/img/m31.jpg"),
    "https://s.example.test/img/m31.jpg",
  );
  assert.equal(imageAddress(null, "/img/m31.jpg"), null);
  assert.equal(imageAddress("https://s", null), null);
  assert.ok(imageAddress("https://s", CARD.m31.imagePath)?.startsWith("data:"));
});

test("the counter once, « Afficher plus » up to 24, then « Affinez la recherche »", () => {
  assert.equal(plain(counterText(fr, "fr", 8, 23)), "8 résultats sur 23");
  assert.equal(plain(counterText(fr, "fr", 1, 1)), "1 résultat sur 1");
  assert.equal(counterText(en, "en", 8, 23), "8 results of 23");
  assert.equal(moreAction(8, 23), "more");
  assert.equal(moreAction(23, 23), null);
  assert.equal(moreAction(24, 40), "refine");
  assert.equal(SEARCH_NEBULA.targets.length, 8);
});

test("a position and a field are written in the expected form", () => {
  assert.equal(formatPosition(10.6847, 41.2688), "0 h 43 min · +41° 16′");
  assert.equal(formatPosition(314, 44.3333), "20 h 56 min · +44° 20′");
  assert.equal(formatPosition(83.82, -5.39), "5 h 35 min · −5° 23′");
  assert.equal(formatField([2.1, 1.6], "fr"), "2,1° × 1,6°");
});

test("a FITS read: found with its proof, the field's candidates, a file that says nothing", () => {
  const found = fitsView(fr, "fr", FITS_FOUND);
  assert.equal(plain(found.heading), "Trouvée dans l’en-tête");
  assert.deepEqual(
    found.cards.map((c) => c.designation),
    ["M31"],
  );
  assert.equal(
    found.proof ?? "",
    "L’en-tête dit OBJECT = « M 31 » et une position (0 h 43 min · +41° 16′) : les deux désignent M31.",
  );
  const field = fitsView(fr, "fr", FITS_FIELD);
  assert.equal(
    plain(field.heading),
    "Dans le champ de l’image (2,1° × 1,6°), la plus proche d’abord",
  );
  assert.deepEqual(
    field.cards.map((c) => c.designation),
    ["NGC 7000", "IC 5070"],
  );
  assert.equal(
    field.proof ?? "",
    "L’en-tête n’a pas de OBJECT ; sa position est 20 h 56 min · +44° 20′.",
  );
  const mute = fitsView(fr, "fr", FITS_MUTE);
  assert.equal(mute.mute, true);
  assert.equal(
    mute.heading,
    "Ce fichier ne dit ni son objet ni sa position : cherchez la cible ci-dessus.",
  );
  assert.deepEqual(mute.cards, []);
});

test("an OBJECT the catalogue does not know offers « Créer … comme nouvelle cible » with the form filled", () => {
  const unknown = fitsView(fr, "fr", {
    ...FITS_FIELD,
    kind: "unknown-object",
    header: { ...FITS_FIELD.header, object: "Comète C/2026 T1" },
  });
  assert.equal(
    unknown.heading,
    "Le catalogue ne connaît pas « Comète C/2026 T1 ».",
  );
  assert.deepEqual(unknown.create, {
    name: "Comète C/2026 T1",
    label: "",
    ra: "20.9333",
    dec: "44.3333",
  });
  const hesitates = fitsView(fr, "fr", {
    ...FITS_FIELD,
    kind: "hesitates",
    header: { ...FITS_FIELD.header, object: "M31" },
    named: CARD.m31,
    candidates: [{ target: CARD.ngc7000, distanceDegrees: 0.48 }],
  });
  assert.deepEqual(
    hesitates.cards.map((c) => c.designation),
    ["M31", "NGC 7000"],
  );
  assert.match(plain(hesitates.heading), /^L’en-tête se contredit/);
});

test("« Nouvelle cible »: a name, one of the seven labels, both coordinates or none", () => {
  assert.equal(PERSONAL_LABELS.length, 7);
  assert.deepEqual(draftToCreate(EMPTY_DRAFT), { ok: false, missing: "name" });
  assert.deepEqual(draftToCreate({ ...EMPTY_DRAFT, name: "Comète" }), {
    ok: false,
    missing: "label",
  });
  assert.deepEqual(
    draftToCreate({
      ...EMPTY_DRAFT,
      name: " Comète C/2026 T1 ",
      label: "Grand champ",
    }),
    {
      ok: true,
      target: {
        name: "Comète C/2026 T1",
        label: "Grand champ",
        raHours: null,
        decDegrees: null,
      },
    },
  );
  assert.deepEqual(
    draftToCreate({
      name: "X",
      label: "Galaxie",
      ra: "20 56 00",
      dec: "+44 20 00",
    }),
    {
      ok: true,
      target: {
        name: "X",
        label: "Galaxie",
        raHours: 20 + 56 / 60,
        decDegrees: 44 + 20 / 60,
      },
    },
  );
  assert.deepEqual(
    draftToCreate({ name: "X", label: "Amas", ra: "20,9", dec: "" }),
    {
      ok: false,
      missing: "coordinates",
    },
  );
  assert.equal(parseRightAscension("20,93"), 20.93);
  assert.ok(Number.isNaN(parseRightAscension("25")));
  assert.equal(parseDeclination("−5 23"), -(5 + 23 / 60));
  assert.ok(Number.isNaN(parseDeclination("91")));
  assert.deepEqual(draftFromGuess(FITS_MUTE), EMPTY_DRAFT);
  // Every label has its words in the ten dictionaries (French here).
  assert.deepEqual(
    PERSONAL_LABELS.map((label) => fr(label.key)),
    [
      "Galaxie",
      "Nébuleuse",
      "Amas",
      "Lune",
      "Planète",
      "Soleil",
      "Grand champ",
    ],
  );
});

test("the button that reads a FITS says what it does, in the agreed words", () => {
  assert.equal(
    fr("wizard.targets.fits.choose"),
    "Trouver la cible avec un FITS",
  );
  assert.equal(
    plain(fr("wizard.targets.framing.open")),
    "Créer un cadrage sur le web",
  );
});

test("the step's stylesheet never hardcodes a colour, only var(--as-*) references", async () => {
  const { readFile } = await import("node:fs/promises");
  const css = await readFile(
    new URL("../theme/targets.css", import.meta.url),
    "utf8",
  );
  assert.doesNotMatch(css, /#[0-9a-f]{3,8}\b/i);
  assert.doesNotMatch(css, /\b(?:rgb|hsl)a?\(/i);
});
