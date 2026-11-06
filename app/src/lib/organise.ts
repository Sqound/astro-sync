// « Ranger les fichiers… »: what the dialog says and draws, from the engine's
// `OrganisePlan` (pure, tested in `organise.test.ts`). The tree is read back
// from the plan's actions, never recomputed from the sessions.

import type {
  OrganiseMode,
  OrganiseOutcome,
  OrganisePlan,
  OrganiseAction,
} from "./bridge";

export const ORGANISE_MODES: readonly OrganiseMode[] = [
  "leave-in-place",
  "copy",
  "move",
  "symlink",
];

/** The key of each mode's name in the segmented command. */
export const FILE_MODE_KEY: Record<OrganiseMode, string> = {
  "leave-in-place": "fileMode.leave",
  copy: "fileMode.copy",
  move: "fileMode.move",
  symlink: "fileMode.link",
};

/** « Déplacer » is the one choice that removes the source. */
export function isDestructive(mode: OrganiseMode): boolean {
  return mode === "move";
}

/** The mode the person can pick: only « Lien symbolique » can be held, by
 * Windows without developer mode. */
export function modeAvailable(
  mode: OrganiseMode,
  windowsNoLink: boolean,
): boolean {
  return !(mode === "symlink" && windowsNoLink);
}

export interface OrganisePrimary {
  /** `organise.done` (no count) or the base of a `.one`/`.other` pair. */
  key: string;
  counted: boolean;
  destructive: boolean;
}

/** The primary of the dialog follows the choice. */
export function organisePrimary(mode: OrganiseMode): OrganisePrimary {
  switch (mode) {
    case "copy":
      return { key: "organise.copy", counted: true, destructive: false };
    case "move":
      return { key: "organise.move", counted: true, destructive: true };
    case "symlink":
      return { key: "organise.link", counted: true, destructive: false };
    default:
      return { key: "organise.done", counted: false, destructive: false };
  }
}

/** The sentence under the tree for the choice (`organise.*Line`). */
export function modeSentenceKey(mode: OrganiseMode): string {
  switch (mode) {
    case "copy":
      return "organise.copyLine";
    case "move":
      return "organise.moveLine";
    case "symlink":
      return "organise.linkLine";
    default:
      return "organise.leaveLine";
  }
}

/** The tone of that sentence: the move warns, the others reassure. */
export function modeSentenceTone(mode: OrganiseMode): "success" | "warning" {
  return mode === "move" ? "warning" : "success";
}

export type PlacedKind = "lights" | "calibration" | "logs" | "device";

export interface Placed {
  night: string;
  kind: PlacedKind;
  /** The target (lights), the calibration folder, or empty (logs). */
  group: string;
  /** The filter level of the lights, empty when there is none. */
  filter: string;
}

function segments(path: string): string[] {
  return path.replace(/\\/g, "/").split("/").filter(Boolean);
}

/** Where an action puts its file, from its destination under the plan's. */
export function placed(
  plan: OrganisePlan,
  action: OrganiseAction,
): Placed | null {
  const root = segments(plan.destination);
  const parts = segments(action.destination);
  const under = parts.slice(root.length);
  // <night>/<target>/lights/[<filter>/]<file>, <night>/calibration/<type>/<file>
  // or <night>/logs/<file>.
  if (under.length < 3) return null;
  const [night, second, third] = under;
  if (second === "logs" && under.length === 3) {
    return { night, kind: "logs", group: "", filter: "" };
  }
  if (second === "calibration" && under.length === 4) {
    return { night, kind: "calibration", group: third, filter: "" };
  }
  // The image a device stacked: <night>/<target>/device/<file>.
  if (third === "device" && under.length === 4) {
    return { night, kind: "device", group: second, filter: "" };
  }
  if (third === "lights" && under.length >= 5) {
    return { night, kind: "lights", group: second, filter: under[3] };
  }
  if (third === "lights" && under.length === 4) {
    return { night, kind: "lights", group: second, filter: "" };
  }
  return null;
}

export interface OrganiseCounts {
  frames: number;
  calibration: number;
  logs: number;
}

