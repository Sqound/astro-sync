// The image a device already stacked, offered as the master of a target: what « Fichiers »
// says of it, what « Traitements » lets the person choose for each family of
// poses, what the engine is asked to render and what « Récapitulatif » says.
// Pure TypeScript, tested under `node --test`; the screens only draw it
// (`screens/night/DeviceMaster.tsx`).
//
// The engine decides what is covered and what is taken by default
// (`device_master.rs`): an image that is alone in its family
// covers it and is taken; a family cut into two images or more restacks its
// poses when they are on the computer, and otherwise takes the biggest image
// and says the poses it leaves aside. Astro Sync never combines two device
// images. The person only overrides: that is `DeviceChoices`.

import type { TranslateParams } from "../i18n/locale.ts";
import type {
  DeviceFamily,
  DeviceImage,
  NightDetection,
  TargetDetection,
} from "./bridge";
import { formatDuration } from "./duration.ts";
import type { FileLine, Seg } from "./wizardFiles.ts";
import {
  conjunction,
  formatMinutes,
  formatSize,
  pluralText,
} from "./written.ts";

type Translate = (key: string, params?: TranslateParams) => string;

/** Where a family's image comes from: the device's, or a new stacking. */
export type DeviceSource = "device" | "restack";

/** What the person chose, by target then family label; a family not named
 * takes the engine's default. */
export type DeviceChoices = Readonly<
  Record<string, Readonly<Record<string, DeviceSource>>>
>;

export const NO_DEVICE_CHOICES: DeviceChoices = {};

/** The device families of a target, the biggest first (as the engine ordered
 * them). */
export function deviceFamiliesOf(
  target: Pick<TargetDetection, "device"> | null | undefined,
): readonly DeviceFamily[] {
  return target?.device?.families ?? [];
}

/** The target has an image of a known device to take. */
export function hasDeviceImage(
  target: Pick<TargetDetection, "device"> | null | undefined,
): boolean {
  return (target?.device?.images.length ?? 0) > 0;
}

/** The source a family takes: the person's choice when it can be honoured
 * (the device choice needs an image to take, the restack the poses), else the
 * engine's default. */
export function sourceOf(
  family: DeviceFamily,
  chosen: DeviceSource | undefined,
): DeviceSource {
  if (chosen === "device" && family.useFile !== null) return "device";
  if (chosen === "restack" && family.lights > 0) return "restack";
  return family.defaultSource;
}

/** The source each family of a target takes, in the families' order. */
export function sourcesOf(
  target: Pick<TargetDetection, "device" | "targetId">,
  choices: DeviceChoices,
): { family: DeviceFamily; source: DeviceSource }[] {
  const mine = choices[target.targetId] ?? {};
  return deviceFamiliesOf(target).map((family) => ({
    family,
    source: sourceOf(family, mine[family.label]),
  }));
}

/** The files the stacking is asked to take as the image of their family
 * (`stacking_run`'s `device`). Empty: every family is stacked from its poses. */
export function deviceUses(
  target: Pick<TargetDetection, "device" | "targetId">,
  choices: DeviceChoices,
): { file: string }[] {
  return sourcesOf(target, choices)
    .filter(({ family, source }) => source === "device" && family.useFile)
    .map(({ family }) => ({ file: family.useFile as string }));
}

/** The image the measure of the target is made on: the one the first family
 * that takes a device image takes. */
export function measuredImage(
  target: Pick<TargetDetection, "device" | "targetId">,
  choices: DeviceChoices,
): DeviceImage | null {
  const used = deviceUses(target, choices)[0];
  return (
    target.device?.images.find((image) => image.fileName === used?.file) ?? null
  );
}

/** The poses of the target are stacked again (the measure of the poses and
 * the poses flagged have a meaning): some pose does not belong to a family
 * that takes the device's image. A target with no device image is stacked
 * from its poses, as it always was. */
