// The wizard's step « Traitements »: five ticked options, each with its tool and its
// condition. Astro Sync reads capabilities (`analysis.logs`,
// `analysis.progression` -- the progression only: the stacking is local and
// never gated --, `capture.variant_limit`), never an offer's name: the names of the offers
// are only words of the texts. Pure TypeScript, tested under `node --test`.

import type { TranslateParams } from "../i18n/locale.ts";
import type {
  Band,
  Finding,
  ImagePlan,
  IntegrationPreview,
  NightDetection,
  NightOnline,
  Pipeline,
  PolishedEngine,
  ToolRow,
  Treatment,
  TreatmentState,
} from "./bridge";
import {
  NO_DEVICE_CHOICES,
  deviceChoiceView,
  hasDeviceImage,
  restacksPoses,
  type DeviceChoiceView,
  type DeviceChoices,
} from "./deviceMaster.ts";
import {
  DEFAULT_FINISH,
  changedFinishes,
  finishLine,
  type PipelineFinish,
  type TargetFinishes,
} from "./finish.ts";
import { calibrationFindings, type NoteLevel } from "./missingMessages.ts";
import type { FileLine, Seg } from "./wizardFiles.ts";
import { baseName } from "./wizardSteps.ts";
import {
  eveningShort,
  integrationTime,
  targetIntegrationView,
  type PlanImageChoices,
  type TargetIntegrationView,
} from "./multiNight.ts";
import {
  NO_PROCESSING_ID,
  inListOrder,
  isDiscouraged,
  pipelineName,
  rendersOf,
  toolPill,
  usesGraxpert,
  type Progress,
  type TargetRenders,
  type ToolPill,
} from "./pipelineChoice.ts";
import { frameLine, type TargetFrames } from "./stackFrame.ts";
import { conjunction, formatSize, pluralText } from "./written.ts";

type Translate = (key: string, params?: TranslateParams) => string;

/** « Sans traitement » (one pipeline more, by
 * target, combinable): the id of the core, `LINEAR_PIPELINE`. */
export { NO_PROCESSING_ID };

/** The five options, in their display order. */
export const TREATMENTS: readonly Treatment[] = [
  "stacking",
  "final_image",
  "sky_annotations",
  "logs",
  "progression",
];

export type OptionState = "available" | "reserved" | "toolMissing" | "nothing";

/** One pipeline chosen for a target, as its card shows it: a neutral label
 * with a gold tick, and an amber word when something is to be said. */
export interface PipelineChip {
  id: string;
  name: string;
  /** « déconseillé pour ce filtre », « sans GraXpert », « GraXpert · 48 % ». */
  warning: string | null;
}

/** One line under the integration of a card: what becomes of the final
 * image online or in final/. */
export interface CardNote {
  segs: Seg[];
  /** Bold (a variant will be created), muted, or with the variant glyph. */
  tone: "strong" | "muted" | "variant";
}

/**
 * The card of a target in the box « Empilement »: its tick in one column for every
 * card, its name, its
 * band, its frames, the pipelines chosen, « Choisir les pipelines », then
 * (ticked) its integration of the night and what becomes of its final image.
 * Unticked: greyed, its choice kept.
 */
export interface StackingCard {
  targetId: string;
  designation: string;
  folderName: string | null;
  ticked: boolean;
  /** The tick can be changed (the stacking can run). */
  enabled: boolean;
  band: Band;
  bandLabel: string;
  /** « 40 poses · 2 h 00 · 1 famille ». */
  line: string;
  /** The filter of its frames (« UV/IR-cut »), for the window. */
  filter: string | null;
  /** « 2 h 00 », for the window; null before the integration is read. */
  duration: string | null;
  /** The same in seconds, for « Cadre de l'empilement » of the window. */
  seconds: number | null;
  frames: number;
  /** The ids chosen, in the display order, « Sans traitement » last. */
  chosen: string[];
  /** The image a device already stacked and the choice it gives;
   * `null` for a target without one, or not ticked. */
  device: DeviceChoiceView | null;
  /** Some pose is stacked again: the measure of the poses and the poses
   * flagged have a meaning (true for a target with no device image). */
  restacksPoses: boolean;
  chips: PipelineChip[];
  /** « Finition modifiée · Correction bibande : Fort · Couleurs +20 % ·
   * Étoiles 50 % »: one per pipeline chosen whose finish differs from the
   * default; none when everything follows it. */
  finishes: { pipeline: string; finish: string }[];
  /** « Cadre : Zone commune »: only when the frame was moved by hand
   * (Automatic says nothing). */
  frame: string | null;
  /** « Aucun pipeline disponible tant que … » (Siril or Python missing). */
  noPipeline: string | null;
  /** « Choisir les pipelines »: tertiary when a choice exists, secondary
   * when none does; null when nothing can be chosen. */
  button: "tertiary" | "secondary" | null;
  /** The integration of the night (ticked cards). */
  integration: TargetIntegrationView | null;
  /** The darks and the flats the target's poses lack, said before the
   * stacking runs: warnings, or informations when
   * there is none at all. */
  calibration: CalibrationLine[];
  notes: CardNote[];
}

