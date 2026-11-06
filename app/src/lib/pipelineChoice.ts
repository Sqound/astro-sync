// The pipelines of each target of « Traitements »: one card per target, its
// pipelines chosen in the window « Pipelines pour {cible} ». A pipeline is
// recommended or discouraged for the filter of the target, never refused; a
// tool it needs that is missing is always offered with its size; « Sans
// traitement » is one choice more, the last card, combinable with the others;
// a target ticked for the stacking never has zero pipeline. Pure TypeScript,
// tested under `node --test`.

import type { TranslateParams } from "../i18n/locale.ts";
import type { Band, Pipeline, PolishedEngine, ToolRow } from "./bridge";
import { formatSize, pluralText } from "./written.ts";

type Translate = (key: string, params?: TranslateParams) => string;

/** « Sans traitement »: the id of the core (`LINEAR_PIPELINE`). */
export const NO_PROCESSING_ID = "linear";
/** « Rendu naturel » and « Correction bibande », the defaults. */
export const NATURAL_ID = "raw";
export const DUALBAND_ID = "dualband-correction";
/** The key of a choice made for every target at once (a choice kept before
 * the choice was made target by target): read as the choice of each target
 * that has none of its own. */
export const EVERY_TARGET = "*";

/** The pipelines chosen, by target id. */
export type TargetRenders = Readonly<Record<string, readonly string[]>>;

/** The pipelines that may run: a script the person refused never does. */
export function usablePipelines(pipelines: readonly Pipeline[]): Pipeline[] {
  return pipelines.filter((pipeline) => pipeline.trust !== "refused");
}

/**
 * What a target starts with: « Rendu naturel », and « Correction bibande »
 * too when the target is dual-band. A space
 * that keeps one image per capture gets one: the correction on a dual-band
 * target, the natural render otherwise. A pipeline this computer does not
 * have is never chosen; with neither, the first that may run.
 */
export function defaultRenders(
  band: Band,
  pipelines: readonly Pipeline[],
  several = true,
): string[] {
  const usable = usablePipelines(pipelines);
  const has = (id: string) => usable.some((pipeline) => pipeline.id === id);
  const chosen: string[] = [];
  if (has(NATURAL_ID)) chosen.push(NATURAL_ID);
  if (band === "dualband" && has(DUALBAND_ID)) {
    if (several) chosen.push(DUALBAND_ID);
    else chosen.splice(0, chosen.length, DUALBAND_ID);
  }
  if (chosen.length === 0 && usable[0]) chosen.push(usable[0].id);
  return chosen;
}

/**
 * The pipelines of `targetId`: its own choice, else the choice kept for
 * every target (`EVERY_TARGET`), else its default. What is no longer listed,
 * or refused, is dropped; « Sans traitement » always stays possible. In a
 * space that keeps one image, only the first is kept.
 */
export function rendersOf(
  chosen: TargetRenders,
  targetId: string,
  band: Band,
  pipelines: readonly Pipeline[],
  several = true,
): string[] {
  const usable = usablePipelines(pipelines);
  const raw = chosen[targetId] ?? chosen[EVERY_TARGET] ?? null;
  const kept = (raw ?? []).filter(
    (id, index, all) =>
      all.indexOf(id) === index &&
      (id === NO_PROCESSING_ID || usable.some((p) => p.id === id)),
  );
  const result =
    kept.length > 0 ? kept : defaultRenders(band, pipelines, several);
  return several ? result : result.slice(0, 1);
}

/** The display order: the pipelines as listed, « Sans traitement »
 * last. */
export function inListOrder(
  ids: readonly string[],
  pipelines: readonly Pipeline[],
): string[] {
  const listed = pipelines
    .map((pipeline) => pipeline.id)
    .filter((id) => ids.includes(id));
  return ids.includes(NO_PROCESSING_ID)
    ? [...listed, NO_PROCESSING_ID]
    : listed;
}

/** Ticking or unticking a card of the window: several when the space keeps
 * variants (zero is possible here: « Valider » then says why it waits), one
 * otherwise. A refused script is never ticked. */
export function toggleChoice(
  current: readonly string[],
  id: string,
  pipelines: readonly Pipeline[],
  several: boolean,
): string[] {
  if (pipelines.find((p) => p.id === id)?.trust === "refused") {
    return [...current];
  }
  if (!several) return [id];
  return current.includes(id)
    ? current.filter((item) => item !== id)
    : inListOrder([...current, id], pipelines);
}

/** Whether ticking `id` must first ask the person's trust in its script. */
export function asksTrust(
  pipelines: readonly Pipeline[],
  id: string,
  current: readonly string[],
): boolean {
  const trust = pipelines.find((p) => p.id === id)?.trust ?? "not_needed";
  return (trust === "ask" || trust === "changed") && !current.includes(id);
}