/** « 417 poses, 561 de calibration et 23 journaux », from the plan. */
export function organiseCounts(plan: OrganisePlan): OrganiseCounts {
  const counts = { frames: 0, calibration: 0, logs: 0 };
  for (const action of plan.actions) {
    const where = placed(plan, action);
    if (!where) continue;
    if (where.kind === "lights") counts.frames += 1;
    else if (where.kind === "calibration") counts.calibration += 1;
    else if (where.kind === "logs") counts.logs += 1;
    // The images of a device are shown in the tree, not counted as poses.
  }
  return counts;
}

export interface TreeTarget {
  name: string;
  /** The images a device stacked, in `device/`. */
  device: number;
  frames: number;
  filters: { name: string; frames: number }[];
}

export interface TreeNight {
  name: string;
  targets: TreeTarget[];
  calibration: string[];
  logs: number;
}

const CALIBRATION_ORDER = ["darks", "flats", "dark-flats", "bias", "biases"];

/** The nights of the plan, the latest first, each with its targets, its
 * calibration folders and its logs. */
export function organiseTree(plan: OrganisePlan): TreeNight[] {
  const nights = new Map<string, TreeNight>();
  for (const action of plan.actions) {
    const where = placed(plan, action);
    if (!where) continue;
    let night = nights.get(where.night);
    if (!night) {
      night = { name: where.night, targets: [], calibration: [], logs: 0 };
      nights.set(where.night, night);
    }
    if (where.kind === "logs") {
      night.logs += 1;
    } else if (where.kind === "device") {
      let target = night.targets.find((item) => item.name === where.group);
      if (!target) {
        target = { name: where.group, device: 0, frames: 0, filters: [] };
        night.targets.push(target);
      }
      target.device += 1;
    } else if (where.kind === "calibration") {
      if (!night.calibration.includes(where.group)) {
        night.calibration.push(where.group);
      }
    } else {
      let target = night.targets.find((item) => item.name === where.group);
      if (!target) {
        target = { name: where.group, device: 0, frames: 0, filters: [] };
        night.targets.push(target);
      }
      target.frames += 1;
      if (where.filter) {
        let filter = target.filters.find((item) => item.name === where.filter);
        if (!filter) {
          filter = { name: where.filter, frames: 0 };
          target.filters.push(filter);
        }
        filter.frames += 1;
      }
    }
  }
  for (const night of nights.values()) {
    night.calibration.sort(
      (a, b) =>
        (CALIBRATION_ORDER.indexOf(a) + 1 || 99) -
        (CALIBRATION_ORDER.indexOf(b) + 1 || 99),
    );
  }
  return [...nights.values()].sort((a, b) => (a.name < b.name ? 1 : -1));
}

export interface TreeWords {
  /** « 80 poses ». */
  poses: (count: number) => string;
  /** « 4 autres nuits ». */
  moreNights: (count: number) => string;
}

/** The tree as the text of the dialog's box: the destination, then at most
 * `maxNights` nights (`moreNights` says how many are left). */
export function organiseTreeText(
  plan: OrganisePlan,
  words: TreeWords,
  maxNights = 2,
): string {
  const nights = organiseTree(plan);
  const shown = nights.slice(0, maxNights);
  const hidden = nights.length - shown.length;
  const rows: { left: string; note: string }[] = [];
  const root = plan.destination.replace(/[\\/]+$/, "");
  rows.push({ left: `${root}/`, note: "" });
  shown.forEach((night, nightIndex) => {
    const lastNight = hidden === 0 && nightIndex === shown.length - 1;
    const branch = lastNight ? "└─ " : "├─ ";
    const trunk = lastNight ? "   " : "│  ";
    rows.push({ left: `${branch}${night.name}/`, note: "" });
    const items: { left: string; note: string }[] = night.targets.flatMap(
      (target) => {
        const device =
          target.device > 0
            ? [{ left: `${target.name}/device/`, note: "" }]
            : [];
        if (target.frames === 0) return device;
        const lights =
          target.filters.length > 0
            ? [
                `${target.name}/lights/${target.filters[0].name}/`,
                ...target.filters.slice(1).map((f) => `lights/${f.name}/`),
              ].join("  ")
            : `${target.name}/lights/`;
        return [{ left: lights, note: words.poses(target.frames) }, ...device];
      },
    );
    if (night.calibration.length > 0) {
      items.push({
        left: `calibration/${night.calibration.map((c) => `${c}/`).join(" ")}`,
        note: "",
      });
    }
    if (night.logs > 0) items.push({ left: "logs/", note: "" });
    items.forEach((item, index) => {
      const mark = index === items.length - 1 ? "└─ " : "├─ ";
      rows.push({ left: `${trunk}${mark}${item.left}`, note: item.note });
    });
  });
  if (hidden > 0) {
    rows.push({ left: `└─ ${words.moreNights(hidden)}`, note: "" });
  }
  const width = Math.max(
    ...rows.filter((row) => row.note).map((row) => [...row.left].length),
    0,
  );
  return rows
    .map((row) =>
      row.note
        ? `${row.left}${" ".repeat(width - [...row.left].length + 4)}${row.note}`
        : row.left,
    )
    .join("\n");
}