/** One line under a target card: a warning (calibration that exists and does
 * not fit) or an information (none at all). */
export interface CalibrationLine {
  level: NoteLevel;
  text: string;
}

/** The badge at the right of an option's title: why it cannot be ticked, or
 * that it waits for the stacking. Neutral for a mere absence or a reserved
 * option, amber when something is to be done -- never red. */
export interface OptionBadge {
  tone: "neutral" | "warning";
  label: string;
  /** `lock` for a reserved option, `warning` for a tool to install. */
  icon: "lock" | "warning" | null;
}

export interface TreatmentOption {
  treatment: Treatment;
  titleKey: string;
  state: OptionState;
  ticked: boolean;
  badge: OptionBadge | null;
  /** The tool line: its name and version, « outil présent »,
   * « introuvable » (bold) or nothing when reserved. */
  tool: Seg[];
  description: string;
  /** Per target: the final images, the logs found, the stacking. */
  lines: FileLine[];
  /** The stacking: one card per target. */
  cards: StackingCard[];
  /** The stacking: what is to install before it can run, each with its
   * button and size (« À installer : Siril Absent Télécharger (75,8 Mo) »). */
  toInstall: ToolPill[];
  /** No FITS frame anywhere: « Aucune pose FITS dans lights/ … ». */
  noFrames: boolean;
  /** Several pipelines per target (`capture.variant_limit` > 0). */
  severalRenders: boolean;
  /** « Voir les offres » on the option (a variant this offer does not
   * keep). */
  offersLink: boolean;
}

export interface TreatmentsView {
  /** « Journaux et progression sont réservés à … » */
  reservedLine: boolean;
  /** « Serveur privé : https://… » */
  privateServer: string | null;
  options: TreatmentOption[];
  /** The slider of the contrast: only when a target is ticked for the
   * stacking. */
  contrastShown: boolean;
}

export interface TreatmentChoices {
  /** Options the person unticked. */
  unticked: ReadonlySet<Treatment>;
  /** Targets ticked for the stacking. */
  stackTargets: ReadonlySet<string>;
  /** The pipelines chosen for each target; a target not named
   * here takes its default (`pipelineChoice.rendersOf`). */
  targetRenders: TargetRenders;
  /** The finishes set by hand, by target then pipeline (« Finition » of
   * the window); a pipeline not named follows the « Finition par défaut ». */
  targetFinishes?: TargetFinishes;
  /** The frames moved by hand, by target (« Cadre de l'empilement » of the
   * window); a target not named is Automatic. */
  targetFrames?: TargetFrames;
  /** « Image du plan multi-nuits », by multi-night plan; a plan not
   * named here takes `DEFAULT_PLAN_IMAGE`. */
  planImage?: Readonly<Record<string, PlanImageChoices>>;
  /** The source of each family that has an image of a device, by target:
   * the device's image, or a new stacking; a family not named takes the
   * engine's default. */
  deviceUse?: DeviceChoices;
}

export const DEFAULT_CHOICES: TreatmentChoices = {
  unticked: new Set(),
  stackTargets: new Set(),
  targetRenders: {},
  targetFinishes: {},
  targetFrames: {},
  planImage: {},
  deviceUse: NO_DEVICE_CHOICES,
};

const TITLE_KEY: Record<Treatment, string> = {
  stacking: "wizard.treatments.stacking.title",
  final_image: "wizard.treatments.final.title",
  sky_annotations: "wizard.treatments.sky.title",
  logs: "wizard.treatments.logs.title",
  progression: "wizard.treatments.progression.title",
};