/** « Appliquer à toutes les cibles »: the selection copied on every target
 * ticked for the stacking, a pipeline discouraged for one of them included
 * (it stays said on its card). */
export function applyToAll(
  chosen: TargetRenders,
  targetIds: Iterable<string>,
  selection: readonly string[],
): Record<string, string[]> {
  const next: Record<string, string[]> = {};
  for (const [key, value] of Object.entries(chosen)) next[key] = [...value];
  for (const targetId of targetIds) next[targetId] = [...selection];
  return next;
}

/* --------------------------- Kept on this computer ------------------------ */

/** What `localStorage` offers (the tests give their own). */
export interface ChoiceStorage {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
}

const storageKey = (sessionId: string) => `astro-sync.pipelines:${sessionId}`;

/**
 * The pipelines chosen for the targets of a night, kept on this computer.
 * A list alone (one choice for every target, as the window kept it before
 * the choice was made target by target) is read silently as the choice of
 * every target that has none of its own (`EVERY_TARGET`).
 */
export function loadTargetRenders(
  storage: ChoiceStorage | null,
  sessionId: string,
): Record<string, string[]> {
  try {
    const text = storage?.getItem(storageKey(sessionId));
    const parsed = text ? (JSON.parse(text) as unknown) : null;
    const ids = (value: unknown) =>
      Array.isArray(value)
        ? value.filter((item): item is string => typeof item === "string")
        : [];
    if (Array.isArray(parsed)) {
      const every = ids(parsed);
      return every.length > 0 ? { [EVERY_TARGET]: every } : {};
    }
    if (parsed && typeof parsed === "object") {
      return Object.fromEntries(
        Object.entries(parsed as Record<string, unknown>)
          .map(([key, value]) => [key, ids(value)] as const)
          .filter(([, value]) => value.length > 0),
      );
    }
  } catch {
    // A choice that cannot be read: every target takes its default.
  }
  return {};
}

export function saveTargetRenders(
  storage: ChoiceStorage | null,
  sessionId: string,
  chosen: TargetRenders,
): void {
  try {
    storage?.setItem(storageKey(sessionId), JSON.stringify(chosen));
  } catch {
    // A choice lost only brings the defaults back.
  }
}

/* ------------------------------ Names, bands ------------------------------ */

/** The name of a pipeline in the language of the window. */
export function pipelineName(
  t: Translate,
  pipelines: readonly Pipeline[],
  id: string,
  locale: string,
): string {
  if (id === NO_PROCESSING_ID) return t("wizard.treatments.noProcessing");
  const pipeline = pipelines.find((p) => p.id === id);
  if (!pipeline) return id;
  return (
    pipeline.displayName ??
    pipeline.name[locale] ??
    pipeline.name.en ??
    pipeline.id
  );
}

/** Whether the pipeline is made for this band of target (« Recommandé »). */
export function isRecommended(pipeline: Pipeline, band: Band): boolean {
  return (pipeline.recommendedFor ?? []).includes(band);
}

/** Whether it is discouraged for this band (« Déconseillé pour ce filtre »). */
export function isDiscouraged(pipeline: Pipeline, band: Band): boolean {
  return (pipeline.discouragedFor ?? []).includes(band);
}

/** The pipelines that use GraXpert (« Rendu soigné »). */
export function usesGraxpert(pipeline: Pipeline | undefined): boolean {
  return (pipeline?.dependencies ?? []).some((d) => d.id === "graxpert");
}

/* ------------------------------ The tools --------------------------------- */

/** A download or an installation in progress, by tool id: its percent, or
 * `null` while the size is unknown. */
export type Progress = Readonly<Record<string, number | null>>;

export interface ToolPill {
  id: string;
  name: string;
  /** `present`, `missing` (or too old), `downloading`. */
  state: "present" | "missing" | "downloading";
  optional: boolean;
  /** « Facultatif », « Absent », « Non installé »; null for a tool present
   * and required. */
  stateLabel: string | null;
  /** « Télécharger (130 Mo) », « Installer (82 Mo) »: what is done with it;
   * `page` opens the download page of the registry. */
  action: { label: string; kind: "download" | "install" | "page" } | null;
  percent: number | null;
}

/** The names of the registry's tools, when their row is not read yet. */
const TOOL_NAMES: Record<string, string> = {
  siril: "Siril",
  starnet2: "StarNet2",
  graxpert: "GraXpert",
};

function toolName(row: ToolRow | undefined, id: string, locale: string) {
  return (
    row?.name[locale] ?? row?.name.en ?? row?.name.fr ?? TOOL_NAMES[id] ?? id
  );
}

/** One tool of a pipeline, as its pill says it: present, missing with its
 * button and size, or downloading with its percent. */
