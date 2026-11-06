// The wizard's step « Récapitulatif », the sending and « Nuit envoyée »: what
// will be sent (results only, each image with its SHA-256), what is kept
// online, what never leaves this computer, the check of the four limits before
// anything is sent, and the resume. Pure TypeScript, tested under
// `node --test`.

import type { TranslateParams } from "../i18n/locale.ts";
import {
  deviceMadeText,
  NO_DEVICE_CHOICES,
  usedDeviceImages,
  type DeviceChoices,
} from "./deviceMaster.ts";
import type {
  ImagePlan,
  ImageQuality,
  NightDetection,
  NightOnline,
  NightPrecheck,
  NightResume,
  QuotaFinding,
  Treatment,
} from "./bridge";
import { calibrationFindings, stackedWithoutText } from "./missingMessages.ts";
import type { Seg } from "./wizardFiles.ts";
import { baseName } from "./wizardSteps.ts";
import { formatDuration } from "./duration.ts";
import { formatSize, pluralText, shortDigest } from "./written.ts";

type Translate = (key: string, params?: TranslateParams) => string;

export interface SummaryRow {
  segs: Seg[];
  /** The right column: a size, « calculé ici », « en ligne ». */
  value: string;
  /** The file in cause of a limit (red). */
  over?: boolean;
  /** A line under the row above it, not a row of its own (the quality of a
   * stacked image): no rule over it, no right column. */
  note?: boolean;
}

export interface SummaryGroup {
  key: "sent" | "keptOnline" | "setAside" | "neverLeaves";
  titleKey: string;
  rows: SummaryRow[];
}

export interface CheckLine {
  ok: boolean;
  text: string;
}

export interface SummaryView {
  leadKey: string;
  /** What the lead says: the short date of the session. */
  leadParams: { date: string };
  groups: SummaryGroup[];
  allowed: boolean;
  /** « Vérification avant l'envoi », when every limit is kept. */
  checks: CheckLine[];
  /** « Une analyse par nuit : … » on a resume. */
  analysisNote: boolean;
  /** « Envoi impossible : rien n'est envoyé », one line per limit. */
  refusals: string[];
  toSend: string;
  primaryKey: string;
}

/** « Qualité de l’image empilée : FWHM 2,1″ · excentricité 0,31 · 212 étoiles
 * »: the FWHM in arc seconds when the scale of the frames is known, else in
 * pixels; without an eccentricity, the sentence leaves it out. */
export function imageQualityText(
  t: Translate,
  locale: string,
  quality: ImageQuality,
  /** `master`: measured on a master the person stacked elsewhere, not on
   * a stacking of Astro Sync's own. */
  source: "stacking" | "master" | "device" = "stacking",
): string {
  const decimals = (value: number, digits: number) =>
    new Intl.NumberFormat(locale, {
      minimumFractionDigits: digits,
      maximumFractionDigits: digits,
    }).format(value);
  const fwhm =
    quality.fwhmArcsec !== null
      ? `${decimals(quality.fwhmArcsec, 1)}″`
      : `${decimals(quality.fwhmPixels, 1)}\u00a0px`;
  const key = (short: boolean) =>
    `wizard.summary.imageQuality${{ stacking: "", master: "Master", device: "Device" }[source]}${short ? "Short" : ""}`;
  const count = new Intl.NumberFormat(locale).format(quality.starCount);
  return quality.eccentricity !== null
    ? t(key(false), {
        fwhm,
        eccentricity: decimals(quality.eccentricity, 2),
        count,
      })
    : t(key(true), { fwhm, count });
}

function designation(
  detection: NightDetection | null,
  online: NightOnline | null,
  targetId: string,
): string {
  return (
    detection?.targets.find((target) => target.targetId === targetId)
      ?.designation ??
    online?.targets.find((target) => target.targetId === targetId)?.name ??
    targetId
  );
}

const SOURCE_KEY: Record<string, string> = {
  phone: "wizard.files.fromPhone",
  web: "wizard.files.fromWeb",
};

