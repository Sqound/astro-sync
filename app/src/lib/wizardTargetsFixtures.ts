// The invented data of the step « Cibles »: the catalogue cards of a search
// « nébuleuse », the headers of three FITS read, a night with no target. One
// source for the tests of the step's words (`targetSearch.test.ts`) and for the
// scenarios of the captures. The names,
// types and constellations are the sky's; the previews are drawn (an inline
// SVG), never a real image.

import type { CatalogueTarget, FitsGuess, TargetSearch } from "./bridge";

/** A drawn preview: a soft glow on the night's blue, tinted by kind. */
function drawn(tint: string): string {
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 56 56"><defs><radialGradient id="g"><stop offset="0" stop-color="${tint}" stop-opacity=".9"/><stop offset=".6" stop-color="${tint}" stop-opacity=".25"/><stop offset="1" stop-color="#0b1626" stop-opacity="0"/></radialGradient></defs><rect width="56" height="56" fill="#0b1626"/><circle cx="28" cy="28" r="24" fill="url(#g)"/><circle cx="12" cy="14" r=".8" fill="#fff"/><circle cx="44" cy="40" r=".7" fill="#fff"/><circle cx="40" cy="10" r=".5" fill="#fff"/></svg>`;
  return `data:image/svg+xml,${encodeURIComponent(svg)}`;
}

const RED = "#d9566b";
const TEAL = "#59b3a8";
const GOLD = "#d8b46a";

function target(
  id: string,
  designation: string,
  name: string,
  typeLabel: string,
  constellationLabel: string,
  raHours: number,
  decDegrees: number,
  magnitude: number,
  sizeLabel: string,
  sizeArcmin: number,
  tint: string,
): CatalogueTarget {
  return {
    id,
    designation,
    name,
    aliases: [],
    typeLabel,
    constellationLabel,
    raHours,
    decDegrees,
    magnitude,
    sizeLabel,
    sizeArcmin,
    imagePath: drawn(tint),
  };
}

export const CARD = {
  ngc7000: target(
    "t-ngc7000",
    "NGC 7000",
    "Nébuleuse de l’Amérique du Nord",
    "Nébuleuse en émission",
    "Cygne",
    20.9784,
    44.33,
    4.0,
    "120 × 100′",
    120,
    RED,
  ),
  ic1396: target(
    "t-ic1396",
    "IC 1396",
    "Nébuleuse de la Trompe d’Éléphant",
    "Nébuleuse en émission",
    "Céphée",
    21.65,
    57.5,
    3.5,
    "170′",
    170,
    RED,
  ),
  m42: target(
    "t-m42",
    "M42",
    "Grande nébuleuse d’Orion",
    "Nébuleuse en émission",
    "Orion",
    5.588,
    -5.39,
    4.0,
    "85 × 60′",
    85,
    RED,
  ),
  ngc6992: target(
    "t-ngc6992",
    "NGC 6992",
    "Dentelles du Cygne",
    "Rémanent de supernova",
    "Cygne",
    20.94,
    31.72,
    7.0,
    "60′",
    60,
    TEAL,
  ),
  ngc7293: target(
    "t-ngc7293",
    "NGC 7293",
    "Nébuleuse de l’Hélice",
    "Nébuleuse planétaire",
    "Verseau",
    22.494,
    -20.84,
    7.6,
    "16′",
    16,
    TEAL,
  ),
  ic5070: target(
    "t-ic5070",
    "IC 5070",
    "Nébuleuse du Pélican",
    "Nébuleuse en émission",
    "Cygne",
    20.8508,
    44.35,
    8.0,
    "60 × 50′",
    60,
    RED,
  ),
  m27: target(
    "t-m27",
    "M27",
    "Nébuleuse de l’Haltère",
    "Nébuleuse planétaire",
    "Petit Renard",
    19.993,
    22.72,
    7.5,
    "8′",
    8,
    TEAL,
  ),
  m8: target(
    "t-m8",
    "M8",
    "Nébuleuse de la Lagune",
    "Nébuleuse en émission",
    "Sagittaire",
    18.06,
    -24.38,
    6.0,
    "90 × 40′",
    90,
    RED,
  ),
  m31: target(
    "t-m31",
    "M31",
    "Galaxie d’Andromède",
    "Galaxie",
    "Andromède",
    0.7123,
    41.2689,
    3.4,
    "190 × 60′",
    190,
    GOLD,
  ),
};

/** State 2: « nébuleuse », eight cards of twenty-three. */
export const SEARCH_NEBULA: TargetSearch = {
  query: "nébuleuse",
  total: 23,
  targets: [
    CARD.ngc7000,
    CARD.ic1396,
    CARD.m42,
    CARD.ngc6992,
    CARD.ngc7293,
    CARD.ic5070,
    CARD.m27,
    CARD.m8,
  ],
};

/** State 4: `OBJECT = 'M 31'` and a position on M31. */
export const FITS_FOUND: FitsGuess = {
  fileName: "M31_Light_120s_0042.fit",
  header: {
    object: "M 31",
    raDegrees: 10.6847,
    decDegrees: 41.2688,
    positionSource: "objctra",
    fieldDegrees: [2.1, 1.6],
    fieldFromHeader: true,
  },
  kind: "found",
  named: CARD.m31,
  candidates: [],
};

/** State 4 bis: coordinates only, NGC 7000 and IC 5070 in the field. */
export const FITS_FIELD: FitsGuess = {
  fileName: "Lights_0007.fit",
  header: {
    object: null,
    raDegrees: 314.0,
    decDegrees: 44.3333,
    positionSource: "objctra",
    fieldDegrees: [2.1, 1.6],
    fieldFromHeader: true,
  },
  kind: "candidates",
  named: null,
  candidates: [
    { target: CARD.ngc7000, distanceDegrees: 0.48 },
    { target: CARD.ic5070, distanceDegrees: 0.89 },
  ],
};

/** State 5: a file that says nothing. */
export const FITS_MUTE: FitsGuess = {
  fileName: "Lights_0001.fit",
  header: {
    object: null,
    raDegrees: null,
    decDegrees: null,
    positionSource: null,
    fieldDegrees: [1, 1],
    fieldFromHeader: false,
  },
  kind: "nothing",
  named: null,
  candidates: [],
};
