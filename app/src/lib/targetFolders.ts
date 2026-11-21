// The folders of a target, in its form (`working_folder::TargetLayout`):
// `input/` for what the person gives Astro Sync, `output/` for what Astro Sync
// makes, or the earlier form of a target folder made before Astro Sync 1.2.0
// (`lights/`, `final/`, `stacked/` side by side), still read as it is. The
// folder names are English in every language. Pure TypeScript, tested under
// `node --test`.

/** `working_folder::TargetLayout`. */
export type TargetLayout = "inputOutput" | "legacy";

/** The folders of one target, relative to it, each with a trailing `/`. */
export interface TargetFolders {
  /** The poses: `input/lights/`. */
  lights: string;
  /** The export of the person's processing software: `input/processed/`. */
  processed: string;
  /** The image a device stacked: `input/device/`. */
  device: string;
  /** What the stacking writes: `output/stacked/`. */
  stacked: string;
  /** The progression images: `output/progression/`. */
  progression: string;
}

const INPUT_OUTPUT: TargetFolders = {
  lights: "input/lights/",
  processed: "input/processed/",
  device: "input/device/",
  stacked: "output/stacked/",
  progression: "output/progression/",
};

const LEGACY: TargetFolders = {
  lights: "lights/",
  processed: "final/",
  device: "device/",
  stacked: "stacked/",
  progression: "progression/",
};

/** The folders of a target of `layout`; a detection of an earlier build says
 * no layout, and every target Astro Sync creates is of the new form. */
export function targetFolders(layout?: TargetLayout | null): TargetFolders {
  return layout === "legacy" ? LEGACY : INPUT_OUTPUT;
}

/** `M31/input/lights/`: a folder of a target, relative to the night. */
export function targetPath(
  folderName: string,
  part: keyof TargetFolders,
  layout?: TargetLayout | null,
): string {
  const name = folderName.replace(/\/+$/u, "");
  return `${name}/${targetFolders(layout)[part]}`;
}

/** What a path relative to a night's folder lies in, by its segments after
 * the night (`["M31", "input", "lights", "Ha", "l0.fit"]`): the target's
 * folder, the part and the segments under the part's folder; `null` for
 * anything else. Both forms are read. */
export function partOfNightPath(
  segments: readonly string[],
): { target: string; part: keyof TargetFolders; under: string[] } | null {
  if (segments.length < 3) return null;
  const [target, first, second] = segments;
  const lower = (value: string | undefined) => (value ?? "").toLowerCase();
  for (const layout of ["inputOutput", "legacy"] as const) {
    const folders = targetFolders(layout);
    for (const part of Object.keys(folders) as (keyof TargetFolders)[]) {
      const names = folders[part].replace(/\/$/u, "").split("/");
      const matches =
        names.length === 2
          ? lower(first) === names[0] && lower(second) === names[1]
          : lower(first) === names[0];
      if (matches && segments.length > names.length + 1) {
        return { target, part, under: segments.slice(1 + names.length) };
      }
    }
  }
  return null;
}