/** The words of a limit passed, said with what, how much and the limit. */
export function refusalText(
  t: Translate,
  locale: string,
  finding: QuotaFinding,
  names: (targetId: string) => string,
  privateServer: boolean,
): string {
  const size = (bytes: number) => formatSize(t, bytes, locale);
  switch (finding.limit) {
    case "file_too_large":
      return t(
        privateServer
          ? "wizard.summary.refused.fileTooLargePrivate"
          : "wizard.summary.refused.fileTooLarge",
        {
          file: baseName(finding.path),
          size: size(finding.size),
          max: size(finding.maxFileBytes),
        },
      );
    case "storage_quota":
      return t("wizard.summary.refused.storage", {
        toSend: size(finding.toSendBytes),
        remaining: size(finding.remainingBytes),
        quota: size(finding.quotaBytes),
      });
    case "variant_limit":
      return t("wizard.summary.refused.variants", {
        target: names(finding.targetId),
        existing: finding.existing,
        adding: finding.adding,
        limit: finding.variantLimit,
      });
    case "capture_count":
      return t("wizard.summary.refused.captures", {
        count: finding.captureCount,
        adding: finding.adding,
        limit: finding.captureCountLimit,
      });
    case "refused":
      return t("wizard.summary.refused.service", {
        file: baseName(finding.path),
        reason: t(`errors.${finding.code}`),
      });
  }
}

/** How many milestones the progression of `frames` light frames gives
 * (`analysis::siril::script::build_milestones` of the core): one frame,
 * then about a quarter, half, three quarters and all of them, each count
 * once. */
export function milestoneCount(frames: number): number {
  if (frames <= 0) return 0;
  const counts = [1];
  if (frames > 1) {
    for (const fraction of [0.25, 0.5, 0.75, 1]) {
      const count = Math.min(
        frames,
        Math.max(1, Math.round(frames * fraction)),
      );
      if (count > counts[counts.length - 1]) counts.push(count);
    }
    if (counts[counts.length - 1] !== frames) counts.push(frames);
  }
  return counts.length;
}

/** The decisions of a progression image: the history of its target's
 * capture, never a capture nor a variant. */
export function isProgressionPlan(plan: ImagePlan): boolean {
  return (
    plan.decision.decision === "progression" ||
    plan.decision.decision === "progression_of_new"
  );
}

/** « Images de progression : 5 pour IC 1805 », one row per target: the
 * progression images that leave when the run made them (`plans`), else
 * the milestones the target's light frames will give. */
export function progressionRows(
  t: Translate,
  input: {
    plans: readonly ImagePlan[];
    detection: NightDetection | null;
    names: (targetId: string) => string;
  },
): SummaryRow[] {
  const made = new Map<string, number>();
  for (const plan of input.plans.filter(isProgressionPlan)) {
    made.set(plan.image.targetId, (made.get(plan.image.targetId) ?? 0) + 1);
  }
  const planned = new Map<string, number>();
  if (made.size === 0) {
    for (const target of input.detection?.targets ?? []) {
      if (target.inNight && target.lightFrames > 0) {
        planned.set(target.targetId, milestoneCount(target.lightFrames));
      }
    }
  }
  return [...(made.size > 0 ? made : planned)].map(([targetId, count]) => ({
    segs: [
      t("wizard.summary.progression", {
        count,
        target: input.names(targetId),
      }),
    ],
    value: t("wizard.summary.computedHerePlural"),
  }));
}

/** The targets whose progression images have no capture to be the
 * history of: set aside, said once per target. */
export function progressionSetAside(
  t: Translate,
  input: {
    plans: readonly ImagePlan[];
    names: (targetId: string) => string;
    size: (bytes: number) => string;
  },
): SummaryRow[] {
  const bytes = new Map<string, number>();
  for (const plan of input.plans) {
    const d = plan.decision;
    if (d.decision === "not_sent" && d.reason === "no_capture_for_progression")
      bytes.set(
        plan.image.targetId,
        (bytes.get(plan.image.targetId) ?? 0) + plan.image.size,
      );
  }
  return [...bytes].map(([targetId, size]) => ({
    segs: [
      t("wizard.summary.progressionNoCapture", {
        target: input.names(targetId),
      }),
    ],
    value: input.size(size),
  }));
}