export function restacksPoses(
  target: Pick<TargetDetection, "device" | "targetId" | "lightFrames">,
  choices: DeviceChoices,
): boolean {
  if (!hasDeviceImage(target)) return true;
  const covered = sourcesOf(target, choices)
    .filter(({ source }) => source === "device")
    .reduce((sum, { family }) => sum + family.lights, 0);
  return covered < target.lightFrames;
}

/** `with` the choice of one family changed (the others kept). */
export function withDeviceChoice(
  choices: DeviceChoices | undefined,
  targetId: string,
  label: string,
  source: DeviceSource,
): DeviceChoices {
  const all = choices ?? NO_DEVICE_CHOICES;
  return { ...all, [targetId]: { ...(all[targetId] ?? {}), [label]: source } };
}

/* --------------------------- Kept on this computer ------------------------ */

/** What `localStorage` offers (the tests give their own). */
export interface DeviceStorage {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
}

const storageKey = (sessionId: string) => `astro-sync.deviceUse:${sessionId}`;

/** The sources the person chose for the families of a night, kept on this
 * computer like the pipelines; anything that cannot be read is a default. */
export function loadDeviceUse(
  storage: DeviceStorage | null,
  sessionId: string,
): DeviceChoices {
  try {
    const text = storage?.getItem(storageKey(sessionId));
    const parsed = text ? (JSON.parse(text) as unknown) : null;
    if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) {
      return NO_DEVICE_CHOICES;
    }
    const out: Record<string, Record<string, DeviceSource>> = {};
    for (const [targetId, families] of Object.entries(parsed)) {
      if (!families || typeof families !== "object") continue;
      for (const [label, source] of Object.entries(families)) {
        if (source === "device" || source === "restack") {
          (out[targetId] ??= {})[label] = source;
        }
      }
    }
    return out;
  } catch {
    return NO_DEVICE_CHOICES;
  }
}

export function saveDeviceUse(
  storage: DeviceStorage | null,
  sessionId: string,
  choices: DeviceChoices,
): void {
  try {
    storage?.setItem(storageKey(sessionId), JSON.stringify(choices));
  } catch {
    // A choice lost only brings the defaults back.
  }
}

/* ------------------------------- The cost -------------------------------- */

/** About what a stacking of the poses takes: measured on the night of
 * 10/10/2026, 430 poses in 10 min 26 s on a Mac. */
export const SECONDS_PER_POSE = 626 / 430;

/** What the scratch folder asks per pose (`stacking::BYTES_PER_LIGHT`). */
export const BYTES_PER_POSE = 210_000_000;

export function restackCost(poses: number): { minutes: number; bytes: number } {
  return {
    minutes: Math.max(1, Math.round((poses * SECONDS_PER_POSE) / 60)),
    bytes: poses * BYTES_PER_POSE,
  };
}

/* ------------------------------ The wording ------------------------------ */

function seconds(t: Translate, locale: string, value: number): string {
  return t("common.secondsShort", {
    count: new Intl.NumberFormat(locale, { maximumFractionDigits: 1 }).format(
      value,
    ),
  });
}

/** « 4 min 40 s », « 3 h 35 min »: an exposure in all. */
function total(t: Translate, value: number): string {
  return formatDuration(t, value * 1000);
}

function posesText(t: Translate, locale: string, count: number): string {
  return pluralText(t, locale, "wizard.files.frames", count);
}

/** « 430 poses de 30 s ». */
function posesOf(
  t: Translate,
  locale: string,
  count: number,
  exposure: number | null,
): string {
  return exposure === null
    ? posesText(t, locale, count)
    : t("deviceMaster.posesOf", {
        poses: posesText(t, locale, count),
        exposure: seconds(t, locale, exposure),
      });
}

/** What names a family beside the others of its target: the exposure, and the
 * filter or the gain when the exposures alone would not tell them apart. */