export function toolPill(
  t: Translate,
  locale: string,
  input: {
    id: string;
    required: boolean;
    bytes?: number | null;
    row: ToolRow | undefined;
    progress: Progress;
  },
): ToolPill {
  const { id, row } = input;
  const python = row?.kind === "python-packages" || id === "python-env";
  const present = row?.state === "present" || row?.state === "downloaded";
  const downloading = id in input.progress;
  const bytes =
    input.bytes ?? row?.downloadBytes ?? row?.download?.bytes ?? null;
  const size = bytes ? formatSize(t, bytes, locale) : null;
  const kind: "download" | "install" | "page" = python
    ? "install"
    : row?.download
      ? "download"
      : "page";
  const label = python
    ? size
      ? t("wizard.pipelines.install", { size })
      : t("wizard.pipelines.installNoSize")
    : size
      ? t("wizard.pipelines.download", { size })
      : t("wizard.pipelines.downloadNoSize");
  return {
    id,
    name: python ? t("wizard.pipelines.python") : toolName(row, id, locale),
    state: present ? "present" : downloading ? "downloading" : "missing",
    optional: !input.required,
    stateLabel: present
      ? input.required
        ? null
        : t("wizard.pipelines.tool.optional")
      : downloading
        ? null
        : input.required
          ? t("wizard.pipelines.tool.absent")
          : t("wizard.pipelines.tool.notInstalled"),
    action: present || downloading ? null : { label, kind },
    percent: downloading ? (input.progress[id] ?? null) : null,
  };
}

/* ------------------------- « Pipelines pour {cible} » ---------------------- */

export interface DialogTarget {
  targetId: string;
  designation: string;
  frames: number;
  /** The duration of its frames in the night, as the card says it. */
  duration: string | null;
  /** The filter of its frames (« UV/IR-cut »); null when none is named. */
  filter: string | null;
  band: Band;
}

export type PipelinePreview =
  { kind: "image"; src: string; alt: string } | { kind: "linear"; alt: string };

export interface PipelineCardView {
  id: string;
  name: string;
  /** « Tous filtres », « Bibande », the filter of an added pipeline. */
  filterLabel: string;
  /** « Ajouté par vous ». */
  added: boolean;
  recommended: boolean;
  /** « Déconseillé pour ce filtre : … ». */
  discouraged: string | null;
  description: string;
  /** The banner under the description (« Aperçu du rendu … »): the image of
   * the pipeline's `preview` (a data URL), or, for « Sans traitement », the
   * linear grey image the application provides; null when there is none
   * (no banner, no empty space). */
  preview: PipelinePreview | null;
  steps: { title: string; text: string }[];
  /** « Pourquoi ces outils »: each tool, its version or « Facultatif », and
   * why this pipeline needs it. */
  why: { name: string; detail: string | null; reason: string }[];
  tools: ToolPill[];
  produces: string;
  ticked: boolean;
  /** A refused script: listed, never ticked. */
  disabled: boolean;
  /** Why it is disabled, or that it will ask. */
  note: string | null;
  /** The engine of the « Rendu soigné » and its cost. */
  engine: string | null;
  /** GraXpert missing or downloading: what the render does meanwhile. */
  fallback: string | null;
}

export interface PipelineDialogView {
  title: string;
  subtitle: string;
  cards: PipelineCardView[];
  /** Ticked: « 2 pipelines choisis », or the reason « Valider » waits. */
  count: string;
  valid: boolean;
  several: boolean;
}

function discouragedReason(
  t: Translate,
  pipeline: Pipeline,
  target: DialogTarget,
): string {
  const filter = target.filter ?? t("wizard.pipelines.unknownFilterName");
  const key =
    pipeline.id === DUALBAND_ID || pipeline.id === "hoo"
      ? `wizard.pipelines.discouraged.${pipeline.id}`
      : "wizard.pipelines.discouraged.other";
  return t(key, { target: target.designation, filter });
}

function filterLabel(t: Translate, filter: string | undefined): string {
  if (!filter || filter === "none") return t("wizard.pipelines.filter.none");
  if (filter === "dual-band") return t("wizard.pipelines.filter.dualband");
  return filter;
}