/** The folder the files come from: the deepest folder all the plan's sources
 * share (« ~/Astro/ASIAIR »), empty when they share none. */
export function sourceFolder(plan: OrganisePlan): string {
  const lists = plan.actions.map((action) => {
    const parts = action.source.replace(/\\/g, "/").split("/");
    return parts.slice(0, -1);
  });
  if (lists.length === 0) return "";
  const common = lists.reduce((shared, parts) => {
    let n = 0;
    while (n < shared.length && n < parts.length && shared[n] === parts[n]) n++;
    return shared.slice(0, n);
  });
  return common.join("/");
}

export interface TextPart {
  text: string;
  code: boolean;
}

/** `text` cut around each of `tokens`, the tokens flagged as code (the
 * sentence names a folder in monospace; the translation stays one string). */
export function emphasise(text: string, tokens: string[]): TextPart[] {
  const wanted = tokens.filter(Boolean);
  if (wanted.length === 0) return [{ text, code: false }];
  const escaped = wanted
    .map((token) => token.replace(/[.*+?^${}()|[\]\\]/g, "\\$&"))
    .join("|");
  return text
    .split(new RegExp(`(${escaped})`))
    .filter((part) => part !== "")
    .map((part) => ({ text: part, code: wanted.includes(part) }));
}

/** The key of what a conflict or a failure says. */
export function reasonKey(reason: string): string {
  switch (reason) {
    case "same-name":
    case "destination-exists":
    case "checksum-differs":
      return `organise.reason.${reason}`;
    default:
      return "organise.reason.io";
  }
}

/** The file's own name, from a path of either style. */
export function fileName(path: string): string {
  return path.replace(/\\/g, "/").split("/").filter(Boolean).pop() ?? path;
}

export type EndKind = "stopped" | "moved" | "copied" | "linked";

export interface EndView {
  kind: EndKind;
  count: number;
  failed: number;
}

/** The end: what the title says, how many were done, how many failed. */
export function organiseEnd(
  mode: OrganiseMode,
  outcome: OrganiseOutcome,
): EndView {
  const kind: EndKind = outcome.stopped
    ? "stopped"
    : mode === "move"
      ? "moved"
      : mode === "symlink"
        ? "linked"
        : "copied";
  return { kind, count: outcome.done, failed: outcome.failed.length };
}

/** The bar's fill, 0 to 100: bytes when the engine counted them, files else. */
export function organisePercent(progress: {
  done: number;
  total: number;
  bytesDone: number;
  bytesTotal: number;
}): number {
  const ratio =
    progress.bytesTotal > 0
      ? progress.bytesDone / progress.bytesTotal
      : progress.total > 0
        ? progress.done / progress.total
        : 0;
  return Math.max(0, Math.min(100, Math.round(ratio * 100)));
}

/** The key of the phase the bar names. */
export function phaseKey(mode: OrganiseMode): string {
  return mode === "move"
    ? "organise.phase.move"
    : mode === "symlink"
      ? "organise.phase.link"
      : "organise.phase.copy";
}