export function summaryView(
  t: Translate,
  locale: string,
  input: {
    precheck: NightPrecheck;
    detection: NightDetection | null;
    online: NightOnline | null;
    treatments: ReadonlySet<Treatment>;
    stackTargets: ReadonlySet<string>;
    /** The source of each family that has an image of a device. */
    deviceUse?: DeviceChoices;
    /** The figures measured on the image of a device, by target (« Mesurer
     * cette image »): said under the stacked image of a target that took it. */
    deviceQuality?: Readonly<Record<string, ImageQuality>>;
    resume: boolean;
    privateServer: boolean;
    /** The session's date, short and localised (`nightShortDate`). */
    date: string;
  },
): SummaryView {
  const { precheck, detection, online, treatments, resume } = input;
  const names = (targetId: string) => designation(detection, online, targetId);
  const size = (bytes: number) => formatSize(t, bytes, locale);
  const allowed = precheck.quota.allowed;
  const overPaths = new Set(
    precheck.quota.findings
      .filter((finding) => finding.limit === "file_too_large")
      .map((finding) => (finding as { path: string }).path),
  );
  const sent: SummaryRow[] = [];
  const kept: SummaryRow[] = [];
  const setAside: SummaryRow[] = [];
  const restacked = (plan: ImagePlan) =>
    treatments.has("stacking") && input.stackTargets.has(plan.image.targetId);
  const plans = precheck.plans.filter(
    (plan) =>
      !(
        restacked(plan) &&
        // The final image of a target that is stacked is replaced by the
        // stacking; so is a stacked image left in `stacked/` by an earlier
        // run: the render of this sending carries its pipeline (`render`),
        // the one left over does not. Neither is ever sent again as a
        // « final image ».
        (plan.image.kind === "final" ||
          (plan.image.kind === "stacked" && plan.image.render === null))
      ),
  );
  // The quality of a stacked image is said once under the first image that
  // carries it: the renders of one stacking share their master's figures.
  const qualitySaid = new Set<string>();
  // What a stacked target went without, said once per
  // target under its stacked image, next to the quality measured: it stays
  // readable when the image is sent.
  const withoutSaid = new Set<string>();
  const withoutNote = (targetId: string): SummaryRow | null => {
    if (withoutSaid.has(targetId) || !detection) return null;
    withoutSaid.add(targetId);
    const text = stackedWithoutText(
      t,
      locale,
      calibrationFindings(t, locale, detection.findings, targetId),
    );
    return text ? { segs: [text], value: "", note: true } : null;
  };
  for (const plan of plans) {
    // The progression images have their own line, per target (below).
    if (plan.image.kind === "progression") continue;
    const target = names(plan.image.targetId);
    const file = baseName(plan.image.path);
    const digest = shortDigest(plan.image.sha256);
    const d = plan.decision;
    if (d.decision === "already_online") {
      kept.push({
        segs: [
          t(
            plan.image.kind === "stacked"
              ? "wizard.summary.keptIdenticalStacked"
              : "wizard.summary.keptIdentical",
            { target },
          ),
        ],
        value: size(plan.image.size),
      });
    } else if (d.decision === "not_sent") {
      // A stacked image above the space's per-file ceiling is set aside with
      // its numbers; the person's final image is sent instead.
      const tooLarge = d.reason === "too_large_for_space";
      setAside.push({
        segs: [
          tooLarge
            ? t("wizard.summary.setAsideTooLarge", {
                file,
                size: size(plan.image.size),
                max: size(precheck.storage.maxFileBytes),
              })
            : t(
                d.reason === "variants_not_allowed"
                  ? "wizard.summary.setAsideNotKept"
                  : "wizard.summary.setAsideOther",
                { file },
              ),
        ],
        value: size(plan.image.size),
      });
    } else if (treatments.has("final_image") || plan.image.kind !== "final") {
      const variant =
        d.decision === "variant" || d.decision === "variant_of_new";
      // A stacked image is never called a final image.
      const lineKey = variant
        ? "wizard.summary.variantOf"
        : plan.image.kind === "stacked"
          ? "wizard.summary.stackedFile"
          : "wizard.summary.finalOf";
      sent.push({
        segs: allowed
          ? [
              t(lineKey, { target, file }),
              t("wizard.summary.digest"),
              { code: digest },
            ]
          : [t(lineKey, { target, file })],
        value: size(plan.image.size),
        over: overPaths.has(plan.image.path),
      });
      const quality = plan.metadata?.imageQuality;
      const said = `${plan.image.targetId}|${JSON.stringify(quality)}`;
      if (quality && allowed && !qualitySaid.has(said)) {
        qualitySaid.add(said);
        sent.push({
          // On an image Astro Sync did not stack, the figures were read on the
          // master the person gave.
          segs: [
            imageQualityText(
              t,
              locale,
              quality,
              plan.fromDevice
                ? "device"
                : plan.image.kind === "final"
                  ? "master"
                  : "stacking",
            ),
          ],
          value: "",
          note: true,
        });
      }
      if (plan.image.kind === "stacked") {
        const note = withoutNote(plan.image.targetId);
        if (note) sent.push(note);
      }
    }
  }
  // The main image online of a target that receives a variant is kept.
  // So is the one of a target about to be stacked again: the new image is a
  // variant of it. Said once per target, and not when an identical image is
  // already listed as kept.
  const keptMainTargets = new Set<string>(
    plans
      .filter((p) => p.decision.decision === "variant")
      .map((p) => p.image.targetId),
  );
  if (treatments.has("stacking")) {
    for (const targetId of input.stackTargets) keptMainTargets.add(targetId);
  }
  for (const targetId of keptMainTargets) {
    if (
      plans.some(
        (p) =>
          p.image.targetId === targetId &&
          p.decision.decision === "already_online",
      )
    )
      continue;
    const capture = online?.targets
      .find((target) => target.targetId === targetId)
      ?.captures.find((item) => item.mainImage);
    if (capture?.mainImage && allowed) {
      kept.push({
        segs: [
          t("wizard.summary.keptMain", {
            target: names(targetId),
            file: capture.mainImage.originalName,
            source: t(
              SOURCE_KEY[capture.mainImage.source ?? ""] ??
                "wizard.files.fromAstroSync",
            ),
          }),
        ],
        value: size(capture.mainImage.byteSize),
      });
    }
  }
  if (treatments.has("stacking")) {
    const deviceUse = input.deviceUse ?? NO_DEVICE_CHOICES;
    for (const targetId of input.stackTargets) {
      sent.push({
        segs: [t("wizard.summary.stackedOf", { target: names(targetId) })],
        value: t("wizard.summary.computedHere"),
      });
      const without = withoutNote(targetId);
      if (without) sent.push(without);
      // The render of the image a device stacked: said
      // where it starts from, with the figures when they were measured; the
      // image itself is below, among what never leaves.
      const target = detection?.targets.find(
        (item) => item.targetId === targetId,
      );
      const used = target ? usedDeviceImages(target, deviceUse) : [];
      for (const image of used) {
        sent.push({
          segs: [deviceMadeText(t, locale, image)],
          value: "",
          note: true,
        });
      }
      const measured = input.deviceQuality?.[targetId];
      if (used.length > 0 && measured && allowed) {
        sent.push({
          segs: [imageQualityText(t, locale, measured, "device")],
          value: "",
          note: true,
        });
      }
    }
  }
  if (treatments.has("logs") && allowed) {
    sent.push({
      segs: [t("wizard.summary.report")],
      value: t("wizard.summary.computedHere"),
    });
  }
  const imagesToSolve =
    plans.filter(
      (plan) =>
        plan.decision.decision !== "not_sent" &&
        plan.decision.decision !== "already_online",
    ).length +
    input.stackTargets.size * (treatments.has("stacking") ? 1 : 0);
  // On a resume, the images online keep their annotations: listed as kept.
  const onlineImages = (online?.targets ?? []).flatMap((target) =>
    target.captures
      .filter((capture) => capture.mainImage)
      .map((capture) => ({ target, image: capture.mainImage! })),
  );
  const unsolvedOnline = onlineImages.filter(
    (item) => item.image.skySolution !== "present",
  ).length;
  if (treatments.has("sky_annotations")) {
    const count = imagesToSolve + unsolvedOnline;
    if (count > 0) {
      sent.push({
        segs: [pluralText(t, locale, "wizard.summary.sky", count)],
        value: t("wizard.summary.computedHerePlural"),
      });
    }
  }
  if (resume) {
    for (const item of onlineImages) {
      const target = item.target.name ?? item.target.targetId ?? "";
      if (!plans.some((plan) => plan.image.targetId === item.target.targetId)) {
        kept.push({
          segs: [
            t("wizard.summary.keptPhone", {
              target,
              file: item.image.originalName,
              source: t(
                SOURCE_KEY[item.image.source ?? ""] ??
                  "wizard.files.fromAstroSync",
              ),
            }),
          ],
          value: size(item.image.byteSize),
        });
      }
      if (item.image.skySolution === "present") {
        kept.push({
          segs: [t("wizard.summary.keptSky", { target })],
          value: t("wizard.summary.online"),
        });
      }
    }
  }
  if (treatments.has("progression") && allowed) {
    sent.push(
      ...progressionRows(t, { plans: precheck.plans, detection, names }),
    );
    setAside.push(
      ...progressionSetAside(t, { plans: precheck.plans, names, size }),
    );
  }

  // What never leaves this computer.
  const never: SummaryRow[] = [];
  if (detection && treatments.has("stacking")) {
    // The image a device stacked is read in place: it never leaves, only the
    // render does.
    const deviceUse = input.deviceUse ?? NO_DEVICE_CHOICES;
    for (const target of detection.targets.filter(
      (item) => item.inNight && input.stackTargets.has(item.targetId),
    )) {
      for (const image of usedDeviceImages(target, deviceUse)) {
        never.push({
          segs: [
            t("deviceMaster.summary.never"),
            " · ",
            { code: image.fileName },
          ],
          value: size(image.bytes),
        });
        never.push({
          segs: [t("deviceMaster.summary.neverNote")],
          value: "",
          note: true,
        });
      }
    }
  }
  if (detection) {
    const lights = detection.targets
      .filter((target) => target.inNight)
      .reduce((sum, target) => sum + target.lightFrames, 0);
    const lightBytes = detection.targets.reduce(
      (sum, target) => sum + target.lightSummary.bytes,
      0,
    );
    const c = detection.calibration;
    const calibration = c.darks + c.flats + c.darkFlats + c.biases;
    const calibrationBytes =
      detection.calibrationSummary.darks.bytes +
      detection.calibrationSummary.flats.bytes;
    if (lights + calibration > 0) {
      never.push({
        segs: [t("wizard.summary.fits", { lights, calibration })],
        value: size(lightBytes + calibrationBytes),
      });
    }
    const kinds = [...new Set(detection.logs.map((log) => log.kind))]
      .filter((kind) => kind !== "unknown")
      .map(
        (kind) =>
          ({ nina: "NINA", phd2: "PHD2", asiair: "ASIAIR" })[kind as "nina"],
      );
    if (detection.logs.length > 0) {
      never.push({
        segs: [t("wizard.summary.rawLogs", { kinds: kinds.join(", ") })],
        value: pluralText(
          t,
          locale,
          "wizard.summary.files",
          detection.logs.length,
        ),
      });
    }
  }

  const groups: SummaryGroup[] = [
    {
      key: "sent",
      titleKey: allowed ? "wizard.summary.sent" : "wizard.summary.toSend",
      rows: sent,
    },
    { key: "keptOnline", titleKey: "wizard.summary.keptOnline", rows: kept },
    { key: "setAside", titleKey: "wizard.summary.setAside", rows: setAside },
    { key: "neverLeaves", titleKey: "wizard.summary.neverLeaves", rows: never },
  ].filter((group) => group.rows.length > 0) as SummaryGroup[];

  // The check before the sending.
  const storage = precheck.storage;
  const toSend = precheck.quota.toSendBytes;
  const checks: CheckLine[] = [];
  const remainingText = size(storage.remainingBytes);
  const after = size(Math.max(0, storage.remainingBytes - toSend));
  checks.push({
    ok: true,
    // A sending too small to change the place left, as written, says
    // nothing of what will remain (« 38 ko »). A server that keeps no quota
    // (a private server) has no place left to count: « no limit ».
    text:
      storage.quotaBytes >= Number.MAX_SAFE_INTEGER
        ? t("wizard.summary.check.placeUnlimited", { toSend: size(toSend) })
        : toSend * 1000 < storage.remainingBytes
          ? t("wizard.summary.check.placeShort", {
              toSend: size(toSend),
              remaining: remainingText,
              quota: size(storage.quotaBytes),
            })
          : t("wizard.summary.check.place", {
              toSend: size(toSend),
              remaining: remainingText,
              quota: size(storage.quotaBytes),
              after,
            }),
  });
  // A space that bounds its number of captures (the free account) says the
  // captures this sending adds and those left, beside the place left.
  const captureLimit = storage.captureCountLimit;
  if (captureLimit > 0 && captureLimit < Number.MAX_SAFE_INTEGER) {
    checks.push({
      ok: true,
      text: t("wizard.summary.check.captures", {
        adding: plans.filter((plan) => plan.decision.decision === "new_capture")
          .length,
        left: Math.max(0, captureLimit - storage.captureCount),
        limit: captureLimit,
      }),
    });
  }
  // What is left of the plans (the final image of a target stacked again, or
  // a stacked image of an earlier run, is replaced and sent no more).
  const sending = plans.filter(
    (plan) =>
      plan.decision.decision === "new_capture" ||
      plan.decision.decision === "variant" ||
      plan.decision.decision === "variant_of_new" ||
      isProgressionPlan(plan),
  );
  const largest = [...sending].sort((a, b) => b.image.size - a.image.size)[0];
  if (largest) {
    checks.push({
      ok: true,
      text: t("wizard.summary.check.largest", {
        file: baseName(largest.image.path),
        size: size(largest.image.size),
        max: size(storage.maxFileBytes),
      }),
    });
  }
  const variantTargets = new Map<string, number>();
  for (const plan of sending) {
    if (plan.decision.decision === "variant") {
      variantTargets.set(
        plan.image.targetId,
        (variantTargets.get(plan.image.targetId) ?? 0) + 1,
      );
    }
  }
  for (const [targetId, adding] of variantTargets) {
    const capture = online?.targets
      .find((target) => target.targetId === targetId)
      ?.captures.find((item) => item.mainImage);
    const existing = capture ? Math.max(0, capture.imageCount - 1) : 0;
    // A space with no cap on the variants (a subscription, a private server:
    // the unlimited sentinel) shows no « N sur M »: the count alone.
    const unlimited = storage.variantLimit >= Number.MAX_SAFE_INTEGER;
    checks.push({
      ok: true,
      text: unlimited
        ? t("wizard.summary.check.variantsUnlimited", {
            target: names(targetId),
            count: existing + adding,
          })
        : t("wizard.summary.check.variants", {
            target: names(targetId),
            count: existing + adding,
            limit: storage.variantLimit,
          }),
    });
  }

  return {
    leadKey: !allowed
      ? "wizard.summary.leadImpossible"
      : resume
        ? "wizard.summary.leadResume"
        : "wizard.summary.lead",
    leadParams: { date: input.date },
    groups,
    allowed,
    checks: allowed ? (resume ? checks.slice(0, 1) : checks) : [],
    analysisNote: allowed && resume && treatments.has("logs"),
    refusals: allowed
      ? []
      : precheck.quota.findings.map((finding) =>
          refusalText(t, locale, finding, names, input.privateServer),
        ),
    toSend: size(toSend),
    primaryKey: resume
      ? "wizard.summary.sendComplements"
      : "wizard.summary.send",
  };
}