/** The window « Pipelines pour {cible} ». */
export function pipelineDialogView(
  t: Translate,
  locale: string,
  input: {
    target: DialogTarget;
    pipelines: readonly Pipeline[];
    tools: readonly ToolRow[];
    selection: readonly string[];
    several: boolean;
    engine: PolishedEngine;
    progress: Progress;
  },
): PipelineDialogView {
  const { target, tools, progress } = input;
  const row = (id: string) => tools.find((item) => item.id === id);
  const graxpert = toolPill(t, locale, {
    id: "graxpert",
    required: false,
    row: row("graxpert"),
    progress,
  });
  const cards: PipelineCardView[] = input.pipelines.map((pipeline) => {
    const trust = pipeline.trust ?? "not_needed";
    const disabled = trust === "refused";
    const graxpertCard = usesGraxpert(pipeline);
    const outputs = (pipeline.outputs ?? ["tif", "png"]).map((ext) =>
      ext === "tif" ? "TIFF" : ext.toUpperCase(),
    );
    return {
      id: pipeline.id,
      name: pipelineName(t, input.pipelines, pipeline.id, locale),
      filterLabel: filterLabel(t, pipeline.filter),
      added: pipeline.source === "user",
      recommended: isRecommended(pipeline, target.band),
      discouraged: isDiscouraged(pipeline, target.band)
        ? discouragedReason(t, pipeline, target)
        : null,
      description:
        pipeline.displayDescription ??
        pipeline.description[locale] ??
        pipeline.description.en ??
        "",
      preview: pipeline.previewData
        ? {
            kind: "image",
            src: pipeline.previewData,
            alt: t("pipeline.preview.alt", {
              name: pipelineName(t, input.pipelines, pipeline.id, locale),
            }),
          }
        : null,
      steps: pipeline.displayDetails ?? [],
      why: (pipeline.dependencies ?? [])
        .filter((dependency) => dependency.reason)
        .map((dependency) => {
          const pill = toolPill(t, locale, {
            id: dependency.id,
            required: dependency.required,
            row: row(dependency.id),
            progress,
          });
          const version = row(dependency.id)?.version ?? null;
          return {
            name: pill.name,
            detail:
              [version, dependency.required ? null : pill.stateLabel]
                .filter(Boolean)
                .join(" · ") || null,
            reason:
              dependency.reason?.[locale] ??
              dependency.reason?.[locale.split("-")[0]] ??
              dependency.reason?.en ??
              "",
          };
        }),
      tools: (pipeline.dependencies ?? []).map((dependency) =>
        toolPill(t, locale, {
          id: dependency.id,
          required: dependency.required,
          bytes: dependency.bytes,
          row: row(dependency.id),
          progress,
        }),
      ),
      produces: t("wizard.pipelines.produces", {
        formats: outputs.join(" · "),
      }),
      ticked: !disabled && input.selection.includes(pipeline.id),
      disabled,
      note: disabled
        ? t("wizard.treatments.renderRefused")
        : trust === "ask" || trust === "changed"
          ? t(
              trust === "changed"
                ? "wizard.treatments.renderChanged"
                : "wizard.treatments.renderAsks",
            )
          : null,
      engine: graxpertCard
        ? t(
            input.engine === "siril"
              ? "wizard.pipelines.engine.siril"
              : "wizard.pipelines.engine.graxpert",
          )
        : null,
      fallback:
        graxpertCard && input.engine !== "siril" && graxpert.state !== "present"
          ? t(
              graxpert.state === "downloading"
                ? "wizard.pipelines.fallbackDownloading"
                : "wizard.pipelines.fallback",
            )
          : null,
    };
  });
  cards.push({
    id: NO_PROCESSING_ID,
    name: t("wizard.treatments.noProcessing"),
    filterLabel: t("wizard.pipelines.filter.none"),
    added: false,
    recommended: false,
    discouraged: null,
    description: t("wizard.pipelines.noProcessingDescription"),
    preview: {
      kind: "linear",
      alt: t("pipeline.preview.alt", {
        name: t("wizard.treatments.noProcessing"),
      }),
    },
    steps: [],
    why: [],
    tools: ["siril", "python-env"].map((id) =>
      toolPill(t, locale, { id, required: true, row: row(id), progress }),
    ),
    produces: t("wizard.pipelines.producesLinear"),
    ticked: input.selection.includes(NO_PROCESSING_ID),
    disabled: false,
    note: null,
    engine: null,
    fallback: null,
  });
  const ticked = cards.filter((card) => card.ticked).length;
  const frames = pluralText(t, locale, "wizard.files.frames", target.frames);
  const subtitle = [
    frames,
    target.duration,
    t("wizard.pipelines.filterOf", {
      filter: target.filter ?? t("wizard.pipelines.unknownFilter"),
    }),
    input.several
      ? t(
          target.band === "dualband"
            ? "wizard.pipelines.kind.dualband"
            : "wizard.pipelines.kind.broadband",
        )
      : t("wizard.pipelines.oneOnly"),
  ]
    .filter(Boolean)
    .join(" · ");
  return {
    title: t("wizard.pipelines.title", { target: target.designation }),
    subtitle,
    cards,
    count:
      ticked > 0
        ? pluralText(t, locale, "wizard.pipelines.count", ticked)
        : t("wizard.pipelines.none"),
    valid: ticked > 0,
    several: input.several,
  };
}
