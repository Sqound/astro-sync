// The step « Cibles »: what the screen shows of a search, of a card, of
// a FITS read, and the checks of the « Nouvelle cible » form. Pure, so the
// tests read the words the screen shows.

import type { TranslateParams } from "../i18n/locale.ts";
import type {
  CatalogueTarget,
  FitsGuess,
  PersonalTargetCreate,
} from "./bridge.ts";
import { oneDecimal, pluralText } from "./written.ts";

type Translate = (key: string, params?: TranslateParams) => string;

/** The pause after the last key before the catalogue is asked (ms). */
export const SEARCH_DELAY_MS = 250;
/** Cards per page (« huit cartes au plus »). */
export const SEARCH_PAGE = 8;
/** « Afficher plus de cibles » stops here. */
export const SEARCH_MAXIMUM = 24;

/** The seven labels of a personal target (the service's keys, the website's
 * form), with the key of their words. */
export const PERSONAL_LABELS: { value: string; key: string }[] = [
  { value: "Galaxie", key: "wizard.targets.label.galaxy" },
  { value: "Nébuleuse", key: "wizard.targets.label.nebula" },
  { value: "Amas", key: "wizard.targets.label.cluster" },
  { value: "Lune", key: "wizard.targets.label.moon" },
  { value: "Planète", key: "wizard.targets.label.planet" },
  { value: "Soleil", key: "wizard.targets.label.sun" },
  { value: "Grand champ", key: "wizard.targets.label.wideField" },
];

/** Two characters at least before the catalogue is asked. */
export function searchReady(text: string): boolean {
  return text.trim().length >= 2;
}

/** A failure that means « no service » (the line « Hors ligne », the gestures
 * that add greyed), as opposed to a refusal said in words. */
export function isOfflineFailure(error: unknown): boolean {
  const code =
    typeof error === "object" && error !== null && "code" in error
      ? String((error as { code: unknown }).code)
      : "";
  return [
    "network_error",
    "service_start_timeout",
    "service_starting",
  ].includes(code);
}

/** The address of a card's preview: a path of the service joined to its
 * address; an absolute address as it is; none when there is no picture. */
export function imageAddress(
  serviceUrl: string | null | undefined,
  path: string | null,
): string | null {
  if (!path) return null;
  if (/^(https?:|data:)/.test(path)) return path;
  if (!serviceUrl) return null;
  return `${serviceUrl.replace(/\/$/, "")}${path.startsWith("/") ? "" : "/"}${path}`;
}

/** What a card writes: the designation, the
 * common name, « type · constellation », « magnitude 4,0 · 120 × 100′ ». */
export function cardLines(
  t: Translate,
  locale: string,
  target: CatalogueTarget,
): { designation: string; name: string | null; kind: string; details: string } {
  return {
    designation: target.designation,
    name: target.name,
    kind: [target.typeLabel, target.constellationLabel]
      .filter(Boolean)
      .join(" · "),
    details: [
      typeof target.magnitude === "number"
        ? t("wizard.targets.card.magnitude", {
            value: oneDecimal(target.magnitude, locale),
          })
        : null,
      target.sizeLabel,
    ]
      .filter(Boolean)
      .join(" · "),
  };
}

/** « 8 résultats sur 23 ». */
export function counterText(
  t: Translate,
  locale: string,
  shown: number,
  total: number,
): string {
  return pluralText(t, locale, "wizard.targets.search.counter", shown, {
    total,
  });
}

/** What follows the cards: « Afficher plus de cibles », « Affinez la
 * recherche » at the limit, or nothing. */
export function moreAction(
  shown: number,
  total: number,
): "more" | "refine" | null {
  if (shown >= total) return null;
  return shown >= SEARCH_MAXIMUM ? "refine" : "more";
}

/** A position as it is written: « 0 h 43 min · +41° 16′ ». */
export function formatPosition(raDegrees: number, decDegrees: number): string {
  const totalMinutes =
    Math.round(((((raDegrees / 15) % 24) + 24) % 24) * 60) % (24 * 60);
  const hours = Math.floor(totalMinutes / 60);
  const minutes = totalMinutes % 60;
  const sign = decDegrees < 0 ? "−" : "+";
  const arcminutes = Math.round(Math.abs(decDegrees) * 60);
  const degrees = Math.floor(arcminutes / 60);
  return `${hours} h ${String(minutes).padStart(2, "0")} min · ${sign}${degrees}° ${String(arcminutes % 60).padStart(2, "0")}′`;
}

/** A field as it is written: « 2,1° × 1,6° ». */
export function formatField(field: [number, number], locale: string): string {
  return `${oneDecimal(field[0], locale)}° × ${oneDecimal(field[1], locale)}°`;
}

/** What the block of a FITS read shows. */
export interface FitsView {
  /** The sentence above the cards. */
  heading: string;
  /** The proof under the cards, in small (« L'en-tête dit … »). */
  proof: string | null;
  cards: CatalogueTarget[];
  /** « Créer « … » comme nouvelle cible », with the form filled. */
  create: PersonalTargetDraft | null;
  /** The neutral notice of a file that says nothing. */
  mute: boolean;
}