function familyName(
  t: Translate,
  locale: string,
  family: DeviceFamily,
  all: readonly DeviceFamily[],
): string {
  if (family.exposureSeconds === null)
    return t("deviceMaster.familyNoExposure");
  const exposure = seconds(t, locale, family.exposureSeconds);
  const same = all.filter(
    (other) =>
      other.exposureSeconds === family.exposureSeconds &&
      other.label !== family.label,
  );
  const extra: string[] = [];
  if (same.some((other) => other.filter !== family.filter) && family.filter) {
    extra.push(family.filter);
  }
  if (
    same.some((other) => other.gain !== family.gain) &&
    family.gain !== null
  ) {
    extra.push(t("wizard.files.gain", { gain: String(family.gain) }));
  }
  return extra.length > 0
    ? t("deviceMaster.familyDetailed", { exposure, detail: extra.join(", ") })
    : t("deviceMaster.family", { exposure });
}

function imagePoses(family: DeviceFamily, images: readonly DeviceImage[]) {
  return images
    .filter((image) => family.images.includes(image.fileName))
    .reduce((sum, image) => sum + image.poses, 0);
}

/** The poses a family has, here or in its images. */
function familyPoses(family: DeviceFamily, images: readonly DeviceImage[]) {
  return family.lights > 0 ? family.lights : imagePoses(family, images);
}

function counts(
  t: Translate,
  locale: string,
  family: DeviceFamily,
  images: readonly DeviceImage[],
): string {
  const numbers = family.images
    .map((name) => images.find((image) => image.fileName === name)?.poses ?? 0)
    .map((poses) => new Intl.NumberFormat(locale).format(poses));
  return t("deviceMaster.counts", { list: conjunction(locale, numbers) });
}

/* ------------------------------ « Fichiers » ------------------------------ */

/** A group of the card: a family and the images that meet it. */
export interface DeviceBoxGroup {
  heading: string | null;
  /** « Une image la couvre », « Aucune image ne la couvre ». */
  status: { tone: "success" | "warning"; text: string } | null;
  lines: FileLine[];
  /** The sentence under the images (the split family). */
  note: string | null;
}

/** The card « Image empilée par l'appareil » under « Poses FITS »:
 * no button, it informs; the choice is at « Traitements ». */
export interface DeviceBox {
  state: { tone: "success" | "warning"; text: string };
  groups: DeviceBoxGroup[];
  /** Plain sentences after the groups. */
  notes: FileLine[];
}