function tool(rows: readonly ToolRow[], id: string): ToolRow | undefined {
  return rows.find((row) => row.id === id);
}

function toolName(
  row: ToolRow | undefined,
  fallback: string,
  locale: string,
): string {
  if (!row) return fallback;
  return row.name[locale] ?? row.name.en ?? row.name.fr ?? fallback;
}

/** Whether a capture of this target is online already. */
function onlineCapture(online: NightOnline | null, targetId: string) {
  return (
    online?.targets
      .find((target) => target.targetId === targetId)
      ?.captures.find((capture) => capture.mainImage) ?? null
  );
}

const NOTHING_KEY: Record<Treatment, string> = {
  stacking: "wizard.treatments.badge.nothing.frames",
  final_image: "wizard.treatments.badge.nothing.final",
  sky_annotations: "wizard.treatments.badge.nothing.sky",
  logs: "wizard.treatments.badge.nothing.logs",
  progression: "wizard.treatments.badge.nothing.frames",
};

/**
 * The states of the core, corrected by what only the window knows: the sky
 * annotations hold on a final image (here or online) or on the image the
 * stacking will make -- the stacking ticked, with a target ticked for it.
 * Without either, there is nothing to annotate, and the option says so.
 * `skyAfterStacking` is true when they hold on the stacking only.
 */
export function resolveTreatments(
  states: readonly TreatmentState[],
  detection: NightDetection | null,
  online: NightOnline | null,
  choices: Pick<TreatmentChoices, "unticked" | "stackTargets">,
): { states: TreatmentState[]; skyAfterStacking: boolean } {
  const sky = states.find((item) => item.treatment === "sky_annotations");
  if (!sky?.available) return { states: [...states], skyAfterStacking: false };
  const hasImage =
    (online?.targets ?? []).some((target) =>
      target.captures.some((capture) => capture.mainImage),
    ) || (detection?.targets ?? []).some((target) => target.finalImage);
  if (hasImage) return { states: [...states], skyAfterStacking: false };
  const stacking = states.find((item) => item.treatment === "stacking");
  const stacked =
    !!stacking?.available &&
    !choices.unticked.has("stacking") &&
    (detection?.targets ?? []).some(
      (target) =>
        target.inNight &&
        (target.lightFrames > 0 || hasDeviceImage(target)) &&
        choices.stackTargets.has(target.targetId),
    );
  if (stacked) return { states: [...states], skyAfterStacking: true };
  return {
    states: states.map((item) =>
      item.treatment === "sky_annotations"
        ? {
            treatment: item.treatment,
            available: false,
            unavailable: { why: "nothing_to_treat" as const },
          }
        : item,
    ),
    skyAfterStacking: false,
  };
}

/**
 * The treatments ticked and available on this night: every option available
 * and not unticked by the person. « Journaux » is thus ticked by default as
 * soon as the night holds a log (PHD2, ASIAIR, NINA) and the space may send
 * its report -- only values leave, never a log --; the person can untick it. The sky annotations also hold on the stacking, which only
 * the window's choices tell.
 */
export function tickedTreatmentsOf(
  states: readonly TreatmentState[],
  detection: NightDetection | null,
  online: NightOnline | null,
  choices: Pick<TreatmentChoices, "unticked" | "stackTargets">,
): Set<Treatment> {
  const resolved = resolveTreatments(states, detection, online, choices).states;
  const available = (treatment: Treatment) =>
    resolved.find((item) => item.treatment === treatment)?.available ?? false;
  return new Set(
    TREATMENTS.filter(
      (treatment) => !choices.unticked.has(treatment) && available(treatment),
    ),
  );
}

function stateOf(state: TreatmentState | undefined): OptionState {
  if (!state || state.available) return "available";
  switch (state.unavailable?.why) {
    case "not_in_subscription":
      return "reserved";
    case "tool_missing":
      return "toolMissing";
    default:
      return "nothing";
  }
}