export function fitsView(
  t: Translate,
  locale: string,
  guess: FitsGuess,
): FitsView {
  const header = guess.header;
  const position =
    header.raDegrees !== null && header.decDegrees !== null
      ? formatPosition(header.raDegrees, header.decDegrees)
      : null;
  const object = header.object ?? "";
  const field = formatField(header.fieldDegrees, locale);
  const candidates = guess.candidates.map((candidate) => candidate.target);
  switch (guess.kind) {
    case "found":
      return {
        heading: t("wizard.targets.fits.found"),
        proof: position
          ? t("wizard.targets.fits.proofBoth", {
              object,
              position,
              designation: guess.named?.designation ?? object,
            })
          : t("wizard.targets.fits.proofObject", { object }),
        cards: guess.named ? [guess.named] : [],
        create: null,
        mute: false,
      };
    case "hesitates":
      return {
        heading: t("wizard.targets.fits.hesitates", {
          object,
          position: position ?? "",
        }),
        proof: null,
        cards: [...(guess.named ? [guess.named] : []), ...candidates],
        create: null,
        mute: false,
      };
    case "candidates":
      return {
        heading: candidates.length
          ? t("wizard.targets.fits.field", { field })
          : t("wizard.targets.fits.fieldEmpty", { field }),
        proof: position
          ? t("wizard.targets.fits.noObject", { position })
          : null,
        cards: candidates,
        create: candidates.length ? null : draftFromGuess(guess),
        mute: false,
      };
    case "unknown-object":
      return {
        heading: t("wizard.targets.fits.unknown", { object }),
        proof: candidates.length
          ? t("wizard.targets.fits.field", { field })
          : null,
        cards: candidates,
        create: draftFromGuess(guess),
        mute: false,
      };
    default:
      return {
        heading: t("wizard.targets.fits.nothing"),
        proof: t("wizard.targets.fits.nothingDetail"),
        cards: [],
        create: null,
        mute: true,
      };
  }
}

/* --------------------------- « Nouvelle cible » --------------------------- */

/** The form as typed. */
export interface PersonalTargetDraft {
  name: string;
  label: string;
  ra: string;
  dec: string;
}

export const EMPTY_DRAFT: PersonalTargetDraft = {
  name: "",
  label: "",
  ra: "",
  dec: "",
};

function trimmedNumber(value: number): string {
  return String(Math.round(value * 10_000) / 10_000);
}

/** The form filled from a header: its `OBJECT` as the name, its position. */
export function draftFromGuess(guess: FitsGuess): PersonalTargetDraft {
  const header = guess.header;
  return {
    name: header.object ?? "",
    label: "",
    ra: header.raDegrees !== null ? trimmedNumber(header.raDegrees / 15) : "",
    dec: header.decDegrees !== null ? trimmedNumber(header.decDegrees) : "",
  };
}

function sexagesimal(text: string): number | null {
  const cleaned = text.trim().replace(",", ".");
  if (cleaned === "") return null;
  const negative = cleaned.startsWith("-") || cleaned.startsWith("−");
  const parts = cleaned
    .replace(/^[+\-−]/, "")
    .split(/[\s:hmsd°′″'"]+/u)
    .filter(Boolean);
  if (parts.length === 0 || parts.length > 3) return null;
  const numbers = parts.map(Number);
  if (numbers.some((n) => !Number.isFinite(n) || n < 0)) return null;
  if (parts.length > 1 && numbers.slice(1).some((n) => n >= 60)) return null;
  const value = numbers[0] + (numbers[1] ?? 0) / 60 + (numbers[2] ?? 0) / 3600;
  return negative ? -value : value;
}

/** Right ascension typed in hours (« 20,93 », « 20 56 00 »): hours in
 * [0, 24), `null` when empty, `NaN` when unreadable. */
export function parseRightAscension(text: string): number | null {
  if (text.trim() === "") return null;
  const hours = sexagesimal(text);
  return hours !== null && hours >= 0 && hours < 24 ? hours : Number.NaN;
}

/** Declination typed in degrees (« 44,3 », « +44 20 00 »). */
export function parseDeclination(text: string): number | null {
  if (text.trim() === "") return null;
  const degrees = sexagesimal(text);
  return degrees !== null && degrees >= -90 && degrees <= 90
    ? degrees
    : Number.NaN;
}

/** What the form sends, or why it cannot yet (`name`, `label`,
 * `coordinates`: both or none, each readable). */
export function draftToCreate(
  draft: PersonalTargetDraft,
):
  | { ok: true; target: PersonalTargetCreate }
  | { ok: false; missing: "name" | "label" | "coordinates" } {
  const name = draft.name.trim();
  if (name === "" || [...name].length > 80)
    return { ok: false, missing: "name" };
  if (!PERSONAL_LABELS.some((label) => label.value === draft.label))
    return { ok: false, missing: "label" };
  const ra = parseRightAscension(draft.ra);
  const dec = parseDeclination(draft.dec);
  const bothOrNone = (ra === null) === (dec === null);
  if (!bothOrNone || Number.isNaN(ra) || Number.isNaN(dec))
    return { ok: false, missing: "coordinates" };
  return {
    ok: true,
    target: { name, label: draft.label, raHours: ra, decDegrees: dec },
  };
}