/** The card of a target, or `null` when its `device/` holds nothing. */
export function deviceBox(
  t: Translate,
  locale: string,
  target: Pick<TargetDetection, "device" | "lightFrames" | "folderName">,
): DeviceBox | null {
  const device = target.device;
  if (!device) return null;
  const images = device.images;
  const families = device.families;
  const several = families.length > 1;
  const groups: DeviceBoxGroup[] = [];
  const notes: FileLine[] = [];
  const pathOf = (image: DeviceImage) => `device/${image.fileName}`;
  const lone = images.length === 1;
  for (const family of families) {
    const mine = family.images
      .map((name) => images.find((image) => image.fileName === name))
      .filter((image): image is DeviceImage => image !== undefined);
    const lines: FileLine[] = [];
    for (const image of mine) {
      lines.push({
        glyph: "ok",
        segs: [
          { code: pathOf(image) },
          ` · ${formatSize(t, image.bytes, locale)}`,
        ],
      });
      lines.push({
        glyph: "none",
        segs: [
          lone
            ? t("deviceMaster.detail", {
                device: image.device,
                poses: posesOf(t, locale, image.poses, image.exposureSeconds),
                total:
                  image.totalSeconds === null
                    ? "—"
                    : total(t, image.totalSeconds),
                format: t(
                  image.solved
                    ? "deviceMaster.formatSolved"
                    : "deviceMaster.format",
                ),
              })
            : t("deviceMaster.detailShort", {
                poses: posesText(t, locale, image.poses),
                total:
                  image.totalSeconds === null
                    ? "—"
                    : total(t, image.totalSeconds),
              }),
        ],
      });
    }
    let note: string | null = null;
    if (family.coverage === "split") {
      const list = counts(t, locale, family, images);
      note =
        family.lights > 0
          ? t("deviceMaster.splitWhy", {
              images: family.images.length,
              counts: list,
              total: posesText(t, locale, family.lights),
            })
          : t("deviceMaster.splitBiggest", {
              images: family.images.length,
              counts: list,
              aside: posesText(t, locale, family.asidePoses),
            });
    }
    groups.push({
      heading: several
        ? t("deviceMaster.familyHeading", {
            name: familyName(t, locale, family, families),
            poses: posesText(t, locale, familyPoses(family, images)),
          })
        : null,
      status: several
        ? family.coverage === "covers"
          ? { tone: "success", text: t("deviceMaster.covers") }
          : { tone: "warning", text: t("deviceMaster.split") }
        : null,
      lines,
      note,
    });
  }
  if (images.length === 1) {
    notes.push({ glyph: "none", segs: [t("deviceMaster.line")] });
  }
  if (several) {
    notes.push({
      glyph: "none",
      segs: [
        t("deviceMaster.familiesApart", {
          families: conjunction(
            locale,
            families
              .filter((family) => family.exposureSeconds !== null)
              .map((family) =>
                seconds(t, locale, family.exposureSeconds as number),
              ),
          ),
        }),
      ],
    });
  }
  if (images.length > 0) {
    notes.push({ glyph: "none", segs: [t("deviceMaster.jpgNever")] });
  }
  // A .jpg with no .fit of its name: stretched, never a master.
  // With poses, the stacking is what it was; without anything else,
  // the card points to the final image.
  for (const jpg of device.jpgs) {
    const name = jpg.split("/").pop() ?? jpg;
    notes.push({ glyph: "warning", segs: [{ code: `device/${name}` }] });
  }
  if (device.jpgs.length > 0) {
    notes.push({
      glyph: "none",
      segs: [
        t(
          target.lightFrames > 0
            ? "deviceMaster.jpg.withPoses"
            : "deviceMaster.jpg.alone",
        ),
      ],
    });
    notes.push({
      glyph: "none",
      segs: [
        t(
          target.lightFrames > 0
            ? "deviceMaster.jpg.copyFit"
            : "deviceMaster.jpg.final",
        ),
      ],
    });
  }
  const found = images.length;
  return {
    state:
      found > 0
        ? {
            tone: "success",
            text: pluralText(t, locale, "deviceMaster.found", found),
          }
        : { tone: "warning", text: t("deviceMaster.notMaster") },
    groups,
    notes,
  };
}

/* ----------------------------- « Traitements » ---------------------------- */

/** One side of the choice of a family. */
export interface DeviceSide {
  /** The side can be taken. */
  available: boolean;
  /** « Par défaut ». */
  isDefault: boolean;
  segs: Seg[];
  /** Why it cannot be taken, in place of a greyed button. */
  why: string | null;
}

/** The choice of one family: the image of the device, or a new stacking. */
export interface DeviceFamilyChoice {
  label: string;
  /** « Famille de 30 s · 430 poses », when the target has several. */
  heading: string | null;
  source: DeviceSource;
  use: DeviceSide;
  restack: DeviceSide;
  /** The file the device side takes, and where the engine finds it. */
  useFile: string | null;
}

/** The reasons to stack again, with what the night says. */
export interface DeviceWhy {
  cases: { title: string; text: string }[];
  table: {
    head: [string, string];
    /** A row of three cells; a `null` third one means the second spans both
     * columns (the signal to noise ratio, equal on both sides). */
    rows: [string, string, string | null][];
  };
  caption: string;
  footnote: string;
}

/** What « Traitements » shows in the card of a target that has an image of a
 * device. */
export interface DeviceChoiceView {
  families: DeviceFamilyChoice[];
  /** The device's name: « Seestar S50 Pro ». */
  device: string;
  /** « Les pipelines choisis partent d'elle comme … ». */
  pipelinesNote: string | null;
  /** « Sans objet pour l'image de l'appareil … » while no family restacks. */
  unused: string | null;
  /** The image the measure of the target is made on. */
  measure: { fileName: string; path: string } | null;
  why: DeviceWhy;
}

/** The static example of the comparison: measured by Astro Sync on
 * a real night, M 45 on 10-11 October 2026, 430 poses of 30 s. */