/* ------------------------------ The sending ------------------------------ */

export type ItemState =
  | "waiting"
  | "local"
  | "sending"
  | "done"
  | "failed"
  /** Nothing to do for this treatment: « rien à traiter ». */
  | "none";

export interface SendingItem {
  key: string;
  label: string;
  state: ItemState;
  /** « 255 Mo sur 412 Mo », « 1 image sur 3 ». */
  detail: string | null;
  /** 0–1, or `null` without a bar. */
  progress: number | null;
  /** A treatment that ran on this computer and is finished says « fait »,
   * not « envoyé »; `durationMs`: how long it took. */
  localDone?: boolean;
  durationMs?: number | null;
  /** The service already held these bytes (same SHA-256): the line says
   * « déjà en ligne, identique », not « envoyé ». */
  alreadyOnline?: boolean;
}

/** The words of an item's state in the right column. */
export function itemStateText(t: Translate, item: SendingItem): string {
  if (item.detail && (item.state === "sending" || item.state === "local")) {
    return item.detail;
  }
  switch (item.state) {
    case "none":
      return t("wizard.sending.nothing");
    case "done":
      if (item.alreadyOnline) return t("wizard.sending.identical");
      if (!item.localDone) return t("wizard.sending.sent");
      return item.durationMs === null || item.durationMs === undefined
        ? t("wizard.sending.done")
        : t("wizard.sending.doneIn", {
            duration: formatDuration(t, item.durationMs),
          });
    case "local":
      return t("wizard.sending.local");
    case "failed":
      return t("wizard.sending.failed");
    case "sending":
      return t("wizard.sending.sending");
    default:
      return t("wizard.sending.waiting");
  }
}