export function treatmentsView(
  t: Translate,
  locale: string,
  input: {
    treatments: readonly TreatmentState[];
    tools: readonly ToolRow[];
    detection: NightDetection | null;
    online: NightOnline | null;
    plans: readonly ImagePlan[];
    /** `capture.variant_limit` > 0: several renders, variants sent. */
    variantsAllowed: boolean;
    /** The service's address when it is a private server. */
    privateServer: string | null;
    pipelines: readonly Pipeline[];
    choices: TreatmentChoices;
    /** « Intégration de la nuit » of each target, by target id, once read. */
    previews?: Readonly<Record<string, IntegrationPreview>>;
    /** The evening of the night (the frames outside it are said). */
    eveningDate?: string | null;
    /** The engine of the « Rendu soigné » (Réglages › Général). */
    engine?: PolishedEngine;
    /** The downloads in progress, by tool id. */
    progress?: Progress;
    /** « Finition par défaut »: what a pipeline without its own finish takes. */
    finishDefault?: PipelineFinish;
  },
): TreatmentsView {
  const { tools, detection, online, plans, choices } = input;
  const resolved = resolveTreatments(
    input.treatments,
    detection,
    online,
    choices,
  );
  const treatments = resolved.states;
  const state = (treatment: Treatment) =>
    stateOf(treatments.find((item) => item.treatment === treatment));
  const ticked = (treatment: Treatment) =>
    state(treatment) === "available" && !choices.unticked.has(treatment);
  const stackingOn = ticked("stacking");
  const targets = (detection?.targets ?? []).filter((target) => target.inNight);

  const badgeOf = (
    treatment: Treatment,
    optionState: OptionState,
  ): OptionBadge | null => {
    switch (optionState) {
      case "reserved":
        return {
          tone: "neutral",
          label: t("wizard.treatments.reservedBadge"),
          icon: "lock",
        };
      case "toolMissing":
        return {
          tone: "warning",
          label: t("wizard.treatments.toolMissing"),
          icon: "warning",
        };
      case "nothing":
        return {
          tone: "neutral",
          label: t(NOTHING_KEY[treatment]),
          icon: null,
        };
      default:
        return treatment === "sky_annotations" && resolved.skyAfterStacking
          ? {
              tone: "neutral",
              label: t("wizard.treatments.badge.afterStacking"),
              icon: null,
            }
          : null;
    }
  };

  const options: TreatmentOption[] = TREATMENTS.map((treatment) => {
    const optionState = state(treatment);
    const base: TreatmentOption = {
      treatment,
      titleKey: TITLE_KEY[treatment],
      state: optionState,
      ticked: ticked(treatment),
      badge: badgeOf(treatment, optionState),
      tool: [],
      description: "",
      lines: [],
      cards: [],
      toInstall: [],
      noFrames: false,
      severalRenders: input.variantsAllowed,
      offersLink: false,
    };
    const missingTool = (name: string): Seg[] => [
      `${name} · `,
      { strong: t("wizard.treatments.notFound") },
    ];
    switch (treatment) {
      case "stacking": {
        const name = t("wizard.treatments.stacking.tool");
        base.tool =
          optionState === "available"
            ? [`${name} · ${t("wizard.treatments.toolPresent")}`]
            : optionState === "toolMissing"
              ? missingTool(name)
              : [name];
        base.description =
          optionState === "toolMissing"
            ? t("wizard.treatments.stacking.notVerified")
            : t("wizard.treatments.stacking.line");
        // The image a device stacked is something to render, poses or not.
        const stackable = targets.filter(
          (target) => target.lightFrames > 0 || hasDeviceImage(target),
        );
        base.noFrames = stackable.length === 0 && optionState !== "reserved";
        if (
          (optionState === "available" && base.ticked) ||
          optionState === "toolMissing"
        ) {
          // A target with no FITS frame has nothing to stack.
          base.cards = stackable.map((target) =>
            stackingCard(t, locale, {
              target,
              open: optionState === "available",
              online,
              choices,
              pipelines: input.pipelines,
              tools,
              several: input.variantsAllowed,
              preview: input.previews?.[target.targetId] ?? null,
              eveningDate: input.eveningDate ?? null,
              engine: input.engine ?? "graxpert-ai",
              progress: input.progress ?? {},
              detectionFolder: detection?.folder ?? null,
              findings: detection?.findings ?? [],
              finishDefault: input.finishDefault,
            }),
          );
          if (optionState === "toolMissing") {
            base.toInstall = ["siril", "python-env"]
              .map((id) =>
                toolPill(t, locale, {
                  id,
                  required: true,
                  row: tools.find((row) => row.id === id),
                  progress: input.progress ?? {},
                }),
              )
              .filter((pill) => pill.state !== "present");
          }
        } else if (optionState === "available") {
          base.description = `${base.description} ${t("wizard.treatments.stacking.lineOnline")}`;
        }
        break;
      }
      case "final_image": {
        base.tool = [t("wizard.treatments.final.tool")];
        base.description = t("wizard.treatments.final.line");
        for (const plan of plans.filter(
          (item) => item.image.kind === "final",
        )) {
          const target = targets.find(
            (item) => item.targetId === plan.image.targetId,
          );
          const designation = target?.designation ?? plan.image.targetId;
          const file = baseName(plan.image.path);
          const size = formatSize(t, plan.image.size, locale);
          const stacked =
            stackingOn && choices.stackTargets.has(plan.image.targetId);
          const decision = plan.decision;
          if (stacked) {
            base.lines.push(
              onlineCapture(online, plan.image.targetId)
                ? {
                    glyph: "none",
                    framed: true,
                    segs: [
                      { strong: designation },
                      ` · ${t("wizard.treatments.final.setAside", { file })}`,
                    ],
                  }
                : {
                    glyph: "ok",
                    segs: [
                      { strong: designation },
                      ` · ${t("wizard.treatments.final.replaced", { file, size })}`,
                    ],
                  },
            );
          } else if (decision.decision === "already_online") {
            base.lines.push({
              glyph: "ok",
              segs: [
                { strong: designation },
                ` · ${t("wizard.treatments.final.identical")}`,
              ],
            });
          } else if (
            decision.decision === "variant" ||
            decision.decision === "variant_of_new"
          ) {
            base.lines.push({
              glyph: "variant",
              segs: [
                { strong: designation },
                ` · ${t("wizard.treatments.final.variant", { file, size })}`,
              ],
            });
          } else if (decision.decision === "not_sent") {
            const why =
              decision.reason === "variants_not_allowed"
                ? "wizard.treatments.final.notKept"
                : decision.reason === "replaced_by_stacking"
                  ? "wizard.treatments.final.setAside"
                  : "wizard.treatments.final.otherFinal";
            if (decision.reason === "variants_not_allowed")
              base.offersLink = true;
            base.lines.push({
              glyph:
                decision.reason === "variants_not_allowed" ? "warning" : "none",
              framed: true,
              segs: [{ strong: designation }, ` · ${t(why, { file })}`],
            });
          } else {
            base.lines.push({
              glyph: "ok",
              segs: [
                { strong: designation },
                ` · ${t("wizard.treatments.final.willSend", { file, size })}`,
              ],
            });
          }
        }
        break;
      }
      case "sky_annotations": {
        const astap = tool(tools, "astap");
        const name = toolName(astap, "ASTAP", locale);
        base.tool =
          optionState === "toolMissing"
            ? missingTool(name)
            : [
                `${[name, astap?.version].filter(Boolean).join(" ")} · ${t("wizard.treatments.sky.tool")}`,
              ];
        base.description =
          optionState === "toolMissing"
            ? t("wizard.treatments.sky.noSolver")
            : t("wizard.treatments.sky.line");
        if (resolved.skyAfterStacking && optionState === "available") {
          base.lines.push({
            glyph: "ok",
            segs: [t("wizard.treatments.sky.afterStacking")],
          });
        }
        break;
      }
      case "logs": {
        const name = t("wizard.treatments.logs.tool");
        base.tool =
          optionState === "reserved"
            ? [name]
            : [`${name} · ${t("wizard.treatments.logs.noTool")}`];
        base.description =
          optionState === "nothing"
            ? t("wizard.treatments.logs.none")
            : t("wizard.treatments.logs.line");
        const logs = detection?.logs ?? [];
        if (optionState === "available" && logs.length > 0) {
          const kinds = [...new Set(logs.map((log) => log.kind))]
            .filter((kind) => kind !== "unknown")
            .map(
              (kind) =>
                ({ nina: "NINA", phd2: "PHD2", asiair: "ASIAIR" })[
                  kind as "nina"
                ],
            );
          base.lines.push({
            glyph: "ok",
            segs: [
              pluralText(
                t,
                locale,
                "wizard.treatments.logs.found",
                logs.length,
              ),
              { code: "logs/" },
              t("wizard.treatments.logs.kinds", { kinds: kinds.join(", ") }),
            ],
          });
        }
        break;
      }
      case "progression": {
        const siril = tool(tools, "siril");
        const name = toolName(siril, "Siril", locale);
        base.tool =
          optionState === "toolMissing"
            ? missingTool(name)
            : optionState === "reserved"
              ? [name]
              : [
                  `${[name, siril?.version].filter(Boolean).join(" ")} · ${t("wizard.treatments.progression.tool")}`,
                ];
        base.description = t("wizard.treatments.progression.line");
        break;
      }
    }
    return base;
  });

  const stacking = options.find((option) => option.treatment === "stacking");
  return {
    reservedLine:
      input.privateServer === null &&
      options.some((option) => option.state === "reserved"),
    privateServer: input.privateServer,
    options,
    contrastShown:
      !!stacking?.ticked && (stacking.cards ?? []).some((card) => card.ticked),
  };
}