export const EXAMPLE = {
  device: { poses: 430, fwhm: 4.14, eccentricity: 0.45, stars: 382 },
  here: { poses: 429, fwhm: 3.65, eccentricity: 0.41, stars: 371 },
  seconds: 626,
  bytes: 430 * BYTES_PER_POSE,
  finer: 12,
} as const;

function sideLabel(
  t: Translate,
  locale: string,
  family: DeviceFamily,
  images: readonly DeviceImage[],
): DeviceSide {
  const image = images.find((item) => item.fileName === family.useFile);
  const isDefault = family.defaultSource === "device";
  if (!image) {
    return {
      available: false,
      isDefault: false,
      segs: [],
      why:
        family.lights > 0
          ? t("deviceMaster.useWhy", {
              counts: counts(t, locale, family, images),
              total: posesText(t, locale, family.lights),
              images: family.images.length,
            })
          : null,
    };
  }
  return {
    available: true,
    isDefault,
    segs: [
      { code: image.fileName },
      ` · ${posesText(t, locale, image.poses)}`,
      ` · ${t("deviceMaster.ready")}`,
    ],
    why: null,
  };
}

function restackSide(
  t: Translate,
  locale: string,
  family: DeviceFamily,
  firstOfCard: boolean,
): DeviceSide {
  if (family.lights === 0) {
    return {
      available: false,
      isDefault: false,
      segs: [],
      why: t("deviceMaster.noPoses"),
    };
  }
  const cost = restackCost(family.lights);
  const defaultSource = family.defaultSource === "restack";
  const detail = t("deviceMaster.restackDetail", {
    poses: posesOf(t, locale, family.lights, family.exposureSeconds),
    duration: formatMinutes(t, cost.minutes),
    space: formatSize(t, cost.bytes, locale),
  });
  return {
    available: true,
    isDefault: defaultSource,
    segs: [
      detail,
      ...(firstOfCard && !defaultSource
        ? [` ${t("deviceMaster.restackUseful")}`]
        : []),
    ],
    why: null,
  };
}

/** What « Pourquoi empiler à nouveau ? » says: four cases, then the comparison
 * measured on a real night. */
export function deviceWhy(
  t: Translate,
  locale: string,
  input: {
    families: readonly DeviceFamily[];
    images: readonly DeviceImage[];
    /** The short date of the night (« 10 oct. »), when known. */
    day: string | null;
  },
): DeviceWhy {
  const split = input.families.find(
    (family) => family.coverage === "split" && family.lights > 0,
  );
  const number = (value: number, digits: number) =>
    new Intl.NumberFormat(locale, {
      minimumFractionDigits: digits,
      maximumFractionDigits: digits,
    }).format(value);
  const px = (value: number) =>
    t("deviceMaster.why.px", { value: number(value, 2) });
  const cost = restackCost(EXAMPLE.device.poses);
  return {
    cases: [
      {
        title: t("deviceMaster.why.nights.title"),
        text: t("deviceMaster.why.nights.text"),
      },
      {
        title: t("deviceMaster.why.cut.title"),
        text:
          split && input.day
            ? t("deviceMaster.why.cut.night", {
                day: input.day,
                poses: posesOf(t, locale, split.lights, split.exposureSeconds),
                images: split.images.length,
                counts: counts(t, locale, split, input.images),
                total: posesText(t, locale, split.lights),
              })
            : t("deviceMaster.why.cut.text"),
      },
      {
        title: t("deviceMaster.why.missed.title"),
        text: t("deviceMaster.why.missed.text"),
      },
      {
        title: t("deviceMaster.why.finer.title"),
        text: t("deviceMaster.why.finer.text", { percent: EXAMPLE.finer }),
      },
    ],
    table: {
      head: [t("deviceMaster.why.device"), t("deviceMaster.why.here")],
      rows: [
        [
          t("deviceMaster.why.rows.poses"),
          String(EXAMPLE.device.poses),
          t("deviceMaster.why.rows.posesHere", { count: EXAMPLE.here.poses }),
        ],
        [
          t("deviceMaster.why.rows.fwhm"),
          px(EXAMPLE.device.fwhm),
          t("deviceMaster.why.rows.fwhmHere", {
            value: px(EXAMPLE.here.fwhm),
            percent: EXAMPLE.finer,
          }),
        ],
        [
          t("deviceMaster.why.rows.stars"),
          `${number(EXAMPLE.device.eccentricity, 2)} · ${EXAMPLE.device.stars}`,
          `${number(EXAMPLE.here.eccentricity, 2)} · ${EXAMPLE.here.stars}`,
        ],
        [
          t("deviceMaster.why.rows.snr"),
          t("deviceMaster.why.rows.snrBoth"),
          null,
        ],
        [
          t("deviceMaster.why.rows.time"),
          t("deviceMaster.why.rows.none"),
          formatDuration(t, EXAMPLE.seconds * 1000),
        ],
        [
          t("deviceMaster.why.rows.room"),
          t("deviceMaster.why.rows.noneF"),
          t("deviceMaster.why.rows.roomHere", {
            space: formatSize(t, cost.bytes, locale),
          }),
        ],
      ],
    },
    caption: t("deviceMaster.why.caption"),
    footnote: t("deviceMaster.why.footnote"),
  };
}