/** The items of a sending, in the screen's order: the report, the images,
 * the sky annotations, the progression; the stacking (not drawn: the
 * progression's line is its model) after the report, once it started. */
export function sendingItems(
  t: Translate,
  _locale: string,
  input: {
    plans: readonly ImagePlan[];
    detection: NightDetection | null;
    online: NightOnline | null;
    treatments: ReadonlySet<Treatment>;
    /** Per item key, what is known of it so far. */
    progress: Readonly<Record<string, Partial<SendingItem>>>;
  },
): SendingItem[] {
  const names = (targetId: string) =>
    designation(input.detection, input.online, targetId);
  const items: SendingItem[] = [];
  const add = (key: string, label: string) =>
    items.push({
      key,
      label,
      state: "waiting",
      detail: null,
      progress: null,
      ...input.progress[key],
    });
  if (input.treatments.has("logs")) add("logs", t("wizard.sending.report"));
  if (input.treatments.has("stacking") && input.progress.stacking) {
    add("stacking", t("wizard.sending.stacking"));
  }
  for (const plan of input.plans) {
    const d = plan.decision.decision;
    if (d === "new_capture" || d === "variant" || d === "variant_of_new") {
      add(
        `image:${plan.image.sha256}`,
        t(
          d === "new_capture"
            ? "wizard.sending.final"
            : "wizard.sending.variant",
          {
            target: names(plan.image.targetId),
          },
        ),
      );
    }
  }
  // The final image ticked with no final image to send: its line stays,
  // « rien à traiter ».
  if (
    input.treatments.has("final_image") &&
    !input.plans.some(
      (plan) =>
        plan.image.kind === "final" &&
        ["new_capture", "variant", "variant_of_new"].includes(
          plan.decision.decision,
        ),
    )
  ) {
    add("finalImage", t("wizard.sending.finalImage"));
    const last = items[items.length - 1];
    items[items.length - 1] = {
      ...last,
      state: "none",
      ...input.progress.finalImage,
    };
  }
  if (input.treatments.has("sky_annotations")) {
    add("sky", t("wizard.sending.sky"));
  }
  if (input.treatments.has("progression")) {
    add("progression", t("wizard.sending.progression"));
  }
  return items.map((item) => ({ ...item, label: item.label }));
}