/** The band of a target: that of its largest family (the one a space that
 * keeps one image stacks); broadband before the integration is read. */
export function targetBand(preview: IntegrationPreview | null): Band {
  const largest = (preview?.families ?? []).reduce<
    IntegrationPreview["families"][number] | null
  >(
    (best, family) =>
      !best || family.totalSeconds > best.totalSeconds ? family : best,
    null,
  );
  return largest?.band ?? "broadband";
}

/** « Siril et l'environnement Python »: the tools the stacking waits for. */
function missingTools(
  t: Translate,
  locale: string,
  tools: readonly ToolRow[],
): string[] {
  return ["siril", "python-env"]
    .filter((id) => {
      const row = tools.find((item) => item.id === id);
      return row?.state !== "present" && row?.state !== "downloaded";
    })
    .map((id) =>
      id === "siril"
        ? (tools.find((item) => item.id === id)?.name[locale] ?? "Siril")
        : t("wizard.stacking.pythonEnv"),
    );
}

/** One card of the box « Empilement ». */
export function stackingCard(
  t: Translate,
  locale: string,
  input: {
    target: NightDetection["targets"][number];
    /** The stacking can run (its tools are here). */
    open: boolean;
    online: NightOnline | null;
    choices: TreatmentChoices;
    pipelines: readonly Pipeline[];
    tools: readonly ToolRow[];
    several: boolean;
    preview: IntegrationPreview | null;
    eveningDate: string | null;
    engine: PolishedEngine;
    progress: Progress;
    /** The night's folder: where the image of a device is read in place. */
    detectionFolder?: string | null;
    /** What the night folder's detection said (the darks and the flats that
     * are missing are told under the card). */
    findings?: readonly Finding[];
    /** « Finition par défaut » (the lines of the finishes that differ). */
    finishDefault?: PipelineFinish;
  },
): StackingCard {
  const { target, preview, pipelines } = input;
  const ticked = input.open && input.choices.stackTargets.has(target.targetId);
  const deviceChoices = input.choices.deviceUse ?? NO_DEVICE_CHOICES;
  const device = ticked
    ? deviceChoiceView(t, locale, {
        target,
        choices: deviceChoices,
        detection: input.detectionFolder
          ? { folder: input.detectionFolder }
          : null,
        day: input.eveningDate ? eveningShort(input.eveningDate, locale) : null,
      })
    : null;
  const band = targetBand(preview);
  // The families the renders start from the image of the device are not
  // integrated: their lines and their master are left out of the integration.
  const deviceLabels = new Set(
    (device?.families ?? [])
      .filter((family) => family.source === "device")
      .map((family) => family.label),
  );
  const families = preview?.families ?? [];
  const integrated = families.filter(
    (family) => !deviceLabels.has(family.label),
  );
  const seconds = families.reduce(
    (sum, family) => sum + family.totalSeconds,
    0,
  );
  const duration = families.length > 0 ? integrationTime(t, seconds) : null;
  const deviceImages = target.device?.images ?? [];
  // A target with an image of a device says it first;
  // without poses here, the poses are the image's.
  const framesCount =
    target.lightFrames > 0
      ? target.lightFrames
      : deviceImages.reduce((sum, image) => sum + image.poses, 0);
  const line = [
    deviceImages[0]?.device ?? null,
    pluralText(t, locale, "wizard.files.frames", framesCount),
    duration,
    families.length > 0
      ? pluralText(t, locale, "multiNight.familyCount", families.length)
      : null,
  ]
    .filter(Boolean)
    .join(" · ");
  const chosen = inListOrder(
    rendersOf(
      input.choices.targetRenders,
      target.targetId,
      band,
      pipelines,
      input.several,
    ),
    pipelines,
  );
  const graxpert = toolPill(t, locale, {
    id: "graxpert",
    required: false,
    row: input.tools.find((row) => row.id === "graxpert"),
    progress: input.progress,
  });
  const chips: PipelineChip[] = chosen.map((id) => {
    const pipeline = pipelines.find((p) => p.id === id);
    let warning: string | null = null;
    if (pipeline && isDiscouraged(pipeline, band)) {
      warning = t("wizard.stacking.discouraged");
    } else if (
      usesGraxpert(pipeline) &&
      input.engine !== "siril" &&
      graxpert.state !== "present"
    ) {
      warning =
        graxpert.state === "downloading"
          ? t("wizard.stacking.downloading", {
              tool: graxpert.name,
              percent: String(graxpert.percent ?? 0),
            })
          : t("wizard.stacking.withoutGraxpert");
    }
    return { id, name: pipelineName(t, pipelines, id, locale), warning };
  });
  const missing = input.open ? [] : missingTools(t, locale, input.tools);
  const noPipeline =
    missing.length > 0
      ? pluralText(t, locale, "wizard.stacking.noPipeline", missing.length, {
          tools: conjunction(locale, missing),
        })
      : null;
  const capture = onlineCapture(input.online, target.targetId);
  const localFinal = target.finalImage ? baseName(target.finalImage) : null;
  const notes: CardNote[] = [];
  if (!input.open) {
    // The stacking cannot run: the card says only why.
  } else if (capture && ticked) {
    notes.push({
      segs: [
        t("wizard.treatments.stacking.variantCreated", {
          target: target.designation,
        }),
      ],
      tone: "strong",
    });
    notes.push({
      segs: [t("wizard.treatments.stacking.onlineKept")],
      tone: "muted",
    });
    if (localFinal) {
      notes.push({
        segs: [
          t("wizard.treatments.stacking.oneVariant", {
            target: target.designation,
            file: localFinal,
          }),
        ],
        tone: "variant",
      });
    }
  } else if (capture) {
    notes.push({
      segs: [t("wizard.stacking.variantWould")],
      tone: "muted",
    });
  } else if (localFinal && ticked) {
    const [before, after] = t("wizard.stacking.insteadOf", {
      file: "\u0000",
    }).split("\u0000");
    notes.push({
      segs: [before, { strong: localFinal }, after ?? ""].filter(
        (seg) => seg !== "",
      ),
      tone: "muted",
    });
  }
  return {
    targetId: target.targetId,
    designation: target.designation,
    folderName: target.folderName ?? null,
    ticked,
    enabled: input.open,
    band,
    bandLabel: t(
      band === "dualband"
        ? "wizard.stacking.band.dualband"
        : "wizard.stacking.band.broadband",
    ),
    line,
    filter:
      families.length > 0
        ? (families.reduce((best, family) =>
            family.totalSeconds > best.totalSeconds ? family : best,
          ).filter ?? null)
        : null,
    duration,
    seconds: families.length > 0 ? seconds : null,
    frames: framesCount,
    device,
    restacksPoses: restacksPoses(target, deviceChoices),
    chosen,
    chips: noPipeline ? [] : chips,
    finishes: noPipeline
      ? []
      : changedFinishes(
          input.choices.targetFinishes,
          target.targetId,
          chosen,
          input.finishDefault ?? DEFAULT_FINISH,
          NO_PROCESSING_ID,
        ).map(({ pipelineId, finish }) => ({
          pipeline: pipelineName(t, pipelines, pipelineId, locale),
          finish: finishLine(finish, t, locale),
        })),
    frame: noPipeline
      ? null
      : frameLine(t, input.choices.targetFrames?.[target.targetId]),
    noPipeline,
    button: !input.open ? null : chosen.length > 0 ? "tertiary" : "secondary",
    integration:
      ticked &&
      preview &&
      target.folderName &&
      (integrated.length > 0 || preview.outside > 0)
        ? targetIntegrationView(t, locale, {
            targetId: target.targetId,
            targetFolder: target.folderName,
            preview: { ...preview, families: integrated },
            eveningDate: input.eveningDate,
          })
        : null,
    calibration: ticked
      ? calibrationFindings(
          t,
          locale,
          input.findings ?? [],
          target.targetId,
        ).map((item) => ({ level: item.level, text: item.treatment }))
      : [],
    notes,
  };
}