/** The choice view of a target, or `null` when it has no image of a device. */
export function deviceChoiceView(
  t: Translate,
  locale: string,
  input: {
    target: Pick<
      TargetDetection,
      "device" | "targetId" | "designation" | "lightFrames"
    >;
    choices: DeviceChoices;
    detection: Pick<NightDetection, "folder"> | null;
    day: string | null;
  },
): DeviceChoiceView | null {
  const { target } = input;
  const device = target.device;
  if (!device || device.images.length === 0) return null;
  const families = device.families;
  const several = families.length > 1;
  const mine = input.choices[target.targetId] ?? {};
  const rows: DeviceFamilyChoice[] = families.map((family, index) => ({
    label: family.label,
    heading: several
      ? t("deviceMaster.familyHeading", {
          name: familyName(t, locale, family, families),
          poses: posesText(t, locale, familyPoses(family, device.images)),
        })
      : null,
    source: sourceOf(family, mine[family.label]),
    use: sideLabel(t, locale, family, device.images),
    restack: restackSide(t, locale, family, index === 0),
    useFile: family.useFile,
  }));
  const choices = {
    targetId: target.targetId,
    device,
  };
  const measured = measuredImage(choices, input.choices);
  const stacksPoses = restacksPoses(
    { ...choices, lightFrames: target.lightFrames },
    input.choices,
  );
  const usesDevice = rows.some((row) => row.source === "device");
  return {
    families: rows,
    device: device.images[0].device,
    pipelinesNote: usesDevice ? t("deviceMaster.pipelinesNote") : null,
    unused: stacksPoses ? null : t("deviceMaster.unused"),
    measure:
      measured && input.detection
        ? {
            fileName: measured.fileName,
            path: `${input.detection.folder}/${measured.relativePath}`,
          }
        : null,
    why: deviceWhy(t, locale, {
      families,
      images: device.images,
      day: input.day,
    }),
  };
}

/* ----------------------------- « Récapitulatif » -------------------------- */

/** The sentence under the stacked image of a target that took the device's:
 * « Faite ici à partir de l'image empilée par le Seestar (430 poses de 30 s) ». */
export function deviceMadeText(
  t: Translate,
  locale: string,
  image: DeviceImage,
): string {
  return t("deviceMaster.summary.made", {
    device: image.device,
    poses: posesOf(t, locale, image.poses, image.exposureSeconds),
  });
}

/** The images of a target the renders start from, the largest family first. */
export function usedDeviceImages(
  target: Pick<TargetDetection, "device" | "targetId">,
  choices: DeviceChoices,
): DeviceImage[] {
  const used = new Set(deviceUses(target, choices).map((use) => use.file));
  return (target.device?.images ?? []).filter((image) =>
    used.has(image.fileName),
  );
}