/** « 212 Mo sur 446 Mo » over the whole sending. */
export function overallText(
  t: Translate,
  locale: string,
  sent: number,
  total: number,
): string {
  return t("wizard.sending.overall", {
    sent: formatSize(t, sent, locale),
    total: formatSize(t, total, locale),
  });
}

/* ------------------------------ Nuit envoyée ----------------------------- */

export function sentLines(
  t: Translate,
  locale: string,
  input: {
    plans: readonly ImagePlan[];
    detection: NightDetection | null;
    online: NightOnline | null;
    treatments: ReadonlySet<Treatment>;
    solved: number;
    progressionImages: number;
  },
): string[] {
  const names = (targetId: string) =>
    designation(input.detection, input.online, targetId);
  const lines: string[] = [];
  const captures = input.plans.filter((plan) =>
    ["new_capture", "variant", "variant_of_new"].includes(
      plan.decision.decision,
    ),
  );
  const identical = input.plans
    .filter(
      (plan) =>
        plan.decision.decision === "already_online" &&
        plan.image.kind !== "progression",
    )
    .map((plan) => names(plan.image.targetId));
  if (captures.length > 0 || identical.length > 0) {
    const sent = captures
      .map((plan) =>
        t(
          plan.decision.decision === "new_capture"
            ? "wizard.sent.newCapture"
            : "wizard.sent.variant",
          { target: names(plan.image.targetId) },
        ),
      )
      .join(", ");
    const parts = [];
    if (captures.length > 0) {
      parts.push(
        pluralText(t, locale, "wizard.sent.captures", captures.length, {
          list: sent,
        }),
      );
    }
    if (identical.length > 0) {
      parts.push(
        pluralText(t, locale, "wizard.sent.identical", identical.length, {
          targets: identical.join(", "),
        }),
      );
    }
    lines.push(parts.join(t("wizard.sent.separator")));
  }
  if (input.treatments.has("logs")) lines.push(t("wizard.sent.report"));
  if (input.treatments.has("sky_annotations") && input.solved > 0) {
    lines.push(pluralText(t, locale, "wizard.summary.sky", input.solved));
  }
  if (input.treatments.has("progression") && input.progressionImages > 0) {
    lines.push(
      pluralText(t, locale, "wizard.sent.progression", input.progressionImages),
    );
  }
  return lines;
}

/* -------------------------------- Resume -------------------------------- */

/** The neutral notice of a resumed night: what is online, and
 * what was added to the folder since. `null` when nothing was sent yet. */
export function resumeNotice(
  t: Translate,
  locale: string,
  input: {
    resume: NightResume;
    online: NightOnline | null;
    detection: NightDetection | null;
  },
): string | null {
  const { resume, online, detection } = input;
  // A resume is a night sent before whose folder received files since.
  if (
    resume.status !== "to_complete" ||
    !resume.missing.some((item) => item.key === "logsAdded")
  ) {
    return null;
  }
  const captures = (online?.targets ?? []).flatMap((target) =>
    target.captures
      .filter((capture) => capture.mainImage)
      .map((capture) => ({ target, image: capture.mainImage! })),
  );
  if (captures.length === 0 && !online?.analysis.present) return null;
  const date = (instant: string) =>
    new Intl.DateTimeFormat(locale, { day: "numeric", month: "short" }).format(
      new Date(instant),
    );
  const onlineParts = captures.map(({ target, image }) =>
    t(
      image.skySolution === "present"
        ? "wizard.resume.imageAnnotated"
        : "wizard.resume.image",
      {
        target: target.name ?? target.targetId ?? "",
        source: t(
          SOURCE_KEY[image.source ?? ""] ?? "wizard.files.fromAstroSync",
        ),
        date: date(image.createdAt),
      },
    ),
  );
  const analysis =
    online?.analysis.present && online.analysis.sentAt
      ? t("wizard.resume.analysis", { date: date(online.analysis.sentAt) })
      : t("wizard.resume.noAnalysis");
  let added = "";
  if (resume.missing.some((m) => m.key === "logsAdded") && detection) {
    const frames = detection.targets.reduce(
      (sum, target) => sum + target.lightFrames,
      0,
    );
    added = t("wizard.resume.added", {
      logs: pluralText(t, locale, "wizard.resume.logs", detection.logs.length),
      frames: pluralText(t, locale, "wizard.files.frames", frames),
    });
  }
  return t("wizard.resume.notice", {
    online: onlineParts.join(", "),
    analysis,
    added,
  }).trim();
}

/** The stacking script's own codes, by the sentence that says them; the
 * others are said by the kind its exit code gives (`bad_input` 2,
 * `tool_missing` 3, `tool_failed` 4). */
const STACKING_CODES: Record<string, string> = {
  bad_input: "bad_input",
  folder_missing: "bad_input",
  frames_outside_lights: "bad_input",
  output_inside_input: "bad_input",
  output_is_final: "bad_input",
  work_not_empty: "bad_input",
  too_few_lights: "too_few_lights",
  // The image of a device that cannot be rendered.
  device_stretched: "device_stretched",
  device_unreadable: "device_unreadable",
  script_not_trusted: "script_not_trusted",
  unknown_pipeline: "unknown_pipeline",
  siril_missing: "siril_missing",
  starnet_missing: "starnet_missing",
  tool_missing: "tool_missing_here",
  calibration_failed: "siril_failed",
  registration_failed: "siril_failed",
  integration_failed: "siril_failed",
  starnet_failed: "starnet_failed",
  render_failed: "tool_failed",
  script_failed: "tool_failed",
  tool_failed: "tool_failed",
};

/** Not enough room for the progression images on the night's disk
 * (`progression_images::ProgressionFailure::NotEnoughSpace`). */
export function progressionSpaceText(
  t: Translate,
  details: Record<string, unknown>,
  locale = "fr",
): string {
  return t("errors.progression.not_enough_space", {
    folder: String(details.folder ?? ""),
    needed: formatSize(t, Number(details.needed ?? 0), locale),
    available: formatSize(t, Number(details.available ?? 0), locale),
  });
}

/** Why a stacking gave no image (`stacking::StackFailure`, the `details`
 * of `stacking_failed`), in words: never a code. */
export function stackingFailureText(
  t: Translate,
  details: Record<string, unknown>,
  locale = "fr",
): string {
  const why = String(details.why ?? "");
  switch (why) {
    case "not_in_subscription":
    case "tool_missing":
    case "one_render_only":
    case "no_lights":
    case "cancelled":
    case "outside_stacked":
      return t(`errors.stacking.${why}`);
    case "not_enough_space":
      return t("errors.stacking.not_enough_space", {
        folder: String(details.folder ?? ""),
        needed: formatSize(t, Number(details.needed ?? 0), locale),
        available: formatSize(t, Number(details.available ?? 0), locale),
      });
    case "failed": {
      const code = STACKING_CODES[String(details.code ?? "")];
      if (!code) return t("errors.stacking_failed");
      // The two failures that leave journals say where (`logs`: the folder
      // of Astro Sync's data they were copied to before the scratch went).
      if (code === "siril_failed" || code === "tool_failed") {
        const logs = String(details.logs ?? "");
        return logs
          ? t(`errors.stacking.${code}`, { logs })
          : t(`errors.stacking.${code}_no_logs`);
      }
      return t(`errors.stacking.${code}`);
    }
    default:
      return t("errors.stacking_failed");
  }
}
